//! Governed node library for E5/I03 (#127): test discovery, isolated
//! validation, and infrastructure diagnostic nodes.
//!
//! OWNERSHIP: Lite-owned `lite.node` capability implementations. Every node
//! here is READ-ONLY with respect to the target repository:
//! - `test-discovery` scans the repo for project manifests and emits a
//!   list of candidate validation commands, each carrying a non-empty
//!   `why` and a deterministic `evidence_ref` (per issue #127 acceptance
//!   criterion 1: "records why each command was selected").
//! - `validation` will run those commands inside an isolated
//!   `GitWorktreeWorkspace` (lite.workspace.v1) and prove the source
//!   checkout is untouched (criterion 2) — added in PR2.
//! - `diagnostic` will classify each failed command into one of
//!   Code/Test/Environment/Timeout/Resource with evidence-backed
//!   provenance (criteria 3-4) — added in PR3.
//!
//! Each node is a `Capability` handler, so the generic nine-gate
//! `NodeRunner` (lite.node.v1 contracts, lite.policy.v1 authorization,
//! redaction, journal durability) drives them unchanged.

use std::path::Path;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::workflow::memory_contracts::canonical_digest;
use crate::workflow::node_contracts::NodeManifestV1;
use crate::workflow::node_runner::{Capability, CapabilityRegistry};

/// Version of the E5/I03 node contracts.
pub const NODE_VALIDATION_VERSION: &str = "1.0.0";

/// Declared capability names (registered in [`test_discovery_registry`]).
pub const CAP_TEST_DISCOVERY: &str = "test-discovery";

// ---------------------------------------------------------------------------
// Typed node outputs (`lite.node.test-discovery` family)
// ---------------------------------------------------------------------------

/// One discovered validation command with the rationale for its selection.
///
/// `why` is non-empty and is derived from the file or manifest evidence that
/// triggered the command (e.g. "Cargo.toml present at repo root — Rust
/// project"). `source` records the specific file or path that produced the
/// command. `evidence_ref` is the canonical digest of `{command, why,
/// source}` and ties the command back to the discovery evidence chain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationCommandV1 {
    pub command: String,
    pub args: Vec<String>,
    pub why: String,
    pub source: String,
    pub evidence_ref: String,
}

/// Output of the `test-discovery` node: a typed list of validation commands
/// with provenance and rationale.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestDiscoveryResultV1 {
    pub schema_version: String,
    pub repo_root: String,
    /// The repository revision at which discovery was performed (HEAD).
    pub revision: String,
    /// Stable, digest-bound id for this discovery run.
    pub discovery_id: String,
    /// Canonical digest over the emitted `commands` array — links
    /// downstream nodes (validation, diagnostic) back to this run.
    pub discovery_digest: String,
    pub commands: Vec<ValidationCommandV1>,
    /// Constraints observed during discovery (e.g. "read-only: discovery
    /// performs no repository writes").
    pub constraints: Vec<String>,
}

// ---------------------------------------------------------------------------
// Project-type detection
// ---------------------------------------------------------------------------

/// One project-type rule: when the listed evidence files are present at the
/// repo root, emit the listed command. The `why` field is rendered
/// deterministically from the rule label and the matched file.
struct DiscoveryRule {
    label: &'static str,
    /// File at the repo root whose presence triggers this rule.
    marker: &'static str,
    command: &'static str,
    args: &'static [&'static str],
    /// A non-empty `source` substring contributed when the rule fires.
    source_note: &'static str,
}

