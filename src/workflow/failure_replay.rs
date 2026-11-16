//! Lite-internal failure-replay evidence (issue #136, Slice C).
//!
//! This module owns the **versioned Lite-internal failure-replay descriptor**
//! and the **failure-replay evidence envelope** that link one previously
//! sealed, genuinely failed task-run to the replay that followed it. It is
//! deliberately **not** a Foundry contract and not a completion claim for
//! #136 or #156:
//!
//! - Lite owns the executable fixture scenarios and the raw failure→replay
//!   evidence. Foundry owns the external benchmark runner, the external
//!   evaluation format, matrix aggregation, comparisons, certification and
//!   the public-claim policy. Nothing in this module is labelled as, or
//!   implied to be, a Foundry artifact.
//! - The [`ExternalEvaluationFormat`] stub on every exported record is typed
//!   `unsupported` on purpose. The Foundry external evaluation format does
//!   not exist yet; until Foundry publishes and pins it, Lite exports raw
//!   JSON records only.
//!
//! What a failure-replay record proves:
//! - the **original failure is real**: the embedded original task-run
//!   envelope re-verifies, its task outcome is `repair-failed`, its
//!   classification is `governed-product-outcome`, and its validator exit
//!   code is the pinned non-zero exit recorded in the replay descriptor.
//! - the **replay is the same task**, not an unrelated success: the replay
//!   envelope re-verifies, and its embedded scenario descriptor has the
//!   SAME canonical scenario digest, the SAME pinned validator command/argv,
//!   the SAME pinned baseline (fixture bytes) and the SAME authority
//!   (writable scopes, repair origin, repair write authority, changed
//!   file set) as the original failure. A replay may not widen authority,
//!   swap validators, change classification or substitute a fresh unrelated
//!   success.
//! - the **link is checked, never asserted**: the descriptor pins the
//!   scenario digest, the baseline content digest, the known failed
//!   validator's command/argv/exit code, the failed file-after digest and
//!   the failed post-run tree digest; `verify()` re-derives every one of
//!   them from the embedded bytes. The `failureIdentity` is a wall-clock-free
//!   stable identity of the failure (scenario digest + validator argv +
//!   observed exit code + baseline + failed post-run digest) used for
//!   dedup/idempotency: the same failure reproduces the same identity.
//!
//! What a failure-replay record does NOT prove: the same wall-clock-free
//! limits as the underlying task-run evidence (see
//! [`LiteTaskRunEvidence`](crate::workflow::scenario::LiteTaskRunEvidence)),
//! and nothing about model coding ability, benchmark standing or Foundry
//! evaluation. It is a bridge for issue #156 (Lite-internal failure-replay
//! work), not a completed external evaluation.
//!
//! Both embedded records are full [`LiteTaskRunEvidence`] envelopes; this
//! module never re-mints or re-signs them. Sealing refuses when any binding
//! fails; nothing is caller-asserted into place.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::workflow::route_resolution::is_hex64;
use crate::workflow::scenario::{
    self, ExternalEvaluationFormat, LiteTaskRunEvidence, RunProvenance, ScenarioOutcomeClass,
    TaskOutcome, TaskRepairOrigin, TaskRepairWriteAuthority, check_fixture_path,
};
use crate::workflow::soma;

/// Schema version of the failure-replay descriptor document itself.
pub const FAILURE_REPLAY_DESCRIPTOR_SCHEMA_VERSION: &str = "1.0.0";
/// Namespace every failure-replay descriptor must declare. Lite-internal.
pub const FAILURE_REPLAY_DESCRIPTOR_NAMESPACE: &str = "lite.fixtures.failure-replay";
/// Schema version of the Lite-internal failure-replay evidence envelope.
pub const FAILURE_REPLAY_SCHEMA_VERSION: &str = "1.0.0";
/// Namespace of the Lite-internal failure-replay evidence envelope.
/// Lite-internal; explicitly NOT a Foundry namespace.
pub const FAILURE_REPLAY_NAMESPACE: &str = "lite.evidence.failure-replay";
/// Identifier of the local fixture runner that binds failures to replays.
pub const FAILURE_REPLAY_RUNNER_ID: &str = "lite-failure-replay-fixture-runner";
/// Revision of the failure-replay runner. Bump on any behavioural change.
pub const FAILURE_REPLAY_RUNNER_REVISION: &str = "1";
/// What [`LiteFailureReplayEvidence::verify`] proves and does not prove.
/// Pinned on every envelope and enforced at sealing and re-verification.
pub const FAILURE_REPLAY_VERIFICATION_SCOPE: &str = "internally consistent operator-observed measurements that bind one previously sealed failed task-run envelope to the replay that followed it against the same pinned fixture, validator and authority. It is NOT cryptographic proof that any external process produced the recorded output, and NOT evidence of model coding ability, standing or Foundry evaluation.";

