//! Model-authored repository-task repair loop (issue #136, Slice D).
//!
//! **Genuinely model-authored code**, not a scripted fixture and not a
//! replay: the patch that lands in the pinned repository is the model's
//! own output, proposed against a real issue statement, a real isolated
//! worktree and a real validator subprocess, with failure feedback routed
//! back into the next attempt. Nothing in this module authors source code
//! itself; it governs what the model is allowed to see, change and prove.
//!
//! Load-bearing invariants (same discipline as Slice B):
//!
//! 1. No governed effect happens before the caller's pins hold: the staged
//!    repair target must match the pinned digest and the staged tree must
//!    match the pinned fingerprint — refusal happens before any model
//!    dispatch.
//! 2. The model holds **no** write authority. Model proposals are applied
//!    through the file policy + editable set + apply-with-rollback path,
//!    under a bounded, recorded permit; the dispatched model node itself
//!    never receives a writable scope.
//! 3. A failed or refused model dispatch **never** authorizes an apply.
//! 4. Every attempt, receipt, refusal and digest is sealed into the report;
//!    a repaired claim is only ever derived from a validator receipt
//!    produced by a subprocess this process spawned, and it is re-derived
//!    at verification.
//! 5. The hidden acceptance test of the task is **never** part of the
//!    model's context. The prompt carries only the issue statement and the
//!    state the model is entitled to inspect; the acceptance overlay
//!    happens outside this loop, after the model is finished.
//!
//! Wire format requested of the model: one fenced block containing a
//! unified diff for the single authorized file. Responses are parsed by the
//! existing [`crate::harness::edit_protocol::parse_edit_response`]; fenced
//! wrappers are stripped first. Budgets are enforced before any transport.
//!
//! [`ScriptedProposer`] is a deterministic in-process test double for the
//! hermetic suite. It is **not** a stand-in for a live model result. The
//! live adapter is wired in `examples/model_repair_live.rs` against the
//! governed remote-service harness path; it is driven by operator
//! configuration, never by task input.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::harness::edit_protocol::{EditOperation, parse_edit_response};
use crate::harness::file_control::{
    FilePolicy, FileSet, resolve_repo_path, validate_repo_relative_path,
};
use crate::harness::patch_applier::{apply_patch_with_rollback, dry_run_patch};
use crate::workflow::redaction::{REDACTED_PLACEHOLDER, Redactor, SECRET_CANARY};
use crate::workflow::scenario::{TaskValidatorProvenance, TaskValidatorRecord};
use crate::workflow::soma::contracts::{AuthorityProfile, EvidenceReference};
use crate::workflow::soma::event::{
    Actor, ActorKind, Compatibility, EventPayload, ResultPayload, WorkEvent, WorkEventBatch,
};
use crate::workflow::soma::types::{ExecutionClass, Hex64, MutationMode};
use serde_json::json;

/// Current schema version of [`ModelRepairReport`].
pub const MODEL_REPAIR_REPORT_SCHEMA: &str = "lite.model-repair.report.v1";

/// Cap on captured validator output retained for digesting (same cap as
/// the executed-scenario suites).
pub const VALIDATOR_OUTPUT_CAP: usize = 1_000_000;

/// Default per-validator subprocess wall-time budget.
pub const VALIDATOR_TIMEOUT: Duration = Duration::from_secs(240);

/// Default ceiling for model attempts per repair.
pub const DEFAULT_MAX_ATTEMPTS: u32 = 4;

/// Ceiling for the cumulative prompt bytes sent across all attempts.
pub const MAX_PROMPT_BYTES_TOTAL: u64 = 96 * 1024;

/// Ceiling for one prompt.
pub const MAX_PROMPT_BYTES: usize = 28 * 1024;

/// Ceiling for the feedback excerpt forwarded into the next attempt.
pub const MAX_FEEDBACK_BYTES: usize = 1_500;

/// Ceiling for the fenced model response we will parse.
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024;

/// Truncation headroom for diff artifacts retained in the report.
pub const MAX_DIFF_BYTES: usize = 16 * 1024;

/// Tripwire bound on total prompt bytes per attempt (defense in depth).
#[must_use]
pub const fn max_prompt_bytes() -> usize {
    MAX_PROMPT_BYTES
}

// ---------------------------------------------------------------------------
// Specs and report types
// ---------------------------------------------------------------------------

/// Optional bounded context replacing the full file content in the prompt.
/// Used when the pinned file exceeds the provider wire budget; the excerpt
/// must be verbatim lines from the pinned tree and the framing states the
/// absolute line range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromptContext {
    /// Verbatim excerpt bytes.
    pub text: String,
    /// 1-based line number of the first excerpt line in the file.
    pub start_line: u64,
    /// Human-readable note (e.g. "excerpt of lines 616-851").
    pub note: String,
}

/// Operator-supplied authorization evidence for the model-dispatch egress.
/// Recorded by the runner from the dispatch spec, never by the model.
/// Absent evidence is recorded as `None`; the strict path treats absent
/// evidence as unverified, never as authorized. Not a credential: a
/// receipt digest and an operator grant/task-scope reference for audit
/// correlation only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EgressGrantEvidence {
    /// sha256 (hex64) of the operator-supplied authorization receipt bytes.
    pub receipt_sha256: String,
    /// Operator grant/task-scope reference authorizing this egress. Not a
    /// credential; recorded for audit correlation.
    pub grant_ref: String,
    /// RFC 3339 UTC instant the operator attests authorization was in force.
    pub authorized_at: String,
}

/// The task the model must solve, as pinned by the caller.
#[derive(Debug, Clone)]
pub struct ModelRepairSpec {
    /// Stable task identifier (e.g. `lite-hsel-decisionid-forgery-0001`).
    pub task_id: String,
    /// The repository-relative, forward-slash path of the single file the
    /// model may change.
    pub repair_target: String,
    /// Absolute path of the isolated writable worktree that carries the
    /// pinned pre-fix tree. The caller stages and pins it before handing
    /// it here; this module verifies it again (fail closed).
    pub worktree: PathBuf,
    /// Expected sha256 of the staged repair target at the pre-fix pin.
    pub repair_target_sha256: String,
    /// Expected digest of the staged tree (see [`tree_digest`]).
    pub workspace_fingerprint: String,
    /// Exact commit the staged worktree is pinned to.
    pub base_revision: String,
    /// The issue statement shown to the model. Never contains the hidden
    /// acceptance test.
    pub issue_statement: String,
    /// Validator executable + argv the report will pin and re-check.
    pub validator: ValidatorSpec,
    /// Wall-time budget per validator subprocess.
    pub validator_timeout: Duration,
    /// Maximum model attempts.
    pub max_attempts: u32,
    /// Where the validator's `CARGO_TARGET_DIR` points. `None` → a unique
    /// temp dir per attempt (incremental state is discarded); `Some(path)`
    /// → reused across attempts for incremental builds of heavy crates.
    /// Build directories are excluded from the worktree fingerprint.
    pub validator_target_dir: Option<PathBuf>,
    /// Optional bounded excerpt replacing the full file in the prompt.
    /// When absent, the full repair-target file is sent (small files only).
    pub prompt_context: Option<PromptContext>,
    /// Operator-configured SOMA source binding (repository + revision +
    /// optional issue number). Absent unless configured; never derived
    /// from the model or the task slug.
    pub source: Option<SomaSourceBinding>,
    /// Operator-supplied authorization evidence for the model-dispatch
    /// egress, when an authorized provider route/grant exists. Absent
    /// unless the operator configures it; never derived from the model,
    /// the task slug or any environment variable.
    pub egress_grant: Option<EgressGrantEvidence>,
}

/// Operator-configured binding of the staged worktree to a tracked
/// repository and issue. Never derived from the model, the task slug or the
/// staging path; absent when not configured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SomaSourceBinding {
    /// GitHub slug of the source repository (e.g. `prometheosai/prometheos-lite`).
    pub repository: String,
    /// Exact commit the staged worktree is pinned to (40/64-hex git sha).
    pub repository_revision: String,
    /// The tracked issue this repair is part of, when configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_number: Option<u64>,
}

/// The model's response plus the provenance observed at the response
/// boundary. Never contains credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposedCompletion {
    /// The model's response text (possibly fenced), post-redaction elsewhere.
    pub content: String,
    /// Responder-issued id from the response body's top-level `id`, when the
    /// configured endpoint exposed one. Absent = the responder did not
    /// expose one; it is never synthesized locally.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_response_id: Option<String>,
    /// Provenance source of `provider_response_id`
    /// (`response-body:id` from the configured harness).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_response_id_source: Option<String>,
    /// Gateway-issued trace id, when a transport exposes one. The current
    /// RemoteServiceHarness does not surface headers, so this is absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_trace_id: Option<String>,
    /// Local request identifier used on the wire (`req-<step>`). Correlation
    /// only; never provider-issued.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_request_id: Option<String>,
}

impl ProposedCompletion {
    /// Test/helper constructor: scripted content without provenance.
    pub fn with_content(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            provider_response_id: None,
            provider_response_id_source: None,
            gateway_trace_id: None,
            local_request_id: None,
        }
    }
}

/// Pinned validator surface (command, argv, what exit code means done).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidatorSpec {
    pub command: String,
    pub args: Vec<String>,
    pub expected_exit_code: i32,
    /// What "done" means in one sentence; copied into the receipt.
    pub assertion: String,
}

/// How the model proposal was obtained. Never contains credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelDescriptor {
    pub provider: String,
    pub model: String,
    /// Harness/adapter identity and revision.
    pub harness_id: String,
    pub harness_revision: String,
}

