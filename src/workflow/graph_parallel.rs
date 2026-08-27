//! Parallel branches, joins, resource locks, graph evidence index (#124).
//!
//! Extends the #121 inevitable-forward-progress law additively:
//! - Resource declarations (`resources: Vec<String>` on nodes) partition the
//!   frontier into concurrency waves where no two co-scheduled nodes share a
//!   resource — conflicting writers (e.g. `repo:write`) cannot co-schedule.
//!   Waves are deterministic (sorted, greedy) so scheduling is replayable.
//! - Join policies (`join: Option<JoinPolicyV1>`) synchronize required
//!   branches. A `JoinEvaluationV1` records the latest outcome per
//!   predecessor (evidence preserved verbatim in node attempts); the
//!   evaluation's digest is the routing basis (mirrors the #122 gate
//!   pattern).
//! - Partial branch failures route deterministically: a join with
//!   `allRequired=false` records Failed predecessors in the eval and the
//!   runner emits a Completed outcome for the join (with the failure
//!   evidence preserved). Partial outcomes are an EXPLICIT allowlist.
//! - `build_evidence_index` returns one canonical digested view over every
//!   node attempt, route decision, gate decision, join evaluation, and
//!   evidence reference — graph-level audit.
//! - `export_mermaid` emits a deterministic Mermaid state diagram from the
//!   durable state (frontier + termination + gates classed).

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use super::graph_state::{
    GraphEdgeV1, GraphManifestV1, GraphRunStateV1, NodeAttemptRecordV1, OutcomeCategory,
    RouteDecisionV1,
};
use super::soma::canonical_digest;

// ---------------------------------------------------------------------------
// JoinPolicyV1 + JoinEvaluationV1
// ---------------------------------------------------------------------------

/// Join semantics for a synchronization node (#124). When `allRequired` is
/// true every predecessor branch must complete; when false, branches whose
/// latest outcome appears in `partialOutcomes` are accepted as "partial" and
/// the join still evaluates as satisfied (evidence preserved).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JoinPolicyV1 {
    pub all_required: bool,
    /// Explicit partial-outcome labels. Each entry names a category the join
    /// accepts in lieu of `Completed`. Empty list with `allRequired=false`
    /// means "any non-running outcome satisfies" — but the runner still has
    /// to declare intent; an empty allowlist is the literal default and the
    /// runner may interpret it either way (we treat it as "Completed only"
    /// in this contract to stay conservative).
    #[serde(default, rename = "partialOutcomes")]
    pub partial_outcomes: Vec<OutcomeLabel>,
}

/// Wire-safe outcome label for partial-join policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeLabel {
    Completed,
    Failed,
    Blocked,
    ReviewRequired,
    Cancelled,
}

impl From<OutcomeCategory> for OutcomeLabel {
    fn from(c: OutcomeCategory) -> Self {
        match c {
            OutcomeCategory::Completed => OutcomeLabel::Completed,
            OutcomeCategory::Failed => OutcomeLabel::Failed,
            OutcomeCategory::Blocked => OutcomeLabel::Blocked,
            OutcomeCategory::ReviewRequired => OutcomeLabel::ReviewRequired,
            OutcomeCategory::Cancelled => OutcomeLabel::Cancelled,
        }
    }
}

/// Sealed evaluation of a join's predecessors at a point in time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JoinEvaluationV1 {
    pub join_node: String,
    /// Per-predecessor latest label (`None` when the predecessor has no
    /// recorded completed attempt yet — the join is NOT ready).
    pub branches: BTreeMap<String, Option<OutcomeLabel>>,
    /// True when, given the join policy, every required branch is
    /// accounted for (Completed or allowed partial).
    pub satisfied: bool,
    #[serde(rename = "recordedAt")]
    pub recorded_at: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "contentDigest"
    )]
    pub content_digest: Option<String>,
}

