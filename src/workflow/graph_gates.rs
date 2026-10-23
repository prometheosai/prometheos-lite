//! Human gates, classified failure routing, and rejection terminality
//! (issue #122). Extends the #120/#121 routing law additively.
//!
//! Law:
//! - Human decisions are DURABLE records (`lite.hdecision.v1`) sealed with a
//!   canonical digest and bound to the gate node + basis evidence. Machine
//!   actors (agent/model/provider/harness/tool) can never author one.
//! - Approval is never inferred: routing THROUGH a gate requires a recorded
//!   decision for that node; absence => typed `HumanGatePending`.
//! - Rejection forces terminality: after `rejected`, the only eligible edge
//!   label is `rejected` AND its target must be a terminal exit; anything
//!   else is typed `RejectionTerminal` (a new authorized route = fresh run).
//! - Classified failures route separately via canonical labels
//!   (`failed-code`, `failed-infra`, `failed-policy`, `failed-evidence`).
//! - Retry edges are plain conditional edges; #121 caps bind them.

use serde::{Deserialize, Serialize};

use crate::workflow::graph_state::{GraphManifestV1, OutcomeCategory, RouteDecisionV1};
use crate::workflow::soma::try_canonical_digest;

/// Schema version for human decision records.
pub const HUMAN_DECISION_VERSION: &str = "1.0.0";

/// Capability prefix marking a node as a human gate.
pub const GATE_CAPABILITY_PREFIX: &str = "gate.";

// ---------------------------------------------------------------------------
// Failure classification -> canonical route labels
// ---------------------------------------------------------------------------

/// Canonical failure classes routed on separate conditional edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClass {
    Code,
    Infrastructure,
    Policy,
    Evidence,
}

impl FailureClass {
    /// Canonical conditional-edge label for this class.
    pub fn route_label(self) -> &'static str {
        match self {
            FailureClass::Code => "failed-code",
            FailureClass::Infrastructure => "failed-infra",
            FailureClass::Policy => "failed-policy",
            FailureClass::Evidence => "failed-evidence",
        }
    }
}

// ---------------------------------------------------------------------------
// HumanDecisionRecordV1
// ---------------------------------------------------------------------------

/// The human decision on a gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HumanVerdict {
    Approved,
    #[serde(rename = "changes_requested")]
    ChangesRequested,
    Rejected,
}

impl HumanVerdict {
    /// The ONLY edge label eligible under this verdict at a gate.
    pub fn edge_label(self) -> &'static str {
        match self {
            HumanVerdict::Approved => "approved",
            HumanVerdict::ChangesRequested => "changes-requested",
            HumanVerdict::Rejected => "rejected",
        }
    }
}

/// Where the human acted. Model/provider/harness/tool actors are
/// unrepresentable here by construction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum ReviewChannel {
    /// Direct interactive approval through the Lite CLI.
    CliInteractive,
    /// A named external human review system (issue tracker, review tool).
    ExternalSystem { system_id: String },
}

/// Durable, authoritative record of one human gate decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HumanDecisionRecordV1 {
    #[serde(rename = "schemaVersion")]
    pub schema_version: String,
    pub gate_node_id: String,
    pub verdict: HumanVerdict,
    /// HUMAN identity — machine actors refuse at construction.
    pub decided_by: String,
    pub channel: ReviewChannel,
    /// Digest of the evidence artifact the human reviewed.
    #[serde(rename = "basisEvidenceDigest")]
    pub basis_evidence_digest: String,
    pub reason: String,
    #[serde(rename = "recordedAt")]
    pub recorded_at: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "contentDigest"
    )]
    pub content_digest: Option<String>,
}

/// Identities that can never authorize a gate (machine actor prefixes).
const MACHINE_IDENTITY_PREFIXES: [&str; 5] = ["model:", "provider:", "harness:", "tool:", "agent:"];