/// One model attempt, sealed as recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelRepairAttempt {
    pub attempt: u32,
    pub prompt_sha256: String,
    pub prompt_bytes: u64,
    /// Model response (post-redaction) digest; `None` on transport failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_bytes: Option<u64>,
    /// Redacted model text, truncated to [`MAX_DIFF_BYTES`]. Diagnostic only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_excerpt: Option<String>,
    /// Patch paths proposed, repo-relative. Always empty on refusals.
    pub proposed_files: Vec<String>,
    pub applied: bool,
    pub rolled_back: bool,
    /// The validator receipt when the validator ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validator: Option<TaskValidatorRecord>,
    /// Typed refusal/failure class for this attempt (e.g. `model-refusal`,
    /// `budget-exceeded`, `unauthorized-proposal`, `unparseable-proposal`,
    /// `validator-rejected`, `provider-unavailable`, `dry-run-rejected`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refusal: Option<String>,
    /// Responder-issued id of the inference this attempt dispatched, when
    /// the configured endpoint exposed one. Absent = the provider did not
    /// issue one (never synthesized).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_response_id: Option<String>,
    /// Provenance source of `provider_response_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_response_id_source: Option<String>,
    /// Gateway-issued trace id, when the transport exposes one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_trace_id: Option<String>,
    /// Local request id on the wire (correlation; never provider-issued).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_request_id: Option<String>,
    pub wall_ms: u64,
}

/// Terminal classification of one model-repair run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelRepairStatus {
    /// Validator ran and exited as pinned; the tree moved off the baseline.
    Repaired,
    /// Validator ran; attempts exhausted without the pinned assertion.
    ValidatorRejected,
    /// Model returned a refusal (or the provider declined).
    ModelRefusal,
    /// Provider/transport failure before any effect.
    ProviderUnavailable,
    /// Proposal failed the authorization preflight; nothing was written.
    UnauthorizedProposal,
    /// Cumulative budgets (attempts, prompt bytes) were exhausted.
    BudgetExhausted,
    /// Caller-provided cancellation token stopped the run between phases.
    Interrupted,
    /// Pre-effect pin/preflight failure; the model was never dispatched.
    SetupError,
}

/// Sealed model-authored-repair evidence (Lite-internal, not a Foundry
/// contract and not a benchmark claim).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelRepairReport {
    pub schema_version: String,
    pub kind: String,
    pub task_id: String,
    pub base_revision: String,
    /// sha256 of the staged repair target at the pin.
    pub repair_target_sha256: String,
    /// The editable target, repo-relative, forward slashes.
    pub repair_target: String,
    pub model: ModelDescriptor,
    pub validator: ValidatorSpec,
    pub status: ModelRepairStatus,
    pub attempts: Vec<ModelRepairAttempt>,
    /// Unique per-execution identity of this run (UUID v4, `exec-<32hex>`).
    /// Generated at run start; every attempt and every `WorkEvent` of this
    /// run binds to it. Never derived from task/model/revision.
    pub execution_id: String,
    /// Operator-configured SOMA source binding, mirrored from the spec.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SomaSourceBinding>,
    /// Operator-supplied egress authorization evidence, mirrored from the
    /// spec at dispatch. Absent unless the operator configured it; never
    /// derived from the model. The strict path treats absent evidence as
    /// unverified (a typed gap), never as authorized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub egress_grant: Option<EgressGrantEvidence>,
    /// The pre-fix validator run. Present unless a pre-effect setup failure
    /// stopped before the baseline could execute. `ObservedSubprocess` exit
    /// code reproduces the pre-fix failing behavior.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_validator: Option<TaskValidatorRecord>,
    /// sha256 of the final worktree tree (paths+blob digests). `None` when
    /// the model was never dispatched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_tree_digest: Option<String>,
    /// sha256 (hex64) of the redacted full diff artifact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_diff_sha256: Option<String>,
    /// The unified diff, truncated to [`MAX_DIFF_BYTES`]. Diagnostic only;
    /// the digest above binds the full artifact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_diff_excerpt: Option<String>,
    pub prompt_bytes_total: u64,
    pub wall_ms: u64,
    /// Sealing instant (RFC 3339 UTC).
    pub sealed_at: String,
    /// Digest binding the attached `WorkEventBatch` emission.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events_sha256: Option<String>,
    /// **Strict SOMA coding-run envelope eligibility.** True iff every
    /// mandatory predicate of [`assess_strict_soma_eligibility`] holds,
    /// including a `Repaired` outcome, an actual patch digest, an observed
    /// pinned validator receipt, an auditable WorkEventBatch whose digest
    /// re-derives, a consistent execution identity, a provider-issued
    /// response id on the sealing attempt, a valid repository/revision/issue
    /// binding, and operator-supplied egress authorization evidence. A
    /// failed or refused run may still be sealed as valid *failure
    /// evidence* (see [`ModelRepairReport::verify`]); it is never `sealable`.
    /// Computed at seal time; re-derived at verification.
    pub sealable: bool,
    /// Explicit reasons the report is not sealable for the strict path.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sealability_reasons: Vec<String>,
}

impl ModelRepairReport {
    /// Re-derive the outcome from the embedded measurements. A report whose
    /// `status` contradicts its attempts/receipts fails verification.
    ///
    /// **Internal structural validity** of the report: schema, attempts,
    /// receipts, digests and identities re-derive as recorded. This is
    /// question A of the strict SOMA eligibility contract and must hold
    /// for failed and refused runs too; it is never evidence of success.
    fn verify_structure(&self) -> Result<(), String> {
        if self.schema_version != MODEL_REPAIR_REPORT_SCHEMA {
            return Err(format!(
                "schema version {} is not {}",
                self.schema_version, MODEL_REPAIR_REPORT_SCHEMA
            ));
        }
        if self.attempts.is_empty()
            && self.status != ModelRepairStatus::SetupError
            && self.status != ModelRepairStatus::Interrupted
        {
            return Err("a non-setup outcome requires at least one attempt".to_string());
        }
        let pinned_exit = self.validator.expected_exit_code;
        if matches!(
            self.status,
            ModelRepairStatus::Repaired | ModelRepairStatus::ValidatorRejected
        ) && self.baseline_validator.is_none()
        {
            return Err(
                "a repaired/rejected outcome requires the baseline validator receipt".to_string(),
            );
        }
        for attempt in &self.attempts {
            if let Some(validator) = &attempt.validator
                && (validator.command != self.validator.command
                    || validator.args != self.validator.args
                    || validator.expected_exit_code != pinned_exit)
            {
                return Err(format!(
                    "attempt {} validator does not match the pinned command/argv/expectation",
                    attempt.attempt
                ));
            }
        }
        if self.status == ModelRepairStatus::Repaired {
            let last = self
                .attempts
                .last()
                .ok_or_else(|| "repaired outcome with no attempts".to_string())?;
            if !last.applied {
                return Err("repaired outcome requires an applied patch".to_string());
            }
            let validator = last
                .validator
                .as_ref()
                .ok_or_else(|| "repaired outcome requires an observed validator".to_string())?;
            if validator.provenance != TaskValidatorProvenance::ObservedSubprocess {
                return Err("repaired outcome requires ObservedSubprocess provenance".to_string());
            }
            if validator.exit_code != Some(pinned_exit) {
                return Err("repaired outcome requires the pinned validator exit code".to_string());
            }
            if self.final_tree_digest.is_none() || self.final_diff_sha256.is_none() {
                return Err("repaired outcome requires tree and diff digests".to_string());
            }
        }
        // Execution identity is load-bearing: tampering with it must break
        // verification. Re-derive the bound events digest.
        if self.execution_id.trim().is_empty() {
            return Err("missing execution identity (executionId)".to_string());
        }
        if let Some(source) = &self.source
            && !source.repository_revision.trim().is_empty()
            && source.repository_revision != self.base_revision
        {
            return Err(format!(
                "source repository revision {} does not match the pinned base revision {}",
                source.repository_revision, self.base_revision
            ));
        }
        let rederived = hex_sha256(&serde_json::to_vec(&batch_of(self)).unwrap_or_default());
        match &self.events_sha256 {
            Some(digest) if *digest == rederived => {}
            Some(_) => {
                return Err(
                    "eventsDigest does not match the WorkEventBatch re-derived from this report"
                        .to_string(),
                );
            }
            None => return Err("missing events digest".to_string()),
        }
        Ok(())
    }

    /// Full verification: internal structural validity plus strict
    /// re-derivation of the sealability verdict recorded in the report.
    ///
    /// # Errors
    /// Returns a message when structural verification fails or the recorded
    /// sealability verdict does not match re-derivation from the report.
    pub fn verify(&self) -> Result<(), String> {
        self.verify_structure()?;
        let (sealable, reasons) = compute_sealability(self);
        if sealable != self.sealable || reasons != self.sealability_reasons {
            return Err("sealability does not match re-derivation from the report".to_string());
        }
        Ok(())
    }
}

/// One typed predicate failure of the strict SOMA eligibility contract.
/// `kind` is one of `missing` (required evidence absent), `invalid`
/// (evidence present but malformed/contradictory) or `unverified` (the
/// proof lives outside this module and was not established, e.g. operator
/// egress authorization, which the runner never fabricates).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SomaEligibilityGap {
    /// Stable predicate identifier (e.g. `patch.present`).
    pub predicate: String,
    /// `missing` | `invalid` | `unverified`.
    pub kind: String,
    /// Operator-readable reason; never a synthesized replacement value.
    pub reason: String,
}

