//! Governed node library for E5/I04 (#128): security review, evidence
//! audit, and independent correctness review nodes.
//!
//! OWNERSHIP: Lite-owned `lite.node` capability implementations. Every
//! node here is READ-ONLY with respect to the target repository: each
//! takes a serialized candidate and emits a typed verdict + findings.
//! None of the three review nodes authorizes apply or merge; apply
//! and merge remain operator/elevated-runner actions whose inputs are
//! the emitted verdicts plus operator-supplied human authority.
//!
//! Concretely the three capabilities are:
//! - `security-review` — pattern-based scan of a candidate for risky
//!   paths, dangerous commands, secret/credential exposure, new
//!   dependency introductions, and authority/attestation mismatches.
//!   Deterministic; runs without a model.
//! - `evidence-audit` — fail-closed check that the expected durable
//!   evidence artifacts (worktree ref + content digest, change
//!   records, validation runs, prior review verdicts) are present and
//!   consistent with the candidate. Returns `reject` if any required
//!   artifact is missing or inconsistent.
//! - `independent-review` — composes a single `approve | changes-
//!   required | reject` verdict from the security and audit
//!   evidence plus the candidate reference. The node NEVER produces
//!   an "apply" or "merge" signal; it classifies. A separate elevated
//!   action consumes the verdict.
//!
//! Each node is a `Capability` handler, so the generic nine-gate
//! `NodeRunner` (lite.node.v1 contracts, lite.policy.v1 authorization,
//! redaction, journal durability) drives them unchanged. Conformance
//! is proven in `tests/node_library_conformance.rs` against the same
//! machinery the E5/I01-I03 nodes use.

use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::workflow::memory_contracts::canonical_digest;
use crate::workflow::node_contracts::NodeManifestV1;
use crate::workflow::node_runner::{Capability, CapabilityRegistry};

/// Version of the E5/I04 node contracts.
pub const NODE_REVIEW_VERSION: &str = "1.0.0";

/// Declared capability names.
pub const CAP_SECURITY_REVIEW: &str = "security-review";
pub const CAP_EVIDENCE_AUDIT: &str = "evidence-audit";
pub const CAP_INDEPENDENT_REVIEW: &str = "independent-review";

/// Safe-binary allowlist. The review node uses a smaller set than the
/// `validation` node because review scans the candidate's command list
/// for danger, not a live process. Adding a binary here without a
/// review of its escape surface is treated as a HARD-STOP in the
/// reviewer's comparative control gate.
const REVIEW_SAFE_BINARIES: &[&str] = &["cargo", "npm", "go", "pytest", "make", "git", "rustc"];

// ---------------------------------------------------------------------------
// Typed node outputs (`lite.node.<capability>` families)
// ---------------------------------------------------------------------------

/// Review verdict emitted by every review-shaped node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewKind {
    Approve,
    ChangesRequired,
    Reject,
}

impl ReviewKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ReviewKind::Approve => "approve",
            ReviewKind::ChangesRequired => "changes-required",
            ReviewKind::Reject => "reject",
        }
    }
}

/// One security finding: a category, severity, evidence path/line, and
/// human-readable message. `severity` is a string (not an enum) so
/// future rules can add new severities without breaking the wire
/// schema. Recognized values today: `info`, `warning`, `error`,
/// `critical`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityFindingV1 {
    pub category: String,
    pub severity: String,
    pub message: String,
    /// Optional evidence pointer: the path or line that triggered the
    /// finding, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

/// Output of the `security-review` node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityReviewResultV1 {
    pub schema_version: String,
    pub repo_root: String,
    pub kind: ReviewKind,
    pub findings: Vec<SecurityFindingV1>,
    /// Canonical digest over `(findings, kind)` — links downstream
    /// nodes (evidence-audit, independent-review) back to this result.
    pub result_digest: String,
    /// Constraints observed (read-only, no source access required).
    pub constraints: Vec<String>,
}

/// One evidence-audit finding: a `kind` (missing | inconsistent),
/// the expected artifact identifier, and the observed value/absence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceAuditFindingV1 {
    pub kind: String,
    pub artifact: String,
    pub expected: String,
    pub observed: String,
}

/// Output of the `evidence-audit` node. Fails closed: if any required
/// evidence artifact is missing or inconsistent, `kind` is `reject`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceAuditResultV1 {
    pub schema_version: String,
    pub work_id: String,
    pub kind: ReviewKind,
    pub missing: Vec<String>,
    pub inconsistencies: Vec<String>,
    /// Canonical digest over the missing + inconsistencies vectors.
    pub result_digest: String,
    /// Constraints observed.
    pub constraints: Vec<String>,
}

