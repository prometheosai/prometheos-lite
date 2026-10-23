//! E6/I02 (#131): the developer-facing graph-run inspector and the human
//! gate decision interface.
//!
//! Slice A of #131 shipped the read-only inspector for the FILE-backed repo
//! workbench (`prometheos work inspect`). This module is Slice B: the
//! inspector for GOVERNED GRAPH RUNS — the durable `graph_checkpoints`
//! registry — plus the explicit human decision actions (approve, reject,
//! request-changes) that #131 requires.
//!
//! Law (E6/I02 acceptance):
//! - A decision is only ever a DURABLE `HumanDecisionRecordV1`:
//!   actor, timestamp, target gate, cited evidence digest, rationale, all
//!   sealed with a content digest. Machine actors can never author one.
//! - A decision may only be recorded against a gate node DECLARED by the
//!   run's graph manifest, and only against evidence that is still ANCHORED
//!   in the run's current state. A stale basis is refused, not recorded.
//! - Writes are compare-and-swap against the checkpoint digest that was
//!   read, and land in the same transaction as the journal event. A
//!   concurrent writer loses the race; nothing is half-written.
//! - Inspection NEVER mutates and never implies a guarantee it did not
//!   check: without a manifest the manifest binding is reported as
//!   `unverified`, not as verified.
//!
//! Recording a decision does NOT route the run. The runtime is the
//! execution authority: it reads the recorded verdict and routes the gate
//! from it. The CLI never advances a run's frontier on its own.

use anyhow::{Context as _, Result};
use serde::Serialize;
use std::path::Path;
use std::sync::Arc;

use prometheos_lite::db::Db;
use prometheos_lite::db::repository::graph_checkpoints;
use prometheos_lite::work::human_principal::{self, VerifiedHumanPrincipal};
use prometheos_lite::work::provenance::JournalContext;
use prometheos_lite::work::types::WorkContext;
use prometheos_lite::workflow::graph_gates::{
    HumanDecisionRecordV1, HumanVerdict, ReviewChannel, decision_basis_is_anchored, is_gate_node,
    record_gate_decision,
};
use prometheos_lite::workflow::graph_state::{GraphManifestV1, GraphRunStateV1};

/// Schema version of the inspection report document.
pub const INSPECTION_SCHEMA_VERSION: &str = "1.0.0";

/// How the run's graph manifest binding was established.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ManifestBinding {
    /// The checkpoint's `graphManifestDigest` matches the supplied manifest.
    Verified,
    /// No manifest was supplied: the binding was NOT checked.
    Unverified,
}

// ---------------------------------------------------------------------------
// Report views
// ---------------------------------------------------------------------------

/// One node's attempts, newest last.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeAttemptView {
    pub node_id: String,
    pub attempts: Vec<AttemptView>,
    /// Digest of the LATEST attempt — the artifact a reviewer would read.
    pub latest_result_digest: Option<String>,
}

/// One recorded node attempt.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptView {
    pub attempt: u32,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub outcome: String,
    pub result_digest: String,
}

/// One recorded route decision (the run's route history).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteView {
    pub recorded_at: String,
    pub from_node: String,
    pub to_node: String,
    pub condition_label: Option<String>,
    pub basis_result_digest: String,
}

/// One durable human gate decision, as the reviewer sees it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GateDecisionView {
    pub gate_node_id: String,
    pub verdict: String,
    /// The human who decided.
    pub decided_by: String,
    pub channel: String,
    /// Digest of the evidence the human reviewed.
    pub basis_evidence_digest: String,
    pub reason: String,
    pub recorded_at: String,
    pub content_digest: Option<String>,
    /// Whether the cited evidence is still anchored in the run. A false here
    /// means the decision is STALE and the gate needs a fresh decision.
    pub evidence_current: bool,
    /// Human-readable reason the decision is stale (absent when current).
    pub stale_reason: Option<String>,
}

/// One durable evidence reference.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceRefView {
    pub id: String,
    pub event_digest: String,
    pub artifact_digest: String,
    pub artifact_kind: String,
    pub produced_by: String,
    pub produced_at: Option<String>,
}

/// The whole-run inspector report.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspection {
    pub schema_version: String,
    pub work_context_id: String,
    pub run_id: String,
    pub graph_id: String,
    pub graph_manifest_digest: String,
    pub repo_revision: String,
    pub portable_state_ref: String,
    pub portable_state_digest: String,
    /// Chain digest sealed into the run state.
    pub chain_digest: Option<String>,
    /// Digest of the stored checkpoint blob (recomputed on read).
    pub checkpoint_digest: String,
    pub manifest_binding: ManifestBinding,
    pub frontier: Vec<String>,
    pub node_attempts: Vec<NodeAttemptView>,
    pub route_history: Vec<RouteView>,
    pub gate_decisions: Vec<GateDecisionView>,
    pub evidence_refs: Vec<EvidenceRefView>,
    pub termination: Option<serde_json::Value>,
    pub implementation_changes: Vec<serde_json::Value>,
}

/// What a recorded decision did.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionOutcome {
    pub work_context_id: String,
    pub run_id: String,
    pub gate_node_id: String,
    pub verdict: String,
    pub decided_by: String,
    pub channel: String,
    pub basis_evidence_digest: String,
    pub reason: String,
    pub recorded_at: String,
    /// Digest of the sealed decision record.
    pub decision_digest: Option<String>,
    /// Digest of the checkpoint after the decision landed.
    pub new_checkpoint_digest: String,
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// List the graph runs registered for a work context.
pub fn list_runs(
    db: &Arc<Db>,
    user_id: &str,
    work_context_id: &str,
) -> Result<Vec<graph_checkpoints::GraphCheckpointRef>> {
    graph_checkpoints::list_checkpoints(&**db, user_id, work_context_id)
}