/// The three questions a SOMA consumer may ask of a sealed report, kept
/// strictly separate.
///
/// **A. Internal report validity** (`internal_report_valid`): the report is
/// well formed and its own digests re-derive. This must hold for failed and
/// refused runs too; it is never evidence of success.
///
/// **B. Execution provenance completeness** (`provenance_complete`): the
/// source binding, repository grammar, revision pin, issue identity,
/// execution identity, WorkEventBatch digest, provider response id and
/// operator egress authorization are present and verifiable.
///
/// **C. Strict SOMA coding-run envelope eligibility** (`eligible`): every
/// mandatory predicate needed to emit the strict envelope can be derived
/// from recorded evidence. An unsuccessful run is never eligible; a failure
/// report remains valid *failure evidence* (A) without impersonating a
/// successful coding repair (C).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StrictSomaEligibility {
    /// A. Internal report validity.
    pub internal_report_valid: bool,
    /// B. Execution provenance completeness.
    pub provenance_complete: bool,
    /// C. Strict envelope eligibility (A ∧ B ∧ every mandatory predicate).
    pub eligible: bool,
    /// Typed gaps, ordered deterministically. Empty iff `eligible`.
    pub gaps: Vec<SomaEligibilityGap>,
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_hex_revision(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_valid_github_slug(s: &str) -> bool {
    fn part_ok(p: &str) -> bool {
        !p.is_empty()
            && p.len() <= 100
            && p.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || b == b'_')
            && !p.starts_with('-')
            && !p.starts_with('.')
            && !p.ends_with('.')
            && p != "."
            && p != ".."
    }
    match s.split_once('/') {
        Some((owner, name)) => part_ok(owner) && part_ok(name) && !name.contains('/'),
        None => false,
    }
}

fn is_rfc3339_utc(s: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(s).is_ok()
}

/// Deterministic assessment of the three eligibility questions.
///
/// No I/O, no clock, no randomness. Failures are reported as typed gaps;
/// nothing is inferred from configuration alone, and proof that lives
/// outside this module (e.g. operator egress authorization, remote
/// commit resolvability) is reported as `unverified` rather than assumed.
#[must_use]
pub fn assess_strict_soma_eligibility(report: &ModelRepairReport) -> StrictSomaEligibility {
    let mut gaps: Vec<SomaEligibilityGap> = Vec::new();
    let mut gap = |predicate: &str, kind: &str, reason: String| {
        gaps.push(SomaEligibilityGap {
            predicate: predicate.to_string(),
            kind: kind.to_string(),
            reason,
        });
    };

    // ---- A. Internal report validity ----------------------------------
    let internal_report_valid = report.verify_structure().is_ok();
    if !internal_report_valid {
        gap(
            "internal.reportValid",
            "invalid",
            "the report does not verify internally (schema, receipts, digests or identities contradict); see ModelRepairReport::verify".to_string(),
        );
    }

    // ---- B. Execution provenance completeness --------------------------
    let mut provenance_ok = true;
    let mut mark = |ok: bool, predicate: &str, kind: &str, reason: String| {
        if !ok {
            provenance_ok = false;
            gap(predicate, kind, reason);
        }
    };

    // Unique, well-formed, consistently bound execution identity.
    let exec_ok = report.execution_id.len() == 37
        && report.execution_id.starts_with("exec-")
        && report.execution_id[5..]
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase());
    mark(
        exec_ok,
        "execution_id.format",
        if report.execution_id.trim().is_empty() {
            "missing"
        } else {
            "invalid"
        },
        "executionId must be `exec-` followed by 32 lowercase hex characters".to_string(),
    );

    let source = report.source.as_ref();
    mark(
        source.is_some(),
        "source.present",
        "missing",
        "no SOMA source binding configured (source.repository/revision/issueNumber); a task slug is not a repository".to_string(),
    );
    if let Some(source) = source {
        mark(
            is_valid_github_slug(source.repository.trim()),
            "source.repository",
            if source.repository.trim().is_empty() {
                "missing"
            } else {
                "invalid"
            },
            format!(
                "repository {:?} is not GitHub grammar owner/name",
                source.repository
            ),
        );
        let revision = source.repository_revision.trim();
        mark(
            is_hex_revision(revision),
            "source.repositoryRevision",
            if revision.is_empty() {
                "missing"
            } else {
                "invalid"
            },
            "repositoryRevision must be a 40- or 64-character hex commit sha".to_string(),
        );
        mark(
            is_hex_revision(revision) && revision == report.base_revision,
            "source.revisionPinned",
            "invalid",
            format!(
                "source repository revision {:?} does not match the pinned base revision {:?}",
                source.repository_revision, report.base_revision
            ),
        );
        let (issue_ok, issue_kind, issue_reason) = match source.issue_number {
            None => (
                false,
                "missing",
                "no tracked issue identity configured; a task slug is not an issue number"
                    .to_string(),
            ),
            Some(0) => (
                false,
                "invalid",
                "issueNumber must be a positive tracked issue number".to_string(),
            ),
            Some(_) => (true, "missing", String::new()),
        };
        mark(issue_ok, "source.issueNumber", issue_kind, issue_reason);
    }

    // WorkEventBatch: exists, and its recorded digest matches the re-derived bytes.
    match &report.events_sha256 {
        None => mark(
            false,
            "events.present",
            "missing",
            "no events emission sealed".to_string(),
        ),
        Some(digest) => {
            let rederived = hex_sha256(&serde_json::to_vec(&batch_of(report)).unwrap_or_default());
            let bound_consistently = exec_ok
                && batch_of(report)
                    .events
                    .iter()
                    .all(|e| e.correlation_id == report.execution_id);
            mark(
                is_hex64(digest) && *digest == rederived,
                "events.digestMatch",
                "invalid",
                "eventsDigest does not re-derive from the WorkEventBatch bound to this report"
                    .to_string(),
            );
            mark(
                bound_consistently,
                "events.executionConsistent",
                "invalid",
                "every WorkEvent must bind to this run's executionId as correlationId".to_string(),
            );
        }
    }

    // Provider-issued response identity on the sealing (last) attempt.
    let sealing = report.attempts.last();
    let provider_id = sealing
        .and_then(|a| a.provider_response_id.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    mark(
        provider_id.is_some(),
        "provider.responseId",
        "missing",
        "the responder did not expose a response/run id on the sealing attempt; not sealable for the strict SOMA provenance path".to_string(),
    );

    // Operator-supplied egress authorization. This module can never
    // synthesize or verify grant scope offline; absence is `unverified`,
    // never `authorized`.
    match &report.egress_grant {
        Some(evidence) => {
            let ok = is_hex64(evidence.receipt_sha256.trim())
                && !evidence.grant_ref.trim().is_empty()
                && is_rfc3339_utc(evidence.authorized_at.trim());
            mark(
                ok,
                "egress.grant",
                "invalid",
                "egress grant evidence is present but malformed (receipt_sha256 must be hex64, grantRef nonempty, authorizedAt RFC 3339)".to_string(),
            );
        }
        None => mark(
            false,
            "egress.grant",
            "unverified",
            "no operator-supplied egress grant receipt was recorded; configuration is not evidence, and the runner never synthesizes one".to_string(),
        ),
    }

    // ---- C. Strict success eligibility ---------------------------------
    // A failed or refused run is never a successful coding repair. The
    // failure report remains valid failure evidence (A) only.
    let repaired = report.status == ModelRepairStatus::Repaired;
    if !repaired {
        gap(
            "status.repaired",
            "invalid",
            format!(
                "run outcome is {:?}; an unsuccessful run is failure evidence and never a successful coding repair",
                report.status
            ),
        );
    }

    // Actual patch artifact: a sealed diff digest bound to the repaired tree.
    match &report.final_diff_sha256 {
        Some(digest) => {
            if !is_hex64(digest) {
                gap(
                    "patch.present",
                    "invalid",
                    "finalDiffSha256 is not a hex64 digest".to_string(),
                );
            } else if let Some(excerpt) = &report.final_diff_excerpt {
                // The excerpt equals the full diff when the diff fit the cap;
                // recompute its digest to detect tampering of retained bytes.
                let excerpt_truncated = excerpt.contains("more bytes truncated");
                if !excerpt_truncated {
                    let recomputed = hex_sha256(excerpt.as_bytes());
                    if recomputed != *digest {
                        gap(
                            "patch.excerptDigest",
                            "invalid",
                            "retained patch excerpt digest does not match finalDiffSha256"
                                .to_string(),
                        );
                    }
                }
            }
        }
        None => gap(
            "patch.present",
            "missing",
            "no sealed patch artifact (finalDiffSha256); the run is not in a repaired outcome"
                .to_string(),
        ),
    }
    if repaired
        && report
            .final_tree_digest
            .as_deref()
            .is_none_or(|d| !is_hex64(d))
    {
        gap(
            "workspace.binding",
            if report.final_tree_digest.is_some() {
                "invalid"
            } else {
                "missing"
            },
            "repaired outcome requires a sealed finalTreeDigest (hex64) binding the patched workspace".to_string(),
        );
    }

    // Validator receipt for the sealing attempt: observed subprocess, pinned exit.
    match &report.baseline_validator {
        Some(baseline) => {
            if baseline.provenance != TaskValidatorProvenance::ObservedSubprocess {
                gap(
                    "validator.baselineObserved",
                    "invalid",
                    "baseline validator receipt is not ObservedSubprocess provenance".to_string(),
                );
            }
        }
        None => gap(
            "validator.baselineObserved",
            "missing",
            "repaired outcome requires the baseline validator receipt".to_string(),
        ),
    }
    match sealing.and_then(|a| a.validator.as_ref()) {
        Some(validator) => {
            let pinned = report.validator.expected_exit_code;
            if validator.provenance != TaskValidatorProvenance::ObservedSubprocess {
                gap(
                    "validator.observed",
                    "invalid",
                    "final validator receipt is not ObservedSubprocess provenance".to_string(),
                );
            }
            if validator.exit_code != Some(pinned) {
                gap(
                    "validator.pinnedExit",
                    "invalid",
                    format!(
                        "final validator exit is {:?}, expected the pinned {}",
                        validator.exit_code, pinned
                    ),
                );
            }
            if validator.timed_out {
                gap(
                    "validator.timedOut",
                    "invalid",
                    "final validator subprocess timed out".to_string(),
                );
            }
        }
        None => gap(
            "validator.observed",
            "missing",
            "repaired outcome requires an observed validator receipt on the sealing attempt"
                .to_string(),
        ),
    }

    let eligible = gaps.is_empty() && repaired && internal_report_valid && provenance_ok;
    StrictSomaEligibility {
        internal_report_valid,
        provenance_complete: provenance_ok,
        eligible,
        gaps,
    }
}

