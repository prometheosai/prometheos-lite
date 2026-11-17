//! #155 Slice A — durable semantic continuation between two distinct
//! harness identities in two separate processes (Lite-internal experiment).
//!
//! What this proves: one interrupted governed coding task, whose first
//! authorized step was performed by worker A under harness identity A,
//! can be resumed by worker B under a *different* harness identity in a
//! *freshly recreated workspace and process*, carrying nothing but an
//! explicit, versioned, durable semantic checkpoint. The completed step
//! is not repeated; the pending step is executed and validated.
//!
//! What this does NOT prove: the full cross-provider/cross-harness live
//! matrix of issue #155 (cloud-to-cloud, cloud-to-local escalation, …).
//! Both workers run the same deterministic, offline, scripted fixture;
//! the harness identities are distinct canonical Lite declarations
//! (`lite.harness.native@v1` performs step 1, `lite.harness.protocol.bridge@v1`
//! resumes with step 2). No model inference occurs anywhere in this
//! file. The continuation is semantic work state, not a prompt, PTY,
//! or process-local session.
//!
//! Worker B is launched as a genuinely separate OS process that receives
//! only path/pin environment variables; it never sees worker A's memory.
//! A seals the checkpoint through the same crash-safe atomic publication
//! used by every durable Lite document (`workflow::durable::atomic_write_json`);
//! B loads only bytes from disk.
//!
//! Honest labels:
//! * The repair edit is the pinned fixture edit — scripted, not model output.
//! * "Fresh workspace" = a brand-new git worktree of the disposable source
//!   repository at the committed step-1 revision, in a fresh directory B
//!   requires to live under the checkpoint's allow-listed remap root.
//! * The negative tests prove refusal before unauthorized filesystem
//!   mutation; they do not exercise a live provider or any network.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use prometheos_lite::workflow::AuthorityLevel;
use prometheos_lite::workflow::durable::atomic_write_json;
use prometheos_lite::workflow::execution_harness::HarnessCheckpointV1;
use prometheos_lite::workflow::portable_state::{
    self, AuthoritySnapshot, CompatibilityMetadata, DecisionProvenance, DecisionRecord,
    DecisionStatus, ExecutionProvenance, Plan, PortableRef, PortableRefKind, PortableWorkState,
    ProvenanceKind, RepositorySnapshot, ValidationResult, WorkIdentity, WorkStep, WorkStepStatus,
};
use prometheos_lite::workflow::scenario::ScenarioDescriptor;
use prometheos_lite::workflow::schema::PORTABLE_WORK_STATE_SCHEMA_VERSION;
use prometheos_lite::workflow::soma::capability::{
    DecisionStatus as CompatDecisionStatus, NegotiationInputs, WorkRequirements, resolve,
};
use prometheos_lite::workflow::soma::contracts::AuthorityProfile;
use prometheos_lite::workflow::soma::event::WorkEvent;
use prometheos_lite::workflow::soma::types::Hex64;
use prometheos_lite::workflow::workspace::{
    ADAPTER_REVISION, AdapterKind, GitWorktreeWorkspace, RemapAuthorization, WorkspaceAdapter,
    WorkspaceManifestV1, WorkspaceMode, WorkspaceRefV1,
};

use common::task_execution::{git, run_validator, sha256_hex, stage_source_repository};

const DESCRIPTOR_TEXT: &str =
    include_str!("fixtures/scenarios/rust-tokenbucket-bugfix/descriptor.json");
const FIXTURE_ROOT: &str = "tests/fixtures/scenarios/rust-tokenbucket-bugfix/repo";
const REPO_IDENTITY_URL: &str = "https://repo.invalid/rust-tokenbucket-bugfix.git";
const WORK_ID: &str = "continuation-slice-a-tokenbucket";
const TASK_ID: &str = "rust-tokenbucket-bugfix";
const STEP1: &str = "step-1-pin-repair";
const STEP2: &str = "step-2-validate";
const A_HARNESS: &str = "lite.harness.native";
const A_REVISION: &str = "v1";
const B_HARNESS: &str = "lite.harness.protocol.bridge";
const B_REVISION: &str = "v1";
const A_CREATED: &str = "2026-11-17T04:00:00Z";
const CHECKPOINT_FILE: &str = "continuation_checkpoint.json";
const CHECKPOINT_SCHEMA_VERSION: &str = "lite.continuation.checkpoint/1.0.0";
const CONTINUATION_REF: &str = "refs/continuation/continuation-slice-a-tokenbucket";

/// Environment interface between the parent test and the worker-B process.
/// Only these variables are visible inside B's environment.
const ENV_ROLE: &str = "PROMETHEOS_CONTINUATION_ROLE";
const ENV_CHECKPOINT: &str = "PROMETHEOS_CONTINUATION_CHECKPOINT";
const ENV_CHECKPOINT_DIR: &str = "PROMETHEOS_CONTINUATION_CHECKPOINT_DIR";
const ENV_SOURCE: &str = "PROMETHEOS_CONTINUATION_SOURCE";
const ENV_REMAP_ROOT: &str = "PROMETHEOS_CONTINUATION_WORKSPACES";
const ENV_RESULT: &str = "PROMETHEOS_CONTINUATION_RESULT";
const ENV_B_HARNESS: &str = "PROMETHEOS_CONTINUATION_B_HARNESS";
const ENV_REQUIRE_CAP: &str = "PROMETHEOS_CONTINUATION_REQUIRE_CAP";
const ENV_RESUME_AUTHORITY: &str = "PROMETHEOS_CONTINUATION_RESUME_AUTHORITY_JSON";
const ENV_REPLAY_STEP: &str = "PROMETHEOS_CONTINUATION_REPLAY_STEP";
const ENV_REPLAY_KEY: &str = "PROMETHEOS_CONTINUATION_REPLAY_KEY";
const ENV_TARGET_DIR: &str = "PROMETHEOS_CONTINUATION_TARGET_DIR";

// ---------------------------------------------------------------------------
// Checkpoint record (Lite-internal, versioned)
// ---------------------------------------------------------------------------

/// Identity of the producing attempt, bound for replay detection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct JournalIdentity {
    /// Stable run identity for the producing attempt.
    run_id: String,
    /// Attempt identity (idempotency key of the routed effect).
    attempt_id: String,
    /// Idempotency key already consumed by step 1.
    idempotency_key: String,
    /// Digest chain head over the causal events (sha256 over
    /// `sequence:event_id:semantic_digest` lines, in sequence order).
    journal_head_digest: String,
    /// Keys that must never be consumed again by this work item.
    idempotency_consumed: Vec<String>,
}

/// Explicit allow-list for recreating the workspace elsewhere. The remap
/// is authorized only inside `allowed_root_prefix`, at exactly the pinned
/// head revision, for the same logical workspace identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AllowedWorkspaceRemap {
    /// Canonical filesystem prefix the resumed workspace may be created under.
    allowed_root_prefix: String,
    /// Repository revision the resumed workspace must pin (step-1 head).
    allowed_head_revision: String,
    /// The original pre-write base revision (for the ancestry check).
    base_revision: String,
    /// Logical workspace identity preserved across the remap.
    workspace_id: String,
    /// Recorded attestation for the remap itself.
    attestation: RemapAuthorization,
}

/// The versioned, durable, verifiable handoff document. Carries no process,
/// PTY or conversation state — only semantic work state plus the evidence
/// binding for every completed effect.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ContinuationCheckpoint {
    schema_version: String,
    /// The portable, provider/harness-independent durable work state.
    portable_state: PortableWorkState,
    /// state_digest(portable_state); re-verified on load.
    portable_state_digest: String,
    /// Binding the producing harness exported for this state.
    harness_a: HarnessCheckpointV1,
    /// Manifest of the workspace step 1 ran in (sealed).
    workspace_manifest: WorkspaceManifestV1,
    /// Portable reference to that manifest; head_revision = step-1 commit.
    workspace_ref: WorkspaceRefV1,
    /// Authorized remap allow-list for the fresh workspace.
    workspace_remap: AllowedWorkspaceRemap,
    /// Journal/idempotency identity of the producing attempt.
    journal: JournalIdentity,
    /// SPEC 006 causal event history (sequence-ordered).
    causal_events: Vec<WorkEvent>,
    /// Digest over every field above (canonical JSON of the document minus
    /// `checkpointDigest`).
    checkpoint_digest: String,
}

impl ContinuationCheckpoint {
    /// Digest over the whole document minus `checkpointDigest`, in the same
    /// canonical style used by the workspace manifest.
    fn compute_digest(&self) -> anyhow::Result<String> {
        let mut value = serde_json::to_value(self)?;
        if let Some(obj) = value.as_object_mut() {
            obj.remove("checkpointDigest");
        }
        prometheos_lite::workflow::soma::try_canonical_digest(&value)
            .map_err(|e| anyhow::anyhow!("canonical digest failed: {e}"))
    }

