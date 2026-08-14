//! Portable work state contract.
//!
//! Describes *what work exists, what happened, why, what remains, and what
//! another compatible model/harness needs to continue safely* — a portable,
//! provider/harness-independent durable record that sits above the evaluation
//! pipeline (it does not depend on [`crate::workflow::evaluate`]).
//!
//! A [`PortableWorkState`] is the machine-readable handoff between executions.
//! It is intentionally **portable**: importing a state must never require the
//! original provider, model, harness, chat, or process memory. What the next
//! run needs is carried explicitly in [`CompatibilityMetadata`]; everything
//! else is optional environmental provenance.
//!
//! Canonical form: [`to_canonical_json`] serializes a state deterministically
//! (recursively sorted object keys, compact output), and [`state_digest`]
//! produces a stable SHA-256 over that canonical form. Semantically identical
//! states always produce identical canonical bytes and identical digests.
//!
//! Import ([`import_portable_state`]) runs a strict pipeline — parse →
//! version → migrate legacy → typed-validate → invariants → refs → decision
//! graph → repository compatibility (when expected metadata is supplied) — and
//! **fails closed**: a state is never partially accepted.

use crate::workflow::durable::atomic_write_json;
use crate::workflow::schema::{
    CURRENT_SCHEMA_VERSION, DocumentType, LEGACY_UNVERSIONED_VERSION, SchemaVersion, VersionStatus,
    validate_version, version_diagnostic,
};
use crate::workflow::{AuthorityLevel, is_hostile_path};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::Path;

/// Canonical file name for a portable work state document.
pub const PORTABLE_STATE_FILENAME: &str = "portable_work_state.json";

/// Durable identity of the work itself. Distinct from the repository or a
/// single execution: `work_id` names the work, `task_id` the task it serves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkIdentity {
    pub work_id: String,
    pub task_id: String,
    /// Human-readable objective this work serves.
    pub objective: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Snapshot of the repository the work was produced against.
///
/// `identity` is a **durable** name (origin URL or stable repository name),
/// never an absolute local path. `local_path` is environmental metadata only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositorySnapshot {
    /// Durable repository identity (origin URL or stable name), never an
    /// absolute local path.
    pub identity: String,
    pub branch: String,
    /// Exact commit SHA the state was produced against.
    pub revision: String,
    /// Local checkout path (environmental metadata only, not durable identity).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
}

/// The accepted plan, persisted so the next run does not reconstruct it from
/// prose. `step_ids` are ordered references into [`PortableWorkState::steps`];
/// each referenced step carries its own status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub plan_id: String,
    pub title: String,
    pub summary: String,
    pub step_ids: Vec<String>,
}

/// A single work step with its lifecycle status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkStep {
    pub step_id: String,
    pub title: String,
    pub status: WorkStepStatus,
    /// Required when the step is [`WorkStepStatus::Blocked`].
    #[serde(default)]
    pub blocked_reason: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkStepStatus {
    Pending,
    InProgress,
    Completed,
    Blocked,
    Cancelled,
}

/// An immutable decision record. Decisions are append-only history: they are
/// superseded, contested, or marked stale — never deleted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub decision_id: String,
    /// What the decision was about.
    pub subject: String,
    /// The decision text.
    pub decision: String,
    pub status: DecisionStatus,
    /// Optional confidence in `0.0..=1.0`. `None` is preferred to a fabricated
    /// value.
    #[serde(default)]
    pub confidence: Option<Confidence>,
    #[serde(default)]
    pub provenance: Vec<DecisionProvenance>,
    /// Decision ids this decision supersedes.
    #[serde(default)]
    pub supersedes: Vec<String>,
    /// The decision id that supersedes this one, when it exists.
    #[serde(default)]
    pub superseded_by: Option<String>,
    /// Decision ids this decision conflicts with. Conflicts must be recorded
    /// on **both** sides or the state fails validation.
    #[serde(default)]
    pub conflicts_with: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionStatus {
    Accepted,
    /// Superseded by a later accepted decision; the record remains auditable.
    Superseded,
    Contested,
    /// Explicitly marked no longer relevant; kept, never deleted.
    Stale,
}

