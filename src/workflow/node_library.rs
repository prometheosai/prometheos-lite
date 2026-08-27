//! Governed node library for E5/I01 (#125): intake, repository discovery, and
//! planning nodes.
//!
//! OWNERSHIP: Lite-owned `lite.node` capability implementations. Every node
//! here is READ-ONLY with respect to the target repository:
//! - `intake` validates a user objective and emits a typed task manifest, or
//!   rejects ambiguous / out-of-repository scope safely (fail closed);
//! - `repo-discovery` builds a revision-qualified index via the existing
//!   `IndexedRepository` engine and records files, languages, tests, and
//!   constraints as evidence — no writes;
//! - `planning` emits a typed, scoped plan linked to discovery evidence.
//!
//! Each node is a `Capability` handler, so the generic nine-gate `NodeRunner`
//! (lite.node.v1 contracts, lite.policy.v1 authorization, redaction, journal
//! durability) drives them unchanged. Conformance is proven by
//! `tests/node_library_conformance.rs` against the same machinery as
//! `tests/node_conformance_kit.rs`.

use std::path::Path;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::workflow::memory_contracts::canonical_digest;
use crate::workflow::node_contracts::NodeManifestV1;
use crate::workflow::node_runner::{Capability, CapabilityRegistry};
use crate::workflow::repo_index::{IndexedRepository, RepoFactBatchV1};

/// Version of the E5/I01 node library contracts.
pub const NODE_LIBRARY_VERSION: &str = "1.0.0";

/// Declared capability names (registered in [`intake_discovery_planning_registry`]).
pub const CAP_INTAKE: &str = "intake";
pub const CAP_DISCOVERY: &str = "repo-discovery";
pub const CAP_PLANNING: &str = "planning";

// ---------------------------------------------------------------------------
// Typed node outputs (`lite.node.<capability>` families)
// ---------------------------------------------------------------------------

/// Output of the `intake` node: a typed task manifest derived from a validated
/// objective.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntakeTaskManifestV1 {
    pub schema_version: String,
    pub task_id: String,
    pub objective: String,
    pub scope: Vec<String>,
    pub authorized: bool,
}

/// One discovered file with its provenance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryFileV1 {
    pub path: String,
    pub language: String,
    pub sha256: String,
}

/// Output of the `repo-discovery` node: files, languages, tests, and
/// constraints recorded as revision-qualified evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryResultV1 {
    pub schema_version: String,
    pub revision: String,
    pub dirty: bool,
    pub file_count: usize,
    pub languages: Vec<String>,
    pub test_files: Vec<String>,
    pub constraints: Vec<String>,
    /// Canonical digest of the emitted `lite.repofact` batch (links planning).
    pub fact_batch_digest: String,
    pub files: Vec<DiscoveryFileV1>,
}

/// One scoped plan step, referencing discovery evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStepV1 {
    pub step: u32,
    pub title: String,
    pub targets: Vec<String>,
    pub evidence_ref: String,
}

/// Output of the `planning` node: a typed plan linked to discovery evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopedPlanV1 {
    pub schema_version: String,
    pub plan_id: String,
    pub objective: String,
    pub discovery_revision: String,
    pub discovery_evidence_id: String,
    pub steps: Vec<PlanStepV1>,
    pub assumptions: Vec<String>,
}

// ---------------------------------------------------------------------------
// Intake
// ---------------------------------------------------------------------------

/// Verbs that mark a concrete, actionable objective (vs. a vague request).
const ACTION_VERBS: &[&str] = &[
    "implement",
    "add",
    "create",
    "fix",
    "repair",
    "remove",
    "delete",
    "refactor",
    "test",
    "document",
    "migrate",
    "update",
    "change",
    "optimize",
    "deprecate",
    "support",
    "extend",
    "introduce",
    "wire",
];

/// True when an objective references a path outside the repository boundary
/// (absolute path, parent traversal, UNC, or Windows drive). Such a scope is
/// unauthorized for an in-repo plan.
fn objective_references_unauthorized_scope(objective: &str) -> bool {
    objective.split_whitespace().any(|tok| {
        tok == ".."
            || tok.starts_with('/')
            || tok.starts_with("\\\\")
            || (tok.len() >= 2
                && tok.as_bytes()[1] == b':'
                && tok.as_bytes()[0].is_ascii_alphabetic())
    })
}