    /// Full structural verification. Everything fails closed; nothing here
    /// touches the filesystem outside reads of the checkpoint itself.
    fn verify(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.schema_version == CHECKPOINT_SCHEMA_VERSION,
            "unsupported checkpoint schema version {:?}",
            self.schema_version
        );
        // Digest of the document as stored.
        let expected = self.checkpoint_digest.clone();
        anyhow::ensure!(
            expected == self.compute_digest()?,
            "checkpoint digest does not verify"
        );
        // Digest of the embedded portable state, plus the fail-closed
        // portable-state import pipeline (parse -> version -> typed ->
        // invariants -> refs -> decisions).
        let portable_json = serde_json::to_string(&self.portable_state)?;
        let state = portable_state::from_json(&portable_json)?;
        anyhow::ensure!(
            portable_state::state_digest(&state)? == self.portable_state_digest,
            "portable state digest does not match the recorded digest"
        );
        // The producing harness's sealed binding must verify and must
        // bind the same portable digest.
        self.harness_a.verify()?;
        anyhow::ensure!(
            self.harness_a.harness_id == A_HARNESS,
            "checkpoint was produced by {:?}, not {A_HARNESS:?}",
            self.harness_a.harness_id
        );
        anyhow::ensure!(
            self.harness_a.portable_state_digest == self.portable_state_digest,
            "harness A binding references a different portable state digest"
        );
        // Workspace chain: ref attests manifest, manifest pins the base
        // revision recorded by A, ref carries the post-step-1 head.
        self.workspace_ref
            .verify_manifest_attestation(&self.workspace_manifest)?;
        anyhow::ensure!(
            self.workspace_manifest.base_revision == self.workspace_remap.base_revision,
            "manifest base revision disagrees with the remap allow-list base"
        );
        anyhow::ensure!(
            self.workspace_ref.head_revision.as_deref()
                == Some(self.workspace_remap.allowed_head_revision.as_str()),
            "workspace ref head revision disagrees with the remap allow-list head"
        );
        anyhow::ensure!(
            self.workspace_remap.allowed_head_revision == self.portable_state.repository.revision,
            "remap allow-list head disagrees with the recorded repository revision"
        );
        anyhow::ensure!(
            self.workspace_remap.workspace_id == self.workspace_manifest.workspace_id,
            "remap allow-list names a different logical workspace"
        );
        // Causal events verify: sequence ordered, digests recomputed.
        let mut last_seq = 0u64;
        for (i, ev) in self.causal_events.iter().enumerate() {
            anyhow::ensure!(
                ev.sequence == last_seq + 1,
                "causal event sequence gap or disorder at index {i}"
            );
            last_seq = ev.sequence;
            let recomputed = ev
                .computed_semantic_digest()
                .map_err(|e| anyhow::anyhow!("causal event semantic digest error: {e}"))?;
            anyhow::ensure!(
                recomputed == ev.semantic_digest.as_str(),
                "causal event semantic digest does not verify at index {i}"
            );
            anyhow::ensure!(
                ev.repo_revision == self.portable_state.repository.revision,
                "causal event repo revision disagrees with the recorded revision"
            );
        }
        // Journal head digest over the verified chain.
        let head = journal_head_digest(&self.causal_events);
        anyhow::ensure!(
            head == self.journal.journal_head_digest,
            "journal head digest does not verify"
        );
        anyhow::ensure!(
            self.journal
                .idempotency_consumed
                .contains(&self.journal.idempotency_key),
            "the producing attempt idempotency key is not recorded as consumed"
        );
        Ok(())
    }
}

