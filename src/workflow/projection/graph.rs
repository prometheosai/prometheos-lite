//! Machinery for the graph projection (E4/X07 Slice 2): the document-wide
//! node registry, the §4 payload/node/edge view structs, the §8 digest
//! preimages/bottom-up driver, and the disclosure-aware payload renderer
//! that assembles `project_graph_json` envelopes (`GraphPayload` plus the
//! disclosure state per boundary). Verification helpers
//! (`verify_graph_projection_bytes`, `verify_graph_against_source`) live
//! here as well.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::workflow::execution_graph::topological_order_body;
use crate::workflow::soma::Diagnostic;
use crate::workflow::soma::contracts::{BodyItem, PortDefinition, WorkflowDefinition};
use crate::workflow::soma::types::{Cardinality, OutcomeVariant, Requiredness};

use super::disclosure::{GraphDisclosurePolicy, normalize_policy, policy_digest_preimage};
use super::envelope::{parse_envelope_bytes, verify_envelope_metadata};
use super::{
    PROJECTION_VERSION_V1, VersionedProjectionEnvelope, digest_of, source_digest_of,
    validated_source,
};

/// Lite-owned graph schema version (§4; §8.3 closed allow-list value).
pub const GRAPH_SCHEMA_VERSION: &str = "lite.graph-projection.v1";

/// Closed allow-list of graph schema versions (§8.3). Fail closed on
/// anything else — unknown/absent/mistyped ⇒ `SOMA-CMP-0001`.
pub const ALLOWED_GRAPH_SCHEMA_VERSIONS: [&str; 1] = [GRAPH_SCHEMA_VERSION];

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
    /// Rendered id of the composite node this scope is the body of
    /// (`None` at the root scope); R1/R2 boundary edges terminate on it.
    pub parent_node: Option<String>,
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
        registry.walk(&root.body, "/body", &[], None, &mut seen_ids)?;
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

    /// Scope index of the child body for the composite node recorded at
    /// `(scope_idx, node_idx)` — the child pointer is
    /// `{scope.ptr}/{decl_index}/body` (declared, not topological).
    pub fn child_scope(&self, scope_idx: usize, node_idx: usize) -> Option<usize> {
        let scope = self.scopes.get(scope_idx)?;
        let node = scope.nodes.get(node_idx)?;
        let expected = format!("{}/{}/body", scope.ptr, node.decl_index);
        self.scopes.iter().position(|s| s.ptr == expected)
    }

    fn walk(
        &mut self,
        items: &'a [BodyItem],
        ptr: &str,
        ancestors: &[String],
        parent_node: Option<String>,
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
            parent_node: parent_node.clone(),
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
            child_ancestors.push(node_id.clone());
            self.walk(
                child_items,
                &child_ptr,
                &child_ancestors,
                Some(node_id),
                seen_ids,
            )?;
        }
        Ok(())
    }
}

/// Payload root (§4); assembled and enveloped by `project_graph_json`
/// — defined here so the payload shape lives with the views.
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
/// appear only under count authorization (presence is decided by the
/// disclosure state at payload-render time).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
/// withheld — the payload renderer attaches exactly one state; this
/// builder leaves all three `None`. Snake→camel field names are explicit
/// so the intent does not depend on enum-level `rename_all` semantics.
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
/// `internalEdges`, or `disclosure` (the payload renderer attaches scope
/// children, edges, and the disclosure state). For a composite the
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

/// One declared supply/consumer port record inside a scope.
struct ScopePort {
    node: String,
    name: String,
    ty: String,
    /// `true` when the port belongs to a composite's boundary.
    composite: bool,
    /// Operation outputs only (`None` on composite ports and consumers).
    emits: Option<Vec<OutcomeVariant>>,
    /// Operation inputs only (`None` on composite ports and producers).
    accepted: Option<Vec<OutcomeVariant>>,
}

fn duplicate_names<'a>(names: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut seen: BTreeSet<&'a str> = BTreeSet::new();
    for name in names {
        if !seen.insert(name) {
            return Some(name.to_string());
        }
    }
    None
}

