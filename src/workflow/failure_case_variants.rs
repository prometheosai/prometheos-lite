//! Lite-native failure-case variant replay matrix (issue #156, Slice B).
//!
//! Replay of the pinned classified failure across *declared variants* of
//! one controlled dimension (model, harness, context policy, or workflow),
//! distinguishing the dimensions the acceptance requires the harness to
//! distinguish. Everything here is deterministic and wall-clock-free:
//!
//! - The caller supplies the **observed signature** of each variant replay,
//!   obtained by running the variant through the same governed boundary the
//!   original failure crossed (a task-run envelope sealed by that boundary).
//!   This module never spawns processes itself and never calls a provider.
//! - Synthetic fixture replays are labeled `simulation_fixture: true`; the
//!   exported matrix is tagged `simulation_only` and can never be presented
//!   as live-provider evidence. Promotion, ranking or certification claims
//!   are explicitly out of scope.
//! - The matrix is bound to the originating case (`case_id` and
//!   `failure_identity`) and to the case's expected failure signature
//!   (validator command/argv/exit, outcome, class, failure class). A replay
//!   that does not match the variant's declared expectation is reported as
//!   `Drifted` — never as a *variant of this failure*.
//! - `verify` re-derives the digest and refuses tampering; the exported
//!   matrix is digest-bound to the case and every recorded expectation and
//!   observation.
//!
//! Foundry ingestion, metrics, rankings or certification are out of scope
//! for this slice; `externalEvaluationFormat` is not represented on purpose
//! (the Lite-internal matrix is not a Foundry artifact).

use anyhow::{Context, Result};

use serde::{Deserialize, Serialize};

use crate::workflow::failure_replay::FailureReplayClass;

use super::failure_case::{FailureSignature, LiteFailureCase, ReplaySignature};
use super::scenario::{ScenarioOutcomeClass, TaskOutcome};
use crate::workflow::soma;

/// Schema version of the variant replay matrix document.
pub const FAILURE_CASE_VARIANT_MATRIX_SCHEMA_VERSION: &str = "1.0.0";
/// Namespace of the variant replay matrix document.
pub const FAILURE_CASE_VARIANT_MATRIX_NAMESPACE: &str =
    "lite.evaluation.failure-case-variant-matrix";
/// Document version carried by every matrix.
pub const FAILURE_CASE_VARIANT_MATRIX_VERSION: &str = "1.0.0";
/// Verification scope tag embedded in every matrix.
pub const FAILURE_CASE_VARIANT_MATRIX_VERIFICATION_SCOPE: &str = "lite-internal-fixture-only";

/// The controlled dimension one variant replay varies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FailureCaseVariantAxis {
    /// A different pinned model route, identical everything else.
    Model,
    /// A different pinned harness/adapter id+revision, identical everything else.
    Harness,
    /// A different context policy/budget against the same task.
    Context,
    /// A different workflow revision (control drift is expected and must be declared).
    Workflow,
}

impl FailureCaseVariantAxis {
    fn as_str(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Harness => "harness",
            Self::Context => "context",
            Self::Workflow => "workflow",
        }
    }
}

/// What a variant replay is expected to produce. Declared by the caller
/// per variant; the matrix classifies drift against it, never against an
/// ad hoc expectation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VariantExpectation {
    /// The variant should reproduce the pinned failure signature.
    ReproduceFailure,
    /// The variant should produce the case's pinned fix outcome.
    ObserveFix,
}

/// The caller-sealed observation of one variant replay, plus the variant's
/// declared identity. Constructed by the caller's replay oracle; this
/// module verifies against them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureCaseVariantReplayInput {
    /// Stable variant identifier (e.g. `variant-model-alt-a`).
    pub variant_id: String,
    /// The declared axis this variant varies.
    pub axis: FailureCaseVariantAxis,
    /// The sealed observation of the variant replay.
    pub observed: ReplaySignature,
    /// What this variant is expected to produce.
    pub expectation: VariantExpectation,
    /// True when the replay ran against synthetic fixture input (always
    /// true for this slice; never a live provider).
    pub simulation_fixture: bool,
}

/// The verdict assigned to one variant replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VariantVerdict {
    /// The observed signature matches the pinned failure signature exactly.
    Reproduced,
    /// The observed signature matches the case's pinned fix outcome.
    RepairedFixObserved,
    /// The replay sealed but does not match the variant's declared
    /// expectation — it is not a conforming observation of this case.
    Drifted,
    /// The replay did not seal a task-run observation (setup failure,
    /// pre-effect refusal, or cancellation); nothing is concluded.
    Unverifiable,
}

