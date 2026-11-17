//! Lite-owned governed-workflow evaluation fixtures (issue #136, Slice A).
//!
//! This module owns the **versioned Lite-internal scenario descriptor** and the
//! **Lite-internal evidence envelope** used by the repeatable fixture scenarios
//! under `tests/fixtures/scenarios/`. It is deliberately **not** a Foundry
//! contract:
//!
//! - Lite owns executable product scenarios, fixture generation and **raw
//!   evidence**. Foundry owns the external benchmark runner, matrix
//!   aggregation, metrics, comparisons, certification and the public-claim
//!   policy. Nothing in this module is labelled as, or implied to be, a
//!   Foundry artifact, and no benchmark score, pass rate, comparison or
//!   certification is ever produced here.
//! - The [`ExternalEvaluationFormat`] stub on every exported record is typed
//!   `unsupported` on purpose. The Foundry external evaluation format does not
//!   exist yet; until it is published and pinned by Foundry, Lite exports raw
//!   JSON records only. That keeps local fixture development unblocked while
//!   preventing any false claim of Foundry integration.
//!
//! Scope of Slice A: ONE complete Rust fixture scenario and ONE Python fixture
//! scenario, executed deterministically through the existing governed
//! route-dispatch boundary with fixture-only local effects (a loopback model
//! fixture; no network beyond loopback, no paid provider, no fixture
//! toolchain execution). Broken or ambiguous tasks are **classified**, never
//! counted as product/model failure:
//!
//! - [`ScenarioOutcomeClass::SetupError`] — deterministic setup defect:
//!   malformed descriptor, unsupported version/language, fixture
//!   content-digest drift, platform path leakage, missing/ambiguous pins,
//!   pre-effect route/workspace/authority refusal. Scenario defect; tells us
//!   nothing about the model or the product.
//! - [`ScenarioOutcomeClass::InvalidTask`] — the task is broken or
//!   self-contradictory under the pinned authority/workspace (for example a
//!   task statement with no actionable content, or authority pins that
//!   contradict each other). Classified, never counted as model failure.
//! - [`ScenarioOutcomeClass::InfrastructureError`] — the harness/runtime
//!   pipeline itself failed after a governed dispatch (for example accounting
//!   failed to seal). A tooling defect, not a product outcome.
//! - [`ScenarioOutcomeClass::GovernedProductOutcome`] — the governed boundary
//!   ran and produced (or refused without effect, then a sealed attempt)
//!   evidence. A failure inside the governed effect is still a product
//!   outcome and is accounted as such; it is not a setup error.
//!
//! Verification boundary of the run envelope: every record the envelope
//! embeds is re-verified from the embedded bytes alone, and every
//! cross-object binding between them is re-checked (descriptor ↔ route
//! evidence ↔ snapshot ↔ manifest ↔ attempt ↔ accounting ↔ graph, plus the
//! execution workspace manifest). Nothing is hex-checked-and-trusted: any
//! digest present in the envelope is re-derived from the embedded bytes.
//! What the envelope does NOT prove is enumerated on
//! [`LiteScenarioRunEvidence`] itself (fixture bytes, latency provenance,
//! provider-reported usage truthfulness); the Foundry external format stays
//! UNSUPPORTED.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::path::{Path, PathBuf};

use crate::workflow::accounting::{AttemptAccountingV1, GraphAccounting};
use crate::workflow::route_dispatch::RoutedDispatchFailure;
use crate::workflow::route_resolution::is_hex64;
use crate::workflow::soma;

/// Schema version of the scenario descriptor document itself.
pub const SCENARIO_SCHEMA_VERSION: &str = "1.0.0";
/// Namespace every scenario descriptor must declare. Lite-internal.
pub const SCENARIO_NAMESPACE: &str = "lite.fixtures.scenario";
/// Schema version of the Lite-internal scenario-run evidence envelope.
pub const SCENARIO_RUN_SCHEMA_VERSION: &str = "1.0.0";
/// Namespace of the Lite-internal scenario-run evidence envelope. Lite-internal;
/// explicitly NOT a Foundry namespace.
pub const SCENARIO_RUN_NAMESPACE: &str = "lite.evidence.scenario-run";
/// Identifier of the local fixture runner that executes scenarios.
pub const SCENARIO_RUNNER_ID: &str = "lite-scenario-fixture-runner";
/// Revision of the local fixture runner. Bump on any behavioural change.
pub const SCENARIO_RUNNER_REVISION: &str = "1";

/// Expected-evidence artifact names a descriptor may pin. Anything else is a
/// setup defect (a pin that cannot be verified is not a pin).
pub const KNOWN_EVIDENCE_ARTIFACTS: &[&str] = &[
    "route-evidence",
    "snapshot",
    "attempt",
    "journal-entry",
    "accounting-record",
    "graph-accounting",
];

/// Dimensions a descriptor may declare as controlled or variable. Unknown
/// dimension names are a setup defect; the controlled/variable split is what
/// lets Foundry later attribute deltas to model, harness, workspace, policy,
/// context planning, provider or governance.
pub const ALLOWED_DIMENSIONS: &[&str] = &[
    "repository-content",
    "task-statement",
    "policy-profile",
    "authority",
    "workspace",
    "harness",
    "harness-revision",
    "route",
    "provider",
    "model",
    "evaluation-instant",
    "context-rules",
    "expected-evidence",
    "validation",
];

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

/// How a scenario run (or failure to run) is classified. Deterministic; never
/// a benchmark signal by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScenarioOutcomeClass {
    /// Deterministic setup defect: descriptor/fixture/pin/IO problem, or a
    /// pre-effect refusal. Not a product or model signal.
    SetupError,
    /// The task itself is broken or self-contradictory under the pinned
    /// authority/workspace. Not a product or model signal.
    InvalidTask,
    /// The harness/runtime pipeline failed after a governed dispatch.
    /// A tooling defect, not a product outcome.
    InfrastructureError,
    /// The governed boundary ran; the outcome is a product outcome
    /// (sealed attempt + accounting), whether it succeeded or failed.
    GovernedProductOutcome,
}

impl ScenarioOutcomeClass {
    pub fn as_str(self) -> &'static str {
        match self {
            ScenarioOutcomeClass::SetupError => "setup-error",
            ScenarioOutcomeClass::InvalidTask => "invalid-task",
            ScenarioOutcomeClass::InfrastructureError => "infrastructure-error",
            ScenarioOutcomeClass::GovernedProductOutcome => "governed-product-outcome",
        }
    }
}

/// One descriptor defect. `class` is either `SetupError` or `InvalidTask` —
/// never a product outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioDefect {
    pub class: ScenarioOutcomeClass,
    pub message: String,
}

impl ScenarioDefect {
    pub fn setup(message: impl Into<String>) -> Self {
        Self {
            class: ScenarioOutcomeClass::SetupError,
            message: message.into(),
        }
    }

    pub fn invalid_task(message: impl Into<String>) -> Self {
        Self {
            class: ScenarioOutcomeClass::InvalidTask,
            message: message.into(),
        }
    }
}

/// Parse/validation failure of a scenario descriptor. Carries every defect
/// found so callers can classify setup vs invalid-task without re-parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioDescriptorError(pub Vec<ScenarioDefect>);

impl fmt::Display for ScenarioDescriptorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "scenario descriptor rejected: ")?;
        for (index, defect) in self.0.iter().enumerate() {
            if index > 0 {
                write!(f, "; ")?;
            }
            write!(f, "{}: {}", defect.class.as_str(), defect.message)?;
        }
        Ok(())
    }
}

impl std::error::Error for ScenarioDescriptorError {}

impl ScenarioDescriptorError {
    /// Worst-case classification: `InvalidTask` if any defect is an
    /// invalid-task defect, otherwise `SetupError`.
    pub fn class(&self) -> ScenarioOutcomeClass {
        if self
            .0
            .iter()
            .any(|d| d.class == ScenarioOutcomeClass::InvalidTask)
        {
            ScenarioOutcomeClass::InvalidTask
        } else {
            ScenarioOutcomeClass::SetupError
        }
    }
}

/// Classify a pre-effect refusal (route resolution or dispatch refused before
/// any effect ran). A broken scenario/environment is never a model failure.
pub fn classify_pre_effect_refusal() -> ScenarioOutcomeClass {
    ScenarioOutcomeClass::SetupError
}

/// Classify the result of one governed dispatch.
pub fn classify_dispatch(
    result: &Result<crate::workflow::route_dispatch::RoutedDispatchOutcome, RoutedDispatchFailure>,
) -> ScenarioOutcomeClass {
    match result {
        Ok(_) => ScenarioOutcomeClass::GovernedProductOutcome,
        Err(RoutedDispatchFailure::Refused(_)) => classify_pre_effect_refusal(),
        // The effect ran and failed in a typed, sealed way. That is a governed
        // product outcome, not a setup error.
        Err(RoutedDispatchFailure::ExecutionFailed(_)) => {
            ScenarioOutcomeClass::GovernedProductOutcome
        }
    }
}

/// Classify an accounting failure after a dispatch. The dispatch happened;
/// the accounting pipeline (local, deterministic) is what failed.
pub fn classify_accounting_failure() -> ScenarioOutcomeClass {
    ScenarioOutcomeClass::InfrastructureError
}

// ---------------------------------------------------------------------------
// Descriptor document
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScenarioKind {
    Bugfix,
    Feature,
    Test,
    Documentation,
    Refactor,
    CheckpointContinuation,
}

/// Languages Slice A supports. Anything else fails closed at validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScenarioLanguage {
    Rust,
    Python,
}