/// The rendered edge set for one scope (§7): outcome edges between
/// operation ports, `boundary-value` edges for everything touching a
/// boundary port, R1 supply from the container's input ports (nested
/// scopes only — root supply is ambient), R2 single-producer pass-through
/// into the container's output ports (nested scopes only), R6 free-input
/// re-check, §7.4 label/ambiguity fail-closed rules, R9 sort. Endpoints
/// outside this scope's nodes (or its container) are cross-boundary
/// references ⇒ `PROJ-0001`.
pub fn scope_edges(
    registry: &GraphRegistry<'_>,
    root: &WorkflowDefinition,
    scope_idx: usize,
) -> Result<Vec<GraphEdgeView>, Vec<Diagnostic>> {
    let scope = registry.scopes.get(scope_idx).ok_or_else(|| {
        vec![Diagnostic::new(
            "PROJ-0001",
            format!("no such scope index {scope_idx}"),
        )]
    })?;

    // This scope's boundary: the root ports at scope 0, else the
    // container's declared ports (the parent node of this scope).
    let (boundary_inputs, boundary_outputs, container): (
        &[PortDefinition],
        &[PortDefinition],
        Option<&str>,
    ) = if scope_idx == 0 {
        (
            root.input_ports.as_slice(),
            root.output_ports.as_slice(),
            None,
        )
    } else {
        let parent = scope.parent_node.as_deref().ok_or_else(|| {
            vec![Diagnostic::new(
                "PROJ-0001",
                format!("scope {} has no container node", scope.ptr),
            )]
        })?;
        let entry = registry.entry(parent).ok_or_else(|| {
            vec![Diagnostic::new(
                "PROJ-0001",
                format!("unknown container node {parent}"),
            )]
        })?;
        let BodyItem::Composite(c) = entry.item else {
            return Err(vec![Diagnostic::new(
                "PROJ-0001",
                format!("scope {} is not inside a composite", scope.ptr),
            )]);
        };
        (
            c.input_ports.as_slice(),
            c.output_ports.as_slice(),
            Some(parent),
        )
    };

    // §7.4: duplicate boundary port names within this scope's boundary.
    if let Some(name) = duplicate_names(boundary_inputs.iter().map(|p| p.name.as_str())) {
        return Err(vec![Diagnostic::new(
            "PROJ-0001",
            format!(
                "duplicate boundary input port {name} in scope {}",
                scope.ptr
            ),
        )]);
    }
    if let Some(name) = duplicate_names(boundary_outputs.iter().map(|p| p.name.as_str())) {
        return Err(vec![Diagnostic::new(
            "PROJ-0001",
            format!(
                "duplicate boundary output port {name} in scope {}",
                scope.ptr
            ),
        )]);
    }

    let mut producers: Vec<ScopePort> = Vec::new();
    let mut consumers: Vec<ScopePort> = Vec::new();
    for node in &scope.nodes {
        match node.item {
            BodyItem::Operation(op) => {
                // §7.4: duplicate port names within one operation.
                if let Some(name) = duplicate_names(op.inputs.iter().map(|i| i.name.as_str())) {
                    return Err(vec![Diagnostic::new(
                        "PROJ-0001",
                        format!("duplicate input port {name} on operation {}", node.node_id),
                    )]);
                }
                if let Some(name) = duplicate_names(op.outputs.iter().map(|o| o.name.as_str())) {
                    return Err(vec![Diagnostic::new(
                        "PROJ-0001",
                        format!("duplicate output port {name} on operation {}", node.node_id),
                    )]);
                }
                for input in &op.inputs {
                    consumers.push(ScopePort {
                        node: node.node_id.clone(),
                        name: input.name.clone(),
                        ty: input.ty.clone(),
                        composite: false,
                        emits: None,
                        accepted: Some(input.accepted_outcomes.clone()),
                    });
                }
                for output in &op.outputs {
                    producers.push(ScopePort {
                        node: node.node_id.clone(),
                        name: output.name.clone(),
                        ty: output.ty.clone(),
                        composite: false,
                        emits: output.emits.clone(),
                        accepted: None,
                    });
                }
            }
            BodyItem::Composite(c) => {
                for port in &c.input_ports {
                    consumers.push(ScopePort {
                        node: node.node_id.clone(),
                        name: port.name.clone(),
                        ty: port.ty.clone(),
                        composite: true,
                        emits: None,
                        accepted: None,
                    });
                }
                for port in &c.output_ports {
                    producers.push(ScopePort {
                        node: node.node_id.clone(),
                        name: port.name.clone(),
                        ty: port.ty.clone(),
                        composite: true,
                        emits: None,
                        accepted: None,
                    });
                }
            }
        }
    }

    let mut edges: Vec<GraphEdgeView> = Vec::new();
    for cons in &consumers {
        let matching: Vec<&ScopePort> = producers.iter().filter(|p| p.name == cons.name).collect();
        if matching.is_empty() {
            // R6/R1: no internal producer ⇒ boundary supply only.
            let Some(bp) = boundary_inputs.iter().find(|p| p.name == cons.name) else {
                return Err(vec![Diagnostic::new(
                    "PROJ-0001",
                    format!(
                        "input {} on {} has no producer and no boundary port \
                         (defensive re-check of SOMA-EXP-0005)",
                        cons.name, cons.node
                    ),
                )]);
            };
            if bp.ty != cons.ty {
                return Err(vec![Diagnostic::new(
                    "PROJ-0001",
                    format!(
                        "incompatible boundary supply for {}: boundary {} vs input {}",
                        cons.name, bp.ty, cons.ty
                    ),
                )]);
            }
            if let Some(parent) = container {
                // R1 (nested): rendered from the container's own port;
                // at root the supply is ambient (§7.1) ⇒ no edge.
                edges.push(GraphEdgeView {
                    kind: GraphEdgeKind::BoundaryValue,
                    from: GraphEndpoint {
                        node: parent.to_string(),
                        port: bp.name.clone(),
                    },
                    to: GraphEndpoint {
                        node: cons.node.clone(),
                        port: cons.name.clone(),
                    },
                    label: vec![bp.ty.clone()],
                });
            }
            continue;
        }
        for prod in matching {
            if prod.composite || cons.composite {
                // §7.2: anything touching a boundary port is a value-type
                // boundary-value edge; composite ports declare no
                // outcomes, so labels are shared value types only.
                if prod.ty != cons.ty {
                    return Err(vec![Diagnostic::new(
                        "PROJ-0001",
                        format!(
                            "incompatible boundary supply for {}: producer {} vs input {}",
                            cons.name, prod.ty, cons.ty
                        ),
                    )]);
                }
                edges.push(GraphEdgeView {
                    kind: GraphEdgeKind::BoundaryValue,
                    from: GraphEndpoint {
                        node: prod.node.clone(),
                        port: prod.name.clone(),
                    },
                    to: GraphEndpoint {
                        node: cons.node.clone(),
                        port: cons.name.clone(),
                    },
                    label: vec![cons.ty.clone()],
                });
            } else {
                // §7.4: label = sorted emits ∩ acceptedOutcomes; a
                // matched connection without label data fails closed.
                let emits = prod.emits.as_deref().unwrap_or(&[]);
                let accepted = cons.accepted.as_deref().unwrap_or(&[]);
                let mut label: Vec<String> = emits
                    .iter()
                    .filter(|v| accepted.contains(v))
                    .map(|v| v.to_string())
                    .collect();
                label.sort();
                label.dedup();
                if label.is_empty() {
                    return Err(vec![Diagnostic::new(
                        "PROJ-0001",
                        format!(
                            "matched connection {}:{} -> {}:{} has no derivable outcome label",
                            prod.node, prod.name, cons.node, cons.name
                        ),
                    )]);
                }
                edges.push(GraphEdgeView {
                    kind: GraphEdgeKind::Outcome,
                    from: GraphEndpoint {
                        node: prod.node.clone(),
                        port: prod.name.clone(),
                    },
                    to: GraphEndpoint {
                        node: cons.node.clone(),
                        port: cons.name.clone(),
                    },
                    label,
                });
            }
        }
    }

    // R2 (nested only): every declared container output port is fed by
    // exactly one internal producer of that name and type.
    if let Some(parent) = container {
        for bp in boundary_outputs {
            let matching: Vec<&ScopePort> =
                producers.iter().filter(|p| p.name == bp.name).collect();
            if matching.is_empty() {
                return Err(vec![Diagnostic::new(
                    "PROJ-0001",
                    format!("absent producer for declared boundary output {}", bp.name),
                )]);
            }
            if matching.len() > 1 {
                return Err(vec![Diagnostic::new(
                    "PROJ-0001",
                    format!("ambiguous boundary output {}", bp.name),
                )]);
            }
            let prod = matching[0];
            if prod.ty != bp.ty {
                return Err(vec![Diagnostic::new(
                    "PROJ-0001",
                    format!(
                        "incompatible boundary supply for {}: producer {} vs port {}",
                        bp.name, prod.ty, bp.ty
                    ),
                )]);
            }
            edges.push(GraphEdgeView {
                kind: GraphEdgeKind::BoundaryValue,
                from: GraphEndpoint {
                    node: prod.node.clone(),
                    port: prod.name.clone(),
                },
                to: GraphEndpoint {
                    node: parent.to_string(),
                    port: bp.name.clone(),
                },
                label: vec![bp.ty.clone()],
            });
        }
    }

    // R3/cross-boundary defense: endpoints must be this scope's nodes or
    // the container node itself — never a hidden child (§7.3 last bullet).
    let scope_ids: BTreeSet<&str> = scope.nodes.iter().map(|n| n.node_id.as_str()).collect();
    for edge in &edges {
        for endpoint in [&edge.from, &edge.to] {
            let known = scope_ids.contains(endpoint.node.as_str())
                || Some(endpoint.node.as_str()) == container;
            if !known {
                return Err(vec![Diagnostic::new(
                    "PROJ-0001",
                    format!("edge endpoint {} crosses a scope boundary", endpoint.node),
                )]);
            }
        }
    }

    // R9: deterministic lexicographic order.
    edges.sort_by(|a, b| {
        a.from
            .node
            .cmp(&b.from.node)
            .then_with(|| a.from.port.cmp(&b.from.port))
            .then_with(|| a.to.node.cmp(&b.to.node))
            .then_with(|| a.to.port.cmp(&b.to.port))
            .then_with(|| a.label.join(",").cmp(&b.label.join(",")))
    });
    Ok(edges)
}