/// Output of the `independent-review` node. Composes a final verdict
/// from a security review and an evidence audit. The node NEVER
/// produces an "apply" or "merge" signal; it only classifies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndependentReviewResultV1 {
    pub schema_version: String,
    pub work_id: String,
    pub kind: ReviewKind,
    pub reasons: Vec<String>,
    /// Digests of the two upstream review-shaped results (security
    /// review + evidence audit), if they were provided. Tied to the
    /// evidence chain.
    pub security_digest: Option<String>,
    pub audit_digest: Option<String>,
    /// Canonical digest over the full result.
    pub result_digest: String,
    /// Constraints observed.
    pub constraints: Vec<String>,
}

// ---------------------------------------------------------------------------
// Input contract types
// ---------------------------------------------------------------------------

/// One command in the candidate's command list. We only need the
/// `command` and `args` fields for review (the validation node owns
/// the full `CommandSpecV1` shape; review is a thin scan over the
/// candidate's plan/ref).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewedCommandV1 {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Optional human-readable label for the command (e.g. plan step
    /// title); the review node does not interpret it, only echoes it
    /// in findings when relevant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Security-review input. Either `candidateCommands` (the plan's
/// commands list) or `candidateDiff` (the implementation diff as a
/// unified string) must be provided. `repoRoot` is the source path
/// (used for path-traversal resolution; no source mutation).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecurityReviewRequestV1 {
    pub repo_root: String,
    #[serde(default)]
    pub candidate_commands: Vec<ReviewedCommandV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_diff: Option<String>,
    /// Optional list of new dependency strings introduced by the
    /// candidate (e.g. `[{"crate": "tokio", "version": "1.2.3"}]`).
    /// The implement/repair node emits this; the review node flags
    /// each entry as `changes-required` for human confirmation.
    #[serde(default)]
    pub introduced_dependencies: Vec<IntroducedDependencyV1>,
    /// Optional list of authority references claimed by the candidate
    /// (e.g. the JSON `WorkspaceRefV1` string). The review node
    /// re-validates the content digest against the ref's manifest
    /// attestation if the original manifest JSON is provided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_manifest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntroducedDependencyV1 {
    pub crate_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
}

/// Evidence-audit input. `expected` enumerates the artifact identifiers
/// the audit must see (e.g. `[ "plan-evidence", "validation-evidence",
/// "implement-evidence" ]`). The audit's contract is fail-closed: any
/// missing or inconsistent entry forces `kind = reject`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceAuditRequestV1 {
    pub work_id: String,
    /// Required artifact identifiers.
    pub expected: Vec<String>,
    /// Map of `artifact -> observed digest` (hex SHA-256) or
    /// `null`/missing if the audit was unable to find the artifact.
    /// Keys are free-form; the audit cross-references them against
    /// `expected`.
    #[serde(default)]
    pub observed: std::collections::BTreeMap<String, Option<String>>,
}

/// Independent-review input. The node composes from the security
/// review and the evidence audit, plus an optional manual-human-flag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IndependentReviewRequestV1 {
    pub work_id: String,
    /// Verdict emitted by the upstream `security-review` node
    /// (`approve`/`changes-required`/`reject`).
    pub security_kind: ReviewKind,
    /// Verdict emitted by the upstream `evidence-audit` node.
    pub audit_kind: ReviewKind,
    /// Optional upstream result digests (for evidence linkage).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub security_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit_digest: Option<String>,
}

// ---------------------------------------------------------------------------
// Pattern-based security rules
// ---------------------------------------------------------------------------

/// One security rule. A rule is matched by exact-substring (case-
/// sensitive, after the haystack is normalized). The first matching
/// rule per (category, evidence) wins; later rules for the same
/// category are not flagged again.
struct SecurityRule {
    category: &'static str,
    severity: &'static str,
    message: &'static str,
    patterns: &'static [&'static str],
}

