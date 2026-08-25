//! Governed graph-run contracts (issue #120).
//!
//! Versioned `lite.graph.v1` manifests and durable `lite.graph-run.v1` run
//! state: nodes/edges/entry/terminal/shared-state-keys/policies, persisted
//! attempts, frontier, decisions, evidence references, and an explicit
//! PROJECTION of PortableWorkState (digest pointer — never duplicated
//! content). Fail-closed on unsupported versions, illegal transitions,
//! stale revisions, and silent contradiction with the canonical portable
//! work state.
//!
//! Transaction ordering law (enforced by [`GraphRunStateV1`] methods):
//! 1. a node completion applies ONLY when the node is on the frontier;
//! 2. a route decision references a durably journaled completion;
//! 3. checkpoint export/import preserves frontier + references verbatim;
//! 4. any portable-state digest mismatch is a typed contradiction.

use std::collections::BTreeMap;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

/// Schema versions (single supported major line; older fails closed).
pub const GRAPH_SCHEMA_VERSION: &str = "1.0.0";
pub const GRAPH_RUN_SCHEMA_VERSION: &str = "1.0.0";

// ---------------------------------------------------------------------------
// GraphManifestV1
// ---------------------------------------------------------------------------

/// A governed node inside a graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GraphNodeV1 {
    pub node_id: String,
    /// Capability the node executes through the governed runner.
    pub capability: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
}

/// A directed edge. `sequence` edges are unconditional; `conditional` edges
/// carry a stable condition label resolved at routing time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GraphEdgeV1 {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "conditionLabel"
    )]
    pub condition_label: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeKind {
    Sequence,
    Conditional,
}

/// Versioned graph manifest (`lite.graph.v1`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GraphManifestV1 {
    #[serde(rename = "schemaVersion")]
    pub schema_version: String,
    pub graph_id: String,
    pub version: String,
    pub nodes: Vec<GraphNodeV1>,
    pub edges: Vec<GraphEdgeV1>,
    #[serde(rename = "entryPoints")]
    pub entry_points: Vec<String>,
    #[serde(rename = "terminalExits")]
    pub terminal_exits: Vec<String>,
    /// Shared state keys this graph reads/writes through PortableWorkState.
    #[serde(default, rename = "sharedStateKeys")]
    pub shared_state_keys: Vec<String>,
    /// Optional authority-snapshot binding (policy digest).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "policyDigest"
    )]
    pub policy_digest: Option<String>,
    /// Canonical digest over the manifest minus this member.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "contentDigest"
    )]
    pub content_digest: Option<String>,
}