/// What a decision is based on. Distinguishes observed facts from model
/// inference, human decisions, and validation evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionProvenance {
    #[serde(rename = "kind")]
    pub kind: ProvenanceKind,
    #[serde(default)]
    pub source: Option<PortableRef>,
    pub summary: String,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceKind {
    /// Observed directly from the repository (commits, diffs, files).
    RepositorySource,
    /// Produced artifact (proposal, patch, report).
    Artifact,
    /// Human review/approval/rejection decision.
    HumanDecision,
    /// Result of a validation run.
    ValidationResult,
    /// Output of a tool (git, compilers, tests).
    ToolResult,
    /// Model-generated inference or output.
    ModelInference,
    /// External evidence (docs, issues, upstream data).
    ExternalEvidence,
    /// Another decision already recorded in this state.
    PriorDecision,
}

/// Confidence in `0.0..=1.0`. Rejects `NaN`, infinity, and out-of-range values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Confidence {
    pub value: f64,
    #[serde(default)]
    pub basis: Option<String>,
}

/// A typed portable reference. URIs must be relative and free of `..`
/// traversal and absolute/escape forms; digests, when present, must be
/// 64-character sha256 hex digests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortableRef {
    pub kind: PortableRefKind,
    pub uri: String,
    #[serde(default)]
    pub digest: Option<String>,
    #[serde(default)]
    pub media_type: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortableRefKind {
    Context,
    Artifact,
    Proposal,
    Diff,
    Validation,
    Review,
    Evidence,
}

/// Preserved result of a validation run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationResult {
    pub result_id: String,
    pub passed: bool,
    pub summary: String,
    #[serde(default)]
    pub evidence: Option<PortableRef>,
    pub executed_at: String,
}

/// Preserved human review/approval result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewResult {
    pub review_id: String,
    pub decision: ReviewDecision,
    pub reviewer: String,
    #[serde(default)]
    pub notes: String,
    pub reviewed_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    Approved,
    Rejected,
    RequestChanges,
}

/// A classified failure. Infrastructure failures are distinct from model
/// failures and validation failures so the next run can choose the right
/// recovery without re-interpreting free-form logs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FailureRecord {
    pub failure_id: String,
    pub class: FailureClass,
    pub stage: String,
    #[serde(default)]
    pub step_id: Option<String>,
    #[serde(default)]
    pub evidence: Option<PortableRef>,
    #[serde(default)]
    pub recoverable: Option<bool>,
    pub occurred_at: String,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    Infrastructure,
    Model,
    Validation,
    Governance,
    Resource,
    Other,
}

/// Snapshot of the authority and policy the work ran under. The next run may
/// reduce authority, never silently expand it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthoritySnapshot {
    pub authority: AuthorityLevel,
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    #[serde(default)]
    pub forbidden_paths: Vec<String>,
    pub allow_dependency_changes: bool,
    #[serde(default)]
    pub max_files_changed: Option<usize>,
    #[serde(default)]
    pub max_lines_changed: Option<usize>,
    #[serde(default)]
    pub policy_digest: Option<String>,
}

/// Which provider/model/harness produced a past execution. All fields are
/// optional: importing a portable state must never require the original
/// provider or harness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionProvenance {
    pub execution_id: String,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub harness: Option<String>,
    #[serde(default)]
    pub harness_version: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
}

/// What a compatible run must provide to resume safely. `state_schema_version`
/// mirrors the top-level version. Capabilities must be **portable** names; a
/// capability that requires the original provider, model, harness, chat, or
/// process memory is rejected because it encodes non-portable continuation
/// state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatibilityMetadata {
    pub state_schema_version: SchemaVersion,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    #[serde(default)]
    pub optional_capabilities: Vec<String>,
    #[serde(default)]
    pub resume_blockers: Vec<String>,
}