impl HumanDecisionRecordV1 {
    /// Author a decision. Fails closed when the identity is a machine actor
    /// or required fields are empty: approval cannot be inferred from model
    /// output because models cannot produce this record at all.
    pub fn author(
        gate_node_id: impl Into<String>,
        verdict: HumanVerdict,
        decided_by: impl Into<String>,
        channel: ReviewChannel,
        basis_evidence_digest: impl Into<String>,
        reason: impl Into<String>,
        recorded_at: impl Into<String>,
    ) -> Result<Self> {
        let decided_by = decided_by.into();
        if decided_by.is_empty() {
            anyhow::bail!("human decision requires a non-empty human identity");
        }
        let lower = decided_by.to_ascii_lowercase();
        for p in MACHINE_IDENTITY_PREFIXES {
            if lower.starts_with(p) {
                anyhow::bail!(
                    "machine actor identity {decided_by:?} cannot authorize a human gate"
                );
            }
        }
        let basis_evidence_digest = basis_evidence_digest.into();
        if basis_evidence_digest.len() != 64 {
            anyhow::bail!("gate decision must cite 64-hex basis evidence digest");
        }
        // #265: the rationale is part of the audit contract, not an optional
        // annotation. An empty or whitespace-only reason would be sealed into a
        // durable, content-digested record that reads as though a rationale had
        // been given. Enforced here, on the record's own authoring path, so the
        // invariant covers every producer (CLI, API, import rebuild) rather
        // than only the argument parser.
        let reason = reason.into();
        if reason.trim().is_empty() {
            anyhow::bail!("human decision requires a non-empty rationale");
        }
        Ok(Self {
            schema_version: HUMAN_DECISION_VERSION.into(),
            gate_node_id: gate_node_id.into(),
            verdict,
            decided_by,
            channel,
            basis_evidence_digest,
            reason,
            recorded_at: recorded_at.into(),
            content_digest: None,
        })
        .map(|mut r| {
            r.content_digest = Some(r.compute_digest());
            r
        })
    }

    /// Canonical digest over the record minus `contentDigest`.
    pub fn compute_digest(&self) -> String {
        let mut v = serde_json::to_value(self).expect("decision serializes");
        if let Some(obj) = v.as_object_mut() {
            obj.remove("contentDigest");
        }
        // Lite-constructed record: violation unreachable per the invariant in
        // `soma::canonical` ("Call-site contract"); panic if it ever breaks.
        try_canonical_digest(&v).expect("canonical digest of Lite-constructed record cannot fail")
    }

    /// Fail-closed parse: version gate, digest verify, machine-actor refusal.
    pub fn parse_json(text: &str) -> Result<Self> {
        let r: Self = serde_json::from_str(text).context("human decision parse failed")?;
        if r.schema_version != HUMAN_DECISION_VERSION {
            anyhow::bail!(
                "unsupported human decision schema version {}",
                r.schema_version
            );
        }
        // Re-author to re-apply machine-actor refusal on import too.
        let rebuilt = Self::author(
            r.gate_node_id.clone(),
            r.verdict,
            r.decided_by.clone(),
            r.channel.clone(),
            r.basis_evidence_digest.clone(),
            r.reason.clone(),
            r.recorded_at.clone(),
        )?;
        match &r.content_digest {
            None => anyhow::bail!("human decision missing contentDigest"),
            Some(d) if *d != rebuilt.compute_digest() => {
                anyhow::bail!("human decision contentDigest does not verify")
            }
            Some(_) => {}
        }
        Ok(r)
    }

    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }
}

use anyhow::{Context as _, Result};

// ---------------------------------------------------------------------------
// Routing law extension
// ---------------------------------------------------------------------------

/// Typed gate-routing failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateRouteError {
    /// Gate reached without any recorded human decision.
    HumanGatePending { node: String },
    /// Post-rejection continuation attempt (only rejected-labeled terminals
    /// are eligible after a rejection).
    RejectionTerminal {
        from: String,
        attempted_target: String,
    },
    /// A recorded decision whose cited evidence is no longer anchored in
    /// the run's current state (stale approval after a rework/artifact
    /// change). The gate returns to pending until a fresh human decision
    /// against the current evidence is recorded (#131).
    StaleGateDecision { node: String, basis: String },
}

impl std::fmt::Display for GateRouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GateRouteError::HumanGatePending { node } => {
                write!(
                    f,
                    "human gate {node:?} pending: no durable decision recorded"
                )
            }
            GateRouteError::RejectionTerminal {
                from,
                attempted_target,
            } => write!(
                f,
                "run rejected at {from:?}; continuation to {attempted_target:?} \
                 requires a new authorized run"
            ),
            GateRouteError::StaleGateDecision { node, basis } => write!(
                f,
                "human gate {node:?} decision is stale: cited evidence {basis} is \
                 no longer anchored in the run; re-decide against the current \
                 evidence"
            ),
        }
    }
}

impl std::error::Error for GateRouteError {}

/// Whether this manifest node is a human gate.
pub fn is_gate_node(manifest: &GraphManifestV1, node_id: &str) -> bool {
    manifest
        .nodes
        .iter()
        .any(|n| n.node_id == node_id && n.capability.starts_with(GATE_CAPABILITY_PREFIX))
}

/// STALENESS ANCHOR (#131, hardened by #266): a gate decision is eligible only
/// while the evidence it cites is still anchored in the run's current state — a
/// CURRENT evidence reference, or the LATEST attempt digest of some node. After
/// a rework cycle supersedes the reviewed artifact (a newer attempt digest, a
/// superseded evidence reference), the recorded decision is stale and routing
/// refuses it: the gate returns to pending for a fresh human decision.
pub fn decision_basis_is_anchored(
    state: &crate::workflow::graph_state::GraphRunStateV1,
    basis: &str,
) -> bool {
    state
        .evidence_refs
        .iter()
        .any(|r| r.artifact_digest == basis && evidence_reference_is_current(state, r))
        || state
            .node_attempts
            .values()
            .filter_map(|attempts| attempts.last())
            .any(|latest| latest.result_digest == basis)
}

