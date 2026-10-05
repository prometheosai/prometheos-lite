//! Private machinery for the graph projection (E4/X07 Slice 2): the
//! document-wide node registry, the §4 payload/node/edge view structs,
//! and the §8 digest preimages/bottom-up driver. The public projector
//! (`project_graph_json`) lands in Task 4 on top of these pieces; this
//! module deliberately emits no payload and changes no conformance test.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::workflow::execution_graph::topological_order_body;
use crate::workflow::soma::Diagnostic;
use crate::workflow::soma::contracts::{BodyItem, PortDefinition, WorkflowDefinition};
use crate::workflow::soma::types::{Cardinality, OutcomeVariant, Requiredness};

use super::digest_of;

/// Lite-owned graph schema version (§4; §8.3 closed allow-list value).
pub const GRAPH_SCHEMA_VERSION: &str = "lite.graph-projection.v1";

/// One node's registration record (§2.4/§4).
#[derive(Debug)]
pub struct RegistryNode<'a> {
    /// Rendered node id `s{NNNN}:{item id}` — zero-based 4-digit
    /// topological index within the containing scope.
    pub node_id: String,
    /// The AST item this node renders.
    pub item: &'a BodyItem,
    /// Rendered ids of enclosing containers, outermost first (§6 rule 4).
    pub ancestors: Vec<String>,
    /// Declaration index within the scope (used for scope pointers).
    pub decl_index: usize,
}

impl RegistryNode<'_> {
    pub fn is_composite(&self) -> bool {
        matches!(self.item, BodyItem::Composite(_))
    }
}

/// All nodes of one scope, in scope-topological order (R8/R9).
#[derive(Debug)]
pub struct ScopeNodes<'a> {
    /// RFC 6901 pointer of this scope's body: `/body`, `/body/1/body`, …
    pub ptr: String,
    pub nodes: Vec<RegistryNode<'a>>,
}

/// Document-wide node registry: every scope (root first, then pre-order
/// DFS through containers) plus a rendered-id index. Duplicate body-item
/// ids anywhere in the document — including equality with an ancestor —
/// and scope cycles fail closed with `PROJ-0001` (§2.4 resolution rules).
#[derive(Debug)]
pub struct GraphRegistry<'a> {
    pub scopes: Vec<ScopeNodes<'a>>,
    index: BTreeMap<String, (usize, usize)>,
}

impl<'a> GraphRegistry<'a> {
    pub fn build(root: &'a WorkflowDefinition) -> Result<GraphRegistry<'a>, Vec<Diagnostic>> {
        let mut registry = GraphRegistry {
            scopes: Vec::new(),
            index: BTreeMap::new(),
        };
        let mut seen_ids: BTreeSet<String> = BTreeSet::new();
        registry.walk(&root.body, "/body", &[], &mut seen_ids)?;
        Ok(registry)
    }

    /// Look up a node by its rendered id (unknown id ⇒ `None`, §6 rule 1).
    pub fn entry(&self, node_id: &str) -> Option<&RegistryNode<'a>> {
        let &(scope, idx) = self.index.get(node_id)?;
        self.scopes.get(scope).and_then(|s| s.nodes.get(idx))
    }

    /// Composite node ids strictly descendants-first: deepest scope level
    /// first, ties by document order — the deterministic bottom-up order
    /// for §8.1 `childSubgraphDigest` computation.
    pub fn composite_postorder(&self) -> Vec<String> {
        let mut composites: Vec<(usize, usize, String)> = Vec::new();
        let mut seq = 0usize;
        for scope in &self.scopes {
            for node in &scope.nodes {
                if node.is_composite() {
                    composites.push((node.ancestors.len(), seq, node.node_id.clone()));
                }
                seq += 1;
            }
        }
        composites.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        composites.into_iter().map(|(_, _, id)| id).collect()
    }