impl JoinEvaluationV1 {
    pub fn compute_digest(&self) -> String {
        let mut v = serde_json::to_value(self).expect("joineval serializes");
        if let Some(obj) = v.as_object_mut() {
            obj.remove("contentDigest");
        }
        canonical_digest(&v)
    }

    fn seal_in_place(&mut self) {
        self.content_digest = Some(self.compute_digest());
    }
}

/// Typed contradictions for join evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinError {
    /// Node does not declare a join policy.
    NotAJoinNode,
    /// Policy requires a non-empty predecessor set but the manifest has none.
    NoPredecessors,
}

impl std::fmt::Display for JoinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            JoinError::NotAJoinNode => "node is not a declared join node",
            JoinError::NoPredecessors => "join node has no predecessors",
        };
        f.write_str(s)
    }
}

impl std::error::Error for JoinError {}

/// Evaluate a join node given the current state, returning a sealed
/// [`JoinEvaluationV1`]. Evidence in `state.node_attempts` is read but never
/// mutated — partial failures are recorded in the evaluation's branch map.
pub fn evaluate_join(
    state: &GraphRunStateV1,
    manifest: &GraphManifestV1,
    join_node: &str,
    recorded_at: impl Into<String>,
) -> Result<JoinEvaluationV1> {
    let policy = manifest
        .nodes
        .iter()
        .find(|n| n.node_id == join_node)
        .and_then(|n| n.join.as_ref())
        .ok_or(JoinError::NotAJoinNode)?;

    let preds: Vec<&GraphEdgeV1> = manifest
        .edges
        .iter()
        .filter(|e| e.to == join_node)
        .collect();
    if preds.is_empty() {
        anyhow::bail!("{}", JoinError::NoPredecessors);
    }

    let mut branches = BTreeMap::new();
    let mut satisfied = true;
    for e in &preds {
        let latest = state
            .node_attempts
            .get(&e.from)
            .and_then(|ats| ats.iter().rev().find(|a| a.completed_at.is_some()));
        let label = latest.map(|a| OutcomeLabel::from(a.outcome));
        let ok = match label {
            Some(OutcomeLabel::Completed) => true,
            Some(other) => !policy.all_required && policy.partial_outcomes.contains(&other),
            None => false,
        };
        if !ok {
            satisfied = false;
        }
        branches.insert(e.from.clone(), label);
    }

    let mut eval = JoinEvaluationV1 {
        join_node: join_node.to_string(),
        branches,
        satisfied,
        recorded_at: recorded_at.into(),
        content_digest: None,
    };
    eval.seal_in_place();
    Ok(eval)
}

/// Record a sealed join evaluation into the durable state. Respects the
/// `joinEvaluations` map (mirrors #122 gate recording). Does NOT mutate node
/// attempts — partial failures stay as evidence. The eval is trusted to have
/// been produced by [`evaluate_join`] (which validated it against the
/// manifest); we only reject an empty join node.
pub fn record_join(state: &mut GraphRunStateV1, eval: JoinEvaluationV1) -> Result<()> {
    if eval.join_node.is_empty() {
        anyhow::bail!("{}", JoinError::NotAJoinNode);
    }
    state.join_evaluations.insert(eval.join_node.clone(), eval);
    state.content_digest = Some(state.compute_digest());
    Ok(())
}

// ---------------------------------------------------------------------------
// Resource locks + concurrency waves
// ---------------------------------------------------------------------------

/// Greedy, deterministic partition of the frontier into concurrency waves.
/// Two nodes share a wave iff their resource sets are DISJOINT. Wave order
/// is the input order (frontier order); intra-wave order is sorted by
/// node_id for replayability.
pub fn concurrency_waves(manifest: &GraphManifestV1, frontier: &[String]) -> Vec<Vec<String>> {
    let mut waves: Vec<(BTreeSet<String>, Vec<String>)> = Vec::new();
    for node_id in frontier {
        let resources = node_resources(manifest, node_id);
        let placed = waves.iter_mut().find_map(|(held, order)| {
            if resources.is_disjoint(held) {
                order.push(node_id.clone());
                held.extend(resources.iter().cloned());
                Some(())
            } else {
                None
            }
        });
        if placed.is_none() {
            let mut held = BTreeSet::new();
            held.extend(resources.iter().cloned());
            waves.push((held, vec![node_id.clone()]));
        }
    }
    waves.into_iter().map(|(_, order)| order).collect()
}