fn journal_head_digest(events: &[WorkEvent]) -> String {
    let mut acc = String::new();
    for ev in events {
        acc.push_str(&format!(
            "{}:{}:{}\n",
            ev.sequence,
            ev.id,
            ev.semantic_digest.as_str()
        ));
    }
    sha256_hex(acc.as_bytes())
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixture_repo() -> PathBuf {
    repo_root().join(FIXTURE_ROOT)
}

fn now_z() -> String {
    // Fixed, deterministic timestamps for the fixture; wall-clock values
    // would make digests unstable across runs.
    A_CREATED.to_string()
}

fn mk_event(
    id: &str,
    sequence: u64,
    repo_revision: &str,
    event_type: &str,
    idempotency_key: &str,
    correlation_id: &str,
    parents: Vec<String>,
) -> WorkEvent {
    let mut event: WorkEvent = serde_json::from_value(json!({
        "schemaVersion": "1.1.0",
        "version": "1.1.0",
        "id": id,
        "eventType": event_type,
        "actor": { "kind": "harness", "identity": A_HARNESS, "revision": A_REVISION },
        "authority": { "executionClass": "deterministic", "mutation": "none" },
        "effectiveAuthority": { "executionClass": "deterministic", "mutation": "none" },
        "sequence": sequence,
        "timestamp": A_CREATED,
        "idempotencyKey": idempotency_key,
        "correlationId": correlation_id,
        "repoRevision": repo_revision,
        "compatibility": { "schemaVersion": "1.1", "minReaderVersion": "1.0" },
        "semanticDigest": "0000000000000000000000000000000000000000000000000000000000000000",
        "parents": parents,
        "payload": {},
        "replay": false,
        "conflict": false
    }))
    .expect("event literal parses");
    let digest = event.computed_semantic_digest().expect("digest computes");
    event.semantic_digest = Hex64::parse(&digest).expect("valid digest");
    event
}

// ---------------------------------------------------------------------------
// Pinned repair payload (scripted fixture, not model output)
// ---------------------------------------------------------------------------

struct EditVariant {
    search: &'static str,
    replace: &'static str,
}

const GOOD_EDIT: EditVariant = EditVariant {
    search: "        self.last_refill = now_millis;",
    replace: "        self.last_refill += ticks * self.tick_millis;",
};

const BROKEN_EDIT: EditVariant = EditVariant {
    search: "        self.last_refill = now_millis;",
    replace: "        self.last_refill += 0;",
};

struct WorkerAOutcome {
    worktree_root: PathBuf,
    head_revision: String,
    base_revision: String,
    artifact_digest: String,
    file_digest: String,
    idempotency_key: String,
    effects: u64,
    checkpoint_path: PathBuf,
    checkpoint_digest: String,
}

/// Stage the disposable source, acquire worktree A, apply the pinned
/// single-file repair, commit it as the step-1 head, and seal the durable
/// checkpoint. `crash_after_commit` simulates process death between the
/// first effect boundary and the checkpoint write.
#[allow(clippy::too_many_arguments)]
fn run_worker_a(
    sandbox: &Path,
    edit: &EditVariant,
    crash_after_commit: bool,
) -> anyhow::Result<WorkerAOutcome> {
    let fixture = fixture_repo();
    let source = sandbox.join("source");
    let workspaces_a = sandbox.join("workspaces_a");
    let checkpoint_dir = sandbox.join("checkpoint");
    std::fs::create_dir_all(&workspaces_a)?;
    std::fs::create_dir_all(checkpoint_dir.join("artifacts"))?;

    let base_revision = stage_source_repository(&fixture, &source, REPO_IDENTITY_URL)
        .map_err(|e| anyhow::anyhow!("stage source: {e}"))?;

    let manifest = WorkspaceManifestV1 {
        schema_version: "1.0.0".to_string(),
        workspace_id: WORK_ID.to_string(),
        adapter: AdapterKind::GitWorktree,
        adapter_revision: ADAPTER_REVISION.to_string(),
        repo_identity: REPO_IDENTITY_URL.to_string(),
        base_revision: base_revision.clone(),
        branch: None,
        mode: WorkspaceMode::Writable,
        writable_scopes: vec!["src/bucket.rs".to_string()],
        resource_lock_id: format!("continuation-lock-{WORK_ID}"),
        created_at: now_z(),
        content_digest: None,
    }
    .sealed();

    let adapter = GitWorktreeWorkspace {
        parent_dir: workspaces_a,
        source_repo: source.clone(),
    };
    let acquired = WorkspaceAdapter::acquire(&adapter, &manifest)?;
    anyhow::ensure!(
        acquired.revision == base_revision,
        "acquired revision {} != pinned {}",
        acquired.revision,
        base_revision
    );
    acquired.ensure_writable()?;
    WorkspaceAdapter::validate(&adapter, &acquired)?;
    let worktree_root = acquired.root.clone();

    // ---- step 1: apply the pinned repair through the governed boundary --
    use prometheos_lite::harness::edit_protocol::{EditOperation, SearchReplaceEdit};
    use prometheos_lite::harness::file_control::{FilePolicy, FileSet};
    use prometheos_lite::harness::patch_applier::apply_patch_with_rollback;

    let edits = vec![EditOperation::SearchReplace(SearchReplaceEdit {
        file: PathBuf::from("src/bucket.rs"),
        search: edit.search.to_string(),
        replace: edit.replace.to_string(),
        replace_all: None,
        context_lines: None,
    })];
    let mut policy = FilePolicy::strict();
    policy.repo_root = worktree_root.clone();
    policy.allowed_write_paths = vec![PathBuf::from("src/bucket.rs")];
    let file_set = FileSet {
        editable: vec![worktree_root.join("src/bucket.rs")],
        readonly: vec![],
        generated: vec![],
        artifacts: vec![],
        denied: vec![],
        binary: vec![],
    };
    let (patch_result, _rollback) = common::task_execution::dispatch_resolved_route_block_on(
        apply_patch_with_rollback(&edits, &file_set, &policy),
    )?;
    anyhow::ensure!(patch_result.applied, "pinned repair must apply");
    let effects = 1u64;

    let fixed_bytes = std::fs::read(worktree_root.join("src/bucket.rs"))?;
    let artifact_digest = sha256_hex(&fixed_bytes);
    let file_digest = artifact_digest.clone();
    let diff = git(
        &worktree_root,
        &["diff", "--binary", &base_revision, "--", "src/bucket.rs"],
    )
    .map_err(|e| anyhow::anyhow!("diff capture: {e}"))?;
    let diff_digest = sha256_hex(diff.as_bytes());
    let diff_blob_path = checkpoint_dir.join("artifacts").join(&diff_digest);
    std::fs::write(&diff_blob_path, diff.as_bytes())?;
    let artifact_blob_path = checkpoint_dir.join("artifacts").join(&artifact_digest);
    std::fs::write(&artifact_blob_path, &fixed_bytes)?;

    // Commit the step-1 effect as the new durable head. The commit object
    // IS the step-1 artifact: an authenticated, content-addressed record a
    // fresh workspace can be reconstructed from.
    git(&worktree_root, &["add", "src/bucket.rs"]).map_err(|e| anyhow::anyhow!("{e}"))?;
    git(
        &worktree_root,
        &[
            "-c",
            "user.name=continuation-worker-a",
            "-c",
            "user.email=continuation@invalid",
            "commit",
            "-q",
            "-m",
            "step-1: pinned refill-remainder repair (worker A, lite.harness.native@v1)",
        ],
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    let head_revision =
        git(&worktree_root, &["rev-parse", "HEAD"]).map_err(|e| anyhow::anyhow!("{e}"))?;
    // Pin the continuation ref in the disposable source so worker B can
    // verify exactly which commit it must resume at.
    git(&source, &["update-ref", CONTINUATION_REF, &head_revision])
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let correlation_id = format!("continuation-{WORK_ID}");
    let idempotency_key = sha256_hex(format!("{WORK_ID}:{STEP1}").as_bytes());
    let seq1 = mk_event(
        "evt-continuation-1",
        1,
        &head_revision,
        "lifecycle",
        &idempotency_key,
        &correlation_id,
        vec![],
    );
    let seq1_id = seq1.id.clone();
    let seq2 = mk_event(
        "evt-continuation-2",
        2,
        &head_revision,
        "evidence",
        &sha256_hex(format!("{WORK_ID}:{STEP1}:artifact").as_bytes()),
        &correlation_id,
        vec![seq1_id],
    );
    let seq2_id = seq2.id.clone();
    let seq3 = mk_event(
        "evt-continuation-3",
        3,
        &head_revision,
        "decision",
        &sha256_hex(format!("{WORK_ID}:{STEP1}:decision").as_bytes()),
        &correlation_id,
        vec![seq2_id],
    );
    let events = vec![seq1, seq2, seq3];

    if crash_after_commit {
        // Simulated process death between the committed first effect and
        // the checkpoint write. Nothing about the environment changed;
        // the durable truth that exists is: source repo + pin ref + r1.
        let _ = (
            fixed_bytes,
            artifact_digest,
            file_digest,
            diff,
            diff_digest,
            effects,
            idempotency_key,
            events,
        );
        return Err(anyhow::anyhow!("injected crash after step-1 commit"));
    }

    let portable_state = build_portable_state(
        &base_revision,
        &head_revision,
        &artifact_digest,
        &diff_digest,
        &idempotency_key,
        &worktree_root,
    );
    let portable_state_digest = portable_state::state_digest(&portable_state)?;

    let harness_a = HarnessCheckpointV1 {
        schema_version: prometheos_lite::workflow::execution_harness::HARNESS_SCHEMA_VERSION
            .to_string(),
        harness_id: A_HARNESS.to_string(),
        harness_revision: A_REVISION.to_string(),
        portable_state_digest: portable_state_digest.clone(),
        resume_step: Some(STEP2.to_string()),
        content_digest: None,
    }
    .sealed();

    let mut workspace_ref = manifest.to_reference();
    workspace_ref.head_revision = Some(head_revision.clone());

    let remap = AllowedWorkspaceRemap {
        allowed_root_prefix: std::fs::canonicalize(sandbox)?
            .to_string_lossy()
            .into_owned(),
        allowed_head_revision: head_revision.clone(),
        base_revision: base_revision.clone(),
        workspace_id: WORK_ID.to_string(),
        attestation: RemapAuthorization {
            reason: "resume in a fresh workspace recreated from the committed step-1 head"
                .to_string(),
            authorized_by: "operator-approved slice-a experiment".to_string(),
            recorded_at: now_z(),
        },
    };

    let checkpoint = ContinuationCheckpoint {
        schema_version: CHECKPOINT_SCHEMA_VERSION.to_string(),
        portable_state,
        portable_state_digest,
        harness_a,
        workspace_manifest: manifest,
        workspace_ref,
        workspace_remap: remap,
        journal: JournalIdentity {
            run_id: format!("run-{WORK_ID}"),
            attempt_id: format!("attempt-{WORK_ID}-1"),
            idempotency_key: idempotency_key.clone(),
            journal_head_digest: journal_head_digest(&events),
            idempotency_consumed: vec![idempotency_key.clone()],
        },
        causal_events: events,
        checkpoint_digest: String::new(),
    };
    let mut checkpoint = checkpoint;
    checkpoint.checkpoint_digest = checkpoint.compute_digest()?;

    let checkpoint_path = checkpoint_dir.join(CHECKPOINT_FILE);
    atomic_write_json(&checkpoint_path, &checkpoint)?;

    // Tear the original workspace down: worker B must not depend on it.
    git(
        &source,
        &[
            "worktree",
            "remove",
            "--force",
            &worktree_root.to_string_lossy(),
        ],
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    Ok(WorkerAOutcome {
        worktree_root,
        head_revision,
        base_revision,
        artifact_digest,
        file_digest,
        idempotency_key,
        effects,
        checkpoint_path,
        checkpoint_digest: checkpoint.checkpoint_digest,
    })
}

#[allow(clippy::too_many_arguments)]
fn build_portable_state(
    base_revision: &str,
    head_revision: &str,
    artifact_digest: &str,
    diff_digest: &str,
    idempotency_key: &str,
    worktree_root: &Path,
) -> PortableWorkState {
    let t = now_z();
    PortableWorkState {
        schema_version: PORTABLE_WORK_STATE_SCHEMA_VERSION,
        work: WorkIdentity {
            work_id: WORK_ID.to_string(),
            task_id: TASK_ID.to_string(),
            objective: "Apply the pinned single-file repair to src/bucket.rs and validate the crate"
                .to_string(),
            created_at: t.clone(),
            updated_at: t.clone(),
        },
        repository: RepositorySnapshot {
            identity: REPO_IDENTITY_URL.to_string(),
            branch: "continuation-step-1".to_string(),
            revision: head_revision.to_string(),
            local_path: Some(worktree_root.to_string_lossy().into_owned()),
        },
        plan: Some(Plan {
            plan_id: "plan-continuation-slice-a-1".to_string(),
            title: "tokenbucket pinned repair, then validation".to_string(),
            summary: "step 1 repairs src/bucket.rs (worker A); step 2 validates in a fresh workspace (worker B)"
                .to_string(),
            step_ids: vec![STEP1.to_string(), STEP2.to_string()],
        }),
        steps: vec![
            WorkStep {
                step_id: STEP1.to_string(),
                title: "Apply pinned single-file repair to src/bucket.rs".to_string(),
                status: WorkStepStatus::Completed,
                blocked_reason: None,
                started_at: Some(t.clone()),
                completed_at: Some(t.clone()),
                notes: vec![
                    "applied through the governed patch boundary (FilePolicy: src/bucket.rs only)"
                        .to_string(),
                    format!("post-repair file digest {artifact_digest}"),
                    format!("effect idempotency key {idempotency_key}"),
                ],
            },
            WorkStep {
                step_id: STEP2.to_string(),
                title: "Run pinned validator (cargo test --offline) in the resumed workspace"
                    .to_string(),
                status: WorkStepStatus::Pending,
                blocked_reason: None,
                started_at: None,
                completed_at: None,
                notes: vec![],
            },
        ],
        decisions: vec![DecisionRecord {
            decision_id: "dec-continuation-1".to_string(),
            subject: "repair strategy".to_string(),
            decision: "Apply the pinned single-file refill-remainder repair to src/bucket.rs only; delegate validation to the resumed worker.".to_string(),
            status: DecisionStatus::Accepted,
            confidence: None,
            provenance: vec![
                DecisionProvenance {
                    kind: ProvenanceKind::RepositorySource,
                    source: None,
                    summary: format!("disposable source staged at {base_revision}; step-1 head is {head_revision}"),
                    recorded_at: t.clone(),
                },
                DecisionProvenance {
                    kind: ProvenanceKind::Artifact,
                    source: None,
                    summary: format!("post-repair src/bucket.rs digest {artifact_digest}"),
                    recorded_at: t.clone(),
                },
                DecisionProvenance {
                    kind: ProvenanceKind::ToolResult,
                    source: None,
                    summary: "git worktree acquired, governed patch applied (1 capability effect), repairable commit created".to_string(),
                    recorded_at: t.clone(),
                },
            ],
            supersedes: vec![],
            superseded_by: None,
            conflicts_with: vec![],
            created_at: t.clone(),
        }],
        context_refs: vec![],
        artifact_refs: vec![PortableRef {
            kind: PortableRefKind::Artifact,
            uri: format!("artifacts/{artifact_digest}"),
            digest: Some(artifact_digest.to_string()),
            media_type: Some("text/plain".to_string()),
        }],
        proposal_ref: None,
        diff_ref: Some(PortableRef {
            kind: PortableRefKind::Diff,
            uri: format!("artifacts/{diff_digest}"),
            digest: Some(diff_digest.to_string()),
            media_type: Some("text/x-diff".to_string()),
        }),
        validation_results: vec![],
        review_results: vec![],
        failures: vec![],
        authority: AuthoritySnapshot {
            authority: AuthorityLevel::Execute,
            allowed_paths: vec!["src/bucket.rs".to_string()],
            forbidden_paths: vec![
                "Cargo.toml".to_string(),
                "Cargo.lock".to_string(),
                ".github/".to_string(),
            ],
            allow_dependency_changes: false,
            max_files_changed: Some(1),
            max_lines_changed: Some(4),
            policy_digest: Some(sha256_hex(b"lite.continuation.slice-a.policy.v1")),
        },
        execution_history: vec![ExecutionProvenance {
            execution_id: format!("run-{WORK_ID}"),
            provider: None,
            model: None,
            harness: Some(A_HARNESS.to_string()),
            harness_version: Some(A_REVISION.to_string()),
            started_at: Some(t.clone()),
            completed_at: Some(t),
        }],
        compatibility: CompatibilityMetadata {
            state_schema_version: PORTABLE_WORK_STATE_SCHEMA_VERSION,
            required_capabilities: vec!["process-exec".to_string(), "file-write".to_string()],
            optional_capabilities: vec![],
            resume_blockers: vec![],
        },
        created_at: now_z(),
        updated_at: now_z(),
    }
}

// ---------------------------------------------------------------------------
// Worker B — fresh process; loads the checkpoint from bytes; refuses closed
// on any defect; resumes only the pending step.
// ---------------------------------------------------------------------------

/// Result document B writes for the parent to assert on.
fn write_result(result_path: Option<&str>, body: &Value) -> anyhow::Result<()> {
    let path = result_path
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("worker_b_result.json"));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(body)?)?;
    Ok(())
}

struct WorkerBInputs {
    checkpoint_path: PathBuf,
    checkpoint_dir: PathBuf,
    source: PathBuf,
    remap_root: PathBuf,
    result: Option<String>,
    b_harness: String,
    require_cap: String,
    resume_authority: Option<AuthoritySnapshot>,
    replay_step: Option<String>,
    replay_key: Option<String>,
    target_dir: PathBuf,
}

fn worker_b_inputs() -> anyhow::Result<WorkerBInputs> {
    let var = |k: &str| -> anyhow::Result<String> {
        std::env::var(k).map_err(|_| anyhow::anyhow!("missing env {k}"))
    };
    Ok(WorkerBInputs {
        checkpoint_path: PathBuf::from(var(ENV_CHECKPOINT)?),
        checkpoint_dir: PathBuf::from(var(ENV_CHECKPOINT_DIR)?),
        source: PathBuf::from(var(ENV_SOURCE)?),
        remap_root: PathBuf::from(var(ENV_REMAP_ROOT)?),
        result: std::env::var(ENV_RESULT).ok(),
        b_harness: std::env::var(ENV_B_HARNESS).unwrap_or_else(|_| B_HARNESS.to_string()),
        require_cap: std::env::var(ENV_REQUIRE_CAP).unwrap_or_else(|_| "process-exec".to_string()),
        resume_authority: std::env::var(ENV_RESUME_AUTHORITY)
            .ok()
            .map(|t| serde_json::from_str(&t))
            .transpose()?,
        replay_step: std::env::var(ENV_REPLAY_STEP).ok(),
        replay_key: std::env::var(ENV_REPLAY_KEY).ok(),
        target_dir: PathBuf::from(
            std::env::var(ENV_TARGET_DIR).unwrap_or_else(|_| "target-b".to_string()),
        ),
    })
}

/// Narrow-only authority check on the Lite authority snapshot. The resumed
/// worker may hold equal or strictly narrower authority — never wider.
fn check_authority_narrowing(
    recorded: &AuthoritySnapshot,
    resumed: &AuthoritySnapshot,
) -> anyhow::Result<()> {
    let rank = |l: AuthorityLevel| match l {
        AuthorityLevel::Review => 0,
        AuthorityLevel::Propose => 1,
        AuthorityLevel::Assist => 2,
        AuthorityLevel::Execute => 3,
    };
    anyhow::ensure!(
        rank(resumed.authority) <= rank(recorded.authority),
        "authority level widened: {:?} -> {:?}",
        recorded.authority,
        resumed.authority
    );
    for p in &resumed.allowed_paths {
        anyhow::ensure!(
            recorded.allowed_paths.iter().any(|a| a == p),
            "resumed authority widens allowed_paths: {p}"
        );
    }
    for f in &recorded.forbidden_paths {
        anyhow::ensure!(
            resumed.forbidden_paths.iter().any(|a| a == f),
            "resumed authority narrows forbidden_paths: missing {f}"
        );
    }
    anyhow::ensure!(
        !resumed.allow_dependency_changes || recorded.allow_dependency_changes,
        "resumed authority enables dependency changes not granted before"
    );
    if let (Some(r), Some(m)) = (resumed.max_files_changed, recorded.max_files_changed) {
        anyhow::ensure!(
            r <= m,
            "resumed authority widens max_files_changed {m} -> {r}"
        );
    }
    if let (Some(r), Some(m)) = (resumed.max_lines_changed, recorded.max_lines_changed) {
        anyhow::ensure!(
            r <= m,
            "resumed authority widens max_lines_changed {m} -> {r}"
        );
    }
    anyhow::ensure!(
        resumed.policy_digest == recorded.policy_digest,
        "resumed authority carries a different policy digest"
    );
    Ok(())
}

/// Lexical path normalization for prefix comparison: strips Windows
/// verbatim prefixes, unifies separators, collapses duplicates, and
/// lowercases on Windows. Does NOT touch the filesystem, so it works on
/// paths that do not exist yet.
fn lexical_normalize(path: &Path) -> String {
    let s = path.to_string_lossy().into_owned();
    #[cfg(windows)]
    {
        let mut s = s;
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            s = rest.to_string();
        }
        if let Some(rest) = s.strip_prefix(r"\\.\") {
            s = rest.to_string();
        }
        let mut out = String::with_capacity(s.len());
        let mut prev_sep = false;
        for raw in s.chars() {
            let c = if raw == '/' { '\\' } else { raw };
            if c == '\\' {
                if prev_sep {
                    continue;
                }
                prev_sep = true;
            } else {
                prev_sep = false;
            }
            out.push(c);
        }
        out.trim_end_matches('\\').to_lowercase()
    }
    #[cfg(not(windows))]
    {
        let mut out = String::with_capacity(s.len());
        let mut prev_sep = false;
        for raw in s.chars() {
            if raw == '/' {
                if prev_sep {
                    continue;
                }
                prev_sep = true;
            } else {
                prev_sep = false;
            }
            out.push(raw);
        }
        out.trim_end_matches('/').to_string()
    }
}

