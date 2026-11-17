//! Lite-native failure-to-evaluation case (issue #156, Slice A).
//!
//! This module owns the **versioned Lite-internal failure case**: a compact,
//! wall-clock-free record that binds one previously sealed, verified failure
//! (a [`LiteFailureReplayEvidence`]) to the minimized reproduction that
//! reproduces it, to the redaction scan performed at export, to the effective
//! authority both runs held, and to the fix evidence that resolved it. It is
//! deliberately **not** a Foundry contract and not a completion claim for
//! #156:
//!
//! - Lite owns the executable fixture scenarios, the raw failure→replay
//!   evidence and this case document. Foundry owns the external benchmark
//!   runner, the external evaluation format, matrix aggregation, comparisons,
//!   certification and the public-claim policy. Nothing in this module is
//!   labelled as, or implied to be, a Foundry artifact.
//! - The [`ExternalEvaluationFormat`] marker on every exported case is typed
//!   `unsupported` on purpose. The Foundry external evaluation format does not
//!   exist yet; until Foundry publishes and pins it, Lite exports raw JSON
//!   records only. No Foundry ingestion, metrics, rankings or certification
//!   are claimed.
//!
//! What a failure case proves:
//! - the **original failure is real**: the embedded chain re-verifies and its
//!   original failure is the pinned governed validation failure selected by
//!   the operator (not a setup error, not an invalid task, not infrastructure
//!   and not a product success);
//! - the **fix is the same task**: the chain's replay re-verifies against the
//!   SAME scenario digest, validator and authority as the failure, so the fix
//!   evidence bound here cannot be an unrelated success;
//! - the **minimization is honest**: every accepted reduction was replayed
//!   against the real governed boundary and the pinned failure still
//!   reproduced with the same signature (validator command/argv/exit code,
//!   task outcome, classification and failure class). A reduction that drifted
//!   the class or broke reproduction is rejected and the file is restored;
//! - the **export passed a secret scan**: the serialized case is scanned for
//!   registered known secrets, [`SECRET_CANARY`] and machine paths. A case that
//!   needs redaction to be safe is REFUSED, never rewritten — a digest-only
//!   artifact must be invariant under redaction;
//! - the **identity is dedupable**: the failure identity is stable across
//!   independent runs of the same wrong repair against the same pinned
//!   fixture. Equivalent occurrences merge under one group; distinct
//!   provenance is never discarded (see [`dedup_cases`]);
//!
//! What a failure case does NOT prove: the same wall-clock-free limits as the
//! underlying task-run evidence, anything about model coding ability, any
//! benchmark standing, and any Foundry evaluation. It is a bridge for issue
//! #156, not a completed external evaluation.

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::workflow::failure_replay::{
    FailureAuthorityPin, FailureReplayClass, FailureValidatorPin, LiteFailureReplayEvidence,
};
use crate::workflow::redaction::{SECRET_CANARY, redact_diagnostics};
use crate::workflow::route_resolution::is_hex64;
use crate::workflow::scenario::{
    self, ExternalEvaluationFormat, RunProvenance, ScenarioOutcomeClass, TaskOutcome,
    check_fixture_path,
};
use crate::workflow::soma;

/// Schema version of the failure-case document itself.
pub const FAILURE_CASE_SCHEMA_VERSION: &str = "1.0.0";
/// Namespace every failure case must declare. Lite-internal.
pub const FAILURE_CASE_NAMESPACE: &str = "lite.evaluation.failure-case";
/// Document version of the failure-case record format.
pub const FAILURE_CASE_VERSION: &str = "1.0.0";
/// Identifier of the local exporter that binds chains to cases.
pub const FAILURE_CASE_RUNNER_ID: &str = "lite-failure-case-exporter";
/// Revision of the failure-case exporter. Bump on any behavioural change.
pub const FAILURE_CASE_RUNNER_REVISION: &str = "1";
/// Identifier of the redaction/secret scan applied at export.
pub const FAILURE_CASE_SCANNER_ID: &str = "lite-redaction-scan/1";
/// What [`LiteFailureCase::verify`] proves and does not prove. Pinned on every
/// case and enforced at sealing and re-verification.
pub const FAILURE_CASE_VERIFICATION_SCOPE: &str = "internally consistent operator-observed measurements binding one previously sealed, verified failure-replay chain to a minimized, redaction-scanned, authority-pinned reproduction and fix. It is NOT cryptographic proof that any external process produced the recorded output, and NOT evidence of model coding ability, standing or Foundry evaluation.";
/// Identifier of the minimization algorithm recorded on every case.
pub const MINIMIZATION_ALGORITHM: &str = "bounded-remove-one-candidate-at-a-time";

// ---------------------------------------------------------------------------
// Source / task pins
// ---------------------------------------------------------------------------

/// Pins identifying the original failure's source repository and task.
/// Everything here is wall-clock-free and deterministic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FailureCaseSourcePin {
    /// `scenarioId` of the scenario that produced the failure.
    pub scenario_id: String,
    /// Canonical sha256 digest of the scenario descriptor (pins the task,
    /// policy, workspace, harness, route, dimensions and validation pins).
    pub scenario_digest: String,
    /// Pinned revision of the source fixture repository.
    pub repository_revision: String,
    /// `fixture` path of the pinned repository (relative, forward slashes).
    pub repository_fixture: String,
    /// `task_run_tree_digest` of the baseline (pre-effect) source tree.
    pub repository_baseline_digest: String,
    /// Canonical sha256 digest of the task statement (pins the task text).
    pub task_statement_digest: String,
    /// Harness id pinned by the scenario.
    pub harness_id: String,
    /// Harness revision pinned by the scenario.
    pub harness_revision: String,
    /// Provider identity pinned by the route.
    pub provider_identity: String,
    /// Model pinned by the route.
    pub model: String,
}

/// What the fix must produce for the case to be resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FailureCaseExpected {
    /// The replay outcome that counts as a fix (`repaired`).
    pub task_outcome: TaskOutcome,
    /// The classification that counts as a fix (`governed-product-outcome`).
    pub classification: ScenarioOutcomeClass,
    /// The pinned validator exit code that counts as a fix (zero).
    pub validator_exit_code: i32,
    /// The pinned expected-assertion statement (what "done" means).
    pub expected_assertion: String,
}

// ---------------------------------------------------------------------------
// Minimized reproduction
// ---------------------------------------------------------------------------

/// One candidate reduction that was REJECTED: after removing `path`, the
/// pinned failure no longer reproduced (class drift, broken reproduction, or
/// a replay that did not seal). The file was restored byte-for-byte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MinimizationRejection {
    pub path: String,
    pub reason: String,
}