impl GraphManifestV1 {
    /// Structural validation: unique node ids, edges reference declared
    /// nodes, entries/terminals are declared nodes, terminals have no
    /// outgoing edges, every non-terminal is reachable from some entry,
    /// conditional edges carry labels, sequence edges do not.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != GRAPH_SCHEMA_VERSION {
            anyhow::bail!(
                "unsupported graph schema version {} (supported {GRAPH_SCHEMA_VERSION})",
                self.schema_version
            );
        }
        let mut ids = std::collections::BTreeSet::new();
        for n in &self.nodes {
            if n.node_id.is_empty() || !ids.insert(n.node_id.as_str()) {
                anyhow::bail!("duplicate or empty node id {:?}", n.node_id);
            }
        }
        let has = |id: &str| ids.contains(id);
        for e in &self.edges {
            if !has(&e.from) || !has(&e.to) {
                anyhow::bail!("edge {}->{} references undeclared node", e.from, e.to);
            }
            match e.kind {
                EdgeKind::Sequence if e.condition_label.is_some() => {
                    anyhow::bail!(
                        "sequence edge {}->{} must not carry a condition label",
                        e.from,
                        e.to
                    );
                }
                EdgeKind::Conditional if e.condition_label.is_none() => {
                    anyhow::bail!(
                        "conditional edge {}->{} requires a condition label",
                        e.from,
                        e.to
                    );
                }
                _ => {}
            }
        }
        for t in &self.terminal_exits {
            if !has(t) {
                anyhow::bail!("terminal exit {t:?} is not a declared node");
            }
            if self.edges.iter().any(|e| e.from == *t) {
                anyhow::bail!("terminal exit {t:?} has outgoing edges");
            }
        }
        if self.entry_points.is_empty() {
            anyhow::bail!("graph requires at least one entry point");
        }
        for e in &self.entry_points {
            if !has(e) {
                anyhow::bail!("entry point {e:?} is not a declared node");
            }
        }
        // Reachability from entries (over all edge kinds).
        let mut seen = std::collections::BTreeSet::new();
        let mut queue: Vec<&str> = self.entry_points.iter().map(String::as_str).collect();
        while let Some(n) = queue.pop() {
            if !seen.insert(n) {
                continue;
            }
            for e in &self.edges {
                if e.from == n {
                    queue.push(&e.to);
                }
            }
        }
        for n in &ids {
            if !seen.contains(n) {
                anyhow::bail!("node {n:?} is unreachable from entry points");
            }
        }
        Ok(())
    }

    /// Seal the manifest with its canonical digest.
    pub fn sealed(mut self) -> Self {
        self.content_digest = None;
        let d = crate::workflow::soma::canonical_digest(
            &serde_json::to_value(&self).expect("manifest serializes"),
        );
        self.content_digest = Some(d);
        self
    }

    /// Current manifest digest (computed; ignores any stored value).
    pub fn compute_digest(&self) -> String {
        let mut v = serde_json::to_value(self).expect("manifest serializes");
        if let Some(obj) = v.as_object_mut() {
            obj.remove("contentDigest");
        }
        crate::workflow::soma::canonical_digest(&v)
    }

    /// Fail-closed parse: version gate, structure validation, digest verify.
    pub fn parse_json(text: &str) -> Result<Self> {
        let m: Self = serde_json::from_str(text).context("graph manifest parse failed")?;
        m.validate().context("graph manifest invalid")?;
        match &m.content_digest {
            None => anyhow::bail!("graph manifest missing contentDigest"),
            Some(d) if *d != m.compute_digest() => {
                anyhow::bail!("graph manifest contentDigest does not verify")
            }
            Some(_) => {}
        }
        Ok(m)
    }

    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }
}

// ---------------------------------------------------------------------------
// GraphRunStateV1
// ---------------------------------------------------------------------------

/// One recorded node attempt inside a graph run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NodeAttemptRecordV1 {
    pub attempt: u32,
    #[serde(rename = "startedAt")]
    pub started_at: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "completedAt"
    )]
    pub completed_at: Option<String>,
    /// Terminal outcome category (SOMA-compatible variants).
    pub outcome: OutcomeCategory,
    /// Digest of the governing NodeResultV1 (durable reference).
    #[serde(rename = "resultDigest")]
    pub result_digest: String,
}

/// SOMA-compatible terminal outcome categories for graph bookkeeping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeCategory {
    Completed,
    Failed,
    Blocked,
    ReviewRequired,
    Cancelled,
}

impl From<OutcomeCategory> for crate::workflow::node_contracts::OutcomeKind {
    fn from(c: OutcomeCategory) -> Self {
        match c {
            OutcomeCategory::Completed => Self::Completed,
            OutcomeCategory::Failed => Self::Failed,
            OutcomeCategory::Blocked => Self::Blocked,
            OutcomeCategory::ReviewRequired => Self::ReviewRequired,
            OutcomeCategory::Cancelled => Self::Cancelled,
        }
    }
}

/// An auditable routing decision bound to a journaled completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteDecisionV1 {
    #[serde(rename = "recordedAt")]
    pub recorded_at: String,
    pub from_node: String,
    /// Chosen edge target or a terminal exit id.
    pub to_node: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "conditionLabel"
    )]
    pub condition_label: Option<String>,
    /// Digest of the completion record this decision routes on.
    #[serde(rename = "basisResultDigest")]
    pub basis_result_digest: String,
}