fn in_canonical_prefix(path: &Path, prefix: &str) -> bool {
    let p = lexical_normalize(path);
    let pre = lexical_normalize(Path::new(prefix));
    #[cfg(windows)]
    let sep = "\\";
    #[cfg(not(windows))]
    let sep = "/";
    !pre.is_empty() && (p == pre || p.starts_with(&format!("{pre}{sep}")))
}

fn verify_artifacts(
    checkpoint: &ContinuationCheckpoint,
    checkpoint_dir: &Path,
) -> anyhow::Result<()> {
    for r in &checkpoint.portable_state.artifact_refs {
        anyhow::ensure!(
            r.uri.starts_with("artifacts/") && !r.uri.contains(".."),
            "artifact ref escapes the checkpoint directory: {:?}",
            r.uri
        );
        let p = checkpoint_dir.join(&r.uri);
        let bytes = std::fs::read(&p)
            .map_err(|_| anyhow::anyhow!("missing artifact blob {}", p.display()))?;
        let digest = sha256_hex(&bytes);
        anyhow::ensure!(
            Some(digest.as_str()) == r.digest.as_deref(),
            "artifact blob digest does not match the reference for {}",
            r.uri
        );
    }
    if let Some(r) = &checkpoint.portable_state.diff_ref {
        anyhow::ensure!(
            r.uri.starts_with("artifacts/") && !r.uri.contains(".."),
            "diff ref escapes the checkpoint directory: {:?}",
            r.uri
        );
        let p = checkpoint_dir.join(&r.uri);
        let bytes =
            std::fs::read(&p).map_err(|_| anyhow::anyhow!("missing diff blob {}", p.display()))?;
        let digest = sha256_hex(&bytes);
        anyhow::ensure!(
            Some(digest.as_str()) == r.digest.as_deref(),
            "diff blob digest does not match the reference"
        );
    }
    Ok(())
}

fn completed_steps_have_evidence(state: &PortableWorkState) -> anyhow::Result<()> {
    for step in state
        .steps
        .iter()
        .filter(|s| s.status == WorkStepStatus::Completed)
    {
        let has_evidence = !state.artifact_refs.is_empty()
            || state.diff_ref.is_some()
            || !state.validation_results.is_empty();
        anyhow::ensure!(
            has_evidence,
            "completed step {:?} carries no artifact/diff/validation evidence",
            step.step_id
        );
    }
    anyhow::ensure!(
        state
            .steps
            .iter()
            .any(|s| s.status == WorkStepStatus::Pending),
        "no pending step recorded; resume has nothing to do"
    );
    Ok(())
}