/// One bounded reduction step: tentatively removed `path`, replayed the
/// pinned failing task against the reduced tree, then either kept or restored
/// the file. `resulting_tree_digest` is the digest of the tree after this
/// step completed (post-removal for accepted steps; restored tree for rejected).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MinimizationStep {
    pub path: String,
    pub accepted: bool,
    pub resulting_tree_digest: String,
    /// Validator exit code observed by the replay, when one ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validator_exit_code: Option<i32>,
}

/// The bounded minimization record. Only incidental material is ever dropped,
/// and every accepted drop was validated by an actual replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MinimizedReproduction {
    /// Algorithm identifier. Must be [`MINIMIZATION_ALGORITHM`].
    pub algorithm: String,
    /// Paths pinned as required; never candidates for removal.
    pub required_paths: Vec<String>,
    /// Digest of the tree before minimization (the pinned source surface, or
    /// an enriched copy of it). Kept as provenance of the reduction attempt.
    pub original_tree_digest: String,
    /// Digest of the tree after minimization.
    pub minimized_tree_digest: String,
    /// Candidates accepted for removal, in acceptance order.
    pub removed_paths: Vec<String>,
    /// Candidates rejected, in attempt order.
    pub rejected_candidates: Vec<MinimizationRejection>,
    /// One entry per attempted removal, in attempt order.
    pub steps: Vec<MinimizationStep>,
}

impl MinimizedReproduction {
    /// Fail-closed internal consistency check. `baseline_digest` is the
    /// chain-pinned baseline the minimized reproduction must reproduce under.
    pub fn validate(&self, baseline_digest: &str) -> Vec<String> {
        let mut defects = Vec::new();
        macro_rules! check_path {
            ($label:expr, $path:expr) => {
                if let Err(message) = check_fixture_path($path) {
                    defects.push(format!("{}: {message}", $label));
                }
            };
        }
        if self.algorithm != MINIMIZATION_ALGORITHM {
            defects.push(format!(
                "algorithm {:?} must be {MINIMIZATION_ALGORITHM:?}",
                self.algorithm
            ));
        }
        if self.required_paths.is_empty() {
            defects.push("requiredPaths must pin at least one required path".to_string());
        }
        {
            let unique: std::collections::BTreeSet<_> = self.required_paths.iter().collect();
            if unique.len() != self.required_paths.len() {
                defects.push("requiredPaths must not contain duplicates".to_string());
            }
        }
        for path in &self.required_paths {
            check_path!("requiredPaths", path);
        }
        {
            let unique: std::collections::BTreeSet<_> = self.removed_paths.iter().collect();
            if unique.len() != self.removed_paths.len() {
                defects.push("removedPaths must not contain duplicates".to_string());
            }
        }
        for path in &self.removed_paths {
            check_path!("removedPaths", path);
            if self.required_paths.contains(path) {
                defects.push(format!("removedPaths must not drop required path {path:?}"));
            }
        }
        {
            let unique: std::collections::BTreeSet<_> = self
                .rejected_candidates
                .iter()
                .map(|rejection| &rejection.path)
                .collect();
            if unique.len() != self.rejected_candidates.len() {
                defects.push("rejectedCandidates must not contain duplicate paths".to_string());
            }
        }
        for rejection in &self.rejected_candidates {
            check_path!("rejectedCandidates", &rejection.path);
            if self.required_paths.contains(&rejection.path) {
                defects.push(format!(
                    "rejectedCandidates must not list required path {:?}",
                    rejection.path
                ));
            }
            if rejection.reason.trim().is_empty() {
                defects.push(format!(
                    "rejectedCandidates path {:?} must carry a non-empty reason",
                    rejection.path
                ));
            }
        }
        for path in &self.removed_paths {
            if self
                .rejected_candidates
                .iter()
                .any(|rejection| &rejection.path == path)
            {
                defects.push(format!(
                    "path {path:?} appears in both removedPaths and rejectedCandidates"
                ));
            }
        }
        // Steps must account for every accepted and every rejected candidate,
        // exactly once each, in order.
        let accepted_in_steps: Vec<&str> = self
            .steps
            .iter()
            .filter(|step| step.accepted)
            .map(|step| step.path.as_str())
            .collect();
        let rejected_in_steps: Vec<&str> = self
            .steps
            .iter()
            .filter(|step| !step.accepted)
            .map(|step| step.path.as_str())
            .collect();
        if accepted_in_steps
            != self
                .removed_paths
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        {
            defects.push("steps accepted in order must equal removedPaths exactly".to_string());
        }
        if rejected_in_steps
            != self
                .rejected_candidates
                .iter()
                .map(|rejection| rejection.path.as_str())
                .collect::<Vec<_>>()
        {
            defects.push(
                "steps rejected in order must equal rejectedCandidates paths exactly".to_string(),
            );
        }
        for step in &self.steps {
            check_path!("steps", &step.path);
            if !is_hex64(&step.resulting_tree_digest) {
                defects.push(format!(
                    "steps path {:?} resultingTreeDigest must be a lower hex sha256 digest",
                    step.path
                ));
            }
        }
        if !is_hex64(&self.original_tree_digest) {
            defects.push("originalTreeDigest must be a lower hex sha256 digest".to_string());
        }
        if !is_hex64(&self.minimized_tree_digest) {
            defects.push("minimizedTreeDigest must be a lower hex sha256 digest".to_string());
        } else if self.minimized_tree_digest != baseline_digest {
            defects.push(
                "minimizedTreeDigest does not equal the chain-pinned baseline digest; the minimized reproduction must reproduce under the pinned source pins"
                    .to_string(),
            );
        }
        defects
    }
}

// ---------------------------------------------------------------------------
// Export scan (redaction boundary)
// ---------------------------------------------------------------------------

/// Summary of the redaction/secret scan applied to the exported case. The
/// scan REFUSES the export on any hit; it never rewrites the case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FailureCaseRedaction {
    /// Identifier of the scanner that ran at export.
    pub scanner: String,
    /// Number of known secrets registered for the scan.
    pub known_secrets_registered: usize,
    /// Always true: [`SECRET_CANARY`] was explicitly checked absent.
    pub canary_scanned: bool,
    /// Number of registered secrets (or the canary) found in the export.
    /// Must be 0.
    pub leaked_secrets: usize,
    /// True when no machine-specific path material is present.
    pub machine_path_free: bool,
}

struct ExportScan {
    leaked_secrets: usize,
    machine_path_free: bool,
}

