//! Typed SOMA contract models: `AuthorityProfile`, `OperationDefinition`,
//! `PortDefinition`, `EvidenceReference`, `WorkflowDefinition`, and
//! `GovernanceConstraint` — mirroring the published v1.1 schemas with
//! fail-closed serde (deny_unknown_fields). Ported from the published
//! reference implementation (`prometheosai/soma`, crates/soma-validate).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Number;

use super::types::{
    Cardinality, ExecutionClass, Hex64, MutationMode, NetworkDefault, OutcomeVariant, WorkflowKind,
};
pub use super::types::{ConstraintKind, Direction, Requiredness};

// ---------------------------------------------------------------------------
// AuthorityProfile
// ---------------------------------------------------------------------------

/// Escalation (recovery) target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct EscalationPolicy {
    pub to: String,
}

/// Human-review requirement over an effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ReviewRequirement {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "reviewerPolicy"
    )]
    pub reviewer_policy: Option<String>,
}

/// Network access policy: a default stance plus an allowlist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct NetworkPolicy {
    pub default: NetworkDefault,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowlist: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProviderPolicy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowlist: Option<Vec<String>>,
}

/// A named secret grant with optional scope narrowing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SecretGrant {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scopes: Option<Vec<String>>,
}

/// Budget ceilings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Budgets {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retries: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concurrency: Option<Number>,
}

impl Budgets {
    pub fn get(&self, dim: &str) -> Option<&Number> {
        match dim {
            "tokens" => self.tokens.as_ref(),
            "cost" => self.cost.as_ref(),
            "duration" => self.duration.as_ref(),
            "retries" => self.retries.as_ref(),
            "concurrency" => self.concurrency.as_ref(),
            _ => None,
        }
    }

    pub const DIMENSIONS: [&'static str; 5] =
        ["tokens", "cost", "duration", "retries", "concurrency"];
}

/// Abstention behavior when review cannot be obtained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Abstention {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behavior: Option<String>,
}

/// Restricted-content rule: content class `to` is prohibited.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ContentRestriction {
    pub to: String,
}

/// The full authority profile granted to a workflow or unit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AuthorityProfile {
    #[serde(rename = "executionClass")]
    pub execution_class: ExecutionClass,
    pub mutation: MutationMode,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "readableScopes"
    )]
    pub readable_scopes: Option<Vec<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "writableScopes"
    )]
    pub writable_scopes: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<BTreeMap<String, Vec<String>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "networkPolicy"
    )]
    pub network_policy: Option<NetworkPolicy>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "providerPolicy"
    )]
    pub provider_policy: Option<ProviderPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secrets: Option<Vec<SecretGrant>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalation: Option<EscalationPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<ReviewRequirement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abstention: Option<Abstention>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budgets: Option<Budgets>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "contentRestrictions"
    )]
    pub content_restrictions: Option<Vec<ContentRestriction>>,
}

impl AuthorityProfile {
    pub fn tool_keys(&self) -> Vec<&str> {
        self.tools
            .as_ref()
            .map(|t| t.keys().map(String::as_str).collect())
            .unwrap_or_default()
    }

    pub fn declared_secret_names(&self) -> Vec<&str> {
        self.secrets
            .as_ref()
            .map(|s| s.iter().map(|g| g.name.as_str()).collect())
            .unwrap_or_default()
    }

    pub fn has_recovery_path(&self) -> bool {
        self.escalation.as_ref().is_some_and(|e| !e.to.is_empty())
    }
}

// ---------------------------------------------------------------------------
// PortDefinition / OperationDefinition / EvidenceReference
// ---------------------------------------------------------------------------

/// A workflow boundary port (`PortDefinition.schema.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PortDefinition {
    pub name: String,
    pub direction: Direction,
    /// SOMA type-vocabulary identifier (validated by the workflow audit).
    #[serde(rename = "type")]
    pub ty: String,
    pub cardinality: Cardinality,
    pub requiredness: Requiredness,
    /// Optional free-form contract attachment.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "dataContract"
    )]
    pub data_contract: Option<BTreeMap<String, serde_json::Value>>,
}