    fn walk(
        &mut self,
        items: &'a [BodyItem],
        ptr: &str,
        ancestors: &[String],
        seen_ids: &mut BTreeSet<String>,
    ) -> Result<(), Vec<Diagnostic>> {
        let Some(order) = topological_order_body(items) else {
            return Err(vec![Diagnostic::new(
                "PROJ-0001",
                format!("scope {ptr} has a dataflow cycle; no topological order"),
            )]);
        };
        let scope_idx = self.scopes.len();
        self.scopes.push(ScopeNodes {
            ptr: ptr.to_string(),
            nodes: Vec::new(),
        });
        for (pos, &decl) in order.iter().enumerate() {
            let item = &items[decl];
            if !seen_ids.insert(item.id().to_string()) {
                return Err(vec![Diagnostic::new(
                    "PROJ-0001",
                    format!(
                        "duplicate body-item id {:?} in document (at {ptr}/{decl})",
                        item.id()
                    ),
                )]);
            }
            let node_id = format!("s{pos:04}:{}", item.id());
            if self.index.contains_key(&node_id) {
                return Err(vec![Diagnostic::new(
                    "PROJ-0001",
                    format!("duplicate node id {node_id}"),
                )]);
            }
            self.index.insert(
                node_id.clone(),
                (scope_idx, self.scopes[scope_idx].nodes.len()),
            );
            self.scopes[scope_idx].nodes.push(RegistryNode {
                node_id,
                item,
                ancestors: ancestors.to_vec(),
                decl_index: decl,
            });
        }
        // Recurse into containers in scope-topological order (pre-order DFS).
        let children: Vec<(String, String, &'a [BodyItem])> = self.scopes[scope_idx]
            .nodes
            .iter()
            .filter_map(|node| {
                let item: &'a BodyItem = node.item;
                match item {
                    BodyItem::Composite(c) => Some((
                        node.node_id.clone(),
                        format!("{}/{}/body", ptr, node.decl_index),
                        c.body.as_slice(),
                    )),
                    BodyItem::Operation(_) => None,
                }
            })
            .collect();
        for (node_id, child_ptr, child_items) in children {
            let mut child_ancestors = ancestors.to_vec();
            child_ancestors.push(node_id);
            self.walk(child_items, &child_ptr, &child_ancestors, seen_ids)?;
        }
        Ok(())
    }
}

/// Payload root (§4); assembled and enveloped by `project_graph_json`
/// (Task 4) — defined here so the payload shape lives with the views.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphPayload {
    pub graph_schema_version: String,
    pub policy_digest: String,
    pub input_ports: Vec<GraphPortView>,
    pub output_ports: Vec<GraphPortView>,
    pub nodes: Vec<GraphNodeView>,
    pub edges: Vec<GraphEdgeView>,
}

/// Root/container boundary port (§4): no `direction`, no `dataContract`
/// (free-form attachments are deliberately not graph topology).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphPortView {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub cardinality: Cardinality,
    pub requiredness: Requiredness,
}

/// Atomic-node input (§4): typed value + accepted outcome variants.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphOpInputView {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(rename = "acceptedOutcomes")]
    pub accepted_outcomes: Vec<OutcomeVariant>,
}

/// Atomic-node output; `emits` absent iff the source declares none (§4).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphOpOutputView {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emits: Option<Vec<OutcomeVariant>>,
}

/// Withheld-boundary disclosure block (§4). `hiddenNodes`/`hiddenEdges`
/// appear only under count authorization (Task 4 decides presence).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphDisclosureView {
    pub withheld: bool,
    pub reason: String,
    pub category: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hidden_nodes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hidden_edges: Option<usize>,
}

/// Edge class (§7.2): `outcome` labels are outcome variants;
/// `boundary-value` labels are the shared value type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GraphEdgeKind {
    Outcome,
    BoundaryValue,
}

/// Edge endpoint: a rendered node id plus one of its port names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEndpoint {
    pub node: String,
    pub port: String,
}

/// One rendered edge (§4): `label` is never empty on a rendered edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdgeView {
    pub kind: GraphEdgeKind,
    pub from: GraphEndpoint,
    pub to: GraphEndpoint,
    pub label: Vec<String>,
}

/// A body item's node view (§4), `kind`-discriminated. `children`/
/// `internalEdges` are present iff revealed and `disclosure` iff
/// withheld — Task 4 attaches exactly one state; this renderer leaves
/// all three `None`. Snake→camel field names are explicit so the
/// intent does not depend on enum-level `rename_all` semantics.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum GraphNodeView {
    Atomic {
        id: String,
        inputs: Vec<GraphOpInputView>,
        outputs: Vec<GraphOpOutputView>,
    },
    Composite {
        id: String,
        inputs: Vec<GraphPortView>,
        outputs: Vec<GraphPortView>,
        #[serde(rename = "childSubgraphDigest")]
        child_subgraph_digest: String,
        #[serde(skip_serializing_if = "Option::is_none", rename = "children")]
        children: Option<Vec<GraphNodeView>>,
        #[serde(skip_serializing_if = "Option::is_none", rename = "internalEdges")]
        internal_edges: Option<Vec<GraphEdgeView>>,
        #[serde(skip_serializing_if = "Option::is_none", rename = "disclosure")]
        disclosure: Option<GraphDisclosureView>,
    },
}

/// Convert a declared boundary port (root or container) to its graph
/// view (§4): name/type/cardinality/requiredness only.
pub fn port_view(port: &PortDefinition) -> GraphPortView {
    GraphPortView {
        name: port.name.clone(),
        ty: port.ty.clone(),
        // `Cardinality`/`Requiredness` are `Copy` (string_enum derives
        // `Copy`): move them — `.clone()` here would fail
        // `clippy::clone_on_copy` under the `-D warnings` gate.
        cardinality: port.cardinality,
        requiredness: port.requiredness,
    }
}