/// Compute (sealable, reasons) from the report. Deterministic.
///
/// `sealable` is true iff [`assess_strict_soma_eligibility`] reports
/// `eligible` — i.e. every mandatory strict `soma.model-coding-run`
/// v1.0.0 envelope predicate holds. Nothing else is claimed by the flag:
/// in particular, a `true` here does not emit the envelope, does not
/// authorize egress, and does not attest external validity of the grant
/// the operator recorded; distribution and emission live outside this
/// module. An unsuccessful run is never `sealable`.
pub fn compute_sealability(report: &ModelRepairReport) -> (bool, Vec<String>) {
    let assessment = assess_strict_soma_eligibility(report);
    (
        assessment.eligible,
        assessment
            .gaps
            .iter()
            .map(|g| format!("{}: {}", g.predicate, g.reason))
            .collect(),
    )
}

impl ModelRepairReport {
    /// SOMA field-level mapping of this report onto the
    /// `soma.model-coding-run` v1.0.0 envelope fields. Rows with status
    /// `gap` carry an explicit reason and never a synthesized value.
    /// This is a derived view; it does not emit the envelope itself.
    /// Every row's verdict is derived from the same assessment that drives
    /// `sealable` ([`assess_strict_soma_eligibility`]), so the mapping can
    /// never claim `present` where the strict verdicts records a gap.
    #[must_use]
    pub fn soma_field_mapping(&self) -> Vec<SomaMappingField> {
        let eligibility = assess_strict_soma_eligibility(self);
        let gap_for = |predicate: &str| eligibility.gaps.iter().find(|g| g.predicate == predicate);
        let mut rows = Vec::new();
        let source = self.source.as_ref();
        let present = |path: &str, value: String, note: &str| SomaMappingField {
            path: path.to_string(),
            status: "present".to_string(),
            value: Some(value),
            note: Some(note.to_string()),
        };
        let gap = |path: &str, reason: &str| SomaMappingField {
            path: path.to_string(),
            status: "gap".to_string(),
            value: None,
            note: Some(reason.to_string()),
        };

        rows.push(match gap_for("source.issueNumber") {
            None => match source.and_then(|s| s.issue_number) {
                Some(n) => present(
                    "source.issueNumber",
                    n.to_string(),
                    "operator-configured, positive tracked issue; not cryptographically verified by this module",
                ),
                None => gap(
                    "source.issueNumber",
                    "issue predicate passed but number is absent (inconsistent state)",
                ),
            },
            Some(g) => gap("source.issueNumber", &g.reason),
        });
        rows.push(match gap_for("source.repository") {
            None => match source {
                Some(s) => present(
                    "source.repository",
                    s.repository.clone(),
                    "operator-configured GitHub-grammar binding at the pinned revision; resolvability not verified by this module",
                ),
                None => gap("source.repository", "repository predicate passed but binding is absent (inconsistent state)"),
            },
            Some(g) => gap("source.repository", &g.reason),
        });
        rows.push(match gap_for("source.repositoryRevision").or_else(|| gap_for("source.revisionPinned")) {
            None => match source {
                Some(s) => present(
                    "source.repositoryRevision",
                    s.repository_revision.clone(),
                    "hex commit sha equal to the pinned base revision; remote resolvability not verified by this module",
                ),
                None => gap("source.repositoryRevision", "revision predicate passed but binding is absent (inconsistent state)"),
            },
            Some(g) => gap("source.repositoryRevision", &g.reason),
        });

        let sealing = self.attempts.last();
        match gap_for("provider.responseId") {
            None => match sealing.and_then(|a| a.provider_response_id.as_deref()) {
                Some(id) => rows.push(present(
                    "model.providerRunId",
                    id.to_string(),
                    sealing
                        .and_then(|a| a.provider_response_id_source.as_deref())
                        .unwrap_or("response-body:id"),
                )),
                None => rows.push(gap(
                    "model.providerRunId",
                    "provider predicate passed but id is absent (inconsistent state)",
                )),
            },
            Some(g) => rows.push(gap("model.providerRunId", &g.reason)),
        }

        rows.push(match &self.egress_grant {
            Some(evidence) if gap_for("egress.grant").is_none() => present(
                "provenance.egressGrant",
                format!("sha256 {}", evidence.receipt_sha256),
                "operator-supplied grant receipt recorded at dispatch; grant scope not re-verifiable by this module",
            ),
            _ => match gap_for("egress.grant") {
                Some(g) => gap(
                    "provenance.egressGrant",
                    &g.reason,
                ),
                None => gap(
                    "provenance.egressGrant",
                    "no operator-supplied egress grant receipt was recorded; configuration is not evidence",
                ),
            },
        });

        rows.push(present(
            "model.harness",
            format!("{}@{}", self.model.harness_id, self.model.harness_revision),
            "harness identity recorded from the harness registration, never from the model",
        ));

        let provider_present = gap_for("provider.responseId").is_none();
        let egress_present = gap_for("egress.grant").is_none();
        let provider_note = match (provider_present, egress_present) {
            (true, true) => {
                "lite-observed-run, modelAuthoredClaim: true, providerRunId and operator egress grant recorded"
            }
            (true, false) => {
                "lite-observed-run, modelAuthoredClaim: true, providerRunId present, egress grant unverified — provenance not sealable for the strict path"
            }
            (false, _) => {
                "lite-observed-run, modelAuthoredClaim: true, providerRunId absent — provenance not sealable for the strict path"
            }
        };
        rows.push(present(
            "provenance",
            "lite-observed-run".to_string(),
            provider_note,
        ));

        // The events row is `present` only when the recorded digest
        // re-derives and binds to this execution identity; the mapping
        // never claims more than the strict assessment does.
        let events_gap = eligibility
            .gaps
            .iter()
            .find(|g| g.predicate.starts_with("events."));
        rows.push(match (&self.events_sha256, events_gap) {
            (Some(digest), None) => present(
                "events",
                format!("WorkEventBatch 1.3.0, {} events, sha256 {}", self.attempts.len() + 1, digest),
                "every event binds to executionId; replay preserves the identity; digest re-derived and matches",
            ),
            (Some(_), Some(g)) => gap("events", &g.reason),
            (None, _) => gap("events", "no events emission sealed"),
        });

        rows.push(match gap_for("workspace.binding") {
            None => match self.final_tree_digest {
                Some(ref digest) => present(
                    "workspace",
                    format!("sha256 {digest}"),
                    "staged git worktree behind the pinned revision; writable scope limited to the repair target",
                ),
                None if self.status == ModelRepairStatus::Repaired => gap(
                    "workspace",
                    "workspace predicate passed but digest is absent (inconsistent state)",
                ),
                None => gap("workspace", "no patched workspace (run is not in a repaired outcome)"),
            },
            Some(g) => gap("workspace", &g.reason),
        });

        rows.push(match gap_for("patch.present").or_else(|| gap_for("patch.excerptDigest")) {
            None => match self.final_diff_sha256 {
                Some(ref digest) if self.status == ModelRepairStatus::Repaired => present(
                    "patch",
                    format!("sha256 {digest}"),
                    "model-authored diff artifact digest; full bytes are not retained by this module — byte-level re-verification happens at SOMA ingestion",
                ),
                Some(_) => gap(
                    "patch",
                    "a patch digest exists but the run outcome is not repaired; an unsuccessful run never presents as a repaired patch",
                ),
                None => gap(
                    "patch",
                    "no accepted patch (run is not in a repaired outcome)",
                ),
            },
            Some(g) => gap("patch", &g.reason),
        });

        // The validation row is `present` only under the strict contract:
        // repaired outcome, baseline receipt observed, final receipt observed
        // and exit code equal to the pinned expectation.
        let validator_gap = eligibility
            .gaps
            .iter()
            .find(|g| g.predicate.starts_with("validator."));
        rows.push(match (&self.baseline_validator, validator_gap) {
            (Some(receipt), None) => present(
                "validation",
                format!(
                    "pinned validator, baseline exit {:?}, final attempt provenance {:?}",
                    receipt.exit_code,
                    self.attempts
                        .last()
                        .and_then(|a| a.validator.as_ref())
                        .map(|v| format!("{:?}", v.provenance))
                        .unwrap_or_else(|| "none".to_string())
                ),
                "validator receipts re-derived at verification; ObservedSubprocess required for repaired",
            ),
            (Some(_), Some(g)) => gap("validation", &g.reason),
            (None, _) => gap("validation", "baseline validator receipt missing"),
        });

        let run_digest = if eligibility.eligible {
            present(
                "runDigest",
                "eligible for emission; envelope not emitted by this module".to_string(),
                "sealable: every mandatory predicate holds; the consumer recomputes runDigest",
            )
        } else {
            gap(
                "runDigest",
                "not eligible: the report fails strict SOMA coding-run predicates (see gaps); an envelope emitted from it would be invalid",
            )
        };
        rows.push(run_digest);

        rows
    }
}

/// One row of the SOMA field mapping (see [`ModelRepairReport::soma_field_mapping`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SomaMappingField {
    pub path: String,
    /// `present` or `gap`.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Deterministic digest of the worktree tree (paths and blob ids; `.git`
/// excluded). Mirrors the executed-scenario tree-digest discipline.
///
/// # Errors
/// Returns a message when the tree cannot be walked.
pub fn tree_digest(worktree: &Path) -> Result<String, String> {
    let mut entries = Vec::new();
    collect_tree_entries(worktree, Path::new(""), &mut entries).map_err(|e| e.to_string())?;
    entries.sort();
    let canonical = entries.join("\n");
    Ok(hex_sha256(canonical.as_bytes()))
}

fn collect_tree_entries(root: &Path, rel: &Path, entries: &mut Vec<String>) -> std::io::Result<()> {
    for component in std::fs::read_dir(root.join(rel))?.flatten() {
        let file_type = component.file_type()?;
        let entry_path = rel.join(component.file_name());
        if file_type.is_dir() {
            // Skip VCS metadata and build artifacts; neither is task content.
            let name = component.file_name();
            if name == ".git"
                || name == "target"
                || name.to_string_lossy().starts_with(".lite-validator-")
            {
                continue;
            }
            collect_tree_entries(root, &entry_path, entries)?;
        } else if file_type.is_file() {
            let rel_slash = entry_path
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            let bytes = std::fs::read(root.join(&entry_path))?;
            entries.push(format!("{}:{}", rel_slash, hex_sha256(&bytes)));
        }
    }
    Ok(())
}

