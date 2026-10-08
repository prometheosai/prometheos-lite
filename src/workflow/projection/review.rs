//! Slice 3 §4 — review-report projection.
//!
//! Render is a pure function of `(WorkflowDefinition, ReviewFacts, policy)`.
//! Against-source verification fresh-renders from THE SAME inputs and
//! compares; the structural byte path checks envelope/schema shape only.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::harness::review::{ReviewIssueType, ReviewReport, ReviewSeverity};
use crate::work::soma_projection::RunKey;
use crate::workflow::evaluate::EvidenceBundle;
use crate::workflow::graph_gates::{HumanDecisionRecordV1, HumanVerdict};
use crate::workflow::projection::envelope;
use crate::workflow::projection::{
    PROJECTION_VERSION_V1, VersionedProjectionEnvelope, digest_of, source_digest_of,
    validated_source,
};
use crate::workflow::soma::Diagnostic;
use crate::workflow::soma::contracts::{EvidenceReference, WorkflowDefinition};
use crate::workflow::soma::types::Hex64;

pub const REVIEW_SCHEMA_VERSION: &str = "lite.review-report.v1";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ReviewDisclosurePolicy {
    #[serde(default)]
    pub authorized_targets: Vec<String>,
    #[serde(default)]
    pub count_authorization: Vec<String>,
}

impl ReviewDisclosurePolicy {
    pub fn normalize(&self) -> Self {
        let mut a = self.authorized_targets.clone();
        let mut c = self.count_authorization.clone();
        a.sort();
        a.dedup();
        c.sort();
        c.dedup();
        Self {
            authorized_targets: a,
            count_authorization: c,
        }
    }
}

/// Serializes `RunKey` via its typed wire spelling (RunKeyKind wire name);
/// the authoritative `RunKey` itself is not serde-derivable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RunKeyView {
    pub kind: String,
    pub id: String,
}

impl From<&RunKey> for RunKeyView {
    fn from(rk: &RunKey) -> Self {
        Self {
            kind: rk.kind.as_wire().to_string(),
            id: rk.id.clone(),
        }
    }
}

/// Scope carries only identity fields already present in authoritative data.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ReviewScope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_key: Option<RunKeyView>,
}