/// The portable work state document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortableWorkState {
    pub schema_version: SchemaVersion,
    pub work: WorkIdentity,
    pub repository: RepositorySnapshot,
    #[serde(default)]
    pub plan: Option<Plan>,
    #[serde(default)]
    pub steps: Vec<WorkStep>,
    /// Immutable decision history.
    #[serde(default)]
    pub decisions: Vec<DecisionRecord>,
    #[serde(default)]
    pub context_refs: Vec<PortableRef>,
    #[serde(default)]
    pub artifact_refs: Vec<PortableRef>,
    #[serde(default)]
    pub proposal_ref: Option<PortableRef>,
    #[serde(default)]
    pub diff_ref: Option<PortableRef>,
    #[serde(default)]
    pub validation_results: Vec<ValidationResult>,
    #[serde(default)]
    pub review_results: Vec<ReviewResult>,
    #[serde(default)]
    pub failures: Vec<FailureRecord>,
    pub authority: AuthoritySnapshot,
    #[serde(default)]
    pub execution_history: Vec<ExecutionProvenance>,
    pub compatibility: CompatibilityMetadata,
    pub created_at: String,
    pub updated_at: String,
}

// ---------------------------------------------------------------------------
// Canonical serialization, digest, export, import
// ---------------------------------------------------------------------------

/// Deterministic canonical JSON for `state`.
///
/// Object keys are recursively sorted (serde_json's `Map` is `BTreeMap`
/// backed, so keys are already ordered; the sort is defensive against future
/// feature changes) and the output is compact, so semantically identical
/// states produce byte-identical output regardless of key insertion order.
pub fn to_canonical_json(state: &PortableWorkState) -> Result<String> {
    let value = canonical_value(state)?;
    serde_json::to_string(&value).context("failed to serialize canonical portable work state")
}

/// Stable SHA-256 digest over [`to_canonical_json`], matching the repository's
/// existing sha256-hex convention.
pub fn state_digest(state: &PortableWorkState) -> Result<String> {
    let canonical = to_canonical_json(state)?;
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    Ok(format!("{:x}", hasher.finalize()))
}

/// Import `json` as a fully-validated [`PortableWorkState`] with no repository
/// compatibility expectation. See [`import_portable_state`] for the full
/// pipeline.
pub fn from_json(json: &str) -> Result<PortableWorkState> {
    import_portable_state(json, None)
}

/// Import a portable work state, validating every layer and failing closed on
/// any defect.
///
/// Pipeline: parse → classify version → migrate legacy in memory → typed
/// validation → structural invariants → portable refs → decision graph →
/// repository compatibility (when `expected_repo` is supplied). A state is
/// never partially accepted: any failure aborts the import with no state.
pub fn import_portable_state(
    json: &str,
    expected_repo: Option<&RepositorySnapshot>,
) -> Result<PortableWorkState> {
    let mut value: Value =
        serde_json::from_str(json).with_context(|| "failed to parse portable work state JSON")?;

    let declared = declared_version(&value)?;
    match validate_version(DocumentType::PortableWorkState, declared)? {
        VersionStatus::Unsupported => bail!(
            "{}",
            version_diagnostic(DocumentType::PortableWorkState, declared).migration_action
        ),
        // Legacy (unversioned) documents: inject the current version in memory
        // and validate the migrated form. The caller's JSON is never rewritten.
        VersionStatus::Legacy => {
            if let Some(obj) = value.as_object_mut() {
                obj.insert(
                    "schema_version".to_string(),
                    Value::String(CURRENT_SCHEMA_VERSION.to_string_owned()),
                );
            }
        }
        VersionStatus::Current => {}
    }

    let state: PortableWorkState =
        serde_json::from_value(value).context("portable work state failed typed validation")?;
    validate_state(&state)?;

    if let Some(expected) = expected_repo {
        validate_repository_compat(expected, &state.repository)?;
    }

    Ok(state)
}

/// Export `state` as a deterministic, durable document at `path`.
///
/// The on-disk form is the canonical value written through the same atomic
/// publication used by every durable document, so a failed export is reported
/// and never silently swallowed.
pub fn export_portable_state(state: &PortableWorkState, path: &Path) -> Result<()> {
    let value = canonical_value(state)?;
    atomic_write_json(path, &value)
}

fn canonical_value(state: &PortableWorkState) -> Result<Value> {
    let mut value =
        serde_json::to_value(state).with_context(|| "failed to serialize portable work state")?;
    sort_keys_recursive(&mut value);
    Ok(value)
}