/// How the original failure is classified for replay purposes. Distinct
/// from [`ScenarioOutcomeClass`]: a governed-boundary run whose pinned
/// validator executed and failed is a **governed validation failure** — not
/// a setup error, not an invalid task, not infrastructure, and not a
/// product success. Only this class may be pinned as a replayable failure
/// in this schema; any other value is refused at validation and at
/// verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FailureReplayClass {
    /// The governed dispatch completed, the fixture-scripted repair was
    /// applied with permission, and the pinned validator exited non-zero.
    GovernedValidationFailure,
}

impl FailureReplayClass {
    pub fn as_str(self) -> &'static str {
        match self {
            FailureReplayClass::GovernedValidationFailure => "governed-validation-failure",
        }
    }
}

/// The known failed validator. Command/argv/exit code are pinned; the exit
/// code MUST be non-zero (a zero-exit "failure" is not a failure).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FailureValidatorPin {
    /// Exact executable, copied from the embedded descriptor's validation
    /// pin.
    pub command: String,
    /// Exact argument vector, copied from the embedded descriptor's
    /// validation pin.
    pub args: Vec<String>,
    /// The observed non-zero exit code of the failed validator run.
    pub exit_code: i32,
}

/// Pins identifying the original failure. Everything here is wall-clock-free
/// and deterministic: the same wrong fixture-scripted repair against the
/// same pinned fixture reproduces the same pins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FailureOriginalPin {
    /// `scenarioId` of the embedded scenario descriptor that produced the
    /// failure.
    pub scenario_id: String,
    /// Canonical sha256 digest of the embedded scenario descriptor (same
    /// for the failure run and the replay run).
    pub scenario_digest: String,
    /// `task_run_tree_digest` of the baseline (pre-effect) source tree;
    /// equal to the embedded descriptor's pinned `repository.contentDigest`.
    pub baseline_content_digest: String,
    /// The classification the original failure sealed with.
    pub classification: ScenarioOutcomeClass,
    /// The task outcome the original failure sealed with.
    pub task_outcome: TaskOutcome,
    /// The distinct replay failure class.
    pub failure_class: FailureReplayClass,
    /// The known failed validator.
    pub validator: FailureValidatorPin,
    /// sha256 of the target file after the failed repair (deterministic
    /// for a fixture-scripted wrong repair).
    pub failed_file_after_digest: String,
    /// `task_run_tree_digest` of the post-run source tree of the failed
    /// repair (deterministic; validator-created files included).
    pub failed_post_run_content_digest: String,
}

/// What the replay must produce for the failure to count as repaired.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FailureReplayPin {
    pub task_outcome: TaskOutcome,
    pub classification: ScenarioOutcomeClass,
    /// The expected exit code of the replay's pinned validator.
    pub expected_exit_code: i32,
}

/// Authority pins: the replay may hold exactly the authority of the
/// original failure, never more.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FailureAuthorityPin {
    /// Writable scopes of both runs; must be identical (no expansion).
    pub writable_scopes: Vec<String>,
    /// Origin of the repair in both runs (scripted fixture, never a model).
    pub repair_origin: TaskRepairOrigin,
    /// Whose authority made the repair write in both runs.
    pub repair_write_authority: TaskRepairWriteAuthority,
    /// Files the repair in either run is permitted to change. The replay's
    /// changed set must equal this set; widening it means the replay is no
    /// longer a replay of this failure.
    pub changed_files: Vec<String>,
}

/// The versioned Lite-internal failure-replay descriptor. Parse with
/// [`FailureReplayDescriptor::parse_json`]; never construct and skip
/// validation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FailureReplayDescriptor {
    pub schema_version: String,
    pub namespace: String,
    /// Stable identifier of this replayable failure, e.g.
    /// `lite-rust-bugfix-arithmetic-replay-001`.
    pub replay_id: String,
    /// Descriptor document version (independent of schema_version).
    pub version: String,
    pub title: String,
    pub original: FailureOriginalPin,
    pub replay: FailureReplayPin,
    pub authority: FailureAuthorityPin,
}