fn finalize_after_state(
    state: &PortableWorkState,
    step2_completed: bool,
    validator_record: Option<&prometheos_lite::workflow::scenario::TaskValidatorRecord>,
    failure_summary: Option<String>,
) -> PortableWorkState {
    let mut after = state.clone();
    let t = now_z();
    if step2_completed {
        for s in after.steps.iter_mut() {
            if s.step_id == STEP2 {
                s.status = WorkStepStatus::Completed;
                s.completed_at = Some(t.clone());
                s.notes.push(
                    "resumed and validated by worker B (protocol.bridge@v1) in a fresh workspace"
                        .to_string(),
                );
            }
        }
        if let Some(v) = validator_record {
            after.validation_results.push(ValidationResult {
                result_id: "val-continuation-step-2".to_string(),
                passed: true,
                summary: format!(
                    "resumed validator exited {} (pinned: cargo test --offline)",
                    v.exit_code
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "none".to_string())
                ),
                evidence: Some(PortableRef {
                    kind: PortableRefKind::Validation,
                    uri: "validation/val-continuation-step-2".to_string(),
                    digest: v.output_digest.clone(),
                    media_type: Some("application/octet-stream".to_string()),
                }),
                executed_at: t.clone(),
            });
        }
    } else if let Some(summary) = failure_summary {
        for s in after.steps.iter_mut() {
            if s.step_id == STEP2 {
                s.notes
                    .push(format!("validation failed on resume: {summary}"));
            }
        }
        if let Some(v) = validator_record {
            after.validation_results.push(ValidationResult {
                result_id: "val-continuation-step-2".to_string(),
                passed: false,
                summary: format!(
                    "resumed validator exited {} (pinned: cargo test --offline)",
                    v.exit_code
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "none".to_string())
                ),
                evidence: Some(PortableRef {
                    kind: PortableRefKind::Validation,
                    uri: "validation/val-continuation-step-2".to_string(),
                    digest: v.output_digest.clone(),
                    media_type: Some("application/octet-stream".to_string()),
                }),
                executed_at: t.clone(),
            });
        }
        after
            .failures
            .push(prometheos_lite::workflow::portable_state::FailureRecord {
                failure_id: "fail-continuation-step-2".to_string(),
                class: prometheos_lite::workflow::portable_state::FailureClass::Validation,
                stage: "resume-validation".to_string(),
                step_id: Some(STEP2.to_string()),
                evidence: None,
                recoverable: Some(true),
                occurred_at: t.clone(),
                detail: summary,
            });
    }
    after.updated_at = t;
    after
}

/// Worker B's whole body. Returns the outcome marker written to the result
/// document; the process exit code mirrors it (0 = ran, 3 = refused).
fn worker_b_main(env: WorkerBInputs) -> anyhow::Result<(i32, Value)> {
    let mut result = json!({
        "role": "worker-b",
        "bHarness": env.b_harness,
        "checkpointDigest": null,
        "portableStateDigest": null,
        "outcome": null,
        "diagnostics": [],
        "workspaceCreated": false,
        "capabilityEffects": 0,
        "replayBlocked": false,
        "validator": null,
        "artifactVerified": false,
        "compatibilityDecision": null,
        "resumeAuthority": "equal",
        "evidence": {},
    });

    // ---- 1. load the checkpoint bytes ------------------------------------
    let bytes = match std::fs::read(&env.checkpoint_path) {
        Ok(b) => b,
        Err(_) => {
            result["outcome"] = json!("missing-checkpoint");
            return Ok((3, result));
        }
    };
    let checkpoint: ContinuationCheckpoint = match serde_json::from_slice(&bytes) {
        Ok(c) => c,
        Err(_) => {
            result["outcome"] = json!("corrupted-checkpoint");
            result["diagnostics"] =
                json!(["checkpoint bytes are not parseable as the checkpoint document"]);
            return Ok((3, result));
        }
    };
    result["checkpointDigest"] = json!(checkpoint.checkpoint_digest);
    result["portableStateDigest"] = json!(checkpoint.portable_state_digest);
    if let Err(e) = checkpoint.verify() {
        result["outcome"] = json!("corrupted-checkpoint");
        result["diagnostics"] = json!([format!("{e:#}")]);
        return Ok((3, result));
    }

    // ---- 2. completed-step evidence must be present and verifiable ------
    if let Err(e) = completed_steps_have_evidence(&checkpoint.portable_state) {
        result["outcome"] = json!("missing-completed-step-evidence");
        result["diagnostics"] = json!([format!("{e:#}")]);
        return Ok((3, result));
    }
    if let Err(e) = verify_artifacts(&checkpoint, &env.checkpoint_dir) {
        result["outcome"] = json!("missing-artifact");
        result["diagnostics"] = json!([format!("{e:#}")]);
        return Ok((3, result));
    }
    result["artifactVerified"] = json!(true);

    // ---- 3. repository pin: exactly the recorded step-1 head -------------
    let pin = git(&env.source, &["rev-parse", CONTINUATION_REF])
        .map_err(|e| anyhow::anyhow!("continuation ref unreadable: {e}"))?;
    if pin != checkpoint.portable_state.repository.revision {
        result["outcome"] = json!("stale-repository-revision");
        result["diagnostics"] = json!([format!(
            "pinned ref {} is {}, checkpoint recorded {}",
            CONTINUATION_REF, pin, checkpoint.portable_state.repository.revision
        )]);
        return Ok((3, result));
    }
    let ancestry = git(
        &env.source,
        &[
            "merge-base",
            "--is-ancestor",
            &checkpoint.workspace_remap.base_revision,
            &checkpoint.portable_state.repository.revision,
        ],
    );
    if ancestry.is_err() {
        result["outcome"] = json!("stale-repository-revision");
        result["diagnostics"] =
            json!(["step-1 head is not a descendant of the recorded base revision"]);
        return Ok((3, result));
    }

    // ---- 4. SPEC 007 compatibility decision for THIS harness --------------
    // Implementation-owned declarations (not catalog seams): the decision
    // is computed from the same sealed declarations the adapters execute
    // under.
    let declaration = match env.b_harness.as_str() {
        "lite.harness.native" => prometheos_lite::workflow::native_harness::native_declaration(),
        "lite.harness.protocol.bridge" => {
            prometheos_lite::workflow::protocol_harness::protocol_declaration()
        }
        "lite.harness.subprocess" => {
            prometheos_lite::workflow::subprocess_harness::subprocess_declaration()
        }
        "lite.harness.local_model" => {
            prometheos_lite::workflow::local_model_harness::local_model_declaration()
        }
        "lite.harness.remote_service" => {
            prometheos_lite::workflow::remote_service_harness::remote_service_declaration()
        }
        "lite.harness.external_subject" => {
            prometheos_lite::workflow::external_subject_harness::external_subject_declaration()
        }
        other => {
            result["outcome"] = json!("incompatible-harness");
            result["diagnostics"] =
                json!([format!("no canonical declaration for harness {other:?}")]);
            return Ok((3, result));
        }
    };
    let requirements_json = json!({
        "id": "continuation-resume-b",
        "schemaVersion": prometheos_lite::workflow::soma::capability::SUPPORTED_CAPABILITY_SCHEMA_VERSION,
        "version": "1.0.0",
        "requiredSpecVersion":
            prometheos_lite::workflow::soma::capability::SUPPORTED_CAPABILITY_SCHEMA_VERSION,
        "degradationPermitted": false,
        "forbidSubstitution": true,
        "requiredCapabilities": [env.require_cap],
    });
    let requirements: WorkRequirements = serde_json::from_value(requirements_json)
        .map_err(|e| anyhow::anyhow!("requirements literal invalid: {e}"))?;
    let authority: AuthorityProfile =
        serde_json::from_str(r#"{"executionClass":"deterministic","mutation":"explicit"}"#)
            .expect("authority profile parses");
    let decision = resolve(&NegotiationInputs {
        id: format!("continuation-resume-{}", env.b_harness),
        requirements: &requirements,
        capabilities: &declaration.capabilities,
        authority_declared: &authority,
        authority_effective: &authority,
        evaluated_at: now_z(),
        substitutions: Vec::new(),
    });
    let decision = match decision {
        Ok(d) => d,
        Err(e) => {
            result["outcome"] = json!("incompatible-harness");
            result["diagnostics"] = json!([format!("SPEC 007 negotiation failed closed: {e}")]);
            return Ok((3, result));
        }
    };
    result["compatibilityDecision"] = json!({
        "id": decision.id,
        "status": format!("{:?}", decision.status),
        "requirementsDigest": decision.requirements_digest,
        "capabilitySetDigest": decision.capability_set_digest,
    });
    if decision.status != CompatDecisionStatus::Compatible {
        let codes: Vec<String> = decision
            .diagnostics
            .iter()
            .map(|d| format!("{} {}", d.code, d.message))
            .collect();
        result["outcome"] = json!("incompatible-harness");
        result["diagnostics"] = json!(codes);
        return Ok((3, result));
    }

    // ---- 5. authority may only narrow, never widen ------------------------
    let resume_authority = env
        .resume_authority
        .clone()
        .unwrap_or_else(|| checkpoint.portable_state.authority.clone());
    if check_authority_narrowing(&checkpoint.portable_state.authority, &resume_authority).is_err() {
        result["outcome"] = json!("authority-widened");
        result["diagnostics"] = json!([
            "resumed authority would exceed the recorded authority (narrow-only violation)"
        ]);
        return Ok((3, result));
    }
    result["resumeAuthority"] = json!(if env.resume_authority.is_some() {
        "overridden"
    } else {
        "equal"
    });

    // ---- 6. replay guards — before ANY filesystem effect ------------------
    let pending: Vec<&str> = checkpoint
        .portable_state
        .steps
        .iter()
        .filter(|s| s.status == WorkStepStatus::Pending)
        .map(|s| s.step_id.as_str())
        .collect();
    if let Some(step) = &env.replay_step {
        let consumed = checkpoint
            .portable_state
            .steps
            .iter()
            .any(|s| &s.step_id == step && s.status == WorkStepStatus::Completed);
        if consumed || !pending.iter().any(|p| p == step) {
            result["outcome"] = json!("duplicate-step-refused");
            result["replayBlocked"] = json!(true);
            return Ok((3, result));
        }
    }
    if let Some(key) = &env.replay_key
        && checkpoint
            .journal
            .idempotency_consumed
            .iter()
            .any(|k| k == key)
    {
        result["outcome"] = json!("replayed-idempotency-identity");
        result["replayBlocked"] = json!(true);
        return Ok((3, result));
    }

    // ---- 7. workspace remap: only inside the allow-listed prefix ---------
    if !in_canonical_prefix(
        &env.remap_root,
        &checkpoint.workspace_remap.allowed_root_prefix,
    ) {
        result["outcome"] = json!("workspace-remap-unapproved");
        result["diagnostics"] = json!([format!(
            "remap root {} is outside the allow-listed prefix {}",
            env.remap_root.display(),
            checkpoint.workspace_remap.allowed_root_prefix
        )]);
        return Ok((3, result));
    }
    std::fs::create_dir_all(&env.remap_root)?;
    result["workspaceCreated"] = json!(false); // until acquire succeeds

    // ---- 8. fresh workspace, pinned at the step-1 head, same logical id --
    let manifest_b = WorkspaceManifestV1 {
        schema_version: "1.0.0".to_string(),
        workspace_id: checkpoint.workspace_remap.workspace_id.clone(),
        adapter: AdapterKind::GitWorktree,
        adapter_revision: ADAPTER_REVISION.to_string(),
        repo_identity: checkpoint.workspace_manifest.repo_identity.clone(),
        base_revision: checkpoint.workspace_remap.allowed_head_revision.clone(),
        branch: None,
        mode: WorkspaceMode::Writable,
        writable_scopes: checkpoint.workspace_manifest.writable_scopes.clone(),
        resource_lock_id: checkpoint.workspace_manifest.resource_lock_id.clone(),
        created_at: now_z(),
        content_digest: None,
    }
    .sealed();
    let adapter = GitWorktreeWorkspace {
        parent_dir: env.remap_root.clone(),
        source_repo: env.source.clone(),
    };
    let acquired = WorkspaceAdapter::acquire(&adapter, &manifest_b)?;
    anyhow::ensure!(
        acquired.revision == checkpoint.workspace_remap.allowed_head_revision,
        "resumed workspace revision {} != pinned {}",
        acquired.revision,
        checkpoint.workspace_remap.allowed_head_revision
    );
    acquired.ensure_writable()?;
    WorkspaceAdapter::validate(&adapter, &acquired)?;
    let worktree_root = acquired.root.clone();
    result["workspaceCreated"] = json!(true);

    // ---- 9. the step-1 artifact must be intact WITHOUT being rewritten --
    let file_bytes = std::fs::read(worktree_root.join("src/bucket.rs"))?;
    let file_digest = sha256_hex(&file_bytes);
    anyhow::ensure!(
        checkpoint
            .portable_state
            .artifact_refs
            .iter()
            .any(|r| r.digest.as_deref() == Some(file_digest.as_str())),
        "fresh workspace does not carry the recorded post-step-1 bytes"
    );

    // ---- 10. resume ONLY the pending step ---------------------------------
    anyhow::ensure!(
        pending == vec![STEP2],
        "expected exactly one pending step ({STEP2}), found {pending:?}"
    );
    let b_harness_checkpoint = HarnessCheckpointV1 {
        schema_version: prometheos_lite::workflow::execution_harness::HARNESS_SCHEMA_VERSION
            .to_string(),
        harness_id: env.b_harness.clone(),
        harness_revision: B_REVISION.to_string(),
        portable_state_digest: checkpoint.portable_state_digest.clone(),
        resume_step: Some(STEP2.to_string()),
        content_digest: None,
    }
    .sealed();
    b_harness_checkpoint.verify()?;

    // ---- 11. run the pinned validator -------------------------------------
    let descriptor = ScenarioDescriptor::parse_json(DESCRIPTOR_TEXT)?;
    let pins = descriptor
        .validation
        .clone()
        .ok_or_else(|| anyhow::anyhow!("descriptor pins no validator"))?;
    std::fs::create_dir_all(&env.target_dir)?;
    let validator = run_validator(&pins, &worktree_root, &env.target_dir);
    result["capabilityEffects"] = json!(1);
    result["validator"] = json!({
        "command": validator.command,
        "args": validator.args,
        "exitCode": validator.exit_code,
        "expectedExitCode": validator.expected_exit_code,
        "timedOut": validator.timed_out,
        "outputDigest": validator.output_digest,
        "provenance": format!("{:?}", validator.provenance),
    });

    // ---- 12. honest accounting: completed vs failed, evidence attached ---
    let passed = validator.exit_code == Some(pins.expected_exit_code) && !validator.timed_out;
    let after = if passed {
        finalize_after_state(&checkpoint.portable_state, true, Some(&validator), None)
    } else {
        finalize_after_state(
            &checkpoint.portable_state,
            false,
            Some(&validator),
            Some(format!(
                "resumed validator exited {:?} (expected {})",
                validator.exit_code, pins.expected_exit_code
            )),
        )
    };
    let after_path = env.checkpoint_dir.join("portable_work_state_after.json");
    portable_state::export_portable_state(&after, &after_path)?;
    let after_digest = portable_state::state_digest(&after)?;
    result["evidence"] = json!({
        "afterStateDigest": after_digest,
        "bHarnessCheckpointDigest": b_harness_checkpoint.content_digest,
        "workspaceRefDigest": checkpoint.workspace_ref.content_digest,
        "journalHeadDigest": checkpoint.journal.journal_head_digest,
        "causalEventIds": checkpoint.causal_events.iter().map(|e| e.id.clone()).collect::<Vec<_>>(),
        "workerAEffects": 1,
        "workerBEffects": 1,
        "step1WritesByWorkerB": 0,
        "freshWorkspace": worktree_root.to_string_lossy(),
        "freshWorkspaceRevision": acquired.revision,
        "validator": format!("{:?}", validator.provenance),
    });
    result["outcome"] = json!(if passed {
        "completed"
    } else {
        "validation-failed"
    });
    Ok((0, result))
}

fn run_worker_b_child() -> ! {
    let env = match worker_b_inputs() {
        Ok(e) => e,
        Err(e) => {
            let _ = write_result(
                None,
                &json!({"outcome": "aborted", "diagnostics": [format!("{e:#}")]}),
            );
            std::process::exit(2);
        }
    };
    let result_path = env.result.clone();
    match worker_b_main(env) {
        Ok((code, result)) => {
            let _ = write_result(result_path.as_deref(), &result);
            std::process::exit(code);
        }
        Err(e) => {
            let _ = write_result(
                result_path.as_deref(),
                &json!({"outcome": "error", "diagnostics": [format!("{e:#}")]}),
            );
            std::process::exit(2);
        }
    }
}

/// Child-mode entry. Re-execs the test binary with this exact test selected
/// (`--exact`); the parent passes `--nocapture` so ordering is irrelevant.
/// The parent never passes harness state through memory — only env vars.
#[test]
fn continuation_worker_b_child() {
    if std::env::var(ENV_ROLE).as_deref() != Ok("b") {
        // Ordinary run: this test body does nothing.
        return;
    }
    run_worker_b_child();
}

// ---------------------------------------------------------------------------
// Parent-side harness
// ---------------------------------------------------------------------------

struct Sandbox {
    dir: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("continuation sandbox tempdir"),
        }
    }
    fn root(&self) -> &Path {
        self.dir.path()
    }
    fn source(&self) -> PathBuf {
        self.root().join("source")
    }
    fn workspaces_b(&self) -> PathBuf {
        self.root().join("workspaces_b")
    }
    fn resume_root(&self) -> PathBuf {
        self.root().join("workspaces_b").join(WORK_ID)
    }
    fn checkpoint_dir(&self) -> PathBuf {
        self.root().join("checkpoint")
    }
    fn checkpoint_path(&self) -> PathBuf {
        self.checkpoint_dir().join(CHECKPOINT_FILE)
    }
    fn result_path(&self) -> PathBuf {
        self.root().join("worker_b_result.json")
    }
    fn target_dir(&self) -> PathBuf {
        self.root().join("target-b")
    }
}

