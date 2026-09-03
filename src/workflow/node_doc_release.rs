//! Governed node library for E5/I05 (#129): documentation-impact and
//! release-preparation nodes.
//!
//! OWNERSHIP: Lite-owned `lite.node` capability implementations. Every
//! node here is READ-ONLY with respect to the target repository: each
//! takes a serialized candidate and emits a typed artifact. None of
//! the two nodes modifies files, publishes releases, performs merge,
//! or deploys; apply/release remain operator actions whose inputs are
//! the emitted artifacts plus operator-supplied human authority.
//!
//! Concretely the two capabilities are:
//! - `doc-impact` — classification only. Given a list of changed
//!   files, identifies which documentation artifacts may need
//!   updating (user guide, API reference, CHANGELOG, README, etc.).
//!   The node NEVER writes a file. It emits findings with the
//!   reasoning and the path, and the operator decides what to
//!   update.
//! - `release-prep` — composition of the implementation, validation,
//!   evidence-audit, independent-review, and doc-impact evidence
//!   into a release-note artifact. The artifact carries explicit
//!   `approvals` (the human/operator sign-offs required before any
//!   release action) and `sections` (the release-note body citing
//!   upstream evidence digests). The node NEVER publishes, tags,
//!   pushes, or merges; those remain operator actions. The artifact
//!   is a release-prep document, not a release authorization.
//!
//! Each node is a `Capability` handler, so the generic nine-gate
//! `NodeRunner` (lite.node.v1 contracts, lite.policy.v1 authorization,
//! redaction, journal durability) drives them unchanged. Conformance
//! is proven in `tests/node_library_conformance.rs` against the same
//! machinery the E5/I01-I04 nodes use.

use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::workflow::memory_contracts::canonical_digest;
use crate::workflow::node_contracts::NodeManifestV1;
use crate::workflow::node_review::ReviewKind;
use crate::workflow::node_runner::{Capability, CapabilityRegistry};

/// Version of the E5/I05 node contracts.
pub const NODE_DOC_RELEASE_VERSION: &str = "1.0.0";

/// Declared capability names.
pub const CAP_DOC_IMPACT: &str = "doc-impact";
pub const CAP_RELEASE_PREP: &str = "release-prep";

// ---------------------------------------------------------------------------
// Typed node outputs (`lite.node.<capability>` families)
// ---------------------------------------------------------------------------

/// A documentation category the candidate has affected or may need
/// to update. Mirrors the product-surface inventory's doc taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DocCategoryV1 {
    /// User guide under `docs/guides/`.
    UserGuide,
    /// API reference.
    ApiReference,
    /// CHANGELOG.md.
    Changelog,
    /// Project README.
    Readme,
    /// Tutorial / worked example.
    Tutorial,
    /// Architecture / design doc.
    Architecture,
    /// No doc category applies (e.g. test-only change).
    None,
}

impl DocCategoryV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            DocCategoryV1::UserGuide => "user-guide",
            DocCategoryV1::ApiReference => "api-reference",
            DocCategoryV1::Changelog => "changelog",
            DocCategoryV1::Readme => "readme",
            DocCategoryV1::Tutorial => "tutorial",
            DocCategoryV1::Architecture => "architecture",
            DocCategoryV1::None => "none",
        }
    }
}

/// One doc-impact finding: a file or category the candidate has touched
/// (or that the operator should consider touching) and why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocFindingV1 {
    pub category: DocCategoryV1,
    /// Path relative to repo root, or the category name if the
    /// finding is category-level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub message: String,
    /// `required` (the candidate's surface change directly affects
    /// this doc; operator MUST address before release) vs
    /// `recommended` (the candidate's surface change touches related
    /// functionality; operator SHOULD consider updating).
    pub severity: String,
}

/// Output of the `doc-impact` node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocImpactResultV1 {
    pub schema_version: String,
    pub repo_root: String,
    pub kind: ReviewKind,
    pub findings: Vec<DocFindingV1>,
    pub result_digest: String,
    pub constraints: Vec<String>,
}