/// `(hiddenNodes, hiddenEdges)` for one boundary: the recursive totals of
/// every scope under `{scope.ptr}/{decl_index}/body` (§6 counts; content
/// stays withheld regardless).
pub fn hidden_counts(
    registry: &GraphRegistry<'_>,
    scope_edge_sets: &[Vec<GraphEdgeView>],
    boundary_id: &str,
) -> Result<(usize, usize), Vec<Diagnostic>> {
    let (scope_idx, node_idx) = registry.index.get(boundary_id).copied().ok_or_else(|| {
        vec![Diagnostic::new(
            "PROJ-0001",
            format!("unknown boundary node {boundary_id}"),
        )]
    })?;
    let child_ptr = {
        let scope = &registry.scopes[scope_idx];
        let node = &scope.nodes[node_idx];
        format!("{}/{}/body", scope.ptr, node.decl_index)
    };
    let mut hidden_nodes = 0usize;
    let mut hidden_edges = 0usize;
    for (i, scope) in registry.scopes.iter().enumerate() {
        if scope.ptr == child_ptr || scope.ptr.starts_with(&format!("{child_ptr}/")) {
            hidden_nodes += scope.nodes.len();
            hidden_edges += scope_edge_sets.get(i).map_or(0, |edges| edges.len());
        }
    }
    Ok((hidden_nodes, hidden_edges))
}

/// One scope's nodes in fully revealed form — the shared shape of §8.1
/// digest content and revealed payload `children`.
fn render_scope(
    registry: &GraphRegistry<'_>,
    scope_edge_sets: &[Vec<GraphEdgeView>],
    scope_idx: usize,
    done: &BTreeMap<String, String>,
) -> Result<Vec<GraphNodeView>, Vec<Diagnostic>> {
    registry.scopes[scope_idx]
        .nodes
        .iter()
        .enumerate()
        .map(|(node_idx, node)| {
            render_revealed(registry, scope_edge_sets, scope_idx, node_idx, node, done)
        })
        .collect()
}

/// A composite with its direct contents attached: children fully revealed
/// recursively, `internalEdges` from that scope's precomputed edge set,
/// no disclosure state anywhere.
fn render_revealed(
    registry: &GraphRegistry<'_>,
    scope_edge_sets: &[Vec<GraphEdgeView>],
    scope_idx: usize,
    node_idx: usize,
    node: &RegistryNode<'_>,
    done: &BTreeMap<String, String>,
) -> Result<GraphNodeView, Vec<Diagnostic>> {
    let mut view = render_node(node.item, &node.node_id, done)?;
    if let GraphNodeView::Composite {
        children,
        internal_edges,
        ..
    } = &mut view
    {
        let child_idx = registry.child_scope(scope_idx, node_idx).ok_or_else(|| {
            vec![Diagnostic::new(
                "PROJ-0001",
                format!("missing child scope for {}", node.node_id),
            )]
        })?;
        *children = Some(render_scope(registry, scope_edge_sets, child_idx, done)?);
        *internal_edges = Some(scope_edge_sets.get(child_idx).cloned().ok_or_else(|| {
            vec![Diagnostic::new(
                "PROJ-0001",
                format!("missing edge set for child scope of {}", node.node_id),
            )]
        })?);
    }
    Ok(view)
}

