//! Slice 3 §4 — review-report projection.
//!
//! Render is a pure function of `(WorkflowDefinition, ReviewFacts, policy)`.
//! Against-source verification fresh-renders from THE SAME inputs and
//! compares; the structural byte path checks envelope/schema/digest shape
//! only. Disclosure is a validated, fail-closed, non-cascading allow-list:
//! nothing sensitive is exposed unless policy authorizes it, and counts are
//! independently sensitive (§6.2).

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use crate::harness::review::{ReviewIssueType, ReviewReport, ReviewSeverity};
use crate::work::soma_projection::RunKey;
use crate::workflow::graph_gates::{HumanDecisionRecordV1, HumanVerdict, ReviewChannel};
use crate::workflow::projection::envelope;
use crate::workflow::projection::{
    PROJECTION_VERSION_V1, VersionedProjectionEnvelope, digest_of, source_digest_of,
    validated_source,
};
use crate::workflow::soma::Diagnostic;
use crate::workflow::soma::contracts::{EvidenceReference, WorkflowDefinition};
use crate::workflow::soma::types::Hex64;

pub const REVIEW_SCHEMA_VERSION: &str = "lite.review-report.v1";

/// Domain label for the review facts digest (§3.4.2).
pub const REVIEW_FACTS_DOMAIN: &str = "projection.review-report.facts.v1";
/// Domain label for the review policy digest (§3.4.1).
pub const REVIEW_POLICY_DOMAIN: &str = "projection.review-report.policy.v1";

/// Closed vocabulary of review disclosure targets (§6.1). An unknown target
/// fails closed with `PROJ-0003`.
pub const REVIEW_DISCLOSURE_TARGETS: &[&str] = &[
    "findings",
    "files",
    "lines",
    "messages",
    "rules",
    "predicates",
    "principals",
    "evidenceReferences",
];
/// Closed vocabulary of review count-authorization targets (§6.2).
pub const REVIEW_COUNT_TARGETS: &[&str] = &[
    "totalIssues",
    "byType",
    "bySeverity",
    "filesReviewed",
    "filesWithIssues",
    "passed",
];

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ReviewDisclosurePolicy {
    #[serde(default)]
    pub authorized_targets: Vec<String>,
    #[serde(default)]
    pub count_authorization: Vec<String>,
}

impl ReviewDisclosurePolicy {
    /// Sort + dedup only; validation is separate (`normalize_review_policy`).
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

    fn has_target(&self, target: &str) -> bool {
        self.authorized_targets.iter().any(|t| t == target)
    }

    fn has_count(&self, target: &str) -> bool {
        self.count_authorization.iter().any(|t| t == target)
    }
}