fn scan_export_bytes(json: &str, known_secrets: &[String]) -> ExportScan {
    let mut leaked = 0usize;
    for secret in known_secrets {
        if !secret.is_empty() && json.contains(secret.as_str()) {
            leaked += 1;
        }
    }
    if json.contains(SECRET_CANARY) {
        leaked += 1;
    }
    // Digest-only exports contain no backslashes at all; anything machine
    // path shaped shows up as a backslash (drive or UNC) or a unix home.
    let machine_path_free = !json.contains('\\')
        && !json.contains("file://")
        && !json.contains("/home/")
        && !json.contains("/Users/")
        && !json.contains("/root/");
    ExportScan {
        leaked_secrets: leaked,
        machine_path_free,
    }
}

fn scan_export(json: &str, known_secrets: &[String]) -> Result<ExportScan> {
    // The case is a digest-only artifact: redaction must be a *no-op* on the
    // export. Any byte that needs rewriting means unauthorized material
    // entered the case, and the export is refused rather than republished.
    let redacted = redact_diagnostics(json, known_secrets);
    ensure!(
        redacted == json,
        "failure case export requires redaction to be safe; refusing to emit a rewritten case"
    );
    let scan = scan_export_bytes(&redacted, known_secrets);
    ensure!(
        scan.leaked_secrets == 0,
        "a known-secret canary survived redaction in the failure case export"
    );
    ensure!(
        scan.machine_path_free,
        "the failure case export contains machine-specific path material"
    );
    Ok(scan)
}

// ---------------------------------------------------------------------------
// The versioned Lite-native failure case
// ---------------------------------------------------------------------------

/// The versioned Lite-internal failure case. Parse with
/// [`LiteFailureCase::parse_json`]; never construct and skip validation.
///
/// Binds: the original failure identity; the source/task pins; the failure
/// classification; the minimized reproduction; the effective authority; the
/// expected result; the validator proof; the controlled dimensions; the
/// reproduction evidence; the fix evidence; the provenance; the export scan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiteFailureCase {
    pub schema_version: String,
    pub namespace: String,
    /// Lite-internal document version (independent of schema_version).
    pub version: String,
    /// Deterministic, wall-clock-free identifier derived from the failure
    /// identity: two exports of the same failure derive the same case id.
    pub case_id: String,
    pub title: String,
    /// Pinned evaluation instant (deterministic; equal to the chain's).
    pub evaluated_at: String,
    /// Stable, wall-clock-free identity of the original failure (dedup key).
    pub failure_identity: String,
    pub source: FailureCaseSourcePin,
    /// The classification the original failure sealed with.
    pub classification: ScenarioOutcomeClass,
    /// The task outcome the original failure sealed with (`repair-failed`).
    pub task_outcome: TaskOutcome,
    /// The distinct replay failure class (`governed-validation-failure`).
    pub failure_class: FailureReplayClass,
    /// The known failed validator.
    pub validator: FailureValidatorPin,
    /// Authority pins: the fix holds exactly the failure'\''s authority.
    pub authority: FailureAuthorityPin,
    /// Controlled dimensions pinned by the embedded scenario descriptor.
    pub controlled_dimensions: Vec<String>,
    pub expected_result: FailureCaseExpected,
    pub minimized_reproduction: MinimizedReproduction,
    /// `contentDigest` of the embedded original-failure envelope.
    pub reproduction_evidence_digest: String,
    /// `contentDigest` of the embedded replay (fix) envelope.
    pub fix_evidence_digest: String,
    /// `contentDigest` of the embedded failure-replay chain.
    pub chain_evidence_digest: String,
    pub redaction: FailureCaseRedaction,
    pub provenance: RunProvenance,
    pub verification_scope: String,
    pub external_evaluation_format: ExternalEvaluationFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
}

impl LiteFailureCase {
    /// Canonical digest of the case minus `contentDigest`.
    pub fn compute_digest(&self) -> Result<String> {
        let mut value = serde_json::to_value(self).context("failure case serializes")?;
        if let Some(obj) = value.as_object_mut() {
            obj.remove("contentDigest");
        }
        soma::try_canonical_digest(&value)
            .map_err(|error| anyhow::anyhow!("failure case canonical digest failed: {error}"))
    }

    /// Fail-closed parse + validation.
    pub fn parse_json(text: &str) -> Result<Self> {
        let case: Self = serde_json::from_str(text).map_err(|error| {
            anyhow::anyhow!(
                "failure case is not a valid {FAILURE_CASE_NAMESPACE} document: {error}"
            )
        })?;
        case.verify()?;
        Ok(case)
    }