/// The boundary's subtree in unwithheld form for §8.1: the subject node
/// with its OWN `childSubgraphDigest` field omitted (a placeholder value
/// is dropped during serialization — `render_node` rightly refuses a
/// composite whose digest is missing), every descendant revealed, no
/// disclosure state anywhere.
pub fn unwithheld_content(
    registry: &GraphRegistry<'_>,
    scope_edge_sets: &[Vec<GraphEdgeView>],
    boundary_id: &str,
    done: &BTreeMap<String, String>,
) -> Result<serde_json::Value, Vec<Diagnostic>> {
    let (scope_idx, node_idx) = registry.index.get(boundary_id).copied().ok_or_else(|| {
        vec![Diagnostic::new(
            "PROJ-0001",
            format!("unknown boundary node {boundary_id}"),
        )]
    })?;
    let node = &registry.scopes[scope_idx].nodes[node_idx];
    let BodyItem::Composite(c) = node.item else {
        return Err(vec![Diagnostic::new(
            "PROJ-0001",
            format!("{boundary_id} is not a composite boundary"),
        )]);
    };
    let child_idx = registry.child_scope(scope_idx, node_idx).ok_or_else(|| {
        vec![Diagnostic::new(
            "PROJ-0001",
            format!("missing child scope for {boundary_id}"),
        )]
    })?;
    let view = GraphNodeView::Composite {
        id: node.node_id.clone(),
        inputs: c.input_ports.iter().map(port_view).collect(),
        outputs: c.output_ports.iter().map(port_view).collect(),
        // Placeholder only: removed below before any digest observes it.
        child_subgraph_digest: String::new(),
        children: Some(render_scope(registry, scope_edge_sets, child_idx, done)?),
        internal_edges: Some(scope_edge_sets.get(child_idx).cloned().ok_or_else(|| {
            vec![Diagnostic::new(
                "PROJ-0001",
                format!("missing edge set for child scope of {boundary_id}"),
            )]
        })?),
        disclosure: None,
    };
    let mut value = serde_json::to_value(&view).map_err(|e| {
        vec![Diagnostic::new(
            "PROJ-0001",
            format!("boundary render cannot be serialized ({e})"),
        )]
    })?;
    // The placeholder digest is dropped here (never observed by a
    // digest): `render_node` rightly refuses a composite whose
    // `childSubgraphDigest` is missing, so the field must disappear
    // before any serialization of the view is compared.
    if let Some(obj) = value.as_object_mut() {
        obj.remove("childSubgraphDigest");
    }
    Ok(value)
}

/// One scope's nodes under the disclosure policy: authorized composites
/// expand (their children re-apply this same policy — non-cascading §6),
/// withheld composites emit their disclosure block (with counts only
/// under count authorization), atomics always render in full.
fn render_payload_scope(
    registry: &GraphRegistry<'_>,
    scope_edge_sets: &[Vec<GraphEdgeView>],
    policy: &GraphDisclosurePolicy,
    scope_idx: usize,
    child_digests: &BTreeMap<String, String>,
) -> Result<Vec<GraphNodeView>, Vec<Diagnostic>> {
    registry.scopes[scope_idx]
        .nodes
        .iter()
        .enumerate()
        .map(|(node_idx, node)| {
            let authorized = policy
                .authorized_boundaries
                .iter()
                .any(|id| id == &node.node_id);
            let mut view = render_node(node.item, &node.node_id, child_digests)?;
            if let GraphNodeView::Composite {
                children,
                internal_edges,
                disclosure,
                ..
            } = &mut view
            {
                if authorized {
                    let child_idx = registry.child_scope(scope_idx, node_idx).ok_or_else(|| {
                        vec![Diagnostic::new(
                            "PROJ-0001",
                            format!("missing child scope for {}", node.node_id),
                        )]
                    })?;
                    *children = Some(render_payload_scope(
                        registry,
                        scope_edge_sets,
                        policy,
                        child_idx,
                        child_digests,
                    )?);
                    *internal_edges =
                        Some(scope_edge_sets.get(child_idx).cloned().ok_or_else(|| {
                            vec![Diagnostic::new(
                                "PROJ-0001",
                                format!("missing edge set for child scope of {}", node.node_id),
                            )]
                        })?);
                } else {
                    let mut hidden_nodes = None;
                    let mut hidden_edges = None;
                    if policy
                        .count_authorization
                        .iter()
                        .any(|id| id == &node.node_id)
                    {
                        let (n, e) = hidden_counts(registry, scope_edge_sets, &node.node_id)?;
                        hidden_nodes = Some(n);
                        hidden_edges = Some(e);
                    }
                    *disclosure = Some(GraphDisclosureView {
                        withheld: true,
                        reason: "private-composite-default".to_string(),
                        category: "composite-boundary".to_string(),
                        hidden_nodes,
                        hidden_edges,
                    });
                }
            }
            Ok(view)
        })
        .collect()
}

/// Byte-stable graph projection of the validated AST (spec §3–§4): policy
/// validation first (§6), every scope's edge set computed
/// disclosure-independently (§7), `childSubgraphDigest`s built bottom-up
/// in unwithheld form (§8.1), then the payload renders under the policy
/// (§5–§6) and is enveloped by the Slice-1 machinery.
pub fn project_graph_json(
    root: &WorkflowDefinition,
    policy: Option<&GraphDisclosurePolicy>,
) -> Result<VersionedProjectionEnvelope<serde_json::Value>, Vec<Diagnostic>> {
    validated_source(root)?;
    let source_digest = source_digest_of(root)?;
    let registry = GraphRegistry::build(root)?;
    let policy = normalize_policy(&registry, policy)?;
    let policy_digest = digest_of(&policy_digest_preimage(&source_digest, &policy))?;

    let mut scope_edge_sets: Vec<Vec<GraphEdgeView>> = Vec::with_capacity(registry.scopes.len());
    for i in 0..registry.scopes.len() {
        scope_edge_sets.push(scope_edges(&registry, root, i)?);
    }

    // §8.1 bottom-up driver. The content closure is infallible by
    // construction, so any render failure is captured and re-raised
    // fail-closed after the driver returns.
    let mut render_error: Option<Vec<Diagnostic>> = None;
    let postorder = registry.composite_postorder();
    let child_digests =
        compute_child_subgraph_digests(&source_digest, &postorder, |boundary_id, done| {
            match unwithheld_content(&registry, &scope_edge_sets, boundary_id, done) {
                Ok(value) => value,
                Err(diags) => {
                    if render_error.is_none() {
                        render_error = Some(diags);
                    }
                    serde_json::json!({})
                }
            }
        });
    if let Some(diags) = render_error {
        return Err(diags);
    }

    let nodes = render_payload_scope(&registry, &scope_edge_sets, &policy, 0, &child_digests)?;
    let payload = GraphPayload {
        graph_schema_version: GRAPH_SCHEMA_VERSION.to_string(),
        policy_digest,
        input_ports: root.input_ports.iter().map(port_view).collect(),
        output_ports: root.output_ports.iter().map(port_view).collect(),
        nodes,
        edges: scope_edge_sets.first().cloned().unwrap_or_default(),
    };
    let payload_value = serde_json::to_value(&payload).map_err(|e| {
        vec![Diagnostic::new(
            "PROJ-0001",
            format!("graph payload cannot be serialized ({e})"),
        )]
    })?;
    let projection_digest = digest_of(&payload_value)?;
    Ok(VersionedProjectionEnvelope {
        projection_version: PROJECTION_VERSION_V1.to_string(),
        schema_version: root.schema_version.clone(),
        source_digest,
        projection_digest,
        payload: payload_value,
    })
}