impl PortDefinition {
    pub fn is_required(&self) -> bool {
        self.requiredness == Requiredness::Required
    }
}

/// An operation input: a typed value with the outcome variants it accepts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct OperationInput {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(rename = "acceptedOutcomes")]
    pub accepted_outcomes: Vec<OutcomeVariant>,
}

/// An operation output: a typed value declaring the outcome variants it can
/// emit (the field that makes SOMA-OUT-0001/0002 decidable).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct OperationOutput {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emits: Option<Vec<OutcomeVariant>>,
}

/// A side effect an operation may produce.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Effect {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub irreversible: Option<bool>,
}

/// Retry policy for an operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RetryPolicy {
    #[serde(rename = "maxAttempts")]
    pub max_attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backoff: Option<String>,
}

/// One deterministic unit inside a workflow body
/// (`OperationDefinition.schema.json`).
///
/// Scope-qualified authority grants use the normative prefix grammar
/// `readable:<scope>` / `writable:<scope>`; bare entries are capability ids.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct OperationDefinition {
    #[serde(rename = "schemaVersion")]
    pub schema_version: String,
    pub id: String,
    pub version: String,
    #[serde(rename = "executionClass")]
    pub execution_class: ExecutionClass,
    pub inputs: Vec<OperationInput>,
    pub outputs: Vec<OperationOutput>,
    /// Granted capability ids and/or scope-qualified grants.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authority: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uses: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<ReviewRequirement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalation: Option<EscalationPolicy>,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        rename = "evidenceObligations"
    )]
    pub evidence_obligations: Vec<String>,
}

/// A reference binding evidence to an artifact and event digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct EvidenceReference {
    pub id: String,
    #[serde(rename = "eventDigest")]
    pub event_digest: Hex64,
    #[serde(rename = "artifactDigest")]
    pub artifact_digest: Hex64,
    #[serde(rename = "artifactKind")]
    pub artifact_kind: String,
    #[serde(rename = "producedBy")]
    pub produced_by: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "producedAt"
    )]
    pub produced_at: Option<String>,
}

// ---------------------------------------------------------------------------
// GovernanceConstraint + WorkflowDefinition
// ---------------------------------------------------------------------------

/// Governance constraint subject: the graph elements a predicate ranges over.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ConstraintSubject {
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "nodeSet")]
    pub node_set: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "edgeSet")]
    pub edge_set: Option<Vec<String>>,
}

/// A governance constraint over the workflow's own declarations. The
/// `predicate` uses the normative grammar `<field> <op> <json-argument>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct GovernanceConstraint {
    pub id: String,
    pub subject: ConstraintSubject,
    pub predicate: String,
    #[serde(rename = "violationCategory")]
    pub violation_category: String,
    pub kind: ConstraintKind,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "evaluationPoint"
    )]
    pub evaluation_point: Option<String>,
}

/// Context disclosure/requirement declaration.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WorkflowContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discloses: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<Vec<String>>,
}

/// An exported named effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct EffectExport {
    pub name: String,
}

/// The top-level governed workflow contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WorkflowDefinition {
    #[serde(rename = "schemaVersion")]
    pub schema_version: String,
    pub id: String,
    pub version: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<WorkflowKind>,
    #[serde(rename = "inputPorts")]
    pub input_ports: Vec<PortDefinition>,
    #[serde(rename = "outputPorts")]
    pub output_ports: Vec<PortDefinition>,
    pub body: Vec<OperationDefinition>,
    pub authority: AuthorityProfile,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<GovernanceConstraint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<EvidenceReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<WorkflowContext>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "effectExports"
    )]
    pub effect_exports: Option<Vec<EffectExport>>,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        rename = "authorityImports"
    )]
    pub authority_imports: Vec<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "repoRevision"
    )]
    pub repo_revision: Option<String>,
    /// Optional self-describing canonical digest (SOMA-CMP-0004).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "contentDigest"
    )]
    pub content_digest: Option<Hex64>,
}