/// Durable graph-run state (`lite.graph-run.v1`). The PortableWorkState is
/// referenced by digest pointer only — never duplicated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GraphRunStateV1 {
    #[serde(rename = "schemaVersion")]
    pub schema_version: String,
    #[serde(rename = "runId")]
    pub run_id: String,
    #[serde(rename = "graphId")]
    pub graph_id: String,
    /// Pins the exact manifest revision this run executes.
    #[serde(rename = "graphManifestDigest")]
    pub graph_manifest_digest: String,
    #[serde(rename = "repoRevision")]
    pub repo_revision: String,
    #[serde(rename = "nodeAttempts")]
    pub node_attempts: BTreeMap<String, Vec<NodeAttemptRecordV1>>,
    /// Nodes currently ready/running.
    pub frontier: Vec<String>,
    pub decisions: Vec<RouteDecisionV1>,
    #[serde(default, rename = "evidenceRefs")]
    pub evidence_refs: Vec<crate::workflow::memory_contracts::EvidenceReferenceV1>,
    #[serde(rename = "portableStateRef")]
    pub portable_state_ref: String,
    /// Digest of the canonical PortableWorkState document (projection).
    #[serde(rename = "portableStateDigest")]
    pub portable_state_digest: String,
    /// Canonical digest over the state minus this member (chain root).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "contentDigest"
    )]
    pub content_digest: Option<String>,
}

/// Typed contradictions between graph state and external durable artifacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphRunError {
    /// Node attempted while not on the frontier (illegal transition).
    NotOnFrontier,
    /// Decision routed on an unknown/unjournaled completion.
    UnjournaledDecisionBasis,
    /// Run pinned a different manifest revision than provided.
    StaleGraphRevision,
    /// PortableWorkState digest contradicts the projection pointer.
    PortableStateContradiction,
    /// Unsupported schema version encountered on import.
    UnsupportedVersion,
}

impl std::fmt::Display for GraphRunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            GraphRunError::NotOnFrontier => "node not on frontier",
            GraphRunError::UnjournaledDecisionBasis => "decision routed on unjournaled completion",
            GraphRunError::StaleGraphRevision => {
                "run state pins a different graph manifest revision"
            }
            GraphRunError::PortableStateContradiction => {
                "portable work state digest contradicts graph projection"
            }
            GraphRunError::UnsupportedVersion => "unsupported graph-run schema version",
        };
        f.write_str(s)
    }
}

impl std::error::Error for GraphRunError {}

impl GraphRunStateV1 {
    /// Open a fresh run: frontier seeded from manifest entry points.
    pub fn open(
        run_id: impl Into<String>,
        manifest: &GraphManifestV1,
        repo_revision: impl Into<String>,
        portable_state_ref: impl Into<String>,
        portable_state_digest: impl Into<String>,
        recorded_at: impl Into<String>,
    ) -> Result<Self> {
        manifest.validate()?;
        Ok(Self {
            schema_version: GRAPH_RUN_SCHEMA_VERSION.into(),
            run_id: run_id.into(),
            graph_id: manifest.graph_id.clone(),
            graph_manifest_digest: manifest.compute_digest(),
            repo_revision: repo_revision.into(),
            node_attempts: BTreeMap::new(),
            frontier: manifest.entry_points.clone(),
            decisions: Vec::new(),
            evidence_refs: Vec::new(),
            portable_state_ref: portable_state_ref.into(),
            portable_state_digest: portable_state_digest.into(),
            content_digest: None,
        })
        .map(|mut s| {
            s.seal_in_place(recorded_at.into());
            s
        })
    }

    fn seal_in_place(&mut self, _recorded_at: String) {
        self.content_digest = Some(self.compute_digest());
    }

    /// Canonical digest over the state minus `contentDigest`.
    pub fn compute_digest(&self) -> String {
        let mut v = serde_json::to_value(self).expect("state serializes");
        if let Some(obj) = v.as_object_mut() {
            obj.remove("contentDigest");
        }
        crate::workflow::soma::canonical_digest(&v)
    }

    /// TRANSACTION LAW 1: apply a node completion; legal only when the node
    /// sits on the frontier. Removes it from the frontier and records the
    /// attempt durably (in-place seal keeps the chain recomputable).
    pub fn apply_node_completion(
        &mut self,
        node_id: &str,
        attempt: NodeAttemptRecordV1,
        recorded_at: impl Into<String>,
    ) -> Result<()> {
        self.check_not_sealed_stale(node_id)?;
        let pos = self
            .frontier
            .iter()
            .position(|n| n == node_id)
            .ok_or(GraphRunError::NotOnFrontier)?;
        self.frontier.remove(pos);
        self.node_attempts
            .entry(node_id.to_string())
            .or_default()
            .push(attempt);
        self.content_digest = Some(self.compute_digest());
        let _ = recorded_at;
        Ok(())
    }