fn hex64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// §6.1: validate both target lists against the closed vocabularies, then
/// normalize. Unknown target ⇒ `PROJ-0003`.
pub fn normalize_review_policy(
    policy: Option<&ReviewDisclosurePolicy>,
) -> Result<ReviewDisclosurePolicy, Vec<Diagnostic>> {
    let raw = policy.cloned().unwrap_or_default();
    let mut diags: Vec<Diagnostic> = Vec::new();
    for t in &raw.authorized_targets {
        if !REVIEW_DISCLOSURE_TARGETS.contains(&t.as_str()) {
            diags.push(Diagnostic::new(
                "PROJ-0003",
                format!("unknown review disclosure target: {t}"),
            ));
        }
    }
    for t in &raw.count_authorization {
        if !REVIEW_COUNT_TARGETS.contains(&t.as_str()) {
            diags.push(Diagnostic::new(
                "PROJ-0003",
                format!("unknown review count-authorization target: {t}"),
            ));
        }
    }
    if !diags.is_empty() {
        diags.sort_by(|a, b| a.code.cmp(&b.code).then_with(|| a.message.cmp(&b.message)));
        diags.dedup_by(|a, b| a.code == b.code && a.message == b.message);
        return Err(diags);
    }
    Ok(raw.normalize())
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
    /// Explicit borrowed authoritative evidence references (item 6: evidence
    /// attribution comes from here, never from the workflow AST).
    pub evidence_references: &'a [EvidenceReference],
    pub scope: ReviewScope,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ReviewProjectionPayload {
    pub review_schema_version: String,
    pub disclosure_policy_digest: String,
    pub report_reference_digest: String,
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
    /// The single distinct channel when uniform; `None` when absent or mixed.
    pub review_channel: Option<String>,
    /// Every distinct channel label observed, sorted (item 6: heterogeneous
    /// channels are represented, never silently collapsed to the first).
    pub review_channels: Vec<String>,
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

fn channel_label(c: &ReviewChannel) -> String {
    match c {
        ReviewChannel::CliInteractive => "cli-interactive".to_string(),
        ReviewChannel::ExternalSystem { system_id } => format!("external-system:{system_id}"),
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
    let mut channels: BTreeSet<String> = BTreeSet::new();
    for g in gates {
        seen.entry(g.decided_by.clone()).or_insert(PrincipalView {
            kind: "human".to_string(),
            identity: Some(g.decided_by.clone()),
        });
        channels.insert(channel_label(&g.channel));
    }
    let review_channels: Vec<String> = channels.iter().cloned().collect();
    // The single distinct channel, or `None` when absent or heterogeneous;
    // `review_channels` always carries the complete set (item 6).
    let review_channel = if review_channels.len() == 1 {
        review_channels.first().cloned()
    } else {
        None
    };
    ReviewAuthoritySummary {
        reviewer_principals: seen.into_values().collect(),
        review_channel,
        review_channels,
        // No authoritative source field carries an executor class; never claim
        // one (item 6: remove hardcoded authority claims).
        executor_class: None,
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
            .as_deref()
            .unwrap_or("")
            .cmp(b.produced_at.as_deref().unwrap_or(""))
            .then(a.event_digest.as_str().cmp(b.event_digest.as_str()))
            .then(a.artifact_digest.as_str().cmp(b.artifact_digest.as_str()))
    });
    out
}

/// Row 14 / item 6: a gate's `basisEvidenceDigest` is the digest of the
/// REVIEWED ARTIFACT (`graph_gates.rs`: "Digest of the evidence artifact the
/// human reviewed"). Resolved against `EvidenceReference.artifactDigest` — never
/// `eventDigest`. A dangling (non-empty) basis that references no present
/// authoritative artifact digest fails closed (`SOMA-CMP-0004`).
fn validate_gate_basis(facts: &ReviewFacts<'_>) -> Result<(), Vec<Diagnostic>> {
    let refs: BTreeSet<&str> = facts
        .evidence_references
        .iter()
        .map(|r| r.artifact_digest.as_str())
        .collect();
    for g in facts.gates {
        let basis = g.basis_evidence_digest.trim();
        if !basis.is_empty() && !refs.contains(basis) {
            return Err(vec![Diagnostic::new(
                "SOMA-CMP-0004",
                format!(
                    "gate {} basis evidence digest does not reference a present evidence artifact",
                    g.gate_node_id
                ),
            )]);
        }
    }
    Ok(())
}

/// `supportedByEvidence` is bound to the gate's artifact-basis evidence: at
/// least one gate carries a non-empty `basisEvidenceDigest` and every
/// non-empty one matches an authoritative `EvidenceReference.artifactDigest`.
fn supported_by_gate_basis(facts: &ReviewFacts<'_>) -> bool {
    let refs: BTreeSet<&str> = facts
        .evidence_references
        .iter()
        .map(|r| r.artifact_digest.as_str())
        .collect();
    let mut any = false;
    for g in facts.gates {
        let basis = g.basis_evidence_digest.trim();
        if basis.is_empty() {
            continue;
        }
        any = true;
        if !refs.contains(basis) {
            return false;
        }
    }
    any
}

fn render_disposition(gates: &[HumanDecisionRecordV1], supported: bool) -> DispositionView {
    if gates.is_empty() {
        return DispositionView {
            status: "unavailable".to_string(),
            review_required: false,
            supported_by_evidence: false,
        };
    }
    let status = if gates.iter().any(|g| g.verdict == HumanVerdict::Rejected) {
        "reject"
    } else if gates
        .iter()
        .any(|g| g.verdict == HumanVerdict::ChangesRequested)
    {
        "changes_required"
    } else if gates.iter().all(|g| g.verdict == HumanVerdict::Approved) {
        "approve"
    } else {
        "unavailable"
    };
    DispositionView {
        status: status.to_string(),
        review_required: status == "changes_required",
        supported_by_evidence: supported,
    }
}

fn push_omission(acc: &mut Vec<OmissionView>, section: &str, category: &str, reason: &str) {
    acc.push(OmissionView {
        section: section.to_string(),
        category: category.to_string(),
        reason: reason.to_string(),
    });
}

fn sort_omissions(omissions: &mut Vec<OmissionView>) {
    omissions.sort_by(|a, b| {
        a.section
            .cmp(&b.section)
            .then(a.category.cmp(&b.category))
            .then(a.reason.cmp(&b.reason))
    });
    omissions.dedup();
}

fn collect_unavailable_omissions(facts: &ReviewFacts<'_>, acc: &mut Vec<OmissionView>) {
    if facts.report.is_none() {
        push_omission(acc, "issues", "unavailable", "noAuthoritativeSource");
        push_omission(acc, "summary", "unavailable", "noAuthoritativeSource");
    }
    if facts.gates.is_empty() {
        push_omission(acc, "gates", "unavailable", "noAuthoritativeSource");
        push_omission(acc, "authority", "unavailable", "noAuthoritativeSource");
    }
    if facts.evidence_references.is_empty() {
        push_omission(acc, "evidence", "unavailable", "noAuthoritativeSource");
    }
}

/// Apply the normalized disclosure policy on top of the raw render: each
/// withheld target has its content removed and an omission appended; counts
/// are gated independently through `countAuthorization` (§6.2).
fn apply_policy(payload: &mut ReviewProjectionPayload, policy: &ReviewDisclosurePolicy) {
    let mut omissions = std::mem::take(&mut payload.omissions);

    if !policy.has_target("findings") {
        if !payload.issues.is_empty() {
            payload.issues.clear();
            push_omission(&mut omissions, "issues", "withheld", "disclosurePolicy");
        }
    } else {
        let allow_files = policy.has_target("files");
        let allow_lines = policy.has_target("lines");
        let allow_messages = policy.has_target("messages");
        let allow_rules = policy.has_target("rules");
        let allow_predicates = policy.has_target("predicates");
        let mut had_file = false;
        let mut had_line = false;
        let mut had_message = false;
        let mut had_rule = false;
        let mut had_predicate = false;
        for issue in payload.issues.iter_mut() {
            if !allow_files && issue.file.is_some() {
                had_file = true;
                issue.file = None;
            }
            if !allow_lines && issue.line.is_some() {
                had_line = true;
                issue.line = None;
            }
            if !allow_messages && (!issue.message.is_empty() || issue.suggestion.is_some()) {
                had_message = true;
                issue.message = String::new();
                issue.suggestion = None;
            }
            if !allow_rules && !issue.rule_id.is_empty() {
                had_rule = true;
                issue.rule_id = String::new();
            }
            if !allow_predicates && (!issue.issue_type.is_empty() || !issue.severity.is_empty()) {
                had_predicate = true;
                issue.issue_type = String::new();
                issue.severity = String::new();
            }
        }
        if had_file {
            push_omission(&mut omissions, "files", "withheld", "disclosurePolicy");
        }
        if had_line {
            push_omission(&mut omissions, "lines", "withheld", "disclosurePolicy");
        }
        if had_message {
            push_omission(&mut omissions, "messages", "withheld", "disclosurePolicy");
        }
        if had_rule {
            push_omission(&mut omissions, "rules", "withheld", "disclosurePolicy");
        }
        if had_predicate {
            push_omission(&mut omissions, "predicates", "withheld", "disclosurePolicy");
        }
    }

    if !policy.has_target("principals") {
        let mut had = false;
        for principal in payload.authority.reviewer_principals.iter_mut() {
            if principal.identity.is_some() {
                had = true;
                principal.identity = None;
            }
        }
        if had {
            push_omission(&mut omissions, "principals", "withheld", "disclosurePolicy");
        }
    }

    if !policy.has_target("evidenceReferences") && !payload.evidence_references.is_empty() {
        payload.evidence_references.clear();
        push_omission(&mut omissions, "evidence", "withheld", "disclosurePolicy");
    }

    let mut counts_withheld = false;
    if !policy.has_count("totalIssues") && payload.summary.total_issues.is_some() {
        payload.summary.total_issues = None;
        counts_withheld = true;
    }
    if !policy.has_count("byType") && !payload.summary.by_type.is_empty() {
        payload.summary.by_type.clear();
        counts_withheld = true;
    }
    if !policy.has_count("bySeverity") && !payload.summary.by_severity.is_empty() {
        payload.summary.by_severity.clear();
        counts_withheld = true;
    }
    if !policy.has_count("filesReviewed") && payload.summary.files_reviewed.is_some() {
        payload.summary.files_reviewed = None;
        counts_withheld = true;
    }
    if !policy.has_count("filesWithIssues") && payload.summary.files_with_issues.is_some() {
        payload.summary.files_with_issues = None;
        counts_withheld = true;
    }
    if !policy.has_count("passed") && payload.summary.passed.is_some() {
        payload.summary.passed = None;
        counts_withheld = true;
    }
    if counts_withheld {
        push_omission(&mut omissions, "summary", "withheld", "disclosurePolicy");
    }

    sort_omissions(&mut omissions);
    payload.omissions = omissions;
}

/// §3.4.2: `reportReferenceDigest` preimage over the authoritative facts.
fn report_reference_digest(
    wf: &WorkflowDefinition,
    facts: &ReviewFacts<'_>,
) -> Result<String, Vec<Diagnostic>> {
    let mut gate_ids: Vec<String> = facts.gates.iter().map(|g| g.gate_node_id.clone()).collect();
    gate_ids.sort();
    gate_ids.dedup();
    let mut evidence_ref_ids: Vec<String> = facts
        .evidence_references
        .iter()
        .map(|r| r.id.clone())
        .collect();
    evidence_ref_ids.sort();
    evidence_ref_ids.dedup();
    let preimage = serde_json::json!({
        "domain": REVIEW_FACTS_DOMAIN,
        "rootSourceDigest": source_digest_of(wf)?,
        "reviewSchemaVersion": REVIEW_SCHEMA_VERSION,
        "facts": {
            "report": facts.report.is_some(),
            "gates": !facts.gates.is_empty(),
            "evidence": !facts.evidence_references.is_empty(),
            "gateIds": gate_ids,
            "evidenceRefIds": evidence_ref_ids,
        },
    });
    digest_of(&preimage)
}

fn policy_digest(
    root_source_digest: &str,
    policy: &ReviewDisclosurePolicy,
) -> Result<String, Vec<Diagnostic>> {
    let preimage = serde_json::json!({
        "domain": REVIEW_POLICY_DOMAIN,
        "schemaVersion": REVIEW_SCHEMA_VERSION,
        "rootSourceDigest": root_source_digest,
        "policy": serde_json::to_value(policy).map_err(|e| {
            vec![Diagnostic::new(
                "PROJ-0001",
                format!("disclosure policy cannot be serialized ({e})"),
            )]
        })?,
    });
    digest_of(&preimage)
}

fn build_payload(
    wf: &WorkflowDefinition,
    facts: &ReviewFacts<'_>,
    disclosure: &ReviewDisclosurePolicy,
) -> Result<ReviewProjectionPayload, Vec<Diagnostic>> {
    let source_digest = source_digest_of(wf)?;
    let issues = render_issues(facts.report)?;
    let mut omissions = Vec::new();
    let gates = render_gates(facts.gates)?;
    let summary = render_summary(facts.report);
    let authority = render_authority(facts.gates);
    let evidence_references = render_evidence_references(facts.evidence_references);
    collect_unavailable_omissions(facts, &mut omissions);
    sort_omissions(&mut omissions);
    let disposition = render_disposition(facts.gates, supported_by_gate_basis(facts));

    Ok(ReviewProjectionPayload {
        review_schema_version: REVIEW_SCHEMA_VERSION.to_string(),
        disclosure_policy_digest: policy_digest(&source_digest, disclosure)?,
        report_reference_digest: report_reference_digest(wf, facts)?,
        source: ReviewSourceIdentity {
            workflow_id: wf.id.clone(),
            source_digest,
            schema_version: wf.schema_version.clone(),
            report_id: facts.scope.report_id.clone(),
            run_key: facts.scope.run_key.clone(),
        },
        authority,
        gates,
        summary,
        issues,
        omissions,
        disposition,
        evidence_references,
    })
}

/// §4: deterministic review-report renderer.
pub fn render_review_projection(
    wf: &WorkflowDefinition,
    facts: &ReviewFacts<'_>,
    policy: Option<&ReviewDisclosurePolicy>,
) -> Result<VersionedProjectionEnvelope<ReviewProjectionPayload>, Vec<Diagnostic>> {
    validated_source(wf)?;
    validate_gate_basis(facts)?;
    let source_digest = source_digest_of(wf)?;
    let normalized = normalize_review_policy(policy)?;
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
    for (label, digest) in [
        (
            "disclosurePolicyDigest",
            &env.payload.disclosure_policy_digest,
        ),
        (
            "reportReferenceDigest",
            &env.payload.report_reference_digest,
        ),
    ] {
        if !hex64(digest) {
            return Err(vec![Diagnostic::new(
                "PROJ-0002",
                format!("{label} is not a lowercase 64-hex digest"),
            )]);
        }
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
/// metadata bounds → sourceDigest → projectionDigest → policy digest →
/// facts digest → fresh render. Any substituted fact flips the fresh render
/// and fails closed.
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
    let expected_source = source_digest_of(wf)?;
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
    let payload: ReviewProjectionPayload = serde_json::from_value(envelope.payload.clone())
        .map_err(|e| {
            vec![Diagnostic::new(
                "SOMA-CMP-0003",
                format!("review payload fails structural validation ({e})"),
            )]
        })?;
    let normalized = normalize_review_policy(policy)?;
    let expected_policy = policy_digest(&expected_source, &normalized)?;
    if expected_policy != payload.disclosure_policy_digest {
        return Err(vec![Diagnostic::new(
            "SOMA-CMP-0004",
            "disclosure policy digest does not verify".to_string(),
        )]);
    }
    let expected_facts = report_reference_digest(wf, facts)?;
    if expected_facts != payload.report_reference_digest {
        return Err(vec![Diagnostic::new(
            "SOMA-CMP-0004",
            "report reference digest does not verify".to_string(),
        )]);
    }
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