/// Inspect one registered graph run. Read-only: never mutates state.
///
/// The manifest is optional. When it is supplied, the run is imported
/// through the FULL checkpoint law (manifest binding + portable-state
/// reconciliation + frontier structure). When it is not, only the chain
/// digest is verified and the manifest binding is reported as
/// `unverified` — the report never claims a check that did not run.
pub fn inspect_run(
    db: &Arc<Db>,
    user_id: &str,
    work_context_id: &str,
    run_id: &str,
    manifest: Option<&GraphManifestV1>,
) -> Result<RunInspection> {
    let (blob, checkpoint_digest) = load_checkpoint(db, user_id, work_context_id, run_id)?;

    let (state, manifest_binding) = match manifest {
        Some(m) => {
            // The portable-state pointer is only ever self-referenced here
            // (the CLI has no external portable-state store), so the
            // reconciliation checks self-consistency, not external truth.
            let peek = GraphRunStateV1::import_for_inspection(&blob)?;
            let state = GraphRunStateV1::import_checkpoint(&blob, m, &peek.portable_state_digest)?;
            (state, ManifestBinding::Verified)
        }
        None => (
            GraphRunStateV1::import_for_inspection(&blob)?,
            ManifestBinding::Unverified,
        ),
    };

    Ok(build_inspection(
        work_context_id,
        run_id,
        checkpoint_digest,
        manifest_binding,
        &state,
    ))
}

fn load_checkpoint(
    db: &Arc<Db>,
    user_id: &str,
    work_context_id: &str,
    run_id: &str,
) -> Result<(String, String)> {
    graph_checkpoints::get_checkpoint(&**db, user_id, work_context_id, run_id)?.ok_or_else(|| {
        anyhow::anyhow!(
            "no graph run '{run_id}' is registered for work context '{work_context_id}'"
        )
    })
}