fn sort_keys_recursive(value: &mut Value) {
    if let Value::Object(map) = value {
        for v in map.values_mut() {
            sort_keys_recursive(v);
        }
        let sorted: Map<String, Value> = map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        *map = sorted;
    } else if let Value::Array(items) = value {
        for item in items {
            sort_keys_recursive(item);
        }
    }
}

fn declared_version(value: &Value) -> Result<SchemaVersion> {
    match value.get("schema_version").and_then(|v| v.as_str()) {
        Some(s) => SchemaVersion::parse(s),
        None => Ok(LEGACY_UNVERSIONED_VERSION),
    }
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

fn validate_state(state: &PortableWorkState) -> Result<()> {
    if !state.schema_version.is_current() {
        bail!(
            "portable work state must declare the current schema version, found {}",
            state.schema_version
        );
    }
    validate_work_identity(&state.work)?;
    validate_repository(&state.repository)?;
    validate_plan_and_steps(state)?;
    validate_refs(state)?;
    validate_decisions(state)?;
    validate_validation_results(&state.validation_results)?;
    validate_failures(&state.failures)?;
    validate_compatibility(&state.compatibility)?;
    validate_execution_history(&state.execution_history)?;
    Ok(())
}

fn validate_work_identity(work: &WorkIdentity) -> Result<()> {
    if work.work_id.trim().is_empty() {
        bail!("work identity has an empty work_id");
    }
    if work.task_id.trim().is_empty() {
        bail!("work identity has an empty task_id");
    }
    if work.objective.trim().is_empty() {
        bail!("work identity has an empty objective");
    }
    if work.created_at.trim().is_empty() {
        bail!("work identity has an empty created_at");
    }
    if work.updated_at.trim().is_empty() {
        bail!("work identity has an empty updated_at");
    }
    Ok(())
}

fn validate_repository(repo: &RepositorySnapshot) -> Result<()> {
    let identity = repo.identity.trim();
    if identity.is_empty() {
        bail!("repository identity is empty");
    }
    if is_hostile_path(identity) {
        bail!(
            "repository identity must be a durable remote name, not a local absolute path: {}",
            repo.identity
        );
    }
    if repo.branch.trim().is_empty() {
        bail!("repository branch is empty");
    }
    if repo.revision.trim().is_empty() {
        bail!("repository revision is empty");
    }
    if let Some(local) = &repo.local_path
        && local.trim().is_empty()
    {
        bail!("repository local_path, when present, must not be empty");
    }
    Ok(())
}

fn validate_plan_and_steps(state: &PortableWorkState) -> Result<()> {
    let mut step_ids: BTreeSet<&str> = BTreeSet::new();
    for s in &state.steps {
        if s.step_id.trim().is_empty() {
            bail!("step with empty step_id");
        }
        if !step_ids.insert(s.step_id.as_str()) {
            bail!("duplicate step_id: {}", s.step_id);
        }
        if s.status == WorkStepStatus::Blocked {
            let reason = s.blocked_reason.as_deref().map(str::trim).unwrap_or("");
            if reason.is_empty() {
                bail!("blocked step {} must carry a blocked_reason", s.step_id);
            }
        }
    }
    if let Some(plan) = &state.plan {
        if plan.plan_id.trim().is_empty() {
            bail!("plan has an empty plan_id");
        }
        if plan.title.trim().is_empty() {
            bail!("plan {} has an empty title", plan.plan_id);
        }
        if plan.step_ids.is_empty() {
            bail!("plan {} lists no steps", plan.plan_id);
        }
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for sid in &plan.step_ids {
            if !seen.insert(sid.as_str()) {
                bail!("plan {} lists step {sid} more than once", plan.plan_id);
            }
            if !step_ids.contains(sid.as_str()) {
                bail!("plan {} references unknown step {sid}", plan.plan_id);
            }
        }
    }
    Ok(())
}

fn validate_refs(state: &PortableWorkState) -> Result<()> {
    let mut refs: Vec<&PortableRef> = Vec::new();
    refs.extend(state.context_refs.iter());
    refs.extend(state.artifact_refs.iter());
    if let Some(r) = &state.proposal_ref {
        refs.push(r);
    }
    if let Some(r) = &state.diff_ref {
        refs.push(r);
    }
    for v in &state.validation_results {
        if let Some(r) = &v.evidence {
            refs.push(r);
        }
    }
    for f in &state.failures {
        if let Some(r) = &f.evidence {
            refs.push(r);
        }
    }
    for d in &state.decisions {
        for p in &d.provenance {
            if let Some(r) = &p.source {
                refs.push(r);
            }
        }
    }
    for r in refs {
        validate_ref(r)?;
    }
    Ok(())
}

fn validate_ref(r: &PortableRef) -> Result<()> {
    let uri = r.uri.trim();
    if uri.is_empty() {
        bail!("portable ref has an empty uri");
    }
    if is_hostile_path(uri) {
        bail!(
            "portable ref uri is absolute or escapes the repository: {}",
            r.uri
        );
    }
    if let Some(digest) = &r.digest {
        let digest = digest.trim();
        if digest.len() != 64 || !digest.chars().all(|c| c.is_ascii_hexdigit()) {
            bail!(
                "portable ref digest must be a 64-character sha256 hex digest: {}",
                r.uri
            );
        }
    }
    if let Some(mt) = &r.media_type
        && mt.trim().is_empty()
    {
        bail!("portable ref media_type is empty: {}", r.uri);
    }
    Ok(())
}

/// Validate the decision graph: unique ids, no dangling references, consistent
/// supersession edges, no supersession cycles, and conflicts recorded on both
/// sides. Superseded decisions stay in history — they are never deleted.
fn validate_decisions(state: &PortableWorkState) -> Result<()> {
    let mut ids: BTreeSet<&str> = BTreeSet::new();
    for d in &state.decisions {
        if d.decision_id.trim().is_empty() {
            bail!("decision with empty decision_id");
        }
        if d.subject.trim().is_empty() {
            bail!("decision {} has an empty subject", d.decision_id);
        }
        if d.decision.trim().is_empty() {
            bail!("decision {} has an empty decision body", d.decision_id);
        }
        if !ids.insert(d.decision_id.as_str()) {
            bail!("duplicate decision_id: {}", d.decision_id);
        }
        if let Some(c) = &d.confidence {
            validate_confidence(c)?;
        }
        for p in &d.provenance {
            if p.summary.trim().is_empty() {
                bail!(
                    "decision {} has a provenance entry without a summary",
                    d.decision_id
                );
            }
        }
    }

    // Every referenced decision must exist.
    for d in &state.decisions {
        for s in &d.supersedes {
            require_decision(
                &state.decisions,
                s,
                &format!("decision {} supersedes unknown {s}", d.decision_id),
            )?;
        }
        for c in &d.conflicts_with {
            require_decision(
                &state.decisions,
                c,
                &format!("decision {} conflicts with unknown {c}", d.decision_id),
            )?;
        }
        if let Some(s) = &d.superseded_by {
            require_decision(
                &state.decisions,
                s,
                &format!("decision {} superseded_by unknown {s}", d.decision_id),
            )?;
        }
    }

    for d in &state.decisions {
        match d.status {
            DecisionStatus::Accepted | DecisionStatus::Contested => {
                if d.superseded_by.is_some() {
                    bail!(
                        "decision {} has status {:?} but is marked superseded_by",
                        d.decision_id,
                        d.status
                    );
                }
            }
            DecisionStatus::Superseded => {
                let Some(super_id) = &d.superseded_by else {
                    bail!(
                        "decision {} is Superseded but has no superseded_by",
                        d.decision_id
                    );
                };
                // The superseding decision must be Accepted and list this one.
                let superseder = by_id(&state.decisions, super_id);
                if superseder.status != DecisionStatus::Accepted {
                    bail!(
                        "decision {} superseded by {} which is not Accepted",
                        d.decision_id,
                        super_id
                    );
                }
                if !superseder.supersedes.iter().any(|x| x == &d.decision_id) {
                    bail!(
                        "decision {} is superseded_by {} but {} does not list it in supersedes",
                        d.decision_id,
                        super_id,
                        super_id
                    );
                }
            }
            DecisionStatus::Stale => {
                if d.superseded_by.is_some() {
                    bail!(
                        "stale decision {} must not be marked superseded_by",
                        d.decision_id
                    );
                }
            }
        }
        // Every supersede edge must be mirrored by the target's superseded_by.
        for s in &d.supersedes {
            let target = by_id(&state.decisions, s);
            if target.superseded_by != Some(d.decision_id.clone()) {
                bail!(
                    "decision {} supersedes {s} but {s} is not marked superseded_by {}",
                    d.decision_id,
                    d.decision_id
                );
            }
        }
    }

    // No supersession cycles (defense in depth: the lifecycle rules above
    // already make an Accepted superseder a structural requirement).
    let edges: Vec<(String, String)> = state
        .decisions
        .iter()
        .flat_map(|d| {
            d.supersedes
                .iter()
                .map(move |s| (d.decision_id.clone(), s.clone()))
        })
        .collect();
    detect_supersession_cycles(&edges)?;

    // Conflicts must be recorded symmetrically, and never against a stale
    // decision (stale decisions are resolved, not in active conflict).
    for d in &state.decisions {
        for c in &d.conflicts_with {
            let other = by_id(&state.decisions, c);
            if !other.conflicts_with.iter().any(|x| x == &d.decision_id) {
                bail!(
                    "decision {} conflicts with {c} but {c} does not record the conflict (untracked)",
                    d.decision_id
                );
            }
            if other.status == DecisionStatus::Stale {
                bail!(
                    "decision {} conflicts with stale decision {c}",
                    d.decision_id
                );
            }
        }
    }
    Ok(())
}

fn by_id<'a>(decisions: &'a [DecisionRecord], id: &str) -> &'a DecisionRecord {
    decisions
        .iter()
        .find(|d| d.decision_id == id)
        .expect("referenced decision must exist after dangling checks")
}