fn hex_sha256(bytes: &[u8]) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

// ---------------------------------------------------------------------------
// Model proposers
// ---------------------------------------------------------------------------

/// Why the model did not produce a usable response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelProposeError {
    /// The endpoint, credential or transport failed before any effect.
    Unavailable(String),
    /// The provider explicitly refused (policy/moderation/no-content).
    Refused(String),
    /// The request/response exceeded an operator budget.
    Budget(String),
}

impl std::fmt::Display for ModelProposeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(m) | Self::Refused(m) | Self::Budget(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for ModelProposeError {}

/// Source of model proposals. Implementations must route through the
/// governed effect path; nothing in this trait grants write authority.
pub trait ModelProposer {
    /// The descriptor recorded in the report (no credentials, ever).
    fn descriptor(&self) -> ModelDescriptor;
    /// Issue one proposal. The prompt is already bounded and redacted.
    /// The returned [`ProposedCompletion`] carries whatever provider
    /// provenance the response boundary observed — possibly none.
    fn propose(&mut self, prompt: &str) -> Result<ProposedCompletion, ModelProposeError>;
}

/// Deterministic in-process scripted proposer for the hermetic suite.
/// **Not** a stand-in for a live model run; it exists so the repair loop
/// can be verified end to end offline.
pub struct ScriptedProposer {
    responses: std::collections::VecDeque<Result<ProposedCompletion, ModelProposeError>>,
    descriptor: ModelDescriptor,
    /// Every prompt observed by the double, in order.
    pub prompts: Vec<String>,
}

impl ScriptedProposer {
    /// Scripted raw-string responses without provider provenance (content
    /// only). Convenience wrapper for the common case.
    pub fn new(
        descriptor: ModelDescriptor,
        responses: Vec<Result<String, ModelProposeError>>,
    ) -> Self {
        Self::with_completions(
            descriptor,
            responses
                .into_iter()
                .map(|r| r.map(ProposedCompletion::with_content))
                .collect(),
        )
    }

    /// Scripted completions with full provider provenance control.
    pub fn with_completions(
        descriptor: ModelDescriptor,
        responses: Vec<Result<ProposedCompletion, ModelProposeError>>,
    ) -> Self {
        Self {
            responses: responses.into(),
            descriptor,
            prompts: Vec::new(),
        }
    }
}

impl ModelProposer for ScriptedProposer {
    fn descriptor(&self) -> ModelDescriptor {
        self.descriptor.clone()
    }

    fn propose(&mut self, prompt: &str) -> Result<ProposedCompletion, ModelProposeError> {
        self.prompts.push(prompt.to_string());
        self.responses
            .pop_front()
            .unwrap_or(Err(ModelProposeError::Budget(
                "scripted proposer: no further responses".to_string(),
            )))
    }
}

// ---------------------------------------------------------------------------
// The repair loop
// ---------------------------------------------------------------------------

fn fence_strip(raw: &str) -> &str {
    let trimmed = raw.trim();
    if let Some(inner) = trimmed.strip_prefix("```diff") {
        return inner.trim_end_matches("```").trim();
    }
    if let Some(inner) = trimmed.strip_prefix("```") {
        return inner.trim_end_matches("```").trim();
    }
    trimmed
}

fn repo_relative_ok(worktree: &Path, raw: &str) -> Result<(PathBuf, String), String> {
    let cleaned = validate_repo_relative_path(Path::new(raw))
        .map_err(|e| format!("edit path is not repo-relative safe: {e}"))?;
    let resolved = resolve_repo_path(worktree, &cleaned)
        .map_err(|e| format!("edit path escapes the worktree: {e}"))?;
    let rel_slash = cleaned
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    Ok((resolved, rel_slash))
}

fn diff_against_baseline(worktree: &Path, target: &str) -> Option<String> {
    let out = Command::new("git")
        .args(["diff", "--no-color", "--no-ext-diff", "--", target])
        .current_dir(worktree)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

fn excerpt(bytes: &[u8], cap: usize) -> String {
    let text = String::from_utf8_lossy(bytes);
    let total = text.len();
    if total <= cap {
        text.into_owned()
    } else {
        format!(
            "{}\n[... {} more bytes truncated ...]",
            &text[..cap],
            total - cap
        )
    }
}

/// Run the pinned validator once, retaining a bounded excerpt for model
/// feedback. The receipt's `ObservedSubprocess` provenance is what lets the
/// sealed report claim a repaired outcome.
fn run_validator_feedback(
    spec: &ValidatorSpec,
    worktree: &Path,
    target_dir: &Path,
    timeout: Duration,
) -> (TaskValidatorRecord, String) {
    fn drain(
        reader: Option<Box<dyn std::io::Read + Send>>,
    ) -> std::thread::JoinHandle<(Vec<u8>, u64)> {
        std::thread::spawn(move || -> (Vec<u8>, u64) {
            let mut buf = Vec::new();
            let mut total: u64 = 0;
            if let Some(mut reader) = reader {
                let mut chunk = [0_u8; 8192];
                loop {
                    match reader.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            total += n as u64;
                            if buf.len() < VALIDATOR_OUTPUT_CAP {
                                let keep = (VALIDATOR_OUTPUT_CAP - buf.len()).min(n);
                                buf.extend_from_slice(&chunk[..keep]);
                            }
                        }
                    }
                }
            }
            (buf, total)
        })
    }

    let started = Instant::now();
    let spawn = Command::new(&spec.command)
        .args(&spec.args)
        .current_dir(worktree)
        .env("CARGO_TARGET_DIR", target_dir)
        .env("CARGO_NET_OFFLINE", "true")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let child = match spawn {
        Ok(child) => child,
        Err(_) => {
            return (
                TaskValidatorRecord {
                    command: spec.command.clone(),
                    args: spec.args.clone(),
                    expected_exit_code: spec.expected_exit_code,
                    assertion: spec.assertion.clone(),
                    exit_code: None,
                    timed_out: false,
                    output_digest: None,
                    output_bytes: 0,
                    duration_ms: started.elapsed().as_millis() as u64,
                    provenance: TaskValidatorProvenance::NotExecuted,
                },
                String::new(),
            );
        }
    };
    let mut child = child;
    let out_thread = drain(
        child
            .stdout
            .take()
            .map(|o| Box::new(o) as Box<dyn std::io::Read + Send>),
    );
    let err_thread = drain(
        child
            .stderr
            .take()
            .map(|e| Box::new(e) as Box<dyn std::io::Read + Send>),
    );
    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() > deadline {
                    timed_out = true;
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => break None,
        }
    };
    let (out_bytes, out_total) = out_thread.join().unwrap_or_default();
    let (err_bytes, err_total) = err_thread.join().unwrap_or_default();
    let mut digest_input = Vec::with_capacity(out_bytes.len() + err_bytes.len());
    digest_input.extend_from_slice(&out_bytes);
    digest_input.extend_from_slice(&err_bytes);
    let excerpt = excerpt(&digest_input, MAX_FEEDBACK_BYTES);
    (
        TaskValidatorRecord {
            command: spec.command.clone(),
            args: spec.args.clone(),
            expected_exit_code: spec.expected_exit_code,
            assertion: spec.assertion.clone(),
            exit_code: status.and_then(|s| s.code()),
            timed_out,
            output_digest: Some(hex_sha256(&digest_input)),
            output_bytes: out_total + err_total,
            duration_ms: started.elapsed().as_millis() as u64,
            provenance: TaskValidatorProvenance::ObservedSubprocess,
        },
        excerpt,
    )
}

fn scripted_grant(worktree: &Path, repair_target: &str) -> (FilePolicy, FileSet) {
    let mut policy = FilePolicy::strict();
    policy.repo_root = worktree.to_path_buf();
    policy.allowed_write_paths = vec![PathBuf::from(repair_target)];
    let file_set = FileSet {
        editable: vec![worktree.join(Path::new(repair_target))],
        readonly: vec![],
        generated: vec![],
        artifacts: vec![],
        denied: vec![],
        binary: vec![],
    };
    (policy, file_set)
}

fn authorize_edits(
    worktree: &Path,
    repair_target: &str,
    edits: &[EditOperation],
) -> Result<Vec<String>, String> {
    let target_abs = resolve_repo_path(worktree, Path::new(repair_target))
        .map_err(|e| format!("repair target refused: {e}"))?;
    let mut proposed = BTreeSet::new();
    for edit in edits {
        let raw: &PathBuf = match edit {
            EditOperation::UnifiedDiff(op) => op.target_file.as_ref().ok_or_else(|| {
                "unified diff without an explicit target file is not permitted".to_string()
            })?,
            EditOperation::SearchReplace(op) => &op.file,
            EditOperation::WholeFile(op) => &op.file,
            EditOperation::CreateFile(op) => &op.file,
            EditOperation::DeleteFile(_) => {
                return Err("delete edits are not permitted for this task".to_string());
            }
            EditOperation::RenameFile(_) => {
                return Err("rename edits are not permitted for this task".to_string());
            }
        };
        let (resolved, rel_slash) =
            repo_relative_ok(worktree, &raw.to_string_lossy().replace('\\', "/"))?;
        if resolved != target_abs {
            return Err(format!(
                "edit path {rel_slash:?} is outside the authorized repair target"
            ));
        }
        proposed.insert(rel_slash);
    }
    Ok(proposed.into_iter().collect())
}

