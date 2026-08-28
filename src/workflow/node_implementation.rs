//! Governed implementation and repair nodes for E5/I02 (#126).
//!
//! OWNERSHIP: Lite-owned `lite.node` capability implementations that perform
//! REPOSITORY WRITES — the executor half of the E5 node library. Unlike the
//! read-only intake/discovery/planning nodes (#125), these nodes acquire an
//! isolated, revision-pinned `GitWorktreeWorkspace` (lite.workspace.v1, issue
//! #171) and commit their changes there, so the source checkout is NEVER
//! touched. Each write is recorded as a durable, reviewable change artifact and
//! emitted as governed evidence (worktree revision + portable workspace ref).
//!
//! Concrete source edits in production are supplied by a provider; the node's
//! governed responsibility here is the write pipeline itself: acquire isolation
//! under authority, record each planned/diagnosed change as an evidence file,
//! commit on the detached worktree, and emit a typed result linked back to the
//! plan / diagnosis. Driven unchanged by the generic nine-gate `NodeRunner`.

use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

use crate::workflow::node_contracts::NodeManifestV1;
use crate::workflow::node_library::ScopedPlanV1;
use crate::workflow::node_runner::{Capability, CapabilityRegistry};
use crate::workflow::workspace::{
    AdapterKind, GitWorktreeWorkspace, WORKSPACE_SCHEMA_VERSION, WorkspaceAdapter,
    WorkspaceManifestV1, WorkspaceMode,
};

/// Version of the E5/I02 node contracts.
pub const NODE_IMPL_VERSION: &str = "1.0.0";

/// Declared capability names.
pub const CAP_IMPLEMENT: &str = "implement";
pub const CAP_REPAIR: &str = "repair";

const ADAPTER_REVISION: &str = "lite.workspace.adapter.v1";

// ---------------------------------------------------------------------------
// Typed inputs / outputs
// ---------------------------------------------------------------------------

/// A diagnosis that the repair node turns into a corrective change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosisV1 {
    pub diagnosis_id: String,
    /// File or symbol the failure was observed against.
    pub failing_target: String,
    pub message: String,
    /// Revision the repair worktree must be pinned to.
    pub base_revision: String,
}

/// One recorded implementation change (committed evidence in the worktree).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImplementationChangeV1 {
    pub step: u32,
    pub title: String,
    pub targets: Vec<String>,
    pub evidence_ref: String,
    pub applied_at: String,
}

/// Output of the `implement` node: a committed, evidence-linked change set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImplementationResultV1 {
    pub schema_version: String,
    pub plan_id: String,
    pub discovery_evidence_id: String,
    /// New HEAD of the isolated worktree after commit (proves a write).
    pub revision: String,
    /// Portable workspace reference (durable identity for resume/evidence).
    pub workspace_ref: String,
    pub changed_files: Vec<String>,
    pub changes: Vec<ImplementationChangeV1>,
}

/// Output of the `repair` node: a corrective change set linked to its diagnosis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairResultV1 {
    pub schema_version: String,
    pub repair_id: String,
    pub diagnosis_ref: String,
    pub failing_target: String,
    pub revision: String,
    pub workspace_ref: String,
    pub changed_files: Vec<String>,
    pub corrective_summary: String,
}

// ---------------------------------------------------------------------------
// Workspace helpers (deterministic git, isolated worktree)
// ---------------------------------------------------------------------------

fn now_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    chrono_like_iso(secs)
}