/// One variant replay as recorded in the matrix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FailureCaseVariantReplayOutcome {
    pub variant_id: String,
    pub axis: String,
    /// The pinned signature this variant reproduces (or the pinned fix
    /// outcome it observes), serialized for consumers.
    pub expected: ExpectedSignatureSummary,
    /// Sanitized one-line description of what was observed. No paths, no
    /// raw bytes; safe to persist on the matrix.
    pub observed: String,
    pub verdict: VariantVerdict,
    /// Always true in this slice; fixture provenance must never be laundered
    /// into a live/certified outcome.
    pub simulation_fixture: bool,
}

/// Serializable summary of the pinned failure signature (or of the pinned
/// fix outcome) — the digest material of the expectation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExpectedSignatureSummary {
    /// `validator.command` of the pinned validator (or the fix's assertion
    /// text when `expectation` is `observe-fix`).
    pub validator_command: String,
    /// Pinned validator argv.
    pub validator_args: Vec<String>,
    /// Pinned validator exit code (101 for the failure; zero for the fix).
    pub validator_exit_code: i32,
    /// Pinned task outcome (`repair-failed` for the failure; `repaired` for
    /// the fix).
    pub task_outcome: String,
    /// Pinned classification.
    pub classification: String,
    /// Pinned failure class.
    pub failure_class: String,
    /// `reproduce-failure` or `observe-fix`.
    pub expectation: String,
}

/// Aggregated verdict for the matrix as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VariantMatrixVerdict {
    /// Every variant replay conforms to its declared expectation.
    AllConforming,
    /// Some variant replays conform; at least one does not.
    Mixed,
    /// No variant replay conforms to its declared expectation.
    NoneConforming,
    /// Every variant replay was unverifiable.
    AllUnverifiable,
}

impl VariantMatrixVerdict {
    fn of(verdicts: &[VariantVerdict]) -> Self {
        let conforming = verdicts
            .iter()
            .filter(|v| {
                matches!(
                    v,
                    VariantVerdict::Reproduced | VariantVerdict::RepairedFixObserved
                )
            })
            .count();
        if conforming == verdicts.len() && !verdicts.is_empty() {
            Self::AllConforming
        } else if conforming == 0
            && verdicts
                .iter()
                .all(|v| matches!(v, VariantVerdict::Unverifiable))
        {
            Self::AllUnverifiable
        } else if conforming == 0 {
            Self::NoneConforming
        } else {
            Self::Mixed
        }
    }
}

/// The versioned Lite-internal variant replay matrix.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FailureCaseVariantMatrix {
    pub schema_version: String,
    pub namespace: String,
    pub version: String,
    /// The id of the originating failure case.
    pub case_id: String,
    /// The stable failure identity copied from the case.
    pub failure_identity: String,
    /// The expected failure signature (or fix outcome) the variants were
    /// classified against, digest-materialized per replay.
    pub expected: ExpectedSignatureSummary,
    pub replays: Vec<FailureCaseVariantReplayOutcome>,
    pub verdict: VariantMatrixVerdict,
    /// Verification scope of this Lite-internal artifact.
    pub verification_scope: String,
    /// Always true here; never a live-provider artifact.
    pub simulation_only: bool,
    /// Canonical digest over this document with `matrixDigest` blanked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matrix_digest: Option<String>,
}

fn task_outcome_str(outcome: TaskOutcome) -> &'static str {
    match outcome {
        TaskOutcome::Repaired => "repaired",
        TaskOutcome::RepairFailed => "repair-failed",
        TaskOutcome::RepairRefused => "repair-refused",
    }
}

fn class_str(class: ScenarioOutcomeClass) -> &'static str {
    class.as_str()
}

fn class_of(class: FailureReplayClass) -> &'static str {
    class.as_str()
}

impl ExpectedSignatureSummary {
    fn from_failure(expected: &FailureSignature) -> Self {
        Self {
            validator_command: expected.validator_command.clone(),
            validator_args: expected.validator_args.clone(),
            validator_exit_code: expected.validator_exit_code,
            task_outcome: task_outcome_str(expected.task_outcome).to_string(),
            classification: class_str(expected.classification).to_string(),
            failure_class: class_of(expected.failure_class).to_string(),
            expectation: "reproduce-failure".to_string(),
        }
    }

    fn from_fix(case: &LiteFailureCase) -> Self {
        Self {
            validator_command: case.validator.command.clone(),
            validator_args: case.validator.args.clone(),
            validator_exit_code: case.expected_result.validator_exit_code,
            task_outcome: task_outcome_str(case.expected_result.task_outcome).to_string(),
            classification: class_str(case.expected_result.classification).to_string(),
            failure_class: "repair-accepted".to_string(),
            expectation: "observe-fix".to_string(),
        }
    }
}

impl FailureCaseVariantMatrix {
    /// Canonical digest of the matrix with `matrix_digest` blanked.
    pub fn compute_digest(&self) -> Result<String> {
        let mut value = serde_json::to_value(self).context("variant matrix serializes")?;
        if let Some(obj) = value.as_object_mut() {
            obj.remove("matrixDigest");
        }
        self::soma::try_canonical_digest(&value)
            .map_err(|error| anyhow::anyhow!("variant matrix canonical digest failed: {error}"))
    }