const RULES: &[DiscoveryRule] = &[
    DiscoveryRule {
        label: "rust",
        marker: "Cargo.toml",
        command: "cargo",
        args: &["test", "--offline"],
        source_note: "Cargo.toml",
    },
    DiscoveryRule {
        label: "node",
        marker: "package.json",
        command: "npm",
        args: &["test", "--ignore-scripts"],
        source_note: "package.json",
    },
    DiscoveryRule {
        label: "go",
        marker: "go.mod",
        command: "go",
        args: &["test", "./..."],
        source_note: "go.mod",
    },
    DiscoveryRule {
        label: "python-pytest",
        marker: "pyproject.toml",
        command: "pytest",
        args: &["-q"],
        source_note: "pyproject.toml",
    },
    DiscoveryRule {
        label: "make",
        marker: "Makefile",
        command: "make",
        args: &["test"],
        source_note: "Makefile",
    },
];

/// Head revision string of the working tree (best-effort; fall back to
/// "unknown" if git is unavailable — the discovery node does not require
/// git and must still emit a result).
fn head_revision(root: &Path) -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

// ---------------------------------------------------------------------------
// Capability handler
// ---------------------------------------------------------------------------

fn run_test_discovery(args: &serde_json::Value) -> Result<String> {
    let root = args
        .get("repoRoot")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("test-discovery requires a repoRoot string"))?;
    let root_path = Path::new(root);
    if !root_path.is_dir() {
        bail!("test-discovery: repoRoot is not a directory: {root}");
    }

    let mut commands = Vec::new();
    for rule in RULES {
        let marker_path = root_path.join(rule.marker);
        if !marker_path.is_file() {
            continue;
        }
        let why = format!(
            "{} project manifest present at repo root ({}): running `{} {}`",
            rule.label,
            rule.marker,
            rule.command,
            rule.args.join(" ")
        );
        let source = format!("{} (matched: {})", rule.source_note, rule.marker);
        let evidence_ref = canonical_digest(&serde_json::json!({
            "command": rule.command,
            "args": rule.args,
            "why": why,
            "source": source,
        }))?;
        commands.push(ValidationCommandV1 {
            command: rule.command.to_string(),
            args: rule.args.iter().map(|s| s.to_string()).collect(),
            why,
            source,
            evidence_ref,
        });
    }

    // Always record a meta-assertion: source-checkout must be clean before
    // any validation run. Listed as a "discoverable" command but tagged
    // with a non-zero priority so consumers can filter it out; here we
    // always include it so the constraint is recorded as evidence.
    let why_porcelain =
        "source checkout must be clean before validation (meta-assertion)".to_string();
    let source_porcelain = "git status".to_string();
    let evidence_ref_porcelain = canonical_digest(&serde_json::json!({
        "command": "git",
        "args": ["status", "--porcelain"],
        "why": why_porcelain,
        "source": source_porcelain,
    }))?;
    commands.push(ValidationCommandV1 {
        command: "git".to_string(),
        args: vec!["status".to_string(), "--porcelain".to_string()],
        why: why_porcelain,
        source: source_porcelain,
        evidence_ref: evidence_ref_porcelain,
    });

    let discovery_digest = canonical_digest(&serde_json::json!(&commands))?;
    let discovery_id = format!("disc-{}", &discovery_digest[..8]);
    let out = TestDiscoveryResultV1 {
        schema_version: NODE_VALIDATION_VERSION.to_string(),
        repo_root: root.to_string(),
        revision: head_revision(root_path),
        discovery_id,
        discovery_digest,
        commands,
        constraints: vec![
            format!("revision {}", head_revision(root_path)),
            "read-only: test-discovery performs no repository writes".to_string(),
        ],
    };
    serde_json::to_string(&out).map_err(Into::into)
}

// ---------------------------------------------------------------------------
// Registry + manifest helpers
// ---------------------------------------------------------------------------