/// Lowercase-64-hex digest shape (§8.3 and the payload digest fields of §4).
fn is_hex64(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Node id grammar (§4): `s{NNNN}:{operation_id}` — checked byte-wise; no
/// regex (dependency policy).
fn node_id_grammar_ok(id: &str) -> bool {
    let b = id.as_bytes();
    b.len() > 6 && b[0] == b's' && b[1..5].iter().all(u8::is_ascii_digit) && b[5] == b':'
}

/// Every byte-path structural refusal is `PROJ-0001` (§9: policy-independent
/// shape errors, dangling/cross-boundary endpoints, non-canonical bytes).
fn shape_err(message: String) -> Vec<Diagnostic> {
    vec![Diagnostic::new("PROJ-0001", message)]
}

/// One visible node's declared port names, for endpoint-reference checks.
struct ScopeNodePorts {
    composite: bool,
    inputs: Vec<String>,
    outputs: Vec<String>,
}

/// One scope's edge array: kinds, endpoint shapes, endpoint references
/// visible in this scope (root `edges` ⇒ root nodes; `internalEdges` ⇒
/// {container, direct children}), ports declared on the endpoint node
/// (atomic: direction-typed; composite: either side), non-empty labels.
fn validate_scope_edges(
    edges: &serde_json::Value,
    index: &BTreeMap<String, ScopeNodePorts>,
    owner: Option<(&str, &ScopeNodePorts)>,
) -> Result<(), Vec<Diagnostic>> {
    let list = edges
        .as_array()
        .ok_or_else(|| shape_err("scope edges must be an array".to_string()))?;
    for edge in list {
        let obj = edge
            .as_object()
            .ok_or_else(|| shape_err("edge must be an object".to_string()))?;
        match obj.get("kind").and_then(serde_json::Value::as_str) {
            Some("outcome") | Some("boundary-value") => {}
            other => return Err(shape_err(format!("edge with unknown kind {other:?}"))),
        }
        for field in ["from", "to"] {
            let endpoint = obj
                .get(field)
                .and_then(serde_json::Value::as_object)
                .ok_or_else(|| shape_err(format!("edge {field} must be an object")))?;
            let node = endpoint
                .get("node")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| shape_err(format!("edge {field} endpoint without a node")))?;
            let port = endpoint
                .get("port")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| shape_err(format!("edge {field} endpoint without a port")))?;
            let ports: &ScopeNodePorts = if let Some(p) = index.get(node) {
                p
            } else if let Some((owner_id, owner_ports)) = owner {
                if owner_id == node {
                    owner_ports
                } else {
                    return Err(shape_err(format!(
                        "edge endpoint {node} is not visible in its scope"
                    )));
                }
            } else {
                return Err(shape_err(format!(
                    "edge endpoint {node} is not visible in its scope"
                )));
            };
            let declared = if ports.composite {
                ports
                    .inputs
                    .iter()
                    .chain(ports.outputs.iter())
                    .any(|p| p.as_str() == port)
            } else if field == "from" {
                ports.outputs.iter().any(|p| p.as_str() == port)
            } else {
                ports.inputs.iter().any(|p| p.as_str() == port)
            };
            if !declared {
                return Err(shape_err(format!(
                    "edge endpoint {node} does not declare port {port}"
                )));
            }
        }
        match obj.get("label").and_then(serde_json::Value::as_array) {
            Some(labels)
                if !labels.is_empty()
                    && labels
                        .iter()
                        .all(|l| matches!(l.as_str(), Some(s) if !s.is_empty())) => {}
            _ => {
                return Err(shape_err(
                    "edge label must be a non-empty array of strings".to_string(),
                ));
            }
        }
    }
    Ok(())
}

