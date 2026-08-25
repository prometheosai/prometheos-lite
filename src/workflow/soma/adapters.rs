//! `HarnessAdapter` and `AdapterConformance`: capability claims, reversible
//! field mappings, checkpoint/resume bindings, and exact semantic-digest
//! verification. Ported from the published reference implementation
//! (`prometheosai/soma`, crates/soma-validate/src/adapters.rs).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::contracts::EvidenceReference;
use super::types::{Hex64, MappingStrategy};
use super::{Diagnostic, SupportedVersion};

// ---------------------------------------------------------------------------
// HarnessAdapter
// ---------------------------------------------------------------------------

/// One reversible field-mapping step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct FieldMapping {
    pub source: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "semanticKey"
    )]
    pub semantic_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "dropOnLoss"
    )]
    pub drop_on_loss: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AdapterMapping {
    pub strategy: MappingStrategy,
    pub fields: Vec<FieldMapping>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AdapterIdentity {
    pub name: String,
    pub revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct EventCorrelation {
    pub scheme: String,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        rename = "requiredFields"
    )]
    pub required_fields: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Cancellation {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "propagatesTo"
    )]
    pub propagates_to: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "preservesState"
    )]
    pub preserves_state: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct EvidenceAttachment {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "maxBytes")]
    pub max_bytes: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct HandoffBehavior {
    /// const true when present
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "preservesSemanticIdentity"
    )]
    pub preserves_semantic_identity: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "requiresCheckpoint"
    )]
    pub requires_checkpoint: Option<bool>,
}

/// The harness adapter contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct HarnessAdapter {
    pub id: String,
    #[serde(rename = "schemaVersion")]
    pub schema_version: String,
    pub version: String,
    pub identity: AdapterIdentity,
    #[serde(rename = "declaredCapabilities")]
    pub declared_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limitations: Vec<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "inputMapping"
    )]
    pub input_mapping: Option<AdapterMapping>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "outputMapping"
    )]
    pub output_mapping: Option<AdapterMapping>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "checkpointImport"
    )]
    pub checkpoint_import: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "checkpointExport"
    )]
    pub checkpoint_export: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "eventCorrelation"
    )]
    pub event_correlation: Option<EventCorrelation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancellation: Option<Cancellation>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "evidenceAttachment"
    )]
    pub evidence_attachment: Option<EvidenceAttachment>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "handoffBehavior"
    )]
    pub handoff_behavior: Option<HandoffBehavior>,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        rename = "failureClassification"
    )]
    pub failure_classification: Vec<String>,
}

impl HarnessAdapter {
    /// Standalone adapter audit (capability claims).
    pub fn audit_claims(&self) -> Vec<Diagnostic> {
        let declared: BTreeSet<&str> = self
            .declared_capabilities
            .iter()
            .map(String::as_str)
            .collect();
        if self
            .capabilities
            .iter()
            .any(|c| !declared.contains(c.as_str()))
        {
            vec![Diagnostic::new(
                "SOMA-ADAPT-0001",
                "capability claim outside declared capabilities",
            )]
        } else {
            Vec::new()
        }
    }
}

// ---------------------------------------------------------------------------
// AdapterConformance
// ---------------------------------------------------------------------------

/// Resume token bound to a checkpoint envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ConformanceResumeToken {
    pub id: String,
    #[serde(rename = "suspensionPointId")]
    pub suspension_point_id: String,
    #[serde(rename = "suspensionDigest")]
    pub suspension_digest: Hex64,
    #[serde(rename = "workflowVersion")]
    pub workflow_version: String,
    #[serde(rename = "schemaVersion")]
    pub schema_version: String,
    #[serde(rename = "authoritySnapshotDigest")]
    pub authority_snapshot_digest: Hex64,
    #[serde(rename = "planDigest")]
    pub plan_digest: Hex64,
    #[serde(rename = "bindingDigest")]
    pub binding_digest: Hex64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expiration: Option<String>,
}