    /// Internal consistency checks (no chain required). Returns every defect.
    fn defects(&self) -> Vec<String> {
        let mut defects = Vec::new();

        if self.schema_version != FAILURE_CASE_SCHEMA_VERSION {
            defects.push(format!(
                "unsupported schemaVersion {:?} (supported {FAILURE_CASE_SCHEMA_VERSION})",
                self.schema_version
            ));
        }
        if self.namespace != FAILURE_CASE_NAMESPACE {
            defects.push(format!(
                "namespace {:?} is not {FAILURE_CASE_NAMESPACE:?}",
                self.namespace
            ));
        }
        if self.version != FAILURE_CASE_VERSION {
            defects.push(format!(
                "unsupported version {:?} (supported {FAILURE_CASE_VERSION})",
                self.version
            ));
        }
        if self.case_id.trim().is_empty() {
            defects.push("caseId must be non-empty".to_string());
        }
        if self.title.trim().is_empty() {
            defects.push("title must be non-empty".to_string());
        }
        if !is_hex64(&self.failure_identity) {
            defects.push("failureIdentity must be a lower hex sha256 digest".to_string());
        }
        if self.case_id
            != format!(
                "lite-failure-case-{}",
                &self.failure_identity[..16.min(self.failure_identity.len())]
            )
        {
            defects.push("caseId must derive from failureIdentity".to_string());
        }
        if self.evaluated_at.trim().is_empty() {
            defects.push("evaluatedAt must be non-empty".to_string());
        }

        // Source pins.
        if self.source.scenario_id.trim().is_empty() {
            defects.push("source.scenarioId must be non-empty".to_string());
        }
        for (label, digest) in [
            ("source.scenarioDigest", &self.source.scenario_digest),
            (
                "source.repositoryBaselineDigest",
                &self.source.repository_baseline_digest,
            ),
            (
                "source.taskStatementDigest",
                &self.source.task_statement_digest,
            ),
        ] {
            if !is_hex64(digest) {
                defects.push(format!("{label} must be a lower hex sha256 digest"));
            }
        }
        if self.source.repository_revision.trim().is_empty() {
            defects.push("source.repositoryRevision must be non-empty".to_string());
        } else if self
            .source
            .repository_revision
            .chars()
            .any(|c| c == '/' || c == '\\' || c == '\0')
        {
            defects.push("source.repositoryRevision must be a bare revision".to_string());
        }
        for (label, fixture) in [("source.repositoryFixture", &self.source.repository_fixture)] {
            if let Err(message) = check_fixture_path(fixture) {
                defects.push(format!("{label}: {message}"));
            }
        }
        for (label, value) in [
            ("source.harnessId", &self.source.harness_id),
            ("source.harnessRevision", &self.source.harness_revision),
            ("source.providerIdentity", &self.source.provider_identity),
            ("source.model", &self.source.model),
        ] {
            if value.trim().is_empty() {
                defects.push(format!("{label} must be non-empty"));
            }
        }

        // Failure classification: governed validation failure only.
        if self.classification != ScenarioOutcomeClass::GovernedProductOutcome {
            defects.push(
                "classification must be governed-product-outcome (a sealed governed failure)"
                    .to_string(),
            );
        }
        if self.task_outcome != TaskOutcome::RepairFailed {
            defects.push(
                "taskOutcome must be repair-failed; a repaired run is not a failure".to_string(),
            );
        }
        if self.failure_class != FailureReplayClass::GovernedValidationFailure {
            defects.push("failureClass must be governed-validation-failure".to_string());
        }

        // Validator proof.
        if self.validator.exit_code == 0 {
            defects.push(
                "validator.exitCode must be non-zero; a zero-exit validator outcome is not a failure"
                    .to_string(),
            );
        }
        if self.validator.command.trim().is_empty() {
            defects.push("validator.command must be non-empty".to_string());
        } else if self
            .validator
            .command
            .chars()
            .any(|c| matches!(c, '/' | '\\' | '\0'))
        {
            defects
                .push("validator.command must be a bare executable name, not a path".to_string());
        }
        if self.validator.args.is_empty() {
            defects.push("validator.args must pin at least one argument".to_string());
        }
        for arg in &self.validator.args {
            if arg.trim().is_empty() {
                defects.push("validator.args entries must be non-empty".to_string());
            }
        }

        // Expected result: a fix that satisfies the original regression.
        if self.expected_result.task_outcome != TaskOutcome::Repaired {
            defects.push("expectedResult.taskOutcome must be repaired".to_string());
        }
        if self.expected_result.classification != ScenarioOutcomeClass::GovernedProductOutcome {
            defects
                .push("expectedResult.classification must be governed-product-outcome".to_string());
        }
        if self.expected_result.validator_exit_code == self.validator.exit_code {
            defects.push(
                "expectedResult.validatorExitCode must differ from the original failure exit code"
                    .to_string(),
            );
        }
        if self.expected_result.expected_assertion.trim().is_empty() {
            defects.push("expectedResult.expectedAssertion must be non-empty".to_string());
        }

        // Authority pins.
        if self.authority.writable_scopes.is_empty() {
            defects.push("authority.writableScopes must pin at least one scope".to_string());
        }
        if self.authority.changed_files.is_empty() {
            defects.push("authority.changedFiles must pin at least one repair target".to_string());
        }
        for path in &self.authority.changed_files {
            if let Err(message) = check_fixture_path(path) {
                defects.push(format!("authority.changedFiles: {message}"));
            }
        }

        // Controlled dimensions.
        if self.controlled_dimensions.is_empty() {
            defects.push("controlledDimensions must not be empty".to_string());
        }
        {
            let unique: std::collections::BTreeSet<_> = self.controlled_dimensions.iter().collect();
            if unique.len() != self.controlled_dimensions.len() {
                defects.push("controlledDimensions must not contain duplicates".to_string());
            }
        }

        // Minimized reproduction binds the chain baseline.
        defects.extend(
            self.minimized_reproduction
                .validate(&self.source.repository_baseline_digest),
        );

        // Evidence digests.
        for (label, digest) in [
            (
                "reproductionEvidenceDigest",
                &self.reproduction_evidence_digest,
            ),
            ("fixEvidenceDigest", &self.fix_evidence_digest),
            ("chainEvidenceDigest", &self.chain_evidence_digest),
        ] {
            if !is_hex64(digest) {
                defects.push(format!("{label} must be a lower hex sha256 digest"));
            }
        }

        // Export scan record.
        if self.redaction.scanner != FAILURE_CASE_SCANNER_ID {
            defects.push(format!(
                "redaction.scanner must be {FAILURE_CASE_SCANNER_ID:?}"
            ));
        }
        if !self.redaction.canary_scanned {
            defects.push("redaction.canaryScanned must be true".to_string());
        }
        if self.redaction.leaked_secrets != 0 {
            defects.push("redaction.leakedSecrets must be 0".to_string());
        }
        if !self.redaction.machine_path_free {
            defects.push("redaction.machinePathFree must be true".to_string());
        }

        defects
    }

    /// Fail-closed verification from the case bytes alone (all internal
    /// bindings re-derive). For cross-envelope proof call
    /// [`LiteFailureCase::verify_against_chain`].
    pub fn verify(&self) -> Result<()> {
        use anyhow::ensure;
        ensure!(
            self.schema_version == FAILURE_CASE_SCHEMA_VERSION,
            "unsupported failure-case schema version {} (supported {FAILURE_CASE_SCHEMA_VERSION})",
            self.schema_version
        );
        ensure!(
            self.namespace == FAILURE_CASE_NAMESPACE,
            "failure-case namespace {:?} is not {FAILURE_CASE_NAMESPACE:?}",
            self.namespace
        );
        match &self.content_digest {
            Some(actual) => {
                let expected = self.compute_digest()?;
                ensure!(
                    *actual == expected,
                    "failure-case contentDigest does not verify"
                );
            }
            None => bail!("failure case is missing its contentDigest"),
        }
        ensure!(
            self.verification_scope == FAILURE_CASE_VERIFICATION_SCOPE,
            "failure-case verificationScope must be pinned to the documented verification limitation"
        );
        let defects = self.defects();
        ensure!(
            defects.is_empty(),
            "failure case is invalid: {}",
            defects.join("; ")
        );
        ensure!(
            self.external_evaluation_format.status == "unsupported",
            "externalEvaluationFormat.status must be \"unsupported\""
        );
        ensure!(
            self.provenance.fixture_only && self.provenance.injected_clock,
            "failure case must be fixture-only with an injected clock"
        );
        Ok(())
    }