/// One scope's nodes under the byte path: id grammar, document-wide
/// uniqueness, kind-specific required fields, withheld/revealed
/// exclusivity, port shapes — then its edges, then (for revealed
/// containers) recursion into each child scope.
fn validate_scope(
    nodes: &[serde_json::Value],
    owner: Option<(&str, &ScopeNodePorts)>,
    edges: &serde_json::Value,
    seen_ids: &mut BTreeSet<String>,
) -> Result<(), Vec<Diagnostic>> {
    let mut index: BTreeMap<String, ScopeNodePorts> = BTreeMap::new();
    let mut revealed: Vec<(&str, &Vec<serde_json::Value>, &serde_json::Value)> = Vec::new();
    for node in nodes {
        let obj = node
            .as_object()
            .ok_or_else(|| shape_err("node must be an object".to_string()))?;
        let id = obj
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        if !node_id_grammar_ok(id) {
            return Err(shape_err(format!(
                "node id {id:?} does not match s{{NNNN}}:<operation_id>"
            )));
        }
        if !seen_ids.insert(id.to_string()) {
            return Err(shape_err(format!("duplicate node id {id}")));
        }
        let composite = match obj.get("kind").and_then(serde_json::Value::as_str) {
            Some("atomic") => false,
            Some("composite") => true,
            other => return Err(shape_err(format!("node {id}: unknown kind {other:?}"))),
        };
        if !composite {
            for forbidden in [
                "childSubgraphDigest",
                "children",
                "internalEdges",
                "disclosure",
            ] {
                if obj.contains_key(forbidden) {
                    return Err(shape_err(format!(
                        "{id}: atomic node must not carry {forbidden}"
                    )));
                }
            }
        }
        let inputs = obj
            .get("inputs")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| shape_err(format!("{id}: inputs must be an array")))?;
        let outputs = obj
            .get("outputs")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| shape_err(format!("{id}: outputs must be an array")))?;
        let mut input_names = Vec::with_capacity(inputs.len());
        for port in inputs {
            let port_obj = port
                .as_object()
                .ok_or_else(|| shape_err(format!("{id}: input port must be an object")))?;
            let name = port_obj
                .get("name")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| shape_err(format!("{id}: input port without a name")))?;
            if port_obj
                .get("type")
                .and_then(serde_json::Value::as_str)
                .is_none()
            {
                return Err(shape_err(format!("{id}: input {name} without a type")));
            }
            if composite {
                for field in ["cardinality", "requiredness"] {
                    if port_obj
                        .get(field)
                        .and_then(serde_json::Value::as_str)
                        .is_none()
                    {
                        return Err(shape_err(format!(
                            "{id}: boundary port {name} without {field}"
                        )));
                    }
                }
            } else {
                let accepted_ok = port_obj
                    .get("acceptedOutcomes")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|values| values.iter().all(serde_json::Value::is_string));
                if !accepted_ok {
                    return Err(shape_err(format!(
                        "{id}: input {name} needs a string acceptedOutcomes array"
                    )));
                }
            }
            input_names.push(name.to_string());
        }
        let mut output_names = Vec::with_capacity(outputs.len());
        for port in outputs {
            let port_obj = port
                .as_object()
                .ok_or_else(|| shape_err(format!("{id}: output port must be an object")))?;
            let name = port_obj
                .get("name")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| shape_err(format!("{id}: output port without a name")))?;
            if port_obj
                .get("type")
                .and_then(serde_json::Value::as_str)
                .is_none()
            {
                return Err(shape_err(format!("{id}: output {name} without a type")));
            }
            if composite {
                for field in ["cardinality", "requiredness"] {
                    if port_obj
                        .get(field)
                        .and_then(serde_json::Value::as_str)
                        .is_none()
                    {
                        return Err(shape_err(format!(
                            "{id}: boundary port {name} without {field}"
                        )));
                    }
                }
            } else {
                let emits_ok = match port_obj.get("emits") {
                    None => true,
                    Some(v) => v
                        .as_array()
                        .is_some_and(|values| values.iter().all(serde_json::Value::is_string)),
                };
                if !emits_ok {
                    return Err(shape_err(format!(
                        "{id}: output {name} emits must be an array of strings"
                    )));
                }
            }
            output_names.push(name.to_string());
        }
        if composite {
            let digest = obj
                .get("childSubgraphDigest")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            if !is_hex64(digest) {
                return Err(shape_err(format!(
                    "{id}: childSubgraphDigest is not a lowercase 64-hex digest"
                )));
            }
            match (
                obj.get("children"),
                obj.get("internalEdges"),
                obj.get("disclosure"),
            ) {
                (Some(children), Some(internal_edges), None) => {
                    let children = children
                        .as_array()
                        .ok_or_else(|| shape_err(format!("{id}: children must be an array")))?;
                    internal_edges.as_array().ok_or_else(|| {
                        shape_err(format!("{id}: internalEdges must be an array"))
                    })?;
                    revealed.push((id, children, internal_edges));
                }
                (None, None, Some(disclosure)) => {
                    let d = disclosure
                        .as_object()
                        .ok_or_else(|| shape_err(format!("{id}: disclosure must be an object")))?;
                    if d.get("withheld").and_then(serde_json::Value::as_bool) != Some(true) {
                        return Err(shape_err(format!("{id}: disclosure.withheld must be true")));
                    }
                    for field in ["reason", "category"] {
                        if d.get(field).and_then(serde_json::Value::as_str).is_none() {
                            return Err(shape_err(format!(
                                "{id}: disclosure without a string {field}"
                            )));
                        }
                    }
                    for field in ["hiddenNodes", "hiddenEdges"] {
                        match d.get(field) {
                            None => {}
                            Some(v) if v.as_u64().is_some() => {}
                            Some(_) => {
                                return Err(shape_err(format!(
                                    "{id}: disclosure.{field} must be a non-negative integer"
                                )));
                            }
                        }
                    }
                }
                (Some(_), _, Some(_)) | (_, Some(_), Some(_)) => {
                    return Err(shape_err(format!(
                        "{id}: withheld composite must not carry children/internalEdges"
                    )));
                }
                (Some(_), None, None) => {
                    return Err(shape_err(format!(
                        "{id}: revealed composite requires internalEdges alongside children"
                    )));
                }
                (None, Some(_), None) => {
                    return Err(shape_err(format!(
                        "{id}: revealed composite requires children alongside internalEdges"
                    )));
                }
                (None, None, None) => {
                    return Err(shape_err(format!(
                        "{id}: composite requires children and internalEdges or a disclosure block"
                    )));
                }
            }
        }
        index.insert(
            id.to_string(),
            ScopeNodePorts {
                composite,
                inputs: input_names,
                outputs: output_names,
            },
        );
    }
    validate_scope_edges(edges, &index, owner)?;
    for (child_owner, children, internal_edges) in revealed {
        let ports = index.get(child_owner).ok_or_else(|| {
            shape_err(format!(
                "{child_owner}: port table missing after registration"
            ))
        })?;
        validate_scope(
            children,
            Some((child_owner, ports)),
            internal_edges,
            seen_ids,
        )?;
    }
    Ok(())
}

/// §11 byte-path payload structural validation: required payload fields,
/// `policyDigest` shape, boundary-port shapes, and the full per-scope walk
/// (id grammar, kind-specific fields, exclusivity, endpoint visibility).
fn validate_graph_payload_structure(payload: &serde_json::Value) -> Result<(), Vec<Diagnostic>> {
    let root = payload
        .as_object()
        .ok_or_else(|| shape_err("graph payload must be an object".to_string()))?;
    for key in [
        "policyDigest",
        "inputPorts",
        "outputPorts",
        "nodes",
        "edges",
    ] {
        if !root.contains_key(key) {
            return Err(shape_err(format!("graph payload is missing {key}")));
        }
    }
    match root["policyDigest"].as_str() {
        Some(d) if is_hex64(d) => {}
        _ => {
            return Err(shape_err(
                "policyDigest is not a lowercase 64-hex digest".to_string(),
            ));
        }
    }
    for key in ["inputPorts", "outputPorts"] {
        let ports = root[key]
            .as_array()
            .ok_or_else(|| shape_err(format!("{key} must be an array")))?;
        for port in ports {
            let port_obj = port
                .as_object()
                .ok_or_else(|| shape_err(format!("{key} port must be an object")))?;
            for field in ["name", "type", "cardinality", "requiredness"] {
                if port_obj
                    .get(field)
                    .and_then(serde_json::Value::as_str)
                    .is_none()
                {
                    return Err(shape_err(format!("{key} port without a string {field}")));
                }
            }
        }
    }
    let root_nodes = root["nodes"]
        .as_array()
        .ok_or_else(|| shape_err("nodes must be an array".to_string()))?;
    let mut seen_ids = BTreeSet::new();
    validate_scope(root_nodes, None, &root["edges"], &mut seen_ids)
}