const SECURITY_RULES: &[SecurityRule] = &[
    // Secrets: hard-coded credential patterns. Each pattern is a
    // unambiguous signature of a credential, not a generic word.
    SecurityRule {
        category: "secrets",
        severity: "critical",
        message: "hard-coded private key (PEM) detected in candidate",
        patterns: &[
            "-----BEGIN RSA PRIVATE KEY-----",
            "-----BEGIN EC PRIVATE KEY-----",
        ],
    },
    SecurityRule {
        category: "secrets",
        severity: "critical",
        message: "hard-coded OpenSSH private key detected in candidate",
        patterns: &["-----BEGIN OPENSSH PRIVATE KEY-----"],
    },
    SecurityRule {
        category: "secrets",
        severity: "critical",
        message: "AWS access key ID detected in candidate (AKIA prefix)",
        patterns: &["AKIA", "ASIA"],
    },
    SecurityRule {
        category: "secrets",
        severity: "critical",
        message: "GitHub personal access token (ghp_) detected in candidate",
        patterns: &["ghp_", "gho_", "ghs_", "ghu_", "ghr_"],
    },
    SecurityRule {
        category: "secrets",
        severity: "critical",
        message: "Slack token (xox*) detected in candidate",
        patterns: &["xoxb-", "xoxp-", "xoxa-", "xoxr-"],
    },
    // Commands: escape-hatch flags. A command that tries to escape
    // the worktree into the source repository is a critical risk.
    SecurityRule {
        category: "commands",
        severity: "critical",
        message: "`git --git-dir` or `git --work-tree` would let the command escape the worktree into the source",
        patterns: &["--git-dir", "--work-tree", "--exec-path"],
    },
    SecurityRule {
        category: "commands",
        severity: "warning",
        message: "path-traversal segment `..` in command argument",
        patterns: &["/../", "\\..\\"],
    },
    // Dependencies: a brand-new `Cargo.toml`/`Cargo.lock` change is
    // always a human-confirmation item.
    SecurityRule {
        category: "dependencies",
        severity: "warning",
        message: "candidate diff modifies Cargo.toml or Cargo.lock (any dependency change is a human-confirmation item)",
        patterns: &["[+]    name = \"", "name = \"", "+Cargo.toml", "Cargo.lock"],
    },
    // Paths: candidate-diff markers indicating a path-traversal in
    // a proposed new file. The pattern is intentionally narrow to
    // avoid false positives on inline `..` in code comments.
    SecurityRule {
        category: "paths",
        severity: "warning",
        message: "candidate diff references a `..` path segment (e.g. `../<file>`)",
        patterns: &["+++ b/../", "--- a/../", "diff --git a/../"],
    },
];

// ---------------------------------------------------------------------------
// Capability handlers
// ---------------------------------------------------------------------------

fn run_security_review(args: &serde_json::Value) -> Result<String> {
    let req: SecurityReviewRequestV1 =
        serde_json::from_value(args.clone()).context("security-review: invalid request JSON")?;
    validate_repo_root(&req.repo_root)?;

    let mut findings: Vec<SecurityFindingV1> = Vec::new();

    // 1. Scan each command for escape-hatch flags and disallowed
    // binaries. The allowlist is the same one the validation node
    // enforces at runtime; review catches it at plan time.
    for cmd in &req.candidate_commands {
        if !REVIEW_SAFE_BINARIES.contains(&cmd.command.as_str()) {
            findings.push(SecurityFindingV1 {
                category: "commands".to_string(),
                severity: "critical".to_string(),
                message: format!(
                    "command binary {:?} is not in the safe-binary allowlist (allowed: {:?})",
                    cmd.command, REVIEW_SAFE_BINARIES
                ),
                evidence: Some(format!("{} {:?}", cmd.command, cmd.args)),
            });
            continue;
        }
        if cmd.command == "git" {
            for arg in &cmd.args {
                if arg == "--git-dir"
                    || arg == "--work-tree"
                    || arg == "--exec-path"
                    || arg.starts_with("--git-dir=")
                    || arg.starts_with("--work-tree=")
                    || arg.starts_with("--exec-path=")
                {
                    findings.push(SecurityFindingV1 {
                        category: "commands".to_string(),
                        severity: "critical".to_string(),
                        message: format!(
                            "git flag {:?} would let the command escape the worktree into the source",
                            arg
                        ),
                        evidence: Some(format!("{} {:?}", cmd.command, cmd.args)),
                    });
                }
            }
        }
        for arg in &cmd.args {
            if arg.split(['/', '\\']).any(|seg| seg == "..") {
                findings.push(SecurityFindingV1 {
                    category: "commands".to_string(),
                    severity: "warning".to_string(),
                    message: format!("path-traversal segment `..` in argument {arg:?}"),
                    evidence: Some(format!("{} {:?}", cmd.command, cmd.args)),
                });
            }
        }
    }

    // 2. Scan the candidate diff (if provided) against the security
    // rule table.
    if let Some(diff) = &req.candidate_diff {
        for rule in SECURITY_RULES {
            for pattern in rule.patterns {
                if diff.contains(pattern) {
                    findings.push(SecurityFindingV1 {
                        category: rule.category.to_string(),
                        severity: rule.severity.to_string(),
                        message: rule.message.to_string(),
                        evidence: Some(format!("pattern={pattern:?}")),
                    });
                    break; // one finding per rule per diff
                }
            }
        }
    }

    // 3. Flag any introduced dependency as `changes-required` for
    // human confirmation. The reviewer cannot know whether the
    // dependency is required; the operator decides.
    for dep in &req.introduced_dependencies {
        findings.push(SecurityFindingV1 {
            category: "dependencies".to_string(),
            severity: "warning".to_string(),
            message: format!(
                "candidate introduces a new dependency `{}` (version {}); requires human confirmation",
                dep.crate_name,
                dep.version.clone().unwrap_or_else(|| "<unspecified>".to_string())
            ),
            evidence: Some(format!("crate={}", dep.crate_name)),
        });
    }

    // 4. If a candidate ref + manifest were provided, verify the ref's
    // content digest equals the manifest's own digest. The PR7 work
    // (issue #126) made the ref attest to the manifest; the review
    // re-verifies it.
    if let (Some(ref_str), Some(manifest_str)) = (&req.candidate_ref, &req.candidate_manifest) {
        let manifest_json: serde_json::Value =
            serde_json::from_str(manifest_str).context("candidate_manifest: invalid JSON")?;
        let manifest_digest = manifest_json
            .get("contentDigest")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if !manifest_digest.is_empty() {
            let ref_json: serde_json::Value =
                serde_json::from_str(ref_str).context("candidate_ref: invalid JSON")?;
            let ref_digest = ref_json
                .get("contentDigest")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if ref_digest != manifest_digest {
                findings.push(SecurityFindingV1 {
                    category: "authority".to_string(),
                    severity: "critical".to_string(),
                    message: format!(
                        "candidate ref contentDigest ({ref_digest}) does not match the supplied manifest's contentDigest ({manifest_digest}); ref has been substituted"
                    ),
                    evidence: Some("contentDigest mismatch".to_string()),
                });
            }
        }
    }

    let kind = if findings.iter().any(|f| f.severity == "critical") {
        ReviewKind::Reject
    } else if findings
        .iter()
        .any(|f| f.severity == "error" || f.severity == "warning")
    {
        ReviewKind::ChangesRequired
    } else {
        ReviewKind::Approve
    };

    let result_digest = canonical_digest(&serde_json::json!({
        "kind": kind,
        "findings": findings,
    }))?;

    let out = SecurityReviewResultV1 {
        schema_version: NODE_REVIEW_VERSION.to_string(),
        repo_root: req.repo_root,
        kind,
        findings,
        result_digest,
        constraints: vec![
            "read-only: security-review performs no repository mutation".to_string(),
            "deterministic: no model or external service is invoked".to_string(),
        ],
    };
    serde_json::to_string(&out).map_err(Into::into)
}