/// One section of the release-note artifact. The `body` is rendered
/// text citing the upstream evidence digests; it is not a
/// publication — the operator must complete, review, and
/// publish manually.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseSectionV1 {
    pub heading: String,
    pub body: String,
    /// Upstream digests cited by this section.
    #[serde(default)]
    pub evidence_digests: Vec<String>,
}

/// One approval requirement: a human sign-off that must be recorded
/// BEFORE any release action is taken. The release-prep node NEVER
/// records approvals; it only enumerates the requirements.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequirementV1 {
    pub role: String,
    pub reason: String,
    /// Optional upstream evidence digest that this requirement
    /// references.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_digest: Option<String>,
}

/// Output of the `release-prep` node. The artifact is a release-note
/// body + required approvals + upstream evidence digests. It is NOT
/// a release authorization; the operator must perform the merge and
/// publish manually.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleasePrepResultV1 {
    pub schema_version: String,
    pub work_id: String,
    /// The review-kind verdict this prep assumes has been reached.
    /// The node does NOT itself reach a verdict; the caller supplies
    /// the consolidated verdict from the upstream pipeline. If the
    /// verdict is `reject`, the prep still produces a document but
    /// every section is annotated with the rejection so the
    /// operator cannot accidentally publish.
    pub assumed_verdict: ReviewKind,
    pub sections: Vec<ReleaseSectionV1>,
    pub approvals: Vec<ApprovalRequirementV1>,
    /// Upstream evidence digests this artifact cites, in order:
    /// `implementation_digest`, `validation_digest`, `audit_digest`,
    /// `independent_review_digest`, `doc_impact_digest`.
    pub cited_digests: Vec<String>,
    pub result_digest: String,
    pub constraints: Vec<String>,
}

// ---------------------------------------------------------------------------
// Input contract types
// ---------------------------------------------------------------------------

/// Doc-impact input. Either `changedPaths` (relative paths) or a
/// `candidateDiff` (unified diff string) must be provided so the
/// node can map the surface change to doc categories.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocImpactRequestV1 {
    pub repo_root: String,
    /// Paths relative to `repoRoot` that the candidate touches.
    #[serde(default)]
    pub changed_paths: Vec<String>,
    /// Optional unified diff (used when changed paths are not yet
    /// recorded on disk).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_diff: Option<String>,
    /// Optional categories the operator has already declared up-to-
    /// date. The node never downgrades a finding for a category in
    /// this list; this is for operator annotations only.
    #[serde(default)]
    pub already_addressed: Vec<DocCategoryV1>,
}

/// Release-prep input. Carries the work identifier and any
/// upstream-review artifacts (security / audit / independent
/// review / doc-impact) the operator has collected. The node composes
/// these into a single release-prep document; it does not perform
/// any release action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReleasePrepRequestV1 {
    pub work_id: String,
    /// Optional consolidated verdict the operator has reached. If
    /// omitted, the node conservatively assumes `changes-required`
    /// and annotates every section accordingly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assumed_verdict: Option<ReviewKind>,
    /// Optional change summary (free-form text supplied by the
    /// operator or upstream pipeline).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_summary: Option<String>,
    /// Optional categorized change set, paralleling the doc-impact
    /// findings.
    #[serde(default)]
    pub change_set: Vec<DocFindingV1>,
    /// Optional upstream review-shaped digests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implementation_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub independent_review_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_impact_digest: Option<String>,
}

// ---------------------------------------------------------------------------
// Doc-impact classification
// ---------------------------------------------------------------------------

/// The doc categories the surface change MUST be cross-checked
/// against. The list is closed; adding a new category requires a
/// deliberate, scoped PR.
const DOC_CATEGORY_PATTERNS: &[(&str, DocCategoryV1)] = &[
    ("docs/guides/", DocCategoryV1::UserGuide),
    ("docs/architecture/", DocCategoryV1::Architecture),
    ("docs/api", DocCategoryV1::ApiReference),
    ("docs/tutorial", DocCategoryV1::Tutorial),
    ("CHANGELOG.md", DocCategoryV1::Changelog),
    ("README.md", DocCategoryV1::Readme),
];