fn require_decision(decisions: &[DecisionRecord], id: &str, message: &str) -> Result<()> {
    if !decisions.iter().any(|d| d.decision_id == id) {
        bail!("{message}");
    }
    Ok(())
}

fn detect_supersession_cycles(edges: &[(String, String)]) -> Result<()> {
    fn visit(
        node: &str,
        edges: &[(String, String)],
        visiting: &mut BTreeSet<String>,
        done: &mut BTreeSet<String>,
    ) -> Result<()> {
        if done.contains(node) {
            return Ok(());
        }
        if !visiting.insert(node.to_string()) {
            bail!("supersession cycle detected involving decision {node}");
        }
        for (from, to) in edges {
            if from == node {
                visit(to, edges, visiting, done)?;
            }
        }
        visiting.remove(node);
        done.insert(node.to_string());
        Ok(())
    }
    let mut visiting: BTreeSet<String> = BTreeSet::new();
    let mut done: BTreeSet<String> = BTreeSet::new();
    for (from, _) in edges {
        visit(from, edges, &mut visiting, &mut done)?;
    }
    Ok(())
}

fn validate_validation_results(results: &[ValidationResult]) -> Result<()> {
    for v in results {
        if v.result_id.trim().is_empty() {
            bail!("validation result with empty result_id");
        }
        if v.executed_at.trim().is_empty() {
            bail!("validation result {} has an empty executed_at", v.result_id);
        }
        if v.summary.trim().is_empty() {
            bail!("validation result {} has an empty summary", v.result_id);
        }
    }
    Ok(())
}

