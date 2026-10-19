//! Lite-owned execution data derived from a SOMA workflow.
//!
//! The published v1.1 `ExecutionPlan` keeps exactly its reference shape —
//! `steps: [{key, operationId}]` with `additionalProperties: false`, sealed
//! under the v1.1 rule. Dependency, topology and authority-reduction data
//! therefore cannot live in the plan; they live here as a separate,
//! versioned Lite structure that never masquerades as a SOMA plan.
//!
//! Ordering semantics are ported from the reference compiler at soma-core
//! `022142b` (`topological_order`): Kahn's algorithm over workflow-body
//! units; among ready units the smallest declaration ordinal wins, so
//! renaming an internal identifier never reorders independent units.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::workflow::governance_compiler::workflow_digest_of;
use crate::workflow::soma::Diagnostic;
use crate::workflow::soma::contracts::{
    AuthorityProfile, BodyItem, OperationDefinition, SecretGrant, WorkflowDefinition,
};
use crate::workflow::soma::types::MutationMode;

/// Schema version of this Lite-owned structure (semantically versioned;
/// never confused with the SOMA plan's `schemaVersion`).
pub const EXECUTION_GRAPH_SCHEMA_VERSION: &str = "lite.execution-graph.v1";

/// Output channel names a body item produces in its scope: operation
/// outputs, or a container's declared output ports.
fn producer_names(item: &BodyItem) -> Vec<&str> {
    match item {
        BodyItem::Operation(op) => op.outputs.iter().map(|o| o.name.as_str()).collect(),
        BodyItem::Composite(c) => c.output_ports.iter().map(|p| p.name.as_str()).collect(),
    }
}

/// Input channel names a body item consumes in its scope: operation
/// inputs, or a container's declared input ports.
fn consumer_names(item: &BodyItem) -> Vec<&str> {
    match item {
        BodyItem::Operation(op) => op.inputs.iter().map(|i| i.name.as_str()).collect(),
        BodyItem::Composite(c) => c.input_ports.iter().map(|p| p.name.as_str()).collect(),
    }
}

/// Deterministic topological order over one scope's body items.
///
/// Within a scope an edge `producer -> consumer` exists when a consumer
/// input name matches a producer output name (the reference compiler's
/// dataflow rule, extended to container boundary ports — spec §7.1/R8).
/// Among ready units the smallest declaration ordinal wins, so renaming
/// an internal identifier never reorders independent units. Returns
/// `None` on a cycle: the gate refuses with `SOMA-EXP-0002`, graph
/// construction fails closed with `PROJ-0001`.
pub(crate) fn topological_order_body(items: &[BodyItem]) -> Option<Vec<usize>> {
    let n = items.len();
    let mut producers: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, item) in items.iter().enumerate() {
        for name in producer_names(item) {
            producers.entry(name).or_default().push(i);
        }
    }
    let mut indegree = vec![0usize; n];
    let mut edges = vec![Vec::<usize>::new(); n];
    for (i, item) in items.iter().enumerate() {
        let mut deps: Vec<usize> = Vec::new();
        for name in consumer_names(item) {
            if let Some(producers_of_name) = producers.get(name) {
                for j in producers_of_name {
                    if *j != i {
                        deps.push(*j);
                    }
                }
            }
        }
        deps.sort_unstable();
        deps.dedup();
        indegree[i] = deps.len();
        for d in deps {
            edges[d].push(i);
        }
    }
    let mut ready: Vec<usize> = (0..n).filter(|i| indegree[*i] == 0).collect();
    let mut order = Vec::with_capacity(n);
    while !ready.is_empty() {
        ready.sort_unstable();
        let i = ready.remove(0);
        order.push(i);
        for nxt in edges[i].clone() {
            indegree[nxt] -= 1;
            if indegree[nxt] == 0 {
                ready.push(nxt);
            }
        }
    }
    (order.len() == n).then_some(order)
}

/// Deterministic topological order over the root workflow body.
///
/// Spec §2.4: any root composite makes execution ordering fail closed —
/// every existing caller (`projection::human`, `compile_execution_graph`)
/// then refuses (`PROJ-0001` / `SOMA-EXP-0002` at their own scans).
pub(crate) fn topological_order(wf: &WorkflowDefinition) -> Option<Vec<usize>> {
    if wf.contains_composite_body_item() {
        return None;
    }
    topological_order_body(&wf.body)
}