    /// Re-derive every binding between the case and the failure-replay chain
    /// it was exported from. Fail-closed: any mismatch rejects the case.
    pub fn verify_against_chain(&self, chain: &LiteFailureReplayEvidence) -> Result<()> {
        chain.verify().context("embedded chain fails to verify")?;
        ensure!(
            self.failure_identity == chain.failure_identity,
            "case failureIdentity does not match the chain's failure identity"
        );
        ensure!(
            self.evaluated_at == chain.evaluated_at,
            "case evaluatedAt does not match the chain's evaluation instant"
        );
        let failure = &chain.original_failure;
        let descriptor = &failure.scenario_run.descriptor;
        ensure!(
            self.source.scenario_id == chain.descriptor.original.scenario_id
                && self.source.scenario_id == descriptor.scenario_id,
            "case source.scenarioId does not match the chain"
        );
        ensure!(
            self.source.scenario_digest == chain.descriptor.original.scenario_digest,
            "case source.scenarioDigest does not match the chain"
        );
        ensure!(
            self.source.repository_baseline_digest
                == chain.descriptor.original.baseline_content_digest
                && self.source.repository_baseline_digest == failure.baseline_content_digest,
            "case source.repositoryBaselineDigest does not match the chain"
        );
        ensure!(
            self.source.repository_fixture == descriptor.repository.fixture,
            "case source.repositoryFixture does not match the chain"
        );
        ensure!(
            self.source.repository_revision == descriptor.repository.revision,
            "case source.repositoryRevision does not match the chain (stale revision)"
        );
        ensure!(
            self.source.harness_id == descriptor.harness.id
                && self.source.harness_revision == descriptor.harness.revision,
            "case harness pins do not match the chain"
        );
        ensure!(
            self.source.provider_identity == descriptor.route.provider_identity
                && self.source.model == descriptor.route.model,
            "case provider/model pins do not match the chain"
        );
        ensure!(
            self.classification == failure.classification
                && self.task_outcome == failure.task_outcome
                && self.failure_class == chain.descriptor.original.failure_class,
            "case failure classification does not match the chain"
        );
        ensure!(
            self.validator == chain.descriptor.original.validator,
            "case validator pin does not match the chain"
        );
        ensure!(
            self.authority == chain.descriptor.authority,
            "case authority pins do not match the chain (authority may not change between failure and fix)"
        );
        ensure!(
            self.controlled_dimensions == descriptor.dimensions.controlled,
            "case controlledDimensions do not match the chain"
        );
        ensure!(
            self.expected_result.task_outcome == chain.descriptor.replay.task_outcome
                && self.expected_result.classification == chain.descriptor.replay.classification
                && self.expected_result.validator_exit_code
                    == chain.descriptor.replay.expected_exit_code,
            "case expectedResult does not match the chain replay pin"
        );
        ensure!(
            self.expected_result.expected_assertion == chain.replay.validator.assertion,
            "case expectedResult.expectedAssertion does not match the chain replay validator assertion"
        );
        ensure!(
            self.reproduction_evidence_digest
                == failure
                    .content_digest
                    .as_deref()
                    .context("chain failure envelope carries no contentDigest")?,
            "case reproductionEvidenceDigest does not match the chain's failure envelope"
        );
        ensure!(
            self.fix_evidence_digest
                == chain
                    .replay
                    .content_digest
                    .as_deref()
                    .context("chain replay envelope carries no contentDigest")?,
            "case fixEvidenceDigest does not match the chain's replay envelope"
        );
        ensure!(
            self.chain_evidence_digest
                == chain
                    .content_digest
                    .as_deref()
                    .context("chain carries no contentDigest")?,
            "case chainEvidenceDigest does not match the chain"
        );
        let statement_digest = soma::try_canonical_digest(&serde_json::Value::String(
            descriptor.task.statement.clone(),
        ))
        .context("task statement canonical digest failed")?;
        ensure!(
            self.source.task_statement_digest == statement_digest,
            "case source.taskStatementDigest does not match the chain"
        );
        Ok(())
    }
}

impl LiteFailureCase {
    /// Build and seal a failure case from a verified failure-replay chain and
    /// a bounded minimization record. Every binding is checked BEFORE
    /// sealing; on any mismatch this returns the discrepancies and nothing is
    /// exported. The export passes a fail-closed secret scan.
    pub fn sealed(
        chain: &LiteFailureReplayEvidence,
        minimization: MinimizedReproduction,
        known_secrets: &[String],
    ) -> Result<Self> {
        chain
            .verify()
            .context("the failure-replay chain fails to verify")?;
        let failure = &chain.original_failure;
        let descriptor = &failure.scenario_run.descriptor;

        let minimization_defects =
            minimization.validate(&chain.descriptor.original.baseline_content_digest);
        if !minimization_defects.is_empty() {
            bail!(
                "minimized reproduction rejected: {}",
                minimization_defects.join("; ")
            );
        }

        let statement_digest = soma::try_canonical_digest(&serde_json::Value::String(
            descriptor.task.statement.clone(),
        ))
        .context("task statement canonical digest failed")?;
        let failure_identity = chain.failure_identity.clone();
        let mut case = Self {
            schema_version: FAILURE_CASE_SCHEMA_VERSION.to_string(),
            namespace: FAILURE_CASE_NAMESPACE.to_string(),
            version: FAILURE_CASE_VERSION.to_string(),
            case_id: format!("lite-failure-case-{}", &failure_identity[..16]),
            title: format!(
                "Lite failure case: {} ({})",
                chain.descriptor.original.scenario_id,
                chain.descriptor.original.failure_class.as_str()
            ),
            evaluated_at: chain.evaluated_at.clone(),
            failure_identity,
            source: FailureCaseSourcePin {
                scenario_id: chain.descriptor.original.scenario_id.clone(),
                scenario_digest: chain.descriptor.original.scenario_digest.clone(),
                repository_revision: descriptor.repository.revision.clone(),
                repository_fixture: descriptor.repository.fixture.clone(),
                repository_baseline_digest: chain.descriptor.original.baseline_content_digest.clone(),
                task_statement_digest: statement_digest,
                harness_id: descriptor.harness.id.clone(),
                harness_revision: descriptor.harness.revision.clone(),
                provider_identity: descriptor.route.provider_identity.clone(),
                model: descriptor.route.model.clone(),
            },
            classification: failure.classification,
            task_outcome: failure.task_outcome,
            failure_class: chain.descriptor.original.failure_class,
            validator: chain.descriptor.original.validator.clone(),
            authority: chain.descriptor.authority.clone(),
            controlled_dimensions: descriptor.dimensions.controlled.clone(),
            expected_result: FailureCaseExpected {
                task_outcome: chain.descriptor.replay.task_outcome,
                classification: chain.descriptor.replay.classification,
                validator_exit_code: chain.descriptor.replay.expected_exit_code,
                expected_assertion: chain.replay.validator.assertion.clone(),
            },
            minimized_reproduction: minimization,
            reproduction_evidence_digest: failure
                .content_digest
                .clone()
                .context("chain failure envelope carries no contentDigest")?,
            fix_evidence_digest: chain
                .replay
                .content_digest
                .clone()
                .context("chain replay envelope carries no contentDigest")?,
            chain_evidence_digest: chain
                .content_digest
                .clone()
                .context("chain carries no contentDigest")?,
            redaction: FailureCaseRedaction {
                scanner: FAILURE_CASE_SCANNER_ID.to_string(),
                known_secrets_registered: known_secrets.iter().filter(|s| !s.is_empty()).count(),
                canary_scanned: true,
                leaked_secrets: 0,
                machine_path_free: true,
            },
            provenance: RunProvenance {
                runner: FAILURE_CASE_RUNNER_ID.to_string(),
                runner_revision: FAILURE_CASE_RUNNER_REVISION.to_string(),
                injected_clock: true,
                fixture_only: true,
                redaction: "digest-only artifacts; export is refused (never rewritten) when the scan finds any registered secret, the test canary, or machine-path material".to_string(),
            },
            verification_scope: FAILURE_CASE_VERIFICATION_SCOPE.to_string(),
            external_evaluation_format: ExternalEvaluationFormat::unsupported(),
            content_digest: None,
        };
        // Export scan BEFORE the digest is computed: the sealed case must
        // already carry the scan summary.
        let pre_json = serde_json::to_string_pretty(&case).context("failure case serializes")?;
        let scan = scan_export(&pre_json, known_secrets)?;
        case.redaction.known_secrets_registered =
            known_secrets.iter().filter(|s| !s.is_empty()).count();
        case.redaction.leaked_secrets = scan.leaked_secrets;
        case.redaction.machine_path_free = scan.machine_path_free;
        // Re-scan the final byte image: the redaction record itself must be scan-clean.
        let final_json = serde_json::to_string_pretty(&case).context("failure case serializes")?;
        scan_export(&final_json, known_secrets)?;
        case.content_digest = Some(case.compute_digest()?);
        case.verify()?;
        case.verify_against_chain(chain)?;
        Ok(case)
    }
}