impl FailureReplayDescriptor {
    /// Fail-closed parse + validation. Every defect found is returned.
    pub fn parse_json(text: &str) -> Result<Self, scenario::ScenarioDescriptorError> {
        let descriptor: Self = match serde_json::from_str(text) {
            Ok(descriptor) => descriptor,
            Err(error) => {
                return Err(scenario::ScenarioDescriptorError(vec![
                    scenario::ScenarioDefect::setup(format!(
                        "failure-replay descriptor is not a valid {FAILURE_REPLAY_DESCRIPTOR_NAMESPACE} document: {error}"
                    )),
                ]));
            }
        };
        let defects = descriptor.validate();
        if !defects.is_empty() {
            return Err(scenario::ScenarioDescriptorError(defects));
        }
        Ok(descriptor)
    }

    /// Collect all defects. Empty means the descriptor is valid.
    pub fn validate(&self) -> Vec<scenario::ScenarioDefect> {
        use scenario::ScenarioDefect;
        let mut defects = Vec::new();

        if self.schema_version != FAILURE_REPLAY_DESCRIPTOR_SCHEMA_VERSION {
            defects.push(ScenarioDefect::setup(format!(
                "unsupported schemaVersion {:?} (supported {FAILURE_REPLAY_DESCRIPTOR_SCHEMA_VERSION})",
                self.schema_version
            )));
        }
        if self.namespace != FAILURE_REPLAY_DESCRIPTOR_NAMESPACE {
            defects.push(ScenarioDefect::setup(format!(
                "namespace {:?} is not {FAILURE_REPLAY_DESCRIPTOR_NAMESPACE:?}",
                self.namespace
            )));
        }
        if self.replay_id.trim().is_empty() {
            defects.push(ScenarioDefect::setup(
                "replayId must be non-empty".to_string(),
            ));
        }
        if self.version.trim().is_empty() {
            defects.push(ScenarioDefect::setup(
                "version must be non-empty".to_string(),
            ));
        }
        if self.title.trim().is_empty() {
            defects.push(ScenarioDefect::setup("title must be non-empty".to_string()));
        }

        // Original failure pins.
        if self.original.scenario_id.trim().is_empty() {
            defects.push(ScenarioDefect::setup(
                "original.scenarioId must be non-empty".to_string(),
            ));
        }
        for (label, digest) in [
            ("original.scenarioDigest", &self.original.scenario_digest),
            (
                "original.baselineContentDigest",
                &self.original.baseline_content_digest,
            ),
            (
                "original.failedFileAfterDigest",
                &self.original.failed_file_after_digest,
            ),
            (
                "original.failedPostRunContentDigest",
                &self.original.failed_post_run_content_digest,
            ),
        ] {
            if !is_hex64(digest) {
                defects.push(ScenarioDefect::setup(format!(
                    "{label} must be a lower hex sha256 digest"
                )));
            }
        }
        if self.original.classification != ScenarioOutcomeClass::GovernedProductOutcome {
            defects.push(ScenarioDefect::setup(
                "original.classification must be governed-product-outcome (a sealed governed failure); setup/infrastructure classes are not replayable failures"
                    .to_string(),
            ));
        }
        if self.original.task_outcome != TaskOutcome::RepairFailed {
            defects.push(ScenarioDefect::setup(
                "original.taskOutcome must be repair-failed; a repaired run is not a failure"
                    .to_string(),
            ));
        }
        if self.original.failure_class != FailureReplayClass::GovernedValidationFailure {
            defects.push(ScenarioDefect::setup(
                "original.failureClass must be governed-validation-failure".to_string(),
            ));
        }
        if self.original.validator.exit_code == 0 {
            defects.push(ScenarioDefect::setup(
                "original.validator.exitCode must be non-zero; a zero-exit validator outcome is not a failure"
                    .to_string(),
            ));
        }
        if self.original.validator.command.trim().is_empty() {
            defects.push(ScenarioDefect::setup(
                "original.validator.command must be non-empty".to_string(),
            ));
        }
        if self
            .original
            .validator
            .command
            .chars()
            .any(|c| matches!(c, '/' | '\\' | '\0'))
        {
            defects.push(ScenarioDefect::setup(
                "original.validator.command must be a bare executable name, not a path".to_string(),
            ));
        }
        if self.original.validator.args.is_empty() {
            defects.push(ScenarioDefect::setup(
                "original.validator.args must pin at least one argument".to_string(),
            ));
        }
        for arg in &self.original.validator.args {
            if arg.trim().is_empty() {
                defects.push(ScenarioDefect::setup(
                    "original.validator.args entries must be non-empty".to_string(),
                ));
            }
        }

        // Replay pins.
        if self.replay.task_outcome != TaskOutcome::Repaired {
            defects.push(ScenarioDefect::setup(
                "replay.taskOutcome must be repaired; a replay that is not expected to repair is not a replay"
                    .to_string(),
            ));
        }
        if self.replay.classification != ScenarioOutcomeClass::GovernedProductOutcome {
            defects.push(ScenarioDefect::setup(
                "replay.classification must be governed-product-outcome".to_string(),
            ));
        }
        if self.replay.expected_exit_code == self.original.validator.exit_code {
            defects.push(ScenarioDefect::setup(
                "replay.expectedExitCode must differ from the original failure's exit code"
                    .to_string(),
            ));
        }

        // Authority pins: the replay holds exactly the original authority.
        if self.authority.writable_scopes.is_empty() {
            defects.push(ScenarioDefect::setup(
                "authority.writableScopes must pin at least one scope".to_string(),
            ));
        }
        if self.authority.changed_files.is_empty() {
            defects.push(ScenarioDefect::setup(
                "authority.changedFiles must pin at least one repair target".to_string(),
            ));
        }
        for path in &self.authority.changed_files {
            if let Err(message) = check_fixture_path(path) {
                defects.push(ScenarioDefect::setup(format!(
                    "authority.changedFiles: {message}"
                )));
            }
        }

        defects
    }