fn run_evidence_audit(args: &serde_json::Value) -> Result<String> {
    let req: EvidenceAuditRequestV1 =
        serde_json::from_value(args.clone()).context("evidence-audit: invalid request JSON")?;
    if req.work_id.is_empty() {
        bail!("evidence-audit: work_id must not be empty");
    }
    if req.expected.is_empty() {
        bail!(
            "evidence-audit: expected list must not be empty (fail-closed: nothing to audit means nothing passes)"
        );
    }

    let mut missing: Vec<String> = Vec::new();
    let mut inconsistencies: Vec<String> = Vec::new();

    // Every expected artifact MUST be present in `observed`. An empty
    // value (or absent key) is treated as missing.
    for art in &req.expected {
        match req.observed.get(art) {
            None => missing.push(art.clone()),
            Some(None) => missing.push(art.clone()),
            Some(Some(digest)) => {
                if digest.len() != 64 || !digest.chars().all(|c| c.is_ascii_hexdigit()) {
                    inconsistencies.push(format!(
                        "{art}: observed digest is not a 64-char lowercase hex SHA-256 ({digest:?})"
                    ));
                }
            }
        }
    }

    // Also fail-closed on any unexpected (extra) artifact whose
    // digest is malformed. Extra malformed digests are evidence of
    // tampering, not noise.
    for (k, v) in &req.observed {
        if let Some(digest) = v
            && !req.expected.contains(k)
            && (digest.len() != 64 || !digest.chars().all(|c| c.is_ascii_hexdigit()))
        {
            inconsistencies.push(format!(
                "{k}: unexpected artifact with malformed digest (not in expected list)"
            ));
        }
    }

    let kind = if !missing.is_empty() || !inconsistencies.is_empty() {
        ReviewKind::Reject
    } else {
        ReviewKind::Approve
    };

    let result_digest = canonical_digest(&serde_json::json!({
        "missing": missing,
        "inconsistencies": inconsistencies,
    }))?;

    let out = EvidenceAuditResultV1 {
        schema_version: NODE_REVIEW_VERSION.to_string(),
        work_id: req.work_id,
        kind,
        missing,
        inconsistencies,
        result_digest,
        constraints: vec![
            "read-only: evidence-audit performs no repository access".to_string(),
            "fail-closed: any missing or inconsistent evidence rejects the audit".to_string(),
        ],
    };
    serde_json::to_string(&out).map_err(Into::into)
}

