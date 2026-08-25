//! Deterministic sequential graph routing with bounded cycles
//! (issue #121). Pure functions over the durable `GraphRunStateV1`:
//! the same durable state plus inputs always produces the same route.
//!
//! Routing law (fail closed):
//! - a node's `OutcomeCategory` selects among its outgoing edges — sequence
//!   edges are unconditionally eligible, conditional edges are eligible iff
//!   their label equals the outcome label;
//! - zero eligible edges => [`RouteError::MissingRoute`];
//! - multiple eligible edges => [`RouteError::AmbiguousRoute`] with the
//!   candidate targets (sorted);
//! - per-node cycle caps and the global attempt budget are enforced BEFORE
//!   any decision is applied.

use crate::workflow::graph_state::{
    GraphManifestV1, GraphRunStateV1, OutcomeCategory, RouteDecisionV1,
};

/// Execution limits bounding cycles and total work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionLimits {
    /// Maximum recorded visits of ONE node before routing away is refused.
    pub max_cycles_per_node: u32,
    /// Maximum total journaled attempts across the whole run.
    pub max_node_attempts_total: u32,
}

impl Default for ExecutionLimits {
    fn default() -> Self {
        Self {
            max_cycles_per_node: 8,
            max_node_attempts_total: 64,
        }
    }
}

/// Typed routing failures (all fail closed; none collapse into a generic error).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteError {
    /// The outcome matched no outgoing edge.
    MissingRoute {
        from: String,
        outcome: OutcomeCategory,
    },
    /// The outcome matched several edges; targets are sorted candidates.
    AmbiguousRoute { from: String, targets: Vec<String> },
    /// Routing would exceed the per-node visit cap.
    CycleLimitExceeded { node: String, limit: u32 },
    /// Routing would exceed the global attempt budget.
    AttemptBudgetExhausted { limit: u32 },
    /// The source node has no journaled completion to route on.
    UnjournaledSource { node: String },
}

impl std::fmt::Display for RouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RouteError::MissingRoute { from, outcome } => {
                write!(f, "missing route from {from:?} for outcome {outcome:?}")
            }
            RouteError::AmbiguousRoute { from, targets } => {
                write!(f, "ambiguous route from {from:?}: candidates {targets:?}")
            }
            RouteError::CycleLimitExceeded { node, limit } => {
                write!(f, "cycle limit exceeded at {node:?} (limit {limit})")
            }
            RouteError::AttemptBudgetExhausted { limit } => {
                write!(f, "global attempt budget exhausted (limit {limit})")
            }
            RouteError::UnjournaledSource { node } => {
                write!(f, "cannot route from {node:?}: no journaled completion")
            }
        }
    }
}

impl std::error::Error for RouteError {}

impl OutcomeCategory {
    /// Canonical snake_case label used to match conditional edge labels.
    pub fn label(self) -> &'static str {
        match self {
            OutcomeCategory::Completed => "completed",
            OutcomeCategory::Failed => "failed",
            OutcomeCategory::Blocked => "blocked",
            OutcomeCategory::ReviewRequired => "review-required",
            OutcomeCategory::Cancelled => "cancelled",
        }
    }
}

impl From<crate::workflow::node_contracts::OutcomeKind> for OutcomeCategory {
    fn from(k: crate::workflow::node_contracts::OutcomeKind) -> Self {
        use crate::workflow::node_contracts::OutcomeKind as K;
        match k {
            K::Completed => Self::Completed,
            K::Failed => Self::Failed,
            K::Blocked => Self::Blocked,
            K::ReviewRequired => Self::ReviewRequired,
            K::Cancelled => Self::Cancelled,
        }
    }
}