/// The run's current frontier identity, computed exactly as a
/// `replay-authorization` evidence reference computes the digest it stores.
fn current_frontier_digest(
    state: &crate::workflow::graph_state::GraphRunStateV1,
) -> Option<String> {
    crate::workflow::soma::try_canonical_digest(&serde_json::json!({
        "runId": state.run_id,
        "frontier": state.frontier,
    }))
    .ok()
}

/// Whether an evidence reference still describes the run's CURRENT evidence
/// (#266).
///
/// Membership in `evidence_refs` is history, not currency: the vector keeps what
/// was once true. A `replay-authorization` reference names the frontier that was
/// reviewed, so it is current only while that frontier is still the run's
/// frontier. Once the run advances, that reference records a superseded state
/// and must not keep an approval alive — otherwise an approval outlives the
/// evidence it cites.
///
/// Other kinds are removed from the vector when they are superseded: a
/// `suspension-point` reference is dropped on resume, so membership is currency
/// for it. A future kind that accumulates instead of being removed must add its
/// own currency rule here rather than inheriting historical membership.
fn evidence_reference_is_current(
    state: &crate::workflow::graph_state::GraphRunStateV1,
    reference: &crate::workflow::memory_contracts::EvidenceReferenceV1,
) -> bool {
    match reference.artifact_kind.as_str() {
        "replay-authorization" => current_frontier_digest(state)
            .is_some_and(|current| current == reference.artifact_digest),
        _ => true,
    }
}

/// Compute the gate-filtered decision for `from_node`.
///
/// Non-gate nodes delegate to the #121 outcome mapping unchanged. Gates:
/// eligibility keys off the recorded HUMAN verdict only — outcomes never
/// open a gate, and no decision means pending (fail closed). After a
/// rejection the target must be a declared terminal exit.
///
/// Gate-identity invariant (#262): this consumer reads `gate_decisions`
/// BY MAP KEY (`from_node`) and never compares the key with the record's
/// own `gate_node_id` - routing from B would attribute a decision authored
/// for gate A to gate B if a mismatched pair ever existed. The pair's
/// identity is enforced at the import boundary
/// (`GraphRunStateV1::import_for_inspection`, inherited by every importer)
/// and at authoring (`record_gate_decision`, which keys by
/// `decision.gate_node_id`), so this consumer only ever observes a matching
/// pair; the inspector projection (`GateDecisionView`) publishes both key
/// and record so any drift is visible, not silent.
pub fn route_after_gate(
    state: &crate::workflow::graph_state::GraphRunStateV1,
    manifest: &GraphManifestV1,
    limits: crate::workflow::graph_exec::ExecutionLimits,
    from_node: &str,
    outcome: OutcomeCategory,
    recorded_at: impl Into<String>,
) -> Result<RouteDecisionV1, anyhow::Error> {
    if !is_gate_node(manifest, from_node) {
        return crate::workflow::graph_exec::route_after(
            state,
            manifest,
            limits,
            from_node,
            outcome,
            recorded_at,
        )
        .map_err(anyhow::Error::from);
    }

    let Some(decision) = state.gate_decisions.get(from_node) else {
        anyhow::bail!(
            "{}",
            GateRouteError::HumanGatePending {
                node: from_node.to_string()
            }
        );
    };

    // Staleness law (#131): approval never outlives the evidence it cites.
    // An anchored decision routes; a stale one returns the gate to pending.
    if !decision_basis_is_anchored(state, &decision.basis_evidence_digest) {
        anyhow::bail!(
            "{}",
            GateRouteError::StaleGateDecision {
                node: from_node.to_string(),
                basis: decision.basis_evidence_digest.clone(),
            }
        );
    }
    let want = decision.verdict.edge_label();

    // Eligibility by verdict label only.
    let mut eligible: Vec<&crate::workflow::graph_state::GraphEdgeV1> = manifest
        .edges
        .iter()
        .filter(|e| e.from == from_node)
        .filter(|e| e.condition_label.as_deref() == Some(want))
        .collect();
    if eligible.is_empty() {
        anyhow::bail!(
            "{}",
            crate::workflow::graph_exec::RouteError::MissingRoute {
                from: from_node.into(),
                outcome,
            }
        );
    }
    eligible.sort_by(|a, b| a.to.cmp(&b.to));

    // Rejection terminality: every target must be a declared terminal exit.
    if decision.verdict == HumanVerdict::Rejected {
        for e in &eligible {
            let is_terminal = manifest.terminal_exits.contains(&e.to);
            if !is_terminal {
                anyhow::bail!(
                    "{}",
                    GateRouteError::RejectionTerminal {
                        from: from_node.to_string(),
                        attempted_target: e.to.clone(),
                    }
                );
            }
        }
    }

    if eligible.len() > 1 {
        anyhow::bail!(
            "{}",
            crate::workflow::graph_exec::RouteError::AmbiguousRoute {
                from: from_node.into(),
                targets: eligible.iter().map(|e| e.to.clone()).collect(),
            }
        );
    }
    Ok(RouteDecisionV1 {
        recorded_at: recorded_at.into(),
        from_node: from_node.to_string(),
        to_node: eligible[0].to.clone(),
        condition_label: eligible[0].condition_label.clone(),
        basis_result_digest: decision.compute_digest(),
    })
}