fn build_inspection(
    work_context_id: &str,
    run_id: &str,
    checkpoint_digest: String,
    manifest_binding: ManifestBinding,
    state: &GraphRunStateV1,
) -> RunInspection {
    let node_attempts = state
        .node_attempts
        .iter()
        .map(|(node_id, attempts)| NodeAttemptView {
            node_id: node_id.clone(),
            attempts: attempts
                .iter()
                .map(|a| AttemptView {
                    attempt: a.attempt,
                    started_at: a.started_at.clone(),
                    completed_at: a.completed_at.clone(),
                    outcome: format!("{:?}", a.outcome).to_lowercase(),
                    result_digest: a.result_digest.clone(),
                })
                .collect(),
            latest_result_digest: attempts.last().map(|a| a.result_digest.clone()),
        })
        .collect();

    let route_history = state
        .decisions
        .iter()
        .map(|d| RouteView {
            recorded_at: d.recorded_at.clone(),
            from_node: d.from_node.clone(),
            to_node: d.to_node.clone(),
            condition_label: d.condition_label.clone(),
            basis_result_digest: d.basis_result_digest.clone(),
        })
        .collect();

    let gate_decisions = state
        .gate_decisions
        .iter()
        .map(|(node_id, d)| {
            let current = decision_basis_is_anchored(state, &d.basis_evidence_digest);
            GateDecisionView {
                gate_node_id: node_id.clone(),
                verdict: verdict_label(d.verdict).to_string(),
                decided_by: d.decided_by.clone(),
                channel: channel_label(&d.channel).to_string(),
                basis_evidence_digest: d.basis_evidence_digest.clone(),
                reason: d.reason.clone(),
                recorded_at: d.recorded_at.clone(),
                content_digest: d.content_digest.clone(),
                evidence_current: current,
                stale_reason: if current {
                    None
                } else {
                    Some(format!(
                        "cited evidence {} is no longer anchored in the run; re-decide against the current evidence",
                        d.basis_evidence_digest
                    ))
                },
            }
        })
        .collect();

    let evidence_refs = state
        .evidence_refs
        .iter()
        .map(|r| EvidenceRefView {
            id: r.id.clone(),
            event_digest: r.event_digest.clone(),
            artifact_digest: r.artifact_digest.clone(),
            artifact_kind: r.artifact_kind.clone(),
            produced_by: r.produced_by.clone(),
            produced_at: r.produced_at.clone(),
        })
        .collect();

    RunInspection {
        schema_version: INSPECTION_SCHEMA_VERSION.into(),
        work_context_id: work_context_id.to_string(),
        run_id: run_id.to_string(),
        graph_id: state.graph_id.clone(),
        graph_manifest_digest: state.graph_manifest_digest.clone(),
        repo_revision: state.repo_revision.clone(),
        portable_state_ref: state.portable_state_ref.clone(),
        portable_state_digest: state.portable_state_digest.clone(),
        chain_digest: state.content_digest.clone(),
        checkpoint_digest,
        manifest_binding,
        frontier: state.frontier.clone(),
        node_attempts,
        route_history,
        gate_decisions,
        evidence_refs,
        termination: state
            .termination
            .as_ref()
            .map(|t| serde_json::to_value(t).unwrap_or(serde_json::Value::Null)),
        implementation_changes: state
            .implementation_changes
            .iter()
            .map(|c| serde_json::to_value(c).unwrap_or(serde_json::Value::Null))
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// Deciding
// ---------------------------------------------------------------------------

/// Record a durable human gate decision on a governed graph run.
///
/// Fail-closed at every step:
/// 1. the run must be registered and readable by this principal;
/// 2. the run must not be terminated;
/// 3. the gate node must be DECLARED as a gate by the run's manifest;
/// 4. the cited basis must still be ANCHORED in the run's current state;
/// 5. the decision names a VERIFIED HUMAN PRINCIPAL — the caller cannot
///    supply the actor identity; it is established by the runtime from
///    the OS session and the working directory's trust anchor
///    (`work::human_principal`), and refused when it cannot be verified;
/// 6. the identity must be a HUMAN (machine actors are refused);
/// 7. the write is a compare-and-swap against the digest that was read.
pub fn decide_run(
    db: &Arc<Db>,
    context: &WorkContext,
    user_id: &str,
    run_id: &str,
    manifest: &GraphManifestV1,
    gate_node_id: &str,
    verdict: HumanVerdict,
    principal: &VerifiedHumanPrincipal,
    basis: &str,
    reason: &str,
) -> Result<DecisionOutcome> {
    let work_context_id = context.id.as_str();
    let (blob, stored_digest) = load_checkpoint(db, user_id, work_context_id, run_id)?;

    // Full import law: manifest binding + chain digest + frontier structure.
    let peek = GraphRunStateV1::import_for_inspection(&blob)?;
    let mut state =
        GraphRunStateV1::import_checkpoint(&blob, manifest, &peek.portable_state_digest)?;

    if state.termination.is_some() {
        anyhow::bail!("graph run '{run_id}' is already terminated");
    }
    if !is_gate_node(manifest, gate_node_id) {
        anyhow::bail!(
            "'{gate_node_id}' is not a declared gate node in the manifest for graph '{}'",
            manifest.graph_id
        );
    }
    if !decision_basis_is_anchored(&state, basis) {
        anyhow::bail!(
            "basis evidence {basis} is not anchored in the current state of run '{run_id}'; \
             decide against evidence the run still holds"
        );
    }

    let recorded_at = chrono::Utc::now().to_rfc3339();
    // The recorded actor is the VERIFIED principal — never a value the
    // caller supplied.
    let actor = principal.identity();
    let decision = HumanDecisionRecordV1::author(
        gate_node_id,
        verdict,
        actor,
        ReviewChannel::CliInteractive,
        basis,
        reason,
        recorded_at.clone(),
    )?;

    record_gate_decision(&mut state, manifest, decision.clone())?;
    let new_checkpoint = state.export_checkpoint()?;
    let new_digest =
        prometheos_lite::workflow::soma::canonical::sha256_hex(new_checkpoint.as_bytes());

    // The checkpoint write and the journal event land in ONE transaction: a
    // failed event write rolls the checkpoint back.
    let conn = db.conn();
    let tx = conn
        .unchecked_transaction()
        .context("failed to start transaction")?;
    let swapped = graph_checkpoints::cas_checkpoint_conn(
        &tx,
        user_id,
        work_context_id,
        run_id,
        &stored_digest,
        &new_checkpoint,
    )?;
    if !swapped {
        anyhow::bail!(
            "checkpoint digest raced: another writer changed run '{run_id}'; the decision was not recorded"
        );
    }

    // The durable journal records the VERIFIED human as the principal and
    // the producer — the schema CHECK ties the principal_id column to the
    // envelope's principal.human.identity, so the durable record cannot
    // claim a different human than the one that was verified.
    let journal = JournalContext::for_request(
        actor,
        format!("cli-{}", uuid::Uuid::new_v4()),
        JournalContext::work_authority(context.autonomy_level, context.approval_policy),
    );
    let envelope = journal.graph_run_envelope(run_id.to_string(), None);
    let event = prometheos_lite::work::event::WorkContextEvent::new(
        uuid::Uuid::new_v4().to_string(),
        work_context_id.to_string(),
        "graph_decision".to_string(),
        serde_json::json!({
            "runId": run_id,
            "gateNodeId": gate_node_id,
            "verdict": verdict_label(verdict),
            "decidedBy": actor,
            "basisEvidenceDigest": basis,
            "reason": reason,
            "recordedAt": recorded_at,
            "decisionDigest": decision.content_digest,
            "newDigest": new_digest,
        }),
    );
    prometheos_lite::db::repository::work_context_events::record_event_conn(
        &tx, &event, &envelope,
    )?;
    tx.commit()?;

    Ok(DecisionOutcome {
        work_context_id: work_context_id.to_string(),
        run_id: run_id.to_string(),
        gate_node_id: gate_node_id.to_string(),
        verdict: verdict_label(verdict).to_string(),
        decided_by: actor.to_string(),
        channel: channel_label(&ReviewChannel::CliInteractive).to_string(),
        basis_evidence_digest: basis.to_string(),
        reason: reason.to_string(),
        recorded_at,
        decision_digest: decision.content_digest,
        new_checkpoint_digest: new_digest,
    })
}

// ---------------------------------------------------------------------------
// Labels + rendering
// ---------------------------------------------------------------------------

/// Resolve and VERIFY the human principal for this session against the
/// working directory's trust anchor.
///
/// This is the only way the CLI obtains an actor identity: the caller
/// cannot pass one. When the OS session identity cannot be established
/// or the anchor does not list it, this REFUSES.
pub fn resolve_human_principal(repo_root: &Path) -> Result<VerifiedHumanPrincipal> {
    human_principal::verify_human_principal(&human_principal::default_anchor_path(repo_root))
}

/// The CLI spelling of a verdict.
pub fn verdict_label(verdict: HumanVerdict) -> &'static str {
    match verdict {
        HumanVerdict::Approved => "approve",
        HumanVerdict::ChangesRequested => "request-changes",
        HumanVerdict::Rejected => "reject",
    }
}

/// Parse a CLI verdict. Anything else is refused — a decision is never
/// inferred from a typo.
pub fn parse_verdict(input: &str) -> Result<HumanVerdict> {
    match input.trim().to_ascii_lowercase().as_str() {
        "approve" | "approved" => Ok(HumanVerdict::Approved),
        "reject" | "rejected" => Ok(HumanVerdict::Rejected),
        "request-changes" | "changes-requested" | "changes_requested" => {
            Ok(HumanVerdict::ChangesRequested)
        }
        other => {
            anyhow::bail!("unknown verdict '{other}': expected approve, reject, or request-changes")
        }
    }
}

fn channel_label(channel: &ReviewChannel) -> &'static str {
    match channel {
        ReviewChannel::CliInteractive => "cli-interactive",
        ReviewChannel::ExternalSystem { .. } => "external-system",
    }
}