    /// Canonical digest of the descriptor document.
    pub fn replay_digest(&self) -> Result<String> {
        let value = serde_json::to_value(self).context("replay descriptor serializes")?;
        soma::try_canonical_digest(&value)
            .map_err(|error| anyhow::anyhow!("replay descriptor canonical digest failed: {error}"))
    }
}

/// Stable, wall-clock-free identity of the embedded failure. Two executions
/// of the same wrong fixture-scripted repair against the same pinned fixture
/// derive the same identity; a different fixture, validator, repair or exit
/// code derives a different one. Used for dedup/idempotency of replayable
/// failures.
pub fn derive_failure_identity(failure: &LiteTaskRunEvidence) -> Result<String> {
    let descriptor = failure.scenario_run.descriptor.scenario_digest()?;
    let value = serde_json::json!({
        "scenarioDigest": descriptor,
        "validatorCommand": failure.validator.command,
        "validatorArgs": failure.validator.args,
        "validatorExitCode": failure.validator.exit_code,
        "classification": failure.classification.as_str(),
        "taskOutcome": match failure.task_outcome {
            TaskOutcome::Repaired => "repaired",
            TaskOutcome::RepairFailed => "repair-failed",
            TaskOutcome::RepairRefused => "repair-refused",
        },
        "baselineContentDigest": failure.baseline_content_digest,
        "failedPostRunContentDigest": failure.post_run_content_digest,
        "failedFileAfterDigest": failure.repair.file_after_digest,
    });
    soma::try_canonical_digest(&value)
        .map_err(|error| anyhow::anyhow!("failure identity canonical digest failed: {error}"))
}

/// The Lite-internal failure-replay evidence envelope. **Embeds the full
/// original failure and the full replay** (each a complete
/// [`LiteTaskRunEvidence`]), so [`LiteFailureReplayEvidence::verify`]
/// re-derives every binding from embedded bytes alone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiteFailureReplayEvidence {
    pub schema_version: String,
    pub namespace: String,
    /// Pinned evaluation instant (deterministic; equal in both embeds).
    pub evaluated_at: String,
    pub descriptor: FailureReplayDescriptor,
    /// The sealed original failure. Re-verified on its own.
    pub original_failure: LiteTaskRunEvidence,
    /// The sealed replay that followed the failure. Re-verified on its own.
    pub replay: LiteTaskRunEvidence,
    /// Stable, wall-clock-free identity of the original failure (dedup key).
    pub failure_identity: String,
    pub external_evaluation_format: ExternalEvaluationFormat,
    pub provenance: RunProvenance,
    pub verification_scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
}