/// Register a durable human decision on the run state (additive field).
pub fn record_gate_decision(
    state: &mut crate::workflow::graph_state::GraphRunStateV1,
    manifest: &GraphManifestV1,
    decision: HumanDecisionRecordV1,
) -> Result<()> {
    if !is_gate_node(manifest, &decision.gate_node_id) {
        anyhow::bail!("{:?} is not a declared gate node", decision.gate_node_id);
    }
    state
        .gate_decisions
        .insert(decision.gate_node_id.clone(), decision);
    state.content_digest = Some(state.compute_digest());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::graph_exec::{ExecutionLimits, route_after};
    use crate::workflow::graph_state::{
        GRAPH_SCHEMA_VERSION, GraphEdgeV1, GraphNodeV1, NodeAttemptRecordV1,
    };

    fn node(id: &str, cap: &str) -> GraphNodeV1 {
        GraphNodeV1 {
            node_id: id.into(),
            capability: cap.into(),
            purpose: None,
            resources: Vec::new(),
            join: None,
        }
    }

    fn edge(from: &str, to: &str, label: Option<&str>) -> GraphEdgeV1 {
        GraphEdgeV1 {
            from: from.into(),
            to: to.into(),
            kind: if label.is_some() {
                crate::workflow::graph_state::EdgeKind::Conditional
            } else {
                crate::workflow::graph_state::EdgeKind::Sequence
            },
            condition_label: label.map(String::from),
        }
    }

    fn gate_manifest() -> GraphManifestV1 {
        GraphManifestV1 {
            schema_version: GRAPH_SCHEMA_VERSION.into(),
            graph_id: "gated".into(),
            version: "1.0.0".into(),
            nodes: vec![
                node("build", "cap.build"),
                node("review", "gate.human"),
                node("merge", "cap.merge"),
                node("rejected-terminal", "cap.rejected"),
            ],
            edges: vec![
                edge("build", "review", None),
                edge("review", "merge", Some("approved")),
                edge("review", "rejected-terminal", Some("rejected")),
            ],
            entry_points: vec!["build".into()],
            terminal_exits: vec!["rejected-terminal".into()],
            shared_state_keys: vec![],
            policy_digest: None,
            content_digest: None,
        }
        .sealed()
    }

    fn complete(state: &mut crate::workflow::graph_state::GraphRunStateV1, id: &str) {
        state
            .apply_node_completion(
                id,
                NodeAttemptRecordV1 {
                    attempt: 1,
                    started_at: "t".into(),
                    completed_at: Some("t".into()),
                    outcome: OutcomeCategory::Completed,
                    // 64-hex digest so decisions may validly cite it.
                    result_digest: "d".repeat(64),
                },
                "now",
            )
            .unwrap();
    }

    fn human_decision(verdict: HumanVerdict) -> HumanDecisionRecordV1 {
        human_decision_on(verdict, &"d".repeat(64))
    }

    /// A decision citing a specific reviewed artifact digest. Real
    /// decisions bind to the evidence the human reviewed (here: the
    /// build attempt's result digest journaled by `complete`).
    fn human_decision_on(verdict: HumanVerdict, basis: &str) -> HumanDecisionRecordV1 {
        HumanDecisionRecordV1::author(
            "review",
            verdict,
            "diego",
            ReviewChannel::CliInteractive,
            basis.to_string(),
            "lgtm",
            "2026-08-25T00:00:00Z",
        )
        .unwrap()
    }

    fn evidence_ref(digest: &str) -> crate::workflow::memory_contracts::EvidenceReferenceV1 {
        crate::workflow::memory_contracts::EvidenceReferenceV1 {
            id: format!("ev-{digest}"),
            event_digest: digest.to_string(),
            artifact_digest: digest.to_string(),
            artifact_kind: "test-artifact".into(),
            produced_by: "test".into(),
            produced_at: Some("t".into()),
        }
    }

    #[test]
    fn approval_requires_durable_human_decision() {
        let m = gate_manifest();
        let mut s =
            crate::workflow::graph_state::GraphRunStateV1::open("r", &m, "rev", "pws", "pd", "t0")
                .unwrap();
        complete(&mut s, "build");
        s.record_route_decision(
            route_after(
                &s,
                &m,
                ExecutionLimits::default(),
                "build",
                OutcomeCategory::Completed,
                "t",
            )
            .unwrap(),
            &m,
        )
        .unwrap();
        // Gate reached with NO decision => pending, never inferred.
        let err = route_after_gate(
            &s,
            &m,
            ExecutionLimits::default(),
            "review",
            OutcomeCategory::Completed,
            "t",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("pending"), "{err}");

        // Record the durable human decision; approved path opens.
        record_gate_decision(&mut s, &m, human_decision(HumanVerdict::Approved)).unwrap();
        let d = route_after_gate(
            &s,
            &m,
            ExecutionLimits::default(),
            "review",
            OutcomeCategory::Completed,
            "t",
        )
        .expect("approved routes");
        assert_eq!(d.to_node, "merge");
        s.record_route_decision(d, &m).unwrap();
        complete(&mut s, "merge");
    }

    #[test]
    fn machine_actors_can_never_authorize() {
        for identity in ["model:gpt", "provider:x", "harness:h", "tool:t", "agent:a"] {
            let err = HumanDecisionRecordV1::author(
                "review",
                HumanVerdict::Approved,
                identity,
                ReviewChannel::ExternalSystem {
                    system_id: "s".into(),
                },
                "e".repeat(64),
                "r",
                "t",
            )
            .unwrap_err()
            .to_string();
            assert!(err.contains("machine actor"), "{identity}: {err}");
        }
    }

    #[test]
    fn rejection_forces_terminal_and_blocks_continuation() {
        let m = gate_manifest();
        let mut s =
            crate::workflow::graph_state::GraphRunStateV1::open("r", &m, "rev", "pws", "pd", "t0")
                .unwrap();
        complete(&mut s, "build");
        s.record_route_decision(
            route_after(
                &s,
                &m,
                ExecutionLimits::default(),
                "build",
                OutcomeCategory::Completed,
                "t",
            )
            .unwrap(),
            &m,
        )
        .unwrap();
        record_gate_decision(&mut s, &m, human_decision(HumanVerdict::Rejected)).unwrap();

        // Rejected verdict may only take the rejected-labeled TERMINAL edge.
        let d = route_after_gate(
            &s,
            &m,
            ExecutionLimits::default(),
            "review",
            OutcomeCategory::Completed,
            "t",
        )
        .expect("rejected routes to terminal");
        assert_eq!(d.to_node, "rejected-terminal");
        s.record_route_decision(d, &m).unwrap();

        // A hostile manifest adding a non-terminal rejected edge cannot
        // continue the run: RejectionTerminal fires.
        let mut hostile = gate_manifest();
        hostile
            .edges
            .push(edge("review", "merge", Some("rejected")));
        let hostile = hostile.sealed();
        let err = route_after_gate(
            &s,
            &hostile,
            ExecutionLimits::default(),
            "review",
            OutcomeCategory::Completed,
            "t",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("new authorized run"), "{err}");
    }

    #[test]
    fn stale_gate_decision_returns_gate_to_pending() {
        let m = gate_manifest();
        let mut s =
            crate::workflow::graph_state::GraphRunStateV1::open("r", &m, "rev", "pws", "pd", "t0")
                .unwrap();
        complete(&mut s, "build");
        let d = route_after(
            &s,
            &m,
            ExecutionLimits::default(),
            "build",
            OutcomeCategory::Completed,
            "t",
        )
        .unwrap();
        s.record_route_decision(d, &m).unwrap();
        record_gate_decision(&mut s, &m, human_decision(HumanVerdict::Approved)).unwrap();
        let routed = route_after_gate(
            &s,
            &m,
            ExecutionLimits::default(),
            "review",
            OutcomeCategory::Completed,
            "t",
        )
        .expect("decision against the current evidence routes");
        assert_eq!(routed.to_node, "merge");

        // Rework: `build` re-executes and supersedes the reviewed artifact.
        s.frontier.push("build".into());
        s.apply_node_completion(
            "build",
            NodeAttemptRecordV1 {
                attempt: 2,
                started_at: "t1".into(),
                completed_at: Some("t2".into()),
                outcome: OutcomeCategory::Completed,
                // A different artifact supersedes the reviewed one.
                result_digest: "2".repeat(64),
            },
            "t2",
        )
        .unwrap();

        // The recorded decision cites the superseded digest: routing is
        // refused and the gate returns to pending for a fresh decision.
        let err = route_after_gate(
            &s,
            &m,
            ExecutionLimits::default(),
            "review",
            OutcomeCategory::Completed,
            "t2",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("stale"), "{err}");
    }

    #[test]
    fn evidence_reference_anchors_and_supersedes_gate_decisions() {
        let m = gate_manifest();
        let mut s =
            crate::workflow::graph_state::GraphRunStateV1::open("r", &m, "rev", "pws", "pd", "t0")
                .unwrap();
        complete(&mut s, "build");
        // The review evidence arrives as a durable evidence reference.
        s.evidence_refs.push(evidence_ref(&"e".repeat(64)));
        record_gate_decision(
            &mut s,
            &m,
            human_decision_on(HumanVerdict::Approved, &"e".repeat(64)),
        )
        .unwrap();
        let d = route_after_gate(
            &s,
            &m,
            ExecutionLimits::default(),
            "review",
            OutcomeCategory::Completed,
            "t",
        )
        .expect("decision anchored by a live evidence reference routes");
        assert_eq!(d.to_node, "merge");

        // The reviewed artifact changes (e.g. replaces the evidence ref):
        // the old decision goes stale.
        s.evidence_refs = vec![evidence_ref(&"f".repeat(64))];
        let err = route_after_gate(
            &s,
            &m,
            ExecutionLimits::default(),
            "review",
            OutcomeCategory::Completed,
            "t",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("stale"), "{err}");
    }

    /// A replay-authorization reference exactly as production writes it
    /// (`src/workflow/graph_resume.rs`): the artifact digest is the reviewed
    /// frontier, and the id is stable per authorizer, so a later replay appends
    /// a second reference without removing the first.
    fn replay_authorization_ref(
        authorizer: &str,
        run_id: &str,
        frontier: &[&str],
        reason: &str,
    ) -> (
        String,
        crate::workflow::memory_contracts::EvidenceReferenceV1,
    ) {
        let auth_digest = crate::workflow::soma::try_canonical_digest(&serde_json::json!({
            "kind": "replay-authorization",
            "authorizedBy": authorizer,
            "reason": reason,
        }))
        .expect("auth digest");
        let frontier_digest = crate::workflow::soma::try_canonical_digest(&serde_json::json!({
            "runId": run_id,
            "frontier": frontier,
        }))
        .expect("frontier digest");
        (
            frontier_digest.clone(),
            crate::workflow::memory_contracts::EvidenceReferenceV1 {
                id: format!("replay-auth:{authorizer}"),
                event_digest: auth_digest,
                artifact_digest: frontier_digest,
                artifact_kind: "replay-authorization".into(),
                produced_by: authorizer.into(),
                produced_at: Some("resume-replay".into()),
            },
        )
    }

    /// #266: the staleness rule must track the CURRENT evidence, not historical
    /// membership. This exercises the production shape — the superseding
    /// reference is APPENDED and the earlier one is deliberately retained — so
    /// the test fails if the rule regresses to "any digest ever seen". Replacing
    /// the vector instead would hide exactly the defect being repaired.
    #[test]
    fn a_superseded_replay_authorization_does_not_keep_an_approval_alive() {
        let m = gate_manifest();
        let mut s =
            crate::workflow::graph_state::GraphRunStateV1::open("r", &m, "rev", "pws", "pd", "t0")
                .unwrap();
        complete(&mut s, "build");
        s.frontier = vec!["review".to_string()];
        let run_id = s.run_id.clone();

        // 1. A replay-authorization reference for the reviewed frontier.
        let (frontier_a, ref_a) = replay_authorization_ref("diego", &run_id, &["review"], "replay");
        s.evidence_refs.push(ref_a);

        // 2. A decision based on that frontier routes.
        record_gate_decision(
            &mut s,
            &m,
            human_decision_on(HumanVerdict::Approved, &frontier_a),
        )
        .unwrap();
        let d = route_after_gate(
            &s,
            &m,
            ExecutionLimits::default(),
            "review",
            OutcomeCategory::Completed,
            "t",
        )
        .expect("the reviewed frontier authorizes routing");
        assert_eq!(d.to_node, "merge");

        // 3. The run advances: a second reference is appended for the new
        //    frontier, and the earlier one is NOT removed.
        let (frontier_b, ref_b) = replay_authorization_ref("diego", &run_id, &["merge"], "replay");
        s.evidence_refs.push(ref_b);
        s.frontier = vec!["merge".to_string()];
        assert!(
            s.evidence_refs
                .iter()
                .any(|r| r.artifact_digest == frontier_a),
            "the superseded reference is deliberately retained (production shape)"
        );

        // 4. The decision based on the superseded frontier no longer authorizes.
        let err = route_after_gate(
            &s,
            &m,
            ExecutionLimits::default(),
            "review",
            OutcomeCategory::Completed,
            "t",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("stale"), "{err}");

        // 5. A decision based on the current frontier proceeds.
        record_gate_decision(
            &mut s,
            &m,
            human_decision_on(HumanVerdict::Approved, &frontier_b),
        )
        .unwrap();
        let d = route_after_gate(
            &s,
            &m,
            ExecutionLimits::default(),
            "review",
            OutcomeCategory::Completed,
            "t",
        )
        .expect("the current frontier authorizes routing");
        assert_eq!(d.to_node, "merge");
    }

    /// #265: a rationale-free decision must not be sealable at all, and the
    /// refusal must live on the record's authoring path so a non-CLI producer
    /// cannot bypass it.
    #[test]
    fn a_decision_without_a_rationale_cannot_be_authored() {
        for reason in ["", "   ", "\n\t ", " \r\n "] {
            let err = HumanDecisionRecordV1::author(
                "review",
                HumanVerdict::Approved,
                "diego",
                ReviewChannel::CliInteractive,
                "a".repeat(64),
                reason,
                "2026-08-25T00:00:00Z",
            )
            .expect_err("a rationale-free decision must be refused");
            assert!(err.to_string().contains("rationale"), "{err}");
        }

        // A real rationale is unaffected.
        let ok = human_decision_on(HumanVerdict::Approved, &"a".repeat(64));
        assert!(ok.content_digest.is_some(), "a real decision still seals");
    }

    #[test]
    fn classified_failures_route_separately() {
        let m = GraphManifestV1 {
            schema_version: GRAPH_SCHEMA_VERSION.into(),
            graph_id: "cls".into(),
            version: "1.0.0".into(),
            nodes: vec![
                node("run", "cap.run"),
                node("fix-code", "cap.fix"),
                node("infra-diag", "cap.diag"),
                node("policy-review", "cap.pol"),
                node("ev-recover", "cap.ev"),
                node("done", "cap.done"),
            ],
            edges: vec![
                edge("run", "fix-code", Some(FailureClass::Code.route_label())),
                edge(
                    "run",
                    "infra-diag",
                    Some(FailureClass::Infrastructure.route_label()),
                ),
                edge(
                    "run",
                    "policy-review",
                    Some(FailureClass::Policy.route_label()),
                ),
                edge(
                    "run",
                    "ev-recover",
                    Some(FailureClass::Evidence.route_label()),
                ),
                edge("fix-code", "done", None),
                edge("infra-diag", "done", None),
                edge("policy-review", "done", None),
                edge("ev-recover", "done", None),
            ],
            entry_points: vec!["run".into()],
            terminal_exits: vec!["done".into()],
            shared_state_keys: vec![],
            policy_digest: None,
            content_digest: None,
        }
        .sealed();

        // Each failure class selects its OWN edge; no cross-routing.
        for (class, target) in [
            (FailureClass::Code, "fix-code"),
            (FailureClass::Infrastructure, "infra-diag"),
            (FailureClass::Policy, "policy-review"),
            (FailureClass::Evidence, "ev-recover"),
        ] {
            let mut s = crate::workflow::graph_state::GraphRunStateV1::open(
                "r", &m, "rev", "pws", "pd", "t0",
            )
            .unwrap();
            complete(&mut s, "run");
            // Generic `failed` matches NO edge (labels are class-specific):
            // generic outcome must never cross-route into a class branch.
            let err = route_after(
                &s,
                &m,
                ExecutionLimits::default(),
                "run",
                OutcomeCategory::Failed,
                "t",
            )
            .unwrap_err()
            .to_string();
            assert!(err.contains("missing route"), "{err}");
            // The class-specific routing uses the class label directly.
            let eligible = m
                .edges
                .iter()
                .filter(|e| {
                    e.from == "run" && e.condition_label.as_deref() == Some(class.route_label())
                })
                .map(|e| e.to.clone())
                .collect::<Vec<_>>();
            assert_eq!(eligible, vec![target.to_string()], "{class:?}");
        }
    }

    #[test]
    fn retry_edges_respect_caps() {
        let loop_graph = GraphManifestV1 {
            schema_version: GRAPH_SCHEMA_VERSION.into(),
            graph_id: "lp".into(),
            version: "1.0.0".into(),
            nodes: vec![node("a", "cap.a"), node("done", "cap.d")],
            edges: vec![
                edge("a", "a", Some("failed")),
                edge("a", "done", Some("completed")),
            ],
            entry_points: vec!["a".into()],
            terminal_exits: vec!["done".into()],
            shared_state_keys: vec![],
            policy_digest: None,
            content_digest: None,
        }
        .sealed();
        let limits = ExecutionLimits {
            max_cycles_per_node: 3,
            max_node_attempts_total: 16,
        };
        let mut s = crate::workflow::graph_state::GraphRunStateV1::open(
            "r",
            &loop_graph,
            "rev",
            "pws",
            "pd",
            "t0",
        )
        .unwrap();
        for _ in 0..2 {
            complete(&mut s, "a");
            let d = route_after(&s, &loop_graph, limits, "a", OutcomeCategory::Failed, "t")
                .expect("retry within cap");
            s.record_route_decision(d, &loop_graph).unwrap();
        }
        complete(&mut s, "a");
        let err = route_after(&s, &loop_graph, limits, "a", OutcomeCategory::Failed, "t")
            .unwrap_err()
            .to_string();
        assert!(err.contains("cycle limit exceeded"), "{err}");
    }

    #[test]
    fn gate_decisions_survive_checkpoints() {
        let m = gate_manifest();
        let mut s =
            crate::workflow::graph_state::GraphRunStateV1::open("r", &m, "rev", "pws", "pd", "t0")
                .unwrap();
        record_gate_decision(&mut s, &m, human_decision(HumanVerdict::Approved)).unwrap();
        let ckpt = s.export_checkpoint().unwrap();
        let back =
            crate::workflow::graph_state::GraphRunStateV1::import_checkpoint(&ckpt, &m, "pd")
                .unwrap();
        assert!(back.gate_decisions.contains_key("review"));
        assert_eq!(
            back.gate_decisions["review"].verdict,
            HumanVerdict::Approved
        );
    }

    #[test]
    fn human_decision_parse_fails_closed() {
        let d = human_decision(HumanVerdict::Approved);
        let good = d.to_json().unwrap();
        assert!(HumanDecisionRecordV1::parse_json(&good).is_ok());

        // Wrong schema version.
        let mut v = serde_json::to_value(&d).unwrap();
        v["schemaVersion"] = serde_json::json!("2.0.0");
        assert!(HumanDecisionRecordV1::parse_json(&v.to_string()).is_err());

        // Tampered digest.
        let mut v2 = serde_json::to_value(&d).unwrap();
        v2["reason"] = serde_json::json!("tampered");
        assert!(HumanDecisionRecordV1::parse_json(&v2.to_string()).is_err());

        // Missing digest.
        let mut v3 = serde_json::to_value(&d).unwrap();
        v3["contentDigest"] = serde_json::Value::Null;
        assert!(HumanDecisionRecordV1::parse_json(&v3.to_string()).is_err());

        // Machine-actor identity refused on IMPORT too.
        let mut v4 = serde_json::to_value(&d).unwrap();
        v4["decidedBy"] = serde_json::json!("model:gpt-x");
        let err4 = HumanDecisionRecordV1::parse_json(&v4.to_string())
            .unwrap_err()
            .to_string();
        assert!(err4.contains("machine actor"), "{err4}");
    }

    #[test]
    fn pre_122_checkpoints_still_import() {
        let m = gate_manifest();
        // A literal PRE-#122 sealed checkpoint: no gateDecisions key, digest
        // computed over exactly these bytes by the old law.
        let legacy = serde_json::json!({
            "schemaVersion": "1.0.0",
            "runId": "legacy-run",
            "graphId": "gated",
            "graphManifestDigest": m.compute_digest(),
            "repoRevision": "rev",
            "nodeAttempts": {
                "build": [{
                    "attempt": 1,
                    "startedAt": "t",
                    "completedAt": "t",
                    "outcome": "completed",
                    "resultDigest": "dg-build"
                }]
            },
            "frontier": ["review"],
            "decisions": [],
            "evidenceRefs": [],
            "portableStateRef": "pws.json",
            "portableStateDigest": "pd",
            "contentDigest": "PLACEHOLDER"
        });
        // Compute the digest the OLD law would have pinned: state minus
        // contentDigest, WITHOUT any gateDecisions member. With the empty-map
        // skip rule, today's compute_digest reproduces those bytes.
        let probe = HumanDecisionProbe {
            inner: legacy.clone(),
        };
        let computed = probe.digest_without_content_digest();
        let mut final_legacy = legacy;
        final_legacy["contentDigest"] = serde_json::json!(computed);

        let imported = crate::workflow::graph_state::GraphRunStateV1::import_checkpoint(
            &final_legacy.to_string(),
            &m,
            "pd",
        )
        .expect("pre-#122 checkpoint imports after #122");
        assert_eq!(imported.frontier, vec!["review".to_string()]);
        assert!(imported.gate_decisions.is_empty());
    }

    /// Digest helper mirroring GraphRunStateV1::compute_digest over an
    /// arbitrary JSON object minus contentDigest (fixture-only).
    struct HumanDecisionProbe {
        inner: serde_json::Value,
    }
    impl HumanDecisionProbe {
        fn digest_without_content_digest(&self) -> String {
            let mut v = self.inner.clone();
            if let Some(obj) = v.as_object_mut() {
                obj.remove("contentDigest");
            }
            crate::workflow::soma::try_canonical_digest(&v)
                .expect("canonical digest of Lite-constructed record cannot fail")
        }
    }
}