/// Load a graph manifest from a file (the run's workflow definition).
pub fn load_manifest(path: &Path) -> Result<GraphManifestV1> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read graph manifest {}", path.display()))?;
    let manifest: GraphManifestV1 = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse graph manifest {}", path.display()))?;
    manifest
        .validate()
        .with_context(|| format!("graph manifest {} is invalid", path.display()))?;
    Ok(manifest)
}

/// Render the inspection report for a terminal reader.
pub fn print_inspection_text(report: &RunInspection) {
    println!("Graph run: {}", report.run_id);
    println!("  WorkContext: {}", report.work_context_id);
    println!(
        "  Graph: {} @ {}",
        report.graph_id, report.graph_manifest_digest
    );
    println!("  Repo revision: {}", report.repo_revision);
    println!(
        "  Portable state: {} ({})",
        report.portable_state_ref, report.portable_state_digest
    );
    println!(
        "  Chain digest: {}",
        report.chain_digest.as_deref().unwrap_or("-")
    );
    println!("  Checkpoint digest: {}", report.checkpoint_digest);
    println!(
        "  Manifest binding: {}",
        manifest_binding_label(report.manifest_binding)
    );
    if let Some(t) = &report.termination {
        println!("  Termination: {}", t);
    }
    println!(
        "  Frontier: {}",
        if report.frontier.is_empty() {
            "(empty)".to_string()
        } else {
            report.frontier.join(", ")
        }
    );

    println!("\nNode attempts:");
    if report.node_attempts.is_empty() {
        println!("  (none)");
    }
    for node in &report.node_attempts {
        println!("  {}", node.node_id);
        for a in &node.attempts {
            println!(
                "    attempt {} [{}] {} -> {}",
                a.attempt,
                a.outcome,
                a.started_at,
                a.completed_at.as_deref().unwrap_or("-")
            );
            println!("      result: {}", a.result_digest);
        }
    }

    println!("\nRoute history:");
    if report.route_history.is_empty() {
        println!("  (none)");
    }
    for r in &report.route_history {
        let label = r.condition_label.as_deref().unwrap_or("-");
        println!(
            "  {} -> {} [{}] basis {}",
            r.from_node, r.to_node, label, r.basis_result_digest
        );
    }

    println!("\nHuman gate decisions:");
    if report.gate_decisions.is_empty() {
        println!("  (none)");
    }
    for d in &report.gate_decisions {
        println!(
            "  {} — {} by {} via {}",
            d.gate_node_id, d.verdict, d.decided_by, d.channel
        );
        println!("    recorded: {}", d.recorded_at);
        println!("    basis:    {}", d.basis_evidence_digest);
        println!("    rationale: {}", d.reason);
        if let Some(c) = &d.content_digest {
            println!("    decision: {c}");
        }
        if !d.evidence_current {
            println!("    STALE: {}", d.stale_reason.as_deref().unwrap_or("-"));
        }
    }

    println!("\nEvidence references:");
    if report.evidence_refs.is_empty() {
        println!("  (none)");
    }
    for e in &report.evidence_refs {
        println!(
            "  {} [{}] {} (produced by {})",
            e.id, e.artifact_kind, e.artifact_digest, e.produced_by
        );
    }
}

fn manifest_binding_label(binding: ManifestBinding) -> &'static str {
    match binding {
        ManifestBinding::Verified => "verified",
        ManifestBinding::Unverified => "unverified (no manifest supplied)",
    }
}