/// Deterministically select the next route after `from_node` completed with
/// `outcome`. Pure: depends only on the durable state, manifest, limits, and
/// the supplied timestamp. Enforces cycle/attempt budgets BEFORE returning
/// the decision; callers persist it via `record_route_decision`.
pub fn route_after(
    state: &GraphRunStateV1,
    manifest: &GraphManifestV1,
    limits: ExecutionLimits,
    from_node: &str,
    outcome: OutcomeCategory,
    recorded_at: impl Into<String>,
) -> Result<RouteDecisionV1, RouteError> {
    // The source must have a journaled completion to route on.
    if state
        .node_attempts
        .get(from_node)
        .map(|v| v.is_empty())
        .unwrap_or(true)
    {
        return Err(RouteError::UnjournaledSource {
            node: from_node.to_string(),
        });
    }
    // Budgets next: refuse BEFORE producing a decision.
    let total_attempts: u32 = state.node_attempts.values().map(|v| v.len() as u32).sum();
    if total_attempts >= limits.max_node_attempts_total {
        return Err(RouteError::AttemptBudgetExhausted {
            limit: limits.max_node_attempts_total,
        });
    }
    let node_visits = state
        .node_attempts
        .get(from_node)
        .map(|v| v.len() as u32)
        .unwrap_or(0);
    if node_visits >= limits.max_cycles_per_node {
        return Err(RouteError::CycleLimitExceeded {
            node: from_node.to_string(),
            limit: limits.max_cycles_per_node,
        });
    }

    // Eligibility by outcome label.
    let mut eligible: Vec<&crate::workflow::graph_state::GraphEdgeV1> = manifest
        .edges
        .iter()
        .filter(|e| e.from == from_node)
        .filter(|e| match e.kind {
            crate::workflow::graph_state::EdgeKind::Sequence => true,
            crate::workflow::graph_state::EdgeKind::Conditional => {
                e.condition_label.as_deref() == Some(outcome.label())
            }
        })
        .collect();
    if eligible.is_empty() {
        return Err(RouteError::MissingRoute {
            from: from_node.into(),
            outcome,
        });
    }
    eligible.sort_by(|a, b| a.to.cmp(&b.to));
    if eligible.len() > 1 {
        return Err(RouteError::AmbiguousRoute {
            from: from_node.into(),
            targets: eligible.iter().map(|e| e.to.clone()).collect(),
        });
    }
    Ok(RouteDecisionV1 {
        recorded_at: recorded_at.into(),
        from_node: from_node.to_string(),
        to_node: eligible[0].to.clone(),
        condition_label: eligible[0].condition_label.clone(),
        basis_result_digest: state
            .node_attempts
            .get(from_node)
            .and_then(|v| v.last())
            .map(|a| a.result_digest.clone())
            .expect("source journaled (checked above)"),
    })
}