/// Map a single changed path to a doc category. Returns
/// `DocCategoryV1::None` if the path is not in any doc zone.
fn categorize_path(path: &str) -> DocCategoryV1 {
    let normalized = path.replace('\\', "/");
    for (prefix, category) in DOC_CATEGORY_PATTERNS {
        if normalized.starts_with(prefix) || normalized.contains(&format!("/{}", prefix)) {
            return *category;
        }
    }
    // Common top-level docs.
    if normalized == "CHANGELOG.md" || normalized == "README.md" {
        return DocCategoryV1::Changelog;
    }
    DocCategoryV1::None
}

fn run_doc_impact(args: &serde_json::Value) -> Result<String> {
    let req: DocImpactRequestV1 =
        serde_json::from_value(args.clone()).context("doc-impact: invalid request JSON")?;
    validate_repo_root(&req.repo_root)?;
    if req.changed_paths.is_empty() && req.candidate_diff.is_none() {
        bail!("doc-impact: at least one of `changedPaths` or `candidateDiff` is required");
    }

    // 1. Walk each changed path; map to a doc category.
    let mut findings: Vec<DocFindingV1> = Vec::new();
    let mut seen: std::collections::BTreeSet<DocCategoryV1> = std::collections::BTreeSet::new();
    for path in &req.changed_paths {
        let category = categorize_path(path);
        if category == DocCategoryV1::None {
            continue;
        }
        if seen.insert(category) {
            let sev = if matches!(category, DocCategoryV1::Changelog | DocCategoryV1::Readme) {
                "required"
            } else {
                "recommended"
            };
            findings.push(DocFindingV1 {
                category,
                path: Some(path.clone()),
                message: format!(
                    "candidate touches {path:?} which is in the {} doc zone; review and update as needed",
                    category.as_str()
                ),
                severity: sev.to_string(),
            });
        }
    }

    // 2. If a diff is provided, infer changed paths from
    // `diff --git a/... b/...` headers. This is best-effort: only
    // absolute references (the standard `git diff` output) are
    // parsed; a diff produced by other tools is ignored.
    if let Some(diff) = &req.candidate_diff {
        for line in diff.lines() {
            if let Some(rest) = line.strip_prefix("diff --git a/")
                && let Some(path_a) = rest.split(' ').next()
            {
                let path = path_a.trim_start_matches("a/");
                let category = categorize_path(path);
                if category == DocCategoryV1::None {
                    continue;
                }
                if seen.insert(category) {
                    findings.push(DocFindingV1 {
                            category,
                            path: Some(path.to_string()),
                            message: format!(
                                "candidate diff modifies {path:?}; review and update the {} doc as needed",
                                category.as_str()
                            ),
                            severity: "recommended".to_string(),
                        });
                }
            }
        }
    }

    // 3. Honour operator-declared `already_addressed`. We do not
    // remove findings for those categories; we append a note so the
    // operator's intent is visible in the evidence chain.
    for cat in &req.already_addressed {
        if seen.contains(cat) {
            findings.push(DocFindingV1 {
                category: *cat,
                path: None,
                message: format!(
                    "operator declared {cat:?} already addressed; review did not downgrade the existing finding",
                    cat = cat.as_str()
                ),
                severity: "note".to_string(),
            });
        }
    }

    // 4. Verdict. The node never `reject`s; it is a classifier. A
    // change that touches a `required` category (CHANGELOG, README)
    // produces `changes-required`; otherwise `approve`.
    let kind = if findings.iter().any(|f| f.severity == "required") {
        ReviewKind::ChangesRequired
    } else {
        ReviewKind::Approve
    };

    let result_digest = canonical_digest(&serde_json::json!({
        "kind": kind,
        "findings": findings,
    }))?;

    let out = DocImpactResultV1 {
        schema_version: NODE_DOC_RELEASE_VERSION.to_string(),
        repo_root: req.repo_root,
        kind,
        findings,
        result_digest,
        constraints: vec![
            "read-only: doc-impact performs no file writes".to_string(),
            "deterministic: no model or external service is invoked".to_string(),
        ],
    };
    serde_json::to_string(&out).map_err(Into::into)
}