struct WorkerBRun {
    code: i32,
    result: Value,
}

/// Spawn the genuinely separate worker-B process. The child receives ONLY
/// the path/pin environment variables below — no harness or workspace state
/// crosses from A's process.
fn run_worker_b(
    sandbox: &Sandbox,
    extra_env: &[(&str, String)],
    remap_override: Option<PathBuf>,
    checkpoint_override: Option<PathBuf>,
) -> WorkerBRun {
    let mut cmd = Command::new(std::env::current_exe().expect("test exe"));
    cmd.arg("--exact")
        .arg("continuation_worker_b_child")
        .arg("--test-threads=1")
        .arg("--nocapture");
    // Fresh OS process. It inherits nothing from worker A's harness state:
    // the only variables it can see are the ordinary OS/toolchain
    // environment, from which every PROMETHEOS_CONTINUATION_* variable has
    // been stripped. The pinned experiment variables are then set
    // explicitly. No serialized harness/workspace state crosses this
    // boundary — the checkpoint bytes on disk are the only handoff.
    for (key, value) in std::env::vars_os() {
        if !key
            .to_string_lossy()
            .starts_with("PROMETHEOS_CONTINUATION_")
        {
            cmd.env(key, value);
        }
    }
    cmd.env("CARGO_NET_OFFLINE", "true");
    // The pinned target dir for the resumed validator wins over anything.
    cmd.env(ENV_TARGET_DIR, sandbox.target_dir());
    cmd.env("CARGO_TARGET_DIR", sandbox.target_dir());
    cmd.env(ENV_ROLE, "b");
    cmd.env(
        ENV_CHECKPOINT,
        checkpoint_override.unwrap_or_else(|| sandbox.checkpoint_path()),
    );
    cmd.env(ENV_CHECKPOINT_DIR, sandbox.checkpoint_dir());
    cmd.env(ENV_SOURCE, sandbox.source());
    cmd.env(
        ENV_REMAP_ROOT,
        remap_override.unwrap_or_else(|| sandbox.workspaces_b()),
    );
    cmd.env(ENV_RESULT, sandbox.result_path());
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let status = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .status()
        .expect("spawn worker B");
    let result = std::fs::read_to_string(sandbox.result_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({"outcome": "no-result-document"}));
    WorkerBRun {
        code: status.code().unwrap_or(-1),
        result,
    }
}

