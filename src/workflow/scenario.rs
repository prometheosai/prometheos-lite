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
fn check_fixture_path(path: &str) -> Result<(), String> {
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
pub fn fixture_content_digest(root: &Path) -> Result<String> {
    let mut entries: Vec<(String, PathBuf)> = Vec::new();
    collect_files(root, root, &mut entries)?;
    entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut material = String::new();
    for (rel, full) in &entries {
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
        }
        // Symlinks and other non-regular files are ignored: the fixture
        // digest pins regular file content only.
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
pub struct RunRepositoryPin {
    pub fixture: String,
    pub content_digest: String,
    pub revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunRoutePin {
    pub route_set_id: String,
    pub route_set_version: String,
    pub candidate_id: String,
    pub provider_identity: String,
    pub provider_locality: String,
    pub model: String,
    pub harness_id: String,
    pub harness_revision: String,
    pub profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunWorkspacePin {
    pub manifest_digest: String,
    pub adapter: String,
    pub adapter_revision: String,
    pub mode: String,
    pub base_revision: String,
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

/// The Lite-internal scenario-run evidence envelope. Carries verified, raw
/// Lite records and digests. NEVER a Foundry contract; see module docs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiteScenarioRunEvidence {
    pub schema_version: String,
    pub namespace: String,
    pub scenario_id: String,
    pub scenario_digest: String,
    pub evaluated_at: String,
    pub repository: RunRepositoryPin,
    pub route: RunRoutePin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<RunWorkspacePin>,
    pub evidence: RunEvidenceChain,
    /// The raw, sealed attempt-accounting record produced natively by Lite.
    pub attempt_accounting: AttemptAccountingV1,
    /// The raw, aggregated graph accounting produced natively by Lite.
    pub graph_accounting: GraphAccounting,
    pub classification: ScenarioOutcomeClass,
    pub external_evaluation_format: ExternalEvaluationFormat,
    pub provenance: RunProvenance,
    /// Digest of the sealed accounting record (real, measured-latency digest).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
}

/// Digest chain over the native Lite records produced by the run. Every
/// digest re-verifies against the sealed records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunEvidenceChain {
    pub route_evidence_digest: String,
    pub snapshot_digest: String,
    pub decision_digest: String,
    pub requirements_digest: String,
    pub candidate_set_digest: String,
    pub attempt_digest: String,
    pub journal_entry_digest: String,
    pub accounting_record_digest: String,
    pub graph_id: String,
    pub graph_attempts: u64,
    pub graph_completed: u64,
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

    #[allow(clippy::too_many_arguments)]
    pub fn sealed(
        descriptor: &ScenarioDescriptor,
        evaluated_at: String,
        route: RunRoutePin,
        workspace: Option<RunWorkspacePin>,
        evidence: RunEvidenceChain,
        attempt_accounting: AttemptAccountingV1,
        graph_accounting: GraphAccounting,
        classification: ScenarioOutcomeClass,
    ) -> Result<Self> {
        let mut record = Self {
            schema_version: SCENARIO_RUN_SCHEMA_VERSION.to_string(),
            namespace: SCENARIO_RUN_NAMESPACE.to_string(),
            scenario_id: descriptor.scenario_id.clone(),
            scenario_digest: descriptor.scenario_digest()?,
            evaluated_at,
            repository: RunRepositoryPin {
                fixture: descriptor.repository.fixture.clone(),
                content_digest: descriptor.repository.content_digest.clone(),
                revision: descriptor.repository.revision.clone(),
            },
            route,
            workspace,
            evidence,
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

    /// Fail-closed re-verification.
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
        ensure!(
            is_hex64(&self.scenario_digest),
            "scenarioDigest is not lower hex sha256"
        );
        for (label, value) in [
            ("routeEvidenceDigest", &self.evidence.route_evidence_digest),
            ("snapshotDigest", &self.evidence.snapshot_digest),
            ("decisionDigest", &self.evidence.decision_digest),
            ("requirementsDigest", &self.evidence.requirements_digest),
            ("candidateSetDigest", &self.evidence.candidate_set_digest),
            ("attemptDigest", &self.evidence.attempt_digest),
            ("journalEntryDigest", &self.evidence.journal_entry_digest),
            (
                "accountingRecordDigest",
                &self.evidence.accounting_record_digest,
            ),
        ] {
            ensure!(is_hex64(value), "{label} is not lower hex sha256");
        }
        if let Some(workspace) = &self.workspace {
            ensure!(
                is_hex64(&workspace.manifest_digest),
                "workspace manifestDigest is not lower hex sha256"
            );
        }
        // Raw records re-verify against their own sealed digests.
        self.attempt_accounting.verify()?;
        ensure!(
            self.attempt_accounting.content_digest.as_deref()
                == Some(self.evidence.accounting_record_digest.as_str()),
            "accountingRecordDigest does not bind the embedded accounting record"
        );
        ensure!(
            self.graph_accounting.schema_version
                == crate::workflow::accounting::ACCOUNTING_SCHEMA_VERSION,
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