/// §11 byte path: envelope parse (duplicate keys -> shape -> envelope
/// metadata -> canonical re-render) -> graphSchemaVersion allow-list ->
/// payload structural validation -> payload digest check. Establishes byte
/// integrity, versions, structure, visible references, and
/// `projectionDigest` only; `policyDigest` and withheld boundary digests
/// are shape-checked here and recomputed exclusively by
/// `verify_graph_against_source` steps 6-7 (§8.1/§8.2).
pub fn verify_graph_projection_bytes(
    raw: &[u8],
) -> Result<VersionedProjectionEnvelope<serde_json::Value>, Vec<Diagnostic>> {
    let env = parse_envelope_bytes::<serde_json::Value>(raw)?;
    match env
        .payload
        .get("graphSchemaVersion")
        .and_then(serde_json::Value::as_str)
    {
        Some(v) if ALLOWED_GRAPH_SCHEMA_VERSIONS.contains(&v) => {}
        other => {
            return Err(vec![Diagnostic::new(
                "SOMA-CMP-0001",
                format!("unsupported graphSchemaVersion {other:?}"),
            )]);
        }
    }
    validate_graph_payload_structure(&env.payload)?;
    let expected = digest_of(&env.payload)?;
    if expected != env.projection_digest {
        return Err(vec![Diagnostic::new(
            "SOMA-CMP-0004",
            "graph projection digest does not verify",
        )]);
    }
    Ok(env)
}

/// §11 step 7: recompute every `childSubgraphDigest` from the source via
/// the §8.1 acyclic construction (same registry, scope edges, and
/// bottom-up driver `project_graph_json` uses).
fn recompute_child_subgraph_digests(
    root: &WorkflowDefinition,
    source_digest: &str,
) -> Result<BTreeMap<String, String>, Vec<Diagnostic>> {
    let registry = GraphRegistry::build(root)?;
    let mut scope_edge_sets: Vec<Vec<GraphEdgeView>> = Vec::with_capacity(registry.scopes.len());
    for i in 0..registry.scopes.len() {
        scope_edge_sets.push(scope_edges(&registry, root, i)?);
    }
    let mut render_error: Option<Vec<Diagnostic>> = None;
    let postorder = registry.composite_postorder();
    let child_digests =
        compute_child_subgraph_digests(source_digest, &postorder, |boundary_id, done| {
            match unwithheld_content(&registry, &scope_edge_sets, boundary_id, done) {
                Ok(value) => value,
                Err(diags) => {
                    if render_error.is_none() {
                        render_error = Some(diags);
                    }
                    serde_json::json!({})
                }
            }
        });
    if let Some(diags) = render_error {
        return Err(diags);
    }
    Ok(child_digests)
}

/// §11 step 7 comparison: every embedded `childSubgraphDigest` in the
/// envelope payload against the value recomputed from the source (hidden
/// boundaries carry no subtree content in the bytes and cannot be forged).
fn compare_child_subgraph_digests(
    nodes: &[serde_json::Value],
    fresh: &BTreeMap<String, String>,
) -> Result<(), Vec<Diagnostic>> {
    for node in nodes {
        if node.get("kind").and_then(serde_json::Value::as_str) != Some("composite") {
            continue;
        }
        let id = node
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let embedded = node
            .get("childSubgraphDigest")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        match fresh.get(id) {
            Some(expected) if expected.as_str() == embedded => {}
            _ => {
                return Err(vec![Diagnostic::new(
                    "SOMA-CMP-0004",
                    format!("childSubgraphDigest for {id} does not verify against the source AST"),
                )]);
            }
        }
        if let Some(children) = node.get("children").and_then(serde_json::Value::as_array) {
            compare_child_subgraph_digests(children, fresh)?;
        }
    }
    Ok(())
}