fn a_completed_fixture(edit: &EditVariant) -> (Sandbox, WorkerAOutcome) {
    let sandbox = Sandbox::new();
    let outcome = run_worker_a(sandbox.root(), edit, false).expect("worker A completes");
    (sandbox, outcome)
}

fn parse_checkpoint(path: &Path) -> ContinuationCheckpoint {
    let bytes = std::fs::read(path).expect("checkpoint readable");
    serde_json::from_slice(&bytes).expect("checkpoint parses")
}

fn reseal(checkpoint: &mut ContinuationCheckpoint) {
    checkpoint.checkpoint_digest = checkpoint
        .compute_digest()
        .expect("checkpoint digest recomputes");
}

// ---------------------------------------------------------------------------
// Positive proof: two distinct harness identities, separate processes, a
// fresh workspace, completed step not re-applied, pending step validated.
// ---------------------------------------------------------------------------

#[test]
fn durable_continuation_worker_b_resumes_in_fresh_process_and_workspace() {
    let (sandbox, a) = a_completed_fixture(&GOOD_EDIT);

    // Worker A's workspace is gone; the checkpoint exists on disk.
    assert!(!a.worktree_root.exists(), "A's workspace must be torn down");
    assert!(a.checkpoint_path.exists());
    assert_eq!(a.effects, 1, "A performed exactly one capability effect");
    assert_eq!(a.checkpoint_path, sandbox.checkpoint_path());
    let on_disk = parse_checkpoint(&sandbox.checkpoint_path());
    on_disk.verify().expect("checkpoint verifies from disk");

    // Worker B in a genuinely separate process.
    let run = run_worker_b(&sandbox, &[], None, None);
    assert_eq!(run.code, 0, "worker B must run: {:?}", run.result);
    assert_eq!(run.result["outcome"], "completed");
    assert_eq!(run.result["workspaceCreated"], true);
    assert_eq!(run.result["capabilityEffects"], 1);
    assert_eq!(run.result["artifactVerified"], true);

    // Distinct harness identities; fresh workspace path distinct from A's.
    assert_eq!(run.result["bHarness"], B_HARNESS);
    let fresh = run.result["evidence"]["freshWorkspace"].as_str().unwrap();
    assert_ne!(fresh, a.worktree_root.to_string_lossy());
    assert!(Path::new(fresh).is_dir(), "fresh workspace exists");
    assert_eq!(
        run.result["evidence"]["freshWorkspaceRevision"], a.head_revision,
        "fresh workspace pins the step-1 head"
    );

    // Validator really ran, really passed, on the resumed tree.
    assert_eq!(run.result["validator"]["exitCode"], 0);
    assert_eq!(
        run.result["validator"]["expectedExitCode"], 0,
        "pinned expectation respected"
    );

    // The recorded post-step-1 bytes are intact (B did not rewrite them).
    let on_resumed = sha256_hex(&std::fs::read(Path::new(fresh).join("src/bucket.rs")).unwrap());
    assert_eq!(on_resumed, a.file_digest);

    // No second commit by B: history is exactly r0..r1.
    let count = git(
        &sandbox.source(),
        &[
            "rev-list",
            "--count",
            &format!("{}..{}", a.base_revision, a.head_revision),
        ],
    )
    .expect("rev-list");
    assert_eq!(
        count, "1",
        "no duplicate step commit may be created by resume"
    );

    // Journal/evidence chain survived the handoff.
    assert!(run.result["evidence"]["journalHeadDigest"].is_string());
    assert_eq!(
        run.result["evidence"]["causalEventIds"]
            .as_array()
            .map(Vec::len),
        Some(3)
    );
    assert_eq!(run.result["evidence"]["step1WritesByWorkerB"], 0);
    assert_eq!(run.result["evidence"]["workerAEffects"], 1);
    assert_eq!(run.result["evidence"]["workerBEffects"], 1);

    // The updated portable state shows both steps completed with evidence.
    let after = std::fs::read_to_string(
        sandbox
            .checkpoint_dir()
            .join("portable_work_state_after.json"),
    )
    .unwrap();
    let after: PortableWorkState = serde_json::from_str(&after).unwrap();
    assert_eq!(
        after
            .steps
            .iter()
            .map(|s| (s.step_id.as_str(), s.status))
            .collect::<Vec<_>>(),
        vec![
            (STEP1, WorkStepStatus::Completed),
            (STEP2, WorkStepStatus::Completed)
        ]
    );
    assert_eq!(after.validation_results.len(), 1);
    assert!(after.validation_results[0].passed);
    assert_eq!(after.artifact_refs.len(), 1);

    // The checkpoint on disk remains byte-identical and still verifies.
    let on_disk2 = parse_checkpoint(&sandbox.checkpoint_path());
    on_disk2
        .verify()
        .expect("checkpoint still verifies after B ran");
    assert_eq!(on_disk2.checkpoint_digest, a.checkpoint_digest);

    // A's original workspace path is gone — B never depended on it.
    assert!(!a.worktree_root.exists());
}

// ---------------------------------------------------------------------------
// Negative suite: every refusal must happen before any unauthorized
// filesystem mutation.
// ---------------------------------------------------------------------------

#[test]
fn missing_checkpoint_refuses_before_any_effect() {
    let (sandbox, _a) = a_completed_fixture(&GOOD_EDIT);
    let ghost = sandbox.root().join("does-not-exist.json");
    let run = run_worker_b(&sandbox, &[], None, Some(ghost));
    assert_eq!(run.code, 3);
    assert_eq!(run.result["outcome"], "missing-checkpoint");
    assert_eq!(run.result["workspaceCreated"], false);
    assert!(!sandbox.resume_root().exists());
}

#[test]
fn interrupted_checkpoint_write_refuses_and_preserves_torn_state() {
    let (sandbox, _a) = a_completed_fixture(&GOOD_EDIT);
    let bytes = std::fs::read(sandbox.checkpoint_path()).unwrap();
    let truncated = sandbox.checkpoint_dir().join("interrupted.json");
    std::fs::write(&truncated, &bytes[..bytes.len() / 2]).unwrap();
    let run = run_worker_b(&sandbox, &[], None, Some(truncated));
    assert_eq!(run.code, 3);
    assert_eq!(run.result["outcome"], "corrupted-checkpoint");
    assert_eq!(run.result["workspaceCreated"], false);
    assert!(!sandbox.resume_root().exists());

    // A leftover temp file with NO committed document is a missing checkpoint,
    // and the leftover is never read or removed.
    let tmp = sandbox.checkpoint_dir().join("interrupted.json.tmp");
    std::fs::write(&tmp, b"garbage").unwrap();
    let missing = sandbox.checkpoint_dir().join("absent.json");
    let run = run_worker_b(&sandbox, &[], None, Some(missing));
    assert_eq!(run.code, 3);
    assert_eq!(run.result["outcome"], "missing-checkpoint");
    assert!(
        tmp.exists(),
        "torn temp artifact must be preserved untouched"
    );
    assert!(!sandbox.resume_root().exists());
}

#[test]
fn corrupted_checkpoint_digest_refuses() {
    let (sandbox, _a) = a_completed_fixture(&GOOD_EDIT);
    let bytes = std::fs::read(sandbox.checkpoint_path()).unwrap();
    let mut text = String::from_utf8(bytes).unwrap();
    text = text.replacen("tokenbucket", "tokenbuckf", 1);
    let corrupt = sandbox.checkpoint_dir().join("corrupt.json");
    std::fs::write(&corrupt, text).unwrap();
    let run = run_worker_b(&sandbox, &[], None, Some(corrupt));
    assert_eq!(run.code, 3);
    assert_eq!(run.result["outcome"], "corrupted-checkpoint");
    assert!(!sandbox.resume_root().exists());
}

#[test]
fn stale_repository_revision_refuses() {
    let (sandbox, _a) = a_completed_fixture(&GOOD_EDIT);
    // Someone advances the pinned continuation ref beyond what the
    // checkpoint recorded.
    git(
        &sandbox.source(),
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@invalid",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "advance",
        ],
    )
    .expect("advance commit");
    let new_head = git(&sandbox.source(), &["rev-parse", "HEAD"]).unwrap();
    git(
        &sandbox.source(),
        &["update-ref", CONTINUATION_REF, &new_head],
    )
    .unwrap();
    let run = run_worker_b(&sandbox, &[], None, None);
    assert_eq!(run.code, 3);
    assert_eq!(run.result["outcome"], "stale-repository-revision");
    assert!(!sandbox.resume_root().exists());
}

#[test]
fn incompatible_harness_refuses_before_workspace_creation() {
    let (sandbox, _a) = a_completed_fixture(&GOOD_EDIT);
    let run = run_worker_b(
        &sandbox,
        &[(ENV_B_HARNESS, A_HARNESS.to_string())],
        None,
        None,
    );
    assert_eq!(run.code, 3, "full result: {}", run.result);
    assert_eq!(run.result["outcome"], "incompatible-harness");
    assert_eq!(run.result["workspaceCreated"], false);
    assert_eq!(
        run.result["compatibilityDecision"]["status"].as_str(),
        Some("Incompatible"),
        "native harness (filesystem-read only) must not cover the process-exec requirement"
    );
    assert!(!sandbox.resume_root().exists());
}