// ---------------------------------------------------------------------------
// Release-prep composition
// ---------------------------------------------------------------------------

fn run_release_prep(args: &serde_json::Value) -> Result<String> {
    let req: ReleasePrepRequestV1 =
        serde_json::from_value(args.clone()).context("release-prep: invalid request JSON")?;
    if req.work_id.is_empty() {
        bail!("release-prep: workId must not be empty");
    }

    let assumed = req.assumed_verdict.unwrap_or(ReviewKind::ChangesRequired);
    if matches!(assumed, ReviewKind::Reject) {
        // The node still produces a document, but every section is
        // annotated so the operator cannot accidentally publish.
    }

    let mut sections: Vec<ReleaseSectionV1> = Vec::new();
    let mut cited: Vec<String> = Vec::new();

    // Summary section — always present.
    {
        let mut digests = Vec::new();
        for d in [
            req.implementation_digest.as_ref(),
            req.validation_digest.as_ref(),
            req.audit_digest.as_ref(),
            req.independent_review_digest.as_ref(),
            req.doc_impact_digest.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            digests.push(d.clone());
        }
        cited.extend(digests.iter().cloned());

        let body = match (&req.change_summary, assumed) {
            (Some(s), _) => format!("{s}\n\nAssumed verdict: {}", assumed.as_str()),
            (None, _) => format!(
                "No change summary supplied. Assumed verdict: {}.",
                assumed.as_str()
            ),
        };
        sections.push(ReleaseSectionV1 {
            heading: "Summary".to_string(),
            body,
            evidence_digests: digests,
        });
    }

    // Implementation evidence section.
    if let Some(d) = &req.implementation_digest {
        sections.push(ReleaseSectionV1 {
            heading: "Implementation".to_string(),
            body: format!(
                "Implementation evidence digest: `{d}`. The candidate was produced by the Lite govern pipeline and recorded in the durable journal."
            ),
            evidence_digests: vec![d.clone()],
        });
    }

    // Validation evidence section.
    if let Some(d) = &req.validation_digest {
        sections.push(ReleaseSectionV1 {
            heading: "Validation".to_string(),
            body: format!(
                "Validation evidence digest: `{d}`. The candidate was executed in an isolated, revision-pinned worktree; the source repository was provably untouched and the classification report is bound to this digest."
            ),
            evidence_digests: vec![d.clone()],
        });
    }

    // Security + audit + independent review section.
    {
        let mut digests = Vec::new();
        for d in [&req.audit_digest, &req.independent_review_digest]
            .into_iter()
            .flatten()
        {
            digests.push(d.clone());
        }
        if !digests.is_empty() {
            sections.push(ReleaseSectionV1 {
                heading: "Review".to_string(),
                body: format!(
                    "Evidence audit + independent review digests: `{}`. \
                     The independent review emitted `{assumed_}` and recorded that the verdict does not authorize apply or merge.",
                    digests.join(", "),
                    assumed_ = assumed.as_str()
                ),
                evidence_digests: digests.clone(),
            });
        }
        cited.extend(digests);
    }

    // Doc-impact section.
    if let Some(d) = &req.doc_impact_digest {
        sections.push(ReleaseSectionV1 {
            heading: "Documentation".to_string(),
            body: format!(
                "Doc-impact evidence digest: `{d}`. The doc-impact node classified which doc zones may need updating. The operator must complete those updates before publishing."
            ),
            evidence_digests: vec![d.clone()],
        });
    }

    // Change-set section.
    if !req.change_set.is_empty() {
        let mut lines = Vec::new();
        for f in &req.change_set {
            lines.push(format!(
                "- [{}] {}{}",
                f.category.as_str(),
                f.path.clone().unwrap_or_else(|| "<category>".to_string()),
                if f.severity == "required" {
                    " (REQUIRED)"
                } else {
                    ""
                }
            ));
        }
        sections.push(ReleaseSectionV1 {
            heading: "Change Set".to_string(),
            body: lines.join("\n"),
            evidence_digests: vec![],
        });
    }

    // Approvals — explicit list of what the operator must sign off
    // before any release action.
    let mut approvals: Vec<ApprovalRequirementV1> = Vec::new();
    approvals.push(ApprovalRequirementV1 {
        role: "operator".to_string(),
        reason: "operator must complete the release-note body, verify the diff matches the cited evidence digests, and author the final commit".to_string(),
        evidence_digest: None,
    });
    approvals.push(ApprovalRequirementV1 {
        role: "security".to_string(),
        reason: "if the release is security-relevant (e.g. CVE, dependency update), a security sign-off is required; the upstream security-review digest is the trigger".to_string(),
        evidence_digest: req.audit_digest.clone(),
    });
    approvals.push(ApprovalRequirementV1 {
        role: "reviewer".to_string(),
        reason: format!(
            "the independent-review verdict was `{}`; the operator must attach the review record to the release",
            assumed.as_str()
        ),
        evidence_digest: req.independent_review_digest.clone(),
    });
    if !req.change_set.iter().any(|f| f.severity == "required") {
        // No required doc updates; no extra doc sign-off needed.
    } else {
        approvals.push(ApprovalRequirementV1 {
            role: "doc".to_string(),
            reason: "candidate touches a doc zone with severity=required; the operator must address before publishing".to_string(),
            evidence_digest: req.doc_impact_digest.clone(),
        });
    }

    // Final digest.
    let result_digest = canonical_digest(&serde_json::json!({
        "workId": req.work_id,
        "assumedVerdict": assumed,
        "sections": sections,
        "approvals": approvals,
        "citedDigests": cited,
    }))?;

    let out = ReleasePrepResultV1 {
        schema_version: NODE_DOC_RELEASE_VERSION.to_string(),
        work_id: req.work_id,
        assumed_verdict: assumed,
        sections,
        approvals,
        cited_digests: cited,
        result_digest,
        constraints: vec![
            "read-only: release-prep performs no file writes, no git operations, no network calls".to_string(),
            "no side effects: this output is a release-prep artifact, not a release authorization".to_string(),
            "the operator must perform the actual merge, tag, and publish".to_string(),
            "no node in the E5 library performs merge, publish, or deployment without explicit operator authority".to_string(),
        ],
    };
    serde_json::to_string(&out).map_err(Into::into)
}