/// Minimal ISO-8601 UTC timestamp without pulling a chrono dependency into the
/// node surface (the workspace/evaluate helpers already standardize on this).
#[allow(clippy::manual_is_multiple_of)]
fn chrono_like_iso(secs: u64) -> String {
    let days = secs / 86400;
    // Proleptic Gregorian day math; sufficient for an audit timestamp.
    let mut y = 1970 + (days / 365);
    let mut d = days;
    loop {
        let leap = if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
            366
        } else {
            365
        };
        if d < leap {
            break;
        }
        d -= leap;
        y += 1;
    }
    let month_days = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let leap = if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
        1
    } else {
        0
    };
    let mut m = 0;
    let mut rem = d;
    while m < 12 {
        let md = month_days[m] + if m == 1 { leap } else { 0 };
        if rem < md {
            break;
        }
        rem -= md;
        m += 1;
    }
    let day = rem + 1;
    let month = m + 1;
    let hour = (secs % 86400) / 3600;
    let min = (secs % 3600) / 60;
    let sec = secs % 60;
    format!("{y:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .context("git invocation failed")?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn acquire_writable_worktree(
    repo_root: &str,
    workspace_parent: &str,
    workspace_id: &str,
    base_revision: &str,
) -> Result<(
    GitWorktreeWorkspace,
    crate::workflow::workspace::WorkspaceManifestV1,
    std::path::PathBuf,
)> {
    let adapter = GitWorktreeWorkspace {
        parent_dir: Path::new(workspace_parent).to_path_buf(),
        source_repo: Path::new(repo_root).to_path_buf(),
    };
    let manifest = WorkspaceManifestV1 {
        schema_version: WORKSPACE_SCHEMA_VERSION.to_string(),
        workspace_id: workspace_id.to_string(),
        adapter: AdapterKind::GitWorktree,
        adapter_revision: ADAPTER_REVISION.to_string(),
        repo_identity: repo_root.to_string(),
        base_revision: base_revision.to_string(),
        branch: None,
        mode: WorkspaceMode::Writable,
        writable_scopes: vec!["repo://fixture".to_string()],
        resource_lock_id: format!("lock-{workspace_id}"),
        created_at: now_iso(),
        content_digest: None,
    }
    .sealed();
    let ws = adapter.acquire(&manifest)?;
    Ok((adapter, manifest, ws.root))
}

// ---------------------------------------------------------------------------
// Implement node
// ---------------------------------------------------------------------------

fn run_implement(args: &serde_json::Value) -> Result<String> {
    let plan_json = args
        .get("plan")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("implement requires a plan (ScopedPlanV1 JSON)"))?;
    let plan: ScopedPlanV1 = serde_json::from_str(plan_json)
        .map_err(|e| anyhow::anyhow!("implement: plan unparseable: {e}"))?;
    let repo_root = args
        .get("repoRoot")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("implement requires repoRoot"))?;
    let workspace_parent = args
        .get("workspaceParent")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("implement requires workspaceParent"))?;

    let workspace_id = format!("impl-{}", plan.plan_id);
    let (_adapter, manifest, root) = acquire_writable_worktree(
        repo_root,
        workspace_parent,
        &workspace_id,
        &plan.discovery_revision,
    )?;

    let changes_dir = root.join("prometheos").join("changes").join(&plan.plan_id);
    std::fs::create_dir_all(&changes_dir)?;

    let mut changes = Vec::new();
    for step in &plan.steps {
        let change = ImplementationChangeV1 {
            step: step.step,
            title: step.title.clone(),
            targets: step.targets.clone(),
            evidence_ref: step.evidence_ref.clone(),
            applied_at: now_iso(),
        };
        std::fs::write(
            changes_dir.join(format!("{}.change.json", step.step)),
            serde_json::to_string_pretty(&change)?,
        )?;
        changes.push(change);
    }

    git(&root, &["add", "-A"])?;
    git(
        &root,
        &[
            "commit",
            "-q",
            "-m",
            &format!("prometheos: implement plan {}", plan.plan_id),
        ],
    )?;
    let revision = git(&root, &["rev-parse", "HEAD"])?;
    let workspace_ref = manifest.to_reference().to_json()?;
    let changed_files: Vec<String> = changes
        .iter()
        .map(|c| format!("prometheos/changes/{}/{}.change.json", plan.plan_id, c.step))
        .collect();

    let out = ImplementationResultV1 {
        schema_version: NODE_IMPL_VERSION.to_string(),
        plan_id: plan.plan_id.clone(),
        discovery_evidence_id: plan.discovery_evidence_id.clone(),
        revision,
        workspace_ref,
        changed_files,
        changes,
    };
    serde_json::to_string(&out).map_err(Into::into)
}

// ---------------------------------------------------------------------------
// Repair node
// ---------------------------------------------------------------------------