fn run_intake(args: &serde_json::Value) -> Result<String> {
    let objective = args
        .get("objective")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("intake requires a string objective"))?;
    if objective_references_unauthorized_scope(objective) {
        bail!(
            "intake rejected: objective references a path outside the repository (unauthorized scope)"
        );
    }
    let words: Vec<&str> = objective.split_whitespace().collect();
    let has_verb = words.iter().any(|w| {
        let lower = w.to_lowercase();
        ACTION_VERBS
            .iter()
            .any(|v| lower == *v || lower.starts_with(&format!("{v} ")) || lower.contains(v))
    });
    if words.len() < 3 || !has_verb {
        bail!(
            "intake rejected: objective is ambiguous; provide a concrete instruction (verb + target)"
        );
    }
    let scope: Vec<String> = args
        .get("requestedScopes")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let digest = canonical_digest(&serde_json::json!(objective))?;
    let task_id = format!("task-{}", &digest[..8]);
    let out = IntakeTaskManifestV1 {
        schema_version: NODE_LIBRARY_VERSION.to_string(),
        task_id,
        objective: objective.to_string(),
        scope,
        authorized: true,
    };
    serde_json::to_string(&out).map_err(Into::into)
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// True for repository-relative paths that are test/spec sources.
fn is_test_path(p: &str) -> bool {
    let lower = p.to_lowercase();
    lower.contains("/test")
        || lower.contains("tests/")
        || lower.contains("/spec")
        || lower.ends_with("_test.rs")
        || lower.ends_with("_spec.rs")
        || lower.ends_with(".test.ts")
        || lower.ends_with(".spec.ts")
        || lower.ends_with("_test.py")
        || lower.ends_with("_spec.py")
}

fn run_discovery(args: &serde_json::Value) -> Result<String> {
    let root = args
        .get("repoRoot")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("repo-discovery requires a repoRoot string"))?;
    let idx = IndexedRepository::build(Path::new(root))?;
    let batch = RepoFactBatchV1::from_index(&idx)?;

    let mut languages: Vec<String> = idx.files.values().map(|f| f.language.clone()).collect();
    languages.sort();
    languages.dedup();

    let test_files: Vec<String> = idx
        .files
        .keys()
        .filter(|p| is_test_path(p))
        .cloned()
        .collect();

    let mut constraints = vec![
        format!("revision {}", idx.identity.revision),
        format!("{} languages indexed", languages.len()),
        "read-only: discovery performs no repository writes".to_string(),
    ];
    if idx.identity.dirty {
        constraints.push("worktree is dirty at discovery".to_string());
    }

    let files: Vec<DiscoveryFileV1> = idx
        .files
        .iter()
        .map(|(p, f)| DiscoveryFileV1 {
            path: p.clone(),
            language: f.language.clone(),
            sha256: f.sha256.clone(),
        })
        .collect();

    let out = DiscoveryResultV1 {
        schema_version: NODE_LIBRARY_VERSION.to_string(),
        revision: idx.identity.revision,
        dirty: idx.identity.dirty,
        file_count: files.len(),
        languages,
        test_files,
        constraints,
        fact_batch_digest: batch.batch_digest,
        files,
    };
    serde_json::to_string(&out).map_err(Into::into)
}

// ---------------------------------------------------------------------------
// Planning
// ---------------------------------------------------------------------------

fn run_planning(args: &serde_json::Value) -> Result<String> {
    let objective = args
        .get("objective")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("planning requires a string objective"))?;
    let discovery_json = args
        .get("discoveryEvidence")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            anyhow::anyhow!("planning requires discoveryEvidence (discovery result JSON)")
        })?;
    let discovery: DiscoveryResultV1 = serde_json::from_str(discovery_json)
        .map_err(|e| anyhow::anyhow!("planning: discovery evidence unparseable: {e}"))?;
    if discovery.files.is_empty() {
        bail!("planning rejected: discovery evidence records no files to plan against");
    }

    let source_files: Vec<String> = discovery
        .files
        .iter()
        .filter(|f| !is_test_path(&f.path))
        .map(|f| f.path.clone())
        .collect();
    let all_files: Vec<String> = discovery.files.iter().map(|f| f.path.clone()).collect();

    let steps = vec![
        PlanStepV1 {
            step: 1,
            title: "Confirm repository structure and constraints".to_string(),
            targets: all_files,
            evidence_ref: discovery.fact_batch_digest.clone(),
        },
        PlanStepV1 {
            step: 2,
            title: "Scope changes to authorized source files".to_string(),
            targets: source_files,
            evidence_ref: discovery.fact_batch_digest.clone(),
        },
        PlanStepV1 {
            step: 3,
            title: "Add or extend tests for changed behavior".to_string(),
            targets: discovery.test_files.clone(),
            evidence_ref: discovery.fact_batch_digest.clone(),
        },
    ];

    let out = ScopedPlanV1 {
        schema_version: NODE_LIBRARY_VERSION.to_string(),
        plan_id: format!("plan-{}", &discovery.fact_batch_digest[..8]),
        objective: objective.to_string(),
        discovery_revision: discovery.revision.clone(),
        discovery_evidence_id: discovery.fact_batch_digest.clone(),
        steps,
        assumptions: vec![
            "plan covers only files present at the discovery revision".to_string(),
            "no repository writes are performed by the planning node".to_string(),
        ],
    };
    serde_json::to_string(&out).map_err(Into::into)
}

