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
use crate::workflow::soma::canonical_digest;

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
        Ok(Self {
            schema_version: HUMAN_DECISION_VERSION.into(),
            gate_node_id: gate_node_id.into(),
            verdict,
            decided_by,
            channel,
            basis_evidence_digest,
            reason: reason.into(),
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
        canonical_digest(&v)
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

/// Compute the gate-filtered decision for `from_node`.
///
/// Non-gate nodes delegate to the #121 outcome mapping unchanged. Gates:
/// eligibility keys off the recorded HUMAN verdict only — outcomes never
/// open a gate, and no decision means pending (fail closed). After a
/// rejection the target must be a declared terminal exit.
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
                    result_digest: format!("dg-{id}"),
                },
                "now",
            )
            .unwrap();
    }

    fn human_decision(verdict: HumanVerdict) -> HumanDecisionRecordV1 {
        HumanDecisionRecordV1::author(
            "review",
            verdict,
            "diego",
            ReviewChannel::CliInteractive,
            "e".repeat(64),
            "lgtm",
            "2026-08-25T00:00:00Z",
        )
        .unwrap()
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
            crate::workflow::soma::canonical_digest(&v)
        }
    }
}