/// Every failure-replay-specific binding that must hold before sealing.
/// Shared by `sealed` and `verify`. `descriptor` is the embedded replay
/// descriptor: both embedded envelopes are checked against ITS pins, so a
/// replay spliced from a different failure cannot borrow a recorded success.
fn check_failure_replay_bindings(
    descriptor: &FailureReplayDescriptor,
    original: &LiteTaskRunEvidence,
    replay: &LiteTaskRunEvidence,
    failure_identity: &str,
) -> Vec<String> {
    let mut violations: Vec<String> = Vec::new();
    let mut fail = |message: String| violations.push(message);

    // ---- Descriptor-pinned original failure -------------------------------
    let original_descriptor = &original.scenario_run.descriptor;
    let replay_descriptor = &replay.scenario_run.descriptor;

    match original_descriptor.scenario_digest() {
        Ok(digest) => {
            if digest != descriptor.original.scenario_digest {
                fail(
                    "original failure scenario digest != replay descriptor original.scenarioDigest"
                        .to_string(),
                );
            }
        }
        Err(error) => fail(format!("original failure scenario digest failed: {error}")),
    }
    if original_descriptor.scenario_id != descriptor.original.scenario_id {
        fail("original failure scenarioId != replay descriptor original.scenarioId".to_string());
    }
    if original.baseline_content_digest != descriptor.original.baseline_content_digest {
        fail("original failure baselineContentDigest != replay descriptor pin".to_string());
    }
    if original.repair.file_after_digest != descriptor.original.failed_file_after_digest {
        fail(
            "original failure fileAfterDigest != replay descriptor original.failedFileAfterDigest"
                .to_string(),
        );
    }
    if original.post_run_content_digest != descriptor.original.failed_post_run_content_digest {
        fail("original failure postRunContentDigest != replay descriptor original.failedPostRunContentDigest".to_string());
    }
    if original.classification != descriptor.original.classification {
        fail("original failure classification != replay descriptor pin".to_string());
    }
    if original.task_outcome != descriptor.original.task_outcome {
        fail("original failure taskOutcome != replay descriptor pin".to_string());
    }
    if original.validator.command != descriptor.original.validator.command {
        fail("original failure validator.command != replay descriptor pin".to_string());
    }
    if original.validator.args != descriptor.original.validator.args {
        fail("original failure validator.args != replay descriptor pin".to_string());
    }
    if original.validator.exit_code != Some(descriptor.original.validator.exit_code) {
        fail("original failure validator.exitCode != replay descriptor pin".to_string());
    }
    if descriptor.original.validator.exit_code == 0 {
        fail("replay descriptor pins a zero-exit original validator; not a failure".to_string());
    }
    match &original_descriptor.validation {
        Some(pins) => {
            if pins.command != descriptor.original.validator.command
                || pins.args != descriptor.original.validator.args
            {
                fail(
                    "replay descriptor original.validator command/args disagree with the embedded descriptor's validation pin"
                        .to_string(),
                );
            }
            // The failed validator MUST have failed against its own pinned
            // expectation: anything else is not a pinned validation failure.
            if descriptor.original.validator.exit_code == pins.expected_exit_code {
                fail(
                    "original failure validator.exitCode equals the pinned expected exit code; the failure is not an unmet pinned assertion"
                        .to_string(),
                );
            }
        }
        None => fail(
            "original failure embedded descriptor pins no validation surface; not a replayable validation failure"
                .to_string(),
        ),
    }

    // ---- Descriptor-pinned replay ------------------------------------------
    match replay_descriptor.scenario_digest() {
        Ok(digest) => {
            if digest != descriptor.original.scenario_digest {
                fail(
                    "replay scenario digest != the original failure's scenario digest; a replay must run the same pinned task, not an unrelated success"
                        .to_string(),
                );
            }
        }
        Err(error) => fail(format!("replay scenario digest failed: {error}")),
    }
    if replay_descriptor.scenario_id != descriptor.original.scenario_id {
        fail("replay scenarioId != original scenarioId".to_string());
    }
    if replay.baseline_content_digest != descriptor.original.baseline_content_digest {
        fail("replay baselineContentDigest != the original failure's pinned baseline".to_string());
    }
    if replay.classification != descriptor.replay.classification {
        fail("replay classification != replay descriptor pin".to_string());
    }
    if replay.task_outcome != descriptor.replay.task_outcome {
        fail("replay taskOutcome != replay descriptor pin".to_string());
    }
    if replay.validator.exit_code != Some(descriptor.replay.expected_exit_code) {
        fail("replay validator.exitCode != replay descriptor replay.expectedExitCode".to_string());
    }
    if let Some(pins) = &replay_descriptor.validation {
        if descriptor.replay.expected_exit_code != pins.expected_exit_code {
            fail(
                "replay descriptor replay.expectedExitCode != embedded descriptor validation.expectedExitCode"
                    .to_string(),
            );
        }
    } else {
        fail("replay embedded descriptor pins no validation surface".to_string());
    }

    // ---- Same task, same authority; never a fresh unrelated success --------
    if replay.validator.command != original.validator.command
        || replay.validator.args != original.validator.args
    {
        fail("replay validator command/args differ from the original failure's; the validator may not be swapped between failure and replay".to_string());
    }
    if original_descriptor.workspace.writable_scopes != replay_descriptor.workspace.writable_scopes
    {
        fail("replay writable scopes widen or shrink the original failure's; authority must not change between failure and replay".to_string());
    }
    if original_descriptor.policy.writable_scopes != replay_descriptor.policy.writable_scopes {
        fail("replay policy writable scopes differ from the original failure's".to_string());
    }
    if replay.repair.origin != descriptor.authority.repair_origin {
        fail("replay repair.origin != replay descriptor authority.repairOrigin".to_string());
    }
    if original.repair.origin != replay.repair.origin {
        fail("replay repair.origin != original failure repair.origin".to_string());
    }
    if replay.repair.write_authority != descriptor.authority.repair_write_authority {
        fail(
            "replay repair.writeAuthority != replay descriptor authority.repairWriteAuthority"
                .to_string(),
        );
    }
    if original.repair.write_authority != replay.repair.write_authority {
        fail("replay repair.writeAuthority != original failure repair.writeAuthority; a replay may not gain authority".to_string());
    }
    if replay.repair.changed_files != descriptor.authority.changed_files {
        fail("replay repair.changedFiles != replay descriptor authority.changedFiles; the repair surface may not widen".to_string());
    }
    if replay.repair.changed_files != original.repair.changed_files {
        fail("replay repair.changedFiles != original failure repair.changedFiles; a replay must touch the same repair surface".to_string());
    }
    if original_descriptor.workspace.writable_scopes != descriptor.authority.writable_scopes {
        fail(
            "original failure writable scopes != replay descriptor authority.writableScopes"
                .to_string(),
        );
    }
    if replay_descriptor.workspace.writable_scopes != descriptor.authority.writable_scopes {
        fail("replay writable scopes != replay descriptor authority.writableScopes".to_string());
    }

    // ---- Failure identity (dedup/idempotency) -------------------------------
    if !is_hex64(failure_identity) {
        fail("failureIdentity must be a lower hex sha256 digest".to_string());
    } else {
        match derive_failure_identity(original) {
            Ok(derived) => {
                if derived != failure_identity {
                    fail(
                        "failureIdentity does not re-derive from the embedded original failure"
                            .to_string(),
                    );
                }
            }
            Err(error) => fail(format!("failureIdentity cannot be derived: {error}")),
        }
    }

    violations
}