fn run_repair(args: &serde_json::Value) -> Result<String> {
    let diagnosis_json = args
        .get("diagnosis")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("repair requires a diagnosis (DiagnosisV1 JSON)"))?;
    let diagnosis: DiagnosisV1 = serde_json::from_str(diagnosis_json)
        .map_err(|e| anyhow::anyhow!("repair: diagnosis unparseable: {e}"))?;
    let repo_root = args
        .get("repoRoot")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("repair requires repoRoot"))?;
    let workspace_parent = args
        .get("workspaceParent")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("repair requires workspaceParent"))?;

    let repair_id = format!("repair-{}", diagnosis.diagnosis_id);
    let (_adapter, manifest, root) = acquire_writable_worktree(
        repo_root,
        workspace_parent,
        &repair_id,
        &diagnosis.base_revision,
    )?;

    let repairs_dir = root.join("prometheos").join("repairs");
    std::fs::create_dir_all(&repairs_dir)?;
    let corrective_summary = format!(
        "applied corrective change for {}: {}",
        diagnosis.failing_target, diagnosis.message
    );
    let record = serde_json::json!({
        "repairId": repair_id,
        "diagnosisId": diagnosis.diagnosis_id,
        "failingTarget": diagnosis.failing_target,
        "summary": corrective_summary,
        "appliedAt": now_iso(),
    });
    std::fs::write(
        repairs_dir.join(format!("{}.repair.json", diagnosis.diagnosis_id)),
        serde_json::to_string_pretty(&record)?,
    )?;

    git(&root, &["add", "-A"])?;
    git(
        &root,
        &[
            "commit",
            "-q",
            "-m",
            &format!("prometheos: repair {}", diagnosis.diagnosis_id),
        ],
    )?;
    let revision = git(&root, &["rev-parse", "HEAD"])?;
    let workspace_ref = manifest.to_reference().to_json()?;
    let changed_files = vec![format!(
        "prometheos/repairs/{}.repair.json",
        diagnosis.diagnosis_id
    )];

    let out = RepairResultV1 {
        schema_version: NODE_IMPL_VERSION.to_string(),
        repair_id,
        diagnosis_ref: diagnosis.diagnosis_id.clone(),
        failing_target: diagnosis.failing_target.clone(),
        revision,
        workspace_ref,
        changed_files,
        corrective_summary,
    };
    serde_json::to_string(&out).map_err(Into::into)
}

// ---------------------------------------------------------------------------
// Registry + manifest helper
// ---------------------------------------------------------------------------

/// Declare the E5/I02 writing nodes. Safe to register alongside the read-only
/// node library; capability names are namespaced by the caller's manifest.
pub fn implementation_repair_registry() -> CapabilityRegistry {
    let mut reg = CapabilityRegistry::new();
    reg.declare(
        CAP_IMPLEMENT,
        Capability::deterministic(&["plan", "repoRoot", "workspaceParent"], run_implement),
    );
    reg.declare(
        CAP_REPAIR,
        Capability::deterministic(&["diagnosis", "repoRoot", "workspaceParent"], run_repair),
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

/// Build a writable `lite.node.v1` manifest for one of the two nodes. These
/// nodes write, so `writableScopes` carries the granted repository scope.
pub fn node_manifest(node_id: &str, capability: &str) -> NodeManifestV1 {
    let (inputs, outputs) = match capability {
        CAP_IMPLEMENT => (
            vec![
                io("plan", "lite.node.plan.ScopedPlan"),
                io("repoRoot", "core.Path"),
                io("workspaceParent", "core.Path"),
            ],
            vec![io("implementation", "lite.node.implement.Result")],
        ),
        _ => (
            vec![
                io("diagnosis", "lite.node.repair.Diagnosis"),
                io("repoRoot", "core.Path"),
                io("workspaceParent", "core.Path"),
            ],
            vec![io("repair", "lite.node.repair.Result")],
        ),
    };
    NodeManifestV1::parse_json(
        &serde_json::json!({
            "schemaVersion": "1.0.0",
            "nodeId": node_id,
            "purpose": capability,
            "inputs": inputs,
            "outputs": outputs,
            "readableScopes": ["repo://fixture"],
            "writableScopes": ["repo://fixture"],
            "retry": {"maxAttempts": 1, "retryableClasses": []}
        })
        .to_string(),
    )
    .expect("implementation/repair node manifest is well-formed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implement_rejects_missing_plan() {
        assert!(
            run_implement(&serde_json::json!({"repoRoot": ".", "workspaceParent": "."})).is_err()
        );
    }

    #[test]
    fn repair_rejects_missing_diagnosis() {
        assert!(run_repair(&serde_json::json!({"repoRoot": ".", "workspaceParent": "."})).is_err());
    }

    #[test]
    fn manifest_declares_writable_scope() {
        let m = node_manifest("node.impl", CAP_IMPLEMENT);
        assert_eq!(m.writable_scopes, vec!["repo://fixture".to_string()]);
        assert_eq!(m.readable_scopes, vec!["repo://fixture".to_string()]);
    }
}