fn build_prompt(
    issue_statement: &str,
    repair_target: &str,
    target_text: &str,
    feedback: Option<&str>,
    context: Option<&PromptContext>,
) -> String {
    let mut prompt = String::with_capacity(target_text.len() + 2 * 1024);
    prompt.push_str(
        "You are a coding agent working on a real repository. Fix the bug described by the issue.\n\n\
         RULES:\n\
         - Change ONLY the file `",
    );
    prompt.push_str(repair_target);
    prompt.push_str(
        "`. No other file may be modified, created, renamed or deleted.\n\
         - Reply with EXACTLY ONE fenced code block containing a unified diff. No prose before or after.\n\
         - The diff must apply cleanly with `git apply`. Keep it minimal.\n\
         - The existing test suite must keep passing; do not delete or weaken tests.\n\n\
         ISSUE:\n",
    );
    prompt.push_str(issue_statement);
    match context {
        Some(context) => {
            prompt.push_str("\n\nCURRENT FILE EXCERPT (");
            prompt.push_str(&context.note);
            prompt.push_str("; verbatim; the full file is larger than the wire budget, so this excerpt shows the region to change). Hunk line numbers must be absolute file line numbers:\n```rust\n");
            prompt.push_str(&context.text);
            prompt.push_str("\n```\n");
        }
        None => {
            prompt.push_str("\n\nCURRENT FILE CONTENT (verbatim):\n```rust\n");
            prompt.push_str(target_text);
            prompt.push_str("\n```\n");
        }
    }
    if let Some(feedback) = feedback {
        prompt.push_str("\nFEEDBACK FROM THE PREVIOUS ATTEMPT:\n");
        prompt.push_str(feedback);
        prompt.push('\n');
    }
    prompt
}