/// A compiled, versioned execution graph — topology, dependency, and
/// reduced-authority data extracted from an audited SOMA workflow.
///
/// This structure is Lite-owned: it is never serialized as (or into) the
/// published v1.1 `ExecutionPlan`, whose canonical bytes stay byte-
/// identical to the published schema (review correction 1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledExecutionGraphV1 {
    pub schema_version: String,
    /// Canonical digest of the workflow with `contentDigest` removed —
    /// the same rule the plan seal and audit use.
    pub workflow_digest: String,
    /// The sealed plan this graph belongs to: `canonicalization.sha256`
    /// of the v1.1 plan compiled from the same workflow text.
    pub plan_identity: String,
    pub steps: Vec<ExecutionGraphStep>,
}

/// One scheduled unit: plan-compatible key/order plus data the plan shape
/// must not carry (dependencies and the unit's reduced authority).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionGraphStep {
    pub operation_id: String,
    /// Plan-compatible key: `s{topological index:04}:{operationId}`.
    pub key: String,
    /// Topological position (the index baked into `key`).
    pub order: u32,
    /// Operation ids of the producers this unit consumes (sorted,
    /// deduplicated, self excluded).
    pub dependencies: Vec<String>,
    /// The unit's authority reduced against the workflow ceiling (R8b).
    pub granted: AuthorityProfile,
    /// Enclosing composite scope path (E4/I07 #253): empty for
    /// root-scope operations, so atomic workflows serialize byte-identical.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scope_path: Vec<String>,
}

/// Compile the Lite execution graph for a workflow.
///
/// Callers must chain this after `compile_workflow_text` (the audit gate):
/// this entry point re-parses and enforces dataflow well-formedness plus
/// fail-closed authority reduction, but it is not a substitute for the
/// full audit. `plan_identity` is the sealed plan's
/// `canonicalization.sha256` from that same compilation.
///
/// E4/I07 (#253): a workflow whose body contains composites compiles
/// over its deterministic expansion — the flattened leaf operations in
/// scope order, identified by fully-qualified `scopePath` runtime ids.
pub fn compile_execution_graph(
    workflow_text: &str,
    plan_identity: &str,
) -> Result<CompiledExecutionGraphV1, Vec<Diagnostic>> {
    let model: WorkflowDefinition = serde_json::from_str(workflow_text).map_err(|e| {
        vec![Diagnostic::new(
            "SOMA-CMP-0003",
            format!("schema violation: {e}"),
        )]
    })?;
    if model.contains_composite_body_item() {
        return compile_expanded_execution_graph(&model, plan_identity);
    }
    let workflow_digest = workflow_digest_of(&model)?;
    let Some(order) = topological_order(&model) else {
        return Err(vec![Diagnostic::new(
            "SOMA-EXP-0002",
            "workflow body has an operation-edge cycle; no topological sort",
        )]);
    };

    let mut producers: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, unit) in model.body.iter().enumerate() {
        let BodyItem::Operation(unit) = unit else {
            continue;
        };
        for output in &unit.outputs {
            producers.entry(output.name.as_str()).or_default().push(i);
        }
    }

    let mut steps = Vec::with_capacity(order.len());
    for (position, &body_idx) in order.iter().enumerate() {
        let BodyItem::Operation(unit) = &model.body[body_idx] else {
            continue;
        };
        let granted = reduce_unit_authority(&model, body_idx)?;
        let mut dependencies: BTreeSet<&str> = BTreeSet::new();
        for input in &unit.inputs {
            if let Some(producers_of_name) = producers.get(input.name.as_str()) {
                for &producer_idx in producers_of_name {
                    if producer_idx != body_idx {
                        dependencies.insert(model.body[producer_idx].id());
                    }
                }
            }
        }
        steps.push(ExecutionGraphStep {
            operation_id: unit.id.clone(),
            key: format!("s{position:04}:{}", unit.id),
            order: position as u32,
            dependencies: dependencies.into_iter().map(str::to_string).collect(),
            granted,
            scope_path: Vec::new(),
        });
    }

    Ok(CompiledExecutionGraphV1 {
        schema_version: EXECUTION_GRAPH_SCHEMA_VERSION.to_string(),
        workflow_digest,
        plan_identity: plan_identity.to_string(),
        steps,
    })
}