/// Checkpoint envelope inside a conformance scenario.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ScenarioCheckpointEnvelope {
    pub id: String,
    #[serde(rename = "suspensionPointId")]
    pub suspension_point_id: String,
    #[serde(rename = "planDigest")]
    pub plan_digest: Hex64,
    #[serde(rename = "authoritySnapshotDigest")]
    pub authority_snapshot_digest: Hex64,
    #[serde(rename = "checkpointDigest")]
    pub checkpoint_digest: Hex64,
    #[serde(rename = "eventsDigest")]
    pub events_digest: Hex64,
    #[serde(rename = "schemaVersion")]
    pub schema_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompatibilityRef {
    pub version: String,
}

/// A handoff round-trip scenario: canonical -> applied mapping -> mapped
/// output -> restored canonical -> identical semantic digest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct HandoffRoundtripScenario {
    /// const "handoff-roundtrip"
    pub kind: String,
    #[serde(rename = "canonicalInput")]
    pub canonical_input: serde_json::Map<String, serde_json::Value>,
    #[serde(rename = "appliedMapping")]
    pub applied_mapping: Vec<FieldMapping>,
    #[serde(rename = "mappedOutput")]
    pub mapped_output: serde_json::Map<String, serde_json::Value>,
    #[serde(rename = "restoredCanonical")]
    pub restored_canonical: serde_json::Map<String, serde_json::Value>,
    #[serde(rename = "semanticDigest")]
    pub semantic_digest: Hex64,
}

/// A checkpoint/resume binding scenario binding adapter, envelope, token,
/// evidence, revision and compatibility version together.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CheckpointResumeScenario {
    /// const "checkpoint-resume"
    pub kind: String,
    #[serde(rename = "adapterId")]
    pub adapter_id: String,
    #[serde(rename = "checkpointEnvelope")]
    pub checkpoint_envelope: ScenarioCheckpointEnvelope,
    #[serde(rename = "resumeToken")]
    pub resume_token: ConformanceResumeToken,
    pub evidence: Vec<EvidenceReference>,
    #[serde(rename = "repoRevision")]
    pub repo_revision: String,
    pub compatibility: CompatibilityRef,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "resumeRequested"
    )]
    pub resume_requested: Option<bool>,
}

/// One conformance scenario; discriminated on the `kind` field.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[allow(clippy::large_enum_variant)]
pub enum ConformanceScenario {
    HandoffRoundtrip(Box<HandoffRoundtripScenario>),
    CheckpointResume(CheckpointResumeScenario),
}

impl<'de> Deserialize<'de> for ConformanceScenario {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let kind = value
            .get("kind")
            .and_then(|k| k.as_str())
            .ok_or_else(|| serde::de::Error::missing_field("kind"))?;
        match kind {
            "handoff-roundtrip" => Ok(ConformanceScenario::HandoffRoundtrip(Box::new(
                serde_json::from_value(value).map_err(serde::de::Error::custom)?,
            ))),
            "checkpoint-resume" => Ok(ConformanceScenario::CheckpointResume(
                serde_json::from_value(value).map_err(serde::de::Error::custom)?,
            )),
            other => Err(serde::de::Error::custom(format!(
                "unknown scenario kind {other:?}"
            ))),
        }
    }
}

/// The adapter conformance container.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AdapterConformance {
    #[serde(rename = "schemaVersion")]
    pub schema_version: String,
    pub version: String,
    pub id: String,
    pub adapter: HarnessAdapter,
    pub scenarios: Vec<ConformanceScenario>,
}