impl LiteFailureReplayEvidence {
    /// Canonical digest of the envelope minus `contentDigest`.
    pub fn compute_digest(&self) -> Result<String> {
        let mut value = serde_json::to_value(self).context("failure-replay evidence serializes")?;
        if let Some(obj) = value.as_object_mut() {
            obj.remove("contentDigest");
        }
        soma::try_canonical_digest(&value).map_err(|error| {
            anyhow::anyhow!("failure-replay evidence canonical digest failed: {error}")
        })
    }

    /// Stable identity of the sealed original failure (dedup key).
    pub fn failure_identity(&self) -> &str {
        &self.failure_identity
    }

    /// Build and seal a failure-replay envelope from two previously sealed
    /// task-run envelopes (the failure and its replay). Every binding is
    /// checked BEFORE sealing; on any mismatch this returns the collected
    /// discrepancies and nothing is exported.
    pub fn sealed(
        descriptor: FailureReplayDescriptor,
        original_failure: LiteTaskRunEvidence,
        replay: LiteTaskRunEvidence,
    ) -> Result<Self> {
        let defects = descriptor.validate();
        if !defects.is_empty() {
            anyhow::bail!(
                "failure-replay descriptor rejected: {}",
                defects
                    .iter()
                    .map(|defect| format!("{}: {}", defect.class.as_str(), defect.message))
                    .collect::<Vec<_>>()
                    .join("; ")
            );
        }
        let evaluated_at = replay.scenario_run.evaluated_at.clone();
        let failure_identity = derive_failure_identity(&original_failure)?;
        let violations = check_failure_replay_bindings(
            &descriptor,
            &original_failure,
            &replay,
            &failure_identity,
        );
        if !violations.is_empty() {
            anyhow::bail!(
                "failure-replay evidence refuses to seal; {} binding(s) failed: {}",
                violations.len(),
                violations.join("; ")
            );
        }
        original_failure.verify()?;
        replay.verify()?;
        let mut record = Self {
            schema_version: FAILURE_REPLAY_SCHEMA_VERSION.to_string(),
            namespace: FAILURE_REPLAY_NAMESPACE.to_string(),
            evaluated_at,
            descriptor,
            original_failure,
            replay,
            failure_identity,
            external_evaluation_format: ExternalEvaluationFormat::unsupported(),
            provenance: RunProvenance {
                runner: FAILURE_REPLAY_RUNNER_ID.to_string(),
                runner_revision: FAILURE_REPLAY_RUNNER_REVISION.to_string(),
                injected_clock: true,
                fixture_only: true,
                redaction: "digest-sealed artifacts carry identities and digests only; raw secret material is never serialized (canary-verified in tests)".to_string(),
            },
            verification_scope: FAILURE_REPLAY_VERIFICATION_SCOPE.to_string(),
            content_digest: None,
        };
        record.content_digest = Some(record.compute_digest()?);
        Ok(record)
    }