/// A run is complete when its frontier is empty: every path reached a
/// terminal exit (terminals never enter the frontier per #120).
pub fn run_complete(state: &GraphRunStateV1) -> bool {
    state.frontier.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::graph_state::{EdgeKind, GraphEdgeV1, GraphNodeV1, NodeAttemptRecordV1};

    fn node(id: &str) -> GraphNodeV1 {
        GraphNodeV1 {
            node_id: id.into(),
            capability: format!("cap.{id}"),
            purpose: None,
        }
    }

    fn seq(from: &str, to: &str) -> GraphEdgeV1 {
        GraphEdgeV1 {
            from: from.into(),
            to: to.into(),
            kind: EdgeKind::Sequence,
            condition_label: None,
        }
    }

    fn cond(from: &str, to: &str, label: &str) -> GraphEdgeV1 {
        GraphEdgeV1 {
            from: from.into(),
            to: to.into(),
            kind: EdgeKind::Conditional,
            condition_label: Some(label.into()),
        }
    }

    fn three_node() -> GraphManifestV1 {
        GraphManifestV1 {
            schema_version: crate::workflow::graph_state::GRAPH_SCHEMA_VERSION.into(),
            graph_id: "g3".into(),
            version: "1.0.0".into(),
            nodes: vec![node("a"), node("b"), node("done")],
            edges: vec![seq("a", "b"), seq("b", "done")],
            entry_points: vec!["a".into()],
            terminal_exits: vec!["done".into()],
            shared_state_keys: vec![],
            policy_digest: None,
            content_digest: None,
        }
        .sealed()
    }

    fn complete(state: &mut GraphRunStateV1, node_id: &str) {
        state
            .apply_node_completion(
                node_id,
                NodeAttemptRecordV1 {
                    attempt: 1,
                    started_at: "t".into(),
                    completed_at: Some("t".into()),
                    outcome: OutcomeCategory::Completed,
                    result_digest: format!("digest-{node_id}"),
                },
                "now",
            )
            .expect("completion on frontier");
    }

    #[test]
    fn three_node_graph_executes_deterministically_to_terminal() {
        let m = three_node();
        let mut s = GraphRunStateV1::open("r", &m, "rev", "pws", "pd", "t0").unwrap();
        assert_eq!(s.frontier, vec!["a"]);

        // Execution order per the #120 law: complete the frontier head,
        // route, which admits the next node, repeat.
        complete(&mut s, "a");
        let d1 = route_after(
            &s,
            &m,
            ExecutionLimits::default(),
            "a",
            OutcomeCategory::Completed,
            "t1",
        )
        .expect("route a->b");
        s.record_route_decision(d1.clone(), &m).unwrap();
        assert_eq!(s.frontier, vec!["b"]);
        complete(&mut s, "b");
        let d2 = route_after(
            &s,
            &m,
            ExecutionLimits::default(),
            "b",
            OutcomeCategory::Completed,
            "t2",
        )
        .expect("route b->done");
        s.record_route_decision(d2, &m).unwrap();
        // Terminal exits never enter the frontier: run complete.
        assert!(run_complete(&s));
        assert_eq!(s.decisions.len(), 2);

        // Decisions persisted as evidence: checkpoint round-trip keeps them.
        let ckpt = s.export_checkpoint().unwrap();
        let back = GraphRunStateV1::import_checkpoint(&ckpt, &m, "pd").unwrap();
        assert_eq!(back.decisions, s.decisions);
    }

    #[test]
    fn same_durable_state_same_route() {
        let m = three_node();
        let mk = || {
            let mut s = GraphRunStateV1::open("r", &m, "rev", "pws", "pd", "t0").unwrap();
            complete(&mut s, "a");
            s
        };
        let s1 = mk();
        let s2 = mk();
        let d1 = route_after(
            &s1,
            &m,
            ExecutionLimits::default(),
            "a",
            OutcomeCategory::Completed,
            "T",
        );
        let d2 = route_after(
            &s2,
            &m,
            ExecutionLimits::default(),
            "a",
            OutcomeCategory::Completed,
            "T",
        );
        assert_eq!(d1, d2, "pure function of durable state + inputs");
    }

    #[test]
    fn missing_and_ambiguous_routes_fail_closed() {
        // Missing: a reachable leaf node with no outgoing edges (and not a
        // terminal) leaves its outcome with zero eligible routes.
        let mut leafy = three_node();
        leafy.nodes.push(node("leaf"));
        leafy.edges.push(seq("b", "leaf"));
        let leafy = leafy.sealed();
        leafy.validate().expect("reachable");
        let mut s0 = GraphRunStateV1::open("r", &leafy, "rev", "pws", "pd", "t0").unwrap();
        complete(&mut s0, "a");
        let d = route_after(
            &s0,
            &leafy,
            ExecutionLimits::default(),
            "a",
            OutcomeCategory::Completed,
            "t",
        )
        .expect("route a->b");
        s0.record_route_decision(d, &leafy).unwrap();
        complete(&mut s0, "b");
        // b has two outgoing: seq to done and seq to leaf -> ambiguous!
        assert!(matches!(
            route_after(
                &s0,
                &leafy,
                ExecutionLimits::default(),
                "b",
                OutcomeCategory::Completed,
                "t"
            ),
            Err(RouteError::AmbiguousRoute { .. })
        ));

        // Pure missing: single conditional whose label never matches.
        let only_cond = GraphManifestV1 {
            edges: vec![cond("a", "done", "failed")],
            nodes: vec![node("a"), node("done")],
            ..three_node()
        }
        .sealed();
        only_cond.validate().expect("valid");
        let mut s1 = GraphRunStateV1::open("r2", &only_cond, "rev", "pws", "pd", "t0").unwrap();
        complete(&mut s1, "a");
        assert!(matches!(
            route_after(
                &s1,
                &only_cond,
                ExecutionLimits::default(),
                "a",
                OutcomeCategory::Completed,
                "t"
            ),
            Err(RouteError::MissingRoute { .. })
        ));

        // Ambiguous: two conditional edges sharing one outcome label.
        let amb = GraphManifestV1 {
            edges: vec![
                cond("a", "x", "failed"),
                cond("a", "y", "failed"),
                seq("x", "done"),
                seq("y", "done"),
            ],
            nodes: vec![node("a"), node("x"), node("y"), node("done")],
            ..three_node()
        }
        .sealed();
        amb.validate().expect("structure valid");
        let mut s2 = GraphRunStateV1::open("r3", &amb, "rev", "pws", "pd", "t0").unwrap();
        complete(&mut s2, "a");
        match route_after(
            &s2,
            &amb,
            ExecutionLimits::default(),
            "a",
            OutcomeCategory::Failed,
            "t",
        ) {
            Err(RouteError::AmbiguousRoute { targets, .. }) => {
                assert_eq!(targets, vec!["x".to_string(), "y".to_string()])
            }
            other => panic!("expected ambiguity, got {other:?}"),
        }

        // Conditional label not matching the emitted outcome is missing.
        assert!(matches!(
            route_after(
                &s2,
                &amb,
                ExecutionLimits::default(),
                "a",
                OutcomeCategory::Blocked,
                "t"
            ),
            Err(RouteError::MissingRoute { .. })
        ));
    }

    #[test]
    fn attempt_and_cycle_limits_stop_loops() {
        // Self-loop graph: a --(retry)--> a, a --> done.
        let loop_graph = GraphManifestV1 {
            schema_version: crate::workflow::graph_state::GRAPH_SCHEMA_VERSION.into(),
            graph_id: "loop".into(),
            version: "1.0.0".into(),
            nodes: vec![node("a"), node("done")],
            // Failure retries the same node; completion exits to the
            // terminal. Both conditional => never ambiguous per outcome.
            edges: vec![cond("a", "a", "failed"), cond("a", "done", "completed")],
            entry_points: vec!["a".into()],
            terminal_exits: vec!["done".into()],
            shared_state_keys: vec![],
            policy_digest: None,
            content_digest: None,
        }
        .sealed();
        // Cap 3 allows two retry loops; the third route refuses.
        let limits = ExecutionLimits {
            max_cycles_per_node: 3,
            max_node_attempts_total: 16,
        };
        let mut s = GraphRunStateV1::open("r", &loop_graph, "rev", "pws", "pd", "t0").unwrap();

        // Visit a twice via retry loops.
        for i in 0..2 {
            complete(&mut s, "a");
            let d = route_after(
                &s,
                &loop_graph,
                limits,
                "a",
                OutcomeCategory::Failed, // matches retry label
                "t",
            )
            .unwrap_or_else(|e| panic!("visit {i} must route: {e}"));
            s.record_route_decision(d, &loop_graph).unwrap();
        }
        // Third cycle exceeds the per-node cap.
        complete(&mut s, "a");
        assert!(matches!(
            route_after(&s, &loop_graph, limits, "a", OutcomeCategory::Failed, "t"),
            Err(RouteError::CycleLimitExceeded { node, limit }) if node == "a" && limit == 3
        ));

        // Global budget exhaustion: two journaled attempts already exhaust
        // a budget of 2, so routing b onward refuses.
        let tight = ExecutionLimits {
            max_cycles_per_node: 9,
            max_node_attempts_total: 2,
        };
        let m3 = three_node();
        let mut s2 = GraphRunStateV1::open("r2", &m3, "rev", "pws", "pd", "t0").unwrap();
        complete(&mut s2, "a");
        let d = route_after(
            &s2,
            &m3,
            ExecutionLimits::default(),
            "a",
            OutcomeCategory::Completed,
            "t",
        )
        .expect("first route inside budget");
        s2.record_route_decision(d, &m3).unwrap();
        complete(&mut s2, "b");
        assert!(matches!(
            route_after(&s2, &m3, tight, "b", OutcomeCategory::Completed, "t"),
            Err(RouteError::AttemptBudgetExhausted { limit: 2 })
        ));
    }

    #[test]
    fn unjournaled_routing_refused() {
        let m = three_node();
        let s = GraphRunStateV1::open("r", &m, "rev", "pws", "pd", "t0").unwrap();
        // Routing from a node with NO journaled completion refuses outright:
        // an ungated decision can never be produced.
        assert!(matches!(
            route_after(&s, &m, ExecutionLimits::default(), "a", OutcomeCategory::Completed, "t"),
            Err(RouteError::UnjournaledSource { node }) if node == "a"
        ));
    }
}