/// Serialize the failure case for export. Re-runs the fail-closed scan on the
/// exact bytes a consumer would persist; a case that fails the scan is never
/// returned.
pub fn case_export_json(case: &LiteFailureCase, known_secrets: &[String]) -> Result<String> {
    case.verify()?;
    let json = serde_json::to_string_pretty(case).context("failure case serializes")?;
    scan_export(&json, known_secrets)?;
    Ok(json)
}

// ---------------------------------------------------------------------------
// Deduplication (never lose provenance)
// ---------------------------------------------------------------------------

/// One observed occurrence of a failure case. Distinct occurrences of an
/// equivalent failure retain their own provenance; nothing is discarded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FailureCaseOccurrence {
    pub case_id: String,
    /// `contentDigest` of this occurrence's sealed case.
    pub case_digest: String,
    /// Digest of the failure-replay chain this occurrence was exported from.
    pub chain_evidence_digest: String,
    /// Exporter runner id recorded on the occurrence's provenance.
    pub runner: String,
    pub runner_revision: String,
    /// Evaluation instant pinned on the occurrence.
    pub evaluated_at: String,
}

/// A group of equivalent occurrences. `representative_case_digest` is the
/// lexicographically smallest case digest (deterministic, wall-clock-free).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DedupGroup {
    pub failure_identity: String,
    /// Canonical digest of the full pin set (source/task/validator/authority/
    /// expected/dimensions). Two cases merge only when this matches too.
    pub equivalence_digest: String,
    pub representative_case_digest: String,
    pub occurrences: Vec<FailureCaseOccurrence>,
}

/// Result of deduplicating a set of cases. Every input case appears exactly
/// once in the output (accounted); conflicting same-identity cases are kept
/// as separate groups and reported, never merged silently.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DedupReport {
    pub groups: Vec<DedupGroup>,
    /// Human-readable conflict notes for same-identity cases with
    /// incompatible provenance/pins (kept separate, never merged).
    pub conflicts: Vec<String>,
    /// Total occurrences across all groups. Must equal the input size.
    pub occurrences_total: usize,
}

impl DedupReport {
    /// Number of input cases fully accounted for by the report.
    pub fn accounted(&self) -> usize {
        self.groups
            .iter()
            .map(|group| group.occurrences.len())
            .sum()
    }
}

/// Digest of the full pin set. Equivalent cases merge; same-identity cases
/// with different pins (incompatible provenance, stale revision, widened
/// authority, swapped validator) get distinct equivalence digests and are
/// kept separate with a conflict note.
fn equivalence_digest(case: &LiteFailureCase) -> Result<String> {
    let value = serde_json::json!({
        "failureIdentity": case.failure_identity,
        "source": case.source,
        "validator": case.validator,
        "authority": case.authority,
        "controlledDimensions": case.controlled_dimensions,
        "expectedResult": case.expected_result,
        "failureClass": case.failure_class,
        "classification": case.classification,
        "taskOutcome": case.task_outcome,
    });
    soma::try_canonical_digest(&value)
        .map_err(|error| anyhow::anyhow!("equivalence canonical digest failed: {error}"))
}