/// Execute the model-authored repair under the governed boundary. The
/// model dispatch happens only after the caller's pins verify.
pub async fn run_model_repair(
    spec: &ModelRepairSpec,
    mut proposer: impl ModelProposer,
    cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> ModelRepairReport {
    let started = Instant::now();
    let execution_id = format!("exec-{}", uuid::Uuid::new_v4().simple());
    let mut report = ModelRepairReport {
        schema_version: MODEL_REPAIR_REPORT_SCHEMA.to_string(),
        kind: "model-authored-repair".to_string(),
        task_id: spec.task_id.clone(),
        base_revision: spec.base_revision.clone(),
        repair_target_sha256: spec.repair_target_sha256.clone(),
        repair_target: spec.repair_target.clone(),
        model: proposer.descriptor(),
        validator: spec.validator.clone(),
        status: ModelRepairStatus::SetupError,
        attempts: Vec::new(),
        execution_id,
        source: spec.source.clone(),
        egress_grant: spec.egress_grant.clone(),
        baseline_validator: None,
        final_tree_digest: None,
        final_diff_sha256: None,
        final_diff_excerpt: None,
        prompt_bytes_total: 0,
        wall_ms: 0,
        sealed_at: String::new(),
        events_sha256: None,
        sealable: false,
        sealability_reasons: Vec::new(),
    };

    // ---- Preflight pins: the staged target must match the pin exactly.
    let target_path = match repo_relative_ok(&spec.worktree, &spec.repair_target) {
        Ok((resolved, _)) => resolved,
        Err(reason) => {
            report.attempts.push(setup_refusal(&reason));
            report.wall_ms = started.elapsed().as_millis() as u64;
            report.sealed_at = rfc3339_now();
            return sealed_return(report);
        }
    };
    let target_bytes = match std::fs::read(&target_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            report.attempts.push(setup_refusal(&format!(
                "staged repair target unreadable: {error}"
            )));
            report.wall_ms = started.elapsed().as_millis() as u64;
            report.sealed_at = rfc3339_now();
            return sealed_return(report);
        }
    };
    let pinned_digest = hex_sha256(&target_bytes);
    if pinned_digest != spec.repair_target_sha256 {
        report.attempts.push(setup_refusal(&format!(
            "staged repair target digest {pinned_digest} does not match the pinned digest {}",
            spec.repair_target_sha256
        )));
        report.wall_ms = started.elapsed().as_millis() as u64;
        report.sealed_at = rfc3339_now();
        return sealed_return(report);
    }
    match tree_digest(&spec.worktree) {
        Ok(tree) if tree == spec.workspace_fingerprint => {}
        Ok(tree) => {
            report.attempts.push(setup_refusal(&format!(
                "staged workspace fingerprint {tree} does not match the pinned fingerprint {}",
                spec.workspace_fingerprint
            )));
            report.wall_ms = started.elapsed().as_millis() as u64;
            report.sealed_at = rfc3339_now();
            return sealed_return(report);
        }
        Err(reason) => {
            report.attempts.push(setup_refusal(&reason));
            report.wall_ms = started.elapsed().as_millis() as u64;
            report.sealed_at = rfc3339_now();
            return sealed_return(report);
        }
    }

    // ---- Baseline validator run (the pinned task's pre-fix behavior).
    let validator_namespace = hex_sha256(
        format!(
            "{}\u{0}{}\u{0}{}",
            spec.worktree.display(),
            spec.workspace_fingerprint,
            spec.task_id
        )
        .as_bytes(),
    );
    let baseline_target_dir = spec.validator_target_dir.clone().unwrap_or_else(|| {
        std::env::temp_dir().join(format!(
            "lite-validator-baseline-{}-{}",
            std::process::id(),
            &validator_namespace[..16]
        ))
    });
    let (baseline_receipt, _) = run_validator_feedback(
        &spec.validator,
        &spec.worktree,
        &baseline_target_dir,
        spec.validator_timeout,
    );
    report.baseline_validator = Some(baseline_receipt.clone());

    let mut feedback = Some(format!(
        "pre-fix validator run: exit code {:?} (provenance: {:?}); stdout+stderr digest {}",
        baseline_receipt.exit_code,
        baseline_receipt.provenance,
        baseline_receipt
            .output_digest
            .as_deref()
            .unwrap_or("unavailable")
    ));

    let mut total_prompt_bytes: u64 = 0;
    for attempt_index in 0..spec.max_attempts {
        if cancel_requested(&cancel) {
            report.status = ModelRepairStatus::Interrupted;
            report.wall_ms = started.elapsed().as_millis() as u64;
            report.sealed_at = rfc3339_now();
            report.prompt_bytes_total = total_prompt_bytes;
            report.final_tree_digest = tree_digest(&spec.worktree).ok();
            return sealed_return(report);
        }
        if total_prompt_bytes >= MAX_PROMPT_BYTES_TOTAL {
            report.attempts.push(attempt_refusal(
                attempt_index + 1,
                ModelRepairStatus::BudgetExhausted.as_str(),
            ));
            report.status = ModelRepairStatus::BudgetExhausted;
            break;
        }
        let target_text = match std::fs::read_to_string(&target_path) {
            Ok(text) => text,
            Err(error) => {
                report.attempts.push(setup_refusal(&format!(
                    "staged repair target unreadable at attempt {attempt_index}: {error}"
                )));
                report.status = ModelRepairStatus::SetupError;
                break;
            }
        };
        let prompt = build_prompt(
            &spec.issue_statement,
            &spec.repair_target,
            &target_text,
            feedback.as_deref(),
            spec.prompt_context.as_ref(),
        );
        let prompt_bytes = prompt.len() as u64;
        if prompt_bytes as usize > MAX_PROMPT_BYTES
            || total_prompt_bytes.saturating_add(prompt_bytes) > MAX_PROMPT_BYTES_TOTAL
        {
            report
                .attempts
                .push(attempt_refusal(attempt_index + 1, "prompt-budget-exceeded"));
            total_prompt_bytes = total_prompt_bytes.saturating_add(prompt_bytes);
            report.status = ModelRepairStatus::BudgetExhausted;
            break;
        }
        total_prompt_bytes = total_prompt_bytes.saturating_add(prompt_bytes);
        let prompt_sha = hex_sha256(prompt.as_bytes());

        let attempt_started = Instant::now();
        let response = proposer.propose(&prompt);
        match response {
            Err(error) => {
                let (refusal, status) = match error {
                    ModelProposeError::Refused(_) => {
                        ("model-refusal", ModelRepairStatus::ModelRefusal)
                    }
                    ModelProposeError::Budget(_) => {
                        ("budget-exceeded", ModelRepairStatus::BudgetExhausted)
                    }
                    ModelProposeError::Unavailable(_) => (
                        "provider-unavailable",
                        ModelRepairStatus::ProviderUnavailable,
                    ),
                };
                report.attempts.push(ModelRepairAttempt {
                    attempt: attempt_index + 1,
                    prompt_sha256: prompt_sha,
                    prompt_bytes,
                    response_sha256: None,
                    response_bytes: None,
                    response_excerpt: None,
                    proposed_files: Vec::new(),
                    applied: false,
                    rolled_back: false,
                    validator: None,
                    refusal: Some(refusal.to_string()),
                    provider_response_id: None,
                    provider_response_id_source: None,
                    gateway_trace_id: None,
                    local_request_id: None,
                    wall_ms: attempt_started.elapsed().as_millis() as u64,
                });
                report.status = status;
                break;
            }
            Ok(completion) => {
                let provider_response_id = completion.provider_response_id.clone();
                let provider_response_id_source = completion.provider_response_id_source.clone();
                let gateway_trace_id = completion.gateway_trace_id.clone();
                let local_request_id = completion.local_request_id.clone();
                let text = completion.content;
                let bounded: String = if text.len() > MAX_RESPONSE_BYTES {
                    text.chars().take(MAX_RESPONSE_BYTES).collect()
                } else {
                    text
                };
                // Evidence records only the redacted artifact; the raw
                // provider text never reaches the report.
                let bounded = redact(&bounded);
                let response_sha = hex_sha256(bounded.as_bytes());
                let response_bytes = bounded.len() as u64;
                let response_excerpt = bounded.chars().take(MAX_DIFF_BYTES).collect::<String>();

                // Parse + authorize (only the pinned repair target).
                let edits = parse_edit_response(fence_strip(&bounded));
                let edits = match edits {
                    Ok(edits) if !edits.is_empty() => edits,
                    Ok(_) => {
                        report.attempts.push(ModelRepairAttempt {
                            attempt: attempt_index + 1,
                            prompt_sha256: prompt_sha,
                            prompt_bytes,
                            response_sha256: Some(response_sha.clone()),
                            response_bytes: Some(response_bytes),
                            response_excerpt: Some(response_excerpt),
                            proposed_files: Vec::new(),
                            applied: false,
                            rolled_back: false,
                            validator: None,
                            refusal: Some("unparseable-proposal".to_string()),
                            provider_response_id: provider_response_id.clone(),
                            provider_response_id_source: provider_response_id_source.clone(),
                            gateway_trace_id: gateway_trace_id.clone(),
                            local_request_id: local_request_id.clone(),
                            wall_ms: attempt_started.elapsed().as_millis() as u64,
                        });
                        feedback = Some(
                            "the response contained no edits; reply with exactly one fenced unified diff"
                                .to_string(),
                        );
                        continue;
                    }
                    Err(error) => {
                        report.attempts.push(ModelRepairAttempt {
                            attempt: attempt_index + 1,
                            prompt_sha256: prompt_sha,
                            prompt_bytes,
                            response_sha256: Some(response_sha.clone()),
                            response_bytes: Some(response_bytes),
                            response_excerpt: Some(response_excerpt),
                            proposed_files: Vec::new(),
                            applied: false,
                            rolled_back: false,
                            validator: None,
                            refusal: Some("unparseable-proposal".to_string()),
                            provider_response_id: provider_response_id.clone(),
                            provider_response_id_source: provider_response_id_source.clone(),
                            gateway_trace_id: gateway_trace_id.clone(),
                            local_request_id: local_request_id.clone(),
                            wall_ms: attempt_started.elapsed().as_millis() as u64,
                        });
                        feedback = Some(format!(
                            "the response was not a parseable edit document: {error}. Reply with exactly one fenced unified diff."
                        ));
                        continue;
                    }
                };
                let proposed_files =
                    match authorize_edits(&spec.worktree, &spec.repair_target, &edits) {
                        Ok(files) => files,
                        Err(_reason) => {
                            report.attempts.push(ModelRepairAttempt {
                                attempt: attempt_index + 1,
                                prompt_sha256: prompt_sha,
                                prompt_bytes,
                                response_sha256: Some(response_sha.clone()),
                                response_bytes: Some(response_bytes),
                                response_excerpt: Some(response_excerpt),
                                proposed_files: Vec::new(),
                                applied: false,
                                rolled_back: false,
                                validator: None,
                                refusal: Some("unauthorized-proposal".to_string()),
                                provider_response_id: provider_response_id.clone(),
                                provider_response_id_source: provider_response_id_source.clone(),
                                gateway_trace_id: gateway_trace_id.clone(),
                                local_request_id: local_request_id.clone(),
                                wall_ms: attempt_started.elapsed().as_millis() as u64,
                            });
                            report.status = ModelRepairStatus::UnauthorizedProposal;
                            break;
                        }
                    };

                // Dry-run preflight (read-only), then apply with rollback.
                let (policy, file_set) = scripted_grant(&spec.worktree, &spec.repair_target);
                let dry_run = dry_run_patch(&edits, &file_set, &policy).await;
                match dry_run {
                    Ok(result) if result.failures.is_empty() => {}
                    Ok(result) => {
                        let reason = result
                            .failures
                            .iter()
                            .map(|f| format!("{}: {}", f.file.display(), f.reason))
                            .collect::<Vec<_>>()
                            .join("; ");
                        report.attempts.push(ModelRepairAttempt {
                            attempt: attempt_index + 1,
                            prompt_sha256: prompt_sha,
                            prompt_bytes,
                            response_sha256: Some(response_sha.clone()),
                            response_bytes: Some(response_bytes),
                            response_excerpt: Some(response_excerpt),
                            proposed_files,
                            applied: false,
                            rolled_back: false,
                            validator: None,
                            refusal: Some(format!("dry-run-rejected: {reason}")),
                            provider_response_id: provider_response_id.clone(),
                            provider_response_id_source: provider_response_id_source.clone(),
                            gateway_trace_id: gateway_trace_id.clone(),
                            local_request_id: local_request_id.clone(),
                            wall_ms: attempt_started.elapsed().as_millis() as u64,
                        });
                        feedback = Some(format!(
                            "the proposed patch does not apply cleanly to the pinned bytes: {reason}"
                        ));
                        continue;
                    }
                    Err(error) => {
                        let reason = format!("{error:#}");
                        report.attempts.push(ModelRepairAttempt {
                            attempt: attempt_index + 1,
                            prompt_sha256: prompt_sha,
                            prompt_bytes,
                            response_sha256: Some(response_sha.clone()),
                            response_bytes: Some(response_bytes),
                            response_excerpt: Some(response_excerpt),
                            proposed_files,
                            applied: false,
                            rolled_back: false,
                            validator: None,
                            refusal: Some(format!("dry-run-refused: {reason}")),
                            provider_response_id: provider_response_id.clone(),
                            provider_response_id_source: provider_response_id_source.clone(),
                            gateway_trace_id: gateway_trace_id.clone(),
                            local_request_id: local_request_id.clone(),
                            wall_ms: attempt_started.elapsed().as_millis() as u64,
                        });
                        feedback = Some(format!("patch preflight refused: {reason}"));
                        continue;
                    }
                }

                let apply = apply_patch_with_rollback(&edits, &file_set, &policy).await;
                let rollback = match apply {
                    Ok((_patch_result, rollback)) => rollback,
                    Err(error) => {
                        let reason = format!("{error:#}");
                        report.attempts.push(ModelRepairAttempt {
                            attempt: attempt_index + 1,
                            prompt_sha256: prompt_sha,
                            prompt_bytes,
                            response_sha256: Some(response_sha.clone()),
                            response_bytes: Some(response_bytes),
                            response_excerpt: Some(response_excerpt),
                            proposed_files,
                            applied: false,
                            rolled_back: false,
                            validator: None,
                            refusal: Some(format!("apply-refused: {reason}")),
                            provider_response_id: provider_response_id.clone(),
                            provider_response_id_source: provider_response_id_source.clone(),
                            gateway_trace_id: gateway_trace_id.clone(),
                            local_request_id: local_request_id.clone(),
                            wall_ms: attempt_started.elapsed().as_millis() as u64,
                        });
                        feedback = Some(format!("patch application refused: {reason}"));
                        continue;
                    }
                };

                // Validate (real subprocess). Restore the pinned bytes on failure.
                let target_dir = spec.validator_target_dir.clone().unwrap_or_else(|| {
                    std::env::temp_dir().join(format!(
                        "lite-validator-attempt-{}-{}-{}",
                        attempt_index + 1,
                        std::process::id(),
                        &validator_namespace[..16]
                    ))
                });
                let (receipt, excerpt_text) = run_validator_feedback(
                    &spec.validator,
                    &spec.worktree,
                    &target_dir,
                    spec.validator_timeout,
                );
                let ok = receipt.exit_code == Some(spec.validator.expected_exit_code);
                let rolled_back = if ok {
                    false
                } else {
                    let _ = rollback.rollback().await;
                    true
                };
                report.attempts.push(ModelRepairAttempt {
                    attempt: attempt_index + 1,
                    prompt_sha256: prompt_sha,
                    prompt_bytes,
                    response_sha256: Some(response_sha),
                    response_bytes: Some(response_bytes),
                    response_excerpt: Some(response_excerpt),
                    proposed_files,
                    applied: true,
                    rolled_back,
                    validator: Some(receipt),
                    refusal: if ok {
                        None
                    } else {
                        Some("validator-rejected".to_string())
                    },
                    provider_response_id: provider_response_id.clone(),
                    provider_response_id_source: provider_response_id_source.clone(),
                    gateway_trace_id: gateway_trace_id.clone(),
                    local_request_id: local_request_id.clone(),
                    wall_ms: attempt_started.elapsed().as_millis() as u64,
                });
                if ok {
                    report.status = ModelRepairStatus::Repaired;
                    report.final_tree_digest = tree_digest(&spec.worktree).ok();
                    let diff = diff_against_baseline(&spec.worktree, &spec.repair_target);
                    if let Some(diff) = diff {
                        report.final_diff_sha256 = Some(hex_sha256(diff.as_bytes()));
                        report.final_diff_excerpt = Some(excerpt(diff.as_bytes(), MAX_DIFF_BYTES));
                    }
                    break;
                }
                feedback = Some(format!(
                    "validator exit code was not the pinned {pinned}. Captured excerpt:\n{excerpt}",
                    pinned = spec.validator.expected_exit_code,
                    excerpt = excerpt_text,
                ));
            }
        }
    }

    // Setup-error runs with only setup refusals stay SetupError.
    if !matches!(
        report.status,
        ModelRepairStatus::Repaired
            | ModelRepairStatus::BudgetExhausted
            | ModelRepairStatus::Interrupted
            | ModelRepairStatus::ModelRefusal
            | ModelRepairStatus::ProviderUnavailable
            | ModelRepairStatus::UnauthorizedProposal
    ) {
        let any_validator_rejected = report
            .attempts
            .iter()
            .any(|a| a.refusal.as_deref() == Some("validator-rejected"));
        let all_setup_errors = !report.attempts.is_empty()
            && report.attempts.iter().all(|a| {
                a.refusal
                    .as_deref()
                    .unwrap_or("")
                    .starts_with("setup-error: ")
            });
        report.status = if any_validator_rejected {
            ModelRepairStatus::ValidatorRejected
        } else if all_setup_errors {
            ModelRepairStatus::SetupError
        } else {
            ModelRepairStatus::BudgetExhausted
        };
    }

    report.prompt_bytes_total = total_prompt_bytes;
    report.wall_ms = started.elapsed().as_millis() as u64;
    report.sealed_at = rfc3339_now();
    if report.final_tree_digest.is_none() && report.status == ModelRepairStatus::Repaired {
        report.final_tree_digest = tree_digest(&spec.worktree).ok();
    }
    seal_report(&mut report);
    report
}