/// Apply a field mapping to a flat object; losses carry the EXACT reason and
/// source field so semantic-loss diagnostics are attributable.
pub fn apply_field_mapping(
    fields: &[FieldMapping],
    obj: &serde_json::Map<String, serde_json::Value>,
) -> (
    serde_json::Map<String, serde_json::Value>,
    Vec<(&'static str, String)>,
) {
    let mut mapped = serde_json::Map::new();
    let mut losses = Vec::new();
    let mut covered = BTreeSet::new();
    for f in fields {
        covered.insert(f.source.as_str());
        if let Some(v) = obj.get(&f.source) {
            mapped.insert(f.target.clone(), v.clone());
        } else if f.required.unwrap_or(false) {
            losses.push(("missing-required", f.source.clone()));
        } else if !f.drop_on_loss.unwrap_or(false) {
            losses.push(("unmapped", f.source.clone()));
        }
    }
    for key in obj.keys() {
        if !covered.contains(key.as_str()) {
            losses.push(("semantic-loss", key.clone()));
        }
    }
    (mapped, losses)
}

fn first_diff_key(
    a: &serde_json::Map<String, serde_json::Value>,
    b: &serde_json::Map<String, serde_json::Value>,
) -> String {
    for k in a.keys() {
        if a.get(k) != b.get(k) {
            return k.clone();
        }
    }
    b.keys()
        .find(|k| !a.contains_key(*k))
        .cloned()
        .unwrap_or_else(|| "<unknown>".into())
}

impl AdapterConformance {
    /// Semantic audit against a supported bundle version.
    pub fn audit(&self, supported: &SupportedVersion) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        let adapter = &self.adapter;

        // SOMA-ADAPT-0001: claims outside declared capabilities.
        let declared_caps: BTreeSet<&str> = adapter
            .declared_capabilities
            .iter()
            .map(String::as_str)
            .collect();
        if adapter
            .capabilities
            .iter()
            .any(|c| !declared_caps.contains(c.as_str()))
        {
            out.push(Diagnostic::new(
                "SOMA-ADAPT-0001",
                "capability claim outside declared capabilities",
            ));
        }

        // SOMA-CMP-0001 on the container itself (both version fields).
        for v in [&self.schema_version, &self.version] {
            if let Ok(parsed) = super::types::SemVer::parse(v)
                && !parsed.is_compatible_with(supported)
            {
                out.push(Diagnostic::new(
                    "SOMA-CMP-0001",
                    format!("container version {v} unsupported"),
                ));
            }
        }

        let out_fields = adapter
            .output_mapping
            .as_ref()
            .map(|m| m.fields.as_slice())
            .unwrap_or(&[]);
        let in_fields = adapter
            .input_mapping
            .as_ref()
            .map(|m| m.fields.as_slice())
            .unwrap_or(&[]);

        for sc in &self.scenarios {
            match sc {
                ConformanceScenario::HandoffRoundtrip(s) => {
                    audit_roundtrip(s, out_fields, in_fields, &mut out)
                }
                ConformanceScenario::CheckpointResume(s) => {
                    audit_checkpoint_resume(adapter, s, supported, &mut out)
                }
            }
        }

        out.sort_by(|a, b| (&a.code, &a.message).cmp(&(&b.code, &b.message)));
        out.dedup_by(|a, b| a.code == b.code && a.message == b.message);
        out
    }
}

fn audit_roundtrip(
    s: &HandoffRoundtripScenario,
    out_fields: &[FieldMapping],
    in_fields: &[FieldMapping],
    out: &mut Vec<Diagnostic>,
) {
    let declared_pairs: BTreeSet<(&str, &str)> = out_fields
        .iter()
        .map(|f| (f.source.as_str(), f.target.as_str()))
        .collect();
    let claimed: BTreeSet<(&str, &str)> = s
        .applied_mapping
        .iter()
        .map(|f| (f.source.as_str(), f.target.as_str()))
        .collect();
    if claimed != declared_pairs {
        out.push(Diagnostic::new(
            "SOMA-ADAPT-0001",
            "applied mapping differs from the declared mapping",
        ));
        return;
    }
    let (mapped, losses) = apply_field_mapping(out_fields, &s.canonical_input);
    if losses.iter().any(|(r, _)| *r == "missing-required") {
        out.push(Diagnostic::new(
            "SOMA-ADAPT-0001",
            "required mapping source missing from canonical input",
        ));
        return;
    }
    if mapped != s.mapped_output {
        let mut d = Diagnostic::new(
            "SOMA-ADAPT-0001",
            "recorded mapped output differs from simulated application",
        );
        d.related.push(first_diff_key(&mapped, &s.mapped_output));
        out.push(d);
        return;
    }
    let (restored, restore_losses) = apply_field_mapping(in_fields, &mapped);
    let semantic_loss = restore_losses
        .iter()
        .find(|(r, _)| *r == "semantic-loss")
        .map(|(_, k)| k.clone());
    if semantic_loss.is_some() || restored != s.restored_canonical {
        let field =
            semantic_loss.unwrap_or_else(|| first_diff_key(&restored, &s.restored_canonical));
        // SOMA-CMP-0004 naming the EXACT dropped/altered mapping field.
        out.push(Diagnostic::new(
            "SOMA-CMP-0004",
            format!("semantic loss at mapping field {field:?}"),
        ));
        return;
    }
    let computed = super::canonical::canonical_digest(&serde_json::Value::Object(restored.clone()));
    if computed != s.semantic_digest.as_str() {
        out.push(Diagnostic::new(
            "SOMA-CMP-0004",
            "restored canonical semantic digest mismatch",
        ));
    }
}