/// Render a recorded decision for a terminal reader.
pub fn print_decision_text(outcome: &DecisionOutcome) {
    println!("Human gate decision recorded");
    println!("  WorkContext: {}", outcome.work_context_id);
    println!("  Run: {}", outcome.run_id);
    println!("  Gate: {}", outcome.gate_node_id);
    println!("  Verdict: {}", outcome.verdict);
    println!("  Decided by: {}", outcome.decided_by);
    println!("  Channel: {}", outcome.channel);
    println!("  Basis evidence: {}", outcome.basis_evidence_digest);
    println!("  Rationale: {}", outcome.reason);
    println!("  Recorded at: {}", outcome.recorded_at);
    if let Some(d) = &outcome.decision_digest {
        println!("  Decision digest: {d}");
    }
    println!("  Checkpoint digest: {}", outcome.new_checkpoint_digest);
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometheos_lite::work::WorkContextService;
    use prometheos_lite::work::types::{ApprovalPolicy, AutonomyLevel, WorkContext, WorkDomain};
    use prometheos_lite::workflow::graph_state::{
        EdgeKind, GraphEdgeV1, GraphNodeV1, NodeAttemptRecordV1, OutcomeCategory, RouteDecisionV1,
    };

    const OWNER: &str = "cli-user";

    struct Fixture {
        _dir: tempfile::TempDir,
        db: Arc<Db>,
        db_path: String,
        context: WorkContext,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir
            .path()
            .join("graph_runs.db")
            .to_str()
            .expect("db path")
            .to_string();
        let db = Arc::new(Db::new(&db_path).expect("db"));
        let journal = JournalContext::for_request(
            OWNER,
            "cli-test".into(),
            JournalContext::work_authority(AutonomyLevel::Review, ApprovalPolicy::Auto),
        );
        let context = WorkContextService::new(db.clone())
            .create_context(
                OWNER.into(),
                "graph runs".into(),
                WorkDomain::General,
                "inspect".into(),
                &journal,
            )
            .expect("create context");
        Fixture {
            _dir: dir,
            db,
            db_path,
            context,
        }
    }

    /// A VERIFIED human principal for this session: the OS account running
    /// this process, listed in a trust anchor. This is the only way to
    /// obtain one — there is no constructor from a caller-supplied name.
    fn verified_principal() -> VerifiedHumanPrincipal {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = human_principal::default_anchor_path(dir.path());
        std::fs::create_dir_all(path.parent().expect("anchor dir")).expect("anchor dir");
        let identity = human_principal::session_identity().expect("session identity");
        let anchor = human_principal::HumanPrincipalAnchor {
            schema_version: human_principal::PRINCIPAL_ANCHOR_SCHEMA_VERSION.into(),
            principals: vec![identity],
        };
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&anchor).expect("anchor json"),
        )
        .expect("write anchor");
        human_principal::verify_human_principal(&path).expect("verified principal")
    }

    fn gate_manifest() -> GraphManifestV1 {
        GraphManifestV1 {
            schema_version: "1.0.0".into(),
            graph_id: "gated".into(),
            version: "1.0.0".into(),
            nodes: vec![
                GraphNodeV1 {
                    node_id: "build".into(),
                    capability: "cap.build".into(),
                    purpose: None,
                    resources: Vec::new(),
                    join: None,
                },
                GraphNodeV1 {
                    node_id: "review".into(),
                    capability: "gate.human".into(),
                    purpose: None,
                    resources: Vec::new(),
                    join: None,
                },
                GraphNodeV1 {
                    node_id: "merge".into(),
                    capability: "cap.merge".into(),
                    purpose: None,
                    resources: Vec::new(),
                    join: None,
                },
            ],
            edges: vec![
                GraphEdgeV1 {
                    from: "build".into(),
                    to: "review".into(),
                    kind: EdgeKind::Sequence,
                    condition_label: None,
                },
                GraphEdgeV1 {
                    from: "review".into(),
                    to: "merge".into(),
                    kind: EdgeKind::Conditional,
                    condition_label: Some("approved".into()),
                },
            ],
            entry_points: vec!["build".into()],
            terminal_exits: vec!["merge".into()],
            shared_state_keys: Vec::new(),
            policy_digest: None,
            content_digest: Some(String::new()),
        }
        .sealed()
    }

    /// A run that reached its gate: `build` completed, routed to `review`.
    fn run_at_the_gate(run_id: &str) -> (GraphManifestV1, String) {
        let manifest = gate_manifest();
        let mut state = GraphRunStateV1::open(
            run_id,
            &manifest,
            "revision-1",
            "pws",
            "2".repeat(64),
            "2026-01-01T00:00:00Z",
        )
        .expect("open run");
        state
            .apply_node_completion(
                "build",
                NodeAttemptRecordV1 {
                    attempt: 1,
                    started_at: "2026-01-01T00:00:00Z".into(),
                    completed_at: Some("2026-01-01T00:00:01Z".into()),
                    outcome: OutcomeCategory::Completed,
                    result_digest: "d".repeat(64),
                },
                "2026-01-01T00:00:01Z",
            )
            .expect("completion");
        state
            .record_route_decision(
                RouteDecisionV1 {
                    recorded_at: "2026-01-01T00:00:02Z".into(),
                    from_node: "build".into(),
                    to_node: "review".into(),
                    condition_label: None,
                    basis_result_digest: "d".repeat(64),
                },
                &manifest,
            )
            .expect("route");
        (manifest, state.export_checkpoint().expect("checkpoint"))
    }

    fn seed(fixture: &Fixture, run_id: &str) -> (GraphManifestV1, String) {
        let (manifest, blob) = run_at_the_gate(run_id);
        graph_checkpoints::upsert_checkpoint(
            &*fixture.db,
            OWNER,
            &fixture.context.id,
            run_id,
            &blob,
        )
        .expect("seed checkpoint");
        (manifest, blob)
    }

    #[test]
    fn verdict_parsing_is_explicit_and_refuses_unknown_spellings() {
        assert_eq!(parse_verdict("approve").unwrap(), HumanVerdict::Approved);
        assert_eq!(parse_verdict("reject").unwrap(), HumanVerdict::Rejected);
        assert_eq!(
            parse_verdict("request-changes").unwrap(),
            HumanVerdict::ChangesRequested
        );
        assert_eq!(
            parse_verdict("CHANGES-REQUESTED").unwrap(),
            HumanVerdict::ChangesRequested
        );
        let err = parse_verdict("maybe").unwrap_err().to_string();
        assert!(err.contains("unknown verdict"), "{err}");
    }

    #[test]
    fn listing_shows_the_registered_runs() {
        let f = fixture();
        seed(&f, "run-1");
        let runs = list_runs(&f.db, OWNER, &f.context.id).expect("list");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].graph_run_id, "run-1");
        assert_eq!(runs[0].checkpoint_digest.len(), 64);
    }

    #[test]
    fn inspection_traces_the_run_to_its_evidence() {
        let f = fixture();
        let (manifest, _) = seed(&f, "run-1");
        let report =
            inspect_run(&f.db, OWNER, &f.context.id, "run-1", Some(&manifest)).expect("inspect");

        assert_eq!(report.run_id, "run-1");
        assert_eq!(report.frontier, vec!["review".to_string()]);
        assert_eq!(report.manifest_binding, ManifestBinding::Verified);
        assert_eq!(report.node_attempts.len(), 1);
        assert_eq!(
            report.node_attempts[0].latest_result_digest,
            Some("d".repeat(64))
        );
        assert_eq!(report.route_history.len(), 1);
        assert_eq!(report.route_history[0].to_node, "review");
    }

    #[test]
    fn inspection_without_a_manifest_reports_the_binding_as_unverified() {
        let f = fixture();
        seed(&f, "run-1");
        let report = inspect_run(&f.db, OWNER, &f.context.id, "run-1", None).expect("inspect");
        assert_eq!(report.manifest_binding, ManifestBinding::Unverified);
        // The chain digest is still verified — a tampered blob is refused.
        assert!(report.chain_digest.is_some());
    }

    #[test]
    fn decision_records_actor_timestamp_basis_and_rationale() {
        let f = fixture();
        let (manifest, _) = seed(&f, "run-1");
        let principal = verified_principal();
        let outcome = decide_run(
            &f.db,
            &f.context,
            OWNER,
            "run-1",
            &manifest,
            "review",
            HumanVerdict::Approved,
            &principal,
            &"d".repeat(64),
            "ship it",
        )
        .expect("decide");

        assert_eq!(outcome.verdict, "approve");
        // The actor is the VERIFIED principal, never a caller-supplied name.
        assert_eq!(outcome.decided_by, principal.identity());
        assert_eq!(outcome.basis_evidence_digest, "d".repeat(64));
        assert_eq!(outcome.reason, "ship it");
        assert!(!outcome.recorded_at.is_empty());
        assert!(outcome.decision_digest.is_some());

        // The decision is durable INSIDE the checkpoint, and the inspector
        // shows it with its actor, timestamp, basis and rationale.
        let report =
            inspect_run(&f.db, OWNER, &f.context.id, "run-1", Some(&manifest)).expect("inspect");
        assert_eq!(report.gate_decisions.len(), 1);
        let d = &report.gate_decisions[0];
        assert_eq!(d.gate_node_id, "review");
        assert_eq!(d.verdict, "approve");
        assert_eq!(d.decided_by, principal.identity());
        assert_eq!(d.reason, "ship it");
        assert_eq!(d.basis_evidence_digest, "d".repeat(64));
        assert!(!d.recorded_at.is_empty());
        assert!(d.evidence_current);
    }

    #[test]
    fn the_durable_journal_records_the_verified_principal_as_the_human() {
        let f = fixture();
        let (manifest, _) = seed(&f, "run-1");
        let principal = verified_principal();
        decide_run(
            &f.db,
            &f.context,
            OWNER,
            "run-1",
            &manifest,
            "review",
            HumanVerdict::Approved,
            &principal,
            &"d".repeat(64),
            "ship it",
        )
        .expect("decide");

        // The journal event's provenance names the verified human — the
        // schema CHECK ties the principal_id column to
        // principal.human.identity, so the durable record cannot claim a
        // different human than the one that was verified.
        let conn = f.db.conn();
        let (principal_id, provenance): (Option<String>, String) = conn
            .query_row(
                "SELECT principal_id, provenance_json FROM work_context_events
                 WHERE work_context_id = ?1 AND event_type = 'graph_decision'",
                rusqlite::params![f.context.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("event row");
        assert_eq!(principal_id.as_deref(), Some(principal.identity()));
        let envelope: serde_json::Value =
            serde_json::from_str(&provenance).expect("provenance json");
        assert_eq!(
            envelope["principal"]["human"]["identity"],
            serde_json::json!(principal.identity())
        );
        assert_eq!(envelope["producer"]["kind"], serde_json::json!("human"));
    }

    #[test]
    fn an_unverifiable_principal_refuses_the_decision() {
        // No trust anchor: the runtime cannot establish a verified human
        // principal, so the decision must be REFUSED — never attributed
        // to a fallback identity.
        let f = fixture();
        let (manifest, _) = seed(&f, "run-1");
        let err = resolve_human_principal(f._dir.path()).expect_err("no anchor must refuse");
        let msg = format!("{err:#}");
        assert!(msg.contains("trust anchor"), "{msg}");

        // The run is untouched: no decision was recorded.
        let report =
            inspect_run(&f.db, OWNER, &f.context.id, "run-1", Some(&manifest)).expect("inspect");
        assert!(report.gate_decisions.is_empty());
    }

    #[test]
    fn decision_refuses_a_node_that_is_not_a_declared_gate() {
        let f = fixture();
        let (manifest, _) = seed(&f, "run-1");
        let principal = verified_principal();
        let err = decide_run(
            &f.db,
            &f.context,
            OWNER,
            "run-1",
            &manifest,
            "build",
            HumanVerdict::Approved,
            &principal,
            &"d".repeat(64),
            "ship it",
        )
        .expect_err("non-gate node must be refused");
        assert!(err.to_string().contains("not a declared gate"), "{err}");
    }

    #[test]
    fn decision_refuses_evidence_the_run_no_longer_holds() {
        let f = fixture();
        let (manifest, _) = seed(&f, "run-1");
        let principal = verified_principal();
        let err = decide_run(
            &f.db,
            &f.context,
            OWNER,
            "run-1",
            &manifest,
            "review",
            HumanVerdict::Approved,
            &principal,
            &"a".repeat(64),
            "ship it",
        )
        .expect_err("unanchored basis must be refused");
        assert!(err.to_string().contains("not anchored"), "{err}");
    }

    #[test]
    fn a_superseded_decision_is_reported_as_stale() {
        let f = fixture();
        let (manifest, _) = seed(&f, "run-1");
        let principal = verified_principal();
        decide_run(
            &f.db,
            &f.context,
            OWNER,
            "run-1",
            &manifest,
            "review",
            HumanVerdict::Approved,
            &principal,
            &"d".repeat(64),
            "ship it",
        )
        .expect("decide");

        // Rework: `build` re-executes, so the reviewed artifact is superseded.
        let (blob, digest_before) =
            graph_checkpoints::get_checkpoint(&*f.db, OWNER, &f.context.id, "run-1")
                .expect("read")
                .expect("present");
        let mut state =
            GraphRunStateV1::import_checkpoint(&blob, &manifest, &"2".repeat(64)).expect("import");
        state.frontier.push("build".into());
        state
            .apply_node_completion(
                "build",
                NodeAttemptRecordV1 {
                    attempt: 2,
                    started_at: "2026-01-01T00:00:10Z".into(),
                    completed_at: Some("2026-01-01T00:00:11Z".into()),
                    outcome: OutcomeCategory::Completed,
                    result_digest: "c".repeat(64),
                },
                "2026-01-01T00:00:11Z",
            )
            .expect("rework");
        let new_blob = state.export_checkpoint().expect("export");
        let digest_after =
            graph_checkpoints::upsert_checkpoint(&*f.db, OWNER, &f.context.id, "run-1", &new_blob)
                .expect("reseed");
        assert_ne!(
            digest_after, digest_before,
            "the rework must have changed the checkpoint"
        );

        let report =
            inspect_run(&f.db, OWNER, &f.context.id, "run-1", Some(&manifest)).expect("inspect");
        assert_eq!(report.gate_decisions.len(), 1);
        assert!(
            !report.gate_decisions[0].evidence_current,
            "a superseded decision must be reported as stale"
        );
        assert!(report.gate_decisions[0].stale_reason.is_some());
    }

    #[test]
    fn another_principals_run_is_not_readable() {
        let f = fixture();
        seed(&f, "run-1");
        let err = inspect_run(&f.db, "someone-else", &f.context.id, "run-1", None)
            .expect_err("ownership must be enforced");
        assert!(err.to_string().contains("owned by"), "{err}");
    }

    // ---------------------------------------------------------------------
    // Trust-anchor protection: the decision path only READS the anchor.
    // ---------------------------------------------------------------------

    #[test]
    fn the_decision_path_never_provisions_or_modifies_the_trust_anchor() {
        let f = fixture();
        let (manifest, _) = seed(&f, "run-1");
        let anchor_path = human_principal::default_anchor_path(f._dir.path());
        std::fs::create_dir_all(anchor_path.parent().expect("anchor dir")).expect("anchor dir");
        let identity = human_principal::session_identity().expect("session identity");
        let anchor = human_principal::HumanPrincipalAnchor {
            schema_version: human_principal::PRINCIPAL_ANCHOR_SCHEMA_VERSION.into(),
            principals: vec![identity],
        };
        let bytes = serde_json::to_string_pretty(&anchor).expect("anchor json");
        std::fs::write(&anchor_path, &bytes).expect("write anchor");
        let modified_before = std::fs::metadata(&anchor_path)
            .expect("meta")
            .modified()
            .expect("modified");

        let principal = human_principal::verify_human_principal(&anchor_path).expect("verified");
        decide_run(
            &f.db,
            &f.context,
            OWNER,
            "run-1",
            &manifest,
            "review",
            HumanVerdict::Approved,
            &principal,
            &"d".repeat(64),
            "ship it",
        )
        .expect("decide");

        // The anchor is byte-identical and untouched: recording a decision
        // never provisions, repairs, or rewrites the trust configuration.
        let after = std::fs::read(&anchor_path).expect("anchor still present");
        assert_eq!(
            after,
            bytes.as_bytes(),
            "the decision must not modify the anchor"
        );
        let modified_after = std::fs::metadata(&anchor_path)
            .expect("meta")
            .modified()
            .expect("modified");
        assert_eq!(
            modified_after, modified_before,
            "the decision must not rewrite the anchor"
        );
    }

    #[test]
    fn a_refused_decision_does_not_silently_provision_an_anchor() {
        let f = fixture();
        let anchor_path = human_principal::default_anchor_path(f._dir.path());
        let err = resolve_human_principal(f._dir.path()).expect_err("no anchor must refuse");
        assert!(format!("{err:#}").contains("trust anchor"), "{err:#}");
        assert!(
            !anchor_path.exists(),
            "a refusal must not silently provision a trust anchor"
        );
    }

    // ---------------------------------------------------------------------
    // F2: the CLI path proves the atomicity and concurrency guarantees
    // directly, not only through the API path that shares the primitives.
    // ---------------------------------------------------------------------

    #[test]
    fn a_failed_event_write_rolls_the_checkpoint_back() {
        // INTERRUPTION: the journal event insert aborts. The checkpoint
        // write must roll back with it — a decision is never durable
        // without its event, and an event is never durable without its
        // decision.
        let f = fixture();
        let (manifest, _) = seed(&f, "run-1");
        let principal = verified_principal();
        let digest_before =
            graph_checkpoints::get_checkpoint(&*f.db, OWNER, &f.context.id, "run-1")
                .expect("read")
                .expect("present")
                .1;

        let conn = rusqlite::Connection::open(&f.db_path).expect("probe connection");
        conn.execute_batch(
            "CREATE TRIGGER veto_graph_decision
             BEFORE INSERT ON work_context_events
             WHEN NEW.event_type = 'graph_decision'
             BEGIN SELECT RAISE(ABORT, 'aborted-by-trigger'); END;",
        )
        .expect("create trigger");

        let err = decide_run(
            &f.db,
            &f.context,
            OWNER,
            "run-1",
            &manifest,
            "review",
            HumanVerdict::Approved,
            &principal,
            &"d".repeat(64),
            "ship it",
        )
        .expect_err("an aborted event write must fail the decision");
        assert!(format!("{err:#}").contains("aborted-by-trigger"), "{err:#}");

        // The checkpoint is pinned to its original bytes: the write-back
        // rolled back together with the failed event insert.
        let (_, digest_after) =
            graph_checkpoints::get_checkpoint(&*f.db, OWNER, &f.context.id, "run-1")
                .expect("read")
                .expect("present");
        assert_eq!(
            digest_after, digest_before,
            "the checkpoint must be unchanged"
        );
        let report =
            inspect_run(&f.db, OWNER, &f.context.id, "run-1", Some(&manifest)).expect("inspect");
        assert!(
            report.gate_decisions.is_empty(),
            "no decision may survive a failed event write"
        );
        let events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM work_context_events
                 WHERE work_context_id = ?1 AND event_type = 'graph_decision'",
                rusqlite::params![f.context.id],
                |r| r.get(0),
            )
            .expect("count events");
        assert_eq!(events, 0, "no event may survive the failed transaction");
    }

    #[test]
    fn a_stale_expected_digest_is_refused_by_the_compare_and_swap() {
        // The guard the decision path relies on, tested directly and
        // deterministically: two writers holding the SAME expected digest —
        // the first must win, the second must be refused. This is the
        // property that makes a lost update impossible.
        let f = fixture();
        let (manifest, blob) = seed(&f, "run-1");
        let expected = graph_checkpoints::get_checkpoint(&*f.db, OWNER, &f.context.id, "run-1")
            .expect("read")
            .expect("present")
            .1;

        // A first writer moves the checkpoint forward.
        let mut state =
            GraphRunStateV1::import_checkpoint(&blob, &manifest, &"2".repeat(64)).expect("import");
        state.frontier.push("build".into());
        state
            .apply_node_completion(
                "build",
                NodeAttemptRecordV1 {
                    attempt: 2,
                    started_at: "2026-01-01T00:00:10Z".into(),
                    completed_at: Some("2026-01-01T00:00:11Z".into()),
                    outcome: OutcomeCategory::Completed,
                    result_digest: "c".repeat(64),
                },
                "2026-01-01T00:00:11Z",
            )
            .expect("rework");
        let first = state.export_checkpoint().expect("export");
        {
            let conn = f.db.conn();
            let tx = conn.unchecked_transaction().expect("tx");
            assert!(
                graph_checkpoints::cas_checkpoint_conn(
                    &tx,
                    OWNER,
                    &f.context.id,
                    "run-1",
                    &expected,
                    &first
                )
                .expect("first cas"),
                "the first writer must win the compare-and-swap"
            );
            tx.commit().expect("commit");
        }

        // A second writer holding the now-stale expected digest is refused.
        let second = blob.replace("\"frontier\":[]", "\"frontier\":[\"merge\"]");
        {
            let conn = f.db.conn();
            let tx = conn.unchecked_transaction().expect("tx");
            assert!(
                !graph_checkpoints::cas_checkpoint_conn(
                    &tx,
                    OWNER,
                    &f.context.id,
                    "run-1",
                    &expected,
                    &second
                )
                .expect("second cas"),
                "a stale expected digest must be refused"
            );
            tx.commit().expect("commit");
        }
    }

    #[test]
    fn two_concurrent_decisions_leave_one_consistent_decision() {
        // CONCURRENCY: two writers race for the same run. The outcome
        // depends on the interleaving — a writer whose READ lands after
        // the other's commit legitimately succeeds (re-deciding a gate
        // supersedes the earlier decision) — so the assertions are the
        // invariants that must hold in EVERY interleaving, not a
        // particular winner:
        //
        // - the gate holds exactly ONE decision (never two conflicting);
        // - the stored blob still matches its recomputed digest (no torn
        //   write);
        // - the journal holds exactly one event per successful decision
        //   (no orphans, no duplicates);
        // - a writer that loses the race fails CLOSED, never silently.
        let f = fixture();
        let (manifest, _) = seed(&f, "run-1");
        let principal = verified_principal();
        let context = f.context.clone();

        // Two SEPARATE connections to the same database file: the race is
        // between independent writers, as two CLI processes would be.
        let db_a = Arc::new(Db::new(&f.db_path).expect("db a"));
        let db_b = Arc::new(Db::new(&f.db_path).expect("db b"));

        let manifest_a = manifest.clone();
        let manifest_b = manifest.clone();
        let context_a = context.clone();
        let context_b = context.clone();
        let principal_a = principal.clone();
        let principal_b = principal.clone();

        let handle_a = std::thread::spawn(move || {
            decide_run(
                &db_a,
                &context_a,
                OWNER,
                "run-1",
                &manifest_a,
                "review",
                HumanVerdict::Approved,
                &principal_a,
                &"d".repeat(64),
                "ship it",
            )
        });
        let handle_b = std::thread::spawn(move || {
            decide_run(
                &db_b,
                &context_b,
                OWNER,
                "run-1",
                &manifest_b,
                "review",
                HumanVerdict::Approved,
                &principal_b,
                &"d".repeat(64),
                "ship it",
            )
        });

        let a = handle_a.join().expect("thread a");
        let b = handle_b.join().expect("thread b");
        let winners = [a.is_ok(), b.is_ok()].iter().filter(|w| **w).count();
        assert!(winners >= 1, "at least one decision must be recorded");
        for (result, other_won) in [(&a, b.is_ok()), (&b, a.is_ok())] {
            match result {
                Err(err) if other_won => {
                    let msg = format!("{err:#}");
                    assert!(
                        msg.contains("raced") || msg.contains("locked") || msg.contains("busy"),
                        "a writer that loses the race must fail closed: {msg}"
                    );
                }
                _ => {}
            }
        }

        // Exactly one decision for the gate, and the stored blob still
        // matches its digest: no torn write, no conflicting decisions.
        let report =
            inspect_run(&f.db, OWNER, &f.context.id, "run-1", Some(&manifest)).expect("inspect");
        assert_eq!(report.gate_decisions.len(), 1);
        let conn = rusqlite::Connection::open(&f.db_path).expect("probe connection");
        let events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM work_context_events
                 WHERE work_context_id = ?1 AND event_type = 'graph_decision'",
                rusqlite::params![f.context.id],
                |r| r.get(0),
            )
            .expect("count events");
        assert_eq!(
            events, winners as i64,
            "the journal must hold exactly one event per recorded decision"
        );
    }
}