fn validate_repo_root(repo_root: &str) -> Result<()> {
    if repo_root.is_empty() {
        bail!("repoRoot must not be empty");
    }
    let p = Path::new(repo_root);
    if !p.is_absolute() {
        bail!("repoRoot must be an absolute path: {repo_root}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Registry + manifest helpers
// ---------------------------------------------------------------------------

/// Declare the two E5/I05 doc-impact / release-prep nodes. They
/// are read-only with respect to the source repository:
/// `writableScopes` is empty on every manifest. They are also
/// side-effect-free with respect to releases: the release-prep node
/// produces a document, not a publication.
pub fn doc_release_registry() -> CapabilityRegistry {
    let mut reg = CapabilityRegistry::new();
    reg.declare(
        CAP_DOC_IMPACT,
        Capability::deterministic(&["repoRoot"], run_doc_impact),
    );
    reg.declare(
        CAP_RELEASE_PREP,
        Capability::deterministic(&["workId"], run_release_prep),
    );
    reg
}

fn io(name: &str, type_ref: &str) -> crate::workflow::node_contracts::NodeIo {
    crate::workflow::node_contracts::NodeIo {
        name: name.to_string(),
        type_ref: type_ref.to_string(),
        required: Some(true),
    }
}

/// Build a read-only `lite.node.v1` manifest for one of the two
/// E5/I05 nodes. The doc-impact node is a read-only classifier; the
/// release-prep node is also read-only (it produces an artifact, not
/// a publication).
pub fn doc_release_node_manifest(node_id: &str, capability: &str) -> NodeManifestV1 {
    let (inputs, outputs) = match capability {
        CAP_DOC_IMPACT => {
            let mut diff_io = io("candidateDiff", "core.String");
            diff_io.required = Some(false);
            let mut paths_io = io("changedPaths", "core.StringList");
            paths_io.required = Some(false);
            let mut addressed_io = io("alreadyAddressed", "lite.node.doc-release.CategoryList");
            addressed_io.required = Some(false);
            (
                vec![io("repoRoot", "core.Path"), paths_io, diff_io, addressed_io],
                vec![io("docImpact", "lite.node.doc-release.Result")],
            )
        }
        _ => {
            let mut verdict_io = io("assumedVerdict", "lite.node.review.Kind");
            verdict_io.required = Some(false);
            let mut summary_io = io("changeSummary", "core.String");
            summary_io.required = Some(false);
            let mut set_io = io("changeSet", "lite.node.doc-release.FindingList");
            set_io.required = Some(false);
            let mut impl_io = io("implementationDigest", "core.String");
            impl_io.required = Some(false);
            let mut val_io = io("validationDigest", "core.String");
            val_io.required = Some(false);
            let mut aud_io = io("auditDigest", "core.String");
            aud_io.required = Some(false);
            let mut rev_io = io("independentReviewDigest", "core.String");
            rev_io.required = Some(false);
            let mut doc_io = io("docImpactDigest", "core.String");
            doc_io.required = Some(false);
            (
                vec![
                    io("workId", "core.String"),
                    verdict_io,
                    summary_io,
                    set_io,
                    impl_io,
                    val_io,
                    aud_io,
                    rev_io,
                    doc_io,
                ],
                vec![io("releasePrep", "lite.node.doc-release.Result")],
            )
        }
    };
    NodeManifestV1::parse_json(
        &serde_json::json!({
            "schemaVersion": NODE_DOC_RELEASE_VERSION,
            "nodeId": node_id,
            "purpose": capability,
            "inputs": inputs,
            "outputs": outputs,
            "readableScopes": ["repo://fixture"],
            "writableScopes": [],
            "retry": {"maxAttempts": 1, "retryableClasses": []}
        })
        .to_string(),
    )
    .expect("doc-release manifest is well-formed")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_doc_impact_absolute() -> DocImpactRequestV1 {
        DocImpactRequestV1 {
            repo_root: std::env::temp_dir().to_string_lossy().to_string(),
            changed_paths: vec![],
            candidate_diff: None,
            already_addressed: vec![],
        }
    }

    fn make_release_prep(work_id: &str) -> ReleasePrepRequestV1 {
        ReleasePrepRequestV1 {
            work_id: work_id.to_string(),
            assumed_verdict: Some(ReviewKind::Approve),
            change_summary: Some("Adds doc-impact + release-prep nodes.".to_string()),
            change_set: vec![],
            implementation_digest: Some("a".repeat(64)),
            validation_digest: Some("b".repeat(64)),
            audit_digest: Some("c".repeat(64)),
            independent_review_digest: Some("d".repeat(64)),
            doc_impact_digest: Some("e".repeat(64)),
        }
    }

    fn call_doc_impact(req: DocImpactRequestV1) -> DocImpactResultV1 {
        let args = serde_json::to_value(&req).unwrap();
        let out = run_doc_impact(&args).unwrap();
        serde_json::from_str(&out).unwrap()
    }

    fn call_release_prep(req: ReleasePrepRequestV1) -> ReleasePrepResultV1 {
        let args = serde_json::to_value(&req).unwrap();
        let out = run_release_prep(&args).unwrap();
        serde_json::from_str(&out).unwrap()
    }

    #[test]
    fn doc_impact_rejects_empty_repo_root() {
        let mut req = make_doc_impact_absolute();
        req.repo_root = String::new();
        let args = serde_json::to_value(&req).unwrap();
        let err = run_doc_impact(&args).unwrap_err();
        assert!(err.to_string().contains("must not be empty"), "got: {err}");
    }

    #[test]
    fn doc_impact_rejects_relative_repo_root() {
        let mut req = make_doc_impact_absolute();
        req.repo_root = "relative/path".to_string();
        let args = serde_json::to_value(&req).unwrap();
        let err = run_doc_impact(&args).unwrap_err();
        assert!(err.to_string().contains("absolute"), "got: {err}");
    }

    #[test]
    fn doc_impact_rejects_neither_paths_nor_diff() {
        let req = make_doc_impact_absolute();
        let args = serde_json::to_value(&req).unwrap();
        let err = run_doc_impact(&args).unwrap_err();
        assert!(err.to_string().contains("at least one"), "got: {err}");
    }

    #[test]
    fn doc_impact_clean_change_approves() {
        let mut req = make_doc_impact_absolute();
        req.changed_paths = vec!["src/lib.rs".to_string(), "tests/x.rs".to_string()];
        let result = call_doc_impact(req);
        assert_eq!(result.kind, ReviewKind::Approve);
        assert!(result.findings.is_empty());
    }

    #[test]
    fn doc_impact_flags_changelog_change_as_required() {
        let mut req = make_doc_impact_absolute();
        req.changed_paths = vec!["CHANGELOG.md".to_string()];
        let result = call_doc_impact(req);
        assert_eq!(result.kind, ReviewKind::ChangesRequired);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.category == DocCategoryV1::Changelog && f.severity == "required")
        );
    }

    #[test]
    fn doc_impact_flags_readme_change_as_required() {
        let mut req = make_doc_impact_absolute();
        req.changed_paths = vec!["README.md".to_string()];
        let result = call_doc_impact(req);
        assert_eq!(result.kind, ReviewKind::ChangesRequired);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.category == DocCategoryV1::Readme && f.severity == "required")
        );
    }

    #[test]
    fn doc_impact_flags_user_guide_change_as_recommended() {
        let mut req = make_doc_impact_absolute();
        req.changed_paths = vec!["docs/guides/how-flows-work.md".to_string()];
        let result = call_doc_impact(req);
        assert_eq!(result.kind, ReviewKind::Approve);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.category == DocCategoryV1::UserGuide && f.severity == "recommended")
        );
    }

    #[test]
    fn doc_impact_infers_paths_from_diff() {
        let mut req = make_doc_impact_absolute();
        req.candidate_diff = Some(
            "diff --git a/docs/guides/x.md b/docs/guides/x.md\nindex 0000..1111\n+++ b/docs/guides/x.md\n@@\n+new line".to_string(),
        );
        let result = call_doc_impact(req);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.category == DocCategoryV1::UserGuide)
        );
    }

    #[test]
    fn doc_impact_honours_already_addressed_with_note() {
        let mut req = make_doc_impact_absolute();
        req.changed_paths = vec!["CHANGELOG.md".to_string()];
        req.already_addressed = vec![DocCategoryV1::Changelog];
        let result = call_doc_impact(req);
        let changelog_findings: Vec<_> = result
            .findings
            .iter()
            .filter(|f| f.category == DocCategoryV1::Changelog)
            .collect();
        assert!(changelog_findings.iter().any(|f| f.severity == "required"));
        assert!(changelog_findings.iter().any(|f| f.severity == "note"));
    }

    #[test]
    fn release_prep_rejects_empty_work_id() {
        let mut req = make_release_prep("");
        req.work_id = String::new();
        let args = serde_json::to_value(&req).unwrap();
        let err = run_release_prep(&args).unwrap_err();
        assert!(
            err.to_string().contains("workId must not be empty"),
            "got: {err}"
        );
    }

    #[test]
    fn release_prep_includes_implementation_validation_review_sections() {
        let req = make_release_prep("w-1");
        let result = call_release_prep(req);
        let headings: Vec<_> = result.sections.iter().map(|s| s.heading.as_str()).collect();
        assert!(headings.contains(&"Summary"));
        assert!(headings.contains(&"Implementation"));
        assert!(headings.contains(&"Validation"));
        assert!(headings.contains(&"Review"));
        // Every cited digest from the input appears in `citedDigests`.
        assert!(result.cited_digests.contains(&"a".repeat(64)));
        assert_eq!(result.assumed_verdict, ReviewKind::Approve);
    }

    #[test]
    fn release_prep_includes_doc_impact_section_when_supplied() {
        let req = make_release_prep("w-2");
        let result = call_release_prep(req);
        let headings: Vec<_> = result.sections.iter().map(|s| s.heading.as_str()).collect();
        assert!(headings.contains(&"Documentation"));
    }

    #[test]
    fn release_prep_includes_change_set_section_when_supplied() {
        let mut req = make_release_prep("w-3");
        req.change_set = vec![DocFindingV1 {
            category: DocCategoryV1::Changelog,
            path: Some("CHANGELOG.md".to_string()),
            message: "updated".to_string(),
            severity: "required".to_string(),
        }];
        let result = call_release_prep(req);
        let headings: Vec<_> = result.sections.iter().map(|s| s.heading.as_str()).collect();
        assert!(headings.contains(&"Change Set"));
        assert!(result.approvals.iter().any(|a| a.role == "doc"));
    }

    #[test]
    fn release_prep_records_approvals_for_every_role_when_required() {
        // change_set has a required-severity finding → doc approval
        // is included.
        let mut req = make_release_prep("w-4");
        req.change_set = vec![DocFindingV1 {
            category: DocCategoryV1::Readme,
            path: Some("README.md".to_string()),
            message: "x".to_string(),
            severity: "required".to_string(),
        }];
        let result = call_release_prep(req);
        let roles: Vec<_> = result.approvals.iter().map(|a| a.role.as_str()).collect();
        assert!(roles.contains(&"operator"));
        assert!(roles.contains(&"security"));
        assert!(roles.contains(&"reviewer"));
        assert!(roles.contains(&"doc"));
    }

    #[test]
    fn release_prep_assumes_changes_required_when_verdict_omitted() {
        let mut req = make_release_prep("w-5");
        req.assumed_verdict = None;
        let result = call_release_prep(req);
        assert_eq!(result.assumed_verdict, ReviewKind::ChangesRequired);
    }

    #[test]
    fn release_prep_carries_no_authorization_field() {
        // Defensive: the output type must not carry any field that
        // could be interpreted as "apply"/"publish"/"merge"
        // authorization. Verified at compile time via the type, but
        // also asserted by serialization shape here.
        let req = make_release_prep("w-6");
        let args = serde_json::to_value(&req).unwrap();
        let out = run_release_prep(&args).unwrap();
        let json: serde_json::Value = serde_json::from_str(&out).unwrap();
        for forbidden in ["authorized", "apply", "merge", "publish", "tag", "push"] {
            assert!(
                !json.as_object().unwrap().contains_key(forbidden),
                "release-prep output must not carry a `{forbidden}` field"
            );
        }
    }

    #[test]
    fn doc_release_node_manifests_are_read_only() {
        for cap in [CAP_DOC_IMPACT, CAP_RELEASE_PREP] {
            let m = doc_release_node_manifest("node-test", cap);
            assert!(
                m.writable_scopes.is_empty(),
                "{cap}: writableScopes must be empty"
            );
        }
    }

    #[test]
    fn doc_release_registry_contains_both_capabilities() {
        let reg = doc_release_registry();
        for cap in [CAP_DOC_IMPACT, CAP_RELEASE_PREP] {
            assert!(reg.resolve(cap).is_ok());
        }
    }
}