fn validate_failures(failures: &[FailureRecord]) -> Result<()> {
    for f in failures {
        if f.failure_id.trim().is_empty() {
            bail!("failure record with empty failure_id");
        }
        if f.stage.trim().is_empty() {
            bail!("failure record {} has an empty stage", f.failure_id);
        }
        if f.occurred_at.trim().is_empty() {
            bail!("failure record {} has an empty occurred_at", f.failure_id);
        }
    }
    Ok(())
}

fn validate_compatibility(c: &CompatibilityMetadata) -> Result<()> {
    if !c.state_schema_version.is_current() {
        bail!(
            "compatibility metadata declares non-current state schema version {}",
            c.state_schema_version
        );
    }
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for cap in &c.required_capabilities {
        let cap = cap.trim();
        if cap.is_empty() {
            bail!("required capability must not be empty");
        }
        let lower = cap.to_lowercase();
        let reserved = [
            "provider:",
            "model:",
            "harness:",
            "chat:",
            "session:",
            "process-memory:",
            "memory:",
        ];
        if reserved.iter().any(|p| lower.starts_with(p)) {
            bail!(
                "required capability '{cap}' encodes non-portable continuation state; \
                 portable capabilities must not require the original provider, model, \
                 harness, chat, or process memory"
            );
        }
        if !seen.insert(cap) {
            bail!("required capability listed more than once: {cap}");
        }
    }
    for b in &c.resume_blockers {
        if b.trim().is_empty() {
            bail!("resume blocker must not be empty");
        }
    }
    Ok(())
}