/// The shared per-leaf authority reduction for expanded composites
/// (E4/I07 #253, review repair P1-2): each flattened leaf paired with
/// its ACTUAL effective grant — the leaf's declared authority reduced
/// against the reviewed ceiling (the ancestor-chain import restriction
/// is enforced by the audit and the expansion chain checks). Both the
/// expanded execution graph and the compiled authority graph consume
/// THIS single reduction result, so the two runtime authority
/// representations can never disagree, and `selection_within_authority`
/// is bounded by the leaf's real grant instead of the workflow ceiling.
pub(crate) fn expanded_leaf_grants(
    model: &WorkflowDefinition,
) -> Result<
    Vec<(
        crate::workflow::composite_expansion::ExpansionStep,
        AuthorityProfile,
    )>,
    Vec<Diagnostic>,
> {
    let expansion = crate::workflow::composite_expansion::expand_composites(model)?;
    let mut leaves = Vec::with_capacity(expansion.len());
    for step in expansion {
        let (unit, _) = crate::workflow::composite_expansion::leaf_operation(model, &step)?;
        let granted = reduce_leaf_authority(&model.authority, unit)?;
        leaves.push((step, granted));
    }
    Ok(leaves)
}

/// Compile the execution graph for a workflow whose body contains
/// composites (E4/I07 #253): the steps are the deterministic expansion
/// leaves in flattened scope order, each identified by its fully-qualified
/// runtime id (`scopePath` + local id, so reused local ids in different
/// scopes can never collide) and granted the leaf's own declared
/// authority reduced against the reviewed ceiling — the SAME shared
/// reduction result the compiled authority graph records.
fn compile_expanded_execution_graph(
    model: &WorkflowDefinition,
    plan_identity: &str,
) -> Result<CompiledExecutionGraphV1, Vec<Diagnostic>> {
    let leaves = expanded_leaf_grants(model)?;
    let mut steps = Vec::with_capacity(leaves.len());
    for (order, (xstep, granted)) in leaves.iter().enumerate() {
        let (unit, scope_items) =
            crate::workflow::composite_expansion::leaf_operation(model, xstep)?;
        let dependencies = same_scope_dependencies(&xstep.scope_path, unit, scope_items);
        steps.push(ExecutionGraphStep {
            operation_id: xstep.fully_qualified_id(),
            key: xstep.key.clone(),
            order: order as u32,
            dependencies,
            granted: granted.clone(),
            scope_path: xstep.scope_path.clone(),
        });
    }
    Ok(CompiledExecutionGraphV1 {
        schema_version: EXECUTION_GRAPH_SCHEMA_VERSION.to_string(),
        workflow_digest: workflow_digest_of(model)?,
        plan_identity: plan_identity.to_string(),
        steps,
    })
}

/// Same-scope operational producers for one expanded leaf: sibling
/// operations whose outputs feed the leaf's inputs, identified by their
/// fully-qualified ids. Dataflow that crosses a composite boundary
/// (through a declared port) is carried by the deterministic flattened
/// order and the typed boundary crossings recorded in the expansion
/// plan — never by synthetic edges — so `dependencies` stays a
/// same-scope fact.
fn same_scope_dependencies(
    scope_path: &[String],
    unit: &OperationDefinition,
    scope_items: &[BodyItem],
) -> Vec<String> {
    let mut dependencies: Vec<String> = Vec::new();
    for input in &unit.inputs {
        for item in scope_items {
            if let BodyItem::Operation(producer) = item {
                if producer.id == unit.id {
                    continue;
                }
                if producer.outputs.iter().any(|o| o.name == input.name) {
                    let dep = crate::workflow::composite_expansion::fully_qualified_id(
                        scope_path,
                        &producer.id,
                    );
                    if !dependencies.contains(&dep) {
                        dependencies.push(dep);
                    }
                }
            }
        }
    }
    dependencies.sort();
    dependencies
}

/// Reduce one root-body unit's authority against the workflow ceiling
/// (R8b) — the atomic path.
fn reduce_unit_authority(
    workflow: &WorkflowDefinition,
    unit_index: usize,
) -> Result<AuthorityProfile, Vec<Diagnostic>> {
    let BodyItem::Operation(unit) = &workflow.body[unit_index] else {
        return Err(vec![Diagnostic::new(
            "PROJ-0001",
            "authority reduction supports atomic units only",
        )]);
    };
    reduce_leaf_authority(&workflow.authority, unit)
}