/// Deduplicate cases by stable failure identity AND full pin equivalence.
/// Fail-closed: any case that fails [`LiteFailureCase::verify`] aborts the
/// whole operation. No occurrence is ever dropped: every input case appears
/// exactly once in the report.
pub fn dedup_cases(cases: &[LiteFailureCase]) -> Result<DedupReport> {
    use std::collections::BTreeMap;

    for case in cases {
        case.verify()
            .context("deduplication refuses an invalid case")?;
    }

    let mut by_identity: BTreeMap<&str, Vec<&LiteFailureCase>> = BTreeMap::new();
    for case in cases {
        by_identity
            .entry(case.failure_identity.as_str())
            .or_default()
            .push(case);
    }

    let mut groups: Vec<DedupGroup> = Vec::new();
    let mut conflicts: Vec<String> = Vec::new();

    for (identity, members) in by_identity {
        let mut by_equivalence: BTreeMap<String, Vec<&LiteFailureCase>> = BTreeMap::new();
        for member in members {
            by_equivalence
                .entry(equivalence_digest(member)?)
                .or_default()
                .push(member);
        }
        for (equivalence, equivalent_members) in by_equivalence {
            let mut occurrences: Vec<FailureCaseOccurrence> = equivalent_members
                .into_iter()
                .map(|case| {
                    Ok(FailureCaseOccurrence {
                        case_id: case.case_id.clone(),
                        case_digest: case
                            .content_digest
                            .clone()
                            .context("case is missing its contentDigest")?,
                        chain_evidence_digest: case.chain_evidence_digest.clone(),
                        runner: case.provenance.runner.clone(),
                        runner_revision: case.provenance.runner_revision.clone(),
                        evaluated_at: case.evaluated_at.clone(),
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            occurrences.sort_by(|a, b| a.case_digest.cmp(&b.case_digest));
            let representative = occurrences
                .first()
                .map(|occurrence| occurrence.case_digest.clone())
                .context("equivalence group must contain at least one case")?;
            groups.push(DedupGroup {
                failure_identity: identity.to_string(),
                equivalence_digest: equivalence,
                representative_case_digest: representative,
                occurrences,
            });
        }
        if groups
            .iter()
            .filter(|group| group.failure_identity == identity)
            .count()
            > 1
        {
            let digests: Vec<String> = groups
                .iter()
                .filter(|group| group.failure_identity == identity)
                .map(|group| group.equivalence_digest[..16].to_string())
                .collect();
            conflicts.push(format!(
                "failure identity {identity} observed with incompatible pins; kept {} separate groups (equivalence digests: {}), none merged",
                digests.len(),
                digests.join(", ")
            ));
        }
    }

    groups.sort_by(|a, b| {
        a.failure_identity
            .cmp(&b.failure_identity)
            .then_with(|| a.equivalence_digest.cmp(&b.equivalence_digest))
    });
    let occurrences_total = groups.iter().map(|group| group.occurrences.len()).sum();
    Ok(DedupReport {
        groups,
        conflicts,
        occurrences_total,
    })
}

// ---------------------------------------------------------------------------
// Minimized reproduction (bounded, replay-validated)
// ---------------------------------------------------------------------------

/// The wall-clock-free signature of the original failure. A minimized
/// reproduction is accepted only when the replay of the reduced tree
/// reproduces EXACTLY this signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureSignature {
    pub validator_command: String,
    pub validator_args: Vec<String>,
    pub validator_exit_code: i32,
    pub task_outcome: TaskOutcome,
    pub classification: ScenarioOutcomeClass,
    pub failure_class: FailureReplayClass,
}

impl FailureSignature {
    /// Derive the expected failure signature from a verified chain.
    pub fn from_chain(chain: &LiteFailureReplayEvidence) -> Result<Self> {
        chain.verify()?;
        Ok(Self {
            validator_command: chain.descriptor.original.validator.command.clone(),
            validator_args: chain.descriptor.original.validator.args.clone(),
            validator_exit_code: chain.descriptor.original.validator.exit_code,
            task_outcome: chain.original_failure.task_outcome,
            classification: chain.original_failure.classification,
            failure_class: chain.descriptor.original.failure_class,
        })
    }

    /// True only when the reduced tree replays to EXACTLY the original
    /// failure signature. Any drift — class, outcome, command, args, exit
    /// code — is a reproduction failure, not a minimization success.
    pub fn reproduces(&self, replay: &ReplaySignature) -> bool {
        replay.sealed
            && replay.validator_command.as_deref() == Some(self.validator_command.as_str())
            && replay.validator_args == self.validator_args
            && replay.validator_exit_code == Some(self.validator_exit_code)
            && replay.task_outcome == Some(self.task_outcome)
            && replay.classification == Some(self.classification)
            && replay.failure_class == Some(self.failure_class)
    }

    /// Sanitized one-line description of what was observed (no paths, no
    /// raw output bytes; safe to persist on the case).
    fn describe_observed(replay: &ReplaySignature) -> String {
        format!(
            "sealed={} outcome={} classification={} command={} args={:?} exit={}",
            replay.sealed,
            replay
                .task_outcome
                .map(|outcome| match outcome {
                    TaskOutcome::Repaired => "repaired",
                    TaskOutcome::RepairFailed => "repair-failed",
                    TaskOutcome::RepairRefused => "repair-refused",
                })
                .unwrap_or("none"),
            replay
                .classification
                .map(ScenarioOutcomeClass::as_str)
                .unwrap_or("none"),
            replay.validator_command.as_deref().unwrap_or("none"),
            replay.validator_args,
            replay
                .validator_exit_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "none".to_string()),
        )
    }
}

/// The outcome of one replay of the pinned failing task against a reduced
/// (or restored) tree. Constructed by the caller's oracle (which owns the
/// governed task boundary); the minimizer stays boundary-agnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplaySignature {
    /// True when the replay sealed a task-run envelope.
    pub sealed: bool,
    pub task_outcome: Option<TaskOutcome>,
    pub classification: Option<ScenarioOutcomeClass>,
    pub failure_class: Option<FailureReplayClass>,
    pub validator_command: Option<String>,
    pub validator_args: Vec<String>,
    pub validator_exit_code: Option<i32>,
}

/// Oracle for one replay of the pinned failing task against the reduced
/// tree. Implementations stage the tree and run the real governed boundary.
pub trait MinimizationOracle {
    /// Replay the pinned failing task against `tree_root` and report the
    /// observed signature. `step_index` identifies the candidate attempt.
    fn replay_reduced_tree(
        &mut self,
        tree_root: &Path,
        step_index: usize,
    ) -> Result<ReplaySignature>;
}

/// Deterministic list of every regular file under `root` (forward-slash,
/// relative, sorted). Fail-closed like the fixture pins: symlinks and
/// unsupported file types are rejected.
pub fn collect_fixture_files(root: &Path) -> Result<Vec<String>> {
    let root_type = std::fs::symlink_metadata(root)
        .with_context(|| format!("fixture root {} does not exist", root.display()))?;
    ensure!(root_type.is_dir(), "fixture root is not a directory");
    let mut entries: Vec<String> = Vec::new();
    collect_files_inner(root, root, &mut entries)?;
    entries.sort();
    entries.dedup();
    Ok(entries)
}

fn collect_files_inner(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
    for entry in std::fs::read_dir(dir)
        .with_context(|| format!("fixture dir {} unreadable", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            bail!("fixture contains a symbolic link: {}", path.display());
        }
        if file_type.is_dir() {
            collect_files_inner(root, &path, out)?;
        } else if file_type.is_file() {
            let rel = path
                .strip_prefix(root)
                .context("fixture path escapes root")?;
            let rel = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            out.push(rel);
        } else {
            bail!(
                "fixture contains an unsupported file type: {}",
                path.display()
            );
        }
    }
    Ok(())
}

fn prune_empty_parent_dirs(start: &Path, root: &Path) {
    let mut dir = start.to_path_buf();
    while dir != root {
        match std::fs::remove_dir(&dir) {
            Ok(()) => match dir.parent() {
                Some(parent) => dir = parent.to_path_buf(),
                None => return,
            },
            Err(_) => return,
        }
    }
}

fn sanitize_replay_error(error: &anyhow::Error, tree_root: &Path) -> String {
    let root_string = tree_root.to_string_lossy().into_owned();
    let mut message = format!("{error:#}");
    // No machine paths may reach the case: drop everything but the first
    // line and replace the root with a placeholder.
    if let Some(newline) = message.find('\n') {
        message.truncate(newline);
    }
    message = message.replace(&root_string, "<fixture>");
    message.chars().take(300).collect()
}

/// Bounded reduction of a writable tree copy. Only files NOT in
/// `required_paths` are candidates, one at a time; every accepted removal
/// was replayed through the real governed boundary and the pinned failure
/// signature reproduced exactly. A candidate whose removal drifts the class,
/// breaks reproduction, or fails to seal is rejected and restored
/// byte-for-byte. `tree_root` is modified IN PLACE (pass a disposable copy).
pub fn minimize_fixture_tree(
    tree_root: &Path,
    required_paths: &[String],
    original: &FailureSignature,
    oracle: &mut dyn MinimizationOracle,
) -> Result<MinimizedReproduction> {
    let root_type = std::fs::symlink_metadata(tree_root)
        .with_context(|| format!("tree root {} does not exist", tree_root.display()))?;
    ensure!(root_type.is_dir(), "tree root is not a directory");
    for required in required_paths {
        check_fixture_path(required).map_err(anyhow::Error::msg)?;
        ensure!(
            tree_root.join(required).is_file(),
            "required path {required:?} is missing from the tree"
        );
    }
    {
        let unique: std::collections::BTreeSet<_> = required_paths.iter().collect();
        ensure!(
            unique.len() == required_paths.len(),
            "required paths contain duplicates"
        );
    }

    let original_tree_digest = scenario::fixture_content_digest(tree_root)?;
    let candidates = collect_fixture_files(tree_root)?
        .into_iter()
        .filter(|path| !required_paths.iter().any(|required| required == path))
        .collect::<Vec<_>>();

    let mut removed_paths = Vec::new();
    let mut rejected_candidates = Vec::new();
    let mut steps = Vec::new();

    for candidate in candidates {
        let path = tree_root.join(&candidate);
        let bytes =
            std::fs::read(&path).with_context(|| format!("candidate {candidate:?} unreadable"))?;
        std::fs::remove_file(&path)
            .with_context(|| format!("candidate {candidate:?} cannot be removed"))?;
        prune_empty_parent_dirs(path.parent().context("candidate has no parent")?, tree_root);
        let after_removal_digest = scenario::fixture_content_digest(tree_root)?;

        let replay = oracle.replay_reduced_tree(tree_root, steps.len());
        match replay {
            Ok(signature) if original.reproduces(&signature) => {
                removed_paths.push(candidate.clone());
                steps.push(MinimizationStep {
                    path: candidate,
                    accepted: true,
                    resulting_tree_digest: after_removal_digest,
                    validator_exit_code: signature.validator_exit_code,
                });
            }
            other => {
                // Restore byte-for-byte; recreate pruned parent directories.
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)
                        .context("cannot recreate candidate parent directories")?;
                }
                std::fs::write(&path, &bytes)
                    .with_context(|| format!("candidate {candidate:?} cannot be restored"))?;
                let restored_digest = scenario::fixture_content_digest(tree_root)?;
                let reason = match other {
                    Ok(signature) => format!(
                        "reduced tree no longer reproduces the pinned failure signature ({})",
                        FailureSignature::describe_observed(&signature)
                    ),
                    Err(error) => format!(
                        "reduced tree replay failed: {}",
                        sanitize_replay_error(&error, tree_root)
                    ),
                };
                steps.push(MinimizationStep {
                    path: candidate.clone(),
                    accepted: false,
                    resulting_tree_digest: restored_digest,
                    validator_exit_code: None,
                });
                rejected_candidates.push(MinimizationRejection {
                    path: candidate,
                    reason,
                });
            }
        }
    }

    let minimized_tree_digest = scenario::fixture_content_digest(tree_root)?;
    Ok(MinimizedReproduction {
        algorithm: MINIMIZATION_ALGORITHM.to_string(),
        required_paths: required_paths.to_vec(),
        original_tree_digest,
        minimized_tree_digest,
        removed_paths,
        rejected_candidates,
        steps,
    })
}