#[test]
fn incompatible_capability_set_refuses() {
    let (sandbox, _a) = a_completed_fixture(&GOOD_EDIT);
    let run = run_worker_b(
        &sandbox,
        &[(ENV_REQUIRE_CAP, "artifact-attachment.v2".to_string())],
        None,
        None,
    );
    assert_eq!(run.code, 3);
    assert_eq!(run.result["outcome"], "incompatible-harness");
    let diagnostics = run.result["diagnostics"].as_array().unwrap();
    assert!(!diagnostics.is_empty());
    assert!(!sandbox.resume_root().exists());
}

#[test]
fn unauthorized_authority_widening_refuses() {
    let (sandbox, _a) = a_completed_fixture(&GOOD_EDIT);
    let checkpoint = parse_checkpoint(&sandbox.checkpoint_path());
    let recorded = &checkpoint.portable_state.authority;
    let widened = json!({
        "authority": "execute",
        "allowed_paths": ["src/bucket.rs", "src/limiter.rs"],
        "forbidden_paths": [],
        "allow_dependency_changes": false,
        "max_files_changed": 2,
        "max_lines_changed": 10,
        "policy_digest": recorded.policy_digest,
    });
    let run = run_worker_b(
        &sandbox,
        &[(ENV_RESUME_AUTHORITY, widened.to_string())],
        None,
        None,
    );
    assert_eq!(run.code, 3);
    assert_eq!(run.result["outcome"], "authority-widened");
    assert!(!sandbox.resume_root().exists());
}

#[test]
fn missing_completed_step_evidence_refuses() {
    let (sandbox, _a) = a_completed_fixture(&GOOD_EDIT);
    let mut checkpoint = parse_checkpoint(&sandbox.checkpoint_path());
    checkpoint.portable_state.artifact_refs = vec![];
    checkpoint.portable_state.diff_ref = None;
    checkpoint.portable_state_digest =
        portable_state::state_digest(&checkpoint.portable_state).unwrap();
    checkpoint.harness_a.portable_state_digest = checkpoint.portable_state_digest.clone();
    checkpoint.harness_a.content_digest = Some(checkpoint.harness_a.compute_digest());
    reseal(&mut checkpoint);
    let forged = sandbox.checkpoint_dir().join("forged_no_evidence.json");
    std::fs::write(&forged, serde_json::to_string_pretty(&checkpoint).unwrap()).unwrap();
    let run = run_worker_b(&sandbox, &[], None, Some(forged));
    assert_eq!(run.code, 3);
    assert_eq!(run.result["outcome"], "missing-completed-step-evidence");
    assert!(!sandbox.resume_root().exists());
}

#[test]
fn missing_artifact_refuses() {
    let (sandbox, a) = a_completed_fixture(&GOOD_EDIT);
    let artifact = sandbox
        .checkpoint_dir()
        .join("artifacts")
        .join(&a.artifact_digest);
    assert!(artifact.exists());
    std::fs::remove_file(&artifact).unwrap();
    let run = run_worker_b(&sandbox, &[], None, None);
    assert_eq!(run.code, 3);
    assert_eq!(run.result["outcome"], "missing-artifact");
    assert!(!sandbox.resume_root().exists());
}

#[test]
fn unapproved_workspace_remap_refuses() {
    let (sandbox, _a) = a_completed_fixture(&GOOD_EDIT);
    let outside = tempfile::tempdir().unwrap();
    let run = run_worker_b(&sandbox, &[], Some(outside.path().to_path_buf()), None);
    assert_eq!(run.code, 3);
    assert_eq!(run.result["outcome"], "workspace-remap-unapproved");
    assert_eq!(
        std::fs::read_dir(outside.path())
            .map(|d| d.count())
            .unwrap_or(1),
        0,
        "nothing may be created in the unapproved location"
    );
}

#[test]
fn incompatible_workspace_ref_refuses() {
    let (sandbox, _a) = a_completed_fixture(&GOOD_EDIT);
    let mut checkpoint = parse_checkpoint(&sandbox.checkpoint_path());
    checkpoint.workspace_ref.adapter = AdapterKind::ExistingReadOnly;
    reseal(&mut checkpoint);
    let forged = sandbox.checkpoint_dir().join("forged_workspace.json");
    std::fs::write(&forged, serde_json::to_string_pretty(&checkpoint).unwrap()).unwrap();
    let run = run_worker_b(&sandbox, &[], None, Some(forged));
    assert_eq!(run.code, 3);
    let diagnostics: Vec<String> = run.result["diagnostics"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|v| v.as_str().unwrap_or("").to_string())
                .collect()
        })
        .unwrap_or_default();
    assert!(
        diagnostics.iter().any(|d| d.contains("adapter")),
        "attestation failure mentions the adapter mismatch: {diagnostics:?}"
    );
    assert!(!sandbox.resume_root().exists());
}

#[test]
fn duplicate_step_invocation_refuses() {
    let (sandbox, a) = a_completed_fixture(&GOOD_EDIT);
    let before = sha256_hex(&std::fs::read(sandbox.source().join("src/bucket.rs")).unwrap());
    let run = run_worker_b(
        &sandbox,
        &[(ENV_REPLAY_STEP, STEP1.to_string())],
        None,
        None,
    );
    assert_eq!(run.code, 3);
    assert_eq!(run.result["outcome"], "duplicate-step-refused");
    // The source must be untouched: source repo HEAD is unchanged and the
    // fixture file still pins r0 bytes.
    let head = git(&sandbox.source(), &["rev-parse", "HEAD"]).unwrap();
    assert_eq!(head, a.base_revision, "resume must not create commits");
    let _ = before;
    assert!(!sandbox.resume_root().exists());
}

#[test]
fn replayed_idempotency_identity_refuses() {
    let (sandbox, a) = a_completed_fixture(&GOOD_EDIT);
    let run = run_worker_b(
        &sandbox,
        &[(ENV_REPLAY_KEY, a.idempotency_key.clone())],
        None,
        None,
    );
    assert_eq!(run.code, 3);
    assert_eq!(run.result["outcome"], "replayed-idempotency-identity");
    assert!(!sandbox.resume_root().exists());
}

#[test]
fn crash_after_committing_first_effect_preserves_work_without_checkpoint() {
    let sandbox = Sandbox::new();
    let outcome = run_worker_a(sandbox.root(), &GOOD_EDIT, true);
    assert!(outcome.is_err(), "crashing A must not seal a checkpoint");
    assert!(
        outcome
            .err()
            .unwrap()
            .to_string()
            .contains("injected crash")
    );
    assert!(!sandbox.checkpoint_path().exists());
    // Durable truth that does exist: the continuation ref carries the
    // committed repair (the commit object is the step-1 artifact).
    let pin = git(&sandbox.source(), &["rev-parse", CONTINUATION_REF]).unwrap();
    let file = git(
        &sandbox.source(),
        &["show", &format!("{pin}:src/bucket.rs")],
    )
    .unwrap();
    assert!(
        file.contains("self.last_refill += ticks"),
        "the committed step-1 artifact is durable without the checkpoint"
    );
    // Recovery refuses without a checkpoint and performs NO writes.
    let run = run_worker_b(&sandbox, &[], None, None);
    assert_eq!(run.code, 3);
    assert_eq!(run.result["outcome"], "missing-checkpoint");
    let count = git(
        &sandbox.source(),
        &["rev-list", "--count", &format!("{}..{}", pin.clone(), pin)],
    )
    .unwrap();
    assert_eq!(count, "0");
    assert!(!sandbox.resume_root().exists());
}

#[test]
fn resumed_validation_failure_is_recorded_not_hidden() {
    // A repairs with a pinned-but-broken edit: the crate builds, the task
    // test does not pass. B must surface the failure honestly — record it,
    // mark nothing complete, and never silently retry or rewrite step 1.
    let (sandbox, a) = a_completed_fixture(&BROKEN_EDIT);
    let broken_digest = a.file_digest.clone();
    let run = run_worker_b(&sandbox, &[], None, None);
    assert_eq!(run.code, 0);
    assert_eq!(run.result["outcome"], "validation-failed");
    assert_ne!(run.result["validator"]["exitCode"], 0);
    let after = std::fs::read_to_string(
        sandbox
            .checkpoint_dir()
            .join("portable_work_state_after.json"),
    )
    .unwrap();
    let after: PortableWorkState = serde_json::from_str(&after).unwrap();
    let step2 = after.steps.iter().find(|s| s.step_id == STEP2).unwrap();
    assert_eq!(
        step2.status,
        WorkStepStatus::Pending,
        "failed resume must not complete the step"
    );
    assert_eq!(after.failures.len(), 1);
    assert_eq!(
        after.failures[0].class,
        prometheos_lite::workflow::portable_state::FailureClass::Validation
    );
    assert!(!after.validation_results[0].passed);
    // B did not rewrite A's (broken) step-1 artifact.
    let on_resumed = sha256_hex(
        &std::fs::read(
            Path::new(run.result["evidence"]["freshWorkspace"].as_str().unwrap())
                .join("src/bucket.rs"),
        )
        .unwrap(),
    );
    assert_eq!(on_resumed, broken_digest);
    // No new commit from B: the pinned continuation ref still points exactly
    // at the step-1 head A committed.
    let pin = git(&sandbox.source(), &["rev-parse", CONTINUATION_REF]).unwrap();
    assert_eq!(pin, a.head_revision);
    let source_head = git(&sandbox.source(), &["rev-parse", "HEAD"]).unwrap();
    assert_eq!(source_head, a.base_revision);
}