/// Slice-3 authoritative input: references to existing authoritative structs.
/// Never a semantic copy, never a parallel evidence ontology.
pub struct ReviewFacts<'a> {
    pub report: Option<&'a ReviewReport>,
    pub gates: &'a [HumanDecisionRecordV1],
    pub evidence_bundles: &'a [EvidenceBundle],
    pub scope: ReviewScope,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ReviewProjectionPayload {
    pub review_schema_version: String,
    pub disclosure_policy_digest: String,
    pub source: ReviewSourceIdentity,
    pub authority: ReviewAuthoritySummary,
    pub gates: Vec<ReviewGateView>,
    pub summary: ReviewSummaryView,
    pub issues: Vec<ReviewIssueView>,
    pub omissions: Vec<OmissionView>,
    pub disposition: DispositionView,
    pub evidence_references: Vec<EvidenceReferenceView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ReviewSourceIdentity {
    pub workflow_id: String,
    pub source_digest: String,
    pub schema_version: String,
    pub report_id: Option<String>,
    pub run_key: Option<RunKeyView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ReviewAuthoritySummary {
    pub reviewer_principals: Vec<PrincipalView>,
    pub review_channel: Option<String>,
    pub executor_class: Option<String>,
    pub repo_binding: Option<RepoBindingView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PrincipalView {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RepoBindingView {
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ReviewGateView {
    pub node_id: String,
    pub gate_kind: String,
    pub verdict: String,
    pub failure_class: Option<String>,
    pub basis_evidence_digest: Option<String>,
    pub provenance_state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ReviewSummaryView {
    pub total_issues: Option<usize>,
    pub by_type: BTreeMap<String, usize>,
    pub by_severity: BTreeMap<String, usize>,
    pub files_reviewed: Option<usize>,
    pub files_with_issues: Option<usize>,
    pub passed: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ReviewIssueView {
    pub issue_type: String,
    pub severity: String,
    pub file: Option<String>,
    pub line: Option<usize>,
    pub message: String,
    pub suggestion: Option<String>,
    pub rule_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct OmissionView {
    pub section: String,
    pub category: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DispositionView {
    pub status: String,
    pub review_required: bool,
    pub supported_by_evidence: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct EvidenceReferenceView {
    pub id: String,
    pub event_digest: Hex64,
    pub artifact_digest: Hex64,
    pub artifact_kind: String,
    pub produced_by: String,
    pub produced_at: Option<String>,
}

fn issue_type_name(t: ReviewIssueType) -> &'static str {
    match t {
        ReviewIssueType::Bug => "bug",
        ReviewIssueType::Security => "security",
        ReviewIssueType::Performance => "performance",
        ReviewIssueType::Maintainability => "maintainability",
        ReviewIssueType::TestGap => "test-gap",
        ReviewIssueType::Style => "style",
        ReviewIssueType::Documentation => "documentation",
        ReviewIssueType::ApiChange => "api-change",
        ReviewIssueType::DependencyChange => "dependency-change",
    }
}

fn severity_name(s: ReviewSeverity) -> &'static str {
    match s {
        ReviewSeverity::Info => "info",
        ReviewSeverity::Low => "low",
        ReviewSeverity::Medium => "medium",
        ReviewSeverity::High => "high",
        ReviewSeverity::Critical => "critical",
    }
}

fn severity_rank(s: ReviewSeverity) -> u8 {
    match s {
        ReviewSeverity::Critical => 0,
        ReviewSeverity::High => 1,
        ReviewSeverity::Medium => 2,
        ReviewSeverity::Low => 3,
        ReviewSeverity::Info => 4,
    }
}

fn gate_verdict_name(v: HumanVerdict) -> &'static str {
    match v {
        HumanVerdict::Approved => "approved",
        HumanVerdict::ChangesRequested => "changes_requested",
        HumanVerdict::Rejected => "rejected",
    }
}

fn render_issues(report: Option<&ReviewReport>) -> Result<Vec<ReviewIssueView>, Vec<Diagnostic>> {
    let Some(report) = report else {
        return Ok(Vec::new());
    };
    let mut out: Vec<(u8, ReviewIssueView)> = report
        .issues
        .iter()
        .map(|i| {
            (
                severity_rank(i.severity),
                ReviewIssueView {
                    issue_type: issue_type_name(i.issue_type).to_string(),
                    severity: severity_name(i.severity).to_string(),
                    file: i.file.clone(),
                    line: i.line,
                    message: i.message.clone(),
                    suggestion: i.suggestion.clone(),
                    rule_id: i.rule_id.clone(),
                },
            )
        })
        .collect();
    out.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.issue_type.cmp(&b.1.issue_type))
            .then(a.1.file.cmp(&b.1.file))
            .then(a.1.line.cmp(&b.1.line))
            .then(a.1.rule_id.cmp(&b.1.rule_id))
            .then(a.1.message.cmp(&b.1.message))
    });
    let issues: Vec<ReviewIssueView> = out.into_iter().map(|(_, v)| v).collect();
    for pair in issues.windows(2) {
        if pair[0] == pair[1] {
            return Err(vec![Diagnostic::new(
                "SOMA-CMP-0011",
                "duplicate review issue identity within the same report".to_string(),
            )]);
        }
    }
    Ok(issues)
}

fn render_gates(gates: &[HumanDecisionRecordV1]) -> Result<Vec<ReviewGateView>, Vec<Diagnostic>> {
    let mut rendered: Vec<ReviewGateView> = gates
        .iter()
        .map(|g| ReviewGateView {
            node_id: g.gate_node_id.clone(),
            gate_kind: "human".to_string(),
            verdict: gate_verdict_name(g.verdict).to_string(),
            failure_class: None,
            basis_evidence_digest: if g.basis_evidence_digest.is_empty() {
                None
            } else {
                Some(g.basis_evidence_digest.clone())
            },
            provenance_state: "source-record".to_string(),
        })
        .collect();
    rendered.sort_by(|a, b| {
        a.node_id
            .cmp(&b.node_id)
            .then(a.verdict.cmp(&b.verdict))
            .then(a.failure_class.cmp(&b.failure_class))
            .then(a.basis_evidence_digest.cmp(&b.basis_evidence_digest))
    });
    for pair in rendered.windows(2) {
        if pair[0] == pair[1] {
            return Err(vec![Diagnostic::new(
                "SOMA-CMP-0011",
                "duplicate gate record identity within the same run".to_string(),
            )]);
        }
    }
    Ok(rendered)
}

fn render_summary(report: Option<&ReviewReport>) -> ReviewSummaryView {
    let Some(report) = report else {
        return ReviewSummaryView {
            total_issues: None,
            by_type: BTreeMap::new(),
            by_severity: BTreeMap::new(),
            files_reviewed: None,
            files_with_issues: None,
            passed: None,
        };
    };
    let by_type = report
        .summary
        .by_type
        .iter()
        .map(|(k, v)| (issue_type_name(*k).to_string(), *v))
        .collect();
    let by_severity = report
        .summary
        .by_severity
        .iter()
        .map(|(k, v)| (severity_name(*k).to_string(), *v))
        .collect();
    ReviewSummaryView {
        total_issues: Some(report.summary.total_issues),
        by_type,
        by_severity,
        files_reviewed: Some(report.summary.files_reviewed),
        files_with_issues: Some(report.summary.files_with_issues),
        passed: Some(report.passed),
    }
}

fn render_authority(gates: &[HumanDecisionRecordV1]) -> ReviewAuthoritySummary {
    let mut seen: BTreeMap<String, PrincipalView> = BTreeMap::new();
    for g in gates {
        seen.entry(g.decided_by.clone()).or_insert(PrincipalView {
            kind: "human".to_string(),
            identity: Some(g.decided_by.clone()),
        });
    }
    ReviewAuthoritySummary {
        reviewer_principals: seen.into_values().collect(),
        review_channel: gates.first().map(|g| match g.channel {
            crate::workflow::graph_gates::ReviewChannel::CliInteractive => {
                "cli-interactive".to_string()
            }
            crate::workflow::graph_gates::ReviewChannel::ExternalSystem { .. } => {
                "external-system".to_string()
            }
        }),
        executor_class: Some("human-decision".to_string()),
        repo_binding: None,
    }
}

fn render_evidence_references(refs: &[EvidenceReference]) -> Vec<EvidenceReferenceView> {
    let mut out: Vec<EvidenceReferenceView> = refs
        .iter()
        .map(|r| EvidenceReferenceView {
            id: r.id.clone(),
            event_digest: r.event_digest.clone(),
            artifact_digest: r.artifact_digest.clone(),
            artifact_kind: r.artifact_kind.clone(),
            produced_by: r.produced_by.clone(),
            produced_at: r.produced_at.clone(),
        })
        .collect();
    out.sort_by(|a, b| {
        a.produced_at
            .cmp(&b.produced_at)
            .then(a.event_digest.as_str().cmp(b.event_digest.as_str()))
            .then(a.artifact_digest.as_str().cmp(b.artifact_digest.as_str()))
    });
    out
}

fn render_disposition(
    gates: &[HumanDecisionRecordV1],
    bundles: &[EvidenceBundle],
) -> Result<DispositionView, Vec<Diagnostic>> {
    if gates.is_empty() {
        return Ok(DispositionView {
            status: "unavailable".to_string(),
            review_required: false,
            supported_by_evidence: false,
        });
    }
    let has_evidence = bundles.iter().any(|b| {
        b.validation
            .as_ref()
            .map(|v| v.validation_passed)
            .unwrap_or(false)
    });
    if gates.iter().any(|g| g.verdict == HumanVerdict::Rejected) {
        return Ok(DispositionView {
            status: "reject".to_string(),
            review_required: false,
            supported_by_evidence: has_evidence,
        });
    }
    if gates
        .iter()
        .any(|g| g.verdict == HumanVerdict::ChangesRequested)
    {
        return Ok(DispositionView {
            status: "changes_required".to_string(),
            review_required: true,
            supported_by_evidence: has_evidence,
        });
    }
    if gates.iter().all(|g| g.verdict == HumanVerdict::Approved) {
        return Ok(DispositionView {
            status: "approve".to_string(),
            review_required: false,
            supported_by_evidence: has_evidence,
        });
    }
    Ok(DispositionView {
        status: "unavailable".to_string(),
        review_required: false,
        supported_by_evidence: has_evidence,
    })
}

fn push_omission(acc: &mut Vec<OmissionView>, section: &str, category: &str, reason: &str) {
    acc.push(OmissionView {
        section: section.to_string(),
        category: category.to_string(),
        reason: reason.to_string(),
    });
}

/// Applies the disclosure policy on top of schema-valid rendered output:
/// disallowed targets become `withheld` markers with their content
/// dropped from the rendered view and a corresponding `OmissionView`.
fn apply_policy(payload: &mut ReviewProjectionPayload, policy: &ReviewDisclosurePolicy) {
    let allow_principals = policy.authorized_targets.iter().any(|t| t == "principals");
    let mut omissions = std::mem::take(&mut payload.omissions);
    if !allow_principals {
        for principal in payload.authority.reviewer_principals.iter_mut() {
            principal.identity = None;
        }
        push_omission(&mut omissions, "principals", "withheld", "disclosurePolicy");
    }
    omissions.sort_by(|a, b| {
        a.section
            .cmp(&b.section)
            .then(a.category.cmp(&b.category))
            .then(a.reason.cmp(&b.reason))
    });
    payload.omissions = omissions;
}

fn collect_omissions(facts: &ReviewFacts<'_>) -> Vec<OmissionView> {
    let mut out = Vec::new();
    if facts.report.is_none() {
        push_omission(&mut out, "issues", "unavailable", "noAuthoritativeSource");
        push_omission(&mut out, "summary", "unavailable", "noAuthoritativeSource");
    }
    if facts.gates.is_empty() {
        push_omission(&mut out, "gates", "unavailable", "noAuthoritativeSource");
    }
    if facts.evidence_bundles.is_empty() {
        push_omission(&mut out, "evidence", "unavailable", "noAuthoritativeSource");
    }
    out.sort_by(|a, b| {
        a.section
            .cmp(&b.section)
            .then(a.category.cmp(&b.category))
            .then(a.reason.cmp(&b.reason))
    });
    out
}

fn build_payload(
    wf: &WorkflowDefinition,
    facts: &ReviewFacts<'_>,
    disclosure: &ReviewDisclosurePolicy,
) -> Result<ReviewProjectionPayload, Vec<Diagnostic>> {
    let issue_renders = render_issues(facts.report)?;
    let gates = render_gates(facts.gates)?;
    let summary = render_summary(facts.report);
    let authority = render_authority(facts.gates);
    let evidence_references = render_evidence_references(&wf.evidence);
    let omissions = collect_omissions(facts);
    let disposition = render_disposition(facts.gates, facts.evidence_bundles)?;

    let normalized = disclosure.normalize();
    let disclosure_policy_digest_preimage = serde_json::json!({
        "domain": "projection.review-report.policy.v1",
        "schemaVersion": REVIEW_SCHEMA_VERSION,
        "rootSourceDigest": source_digest_of(wf)?,
        "policy": serde_json::to_value(&normalized).map_err(|e| {
            vec![Diagnostic::new(
                "PROJ-0001",
                format!("disclosure policy cannot be serialized ({e})"),
            )]
        })?,
    });
    let disclosure_policy_digest = digest_of(&disclosure_policy_digest_preimage)?;

    Ok(ReviewProjectionPayload {
        review_schema_version: REVIEW_SCHEMA_VERSION.to_string(),
        disclosure_policy_digest,
        source: ReviewSourceIdentity {
            workflow_id: wf.id.clone(),
            source_digest: source_digest_of(wf)?,
            schema_version: wf.schema_version.clone(),
            report_id: facts.scope.report_id.clone(),
            run_key: facts.scope.run_key.clone(),
        },
        authority,
        gates,
        summary,
        issues: issue_renders,
        omissions,
        disposition,
        evidence_references,
    })
}

fn validate_disclosure_policy(
    payload: &ReviewProjectionPayload,
    policy: Option<&ReviewDisclosurePolicy>,
) -> Result<ReviewDisclosurePolicy, Vec<Diagnostic>> {
    let normalized = policy.cloned().unwrap_or_default().normalize();
    let expected_preimage = serde_json::json!({
        "domain": "projection.review-report.policy.v1",
        "schemaVersion": REVIEW_SCHEMA_VERSION,
        "rootSourceDigest": payload.source.source_digest,
        "policy": serde_json::to_value(&normalized).map_err(|e| {
            vec![Diagnostic::new(
                "PROJ-0001",
                format!("disclosure policy cannot be serialized ({e})"),
            )]
        })?,
    });
    let expected = digest_of(&expected_preimage)?;
    if expected != payload.disclosure_policy_digest {
        return Err(vec![Diagnostic::new(
            "SOMA-CMP-0004",
            "disclosure policy digest does not verify".to_string(),
        )]);
    }
    Ok(normalized)
}

/// §4: deterministic review-report renderer.
pub fn render_review_projection(
    wf: &WorkflowDefinition,
    facts: &ReviewFacts<'_>,
    policy: Option<&ReviewDisclosurePolicy>,
) -> Result<VersionedProjectionEnvelope<ReviewProjectionPayload>, Vec<Diagnostic>> {
    validated_source(wf)?;
    let source_digest = source_digest_of(wf)?;
    let normalized = policy.cloned().unwrap_or_default().normalize();
    let mut payload = build_payload(wf, facts, &normalized)?;
    apply_policy(&mut payload, &normalized);
    let payload_value = serde_json::to_value(&payload).map_err(|e| {
        vec![Diagnostic::new(
            "PROJ-0001",
            format!("review payload cannot be serialized ({e})"),
        )]
    })?;
    let projection_digest = digest_of(&payload_value)?;
    Ok(VersionedProjectionEnvelope {
        projection_version: PROJECTION_VERSION_V1.to_string(),
        schema_version: wf.schema_version.clone(),
        source_digest,
        projection_digest,
        payload,
    })
}

/// §7: structural byte verifier (source-independent; derived digests are
/// SHAPE-checked only — full recompute happens in the against-source path).
pub fn verify_review_projection_bytes(
    raw: &[u8],
) -> Result<VersionedProjectionEnvelope<ReviewProjectionPayload>, Vec<Diagnostic>> {
    let env = envelope::parse_envelope_bytes::<ReviewProjectionPayload>(raw)?;
    if env.payload.review_schema_version != REVIEW_SCHEMA_VERSION {
        return Err(vec![Diagnostic::new(
            "SOMA-CMP-0001",
            format!(
                "unsupported reviewSchemaVersion {}",
                env.payload.review_schema_version
            ),
        )]);
    }
    let payload_value = serde_json::to_value(&env.payload).map_err(|e| {
        vec![Diagnostic::new(
            "PROJ-0001",
            format!("review payload cannot be serialized ({e})"),
        )]
    })?;
    let expected = digest_of(&payload_value)?;
    if expected != env.projection_digest {
        return Err(vec![Diagnostic::new(
            "SOMA-CMP-0004",
            "review projection digest does not verify".to_string(),
        )]);
    }
    Ok(env)
}

/// §7: against-source verifier. Fresh-renders with the SAME authoritative
/// inputs the caller used to produce the candidate, then compares:
/// metadata bounds → policy re-normalization → fresh render → byte-level
/// identity. Any substituted fact flips the fresh render and fails closed.
pub fn verify_review_against_source(
    envelope: &VersionedProjectionEnvelope<serde_json::Value>,
    wf: &WorkflowDefinition,
    facts: &ReviewFacts<'_>,
    policy: Option<&ReviewDisclosurePolicy>,
) -> Result<(), Vec<Diagnostic>> {
    envelope::verify_envelope_metadata(envelope)?;
    if envelope.schema_version != wf.schema_version {
        return Err(vec![Diagnostic::new(
            "PROJ-0002",
            "schemaVersion does not match the source AST".to_string(),
        )]);
    }
    let mut source_value = serde_json::to_value(wf).map_err(|e| {
        vec![Diagnostic::new(
            "PROJ-0001",
            format!("workflow cannot be serialized for verification ({e})"),
        )]
    })?;
    if let Some(obj) = source_value.as_object_mut() {
        obj.remove("contentDigest");
    }
    let expected_source = digest_of(&source_value)?;
    if expected_source != envelope.source_digest {
        return Err(vec![Diagnostic::new(
            "PROJ-0002",
            "sourceDigest does not match the source AST".to_string(),
        )]);
    }
    let expected_payload = digest_of(&envelope.payload)?;
    if expected_payload != envelope.projection_digest {
        return Err(vec![Diagnostic::new(
            "SOMA-CMP-0004",
            "review projection digest does not verify".to_string(),
        )]);
    }
    // Derived `disclosurePolicyDigest` and derived-structure shape is
    // recomputed from the NORMALIZED policy under the §3.4.1 domain.
    let payload: ReviewProjectionPayload = serde_json::from_value(envelope.payload.clone())
        .map_err(|e| {
            vec![Diagnostic::new(
                "SOMA-CMP-0003",
                format!("review payload fails structural validation ({e})"),
            )]
        })?;
    let normalized = validate_disclosure_policy(&payload, policy)?;
    let fresh = render_review_projection(wf, facts, Some(&normalized))?;
    let fresh_value = serde_json::to_value(&fresh.payload).map_err(|e| {
        vec![Diagnostic::new(
            "PROJ-0001",
            format!("review payload cannot be serialized ({e})"),
        )]
    })?;
    if fresh.source_digest != envelope.source_digest || fresh_value != envelope.payload {
        return Err(vec![Diagnostic::new(
            "PROJ-0002",
            "review projection does not match a fresh render of the authoritative inputs"
                .to_string(),
        )]);
    }
    Ok(())
}