/// Derive a reduced-tree descriptor from the pinned scenario descriptor.
/// Only the `repository` pins change; every other pin (task, policy,
/// workspace, harness, route, validation, dimensions) is carried over
/// verbatim so the reduced reproduction pins the same task under the same
/// authority. Fail-closed: the derived document must parse and validate.
pub fn derive_reduced_descriptor(
    descriptor_text: &str,
    fixture: &str,
    content_digest: &str,
    revision: &str,
) -> Result<String> {
    // Fixture argument must be a clean relative forward-slash path.
    ensure!(!fixture.is_empty(), "fixture must be non-empty");
    ensure!(
        !fixture.chars().any(|c| c == '\\' || c == ':'),
        "fixture must use forward slashes and must not carry a drive prefix"
    );
    for segment in fixture.split('/') {
        ensure!(
            !segment.is_empty(),
            "fixture must not contain empty segments"
        );
        ensure!(
            segment != "." && segment != "..",
            "fixture must not contain {segment:?}"
        );
    }
    ensure!(
        is_hex64(content_digest),
        "contentDigest must be a lower hex sha256 digest"
    );
    ensure!(
        matches!(revision.len(), 40 | 64)
            && revision
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "revision must be a lower hex git object id (40 or 64 chars)"
    );

    let mut value: serde_json::Value =
        serde_json::from_str(descriptor_text).context("pinned scenario descriptor is not JSON")?;
    let repository = value
        .get_mut("repository")
        .and_then(|repository| repository.as_object_mut())
        .context("scenario descriptor pins no repository object")?;
    repository.insert(
        "fixture".to_string(),
        serde_json::Value::String(fixture.to_string()),
    );
    repository.insert(
        "contentDigest".to_string(),
        serde_json::Value::String(content_digest.to_string()),
    );
    repository.insert(
        "revision".to_string(),
        serde_json::Value::String(revision.to_string()),
    );
    let rendered = serde_json::to_string_pretty(&value).context("reduced descriptor serializes")?;
    // Fail-closed: the derived descriptor must parse and validate.
    scenario::ScenarioDescriptor::parse_json(&rendered)
        .map_err(|error| anyhow::anyhow!("reduced descriptor rejected: {error:?}"))?;
    Ok(rendered)
}