fn audit_checkpoint_resume(
    adapter: &HarnessAdapter,
    s: &CheckpointResumeScenario,
    supported: &SupportedVersion,
    out: &mut Vec<Diagnostic>,
) {
    let env = &s.checkpoint_envelope;
    let tok = &s.resume_token;

    if s.adapter_id != adapter.id {
        out.push(Diagnostic::new(
            "SOMA-ADAPT-0001",
            "scenario does not bind to the container adapter",
        ));
    }
    let resume = s.resume_requested.unwrap_or(false);
    if resume && adapter.checkpoint_import != Some(true) {
        out.push(Diagnostic::new(
            "SOMA-ADAPT-0001",
            "resume requested without declared checkpointImport",
        ));
    }
    if !resume && adapter.checkpoint_export != Some(true) {
        out.push(Diagnostic::new(
            "SOMA-ADAPT-0001",
            "suspend scenario without declared checkpointExport",
        ));
    }

    // SOMA-RES-0003: every digest binding must hold.
    if tok.plan_digest != env.plan_digest
        || tok.suspension_digest != env.checkpoint_digest
        || tok.authority_snapshot_digest != env.authority_snapshot_digest
    {
        out.push(Diagnostic::new(
            "SOMA-RES-0003",
            "token digests do not bind the checkpoint",
        ));
    }
    for e in &s.evidence {
        if e.artifact_digest != env.checkpoint_digest {
            out.push(Diagnostic::new(
                "SOMA-RES-0003",
                "evidence artifact does not witness the checkpoint state",
            ));
        }
    }

    // SOMA-RES-0001: revision staleness.
    if tok.revision.as_deref() != Some(s.repo_revision.as_str()) {
        out.push(Diagnostic::new(
            "SOMA-RES-0001",
            "resume token revision is stale",
        ));
    }

    // SOMA-RES-0002: expired at issuance.
    let produced: Vec<&str> = s
        .evidence
        .iter()
        .filter_map(|e| e.produced_at.as_deref())
        .collect();
    if let (Some(exp), Some(min_produced)) = (tok.expiration.as_deref(), produced.iter().min())
        && min_produced >= &exp
    {
        out.push(Diagnostic::new(
            "SOMA-RES-0002",
            "resume token expired at issuance",
        ));
    }

    // SOMA-RES-0004: token version support.
    for v in [&tok.workflow_version, &tok.schema_version] {
        if let Ok(parsed) = super::types::SemVer::parse(v)
            && !parsed.is_compatible_with(supported)
        {
            out.push(Diagnostic::new(
                "SOMA-RES-0004",
                "resume token version unsupported",
            ));
            break;
        }
    }

    // SOMA-CMP-0001: scenario compatibility version.
    if let Ok(v) = super::types::SemVer::parse(&s.compatibility.version)
        && !v.is_compatible_with(supported)
    {
        out.push(Diagnostic::new(
            "SOMA-CMP-0001",
            "scenario compatibility unsupported",
        ));
    }
}