fn cancel_requested(cancel: &Option<std::sync::Arc<std::sync::atomic::AtomicBool>>) -> bool {
    cancel
        .as_ref()
        .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::SeqCst))
}

fn setup_refusal(reason: &str) -> ModelRepairAttempt {
    ModelRepairAttempt {
        attempt: 0,
        prompt_sha256: hex_sha256(reason.as_bytes()),
        prompt_bytes: reason.len() as u64,
        response_sha256: None,
        response_bytes: None,
        response_excerpt: None,
        proposed_files: Vec::new(),
        applied: false,
        rolled_back: true,
        validator: None,
        refusal: Some(format!("setup-error: {reason}")),
        provider_response_id: None,
        provider_response_id_source: None,
        gateway_trace_id: None,
        local_request_id: None,
        wall_ms: 0,
    }
}

fn attempt_refusal(attempt: u32, class: &str) -> ModelRepairAttempt {
    ModelRepairAttempt {
        attempt,
        prompt_sha256: hex_sha256(class.as_bytes()),
        prompt_bytes: 0,
        response_sha256: None,
        response_bytes: None,
        response_excerpt: None,
        proposed_files: Vec::new(),
        applied: false,
        rolled_back: false,
        validator: None,
        refusal: Some(class.to_string()),
        provider_response_id: None,
        provider_response_id_source: None,
        gateway_trace_id: None,
        local_request_id: None,
        wall_ms: 0,
    }
}

fn seal_report(report: &mut ModelRepairReport) {
    let events = batch_of(report);
    report.events_sha256 = Some(hex_sha256(&serde_json::to_vec(&events).unwrap_or_default()));
    let (sealable, reasons) = compute_sealability(report);
    report.sealable = sealable;
    report.sealability_reasons = reasons;
}

/// Seal and return on early-exit paths.
fn sealed_return(mut report: ModelRepairReport) -> ModelRepairReport {
    seal_report(&mut report);
    report
}

trait ModelRepairStatusExt {
    fn as_str(&self) -> &'static str;
}

impl ModelRepairStatusExt for ModelRepairStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Repaired => "repaired",
            Self::ValidatorRejected => "validator-rejected",
            Self::ModelRefusal => "model-refusal",
            Self::ProviderUnavailable => "provider-unavailable",
            Self::UnauthorizedProposal => "unauthorized-proposal",
            Self::BudgetExhausted => "budget-exhausted",
            Self::Interrupted => "interrupted",
            Self::SetupError => "setup-error",
        }
    }
}

fn rfc3339_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    crate::workflow::model_repair::format_unix_utc(secs)
}

#[doc(hidden)]
#[must_use]
pub fn format_unix_utc(secs: u64) -> String {
    // Civil-from-days (Howard Hinnant's algorithm), dependency-free.
    let z = (secs / 86_400) as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    let rem = secs % 86_400;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn redact(text: &str) -> String {
    let _ = REDACTED_PLACEHOLDER;
    Redactor::new()
        .with_known_secrets(&[SECRET_CANARY.to_string()])
        .redact(text)
}

/// Derive the `WorkEventBatch` emission for a sealed report: one `result`
/// event per attempt plus a terminal `outcome`, evidence-bound, ready for
/// SOMA verification (`soma_validate::event::WorkEventBatch::audit`).
#[must_use]
pub fn batch_of(report: &ModelRepairReport) -> WorkEventBatch {
    // Per-execution identity: every event of this run binds to it; repeated
    // executions of the same task/model/revision never share one.
    let run_id = report.execution_id.clone();
    let actor = Actor {
        kind: ActorKind::Model,
        identity: report.model.model.clone(),
        revision: Some(report.model.harness_revision.clone()),
        implementation: None,
    };
    let mut events = Vec::with_capacity(report.attempts.len() + 1);
    for (index, attempt) in report.attempts.iter().enumerate() {
        let event_id = format!("evt-{}-{}", &run_id[..16], index);
        let parents = if index == 0 {
            Vec::new()
        } else {
            vec![format!("evt-{}-{}", &run_id[..16], index - 1)]
        };
        let bindings = serde_json::Map::from_iter([
            ("promptSha256".to_string(), json!(attempt.prompt_sha256)),
            (
                "responseSha256".to_string(),
                json!(
                    attempt
                        .response_sha256
                        .clone()
                        .unwrap_or_else(|| "absent".to_string())
                ),
            ),
            ("applied".to_string(), json!(attempt.applied)),
            (
                "providerResponseId".to_string(),
                json!(
                    attempt
                        .provider_response_id
                        .clone()
                        .unwrap_or_else(|| "absent".to_string())
                ),
            ),
            (
                "validatorExit".to_string(),
                attempt
                    .validator
                    .as_ref()
                    .and_then(|v| v.exit_code)
                    .map(|e| json!(e))
                    .unwrap_or_else(|| json!(null)),
            ),
        ]);
        let payload = EventPayload {
            result: Some(ResultPayload {
                kind: format!("model-repair-attempt-{}", attempt.attempt),
                bindings: Some(bindings),
            }),
            outcome: None,
            approval: None,
            checkpoint_envelope: None,
            resume_token: None,
            decision: None,
            capability: None,
            handoff: None,
        };
        let mut event = WorkEvent {
            schema_version: "1.3.0".to_string(),
            version: "1.3.0".to_string(),
            id: event_id,
            event_type: "result".to_string(),
            actor: actor.clone(),
            authority: model_authority_ceiling(),
            effective_authority: model_authority_ceiling(),
            sequence: index as u64,
            timestamp: report.sealed_at.clone(),
            idempotency_key: format!("idem-{}-{}", &run_id[..16], index),
            correlation_id: run_id.clone(),
            repo_revision: report.base_revision.clone(),
            compatibility: Compatibility {
                schema_version: "1.3.0".to_string(),
                min_reader_version: None,
            },
            semantic_digest: Hex64::parse(&"00".repeat(32)).expect("zero digest parses"),
            parents,
            evidence: None,
            implementation: None,
            payload: Some(payload),
            replay: false,
            conflict: false,
        };
        let artifact_digest = attempt
            .validator
            .as_ref()
            .and_then(|v| v.output_digest.clone())
            .unwrap_or_else(|| attempt.prompt_sha256.clone());
        seal_semantic(&mut event, &artifact_digest, &report.sealed_at);
        events.push(event);
    }
    let outcome_status = match report.status {
        ModelRepairStatus::Repaired => "repaired",
        _ => "task-failed",
    };
    let outcome_id = format!("evt-{}-{}", &run_id[..16], events.len());
    let parents = events
        .last()
        .map(|e| vec![e.id.clone()])
        .unwrap_or_default();
    let outcome_payload = crate::workflow::soma::event::OutcomePayload {
        status: outcome_status.to_string(),
        accepted_by: Vec::new(),
    };
    let mut outcome = WorkEvent {
        schema_version: "1.3.0".to_string(),
        version: "1.3.0".to_string(),
        id: outcome_id,
        event_type: "outcome".to_string(),
        actor,
        authority: model_authority_ceiling(),
        effective_authority: model_authority_ceiling(),
        sequence: events.len() as u64,
        timestamp: report.sealed_at.clone(),
        idempotency_key: format!("idem-{}-{}", &run_id[..16], events.len()),
        correlation_id: run_id.clone(),
        repo_revision: report.base_revision.clone(),
        compatibility: Compatibility {
            schema_version: "1.3.0".to_string(),
            min_reader_version: None,
        },
        semantic_digest: Hex64::parse(&"00".repeat(32)).expect("zero digest parses"),
        parents,
        evidence: None,
        implementation: None,
        payload: Some(EventPayload {
            outcome: Some(outcome_payload),
            result: None,
            approval: None,
            checkpoint_envelope: None,
            resume_token: None,
            decision: None,
            capability: None,
            handoff: None,
        }),
        replay: false,
        conflict: false,
    };
    seal_semantic(
        &mut outcome,
        &hex_sha256(outcome_status.as_bytes()),
        &report.sealed_at,
    );
    events.push(outcome);
    WorkEventBatch {
        schema_version: "1.3.0".to_string(),
        version: "1.3.0".to_string(),
        run_id,
        events,
        compatibility: None,
    }
}

fn seal_semantic(event: &mut WorkEvent, artifact_digest: &str, produced_at: &str) {
    let computed = event
        .computed_semantic_digest()
        .expect("semantic digest is computable");
    event.semantic_digest = Hex64::parse(&computed).expect("digest is hex64");
    let artifact = Hex64::parse(artifact_digest)
        .unwrap_or_else(|_| Hex64::parse(&"11".repeat(32)).expect("constant digest parses"));
    event.evidence = Some(vec![EvidenceReference {
        id: format!("ev-{computed}"),
        event_digest: Hex64::parse(&computed).expect("digest is hex64"),
        artifact_digest: artifact,
        artifact_kind: "model-repair-attempt-record".to_string(),
        produced_by: "lite.model_repair".to_string(),
        produced_at: Some(produced_at.to_string()),
    }]);
}

fn model_authority_ceiling() -> AuthorityProfile {
    // The model-authored run is the definition of ModelAssisted execution
    // with mutation none: the model proposes, the governed boundary applies.
    AuthorityProfile {
        execution_class: ExecutionClass::ModelAssisted,
        mutation: MutationMode::None_,
        readable_scopes: Some(vec!["repo://task".to_string()]),
        writable_scopes: None,
        tools: None,
        network_policy: None,
        provider_policy: None,
        secrets: None,
        escalation: None,
        review: None,
        abstention: None,
        budgets: None,
        content_restrictions: None,
    }
}

/// Canonical bytes helper for external verifiers: sha256 over the report's
/// canonical JSON serialization (field order is stable).
#[must_use]
pub fn report_digest(report: &ModelRepairReport) -> String {
    hex_sha256(&serde_json::to_vec(report).expect("report serializes"))
}