/// Render one body item as its node view WITHOUT `children`,
/// `internalEdges`, or `disclosure` (Task 4 attaches scope children,
/// edges, and the disclosure state). For a composite the
/// `childSubgraphDigest` must already exist in `child_digests`
/// (computed bottom-up, §8.1); a miss fails closed with `PROJ-0001`.
pub fn render_node(
    item: &BodyItem,
    node_id: &str,
    child_digests: &BTreeMap<String, String>,
) -> Result<GraphNodeView, Vec<Diagnostic>> {
    match item {
        BodyItem::Operation(op) => Ok(GraphNodeView::Atomic {
            id: node_id.to_string(),
            inputs: op
                .inputs
                .iter()
                .map(|i| GraphOpInputView {
                    name: i.name.clone(),
                    ty: i.ty.clone(),
                    accepted_outcomes: i.accepted_outcomes.clone(),
                })
                .collect(),
            outputs: op
                .outputs
                .iter()
                .map(|o| GraphOpOutputView {
                    name: o.name.clone(),
                    ty: o.ty.clone(),
                    emits: o.emits.clone(),
                })
                .collect(),
        }),
        BodyItem::Composite(c) => {
            let child_subgraph_digest = child_digests.get(node_id).cloned().ok_or_else(|| {
                vec![Diagnostic::new(
                    "PROJ-0001",
                    format!("missing childSubgraphDigest for {node_id}"),
                )]
            })?;
            Ok(GraphNodeView::Composite {
                id: node_id.to_string(),
                inputs: c.input_ports.iter().map(port_view).collect(),
                outputs: c.output_ports.iter().map(port_view).collect(),
                child_subgraph_digest,
                children: None,
                internal_edges: None,
                disclosure: None,
            })
        }
    }
}

/// §8.1 preimage: the boundary's subtree in unwithheld form — the
/// subject's own `childSubgraphDigest` field omitted by the builder,
/// descendants' digests embedded as final values (bottom-up).
pub fn child_subgraph_digest_preimage(
    root_source_digest: &str,
    boundary_id: &str,
    content: &serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "domain": "projection.graph.child-subgraph.v1",
        "graphSchemaVersion": GRAPH_SCHEMA_VERSION,
        "rootSourceDigest": root_source_digest,
        "boundaryId": boundary_id,
        "content": content.clone(),
    })
}