/// Declare the test-discovery node. PR2 will add `validation` and PR3 will
/// add `diagnostic` to the same registry; the function is split per PR so
/// each PR is independently reviewable.
pub fn test_discovery_registry() -> CapabilityRegistry {
    let mut reg = CapabilityRegistry::new();
    reg.declare(
        CAP_TEST_DISCOVERY,
        Capability::deterministic(&["repoRoot"], run_test_discovery),
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

/// Build a read-only `lite.node.v1` manifest for the test-discovery node.
/// Like the rest of the E5 library, this node never writes, so
/// `writableScopes` is empty.
pub fn node_manifest(node_id: &str) -> NodeManifestV1 {
    NodeManifestV1::parse_json(
        &serde_json::json!({
            "schemaVersion": NODE_VALIDATION_VERSION,
            "nodeId": node_id,
            "purpose": CAP_TEST_DISCOVERY,
            "inputs": [io("repoRoot", "core.Path")],
            "outputs": [io("discovery", "lite.node.test-discovery.Result")],
            "readableScopes": ["repo://fixture"],
            "writableScopes": [],
            "retry": {"maxAttempts": 1, "retryableClasses": []}
        })
        .to_string(),
    )
    .expect("test-discovery manifest is well-formed")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_with_files(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, contents) in files {
            let full = dir.path().join(path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&full, contents).unwrap();
        }
        dir
    }

    #[test]
    fn rejects_missing_repo_root() {
        let args = serde_json::json!({});
        assert!(run_test_discovery(&args).is_err());
    }

    #[test]
    fn rejects_non_directory_repo_root() {
        let dir = tempfile::tempdir().unwrap();
        let bogus = dir.path().join("not-a-dir");
        let args = serde_json::json!({"repoRoot": bogus.to_string_lossy()});
        assert!(run_test_discovery(&args).is_err());
    }

    #[test]
    fn empty_repo_records_only_meta_assertion_command() {
        let dir = fixture_with_files(&[]);
        let args = serde_json::json!({"repoRoot": dir.path().to_string_lossy()});
        let out = run_test_discovery(&args).unwrap();
        let parsed: TestDiscoveryResultV1 = serde_json::from_str(&out).unwrap();
        // Empty repo → only the git-status meta-assertion is recorded.
        assert_eq!(parsed.commands.len(), 1);
        assert_eq!(parsed.commands[0].command, "git");
        assert!(!parsed.commands[0].why.is_empty());
        assert!(!parsed.commands[0].evidence_ref.is_empty());
    }

    #[test]
    fn cargo_manifest_produces_cargo_test_with_why() {
        let dir = fixture_with_files(&[("Cargo.toml", "[package]\nname = \"x\"\n")]);
        let args = serde_json::json!({"repoRoot": dir.path().to_string_lossy()});
        let out = run_test_discovery(&args).unwrap();
        let parsed: TestDiscoveryResultV1 = serde_json::from_str(&out).unwrap();
        // Cargo rule + meta-assertion.
        assert_eq!(parsed.commands.len(), 2);
        let cargo = parsed
            .commands
            .iter()
            .find(|c| c.command == "cargo")
            .expect("cargo command present");
        assert_eq!(cargo.args, vec!["test", "--offline"]);
        assert!(cargo.why.contains("Cargo.toml"));
        assert!(cargo.why.contains("cargo test --offline"));
        assert!(cargo.source.contains("Cargo.toml"));
        // evidence_ref is 64 lowercase hex.
        assert_eq!(cargo.evidence_ref.len(), 64);
        assert!(cargo.evidence_ref.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn discovery_is_deterministic() {
        let dir = fixture_with_files(&[("Cargo.toml", "[package]\nname = \"x\"\n")]);
        let args = serde_json::json!({"repoRoot": dir.path().to_string_lossy()});
        let a = run_test_discovery(&args).unwrap();
        let b = run_test_discovery(&args).unwrap();
        // Same inputs → identical command-level evidence_refs and
        // discovery_digest.
        let pa: TestDiscoveryResultV1 = serde_json::from_str(&a).unwrap();
        let pb: TestDiscoveryResultV1 = serde_json::from_str(&b).unwrap();
        assert_eq!(pa.discovery_digest, pb.discovery_digest);
        for (ca, cb) in pa.commands.iter().zip(pb.commands.iter()) {
            assert_eq!(ca.evidence_ref, cb.evidence_ref);
        }
    }
}