fn node_resources(manifest: &GraphManifestV1, node_id: &str) -> BTreeSet<String> {
    manifest
        .nodes
        .iter()
        .find(|n| n.node_id == node_id)
        .map(|n| n.resources.iter().cloned().collect())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Graph evidence index
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IndexedAttemptV1 {
    #[serde(rename = "nodeId")]
    pub node_id: String,
    pub attempt: u32,
    pub outcome: OutcomeCategory,
    #[serde(rename = "startedAt")]
    pub started_at: String,
    #[serde(default, rename = "completedAt")]
    pub completed_at: Option<String>,
    #[serde(rename = "resultDigest")]
    pub result_digest: String,
}

impl From<&NodeAttemptRecordV1> for IndexedAttemptV1 {
    fn from(a: &NodeAttemptRecordV1) -> Self {
        Self {
            node_id: String::new(),
            attempt: a.attempt,
            outcome: a.outcome,
            started_at: a.started_at.clone(),
            completed_at: a.completed_at.clone(),
            result_digest: a.result_digest.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IndexedDecisionV1 {
    #[serde(rename = "recordedAt")]
    pub recorded_at: String,
    #[serde(rename = "fromNode")]
    pub from_node: String,
    #[serde(rename = "toNode")]
    pub to_node: String,
    #[serde(default, rename = "conditionLabel")]
    pub condition_label: Option<String>,
    #[serde(rename = "basisResultDigest")]
    pub basis_result_digest: String,
}

impl From<&RouteDecisionV1> for IndexedDecisionV1 {
    fn from(d: &RouteDecisionV1) -> Self {
        Self {
            recorded_at: d.recorded_at.clone(),
            from_node: d.from_node.clone(),
            to_node: d.to_node.clone(),
            condition_label: d.condition_label.clone(),
            basis_result_digest: d.basis_result_digest.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IndexedJoinV1 {
    #[serde(rename = "joinNode")]
    pub join_node: String,
    pub satisfied: bool,
    #[serde(rename = "recordedAt")]
    pub recorded_at: String,
    #[serde(rename = "contentDigest")]
    pub content_digest: String,
    /// Per-branch latest outcome label for transparency.
    pub branches: BTreeMap<String, Option<OutcomeLabel>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GraphEvidenceIndexV1 {
    #[serde(rename = "schemaVersion")]
    pub schema_version: String,
    #[serde(rename = "runId")]
    pub run_id: String,
    #[serde(rename = "graphId")]
    pub graph_id: String,
    #[serde(rename = "repoRevision")]
    pub repo_revision: String,
    pub attempts: Vec<IndexedAttemptV1>,
    pub decisions: Vec<IndexedDecisionV1>,
    pub joins: Vec<IndexedJoinV1>,
    #[serde(rename = "evidenceRefs")]
    pub evidence_refs: Vec<crate::workflow::memory_contracts::EvidenceReferenceV1>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "contentDigest"
    )]
    pub content_digest: Option<String>,
}

pub const EVIDENCE_INDEX_VERSION: &str = "1.0.0";

/// Build the graph-level evidence index over the current state. Returns a
/// sealed digest over the assembled view. Every attempt, decision, join
/// evaluation, and evidence reference is reachable through the index.
pub fn build_evidence_index(state: &GraphRunStateV1) -> GraphEvidenceIndexV1 {
    let mut attempts: Vec<IndexedAttemptV1> = Vec::new();
    for (node_id, ats) in &state.node_attempts {
        for a in ats {
            let mut idx = IndexedAttemptV1::from(a);
            idx.node_id = node_id.clone();
            attempts.push(idx);
        }
    }
    attempts.sort_by(|x, y| {
        x.node_id
            .cmp(&y.node_id)
            .then(x.attempt.cmp(&y.attempt))
            .then(x.started_at.cmp(&y.started_at))
    });

    let mut decisions: Vec<IndexedDecisionV1> = state
        .decisions
        .iter()
        .map(IndexedDecisionV1::from)
        .collect();
    decisions.sort_by(|x, y| {
        x.recorded_at
            .cmp(&y.recorded_at)
            .then(x.from_node.cmp(&y.from_node))
            .then(x.to_node.cmp(&y.to_node))
    });

    let mut joins: Vec<IndexedJoinV1> = state
        .join_evaluations
        .iter()
        .map(|(k, j)| IndexedJoinV1 {
            join_node: k.clone(),
            satisfied: j.satisfied,
            recorded_at: j.recorded_at.clone(),
            content_digest: j.content_digest.clone().unwrap_or_default(),
            branches: j.branches.clone(),
        })
        .collect();
    joins.sort_by(|x, y| x.join_node.cmp(&y.join_node));

    let mut idx = GraphEvidenceIndexV1 {
        schema_version: EVIDENCE_INDEX_VERSION.into(),
        run_id: state.run_id.clone(),
        graph_id: state.graph_id.clone(),
        repo_revision: state.repo_revision.clone(),
        attempts,
        decisions,
        joins,
        evidence_refs: state.evidence_refs.clone(),
        content_digest: None,
    };
    idx.content_digest = Some(compute_index_digest(&idx));
    idx
}

fn compute_index_digest(idx: &GraphEvidenceIndexV1) -> String {
    let mut v = serde_json::to_value(idx).expect("index serializes");
    if let Some(obj) = v.as_object_mut() {
        obj.remove("contentDigest");
    }
    canonical_digest(&v)
}

// ---------------------------------------------------------------------------
// Mermaid visualization
// ---------------------------------------------------------------------------

/// Emit a deterministic Mermaid `stateDiagram-v2` from the manifest + state.
/// Frontier nodes carry a `:::frontier` class; terminated runs emit a
/// terminal marker. Evidence-preserving, replayable.
pub fn export_mermaid(manifest: &GraphManifestV1, state: &GraphRunStateV1) -> String {
    let frontier: BTreeSet<&str> = state.frontier.iter().map(String::as_str).collect();
    let terminated = state.termination.is_some();
    let mut out = String::new();
    out.push_str("stateDiagram-v2\n");
    if terminated {
        out.push_str("  [*] --> terminated\n");
    }
    for n in &manifest.nodes {
        let suffix = if frontier.contains(n.node_id.as_str()) {
            ":::frontier"
        } else {
            ""
        };
        out.push_str(&format!("  {}{}\n", n.node_id, suffix));
    }
    for e in &manifest.edges {
        let arrow = match e.kind {
            super::graph_state::EdgeKind::Sequence => "-->",
            super::graph_state::EdgeKind::Conditional => "-->",
        };
        let label = e
            .condition_label
            .as_deref()
            .map(|l| format!(" : {l}"))
            .unwrap_or_default();
        out.push_str(&format!("  {} {} {}{}\n", e.from, arrow, e.to, label));
    }
    if terminated {
        out.push_str("  terminated --> [*]\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::graph_state::{EdgeKind, GRAPH_SCHEMA_VERSION, GraphEdgeV1, GraphNodeV1};

    fn manifest_with_join() -> GraphManifestV1 {
        let nodes = vec![
            GraphNodeV1 {
                node_id: "a".into(),
                capability: "c.a".into(),
                purpose: None,
                resources: vec!["repo:write".into()],
                join: None,
            },
            GraphNodeV1 {
                node_id: "b".into(),
                capability: "c.b".into(),
                purpose: None,
                resources: vec!["repo:write".into()],
                join: None,
            },
            GraphNodeV1 {
                node_id: "c".into(),
                capability: "c.c".into(),
                purpose: None,
                resources: vec![],
                join: None,
            },
            GraphNodeV1 {
                node_id: "join".into(),
                capability: "c.join".into(),
                purpose: None,
                resources: vec![],
                join: Some(JoinPolicyV1 {
                    all_required: false,
                    partial_outcomes: vec![OutcomeLabel::Failed],
                }),
            },
            GraphNodeV1 {
                node_id: "done".into(),
                capability: "c.done".into(),
                purpose: None,
                resources: vec![],
                join: None,
            },
        ];
        GraphManifestV1 {
            schema_version: GRAPH_SCHEMA_VERSION.into(),
            graph_id: "g".into(),
            version: "1.0.0".into(),
            nodes,
            edges: vec![
                GraphEdgeV1 {
                    from: "a".into(),
                    to: "join".into(),
                    kind: EdgeKind::Sequence,
                    condition_label: None,
                },
                GraphEdgeV1 {
                    from: "b".into(),
                    to: "join".into(),
                    kind: EdgeKind::Sequence,
                    condition_label: None,
                },
                GraphEdgeV1 {
                    from: "c".into(),
                    to: "join".into(),
                    kind: EdgeKind::Sequence,
                    condition_label: None,
                },
                GraphEdgeV1 {
                    from: "join".into(),
                    to: "done".into(),
                    kind: EdgeKind::Sequence,
                    condition_label: None,
                },
            ],
            entry_points: vec!["a".into(), "b".into(), "c".into()],
            terminal_exits: vec!["done".into()],
            shared_state_keys: vec![],
            policy_digest: None,
            content_digest: None,
        }
        .sealed()
    }

    fn complete_attempt() -> NodeAttemptRecordV1 {
        NodeAttemptRecordV1 {
            attempt: 1,
            started_at: "t0".into(),
            completed_at: Some("t1".into()),
            outcome: OutcomeCategory::Completed,
            result_digest: "d".repeat(64),
        }
    }

    fn failed_attempt() -> NodeAttemptRecordV1 {
        NodeAttemptRecordV1 {
            attempt: 1,
            started_at: "t0".into(),
            completed_at: Some("t1".into()),
            outcome: OutcomeCategory::Failed,
            result_digest: "e".repeat(64),
        }
    }

    /// Invariant: no two nodes in the same wave share any resource.
    fn assert_wave_invariant(m: &GraphManifestV1, waves: &[Vec<String>]) {
        for w in waves {
            let mut held: BTreeSet<String> = BTreeSet::new();
            for n in w {
                let r = node_resources(m, n);
                for x in &r {
                    assert!(
                        held.insert(x.clone()),
                        "resource {x:?} shared within wave {w:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn waves_partition_conflicting_resources() {
        let m = manifest_with_join();
        // a and b both hold repo:write -> they can NEVER co-schedule.
        let waves = concurrency_waves(&m, &["a".into(), "b".into()]);
        assert_eq!(waves, vec![vec!["a".to_string()], vec!["b".to_string()]]);
        assert_wave_invariant(&m, &waves);
    }

    #[test]
    fn waves_co_schedule_disjoint_resources() {
        let m = manifest_with_join();
        // c and done hold no resources -> they share a wave (first-fit).
        let waves = concurrency_waves(&m, &["c".into(), "done".into()]);
        assert_eq!(waves, vec![vec!["c".to_string(), "done".to_string()]]);

        // Mixed order: a repo:write, then c (no res -> joins wave0), then b
        // (repo:write conflicts with wave0 -> new wave).
        let mixed = concurrency_waves(&m, &["a".into(), "c".into(), "b".into()]);
        assert_eq!(
            mixed,
            vec![
                vec!["a".to_string(), "c".to_string()],
                vec!["b".to_string()]
            ]
        );
        assert_wave_invariant(&m, &mixed);
    }

    #[test]
    fn join_all_required_satisfied_only_when_all_complete() {
        let m = manifest_with_join();
        // Make join all_required for this test (overriding manifest default).
        let mut m2 = m.clone();
        for n in &mut m2.nodes {
            if n.node_id == "join" {
                n.join = Some(JoinPolicyV1 {
                    all_required: true,
                    partial_outcomes: vec![],
                });
            }
        }
        let m2 = m2.sealed();

        let mut state = GraphRunStateV1::open("r", &m2, "rev-1", "pws", "pd", "t0").unwrap();
        // Only a completed -> join not ready.
        state
            .apply_node_completion("a", complete_attempt(), "t1")
            .unwrap();
        let eval = evaluate_join(&state, &m2, "join", "t2").unwrap();
        assert!(!eval.satisfied);
        // b + c completed -> satisfied.
        state
            .apply_node_completion("b", complete_attempt(), "t2")
            .unwrap();
        state
            .apply_node_completion("c", complete_attempt(), "t3")
            .unwrap();
        let eval2 = evaluate_join(&state, &m2, "join", "t4").unwrap();
        assert!(eval2.satisfied);
    }

    #[test]
    fn partial_failure_routes_deterministically() {
        let m = manifest_with_join();
        let mut state = GraphRunStateV1::open("r", &m, "rev-1", "pws", "pd", "t0").unwrap();
        // a completes; b fails; c completes. partial_outcomes=[Failed] -> satisfied.
        state
            .apply_node_completion("a", complete_attempt(), "t1")
            .unwrap();
        state
            .apply_node_completion("b", failed_attempt(), "t2")
            .unwrap();
        state
            .apply_node_completion("c", complete_attempt(), "t3")
            .unwrap();
        let eval = evaluate_join(&state, &m, "join", "t4").unwrap();
        assert!(eval.satisfied);
        assert_eq!(eval.branches.get("b"), Some(&Some(OutcomeLabel::Failed)));
        // b's attempt is still in state (evidence preserved).
        assert_eq!(state.node_attempts["b"][0].outcome, OutcomeCategory::Failed);
    }

    #[test]
    fn record_join_evaluations_roundtrip() {
        let m = manifest_with_join();
        let mut state = GraphRunStateV1::open("r", &m, "rev-1", "pws", "pd", "t0").unwrap();
        state
            .apply_node_completion("a", complete_attempt(), "t1")
            .unwrap();
        let eval = evaluate_join(&state, &m, "join", "t2").unwrap();
        record_join(&mut state, eval.clone()).unwrap();
        assert!(state.join_evaluations.contains_key("join"));
        // The eval is the basis for routing; record_route_decision must accept.
        let ckpt = state.export_checkpoint().unwrap();
        assert!(ckpt.contains("joinEvaluations"));
    }

    #[test]
    fn routing_accepts_join_eval_digest_as_basis() {
        let m = manifest_with_join();
        let mut state = GraphRunStateV1::open("r", &m, "rev-1", "pws", "pd", "t0").unwrap();
        state
            .apply_node_completion("a", complete_attempt(), "t1")
            .unwrap();
        state
            .apply_node_completion("b", complete_attempt(), "t2")
            .unwrap();
        state
            .apply_node_completion("c", complete_attempt(), "t3")
            .unwrap();
        let eval = evaluate_join(&state, &m, "join", "t4").unwrap();
        assert!(eval.satisfied);
        record_join(&mut state, eval.clone()).unwrap();
        // The join evaluation's digest authorizes routing FROM the join node
        // (mirrors the #122 gate-digest basis path). The law pushes `done`
        // onto the frontier.
        state
            .record_route_decision(
                RouteDecisionV1 {
                    recorded_at: "t5".into(),
                    from_node: "join".into(),
                    to_node: "done".into(),
                    condition_label: None,
                    basis_result_digest: eval.compute_digest(),
                },
                &m,
            )
            .expect("join-eval basis accepted");
        // The decision is journaled (basis accepted); `done` is a terminal exit
        // so it does NOT enter the frontier (per the #121 forward-progress law).
        assert_eq!(state.decisions.len(), 1);
        assert!(state.frontier.is_empty());
    }

    #[test]
    fn evaluate_join_on_non_join_node_returns_typed_error() {
        let m = manifest_with_join();
        let state = GraphRunStateV1::open("r", &m, "rev-1", "pws", "pd", "t0").unwrap();
        let err = evaluate_join(&state, &m, "a", "t1")
            .unwrap_err()
            .to_string();
        assert!(err.contains("not a declared join"), "{err}");
    }

    #[test]
    fn evidence_index_links_every_artifact() {
        let m = manifest_with_join();
        let mut state = GraphRunStateV1::open("r", &m, "rev-1", "pws", "pd", "t0").unwrap();
        state
            .apply_node_completion("a", complete_attempt(), "t1")
            .unwrap();
        state
            .apply_node_completion("b", complete_attempt(), "t2")
            .unwrap();
        let eval = evaluate_join(&state, &m, "join", "t3").unwrap();
        record_join(&mut state, eval.clone()).unwrap();
        state
            .record_route_decision(
                RouteDecisionV1 {
                    recorded_at: "t4".into(),
                    from_node: "a".into(),
                    to_node: "join".into(),
                    condition_label: None,
                    basis_result_digest: "d".repeat(64),
                },
                &m,
            )
            .unwrap();
        let idx = build_evidence_index(&state);
        // Two attempts (a, b), one decision, one join evaluation.
        assert_eq!(idx.attempts.len(), 2);
        assert_eq!(idx.decisions.len(), 1);
        assert_eq!(idx.joins.len(), 1);
        assert!(idx.content_digest.is_some());
        // Determinism: rebuild and compare digests.
        let idx2 = build_evidence_index(&state);
        assert_eq!(idx.content_digest, idx2.content_digest);
    }

    #[test]
    fn mermaid_export_is_deterministic_and_marks_frontier() {
        let m = manifest_with_join();
        let mut state = GraphRunStateV1::open("r", &m, "rev-1", "pws", "pd", "t0").unwrap();
        // Frontier starts as entry points [a,b,c]. Complete "a" -> removed.
        state
            .apply_node_completion("a", complete_attempt(), "t1")
            .unwrap();
        let mm = export_mermaid(&m, &state);
        // b and c remain on the frontier; a does not; join is declared but not
        // yet on the frontier (it enters via a route decision).
        assert!(mm.contains("b:::frontier"));
        assert!(mm.contains("c:::frontier"));
        assert!(mm.contains("join")); // declared node, rendered
        assert!(!mm.contains("a:::frontier"));
        assert!(!mm.contains("join:::frontier"));
        // Re-running yields identical text.
        assert_eq!(mm, export_mermaid(&m, &state));
    }

    #[test]
    fn legacy_manifest_without_resources_or_join_imports_identically() {
        // A pre-#124 manifest (no resources/join keys) parses into the new
        // struct with default-empty additive fields and re-serializes to the
        // SAME bytes (backward-compatible: additive fields omit when empty).
        let mut m = manifest_with_join();
        for n in &mut m.nodes {
            n.resources.clear();
            n.join = None;
        }
        // Seal AFTER clearing so the stored digest matches the no-fields body.
        let pre = m.clone().sealed();
        let text = serde_json::to_string(&pre).unwrap();
        // The additive field KEYS must be absent (node "join"'s id VALUE is
        // `"join"` which is expected and fine).
        assert!(!text.contains("\"resources\":"));
        assert!(!text.contains("\"join\":"));
        let re = GraphManifestV1::parse_json(&text).expect("legacy manifest parses");
        for n in &re.nodes {
            assert!(n.resources.is_empty(), "{:?}", n.node_id);
            assert!(n.join.is_none(), "{:?}", n.node_id);
        }
        // Re-serialize of the parsed form is byte-identical.
        let text2 = serde_json::to_string(&re).unwrap();
        assert_eq!(text, text2);
    }
}