/// §11 against-source path (B2 pattern): ordered, self-contained, no
/// prior byte verification required. Steps 6-7 deliberately precede the
/// wholesale comparison so derived-digest mismatches report
/// `SOMA-CMP-0004`, never `PROJ-0002`:
/// 1 source audit + envelope metadata; 2 graphSchemaVersion allow-list;
/// 3 envelope payload digest; 4 envelope/root schema binding;
/// 5 policy validation/normalization (`PROJ-0003`);
/// 6 policyDigest recomputation; 7 childSubgraphDigest recomputation;
/// 8 fresh `project_graph_json` identity — `sourceDigest` + full payload
/// equality (`PROJ-0002`).
pub fn verify_graph_against_source(
    envelope: &VersionedProjectionEnvelope<serde_json::Value>,
    root: &WorkflowDefinition,
    policy: Option<&GraphDisclosurePolicy>,
) -> Result<(), Vec<Diagnostic>> {
    // Step 1: source audit gate, then envelope metadata (Slice-1 codes).
    validated_source(root)?;
    verify_envelope_metadata(envelope)?;
    // Step 2: graphSchemaVersion allow-list (§8.3).
    match envelope
        .payload
        .get("graphSchemaVersion")
        .and_then(serde_json::Value::as_str)
    {
        Some(v) if ALLOWED_GRAPH_SCHEMA_VERSIONS.contains(&v) => {}
        other => {
            return Err(vec![Diagnostic::new(
                "SOMA-CMP-0001",
                format!("unsupported graphSchemaVersion {other:?}"),
            )]);
        }
    }
    // Step 3: envelope payload digest.
    let expected_payload = digest_of(&envelope.payload)?;
    if expected_payload != envelope.projection_digest {
        return Err(vec![Diagnostic::new(
            "SOMA-CMP-0004",
            "graph projection digest does not verify",
        )]);
    }
    // Step 4: envelope/root schema binding.
    if envelope.schema_version != root.schema_version {
        return Err(vec![Diagnostic::new(
            "PROJ-0002",
            "schemaVersion does not match the source AST",
        )]);
    }
    // Step 5: policy validation and normalization (§6, PROJ-0003).
    let registry = GraphRegistry::build(root)?;
    let normalized = normalize_policy(&registry, policy)?;
    // Step 6: policyDigest recomputed from the SOURCE root digest (§8.2).
    let source_digest = source_digest_of(root)?;
    let expected_policy = digest_of(&policy_digest_preimage(&source_digest, &normalized))?;
    if envelope
        .payload
        .get("policyDigest")
        .and_then(serde_json::Value::as_str)
        != Some(expected_policy.as_str())
    {
        return Err(vec![Diagnostic::new(
            "SOMA-CMP-0004",
            "policyDigest does not verify against the source AST",
        )]);
    }
    // Step 7: childSubgraphDigest recomputation (§8.1).
    let fresh_child_digests = recompute_child_subgraph_digests(root, &source_digest)?;
    if let Some(nodes) = envelope
        .payload
        .get("nodes")
        .and_then(serde_json::Value::as_array)
    {
        compare_child_subgraph_digests(nodes, &fresh_child_digests)?;
    }
    // Step 8: fresh-render identity.
    let fresh = project_graph_json(root, policy)?;
    if fresh.source_digest != envelope.source_digest || fresh.payload != envelope.payload {
        return Err(vec![Diagnostic::new(
            "PROJ-0002",
            "graph projection does not match a fresh render of the source AST",
        )]);
    }
    Ok(())
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

    /// Spec §12 test 19b — R6 defense: with the root boundary supply removed,
    /// a producer-less input must be refused by the builder itself even though
    /// the registry (which does not model supply) builds fine.
    #[test]
    fn r6_defensive_free_input_has_no_boundary_port() {
        let mut wf = wf();
        wf.input_ports.clear();
        let registry = GraphRegistry::build(&wf).expect("registry does not check supply");
        let err = scope_edges(&registry, &wf, 0).expect_err("free input must be refused");
        assert_eq!(err[0].code, "PROJ-0001");
        assert!(
            err[0].message.contains("no producer and no boundary port"),
            "unexpected message: {}",
            err[0].message
        );
    }

    /// Spec §12 test 25 — dataflow defensive rules unit-tested below the
    /// gate: empty labels/intersections, duplicate operation ports, duplicate
    /// boundary ports. (Dangling endpoints are covered by the byte-path craft
    /// in Task 5's test 22 — they are unreachable through `scope_edges` itself.)
    #[test]
    fn dataflow_defensive_rules() {
        // Matched connection whose producer declares no `emits` (§7.4).
        // Each case is block-scoped: `let mut wf = wf();` must still see the
        // fixture *function* in every case, never the previous case's value.
        {
            let mut wf = wf();
            let BodyItem::Operation(prep) = &mut wf.body[0] else {
                panic!("prep is an operation");
            };
            prep.outputs[0].emits = None;
            let registry = GraphRegistry::build(&wf).expect("registry");
            let err = scope_edges(&registry, &wf, 0).expect_err("empty label refused");
            assert_eq!(err[0].code, "PROJ-0001");
            assert!(err[0].message.contains("no derivable outcome label"));
        }

        // Non-empty emits with an empty intersection against acceptedOutcomes.
        {
            let mut wf = wf();
            let BodyItem::Operation(prep) = &mut wf.body[0] else {
                panic!("prep is an operation");
            };
            prep.outputs[0].emits = Some(vec![OutcomeVariant::Blocked]);
            let registry = GraphRegistry::build(&wf).expect("registry");
            let err = scope_edges(&registry, &wf, 0).expect_err("empty intersection refused");
            assert_eq!(err[0].code, "PROJ-0001");
            assert!(err[0].message.contains("no derivable outcome label"));
        }

        // Duplicate input names within one operation.
        {
            let mut wf = wf();
            let BodyItem::Operation(audit_op) = &mut wf.body[2] else {
                panic!("audit is an operation");
            };
            audit_op.inputs.push(audit_op.inputs[0].clone());
            let registry = GraphRegistry::build(&wf).expect("registry");
            let err = scope_edges(&registry, &wf, 0).expect_err("duplicate input refused");
            assert_eq!(err[0].code, "PROJ-0001");
            assert!(err[0].message.contains("duplicate input port"));
        }

        // Duplicate output names within one operation.
        {
            let mut wf = wf();
            let BodyItem::Operation(prep) = &mut wf.body[0] else {
                panic!("prep is an operation");
            };
            prep.outputs.push(prep.outputs[0].clone());
            let registry = GraphRegistry::build(&wf).expect("registry");
            let err = scope_edges(&registry, &wf, 0).expect_err("duplicate output refused");
            assert_eq!(err[0].code, "PROJ-0001");
            assert!(err[0].message.contains("duplicate output port"));
        }

        // Duplicate boundary port names within the scope's boundary.
        {
            let mut wf = wf();
            wf.input_ports.push(wf.input_ports[0].clone());
            let registry = GraphRegistry::build(&wf).expect("registry");
            let err = scope_edges(&registry, &wf, 0).expect_err("duplicate boundary port refused");
            assert_eq!(err[0].code, "PROJ-0001");
            assert!(err[0].message.contains("duplicate boundary input port"));
        }
    }

    #[test]
    fn hidden_counts_totals_every_descendant_scope() {
        let wf = wf();
        let registry = GraphRegistry::build(&wf).expect("registry");
        let mut sets = Vec::with_capacity(registry.scopes.len());
        for i in 0..registry.scopes.len() {
            sets.push(scope_edges(&registry, &wf, i).expect("edges"));
        }
        let (nodes, edges) = hidden_counts(&registry, &sets, "s0001:flow").expect("counts");
        assert_eq!(
            (nodes, edges),
            (2, 4),
            "flow hides two nodes and four edges"
        );
        let err = hidden_counts(&registry, &sets, "s9999:ghost").expect_err("unknown id");
        assert_eq!(err[0].code, "PROJ-0001");
    }
}