// ---------------------------------------------------------------------------
// Registry + manifest helpers
// ---------------------------------------------------------------------------

/// Declare the three E5/I01 nodes. Safe to register alongside other node
/// registries; capability names are namespaced by the caller's manifest.
pub fn intake_discovery_planning_registry() -> CapabilityRegistry {
    let mut reg = CapabilityRegistry::new();
    reg.declare(
        CAP_INTAKE,
        Capability::deterministic(&["objective"], run_intake),
    );
    reg.declare(
        CAP_DISCOVERY,
        Capability::deterministic(&["repoRoot"], run_discovery),
    );
    reg.declare(
        CAP_PLANNING,
        Capability::deterministic(&["objective", "discoveryEvidence"], run_planning),
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

/// Build a read-only `lite.node.v1` manifest for one of the three nodes.
/// These nodes never write, so `writableScopes` is empty; `readableScopes`
/// carries the repository scope they may read.
pub fn node_manifest(node_id: &str, capability: &str) -> NodeManifestV1 {
    let (inputs, outputs) = match capability {
        CAP_INTAKE => (
            vec![
                io("objective", "core.String"),
                io("requestedScopes", "core.StringList"),
            ],
            vec![io("task", "lite.node.intake.TaskManifest")],
        ),
        CAP_DISCOVERY => (
            vec![io("repoRoot", "core.Path")],
            vec![io("discovery", "lite.node.discovery.Result")],
        ),
        _ => (
            vec![
                io("objective", "core.String"),
                io("discoveryEvidence", "lite.node.discovery.Result"),
            ],
            vec![io("plan", "lite.node.plan.ScopedPlan")],
        ),
    };
    NodeManifestV1::parse_json(
        &serde_json::json!({
            "schemaVersion": NODE_LIBRARY_VERSION,
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
    .expect("node library manifest is well-formed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intake_rejects_ambiguous_objective() {
        assert!(run_intake(&serde_json::json!({"objective": "thing"})).is_err());
        assert!(run_intake(&serde_json::json!({"objective": "make it better"})).is_err());
        assert!(run_intake(&serde_json::json!({"objective": "do the stuff"})).is_err());
    }

    #[test]
    fn intake_rejects_unauthorized_scope() {
        assert!(run_intake(&serde_json::json!({"objective": "refactor /etc/passwd"})).is_err());
        assert!(run_intake(&serde_json::json!({"objective": "fix C:/windows/system32"})).is_err());
        assert!(run_intake(&serde_json::json!({"objective": "migrate ../secrets"})).is_err());
    }

    #[test]
    fn intake_accepts_concrete_objective() {
        let out = run_intake(&serde_json::json!({"objective": "implement retry backoff for the http client", "requestedScopes": ["repo://fixture"]})).unwrap();
        let m: IntakeTaskManifestV1 = serde_json::from_str(&out).unwrap();
        assert!(m.authorized);
        assert_eq!(m.scope, vec!["repo://fixture"]);
        assert!(m.task_id.starts_with("task-"));
    }

    #[test]
    fn planning_rejects_when_discovery_empty() {
        let empty = DiscoveryResultV1 {
            schema_version: NODE_LIBRARY_VERSION.into(),
            revision: "r".into(),
            dirty: false,
            file_count: 0,
            languages: vec![],
            test_files: vec![],
            constraints: vec![],
            fact_batch_digest: "d".repeat(64),
            files: vec![],
        };
        let args = serde_json::json!({"objective": "implement feature", "discoveryEvidence": serde_json::to_string(&empty).unwrap()});
        assert!(run_planning(&args).is_err());
    }
}