/// The expected observable outcome class. Slice A supports exactly one:
/// the governed path records a sealed, accounted attempt. No benchmark
/// outcome vocabulary exists here on purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExpectedOutcome {
    AttemptRecordedAndAccounted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScenarioWorkspaceAdapter {
    ExistingReadOnly,
    GitWorktree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScenarioWorkspaceMode {
    ReadOnly,
    Writable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskPin {
    /// The task statement exactly as the subject will see it.
    pub statement: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepositoryPin {
    /// Fixture path relative to the repository root, forward slashes only.
    /// Never an absolute path, never a user-machine path.
    pub fixture: String,
    /// sha256 over the pinned fixture content (see [`fixture_content_digest`]).
    pub content_digest: String,
    /// Logical revision of the fixture content (not a git commit; the fixture
    /// is content-pinned, not history-pinned).
    pub revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExpectedPin {
    pub outcome: ExpectedOutcome,
    /// Human-readable statement of the expected observable outcome.
    pub observable_outcome: String,
    /// Evidence artifacts the run must seal. Subset of
    /// [`KNOWN_EVIDENCE_ARTIFACTS`]; unknown names are a setup defect.
    pub evidence_artifacts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequirementsPin {
    pub degradation_permitted: bool,
    pub forbid_substitution: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PolicyPin {
    /// Route profile name; must be `standard` in Slice A.
    pub profile: String,
    pub execution_class: String,
    pub mutation: String,
    pub readable_scopes: Vec<String>,
    pub writable_scopes: Vec<String>,
    pub network_policy_default: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspacePin {
    pub adapter: ScenarioWorkspaceAdapter,
    pub adapter_revision: String,
    pub mode: ScenarioWorkspaceMode,
    pub writable_scopes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HarnessPin {
    pub id: String,
    pub revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoutePin {
    pub route_set_id: String,
    pub route_set_version: String,
    pub candidate_id: String,
    pub provider_identity: String,
    pub provider_locality: String,
    pub model: String,
    pub priority: u32,
    pub profile: String,
    /// Injected evaluation instant (RFC 3339 UTC-Z). Pinned, never wall-clock.
    pub evaluated_at: String,
    /// Injected dispatch instant (RFC 3339 UTC-Z). Pinned, never wall-clock.
    pub dispatch_at: String,
    pub trust_verified_at: String,
    pub trust_ttl_seconds: u64,
    /// The pin is mandatory: an unpinned route set is ambiguous by definition.
    pub pinned_candidate_id: String,
}

/// Versioned pins for the validator that "success" means for a task.
/// Embedded on every executed-task descriptor; absent on read-only
/// descriptors. Validated fail-closed when present. Task-run sealing
/// REQUIRES these pins: a repaired claim is only meaningful against a
/// pinned expected assertion and a pinned command/argv. The pins are
/// immutable control data, not operator input — a scenario that does not
/// pin its validator cannot produce task-run success evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidationPin {
    /// Document version of this section. Supported: "1.0.0".
    pub schema_version: String,
    /// Exact validator executable, e.g. `cargo`.
    pub command: String,
    /// Exact validator arguments, e.g. `["test", "--offline"]`.
    pub args: Vec<String>,
    /// The only exit code that means "the task assertion holds".
    pub expected_exit_code: i32,
    /// Human statement of the expected task assertion (the fixture's
    /// definition of done).
    pub expected_assertion: String,
    /// sha256 of the post-repair file the validator validated
    /// (portable: single file, no toolchain artifacts). A forged repair
    /// record with different bytes cannot satisfy this pin.
    pub expected_file_after_digest: String,
    /// sha256 tree digest of the post-run workspace (via
    /// [`task_run_tree_digest`]; includes validator-created files such as
    /// `Cargo.lock`, so it is toolchain-sensitive — the descriptor pins
    /// fail closed on drift on purpose).
    pub expected_post_run_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DimensionsPin {
    /// Dimensions held constant by this scenario.
    pub controlled: Vec<String>,
    /// Dimensions this scenario deliberately varies. Empty in Slice A:
    /// every committed fixture is fully controlled.
    pub variable: Vec<String>,
}

/// The versioned Lite-internal scenario descriptor. Parse with
/// [`ScenarioDescriptor::parse_json`]; never construct and skip validation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScenarioDescriptor {
    pub schema_version: String,
    pub namespace: String,
    pub scenario_id: String,
    /// Descriptor document version (independent of schema_version).
    pub version: String,
    pub title: String,
    pub kind: ScenarioKind,
    pub language: ScenarioLanguage,
    pub task: TaskPin,
    pub repository: RepositoryPin,
    pub expected: ExpectedPin,
    pub requirements: RequirementsPin,
    pub policy: PolicyPin,
    pub workspace: WorkspacePin,
    pub harness: HarnessPin,
    pub route: RoutePin,
    pub dimensions: DimensionsPin,
    /// Optional, required for executed-task (task-run) sealing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ValidationPin>,
}

impl ScenarioDescriptor {
    /// Fail-closed parse + validation. Every defect found is returned.
    pub fn parse_json(text: &str) -> Result<Self, ScenarioDescriptorError> {
        let descriptor: Self = match serde_json::from_str(text) {
            Ok(descriptor) => descriptor,
            Err(error) => {
                return Err(ScenarioDescriptorError(vec![ScenarioDefect::setup(
                    format!("descriptor is not a valid {SCENARIO_NAMESPACE} document: {error}"),
                )]));
            }
        };
        let defects = descriptor.validate();
        if !defects.is_empty() {
            return Err(ScenarioDescriptorError(defects));
        }
        Ok(descriptor)
    }

    /// Collect all defects. Empty means the descriptor is valid.
    pub fn validate(&self) -> Vec<ScenarioDefect> {
        let mut defects = Vec::new();

        if self.schema_version != SCENARIO_SCHEMA_VERSION {
            defects.push(ScenarioDefect::setup(format!(
                "unsupported schemaVersion {:?} (supported {SCENARIO_SCHEMA_VERSION})",
                self.schema_version
            )));
        }
        if self.namespace != SCENARIO_NAMESPACE {
            defects.push(ScenarioDefect::setup(format!(
                "namespace {:?} is not {SCENARIO_NAMESPACE:?}",
                self.namespace
            )));
        }
        if self.scenario_id.trim().is_empty() {
            defects.push(ScenarioDefect::setup(
                "scenarioId must be non-empty".to_string(),
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

        // Task pins: an empty or whitespace task statement is an ambiguous
        // task, not a product signal.
        if self.task.statement.trim().len() < 12 {
            defects.push(ScenarioDefect::invalid_task(
                "task.statement is empty or too short to be actionable",
            ));
        }

        // Repository pins.
        if let Err(message) = check_fixture_path(&self.repository.fixture) {
            defects.push(ScenarioDefect::setup(message));
        }
        if !is_hex64(&self.repository.content_digest) {
            defects.push(ScenarioDefect::setup(
                "repository.contentDigest must be a lower hex sha256 digest".to_string(),
            ));
        }
        if self.repository.revision.trim().is_empty() {
            defects.push(ScenarioDefect::setup(
                "repository.revision must be non-empty".to_string(),
            ));
        }

        // Expected outcome pins.
        if self.expected.observable_outcome.trim().is_empty() {
            defects.push(ScenarioDefect::setup(
                "expected.observableOutcome must state the expected outcome".to_string(),
            ));
        }
        if self.expected.evidence_artifacts.is_empty() {
            defects.push(ScenarioDefect::setup(
                "expected.evidenceArtifacts must pin at least one artifact".to_string(),
            ));
        }
        for artifact in &self.expected.evidence_artifacts {
            if !KNOWN_EVIDENCE_ARTIFACTS.contains(&artifact.as_str()) {
                defects.push(ScenarioDefect::setup(format!(
                    "expected.evidenceArtifacts contains unknown artifact {artifact:?}"
                )));
            }
        }

        // Authority coherence: a read-only workspace cannot hand out writable
        // authority; pins that contradict each other describe no executable
        // task.
        if self.workspace.mode == ScenarioWorkspaceMode::ReadOnly
            && (!self.workspace.writable_scopes.is_empty()
                || !self.policy.writable_scopes.is_empty())
        {
            defects.push(ScenarioDefect::invalid_task(
                "read-only workspace pin contradicts declared writable scopes".to_string(),
            ));
        }
        if self.workspace.mode == ScenarioWorkspaceMode::Writable
            && self.workspace.writable_scopes.is_empty()
        {
            defects.push(ScenarioDefect::invalid_task(
                "writable workspace mode requires at least one writable scope".to_string(),
            ));
        }

        // Policy / requirements pins.
        if self.policy.profile != "standard" {
            defects.push(ScenarioDefect::setup(format!(
                "policy.profile {:?} is not \"standard\"",
                self.policy.profile
            )));
        }
        if self.policy.execution_class != "deterministic" || self.policy.mutation != "none" {
            defects.push(ScenarioDefect::setup(
                "policy must pin executionClass \"deterministic\" and mutation \"none\""
                    .to_string(),
            ));
        }
        if self.requirements.degradation_permitted || !self.requirements.forbid_substitution {
            defects.push(ScenarioDefect::setup(
                "requirements must pin degradationPermitted=false and forbidSubstitution=true"
                    .to_string(),
            ));
        }

        // Validator pins (optional at parse; required for task-run sealing).
        if let Some(validation) = &self.validation {
            if validation.schema_version != "1.0.0" {
                defects.push(ScenarioDefect::setup(format!(
                    "validation.schemaVersion {:?} is not \"1.0.0\"",
                    validation.schema_version
                )));
            }
            if validation.command.trim().is_empty() {
                defects.push(ScenarioDefect::setup(
                    "validation.command must be non-empty".to_string(),
                ));
            }
            if validation
                .command
                .chars()
                .any(|c| matches!(c, '/' | '\\' | '\0'))
            {
                defects.push(ScenarioDefect::setup(
                    "validation.command must be a bare executable name, not a path".to_string(),
                ));
            }
            if validation.args.is_empty() {
                defects.push(ScenarioDefect::setup(
                    "validation.args must pin at least one argument".to_string(),
                ));
            }
            for arg in &validation.args {
                if arg.trim().is_empty() {
                    defects.push(ScenarioDefect::setup(
                        "validation.args entries must be non-empty".to_string(),
                    ));
                }
            }
            if validation.expected_assertion.trim().len() < 8 {
                defects.push(ScenarioDefect::setup(
                    "validation.expectedAssertion must state the expected assertion".to_string(),
                ));
            }
            if !is_hex64(&validation.expected_file_after_digest) {
                defects.push(ScenarioDefect::setup(
                    "validation.expectedFileAfterDigest must be a lower hex sha256 digest"
                        .to_string(),
                ));
            }
            if !is_hex64(&validation.expected_post_run_digest) {
                defects.push(ScenarioDefect::setup(
                    "validation.expectedPostRunDigest must be a lower hex sha256 digest"
                        .to_string(),
                ));
            }
        }

        // Harness pins.
        if self.harness.id.trim().is_empty() || self.harness.revision.trim().is_empty() {
            defects.push(ScenarioDefect::setup(
                "harness.id and harness.revision must be non-empty".to_string(),
            ));
        }

        // Route pins: full pin or nothing. An unpinned candidate is ambiguous.
        if self.route.pinned_candidate_id != self.route.candidate_id {
            defects.push(ScenarioDefect::setup(
                "route.pinnedCandidateId must equal route.candidateId (route sets are pinned)"
                    .to_string(),
            ));
        }
        if self.route.profile != self.policy.profile {
            defects.push(ScenarioDefect::invalid_task(
                "route.profile and policy.profile disagree".to_string(),
            ));
        }
        for (label, instant) in [
            ("route.evaluatedAt", &self.route.evaluated_at),
            ("route.dispatchAt", &self.route.dispatch_at),
            ("route.trustVerifiedAt", &self.route.trust_verified_at),
        ] {
            if !is_rfc3339_z(instant) {
                defects.push(ScenarioDefect::setup(format!(
                    "{label} {instant:?} is not a genuine RFC 3339 UTC-Z timestamp"
                )));
            }
        }
        if self.route.trust_ttl_seconds == 0 {
            defects.push(ScenarioDefect::setup(
                "route.trustTtlSeconds must be at least 1".to_string(),
            ));
        }
        for (label, value) in [
            ("route.routeSetId", &self.route.route_set_id),
            ("route.routeSetVersion", &self.route.route_set_version),
            ("route.providerIdentity", &self.route.provider_identity),
            ("route.providerLocality", &self.route.provider_locality),
            ("route.model", &self.route.model),
        ] {
            if value.trim().is_empty() {
                defects.push(ScenarioDefect::setup(format!("{label} must be non-empty")));
            }
        }

        // Dimension pins.
        for dimension in self
            .dimensions
            .controlled
            .iter()
            .chain(&self.dimensions.variable)
        {
            if !ALLOWED_DIMENSIONS.contains(&dimension.as_str()) {
                defects.push(ScenarioDefect::setup(format!(
                    "dimensions contains unknown dimension {dimension:?}"
                )));
            }
        }
        for dimension in &self.dimensions.variable {
            if self.dimensions.controlled.contains(dimension) {
                defects.push(ScenarioDefect::setup(format!(
                    "dimension {dimension:?} cannot be both controlled and variable"
                )));
            }
        }

        defects
    }

    /// Canonical digest of the descriptor document. Deterministic across
    /// platforms; used as `scenarioDigest` in run evidence.
    pub fn scenario_digest(&self) -> anyhow::Result<String> {
        let value = serde_json::to_value(self).context("descriptor serializes")?;
        soma::try_canonical_digest(&value)
            .map_err(|error| anyhow::anyhow!("descriptor canonical digest failed: {error}"))
    }
}

fn is_rfc3339_z(value: &str) -> bool {
    if value.len() != 20
        || !value.ends_with('Z')
        || value.as_bytes()[4] != b'-'
        || value.as_bytes()[7] != b'-'
        || value.as_bytes()[10] != b'T'
        || value.as_bytes()[13] != b':'
        || value.as_bytes()[16] != b':'
    {
        return false;
    }
    value[..4].chars().all(|c| c.is_ascii_digit())
        && value[5..7].chars().all(|c| c.is_ascii_digit())
        && value[8..10].chars().all(|c| c.is_ascii_digit())
        && value[11..13].chars().all(|c| c.is_ascii_digit())
        && value[14..16].chars().all(|c| c.is_ascii_digit())
        && value[17..19].chars().all(|c| c.is_ascii_digit())
}

/// Reject any fixture path that is absolute, rooted, drive-qualified,
/// backslash-separated or escapes the repository root.
pub(crate) fn check_fixture_path(path: &str) -> Result<(), String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("repository.fixture must be non-empty".to_string());
    }
    if trimmed.starts_with('/') || trimmed.starts_with('\\') {
        return Err(format!(
            "repository.fixture {trimmed:?} is absolute; pin a repo-relative path"
        ));
    }
    if trimmed.contains('\\') {
        return Err(format!(
            "repository.fixture {trimmed:?} contains backslashes; use forward slashes"
        ));
    }
    if trimmed.contains(':') {
        return Err(format!(
            "repository.fixture {trimmed:?} looks drive-qualified; pin a repo-relative path"
        ));
    }
    for segment in trimmed.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(format!(
                "repository.fixture {trimmed:?} has an empty, '.', or '..' segment"
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Fixture content pinning
// ---------------------------------------------------------------------------

/// Deterministic content digest of a fixture directory.
///
/// Algorithm: collect every regular file under `root`, express each path
/// relative to `root` with forward slashes, sort by UTF-8 bytes, then digest
/// the concatenation of `"{relpath} {hex-sha256-of-file}\n"` lines. The same
/// tree yields the same digest on every platform; a single byte of fixture
/// drift changes the digest.
///
/// Fail-closed on pinned fixtures: symbolic links and any file type other
/// than a regular file or directory (fifos, sockets, devices, unknown mounts)
/// are REJECTED, never silently omitted — an omitted entry could change the
/// meaning of a pinned fixture without changing its digest.
pub fn fixture_content_digest(root: &Path) -> Result<String> {
    let root_type = std::fs::symlink_metadata(root)
        .with_context(|| format!("fixture root {root:?} does not exist"))?;
    anyhow::ensure!(
        root_type.is_dir(),
        "fixture root {root:?} is not a directory (symlink?)"
    );
    let mut entries: Vec<(String, PathBuf)> = Vec::new();
    collect_files(root, root, &mut entries)?;
    hash_fixture_entries(&mut entries)
}

/// Deterministic digest of a task-run workspace source tree, like
/// [`fixture_content_digest`] but skipping a **top-level `.git` entry**.
///
/// A git worktree's `.git` is a regular file whose content embeds the
/// absolute path of the source repository's worktree metadata directory, so
/// it is machine-dependent and must not participate in a content pin. The
/// git object store itself lives in the SOURCE repository, never in the
/// worktree, so skipping the pointer file loses no source bytes. Only the
/// top-level entry is skipped; a nested `.git` is rejected like any other
/// unexpected file type. Fail-closed on everything else.
pub fn task_run_tree_digest(root: &Path) -> Result<String> {
    let root_type = std::fs::symlink_metadata(root)
        .with_context(|| format!("task root {root:?} does not exist"))?;
    anyhow::ensure!(
        root_type.is_dir(),
        "task root {root:?} is not a directory (symlink?)"
    );
    let mut entries: Vec<(String, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(root).with_context(|| format!("task dir {root:?} unreadable"))? {
        let entry = entry?;
        if entry.file_name() == ".git" {
            continue;
        }
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            anyhow::bail!(
                "pinned fixture contains a symbolic link: {} (unexpected link; refused)",
                path.display()
            );
        }
        if file_type.is_dir() {
            collect_files(root, &path, &mut entries)?;
        } else if file_type.is_file() {
            let rel = path
                .strip_prefix(root)
                .with_context(|| format!("fixture path {path:?} escapes root {root:?}"))?;
            let rel = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            entries.push((rel, path));
        } else {
            anyhow::bail!(
                "pinned fixture contains an unsupported file type: {} (only regular files and directories are pinned)",
                path.display()
            );
        }
    }
    hash_fixture_entries(&mut entries)
}

fn hash_fixture_entries(entries: &mut [(String, PathBuf)]) -> Result<String> {
    entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut material = String::new();
    for (rel, full) in entries.iter() {
        let bytes =
            std::fs::read(full).with_context(|| format!("fixture file {full:?} unreadable"))?;
        let file_digest = hex_lower(&Sha256::digest(&bytes));
        material.push_str(rel);
        material.push(' ');
        material.push_str(&file_digest);
        material.push('\n');
    }
    Ok(hex_lower(&Sha256::digest(material.as_bytes())))
}

/// Verify a fixture directory against its pinned content digest.
pub fn verify_fixture_content(root: &Path, expected: &str) -> Result<()> {
    if !is_hex64(expected) {
        anyhow::bail!("pinned repository.contentDigest is not a lower hex sha256 digest");
    }
    let actual = fixture_content_digest(root)?;
    anyhow::ensure!(
        actual == expected,
        "fixture content digest drift: pinned {expected}, on-disk {actual}"
    );
    Ok(())
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<()> {
    for entry in
        std::fs::read_dir(dir).with_context(|| format!("fixture dir {dir:?} unreadable"))?
    {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            anyhow::bail!(
                "pinned fixture contains a symbolic link: {} (unexpected link; refused)",
                path.display()
            );
        }
        if file_type.is_dir() {
            collect_files(root, &path, out)?;
        } else if file_type.is_file() {
            let rel = path
                .strip_prefix(root)
                .with_context(|| format!("fixture path {path:?} escapes root {root:?}"))?;
            let rel = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            out.push((rel, path));
        } else {
            anyhow::bail!(
                "pinned fixture contains an unsupported file type: {} (only regular files and directories are pinned)",
                path.display()
            );
        }
    }
    Ok(())
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

// ---------------------------------------------------------------------------
// Lite-internal run evidence envelope
// ---------------------------------------------------------------------------

use crate::workflow::accounting::ACCOUNTING_SCHEMA_VERSION;
use crate::workflow::node_contracts::NodeManifestV1;
use crate::workflow::route_dispatch::RoutedAttemptV1;
use crate::workflow::route_dispatch::RoutedDispatchOutcome;
use crate::workflow::route_resolution::{
    EffectiveExecutionSnapshotV1, RouteEvidenceV1, RouteResolution,
};
use crate::workflow::workspace::{WorkspaceManifestV1, WorkspaceMode};

/// Explicit, typed "we do not ship the Foundry external format yet" marker.
/// There is no public constructor that flips the status; every exported
/// record carries `unsupported` and [`LiteScenarioRunEvidence::verify`]
/// refuses anything else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExternalEvaluationFormat {
    /// Always `"unsupported"` in Slice A.
    pub status: String,
    /// The contract this would bind to, named precisely, not sketched.
    pub contract: String,
    /// Why it is unsupported.
    pub detail: String,
}

impl ExternalEvaluationFormat {
    pub fn unsupported() -> Self {
        Self {
            status: "unsupported".to_string(),
            contract: "foundry.workflow-evaluation (unpublished)".to_string(),
            detail: "Lite exports raw evidence records only; the Foundry external evaluation format is unpublished. No Foundry consumption, metrics or comparisons are claimed.".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunProvenance {
    pub runner: String,
    pub runner_revision: String,
    pub injected_clock: bool,
    pub fixture_only: bool,
    pub redaction: String,
}

/// The Lite-internal scenario-run evidence envelope. **Every record the
/// envelope references is embedded in it**, so [`LiteScenarioRunEvidence::verify`]
/// re-verifies each record's own sealed digest AND every cross-object binding
/// from the embedded bytes alone — no digest in the envelope is taken on
/// trust, and nothing is labelled as verified that is not re-derived. NEVER a
/// Foundry contract; see module docs.
///
/// What the envelope does NOT prove (honesty boundary, not a defect):
/// - the fixture directory's bytes (bound by `repository.contentDigest`,
///   enforced at pin time by [`verify_fixture_content`]; not re-hashed here);
/// - the wall-clock provenance of the measured dispatch latency;
/// - the truthfulness of the (loopback) provider's reported usage numbers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiteScenarioRunEvidence {
    pub schema_version: String,
    pub namespace: String,
    pub evaluated_at: String,
    /// The embedded, validated scenario descriptor. `scenario_id` and
    /// `scenario_digest` are DERIVED from it, never stored separately.
    pub descriptor: ScenarioDescriptor,
    /// Raw sealed route evidence (resolution decision; self-verifies).
    pub route_evidence: RouteEvidenceV1,
    /// Raw sealed effective execution snapshot (self-verifies, incl. the
    /// SPEC 006 ExecutionProfile audit).
    pub snapshot: EffectiveExecutionSnapshotV1,
    /// Raw node manifest under the reviewed permit.
    pub manifest: NodeManifestV1,
    /// Raw sealed routed attempt (self-verifies, incl. journal binding).
    pub attempt: RoutedAttemptV1,
    /// The execution workspace manifest, when one was pinned and provided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_manifest: Option<WorkspaceManifestV1>,
    /// The raw, sealed attempt-accounting record produced natively by Lite.
    pub attempt_accounting: AttemptAccountingV1,
    /// The raw, aggregated graph accounting produced natively by Lite.
    /// Derived; [`LiteScenarioRunEvidence::verify`] re-derives it from the
    /// embedded accounting records and requires equality.
    pub graph_accounting: GraphAccounting,
    pub classification: ScenarioOutcomeClass,
    pub external_evaluation_format: ExternalEvaluationFormat,
    pub provenance: RunProvenance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
}

impl LiteScenarioRunEvidence {
    /// Canonical digest of the envelope minus `contentDigest`.
    pub fn compute_digest(&self) -> Result<String> {
        let mut value = serde_json::to_value(self).context("scenario-run evidence serializes")?;
        if let Some(obj) = value.as_object_mut() {
            obj.remove("contentDigest");
        }
        soma::try_canonical_digest(&value).map_err(|error| {
            anyhow::anyhow!("scenario-run evidence canonical digest failed: {error}")
        })
    }

    /// Scenario identity, derived from the embedded descriptor.
    pub fn scenario_id(&self) -> &str {
        &self.descriptor.scenario_id
    }

    /// Canonical digest of the embedded descriptor.
    pub fn scenario_digest(&self) -> Result<String> {
        self.descriptor.scenario_digest()
    }

    /// Build and seal an envelope from the records of one completed run.
    ///
    /// Every cross-object binding is checked BEFORE the envelope is sealed;
    /// on any mismatch this returns the collected discrepancies and nothing is
    /// exported. The caller passes the run's actual records; nothing here is
    /// caller-asserted into place.
    #[allow(clippy::too_many_arguments)]
    pub fn sealed(
        descriptor: ScenarioDescriptor,
        resolution: &RouteResolution,
        manifest: &NodeManifestV1,
        outcome: &RoutedDispatchOutcome,
        attempt_accounting: AttemptAccountingV1,
        graph_accounting: GraphAccounting,
        workspace_manifest: Option<&WorkspaceManifestV1>,
        classification: ScenarioOutcomeClass,
    ) -> Result<Self> {
        let violations = check_cross_bindings(
            &descriptor,
            &resolution.evidence,
            &resolution.snapshot,
            manifest,
            &outcome.attempt,
            workspace_manifest,
            &attempt_accounting,
            &graph_accounting,
        );
        if !violations.is_empty() {
            anyhow::bail!(
                "scenario-run evidence refuses to seal; {} cross-object binding(s) failed: {}",
                violations.len(),
                violations.join("; ")
            );
        }
        let mut record = Self {
            schema_version: SCENARIO_RUN_SCHEMA_VERSION.to_string(),
            namespace: SCENARIO_RUN_NAMESPACE.to_string(),
            evaluated_at: resolution.evidence.evaluated_at.clone(),
            descriptor,
            route_evidence: resolution.evidence.clone(),
            snapshot: resolution.snapshot.clone(),
            manifest: manifest.clone(),
            attempt: outcome.attempt.clone(),
            workspace_manifest: workspace_manifest.cloned(),
            attempt_accounting,
            graph_accounting,
            classification,
            external_evaluation_format: ExternalEvaluationFormat::unsupported(),
            provenance: RunProvenance {
                runner: SCENARIO_RUNNER_ID.to_string(),
                runner_revision: SCENARIO_RUNNER_REVISION.to_string(),
                injected_clock: true,
                fixture_only: true,
                redaction: "digest-sealed artifacts carry identities and digests only; raw secret material is never serialized (canary-verified in tests)".to_string(),
            },
            content_digest: None,
        };
        record.content_digest = Some(record.compute_digest()?);
        Ok(record)
    }

    /// Fail-closed re-verification from the embedded bytes alone.
    ///
    /// Verifies: document version/namespace; the envelope's own digest; each
    /// embedded record's own sealed digest (descriptor validation, route
    /// evidence, snapshot incl. the SPEC 006 audit, attempt incl. its journal
    /// entry, accounting record, workspace manifest); EVERY cross-object
    /// binding (descriptor ↔ repository/route/harness/provider identity,
    /// snapshot ↔ route evidence, attempt ↔ snapshot/manifest, attempt ↔
    /// accounting, accounting ↔ graph re-derived from embedded records,
    /// workspace manifest ↔ descriptor pins and snapshot). Nothing is
    /// hex-checked-and-trusted: every value is re-derived or cross-bound.
    pub fn verify(&self) -> anyhow::Result<()> {
        use anyhow::ensure;
        ensure!(
            self.schema_version == SCENARIO_RUN_SCHEMA_VERSION,
            "unsupported scenario-run schema version {} (supported {SCENARIO_RUN_SCHEMA_VERSION})",
            self.schema_version
        );
        ensure!(
            self.namespace == SCENARIO_RUN_NAMESPACE,
            "scenario-run namespace {:?} is not {SCENARIO_RUN_NAMESPACE:?}",
            self.namespace
        );
        match &self.content_digest {
            Some(actual) => {
                let expected = self.compute_digest()?;
                ensure!(
                    *actual == expected,
                    "scenario-run evidence contentDigest does not verify"
                );
            }
            None => anyhow::bail!("scenario-run evidence is missing its contentDigest"),
        }
        // The embedded descriptor must be coherent on its own.
        let defects = self.descriptor.validate();
        ensure!(
            defects.is_empty(),
            "embedded scenario descriptor is invalid: {}",
            defects
                .iter()
                .map(|defect| defect.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        );
        // Embedded records re-verify against their own sealed digests.
        self.route_evidence.verify()?;
        self.snapshot.verify()?;
        self.attempt.verify()?;
        self.attempt_accounting.verify()?;
        if let Some(workspace) = &self.workspace_manifest {
            let stored = workspace.content_digest.as_deref().ok_or_else(|| {
                anyhow::anyhow!("workspace manifest is missing its contentDigest")
            })?;
            let recomputed = workspace.compute_digest();
            ensure!(
                stored == recomputed,
                "workspace manifest contentDigest does not verify"
            );
        }
        // Cross-object bindings from the embedded bytes alone.
        let violations = check_cross_bindings(
            &self.descriptor,
            &self.route_evidence,
            &self.snapshot,
            &self.manifest,
            &self.attempt,
            self.workspace_manifest.as_ref(),
            &self.attempt_accounting,
            &self.graph_accounting,
        );
        ensure!(
            violations.is_empty(),
            "scenario-run evidence failed cross-object binding checks: {}",
            violations.join("; ")
        );
        ensure!(
            self.graph_accounting.schema_version == ACCOUNTING_SCHEMA_VERSION,
            "embedded graph accounting has an unsupported schema version"
        );
        // The external format stays unsupported. Fail closed otherwise.
        ensure!(
            self.external_evaluation_format.status == "unsupported",
            "externalEvaluationFormat.status must be \"unsupported\" in Slice A"
        );
        ensure!(
            self.provenance.fixture_only && self.provenance.injected_clock,
            "scenario-run evidence must be fixture-only with an injected clock"
        );
        Ok(())
    }
}

fn workspace_adapter_family(adapter: crate::workflow::workspace::AdapterKind) -> &'static str {
    match adapter {
        crate::workflow::workspace::AdapterKind::ExistingReadOnly => "existing-read-only",
        crate::workflow::workspace::AdapterKind::GitWorktree => "git-worktree",
    }
}

fn workspace_mode_name(mode: WorkspaceMode) -> &'static str {
    match mode {
        WorkspaceMode::ReadOnly => "read-only",
        WorkspaceMode::Writable => "writable",
    }
}

/// Mode spelling used by the route snapshot's WorkspaceEvidenceV1 (SPEC 006
/// mapping), which differs from the workspace manifest's own spelling.
fn snapshot_workspace_mode_name(mode: WorkspaceMode) -> &'static str {
    match mode {
        WorkspaceMode::ReadOnly => "read",
        WorkspaceMode::Writable => "write",
    }
}

/// Every cross-object binding required for the envelope to be trustworthy.
/// Returns the full list of violations (empty = bound). Shared by `sealed`
/// (refuses to export on violations) and `verify` (refuses to re-admit).
#[allow(clippy::too_many_arguments)]
fn check_cross_bindings(
    descriptor: &ScenarioDescriptor,
    route_evidence: &RouteEvidenceV1,
    snapshot: &EffectiveExecutionSnapshotV1,
    manifest: &NodeManifestV1,
    attempt: &RoutedAttemptV1,
    workspace_manifest: Option<&WorkspaceManifestV1>,
    accounting: &AttemptAccountingV1,
    graph: &GraphAccounting,
) -> Vec<String> {
    use crate::workflow::route_resolution::RouteProfile;
    let mut violations: Vec<String> = Vec::new();
    let mut fail = |message: String| violations.push(message);

    // ---- Scenario descriptor <-> route evidence -----------------------------
    if route_evidence.route_set_id != descriptor.route.route_set_id {
        fail(format!(
            "routeEvidence.routeSetId {:?} != descriptor.route.routeSetId {:?}",
            route_evidence.route_set_id, descriptor.route.route_set_id
        ));
    }
    if route_evidence.route_set_version != descriptor.route.route_set_version {
        fail("routeEvidence.routeSetVersion != descriptor.route.routeSetVersion".to_string());
    }
    if route_evidence.evaluated_at != descriptor.route.evaluated_at {
        fail("routeEvidence.evaluatedAt != descriptor.route.evaluatedAt".to_string());
    }
    if route_evidence.selected.candidate_id != descriptor.route.candidate_id {
        fail("routeEvidence.selected.candidateId != descriptor.route.candidateId".to_string());
    }
    if route_evidence.selected.provider_identity != descriptor.route.provider_identity {
        fail(
            "routeEvidence.selected.providerIdentity != descriptor.route.providerIdentity"
                .to_string(),
        );
    }
    if route_evidence.selected.model.as_deref() != Some(descriptor.route.model.as_str()) {
        fail("routeEvidence.selected.model != descriptor.route.model".to_string());
    }
    if route_evidence.selected.harness_id != descriptor.harness.id {
        fail("routeEvidence.selected.harnessId != descriptor.harness.id".to_string());
    }
    if route_evidence.selected.harness_revision != descriptor.harness.revision {
        fail("routeEvidence.selected.harnessRevision != descriptor.harness.revision".to_string());
    }
    if route_evidence.selected.priority != descriptor.route.priority {
        fail("routeEvidence.selected.priority != descriptor.route.priority".to_string());
    }
    if route_evidence.pinned_candidate_id.as_deref()
        != Some(descriptor.route.pinned_candidate_id.as_str())
    {
        fail("routeEvidence.pinnedCandidateId != descriptor.route.pinnedCandidateId".to_string());
    }
    if descriptor.route.pinned_candidate_id != descriptor.route.candidate_id {
        fail("descriptor.route.pinnedCandidateId != descriptor.route.candidateId".to_string());
    }
    if route_evidence.profile != RouteProfile::Standard || descriptor.route.profile != "standard" {
        fail("profile must be \"standard\" in the evidence and the descriptor pin".to_string());
    }

    // ---- Route evidence <-> snapshot ----------------------------------------
    if snapshot.route_set_id != route_evidence.route_set_id
        || snapshot.route_set_version != route_evidence.route_set_version
        || snapshot.evaluated_at != route_evidence.evaluated_at
    {
        fail("snapshot route set/version/instant does not match the route evidence".to_string());
    }
    if snapshot.selected != route_evidence.selected {
        fail("snapshot.selected != routeEvidence.selected".to_string());
    }
    if snapshot.decision_digest != route_evidence.decision_digest {
        fail("snapshot.decisionDigest != routeEvidence.decisionDigest".to_string());
    }
    if snapshot.requirements_digest != route_evidence.requirements_digest {
        fail("snapshot.requirementsDigest != routeEvidence.requirementsDigest".to_string());
    }
    if snapshot.capability_set_digest != route_evidence.capability_set_digest {
        fail("snapshot.capabilitySetDigest != routeEvidence.capabilitySetDigest".to_string());
    }
    if snapshot.authority_declared_digest != route_evidence.authority_declared_digest {
        fail(
            "snapshot.authorityDeclaredDigest != routeEvidence.authorityDeclaredDigest".to_string(),
        );
    }
    if snapshot.authority_effective_digest != route_evidence.authority_effective_digest {
        fail(
            "snapshot.authorityEffectiveDigest != routeEvidence.authorityEffectiveDigest"
                .to_string(),
        );
    }
    if route_evidence.snapshot_digest != snapshot.content_digest.clone().unwrap_or_default() {
        fail("routeEvidence.snapshotDigest does not bind the embedded snapshot".to_string());
    }

    // ---- Attempt <-> snapshot / route evidence / manifest --------------------
    if attempt.snapshot_digest != snapshot.content_digest.clone().unwrap_or_default() {
        fail("attempt.snapshotDigest does not bind the embedded snapshot".to_string());
    }
    if Some(attempt.route_evidence_digest.as_str()) != route_evidence.content_digest.as_deref() {
        fail("attempt.routeEvidenceDigest does not bind the embedded route evidence".to_string());
    }
    if attempt.decision_digest != route_evidence.decision_digest {
        fail("attempt.decisionDigest != routeEvidence.decisionDigest".to_string());
    }
    match serde_json::to_value(manifest).map_err(|error| error.to_string()) {
        Ok(manifest_value) => match soma::try_canonical_digest(&manifest_value) {
            Ok(manifest_digest) => {
                if attempt.manifest_digest != manifest_digest {
                    fail("attempt.manifestDigest does not bind the embedded manifest".to_string());
                }
            }
            Err(error) => fail(format!("manifest canonical digest failed: {error}")),
        },
        Err(error) => fail(format!("manifest does not serialize: {error}")),
    }
    if attempt.harness_id != snapshot.selected.harness_id {
        fail("attempt.harnessId != snapshot.selected.harnessId".to_string());
    }
    if attempt.provider_identity != snapshot.selected.provider_identity {
        fail("attempt.providerIdentity != snapshot.selected.providerIdentity".to_string());
    }
    if attempt.model != snapshot.selected.model {
        fail("attempt.model != snapshot.selected.model".to_string());
    }

    // ---- Workspace manifest <-> descriptor / snapshot / attempt ---------------
    match (workspace_manifest, &snapshot.workspace) {
        (Some(workspace), Some(ws_evidence)) => {
            let recomputed_manifest = workspace.compute_digest();
            if workspace.content_digest.as_deref() != Some(recomputed_manifest.as_str()) {
                fail("workspace manifest contentDigest does not verify".to_string());
            }
            let manifest_family = workspace_adapter_family(workspace.adapter);
            let pin_family = match descriptor.workspace.adapter {
                ScenarioWorkspaceAdapter::ExistingReadOnly => "existing-read-only",
                ScenarioWorkspaceAdapter::GitWorktree => "git-worktree",
            };
            if manifest_family != pin_family {
                fail("workspace manifest adapter does not match the descriptor pin".to_string());
            }
            if workspace.adapter_revision != descriptor.workspace.adapter_revision {
                fail(
                    "workspace manifest adapterRevision != descriptor.workspace.adapterRevision"
                        .to_string(),
                );
            }
            let manifest_mode = workspace_mode_name(workspace.mode);
            let pin_mode = match descriptor.workspace.mode {
                ScenarioWorkspaceMode::ReadOnly => "read-only",
                ScenarioWorkspaceMode::Writable => "writable",
            };
            if manifest_mode != pin_mode {
                fail("workspace manifest mode does not match the descriptor pin".to_string());
            }
            if workspace.base_revision != descriptor.repository.revision {
                fail(
                    "workspace manifest baseRevision != descriptor.repository.revision".to_string(),
                );
            }
            if workspace.writable_scopes != descriptor.workspace.writable_scopes {
                fail(
                    "workspace manifest writableScopes != descriptor.workspace.writableScopes"
                        .to_string(),
                );
            }
            if workspace.workspace_id != format!("scenario-workspace-{}", descriptor.scenario_id) {
                fail("workspace manifest workspaceId does not bind the scenario id".to_string());
            }
            if ws_evidence.workspace_id != workspace.workspace_id {
                fail(
                    "snapshot workspace evidence workspaceId != workspace manifest workspaceId"
                        .to_string(),
                );
            }
            if ws_evidence.adapter != workspace_adapter_family(workspace.adapter) {
                fail(
                    "snapshot workspace evidence adapter != workspace manifest adapter".to_string(),
                );
            }
            if ws_evidence.adapter_revision != workspace.adapter_revision {
                fail(
                    "snapshot workspace evidence adapterRevision != workspace manifest adapterRevision"
                        .to_string(),
                );
            }
            if ws_evidence.mode != snapshot_workspace_mode_name(workspace.mode) {
                fail("snapshot workspace evidence mode != workspace manifest mode".to_string());
            }
            if ws_evidence.manifest_digest != workspace.compute_digest() {
                fail(
                    "snapshot workspace evidence manifestDigest != workspace manifest digest"
                        .to_string(),
                );
            }
            if attempt.workspace_manifest_digest.as_deref()
                != Some(ws_evidence.manifest_digest.as_str())
            {
                fail(
                    "attempt.workspaceManifestDigest does not bind the snapshot workspace evidence"
                        .to_string(),
                );
            }
        }
        (None, None) => {
            if attempt.workspace_manifest_digest.is_some() {
                fail("attempt binds a workspace manifest but none is embedded".to_string());
            }
        }
        _ => fail(
            "workspace manifest presence does not match the snapshot workspace binding".to_string(),
        ),
    }

    // ---- Accounting <-> attempt / manifest / descriptor / graph --------------
    if accounting.outcome != crate::workflow::accounting::AttemptOutcomeAccounting::Completed {
        fail(
            "accounting outcome is not Completed; this export only seals completed runs"
                .to_string(),
        );
    } else {
        let attempt_digest = attempt.content_digest.clone().unwrap_or_default();
        if accounting.attempt_digest.as_deref() != Some(attempt_digest.as_str()) {
            fail("accounting.attemptDigest does not bind the embedded attempt".to_string());
        }
        if attempt.outcome_kind != "completed" {
            fail(
                "completed accounting outcome but the attempt outcomeKind is not \"completed\""
                    .to_string(),
            );
        }
    }
    if accounting.journal_digest.as_deref() != Some(attempt.journal_entry.entry_digest.as_str()) {
        fail("accounting.journalDigest does not bind the embedded journal entry".to_string());
    }
    if accounting.node_id != manifest.node_id {
        fail("accounting.nodeId != manifest.nodeId".to_string());
    }
    if accounting.idempotency_key != attempt.idempotency_key
        || attempt.idempotency_key != attempt.journal_entry.identity_key
    {
        fail("accounting/attempt/journal idempotency keys do not bind together".to_string());
    }
    if accounting.harness_id != attempt.harness_id {
        fail("accounting.harnessId != attempt.harnessId".to_string());
    }
    if accounting.harness_revision != descriptor.harness.revision {
        fail("accounting.harnessRevision != descriptor.harness.revision".to_string());
    }
    if accounting.provider_identity != attempt.provider_identity {
        fail("accounting.providerIdentity != attempt.providerIdentity".to_string());
    }
    if accounting.model != attempt.model {
        fail("accounting.model != attempt.model".to_string());
    }
    if accounting.route_set_id != route_evidence.route_set_id {
        fail("accounting.routeSetId != routeEvidence.routeSetId".to_string());
    }

    // ---- Graph <-> accounting (re-derived, never trusted) ---------------------
    if graph.schema_version != ACCOUNTING_SCHEMA_VERSION {
        fail("graph accounting schema version is not supported".to_string());
    }
    if graph.graph_id != accounting.graph_id {
        fail("graph.graphId != accounting.graphId".to_string());
    }
    let mut ledger = crate::workflow::accounting::AccountingLedger::new();
    match ledger.record(accounting.clone()) {
        Ok(_) => match ledger.aggregate_graph(&accounting.graph_id) {
            Ok(rebuilt) => {
                if rebuilt.attempts != graph.attempts {
                    fail(format!(
                        "graph.attempts {} != re-derived {}",
                        graph.attempts, rebuilt.attempts
                    ));
                }
                if rebuilt.completed != graph.completed {
                    fail(format!(
                        "graph.completed {} != re-derived {}",
                        graph.completed, rebuilt.completed
                    ));
                }
                if rebuilt.failed != graph.failed {
                    fail(format!(
                        "graph.failed {} != re-derived {}",
                        graph.failed, rebuilt.failed
                    ));
                }
                if rebuilt.cancelled != graph.cancelled {
                    fail(format!(
                        "graph.cancelled {} != re-derived {}",
                        graph.cancelled, rebuilt.cancelled
                    ));
                }
                if rebuilt.usage != graph.usage {
                    fail(
                        "graph.usage != re-derived usage from embedded accounting records"
                            .to_string(),
                    );
                }
                if rebuilt.cost != graph.cost {
                    fail(
                        "graph.cost != re-derived cost from embedded accounting records"
                            .to_string(),
                    );
                }
                if rebuilt.latency_ms_total != graph.latency_ms_total
                    || rebuilt.latency_ms_max != graph.latency_ms_max
                {
                    fail(
                        "graph latency aggregates != re-derived from embedded accounting records"
                            .to_string(),
                    );
                }
                let mut rebuilt_nodes = rebuilt.nodes.clone();
                let mut graph_nodes = graph.nodes.clone();
                rebuilt_nodes.sort_by(|a, b| a.node_id.cmp(&b.node_id));
                graph_nodes.sort_by(|a, b| a.node_id.cmp(&b.node_id));
                if rebuilt_nodes != graph_nodes {
                    fail(
                        "graph nodes != re-derived node aggregates from embedded accounting records"
                            .to_string(),
                    );
                }
            }
            Err(error) => fail(format!("graph aggregate cannot be re-derived: {error}")),
        },
        Err(error) => fail(format!(
            "embedded accounting record is rejected by the ledger: {error}"
        )),
    }

    violations
}

/// Classify a completed run into the outcome lattice. The governed boundary
/// ran either to a sealed attempt or to a typed, sealed failure; both are
/// product outcomes. Only pre-effect refusals and pipeline defects classify
/// otherwise.
pub fn classify_completed_run(
    resolution_ok: bool,
    dispatch: &Result<
        crate::workflow::route_dispatch::RoutedDispatchOutcome,
        RoutedDispatchFailure,
    >,
    accounting_ok: bool,
) -> ScenarioOutcomeClass {
    if !resolution_ok {
        return classify_pre_effect_refusal();
    }
    match classify_dispatch(dispatch) {
        ScenarioOutcomeClass::SetupError => ScenarioOutcomeClass::SetupError,
        ScenarioOutcomeClass::GovernedProductOutcome => {
            if accounting_ok {
                ScenarioOutcomeClass::GovernedProductOutcome
            } else {
                classify_accounting_failure()
            }
        }
        other => other,
    }
}

/// Serialize the envelope as canonical-ish JSON for inspection. Key order is
/// struct order; callers compare digests, never raw strings, across platforms.
pub fn envelope_json(evidence: &LiteScenarioRunEvidence) -> Result<String> {
    serde_json::to_string_pretty(evidence).context("scenario-run evidence serializes")
}

// ---------------------------------------------------------------------------
// Issue #136 Slice B — task-execution fixture evidence (Lite-internal)
// ---------------------------------------------------------------------------

/// Schema version of the Lite-internal task-run evidence document. 1.1.0
/// binds the validator record to the descriptor's pinned command/argv,
/// expected assertion and result receipt provenance.
pub const TASK_RUN_SCHEMA_VERSION: &str = "1.1.0";
/// Namespace of the Lite-internal task-run evidence envelope. Lite-internal;
/// explicitly NOT a Foundry namespace.
pub const TASK_RUN_NAMESPACE: &str = "lite.evidence.task-run";
/// Identifier of the local fixture runner that executes the task fixture.
pub const TASK_RUN_RUNNER_ID: &str = "lite-task-execution-fixture-runner";
/// Revision of the task fixture runner. Bump on any behavioural change.
pub const TASK_RUN_RUNNER_REVISION: &str = "2";
/// What `LiteTaskRunEvidence::verify` proves and does not prove about the
/// validator receipt. Pinned on every envelope and enforced at sealing.
pub const TASK_RUN_VERIFICATION_SCOPE: &str = "internally consistent operator-observed measurements: the embedded records re-derive cleanly. It is NOT cryptographic proof that an external process produced the recorded output; a fully consistent re-forgery cannot be detected from bytes alone. The pinned descriptor command/argv and the pinned expected-outcome digests narrow the forgery surface but do not remove it.";

/// Where the repair content came from. Exactly two origins exist and they
/// are typed (not free-form strings) so "the model fixed the bug" can
/// never be smuggled into this record or scripted provenance can never be
/// relabelled as model output:
///
/// * [`TaskRepairOrigin::ScriptedFixture`] — a pre-scripted, committed,
///   reviewed repair fixture.
/// * [`TaskRepairOrigin::ModelAuthored`] — a patch authored at run time by
///   the routed model, bound to the model's raw response and to the parsed
///   proposal it produced (see [`ModelPatchProvenance`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskRepairOrigin {
    /// Scripted fixture repair, authored against the pinned bytes.
    ScriptedFixture,
    /// Repair authored at run time by the routed model (#136 Slice D).
    ModelAuthored,
}

/// What happened to the task itself. Deliberately distinct from
/// [`ScenarioOutcomeClass`] (which says which part of the system ran): a
/// repair that applies cleanly and still fails `cargo test` is a **task
/// failure**, a pre-effect refusal is a **setup failure**, and neither is a
/// false success.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskOutcome {
    /// Patch applied; the validator ran and exited 0; the post-run tree
    /// digest moved off the baseline.
    Repaired,
    /// Patch applied but the task was not achieved: the validator failed,
    /// timed out, or could not run. Never a false success.
    RepairFailed,
    /// Pre-effect refusal (pin/workspace/authority/policy/fixture mismatch).
    /// Zero filesystem effects by construction; never a false success.
    RepairRefused,
}

/// Where the repair write authority came from. Typed so the two write
/// paths can never be confused: a scripted fixture repair authorized by
/// the workspace+file policy, or a model-authored repair authorized the
/// same way (never by the governed model dispatch itself — its node
/// manifest pins `writableScopes: []`). The governed model dispatch holds
/// no write authority in either case; this record makes the separation
/// explicit instead of borrowing the model's authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskRepairWriteAuthority {
    /// Write authorized by the isolated writable workspace (#171) plus the
    /// file policy editable set for a fixture-scripted patch. Never the
    /// model's authority.
    ScriptedFixtureWorkspaceFilePolicy,
    /// Write authorized by the isolated writable workspace (#171) plus the
    /// file policy editable set for a model-authored patch. Never the
    /// model's authority.
    ModelAuthoredWorkspaceFilePolicy,
}

/// How the recorded validator result was observed. Only a subprocess the
/// runner spawned itself with captured output/exit code is `ObservedSubprocess`;
/// anything the runner did not execute carries `NotExecuted` and can never
/// support a repaired outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskValidatorProvenance {
    /// The fixture runner spawned the subprocess, captured stdout+stderr,
    /// measured the exit code and the wall time in-process.
    ObservedSubprocess,
    /// The validator did not produce a process result (refused preflight,
    /// or a tool that could not be spawned/executed).
    NotExecuted,
}

/// How a model-patch provenance was observed. Required for any
/// model-authored repair sealed after the 2026-11 trust-boundary repair.
/// The offline suite can only ever produce
/// [`TaskRepairProvenanceKind::CannedTest`]; a response observed through an
/// authorized real-model loopback gateway produces
/// [`TaskRepairProvenanceKind::OperatorObservedLive`]. A sealed canned
/// fixture can never be upgraded to the live kind by re-sealing, and an
/// offline envelope can never claim the live kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskRepairProvenanceKind {
    /// Deterministic canned loopback responder used by the offline suite.
    /// Not a genuine external model, not independently authenticated.
    CannedTest,
    /// Response observed through an authorized real-model loopback gateway.
    /// Operator-observed, not cryptographically authenticated by the
    /// provider (no provider attestation is retained).
    OperatorObservedLive,
}

/// Provenance of a model-authored patch (#136 Slice D, trust-boundary
/// repair). Binds the edit digest to the *embedded routed attempt*, to the
/// raw response bytes recorded in this very record, and to the routed
/// provider/model and harness identity. The recorded response bytes are
/// self-consistency-checked (the raw response hashes to
/// [`ModelPatchProvenance::model_response_digest`]); provider authenticity
/// is operator-observed and is NOT cryptographically attested.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelPatchProvenance {
    /// sha256 (hex64) of the raw model response bytes observed by the
    /// runner, before any parsing.
    pub model_response_digest: String,
    /// Canonical digest ([`try_canonical_digest`] equivalent over the
    /// parsed proposal document) of the edit operations the model
    /// proposed. Must equal [`TaskRepairRecord::patch_digest`].
    pub proposal_digest: String,
    /// 1-based attempt number that produced the applied patch.
    pub attempt: u32,
    /// Model identifier as pinned by the route candidate and re-checked by
    /// the harness. Never taken from the model's self-report; must equal
    /// the embedded routed attempt's model.
    pub model_identity: String,
    /// Harness id and revision that mediated the call,
    /// `"{harnessId}@{revision}"` (e.g. `lite.harness.local_model@v1`).
    /// Must equal the embedded routed attempt's harness id and the
    /// descriptor's pinned harness revision.
    pub harness_revision: String,
    /// How this provenance was observed. `None` on envelopes sealed
    /// before the trust-boundary repair; such envelopes can never pass
    /// verification again (they are preserved historical evidence, not
    /// policy-compliant run records).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<TaskRepairProvenanceKind>,
    /// The raw model response bytes as observed by the gateway/harness,
    /// capped by the harness's output limit. Verification recomputes
    /// their sha256 and requires it to equal [`ModelPatchProvenance::model_response_digest`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_response: Option<String>,
    /// Idempotency key of the routed attempt that produced the applied
    /// patch; must equal the embedded routed attempt's idempotency key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_idempotency_key: Option<String>,
}

/// The repair the fixture runner attempted. Single-file scoped on purpose:
/// the pinned tasks carry one editable source file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskRepairRecord {
    pub origin: TaskRepairOrigin,
    /// Whose authority made the write. Explicitly the workspace/file-policy
    /// permit — never the governed model dispatch.
    pub write_authority: TaskRepairWriteAuthority,
    /// Patch action used, e.g. `"search-replace"`.
    pub action: String,
    /// Repo-relative, forward-slash paths the patch touched. Empty iff
    /// `applied` is false.
    pub changed_files: Vec<String>,
    /// sha256 of the single target file before the patch.
    pub file_before_digest: String,
    /// sha256 of the single target file after the patch (equal to
    /// `file_before_digest` when refused).
    pub file_after_digest: String,
    /// sha256 of the canonical patch document (the edit operations).
    pub patch_digest: String,
    /// Whether the patch was actually applied to the workspace.
    pub applied: bool,
    /// Model-authored provenance. Present iff `origin` is
    /// [`TaskRepairOrigin::ModelAuthored`]; absent for scripted fixtures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_provenance: Option<ModelPatchProvenance>,
}

/// The validator the fixture runner executed. The command/argv/expected
/// exit code are PINNED by the descriptor's `validation` section and
/// re-checked here; a different command, argument list, expected assertion
/// or receipt shape masquerading as this validator is rejected at
/// verification. A missing/broken validator is an infrastructure condition,
/// recorded as-is — never silently upgraded to pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskValidatorRecord {
    /// Exact executable, copied from the descriptor pin.
    pub command: String,
    /// Exact argument vector, copied from the descriptor pin.
    pub args: Vec<String>,
    /// The pinned expected exit code that means the assertion holds.
    pub expected_exit_code: i32,
    /// The pinned expected-assertion statement (what "done" means).
    pub assertion: String,
    /// Process exit code, or `None` when the validator never produced one
    /// (tool missing or timed out and was killed).
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    /// sha256 (hex64) of the captured stdout+stderr bytes. `None` only when
    /// the validator never ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_digest: Option<String>,
    /// Total captured stdout+stderr bytes.
    pub output_bytes: u64,
    /// Measured wall time of the validator subprocess in milliseconds.
    /// Provenance of the measurement, not a benchmark.
    pub duration_ms: u64,
    /// How the result was observed. Only `ObservedSubprocess` may carry a
    /// repaired outcome.
    pub provenance: TaskValidatorProvenance,
}

/// The Lite-internal task-run evidence envelope. **Embeds the entire
/// [`LiteScenarioRunEvidence`]** (descriptor, route evidence, snapshot,
/// attempt, journal entry, accounting, graph aggregate, workspace manifest)
/// rather than referencing it by digest, so [`LiteTaskRunEvidence::verify`]
/// re-derives everything from embedded bytes alone. Never a Foundry
/// contract; the external evaluation format stays UNSUPPORTED.
///
/// Honesty boundary: this record proves a scripted fixture repair — or a
/// model-authored repair bound by [`ModelPatchProvenance`] to the raw
/// model response and to the parsed proposal the applied edits came from —
/// was applied in an authorised disposable workspace and that the pinned
/// validator ran and exited with the recorded status. It does **not**
/// rank models, does not benchmark anything, and carries no benchmark
/// semantics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiteTaskRunEvidence {
    pub schema_version: String,
    pub namespace: String,
    /// Pinned evaluation instant of the underlying run (deterministic).
    pub evaluated_at: String,
    pub scenario_run: LiteScenarioRunEvidence,
    pub repair: TaskRepairRecord,
    pub validator: TaskValidatorRecord,
    /// [`task_run_tree_digest`] of the workspace source tree before any
    /// effect (git worktree pointer excluded). Must equal the embedded
    /// descriptor's pinned `repository.contentDigest`.
    pub baseline_content_digest: String,
    /// [`task_run_tree_digest`] of the workspace source tree after the patch
    /// and after the validator ran (includes validator-created files such as
    /// `Cargo.lock`). Must equal the descriptor's pinned
    /// `validation.expectedPostRunDigest` when the repair was applied.
    pub post_run_content_digest: String,
    pub task_outcome: TaskOutcome,
    pub classification: ScenarioOutcomeClass,
    pub external_evaluation_format: ExternalEvaluationFormat,
    pub provenance: RunProvenance,
    /// Pinned at sealing to [`TASK_RUN_VERIFICATION_SCOPE`]; re-verified at
    /// verification. States exactly what this record proves and does not
    /// prove about the validator receipt.
    pub verification_scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
}

/// Well-known sha256 of the empty byte string. Receipt consistency for the
/// zero-output-bytes case is checkable and is checked.
fn sha256_empty_hex() -> String {
    hex_lower(&Sha256::digest([]))
}

/// Derive the outcome that follows from the measured fields. Used by
/// [`LiteTaskRunEvidence::verify`] so a re-sealed envelope cannot assert a
/// success the measurements do not support: success requires a subprocess the
/// runner actually executed whose exit code equals the descriptor-pinned
/// expected exit code.
fn derive_task_outcome(
    repair: &TaskRepairRecord,
    validator: &TaskValidatorRecord,
    expected_exit_code: i32,
) -> TaskOutcome {
    if !repair.applied {
        return TaskOutcome::RepairRefused;
    }
    if validator.provenance == TaskValidatorProvenance::ObservedSubprocess
        && validator.exit_code == Some(expected_exit_code)
        && !validator.timed_out
    {
        return TaskOutcome::Repaired;
    }
    TaskOutcome::RepairFailed
}

/// Every task-run-specific binding that must hold before sealing. Returns
/// the full list of violations (empty = bound). Shared by `sealed` and
/// `verify`. `descriptor` is the embedded run's descriptor: the validator
/// receipt is checked against ITS pins, so a spliced record cannot borrow
/// another scenario's expectation. `attempt` is the embedded routed
/// attempt: a model-authored repair is bound to it exactly, so a record
/// cannot borrow another dispatch's identity.
fn check_task_run_bindings(
    descriptor: &ScenarioDescriptor,
    attempt: &RoutedAttemptV1,
    repair: &TaskRepairRecord,
    validator: &TaskValidatorRecord,
    baseline: &str,
    post_run: &str,
    task_outcome: TaskOutcome,
    classification: ScenarioOutcomeClass,
) -> Vec<String> {
    let mut violations: Vec<String> = Vec::new();
    let mut fail = |message: String| violations.push(message);

    if repair.action.trim().is_empty() {
        fail("repair.action must be non-empty".to_string());
    }
    if !is_hex64(&repair.file_before_digest) {
        fail("repair.fileBeforeDigest must be a lower hex sha256 digest".to_string());
    }
    if !is_hex64(&repair.file_after_digest) {
        fail("repair.fileAfterDigest must be a lower hex sha256 digest".to_string());
    }
    if !is_hex64(&repair.patch_digest) {
        fail("repair.patchDigest must be a lower hex sha256 digest".to_string());
    }
    if !is_hex64(baseline) {
        fail("baselineContentDigest must be a lower hex sha256 digest".to_string());
    }
    if !is_hex64(post_run) {
        fail("postRunContentDigest must be a lower hex sha256 digest".to_string());
    }
    // ---- Origin / authority / provenance consistency ------------------------
    match repair.origin {
        TaskRepairOrigin::ScriptedFixture => {
            if repair.write_authority
                != TaskRepairWriteAuthority::ScriptedFixtureWorkspaceFilePolicy
            {
                fail("a scripted-fixture repair must carry the scripted-fixture workspace/file-policy write authority".to_string());
            }
            if repair.model_provenance.is_some() {
                fail(
                    "a scripted-fixture repair cannot carry model-authored provenance".to_string(),
                );
            }
        }
        TaskRepairOrigin::ModelAuthored => {
            if repair.write_authority != TaskRepairWriteAuthority::ModelAuthoredWorkspaceFilePolicy
            {
                fail("a model-authored repair must carry the model-authored workspace/file-policy write authority".to_string());
            }
            match &repair.model_provenance {
                None => fail("a model-authored repair must carry model provenance".to_string()),
                Some(provenance) => {
                    if !is_hex64(&provenance.model_response_digest) {
                        fail(
                            "modelProvenance.modelResponseDigest must be a lower hex sha256 digest"
                                .to_string(),
                        );
                    }
                    if !is_hex64(&provenance.proposal_digest) {
                        fail(
                            "modelProvenance.proposalDigest must be a lower hex sha256 digest"
                                .to_string(),
                        );
                    }
                    if provenance.attempt == 0 {
                        fail("modelProvenance.attempt must be at least 1".to_string());
                    }
                    if provenance.model_identity.trim().is_empty() {
                        fail("modelProvenance.modelIdentity must be non-empty".to_string());
                    }
                    if provenance.harness_revision.trim().is_empty() {
                        fail("modelProvenance.harnessRevision must be non-empty".to_string());
                    }
                    // ---- Trust-boundary binding: the embedded routed attempt --
                    match &provenance.kind {
                        None => fail(
                            "modelProvenance.kind must state canned-test or operator-observed-live"
                                .to_string(),
                        ),
                        Some(TaskRepairProvenanceKind::CannedTest)
                        | Some(TaskRepairProvenanceKind::OperatorObservedLive) => {}
                    }
                    match &provenance.raw_response {
                        None => fail(
                            "modelProvenance.rawResponse must embed the raw response bytes (re-sealed historical envelopes can never verify)"
                                .to_string(),
                        ),
                        Some(raw) => {
                            if !is_hex64(&provenance.model_response_digest) {
                                // already reported above; avoid cascading errors
                            } else {
                                let digest =
                                    hex_lower(&Sha256::digest(raw.as_bytes()));
                                if digest != provenance.model_response_digest {
                                    fail(
                                        "modelProvenance.rawResponse does not hash to modelProvenance.modelResponseDigest"
                                            .to_string(),
                                    );
                                }
                            }
                        }
                    }
                    match &provenance.attempt_idempotency_key {
                        None => fail(
                            "modelProvenance.attemptIdempotencyKey must bind the embedded routed attempt"
                                .to_string(),
                        ),
                        Some(key) => {
                            if key != &attempt.idempotency_key {
                                fail(
                                    "modelProvenance.attemptIdempotencyKey != embedded routed attempt idempotency key (the recorded patch is not bound to this dispatch)"
                                        .to_string(),
                                );
                            }
                        }
                    }
                    let expected_model = attempt.model.clone().unwrap_or_default();
                    if provenance.model_identity != expected_model {
                        fail(
                            "modelProvenance.modelIdentity != embedded routed attempt model (provider/model identity mismatch)"
                                .to_string(),
                        );
                    }
                    let expected_harness =
                        format!("{}@{}", attempt.harness_id, descriptor.harness.revision);
                    if provenance.harness_revision != expected_harness {
                        fail(
                            "modelProvenance.harnessRevision != descriptor harness id@revision (harness identity mismatch)"
                                .to_string(),
                        );
                    }
                    if provenance.proposal_digest != repair.patch_digest {
                        fail(
                            "modelProvenance.proposalDigest != repair.patchDigest (the applied edits are not the edits the model proposed)"
                                .to_string(),
                        );
                    }
                }
            }
        }
    }
    // ---- Descriptor-pinned validator surface -------------------------------
    let pins = match &descriptor.validation {
        Some(pins) => Some(pins),
        None => {
            fail(
                "task-run evidence requires the descriptor to pin a `validation` section (command/argv, expected exit code, expected assertion and expected digests); unpinned validators carry no claim of success"
                    .to_string(),
            );
            None
        }
    };
    if let Some(pins) = pins {
        if validator.command != pins.command {
            fail(format!(
                "validator.command {:?} != descriptor validation.command {:?}",
                validator.command, pins.command
            ));
        }
        if validator.args != pins.args {
            fail(format!(
                "validator.args {:?} != descriptor validation.args {:?}",
                validator.args, pins.args
            ));
        }
        if validator.expected_exit_code != pins.expected_exit_code {
            fail(
                "validator.expectedExitCode != descriptor validation.expectedExitCode".to_string(),
            );
        }
        if validator.assertion != pins.expected_assertion {
            fail("validator.assertion != descriptor validation.expectedAssertion".to_string());
        }
        if baseline != descriptor.repository.content_digest {
            fail(
                "baselineContentDigest != descriptor.repository.contentDigest (the measured pre-image is not the pinned fixture)"
                    .to_string(),
            );
        }
    }

    // ---- Validator receipt consistency --------------------------------------
    match validator.provenance {
        TaskValidatorProvenance::NotExecuted => {
            if validator.exit_code.is_some() || validator.timed_out {
                fail("a NotExecuted validator cannot carry a process result".to_string());
            }
            if validator.output_digest.is_some() {
                fail("a NotExecuted validator cannot carry an output digest".to_string());
            }
            if validator.output_bytes != 0 {
                fail("a NotExecuted validator must carry zero captured bytes".to_string());
            }
        }
        TaskValidatorProvenance::ObservedSubprocess => match &validator.output_digest {
            None => fail(
                "an ObservedSubprocess validator must carry the digest of its captured output"
                    .to_string(),
            ),
            Some(digest) => {
                if !is_hex64(digest) {
                    fail("validator.outputDigest must be a lower hex sha256 digest".to_string());
                } else if validator.output_bytes == 0 && digest != &sha256_empty_hex() {
                    fail(
                        "a zero-byte capture must digest to the well-known empty sha256"
                            .to_string(),
                    );
                }
            }
        },
    }
    if validator.exit_code.is_some()
        && validator.provenance != TaskValidatorProvenance::ObservedSubprocess
    {
        fail("only an ObservedSubprocess validator can carry an exit code".to_string());
    }

    // The outcome must follow from the measurements, and the classification
    // must follow from the outcome. No combination asserts success without
    // the pinned validator's exit status supporting it.
    let expected_exit_code = pins.map(|pins| pins.expected_exit_code).unwrap_or(0);
    let derived = derive_task_outcome(repair, validator, expected_exit_code);
    if derived != task_outcome {
        fail(format!(
            "taskOutcome {task_outcome:?} does not follow from applied={} exit_code={:?} timed_out={} (derived {derived:?})",
            repair.applied, validator.exit_code, validator.timed_out
        ));
    }
    for path in &repair.changed_files {
        if let Err(message) = check_fixture_path(path) {
            fail(format!("repair.changedFiles: {message}"));
        }
        if path.contains('\\') || Path::new(path).is_absolute() {
            fail(format!(
                "repair.changedFiles entry {path:?} is not repo-relative"
            ));
        }
    }
    if repair.applied != !repair.changed_files.is_empty() {
        fail("repair.applied and repair.changedFiles disagree (empty change set but applied, or vice versa)".to_string());
    }
    if repair.applied && repair.file_after_digest == repair.file_before_digest {
        fail("repair claims to be applied but the file digest did not move".to_string());
    }
    if !repair.applied && repair.file_after_digest != repair.file_before_digest {
        fail("repair was refused but the file digest moved".to_string());
    }
    if repair.applied && post_run == baseline {
        fail("repair was applied but the tree digest did not move off the baseline".to_string());
    }
    if !repair.applied && post_run != baseline {
        fail(
            "pre-effect refusal must leave the tree digest at the baseline (zero effects)"
                .to_string(),
        );
    }
    if validator.timed_out && validator.exit_code.is_some() {
        fail("a timed-out validator cannot carry an exit code".to_string());
    }

    match task_outcome {
        TaskOutcome::Repaired => {
            if classification != ScenarioOutcomeClass::GovernedProductOutcome {
                fail("a repaired task must be a governed-product-outcome, not a setup/infrastructure class".to_string());
            }
            if validator.provenance != TaskValidatorProvenance::ObservedSubprocess {
                fail(
                    "a repaired outcome requires ObservedSubprocess validator provenance"
                        .to_string(),
                );
            }
            if let Some(pins) = &descriptor.validation {
                if validator.exit_code != Some(pins.expected_exit_code) {
                    fail(
                        "repaired outcome but validator.exitCode != pinned validation.expectedExitCode"
                            .to_string(),
                    );
                }
                // Byte-exact pins bind a repair to the task-setter's reference
                // bytes. They are meaningful for a scripted fixture; a genuine
                // model may author any behavior-correct repair, so for a
                // model-authored repair the applied-vs-proposed provenance
                // binding above replaces the byte pin.
                if repair.origin == TaskRepairOrigin::ScriptedFixture {
                    if repair.file_after_digest != pins.expected_file_after_digest {
                        fail(
                            "repaired outcome but repair.fileAfterDigest != pinned validation.expectedFileAfterDigest"
                                .to_string(),
                        );
                    }
                    if post_run != pins.expected_post_run_digest {
                        fail(
                            "repaired outcome but postRunContentDigest != pinned validation.expectedPostRunDigest"
                                .to_string(),
                        );
                    }
                }
            }
        }
        TaskOutcome::RepairFailed => {
            if !matches!(
                classification,
                ScenarioOutcomeClass::GovernedProductOutcome
                    | ScenarioOutcomeClass::InfrastructureError
            ) {
                fail("a failed repair must classify as governed-product-outcome (validator ran and failed) or infrastructure-error (validator could not complete), never as success or setup".to_string());
            }
        }
        TaskOutcome::RepairRefused => {
            if !matches!(
                classification,
                ScenarioOutcomeClass::SetupError | ScenarioOutcomeClass::InvalidTask
            ) {
                fail(
                    "a refused repair (pre-effect) must classify as setup-error or invalid-task"
                        .to_string(),
                );
            }
            if validator.exit_code.is_some() || validator.timed_out {
                fail(
                    "a refused repair must not carry a validator result (the validator never ran)"
                        .to_string(),
                );
            }
        }
    }
    if validator.timed_out && classification != ScenarioOutcomeClass::InfrastructureError {
        fail("a timed-out validator classifies as infrastructure-error".to_string());
    }
    if validator.exit_code.is_none()
        && !validator.timed_out
        && classification != ScenarioOutcomeClass::SetupError
        && classification != ScenarioOutcomeClass::InvalidTask
        && classification != ScenarioOutcomeClass::InfrastructureError
    {
        fail("a validator without an exit code cannot be a governed-product-outcome".to_string());
    }

    violations
}

impl LiteTaskRunEvidence {
    /// Canonical digest of the envelope minus `contentDigest`.
    pub fn compute_digest(&self) -> Result<String> {
        let mut value = serde_json::to_value(self).context("task-run evidence serializes")?;
        if let Some(obj) = value.as_object_mut() {
            obj.remove("contentDigest");
        }
        soma::try_canonical_digest(&value)
            .map_err(|error| anyhow::anyhow!("task-run evidence canonical digest failed: {error}"))
    }

    /// Scenario identity, derived from the embedded descriptor.
    pub fn scenario_id(&self) -> &str {
        self.scenario_run.scenario_id()
    }

    /// Canonical digest of the embedded descriptor.
    pub fn scenario_digest(&self) -> Result<String> {
        self.scenario_run.scenario_digest()
    }

    /// Build and seal a task-run envelope from the records of one completed
    /// (or refused) task run. Every binding is checked BEFORE sealing; on any
    /// mismatch this returns the collected discrepancies and nothing is
    /// exported. Nothing here is caller-asserted into place: the outcome is
    /// re-derived from the measured fields.
    #[allow(clippy::too_many_arguments)]
    pub fn sealed(
        scenario_run: LiteScenarioRunEvidence,
        repair: TaskRepairRecord,
        validator: TaskValidatorRecord,
        baseline_content_digest: String,
        post_run_content_digest: String,
        classification: ScenarioOutcomeClass,
    ) -> Result<Self> {
        let task_outcome = derive_task_outcome(
            &repair,
            &validator,
            scenario_run
                .descriptor
                .validation
                .as_ref()
                .map(|pins| pins.expected_exit_code)
                .unwrap_or(0),
        );
        let violations = check_task_run_bindings(
            &scenario_run.descriptor,
            &scenario_run.attempt,
            &repair,
            &validator,
            &baseline_content_digest,
            &post_run_content_digest,
            task_outcome,
            classification,
        );
        if !violations.is_empty() {
            anyhow::bail!(
                "task-run evidence refuses to seal; {} binding(s) failed: {}",
                violations.len(),
                violations.join("; ")
            );
        }
        scenario_run.verify()?;
        let mut record = Self {
            schema_version: TASK_RUN_SCHEMA_VERSION.to_string(),
            namespace: TASK_RUN_NAMESPACE.to_string(),
            evaluated_at: scenario_run.evaluated_at.clone(),
            scenario_run,
            repair,
            validator,
            baseline_content_digest,
            post_run_content_digest,
            task_outcome,
            classification,
            external_evaluation_format: ExternalEvaluationFormat::unsupported(),
            provenance: RunProvenance {
                runner: TASK_RUN_RUNNER_ID.to_string(),
                runner_revision: TASK_RUN_RUNNER_REVISION.to_string(),
                injected_clock: true,
                fixture_only: true,
                redaction: "digest-sealed artifacts carry identities and digests only; raw secret material is never serialized (canary-verified in tests)".to_string(),
            },
            verification_scope: TASK_RUN_VERIFICATION_SCOPE.to_string(),
            content_digest: None,
        };
        record.content_digest = Some(record.compute_digest()?);
        Ok(record)
    }

    /// Fail-closed re-verification from the embedded bytes alone.
    ///
    /// Re-verifies the embedded scenario-run envelope (which re-derives every
    /// route/attempt/journal/accounting/workspace binding), then re-checks the
    /// task-run-specific bindings and re-derives the outcome from the
    /// measured fields. An envelope sealed over measurements that do not
    /// support a claimed outcome is rejected, never admitted.
    pub fn verify(&self) -> anyhow::Result<()> {
        use anyhow::ensure;
        ensure!(
            self.schema_version == TASK_RUN_SCHEMA_VERSION,
            "unsupported task-run schema version {} (supported {TASK_RUN_SCHEMA_VERSION})",
            self.schema_version
        );
        ensure!(
            self.namespace == TASK_RUN_NAMESPACE,
            "task-run namespace {:?} is not {TASK_RUN_NAMESPACE:?}",
            self.namespace
        );
        match &self.content_digest {
            Some(actual) => {
                let expected = self.compute_digest()?;
                ensure!(
                    *actual == expected,
                    "task-run evidence contentDigest does not verify"
                );
            }
            None => anyhow::bail!("task-run evidence is missing its contentDigest"),
        }
        ensure!(
            self.evaluated_at == self.scenario_run.evaluated_at,
            "task-run evaluatedAt does not bind the embedded run's evaluation instant"
        );
        ensure!(
            self.verification_scope == TASK_RUN_VERIFICATION_SCOPE,
            "task-run verificationScope must be pinned to the documented verification limitation"
        );
        self.scenario_run.verify()?;
        let violations = check_task_run_bindings(
            &self.scenario_run.descriptor,
            &self.scenario_run.attempt,
            &self.repair,
            &self.validator,
            &self.baseline_content_digest,
            &self.post_run_content_digest,
            self.task_outcome,
            self.classification,
        );
        ensure!(
            violations.is_empty(),
            "task-run evidence failed binding checks: {}",
            violations.join("; ")
        );
        ensure!(
            self.external_evaluation_format.status == "unsupported",
            "externalEvaluationFormat.status must be \"unsupported\""
        );
        ensure!(
            self.provenance.fixture_only && self.provenance.injected_clock,
            "task-run evidence must be fixture-only with an injected clock"
        );
        Ok(())
    }
}

/// Serialize the task-run envelope as canonical-ish JSON for inspection.
pub fn task_run_json(evidence: &LiteTaskRunEvidence) -> Result<String> {
    serde_json::to_string_pretty(evidence).context("task-run evidence serializes")
}