/// Compute every boundary's `childSubgraphDigest` bottom-up (§8.1).
///
/// `postorder` lists composite node ids strictly descendants-first
/// (`GraphRegistry::composite_postorder`). `build_content` returns the
/// boundary's unwithheld subtree content: it MUST omit the subject's
/// own `childSubgraphDigest` field (asserted by in-module tests) and
/// MUST embed already-computed descendant digests from the supplied
/// map. Preimages are string-valued, so canonicalization cannot fail.
pub fn compute_child_subgraph_digests(
    root_source_digest: &str,
    postorder: &[String],
    mut build_content: impl FnMut(&str, &BTreeMap<String, String>) -> serde_json::Value,
) -> BTreeMap<String, String> {
    let mut done: BTreeMap<String, String> = BTreeMap::new();
    for boundary_id in postorder {
        let content = build_content(boundary_id, &done);
        let preimage = child_subgraph_digest_preimage(root_source_digest, boundary_id, &content);
        let digest = digest_of(&preimage)
            .expect("graph digest preimage is string-only and always canonicalizable");
        done.insert(boundary_id.clone(), digest);
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    const NESTED: &str = include_str!("../../../tests/fixtures/slice2/wf-nested.json");

    fn wf() -> WorkflowDefinition {
        serde_json::from_str(NESTED).expect("wf-nested parses")
    }

    #[test]
    fn registry_walks_scopes_in_topological_preorder() {
        let wf = wf();
        let registry = GraphRegistry::build(&wf).expect("fixture registry builds");
        let root_ids: Vec<&str> = registry.scopes[0]
            .nodes
            .iter()
            .map(|n| n.node_id.as_str())
            .collect();
        assert_eq!(root_ids, ["s0000:prep", "s0001:flow", "s0002:audit"]);

        let flow_body = &registry.scopes[1];
        assert_eq!(flow_body.ptr, "/body/1/body");
        // Per-scope topological index restarts inside each container:
        // flow's body scope holds `inner` at position 0 → s0000:inner.
        assert_eq!(flow_body.nodes[0].node_id, "s0000:inner");
        assert_eq!(flow_body.nodes[0].item.id(), "inner");
        assert_eq!(registry.scopes[2].ptr, "/body/1/body/0/body");
        let leaf = &registry.scopes[2].nodes[0];
        assert_eq!(leaf.node_id, "s0000:leaf");
        assert_eq!(leaf.ancestors, ["s0001:flow", "s0000:inner"]);

        let flow = registry.entry("s0001:flow").expect("flow registered");
        assert!(flow.is_composite());
        let prep = registry.entry("s0000:prep").expect("prep registered");
        assert!(!prep.is_composite());
        assert!(registry.entry("s9999:ghost").is_none());
        assert_eq!(
            registry.composite_postorder(),
            ["s0000:inner", "s0001:flow"],
            "descendants-first order drives §8.1"
        );
    }

    #[test]
    fn registry_rejects_duplicate_ids_document_wide() {
        let mut wf = wf();
        let BodyItem::Operation(op) = &mut wf.body[0] else {
            panic!("prep is an operation");
        };
        op.id = "audit".to_string(); // collides with root body[2]
        let err = GraphRegistry::build(&wf).expect_err("duplicate id refused");
        assert_eq!(err[0].code, "PROJ-0001");
        assert!(
            err[0].message.contains("duplicate body-item id"),
            "unexpected message: {}",
            err[0].message
        );
    }

    #[test]
    fn registry_refuses_scope_cycles_defensively() {
        let mut wf = wf();
        let BodyItem::Operation(prep) = &mut wf.body[0] else {
            panic!("prep is an operation");
        };
        // prep now consumes what `flow` outputs while `flow` consumes
        // what `prep` outputs: a root-scope cycle (gate: SOMA-EXP-0002;
        // registry defense: PROJ-0001).
        prep.inputs[0].name = "ready".to_string();
        let err = GraphRegistry::build(&wf).expect_err("cycle refused");
        assert_eq!(err[0].code, "PROJ-0001");
        assert!(
            err[0].message.contains("dataflow cycle"),
            "unexpected: {}",
            err[0].message
        );
    }

    #[test]
    fn render_node_maps_atomic_and_composite_shapes() {
        let wf = wf();
        let registry = GraphRegistry::build(&wf).expect("registry");
        let digests = BTreeMap::from([("s0001:flow".to_string(), "d".repeat(64))]);

        let prep = registry.entry("s0000:prep").expect("prep");
        let node = render_node(prep.item, &prep.node_id, &digests).expect("renders");
        assert_eq!(
            serde_json::to_value(&node).expect("serializes"),
            serde_json::json!({
                "kind": "atomic",
                "id": "s0000:prep",
                "inputs": [
                    { "name": "order", "type": "Order", "acceptedOutcomes": ["Produced"] }
                ],
                "outputs": [
                    { "name": "staged", "type": "Order", "emits": ["Produced"] }
                ]
            })
        );

        let flow = registry.entry("s0001:flow").expect("flow");
        let node = render_node(flow.item, &flow.node_id, &digests).expect("renders");
        let v = serde_json::to_value(&node).expect("serializes");
        assert_eq!(v["kind"], "composite");
        assert_eq!(v["childSubgraphDigest"], "d".repeat(64));
        assert_eq!(
            v["inputs"],
            serde_json::json!([
                { "name": "staged", "type": "Order",
                  "cardinality": "single", "requiredness": "required" }
            ])
        );
        assert!(v.get("children").is_none(), "renderer attaches no state");
        assert!(v.get("internal_edges").is_none());
        assert!(v.get("disclosure").is_none());

        let err = render_node(flow.item, &flow.node_id, &BTreeMap::new())
            .expect_err("missing digest fails closed");
        assert_eq!(err[0].code, "PROJ-0001");
    }

    #[test]
    fn child_digests_are_deterministic_transitive_and_acyclic() {
        let postorder = vec!["s0000:inner".to_string(), "s0001:flow".to_string()];
        let build = |tweak: bool| {
            compute_child_subgraph_digests("a".repeat(64).as_str(), &postorder, |id, done| {
                assert!(
                    !done.contains_key(id),
                    "a boundary's own digest must never feed its own preimage"
                );
                serde_json::json!({
                    "id": id,
                    "descendants": done.clone(),
                    "leaf": if tweak && id == "s0000:inner" { "tweaked" } else { "pristine" },
                })
            })
        };

        let one = build(false);
        let twice = build(false);
        assert_eq!(one, twice, "digest construction must be deterministic");
        assert_eq!(one.len(), 2);
        assert_eq!(one["s0001:flow"].len(), 64, "lowercase 64-hex digest");

        let tweaked = build(true);
        assert_ne!(one["s0000:inner"], tweaked["s0000:inner"]);
        assert_ne!(
            one["s0001:flow"], tweaked["s0001:flow"],
            "a descendant change must move every ancestor digest (transitive binding)"
        );
    }
}