    /// Fail-closed re-verification from the embedded bytes alone.
    pub fn verify(&self) -> anyhow::Result<()> {
        use anyhow::ensure;
        ensure!(
            self.schema_version == FAILURE_REPLAY_SCHEMA_VERSION,
            "unsupported failure-replay schema version {} (supported {FAILURE_REPLAY_SCHEMA_VERSION})",
            self.schema_version
        );
        ensure!(
            self.namespace == FAILURE_REPLAY_NAMESPACE,
            "failure-replay namespace {:?} is not {FAILURE_REPLAY_NAMESPACE:?}",
            self.namespace
        );
        match &self.content_digest {
            Some(actual) => {
                let expected = self.compute_digest()?;
                ensure!(
                    *actual == expected,
                    "failure-replay evidence contentDigest does not verify"
                );
            }
            None => anyhow::bail!("failure-replay evidence is missing its contentDigest"),
        }
        ensure!(
            self.verification_scope == FAILURE_REPLAY_VERIFICATION_SCOPE,
            "failure-replay verificationScope must be pinned to the documented verification limitation"
        );
        let defects = self.descriptor.validate();
        ensure!(
            defects.is_empty(),
            "embedded failure-replay descriptor is invalid: {}",
            defects
                .iter()
                .map(|defect| defect.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        );
        ensure!(
            self.evaluated_at == self.original_failure.scenario_run.evaluated_at
                && self.evaluated_at == self.replay.scenario_run.evaluated_at,
            "failure-replay evaluatedAt does not bind the embedded runs' evaluation instant"
        );
        // The embedded envelopes re-verify on their own.
        self.original_failure.verify()?;
        self.replay.verify()?;
        // ... and every cross-envelope binding re-derives.
        let violations = check_failure_replay_bindings(
            &self.descriptor,
            &self.original_failure,
            &self.replay,
            &self.failure_identity,
        );
        ensure!(
            violations.is_empty(),
            "failure-replay evidence failed binding checks: {}",
            violations.join("; ")
        );
        ensure!(
            self.external_evaluation_format.status == "unsupported",
            "externalEvaluationFormat.status must be \"unsupported\""
        );
        ensure!(
            self.provenance.fixture_only && self.provenance.injected_clock,
            "failure-replay evidence must be fixture-only with an injected clock"
        );
        Ok(())
    }
}

/// Serialize the failure-replay envelope as canonical-ish JSON for inspection.
pub fn failure_replay_json(evidence: &LiteFailureReplayEvidence) -> Result<String> {
    serde_json::to_string_pretty(evidence).context("failure-replay evidence serializes")
}