fn validate_execution_history(history: &[ExecutionProvenance]) -> Result<()> {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for e in history {
        if e.execution_id.trim().is_empty() {
            bail!("execution provenance with empty execution_id");
        }
        if !seen.insert(e.execution_id.as_str()) {
            bail!("duplicate execution_id: {}", e.execution_id);
        }
        // provider/model/harness/harness_version are all optional: importing a
        // portable state must never require the original provider or harness.
    }
    Ok(())
}

fn validate_repository_compat(
    expected: &RepositorySnapshot,
    actual: &RepositorySnapshot,
) -> Result<()> {
    if expected.identity != actual.identity {
        bail!(
            "repository identity mismatch: expected '{}', state recorded '{}'",
            expected.identity,
            actual.identity
        );
    }
    if expected.branch != actual.branch {
        bail!(
            "repository branch mismatch: expected '{}', state recorded '{}'",
            expected.branch,
            actual.branch
        );
    }
    if expected.revision != actual.revision {
        bail!(
            "repository revision mismatch: expected '{}', state recorded '{}'",
            expected.revision,
            actual.revision
        );
    }
    Ok(())
}

fn validate_confidence(c: &Confidence) -> Result<()> {
    if !c.value.is_finite() {
        bail!("confidence must be a finite number");
    }
    if !(0.0..=1.0).contains(&c.value) {
        bail!("confidence must be within 0.0..=1.0, found {}", c.value);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confidence_rejects_non_finite_and_out_of_range() {
        assert!(
            validate_confidence(&Confidence {
                value: f64::NAN,
                basis: None
            })
            .is_err()
        );
        assert!(
            validate_confidence(&Confidence {
                value: f64::INFINITY,
                basis: None
            })
            .is_err()
        );
        assert!(
            validate_confidence(&Confidence {
                value: -0.1,
                basis: None
            })
            .is_err()
        );
        assert!(
            validate_confidence(&Confidence {
                value: 1.5,
                basis: None
            })
            .is_err()
        );
        assert!(
            validate_confidence(&Confidence {
                value: 0.0,
                basis: None
            })
            .is_ok()
        );
        assert!(
            validate_confidence(&Confidence {
                value: 1.0,
                basis: None
            })
            .is_ok()
        );
        assert!(
            validate_confidence(&Confidence {
                value: 0.5,
                basis: Some("ok".into())
            })
            .is_ok()
        );
    }

    #[test]
    fn sort_keys_recursive_is_stable_and_nested() {
        let mut value = serde_json::json!({
            "b": 1,
            "a": { "z": [ { "k2": 1, "k1": 2 } ], "y": 3 },
        });
        let before = value.clone();
        sort_keys_recursive(&mut value);
        assert_eq!(value, before);
        assert_eq!(
            serde_json::to_string(&value).unwrap(),
            "{\"a\":{\"y\":3,\"z\":[{\"k1\":2,\"k2\":1}]},\"b\":1}"
        );
    }

    #[test]
    fn unsupported_future_version_message_is_actionable() {
        let err = from_json(r#"{"schema_version":"99.0.0"}"#).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("PortableWorkState"), "{msg}");
        assert!(msg.contains("fail closed"), "{msg}");
    }
}