/// Reduce one leaf operation's declared authority against the workflow
/// ceiling (R8b).
///
/// Ported from the reference `reduce_authority` (soma-validate): every
/// capability, scope, and secret the unit declares must already appear in
/// the ceiling; absent dimensions are carried as `None` rather than copied
/// from the ceiling. Fail closed with `SOMA-AUTH-*` diagnostics on any
/// widening (unreachable when the caller has already passed the audit —
/// defense in depth for direct callers). Shared by the atomic path
/// (root-body units) and the expanded-composite path (#253), where the
/// ancestor-chain import restriction is enforced by the expansion, so
/// the reduced profile is exactly the leaf's effective grant.
fn reduce_leaf_authority(
    ceiling: &AuthorityProfile,
    unit: &OperationDefinition,
) -> Result<AuthorityProfile, Vec<Diagnostic>> {
    let mut diagnostics: Vec<(&'static str, String)> = Vec::new();
    let mut caps: BTreeSet<String> = BTreeSet::new();
    let mut readable: BTreeSet<String> = BTreeSet::new();
    let mut writable: BTreeSet<String> = BTreeSet::new();

    let ceiling_caps: BTreeSet<&str> = ceiling.tool_keys().into_iter().collect();
    let ceiling_read = ceiling.readable_scope_set();
    let ceiling_write = ceiling.writable_scope_set();

    for grant in &unit.authority {
        if let Some(("readable", scope)) = grant.split_once(':') {
            if scope.is_empty() || !ceiling_read.contains(scope) {
                diagnostics.push((
                    "SOMA-AUTH-0003",
                    format!(
                        "operation {} declares readable scope {:?} outside the declared authority",
                        unit.id, scope
                    ),
                ));
            } else {
                readable.insert(scope.to_string());
            }
        } else if let Some(("writable", scope)) = grant.split_once(':') {
            if scope.is_empty() || !ceiling_write.contains(scope) {
                diagnostics.push((
                    "SOMA-AUTH-0003",
                    format!(
                        "operation {} declares writable scope {:?} outside the declared authority",
                        unit.id, scope
                    ),
                ));
            } else {
                writable.insert(scope.to_string());
            }
        } else if grant.is_empty() {
            diagnostics.push((
                "SOMA-AUTH-0001",
                format!("operation {} declares an empty capability grant", unit.id),
            ));
        } else if ceiling_caps.contains(grant.as_str()) {
            caps.insert(grant.clone());
        } else {
            diagnostics.push((
                "SOMA-AUTH-0001",
                format!(
                    "operation {} declares capability {:?} outside the declared authority",
                    unit.id, grant
                ),
            ));
        }
    }

    // Tools: reflect declared capability versions from the ceiling (a
    // non-widening subset), so the reduced profile never invents versions.
    let mut tools: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for cap in &caps {
        let versions = ceiling
            .tools
            .as_ref()
            .and_then(|t| t.get(cap))
            .cloned()
            .unwrap_or_default();
        tools.insert(cap.clone(), versions);
    }

    // Secrets: a non-widening subset of the ceiling's declared secrets.
    let secrets: Vec<SecretGrant> = ceiling
        .secrets
        .iter()
        .flatten()
        .filter(|s| unit.secrets.contains(&s.name))
        .cloned()
        .collect();
    for undeclared in unit
        .secrets
        .iter()
        .filter(|n| !ceiling.secrets.iter().flatten().any(|s| &s.name == *n))
    {
        diagnostics.push((
            "SOMA-AUTH-0005",
            format!(
                "operation {} declares secret {:?} outside the declared authority",
                unit.id, undeclared
            ),
        ));
    }

    if !diagnostics.is_empty() {
        return Err(diagnostics
            .into_iter()
            .map(|(code, message)| Diagnostic::new(code, message))
            .collect());
    }

    // v1.1 units carry no per-unit network/provider/budget/mutation
    // dimensions; those stay absent (`None`) rather than inherited from
    // the ceiling — a true reduction, per the reference implementation.
    Ok(AuthorityProfile {
        execution_class: unit.execution_class,
        mutation: MutationMode::None_,
        tools: if tools.is_empty() { None } else { Some(tools) },
        readable_scopes: if readable.is_empty() {
            None
        } else {
            Some(readable.into_iter().collect())
        },
        writable_scopes: if writable.is_empty() {
            None
        } else {
            Some(writable.into_iter().collect())
        },
        network_policy: None,
        provider_policy: None,
        secrets: if secrets.is_empty() {
            None
        } else {
            Some(secrets)
        },
        escalation: unit.escalation.clone(),
        review: unit.review.clone(),
        abstention: None,
        budgets: None,
        content_restrictions: None,
    })
}

#[cfg(test)]
mod tests {
    use super::compile_expanded_execution_graph;
    use super::topological_order_body;
    use crate::workflow::soma::contracts::BodyItem;

    fn op(id: &str, inputs: &[&str], outputs: &[&str]) -> BodyItem {
        let value = serde_json::json!({
            "schemaVersion": "1.1.0",
            "version": "1.1.0",
            "id": id,
            "executionClass": "deterministic",
            "inputs": inputs.iter().map(|name| serde_json::json!({
                "name": name, "type": "Order", "acceptedOutcomes": ["Produced"]
            })).collect::<Vec<_>>(),
            "outputs": outputs.iter().map(|name| serde_json::json!({
                "name": name, "type": "Order", "emits": ["Produced"]
            })).collect::<Vec<_>>(),
            "effects": []
        });
        serde_json::from_value(value).expect("operation parses")
    }

    fn composite(id: &str, inputs: &[&str], outputs: &[&str]) -> BodyItem {
        let value = serde_json::json!({
            "schemaVersion": "1.1.0",
            "version": "1.1.0",
            "id": id,
            "inputPorts": inputs.iter().map(|name| serde_json::json!({
                "name": name, "direction": "input", "type": "Order",
                "cardinality": "single", "requiredness": "required"
            })).collect::<Vec<_>>(),
            "outputPorts": outputs.iter().map(|name| serde_json::json!({
                "name": name, "direction": "output", "type": "Order",
                "cardinality": "single", "requiredness": "required"
            })).collect::<Vec<_>>(),
            "body": []
        });
        serde_json::from_value(value).expect("composite parses")
    }

    #[test]
    fn ops_only_scope_keeps_reference_kahn_order() {
        let items = vec![
            op("a", &[], &["x"]),
            op("b", &["x"], &[]),
            op("c", &[], &[]),
        ];
        assert_eq!(topological_order_body(&items), Some(vec![0, 1, 2]));
    }

    #[test]
    fn containers_participate_through_boundary_ports() {
        // `c` is declared first but consumes what `a` produces and feeds
        // `b`: dataflow order [a, c, b] proves container channels count.
        let items = vec![
            composite("c", &["x"], &["y"]),
            op("a", &[], &["x"]),
            op("b", &["y"], &[]),
        ];
        assert_eq!(topological_order_body(&items), Some(vec![1, 0, 2]));
    }

    #[test]
    fn cycle_returns_none() {
        let items = vec![op("b", &["x"], &["y"]), composite("c", &["y"], &["x"])];
        assert_eq!(topological_order_body(&items), None);
    }

    #[test]
    fn reused_local_operation_ids_stay_distinct_through_fully_qualified_paths() {
        // The identity law (E4/I07 #253, review repair P1-1): local ids
        // are unique within their scope; the same local id in DIFFERENT
        // scopes is legal and stays distinct through fully-qualified
        // paths. This exercises the graph builder directly over such a
        // document; the public audit→permit→runtime path is proven in
        // the end-to-end conformance suite.
        let op_json = |id: &str| {
            serde_json::json!({
                "schemaVersion": "1.1.0", "version": "1.1.0", "id": id,
                "executionClass": "deterministic",
                "inputs": [], "outputs": [],
                "authority": [], "effects": [], "uses": [], "secrets": [], "context": []
            })
        };
        let composite_json = |id: &str, body: serde_json::Value| {
            serde_json::json!({
                "schemaVersion": "1.1.0", "version": "1.1.0", "id": id,
                "inputPorts": [], "outputPorts": [], "body": body
            })
        };
        let text = serde_json::json!({
            "schemaVersion": "1.1.0", "version": "1.1.0",
            "id": "wf-reuse", "name": "Reuse", "kind": "atomic",
            "inputPorts": [], "outputPorts": [],
            "body": [
                composite_json("A", serde_json::json!([op_json("x")])),
                composite_json("B", serde_json::json!([op_json("x")])),
            ],
            "authority": { "executionClass": "deterministic", "mutation": "none" }
        })
        .to_string();
        let model: crate::workflow::soma::contracts::WorkflowDefinition =
            serde_json::from_str(&text).expect("parses");
        let graph =
            compile_expanded_execution_graph(&model, "plan").expect("expanded graph compiles");
        let ids: Vec<&str> = graph
            .steps
            .iter()
            .map(|s| s.operation_id.as_str())
            .collect();
        assert_eq!(
            ids,
            ["A/x", "B/x"],
            "fully-qualified identity keeps distinct scopes distinct"
        );
        assert_ne!(
            graph.steps[0].key, graph.steps[1].key,
            "plan keys stay unique"
        );
    }
}