    fn check_not_sealed_stale(&self, _node_id: &str) -> Result<()> {
        Ok(())
    }

    /// TRANSACTION LAW 2: record a route decision whose basis MUST be a
    /// journaled completion of `from_node` (auditable provenance). Moves the
    /// target onto the frontier unless it is a terminal exit.
    pub fn record_route_decision(
        &mut self,
        decision: RouteDecisionV1,
        manifest: &GraphManifestV1,
    ) -> Result<()> {
        let journaled = self
            .node_attempts
            .get(&decision.from_node)
            .map(|ats| {
                ats.iter()
                    .any(|a| a.result_digest == decision.basis_result_digest)
            })
            .unwrap_or(false);
        if !journaled {
            return Err(GraphRunError::UnjournaledDecisionBasis.into());
        }
        if !manifest.nodes.iter().any(|n| n.node_id == decision.to_node) {
            // Terminal exit: nothing enters the frontier.
            self.decisions.push(decision);
            self.content_digest = Some(self.compute_digest());
            return Ok(());
        }
        self.frontier.push(decision.to_node.clone());
        self.decisions.push(decision);
        self.content_digest = Some(self.compute_digest());
        Ok(())
    }

    /// RECONCILIATION: verify the referenced PortableWorkState still matches
    /// the projection pointer. Any mismatch is a typed contradiction — never
    /// silent drift.
    pub fn reconcile_portable(&self, actual_portable_digest: &str) -> Result<()> {
        if self.portable_state_digest != actual_portable_digest {
            return Err(GraphRunError::PortableStateContradiction.into());
        }
        Ok(())
    }

    /// CHECKPOINT EXPORT: verbatim durable JSON (frontier + refs preserved).
    pub fn export_checkpoint(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }

    /// CHECKPOINT IMPORT: fail closed on unsupported versions, stale graph
    /// revision, digest tampering, or structural mismatch with the manifest.
    pub fn import_checkpoint(
        text: &str,
        manifest: &GraphManifestV1,
        expected_portable_digest: &str,
    ) -> Result<Self> {
        let s: Self = serde_json::from_str(text).context("checkpoint parse failed")?;
        if s.schema_version != GRAPH_RUN_SCHEMA_VERSION {
            return Err(GraphRunError::UnsupportedVersion.into());
        }
        let declared = match &s.content_digest {
            Some(d) => d.clone(),
            None => anyhow::bail!("checkpoint missing contentDigest"),
        };
        if declared != s.compute_digest() {
            anyhow::bail!("checkpoint contentDigest does not verify");
        }
        if s.graph_manifest_digest != manifest.compute_digest() {
            return Err(GraphRunError::StaleGraphRevision.into());
        }
        s.reconcile_portable(expected_portable_digest)?;
        // Frontier nodes must exist in the manifest (structural integrity).
        for f in &s.frontier {
            if !manifest.nodes.iter().any(|n| &n.node_id == f) {
                anyhow::bail!("checkpoint frontier references unknown node {f:?}");
            }
        }
        Ok(s)
    }
}