fn run_independent_review(args: &serde_json::Value) -> Result<String> {
    let req: IndependentReviewRequestV1 =
        serde_json::from_value(args.clone()).context("independent-review: invalid request JSON")?;
    if req.work_id.is_empty() {
        bail!("independent-review: work_id must not be empty");
    }

    let mut reasons: Vec<String> = Vec::new();

    // Composition rules:
    //   - security = reject   AND audit = reject   => reject
    //   - either  = reject                          => reject
    //   - either  = changes-required               => changes-required
    //   - both    = approve                         => approve
    // The review NEVER escalates upstream kinds; the most severe
    // wins. It does NOT invent intermediate kinds.
    let kind = match (req.security_kind, req.audit_kind) {
        (ReviewKind::Reject, _) | (_, ReviewKind::Reject) => ReviewKind::Reject,
        (ReviewKind::ChangesRequired, _) | (_, ReviewKind::ChangesRequired) => {
            ReviewKind::ChangesRequired
        }
        (ReviewKind::Approve, ReviewKind::Approve) => ReviewKind::Approve,
    };

    reasons.push(format!(
        "security review verdict: {}",
        req.security_kind.as_str()
    ));
    reasons.push(format!(
        "evidence audit verdict: {}",
        req.audit_kind.as_str()
    ));
    reasons.push(format!("composed verdict: {}", kind.as_str()));
    reasons.push(
        "review does not authorize apply or merge; that requires a separate elevated action"
            .to_string(),
    );

    let result_digest = canonical_digest(&serde_json::json!({
        "kind": kind,
        "reasons": reasons,
        "securityDigest": req.security_digest,
        "auditDigest": req.audit_digest,
    }))?;

    let out = IndependentReviewResultV1 {
        schema_version: NODE_REVIEW_VERSION.to_string(),
        work_id: req.work_id,
        kind,
        reasons,
        security_digest: req.security_digest,
        audit_digest: req.audit_digest,
        result_digest,
        constraints: vec![
            "read-only: independent-review does not mutate the repository or the journal"
                .to_string(),
            "no side effects: this verdict is a classification, not an authorization".to_string(),
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

/// Declare the three E5/I04 review nodes. They are read-only with
/// respect to the source repository: `writableScopes` is empty on every
/// manifest. The capability-level `required_args` covers only the
/// fields that are always required; optional fields are declared on
/// the manifest as `required: Some(false)` and the handler enforces
/// the presence/absence policy itself.
pub fn review_registry() -> CapabilityRegistry {
    let mut reg = CapabilityRegistry::new();
    reg.declare(
        CAP_SECURITY_REVIEW,
        Capability::deterministic(&["repoRoot"], run_security_review),
    );
    reg.declare(
        CAP_EVIDENCE_AUDIT,
        Capability::deterministic(&["workId", "expected"], run_evidence_audit),
    );
    reg.declare(
        CAP_INDEPENDENT_REVIEW,
        Capability::deterministic(
            &["workId", "securityKind", "auditKind"],
            run_independent_review,
        ),
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

/// Build a read-only `lite.node.v1` manifest for one of the three
/// E5/I04 review nodes. The review nodes never write; `writableScopes`
/// is empty.
pub fn review_node_manifest(node_id: &str, capability: &str) -> NodeManifestV1 {
    let (inputs, outputs) = match capability {
        CAP_SECURITY_REVIEW => {
            let mut diff_io = io("candidateDiff", "core.String");
            diff_io.required = Some(false);
            let mut cmds_io = io("candidateCommands", "lite.node.review.CommandList");
            cmds_io.required = Some(false);
            let mut deps_io = io("introducedDependencies", "lite.node.review.DependencyList");
            deps_io.required = Some(false);
            let mut ref_io = io("candidateRef", "core.String");
            ref_io.required = Some(false);
            let mut manifest_io = io("candidateManifest", "core.String");
            manifest_io.required = Some(false);
            (
                vec![
                    io("repoRoot", "core.Path"),
                    cmds_io,
                    diff_io,
                    deps_io,
                    ref_io,
                    manifest_io,
                ],
                vec![io("security", "lite.node.review.SecurityResult")],
            )
        }
        CAP_EVIDENCE_AUDIT => {
            let mut observed_io = io("observed", "lite.node.review.ObservedMap");
            observed_io.required = Some(false);
            (
                vec![
                    io("workId", "core.String"),
                    io("expected", "core.StringList"),
                    observed_io,
                ],
                vec![io("audit", "lite.node.review.AuditResult")],
            )
        }
        _ => (
            vec![
                io("workId", "core.String"),
                io("securityKind", "lite.node.review.Kind"),
                io("auditKind", "lite.node.review.Kind"),
            ],
            vec![io("review", "lite.node.review.IndependentResult")],
        ),
    };
    NodeManifestV1::parse_json(
        &serde_json::json!({
            "schemaVersion": NODE_REVIEW_VERSION,
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
    .expect("review manifest is well-formed")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_cmd(command: &str, args: &[&str]) -> ReviewedCommandV1 {
        ReviewedCommandV1 {
            command: command.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            label: None,
        }
    }

    fn make_audit(work_id: &str, expected: &[&str]) -> EvidenceAuditRequestV1 {
        EvidenceAuditRequestV1 {
            work_id: work_id.to_string(),
            expected: expected.iter().map(|s| s.to_string()).collect(),
            observed: std::collections::BTreeMap::new(),
        }
    }

    fn make_review(
        work_id: &str,
        security: ReviewKind,
        audit: ReviewKind,
    ) -> IndependentReviewRequestV1 {
        IndependentReviewRequestV1 {
            work_id: work_id.to_string(),
            security_kind: security,
            audit_kind: audit,
            security_digest: Some("a".repeat(64)),
            audit_digest: Some("b".repeat(64)),
        }
    }

    fn make_security(_repo_root: &str) -> SecurityReviewRequestV1 {
        SecurityReviewRequestV1 {
            repo_root: _repo_root.to_string(),
            candidate_commands: vec![],
            candidate_diff: None,
            introduced_dependencies: vec![],
            candidate_ref: None,
            candidate_manifest: None,
        }
    }

    /// Build a SecurityReviewRequestV1 with a system-temp absolute
    /// `repoRoot` so the absolute-path check passes on every platform
    /// (Windows, macOS, Linux).
    fn make_security_absolute() -> SecurityReviewRequestV1 {
        make_security(&std::env::temp_dir().to_string_lossy())
    }

    fn call_security(req: SecurityReviewRequestV1) -> SecurityReviewResultV1 {
        let args = serde_json::to_value(&req).unwrap();
        let out = run_security_review(&args).unwrap();
        serde_json::from_str(&out).unwrap()
    }

    fn call_audit(req: EvidenceAuditRequestV1) -> EvidenceAuditResultV1 {
        let args = serde_json::to_value(&req).unwrap();
        let out = run_evidence_audit(&args).unwrap();
        serde_json::from_str(&out).unwrap()
    }

    fn call_review(req: IndependentReviewRequestV1) -> IndependentReviewResultV1 {
        let args = serde_json::to_value(&req).unwrap();
        let out = run_independent_review(&args).unwrap();
        serde_json::from_str(&out).unwrap()
    }

    #[test]
    fn security_review_rejects_repo_root_must_be_absolute() {
        let mut req = make_security("relative/path");
        req.repo_root = "relative/path".to_string();
        let args = serde_json::to_value(&req).unwrap();
        let err = run_security_review(&args).unwrap_err();
        assert!(err.to_string().contains("absolute path"), "got: {err}");
    }

    #[test]
    fn security_review_rejects_empty_repo_root() {
        let args = serde_json::json!({"repoRoot": ""});
        let err = run_security_review(&args).unwrap_err();
        assert!(err.to_string().contains("must not be empty"), "got: {err}");
    }

    #[test]
    fn security_review_clean_candidate_approves() {
        let mut req = make_security_absolute();
        req.candidate_commands = vec![
            make_cmd("cargo", &["test", "--offline"]),
            make_cmd("git", &["rev-parse", "HEAD"]),
        ];
        let result = call_security(req);
        assert_eq!(result.kind, ReviewKind::Approve);
        assert!(result.findings.is_empty());
        assert_eq!(result.result_digest.len(), 64);
    }

    #[test]
    fn security_review_rejects_critical_secrets() {
        let mut req = make_security_absolute();
        req.candidate_diff = Some(
            "diff --git a/keys.txt b/keys.txt\n+++ b/keys.txt\n+-----BEGIN RSA PRIVATE KEY-----\nABC\n"
                .to_string(),
        );
        let result = call_security(req);
        assert_eq!(result.kind, ReviewKind::Reject);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.category == "secrets" && f.severity == "critical")
        );
    }

    #[test]
    fn security_review_flags_github_pat() {
        let mut req = make_security_absolute();
        req.candidate_diff =
            Some("let token = \"ghp_abcdefghijklmnopqrstuvwxyz0123456789\";\n".to_string());
        let result = call_security(req);
        assert_eq!(result.kind, ReviewKind::Reject);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.message.contains("GitHub personal access token"))
        );
    }

    #[test]
    fn security_review_flags_aws_key() {
        let mut req = make_security_absolute();
        req.candidate_diff = Some("let key = \"AKIAIOSFODNN7EXAMPLE\";\n".to_string());
        let result = call_security(req);
        assert_eq!(result.kind, ReviewKind::Reject);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.message.contains("AWS access key"))
        );
    }

    #[test]
    fn security_review_flags_git_escape_flags() {
        let mut req = make_security_absolute();
        req.candidate_commands = vec![make_cmd(
            "git",
            &[
                "--git-dir",
                "/tmp/repo/.git",
                "--work-tree",
                "/tmp/repo",
                "rm",
                "leak.txt",
            ],
        )];
        let result = call_security(req);
        assert_eq!(result.kind, ReviewKind::Reject);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.message.contains("would let the command escape"))
        );
    }

    #[test]
    fn security_review_flags_path_traversal() {
        let mut req = make_security_absolute();
        req.candidate_commands = vec![make_cmd("cp", &["/etc/passwd", "/tmp/repo/../leak.txt"])];
        let result = call_security(req);
        // `cp` is not in the safe allowlist → critical.
        assert_eq!(result.kind, ReviewKind::Reject);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.message.contains("not in the safe-binary allowlist"))
        );
    }

    #[test]
    fn security_review_flags_introduced_dependencies_as_warning() {
        let mut req = make_security_absolute();
        req.introduced_dependencies.push(IntroducedDependencyV1 {
            crate_name: "tokio".to_string(),
            version: Some("1.2.3".to_string()),
            registry: Some("crates.io".to_string()),
        });
        let result = call_security(req);
        // Dependency change is a warning, not critical → changes-required.
        assert_eq!(result.kind, ReviewKind::ChangesRequired);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.message.contains("new dependency"))
        );
    }

    #[test]
    fn security_review_flags_cargo_toml_diff() {
        let mut req = make_security_absolute();
        req.candidate_diff = Some(
            "diff --git a/Cargo.toml b/Cargo.toml\n+++ b/Cargo.toml\n@@ -1,2 +1,3 @@\n [package]\n name = \"x\"\n+[+]    name = \"newdep\""
                .to_string(),
        );
        let result = call_security(req);
        assert_eq!(result.kind, ReviewKind::ChangesRequired);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.message.contains("Cargo.toml or Cargo.lock"))
        );
    }

    #[test]
    fn security_review_rejects_substituted_ref() {
        let mut req = make_security_absolute();
        // A ref whose contentDigest does not match the manifest's.
        req.candidate_ref = Some(r#"{"schemaVersion":"1.1.0","contentDigest":"AAAA"}"#.to_string());
        req.candidate_manifest = Some(r#"{"contentDigest":"BBBB"}"#.to_string());
        let result = call_security(req);
        assert_eq!(result.kind, ReviewKind::Reject);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.category == "authority" && f.message.contains("substituted"))
        );
    }

    #[test]
    fn security_review_accepts_matching_ref() {
        let mut req = make_security_absolute();
        let same = "a".repeat(64);
        req.candidate_ref = Some(format!(r#"{{"contentDigest":"{same}"}}"#));
        req.candidate_manifest = Some(format!(r#"{{"contentDigest":"{same}"}}"#));
        let result = call_security(req);
        assert_eq!(result.kind, ReviewKind::Approve);
    }

    #[test]
    fn evidence_audit_rejects_when_expected_missing() {
        let req = make_audit("w-1", &["plan-evidence", "validation-evidence"]);
        // `observed` is empty → both missing.
        let result = call_audit(req);
        assert_eq!(result.kind, ReviewKind::Reject);
        assert_eq!(result.missing.len(), 2);
        assert!(result.missing.contains(&"plan-evidence".to_string()));
        assert!(result.missing.contains(&"validation-evidence".to_string()));
    }

    #[test]
    fn evidence_audit_rejects_malformed_digest() {
        let mut req = make_audit("w-2", &["x"]);
        req.observed
            .insert("x".to_string(), Some("not-a-real-digest".to_string()));
        let result = call_audit(req);
        assert_eq!(result.kind, ReviewKind::Reject);
        assert!(
            result
                .inconsistencies
                .iter()
                .any(|s| s.contains("not a 64-char"))
        );
    }

    #[test]
    fn evidence_audit_rejects_unexpected_malformed_artifact() {
        let mut req = make_audit("w-3", &["x"]);
        req.observed
            .insert("unexpected-key".to_string(), Some("short".to_string()));
        let result = call_audit(req);
        assert_eq!(result.kind, ReviewKind::Reject);
        assert!(
            result
                .inconsistencies
                .iter()
                .any(|s| s.contains("unexpected"))
        );
    }

    #[test]
    fn evidence_audit_approves_when_all_present_and_well_formed() {
        let mut req = make_audit("w-4", &["a", "b"]);
        req.observed.insert("a".to_string(), Some("a".repeat(64)));
        req.observed.insert("b".to_string(), Some("b".repeat(64)));
        let result = call_audit(req);
        assert_eq!(result.kind, ReviewKind::Approve);
        assert!(result.missing.is_empty());
        assert!(result.inconsistencies.is_empty());
    }

    #[test]
    fn evidence_audit_rejects_empty_expected() {
        let req = make_audit("w-5", &[]);
        let args = serde_json::to_value(&req).unwrap();
        let err = run_evidence_audit(&args).unwrap_err();
        assert!(
            err.to_string().contains("expected list must not be empty"),
            "got: {err}"
        );
    }

    #[test]
    fn evidence_audit_rejects_empty_work_id() {
        let req = make_audit("", &["x"]);
        let args = serde_json::to_value(&req).unwrap();
        let err = run_evidence_audit(&args).unwrap_err();
        assert!(
            err.to_string().contains("work_id must not be empty"),
            "got: {err}"
        );
    }

    #[test]
    fn independent_review_reject_if_either_rejects() {
        let r = call_review(make_review("w", ReviewKind::Reject, ReviewKind::Approve));
        assert_eq!(r.kind, ReviewKind::Reject);
        let r = call_review(make_review("w", ReviewKind::Approve, ReviewKind::Reject));
        assert_eq!(r.kind, ReviewKind::Reject);
        let r = call_review(make_review("w", ReviewKind::Reject, ReviewKind::Reject));
        assert_eq!(r.kind, ReviewKind::Reject);
    }

    #[test]
    fn independent_review_changes_required_if_either_changes() {
        let r = call_review(make_review(
            "w",
            ReviewKind::Approve,
            ReviewKind::ChangesRequired,
        ));
        assert_eq!(r.kind, ReviewKind::ChangesRequired);
        let r = call_review(make_review(
            "w",
            ReviewKind::ChangesRequired,
            ReviewKind::Approve,
        ));
        assert_eq!(r.kind, ReviewKind::ChangesRequired);
    }

    #[test]
    fn independent_review_approve_only_when_both_approve() {
        let r = call_review(make_review("w", ReviewKind::Approve, ReviewKind::Approve));
        assert_eq!(r.kind, ReviewKind::Approve);
        let reasons_joined = r.reasons.join("\n");
        assert!(reasons_joined.contains("composed verdict: approve"));
        assert!(reasons_joined.contains("does not authorize apply or merge"));
    }

    #[test]
    fn independent_review_rejects_empty_work_id() {
        let req = make_review("", ReviewKind::Approve, ReviewKind::Approve);
        let args = serde_json::to_value(&req).unwrap();
        let err = run_independent_review(&args).unwrap_err();
        assert!(
            err.to_string().contains("work_id must not be empty"),
            "got: {err}"
        );
    }

    #[test]
    fn review_node_manifests_are_read_only() {
        // Every review node manifest must declare `writableScopes: []`.
        for cap in [
            CAP_SECURITY_REVIEW,
            CAP_EVIDENCE_AUDIT,
            CAP_INDEPENDENT_REVIEW,
        ] {
            let m = review_node_manifest("node-review-test", cap);
            assert!(
                m.writable_scopes.is_empty(),
                "{cap}: writableScopes must be empty (review nodes are read-only)"
            );
            assert!(
                !m.readable_scopes.is_empty(),
                "{cap}: readableScopes must be present so the policy gate can verify authority"
            );
        }
    }

    #[test]
    fn review_registry_contains_all_three_capabilities() {
        let reg = review_registry();
        // CapabilityRegistry exposes `resolve`; verify each cap is
        // registered.
        for cap in [
            CAP_SECURITY_REVIEW,
            CAP_EVIDENCE_AUDIT,
            CAP_INDEPENDENT_REVIEW,
        ] {
            assert!(
                reg.resolve(cap).is_ok(),
                "capability {cap} must be registered in the review registry"
            );
        }
    }

    #[test]
    fn review_registry_capabilities_have_their_required_args() {
        // The capability-level required_args must match the always-
        // present fields of the request types.
        let reg = review_registry();
        let sec = reg.resolve(CAP_SECURITY_REVIEW).unwrap();
        assert!(sec.required_args.contains(&"repoRoot"));
        let aud = reg.resolve(CAP_EVIDENCE_AUDIT).unwrap();
        assert!(aud.required_args.contains(&"workId"));
        assert!(aud.required_args.contains(&"expected"));
        let rev = reg.resolve(CAP_INDEPENDENT_REVIEW).unwrap();
        assert!(rev.required_args.contains(&"workId"));
        assert!(rev.required_args.contains(&"securityKind"));
        assert!(rev.required_args.contains(&"auditKind"));
    }
}