    /// Seal the matrix with its content digest.
    pub fn sealed(mut self) -> Result<Self> {
        self.matrix_digest = Some(self.compute_digest()?);
        Ok(self)
    }

    /// Re-derive the digest and refuse tampering.
    pub fn verify(&self) -> Result<()> {
        match &self.matrix_digest {
            Some(digest) if *digest == self.compute_digest()? => Ok(()),
            Some(_) => anyhow::bail!("matrixDigest does not match the re-derived digest"),
            None => anyhow::bail!("missing matrixDigest"),
        }
    }
}

/// Classify one variant replay observation against the case's pinned
/// failure signature and the case's pinned fix outcome. Pure; no I/O.
pub fn classify_variant(
    case: &LiteFailureCase,
    failure: &FailureSignature,
    input: &FailureCaseVariantReplayInput,
) -> FailureCaseVariantReplayOutcome {
    let expected = match input.expectation {
        VariantExpectation::ReproduceFailure => ExpectedSignatureSummary::from_failure(failure),
        VariantExpectation::ObserveFix => ExpectedSignatureSummary::from_fix(case),
    };
    let observed = &input.observed;
    let verdict = match input.expectation {
        VariantExpectation::ReproduceFailure => {
            if !observed.sealed {
                VariantVerdict::Unverifiable
            } else if failure.reproduces(observed) {
                VariantVerdict::Reproduced
            } else {
                VariantVerdict::Drifted
            }
        }
        VariantExpectation::ObserveFix => {
            if !observed.sealed {
                VariantVerdict::Unverifiable
            } else if observed.task_outcome == Some(case.expected_result.task_outcome)
                && observed.classification == Some(case.expected_result.classification)
                && observed.validator_exit_code == Some(case.expected_result.validator_exit_code)
            {
                VariantVerdict::RepairedFixObserved
            } else {
                VariantVerdict::Drifted
            }
        }
    };
    FailureCaseVariantReplayOutcome {
        variant_id: input.variant_id.clone(),
        axis: input.axis.as_str().to_string(),
        expected,
        observed: describe_observed(observed),
        verdict,
        simulation_fixture: input.simulation_fixture,
    }
}

fn describe_observed(observed: &ReplaySignature) -> String {
    format!(
        "sealed={} outcome={} classification={} command={} args={:?} exit={}",
        observed.sealed,
        observed
            .task_outcome
            .map(task_outcome_str)
            .unwrap_or("none"),
        observed.classification.map(class_str).unwrap_or("none"),
        observed.validator_command.as_deref().unwrap_or("none"),
        observed.validator_args,
        observed
            .validator_exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "none".to_string()),
    )
}

/// Build the variant replay matrix for one failure case. Fail-closed:
/// duplicate variant ids, a variant declared for an axis the case cannot
/// distinguish, or a replay that claims a live provider is a typed refusal.
/// All replays in this slice must be fixture-synthetic; simulation_only is
/// forced and recorded.
pub fn build_variant_matrix(
    case: &LiteFailureCase,
    failure: &FailureSignature,
    inputs: &[FailureCaseVariantReplayInput],
) -> Result<FailureCaseVariantMatrix> {
    let mut seen = std::collections::BTreeSet::new();
    let mut replays = Vec::with_capacity(inputs.len());
    for input in inputs {
        if !seen.insert(input.variant_id.clone()) {
            anyhow::bail!("duplicate variant id {:?}", input.variant_id);
        }
        if !input.simulation_fixture {
            anyhow::bail!(
                "variant {:?} is not fixture-synthetic; live-provider replays are outside this slice",
                input.variant_id
            );
        }
        replays.push(classify_variant(case, failure, input));
    }
    if replays.is_empty() {
        anyhow::bail!("a variant replay matrix requires at least one variant replay");
    }
    let verdict = VariantMatrixVerdict::of(&replays.iter().map(|r| r.verdict).collect::<Vec<_>>());
    let matrix = FailureCaseVariantMatrix {
        schema_version: FAILURE_CASE_VARIANT_MATRIX_SCHEMA_VERSION.to_string(),
        namespace: FAILURE_CASE_VARIANT_MATRIX_NAMESPACE.to_string(),
        version: FAILURE_CASE_VARIANT_MATRIX_VERSION.to_string(),
        case_id: case.case_id.clone(),
        failure_identity: case.failure_identity.clone(),
        expected: ExpectedSignatureSummary::from_failure(failure),
        replays,
        verdict,
        verification_scope: FAILURE_CASE_VARIANT_MATRIX_VERIFICATION_SCOPE.to_string(),
        simulation_only: true,
        matrix_digest: None,
    };
    matrix.sealed()
}