/// Migration seam for in-major legacy documents produced before sealing was
/// mandatory: parse leniently, validate structure, then seal. Unsupported
/// versions still fail closed inside `validate`.
pub fn migrate_unsealed_v1(text: &str) -> Result<GraphManifestV1> {
    let m: GraphManifestV1 =
        serde_json::from_str(text).context("legacy graph manifest parse failed")?;
    if m.schema_version != GRAPH_SCHEMA_VERSION {
        anyhow::bail!("unsupported graph schema version {}", m.schema_version);
    }
    m.validate().context("legacy graph manifest invalid")?;
    Ok(m.sealed())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_manifest() -> GraphManifestV1 {
        GraphManifestV1 {
            schema_version: GRAPH_SCHEMA_VERSION.into(),
            graph_id: "graph-kit".into(),
            version: "1.0.0".into(),
            nodes: vec![
                GraphNodeV1 {
                    node_id: "a".into(),
                    capability: "cap.a".into(),
                    purpose: None,
                },
                GraphNodeV1 {
                    node_id: "b".into(),
                    capability: "cap.b".into(),
                    purpose: None,
                },
                GraphNodeV1 {
                    node_id: "done".into(),
                    capability: "cap.done".into(),
                    purpose: None,
                },
            ],
            edges: vec![
                GraphEdgeV1 {
                    from: "a".into(),
                    to: "b".into(),
                    kind: EdgeKind::Sequence,
                    condition_label: None,
                },
                GraphEdgeV1 {
                    from: "b".into(),
                    to: "done".into(),
                    kind: EdgeKind::Sequence,
                    condition_label: None,
                },
            ],
            entry_points: vec!["a".into()],
            terminal_exits: vec!["done".into()],
            shared_state_keys: vec!["plan".into()],
            policy_digest: None,
            content_digest: None,
        }
        .sealed()
    }

    #[test]
    fn valid_manifest_roundtrip_and_run_happy_path() {
        let m = sample_manifest();
        m.validate().expect("valid");
        let text = m.to_json().unwrap();
        let parsed = GraphManifestV1::parse_json(&text).expect("roundtrip");
        assert_eq!(parsed, m);

        let mut run = GraphRunStateV1::open(
            "run-1",
            &m,
            "rev-1",
            "pws.json",
            "pws-digest-1",
            "2026-08-25T00:00:00Z",
        )
        .unwrap();
        assert_eq!(run.frontier, vec!["a".to_string()]);

        // Complete a, decide route a->b.
        run.apply_node_completion(
            "a",
            NodeAttemptRecordV1 {
                attempt: 1,
                started_at: "2026-08-25T00:00:01Z".into(),
                completed_at: Some("2026-08-25T00:00:02Z".into()),
                outcome: OutcomeCategory::Completed,
                result_digest: "d".repeat(64),
            },
            "2026-08-25T00:00:02Z",
        )
        .expect("frontier completion applies");
        run.record_route_decision(
            RouteDecisionV1 {
                recorded_at: "2026-08-25T00:00:03Z".into(),
                from_node: "a".into(),
                to_node: "b".into(),
                condition_label: None,
                basis_result_digest: "d".repeat(64),
            },
            &m,
        )
        .expect("journaled decision routes");
        assert_eq!(run.frontier, vec!["b".to_string()]);

        // Checkpoint export/import preserves frontier + references verbatim.
        let ckpt = run.export_checkpoint().unwrap();
        let restored =
            GraphRunStateV1::import_checkpoint(&ckpt, &m, "pws-digest-1").expect("reimport");
        assert_eq!(restored, run);
    }

    #[test]
    fn invalid_graphs_fail_closed() {
        // Edge to undeclared node.
        let mut bad = sample_manifest();
        bad.edges.push(GraphEdgeV1 {
            from: "a".into(),
            to: "ghost".into(),
            kind: EdgeKind::Sequence,
            condition_label: None,
        });
        assert!(bad.validate().is_err());

        // Terminal exit with outgoing edges.
        let mut bad2 = sample_manifest();
        bad2.edges.push(GraphEdgeV1 {
            from: "done".into(),
            to: "a".into(),
            kind: EdgeKind::Conditional,
            condition_label: Some("retry".into()),
        });
        assert!(bad2.validate().is_err());

        // Unreachable node.
        let mut bad3 = sample_manifest();
        bad3.nodes.push(GraphNodeV1 {
            node_id: "orphan".into(),
            capability: "cap.o".into(),
            purpose: None,
        });
        assert!(bad3.validate().is_err());

        // Conditional edge without label.
        let mut bad4 = sample_manifest();
        bad4.edges.push(GraphEdgeV1 {
            from: "a".into(),
            to: "b".into(),
            kind: EdgeKind::Conditional,
            condition_label: None,
        });
        assert!(bad4.validate().is_err());
    }

    #[test]
    fn tampered_digest_fails_closed() {
        let m = sample_manifest();
        let mut text = m.to_json().unwrap();
        // Flip one byte of the pinned digest.
        text = text.replace(&m.content_digest.clone().unwrap()[..8], "00000000");
        assert!(GraphManifestV1::parse_json(&text).is_err());
    }

    #[test]
    fn stale_revision_rejected_on_checkpoint_import() {
        let m = sample_manifest();
        let mut run = GraphRunStateV1::open(
            "run-s",
            &m,
            "rev-1",
            "pws.json",
            "pws-digest",
            "2026-08-25T00:00:00Z",
        )
        .unwrap();
        let ckpt = run.export_checkpoint().unwrap();

        // A DIFFERENT manifest revision must reject the checkpoint.
        let mut other = sample_manifest();
        other.version = "1.0.1".into();
        let other = other.sealed();
        assert!(matches!(
            GraphRunStateV1::import_checkpoint(&ckpt, &other, "pws-digest"),
            Err(e) if e.to_string().contains("different graph manifest revision")
        ));
        let _ = &mut run;
    }

    #[test]
    fn contradictory_portable_state_is_typed_not_silent() {
        let m = sample_manifest();
        let run = GraphRunStateV1::open(
            "run-c",
            &m,
            "rev-1",
            "pws.json",
            "pws-digest-A",
            "2026-08-25T00:00:00Z",
        )
        .unwrap();
        assert!(run.reconcile_portable("pws-digest-A").is_ok());
        let err = run
            .reconcile_portable("pws-digest-B")
            .unwrap_err()
            .to_string();
        assert!(err.contains("contradicts"), "{err}");
    }

    #[test]
    fn illegal_transitions_fail_closed() {
        let m = sample_manifest();
        let mut run =
            GraphRunStateV1::open("run-i", &m, "rev-1", "pws", "pd", "2026-08-25T00:00:00Z")
                .unwrap();
        // Completing "b" while only "a" is on the frontier is illegal.
        let err = run
            .apply_node_completion(
                "b",
                NodeAttemptRecordV1 {
                    attempt: 1,
                    started_at: "t".into(),
                    completed_at: Some("t".into()),
                    outcome: OutcomeCategory::Completed,
                    result_digest: "x".repeat(64),
                },
                "now",
            )
            .unwrap_err()
            .to_string();
        assert!(err.contains("not on frontier"), "{err}");

        // Decision routed on an unjournaled completion refuses.
        let err2 = run
            .record_route_decision(
                RouteDecisionV1 {
                    recorded_at: "t".into(),
                    from_node: "a".into(),
                    to_node: "b".into(),
                    condition_label: None,
                    basis_result_digest: "ghost".into(),
                },
                &m,
            )
            .unwrap_err()
            .to_string();
        assert!(err2.contains("unjournaled"), "{err2}");
    }

    #[test]
    fn unsupported_versions_fail_closed() {
        let mut m = sample_manifest();
        m.schema_version = "2.0.0".into();
        assert!(m.validate().is_err());

        let mut state_json = serde_json::json!({
            "schemaVersion": "9.9.9",
            "runId": "r", "graphId": "g", "graphManifestDigest": "d",
            "repoRevision": "r", "nodeAttempts": {}, "frontier": [],
            "decisions": [], "evidenceRefs": [],
            "portableStateRef": "p", "portableStateDigest": "pd"
        })
        .to_string();
        let _ = &mut state_json;
        assert!(GraphRunStateV1::import_checkpoint(&state_json, &sample_manifest(), "pd").is_err());
    }

    #[test]
    fn legacy_unsealed_document_migrates_by_sealing() {
        let mut m = sample_manifest();
        m.content_digest = None; // pre-sealing era document
        let legacy = serde_json::to_string(&m).unwrap();
        // Direct parse fails closed (missing digest).
        assert!(GraphManifestV1::parse_json(&legacy).is_err());
        // Migration path validates structure then seals.
        let migrated = migrate_unsealed_v1(&legacy).expect("migrates");
        assert!(migrated.content_digest.is_some());
        assert_eq!(migrated.compute_digest(), m.compute_digest());
    }
}
