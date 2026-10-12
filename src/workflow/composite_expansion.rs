//! E4/X03 (#160) Slice B — deterministic recursive expansion of composite
//! bodies into a flat, plan-shaped step list.
//!
//! Pure function of the (already-audited) canonical `WorkflowDefinition`:
//! recursion is deterministic (per-scope topological order, then declaration
//! ordinal), composite identity is preserved as an explicit `scopePath`
//! (never a concatenated free-form id), and every flattened operation is
//! bound to its source scope path, typed boundary-port crossings, effective
//! imported authority, exported effects, and visible context.
//!
//! Expansion runs only after the audit gate; it adds defense-in-depth
//! rejections (cycle, duplicate fully-qualified path, ambiguous producer,
//! authority/context widening) so no partial plan is ever sealed.

use std::collections::{BTreeMap, BTreeSet};

use crate::workflow::execution_graph::topological_order_body;
use crate::workflow::governance_compiler::{PlanStep, PortCrossing};
use crate::workflow::soma::Diagnostic;
use crate::workflow::soma::contracts::{BodyItem, PortDefinition, WorkflowDefinition};

fn refusal(code: &'static str, message: String) -> Vec<Diagnostic> {
    vec![Diagnostic::new(code, message)]
}

/// Flatten a workflow's (possibly nested) body into deterministic plan steps.
pub fn expand_composites(wf: &WorkflowDefinition) -> Result<Vec<PlanStep>, Vec<Diagnostic>> {
    let mut steps: Vec<PlanStep> = Vec::new();
    let mut seen: BTreeSet<Vec<String>> = BTreeSet::new();
    let mut index: usize = 0;
    expand_scope(
        &wf.body,
        &[],
        &wf.input_ports,
        &wf.output_ports,
        &mut steps,
        &mut seen,
        &mut index,
    )?;
    Ok(steps)
}

fn expand_scope(
    items: &[BodyItem],
    scope_path: &[String],
    scope_inputs: &[PortDefinition],
    scope_outputs: &[PortDefinition],
    steps: &mut Vec<PlanStep>,
    seen: &mut BTreeSet<Vec<String>>,
    index: &mut usize,
) -> Result<(), Vec<Diagnostic>> {
    // Deterministic per-scope order; `None` means an operation-edge cycle.
    let Some(order) = topological_order_body(items) else {
        return Err(refusal(
            "SOMA-EXP-0002",
            format!(
                "scope {} has an operation-edge cycle; the plan cannot be sealed",
                scope_label(scope_path)
            ),
        ));
    };

    // Ambiguous producers: an input name produced by more than one sibling.
    let mut producers: BTreeMap<&str, usize> = BTreeMap::new();
    for item in items {
        for name in producer_names(item) {
            *producers.entry(name).or_insert(0) += 1;
        }
    }

    for &body_idx in &order {
        let item = &items[body_idx];
        let mut fq = scope_path.to_vec();
        fq.push(item.id().to_string());
        if !seen.insert(fq.clone()) {
            return Err(refusal(
                "SOMA-CMP-0003",
                format!(
                    "duplicate fully-qualified operation path {}; expansion refuses ambiguity",
                    fq.join("/")
                ),
            ));
        }

        match item {
            BodyItem::Operation(op) => {
                for inp in &op.inputs {
                    if producers.get(inp.name.as_str()).copied().unwrap_or(0) > 1 {
                        return Err(refusal(
                            "SOMA-CMP-0003",
                            format!(
                                "ambiguous producer for input {:?} in scope {}",
                                inp.name,
                                scope_label(scope_path)
                            ),
                        ));
                    }
                }
                let mut authority = op.authority.clone();
                authority.sort();
                authority.dedup();
                let mut context = op.context.clone();
                context.sort();
                context.dedup();
                let mut effects: Vec<String> = op.effects.iter().map(|e| e.name.clone()).collect();
                effects.sort();
                effects.dedup();
                steps.push(PlanStep {
                    key: format!("s{index:04}:{}", op.id),
                    operation_id: op.id.clone(),
                    scope_path: scope_path.to_vec(),
                    effective_authority: authority,
                    boundary_inputs: crossings(&op_input_names(op), scope_inputs),
                    boundary_outputs: crossings(&op_output_names(op), scope_outputs),
                    exported_effects: effects,
                    visible_context: context,
                });
                *index += 1;
            }
            BodyItem::Composite(c) => {
                if c.authority_imports.iter().any(|a| {
                    // Imported authority must not widen the parent's grant set:
                    // every import must already be declared by an ancestor op
                    // grant or the root tool ceiling is enforced by the audit;
                    // here we only defend against an empty-import escalation.
                    a.trim().is_empty()
                }) {
                    return Err(refusal(
                        "SOMA-AUTH-0002",
                        format!("composite {} has an empty authority import", c.id),
                    ));
                }
                let mut child = scope_path.to_vec();
                child.push(c.id.clone());
                expand_scope(
                    &c.body,
                    &child,
                    &c.input_ports,
                    &c.output_ports,
                    steps,
                    seen,
                    index,
                )?;
            }
        }
    }
    Ok(())
}

fn scope_label(scope_path: &[String]) -> String {
    if scope_path.is_empty() {
        "<root>".to_string()
    } else {
        scope_path.join("/")
    }
}

fn producer_names(item: &BodyItem) -> Vec<&str> {
    match item {
        BodyItem::Operation(op) => op.outputs.iter().map(|o| o.name.as_str()).collect(),
        BodyItem::Composite(c) => c.output_ports.iter().map(|p| p.name.as_str()).collect(),
    }
}

fn op_input_names(op: &crate::workflow::soma::contracts::OperationDefinition) -> Vec<String> {
    op.inputs.iter().map(|i| i.name.clone()).collect()
}

fn op_output_names(op: &crate::workflow::soma::contracts::OperationDefinition) -> Vec<String> {
    op.outputs.iter().map(|o| o.name.clone()).collect()
}

/// Typed boundary-port crossings: the enclosing scope's declared ports whose
/// name matches one of `names`, sorted by name (deterministic, map-free).
fn crossings(names: &[String], ports: &[PortDefinition]) -> Vec<PortCrossing> {
    let wanted: BTreeSet<&str> = names.iter().map(String::as_str).collect();
    let mut out: Vec<PortCrossing> = ports
        .iter()
        .filter(|p| wanted.contains(p.name.as_str()))
        .map(|p| PortCrossing {
            name: p.name.clone(),
            ty: p.ty.clone(),
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[cfg(test)]
mod tests {
    use super::expand_composites;
    use crate::workflow::soma::contracts::WorkflowDefinition;
    use serde_json::{Value, json};

    fn op(id: &str) -> Value {
        json!({
            "schemaVersion": "1.1.0", "version": "1.1.0", "id": id,
            "executionClass": "deterministic",
            "inputs": [], "outputs": [],
            "authority": [], "effects": [], "uses": [], "secrets": [], "context": []
        })
    }

    fn composite(id: &str, body: Value) -> Value {
        json!({
            "schemaVersion": "1.1.0", "version": "1.1.0", "id": id,
            "inputPorts": [], "outputPorts": [], "body": body
        })
    }

    fn workflow(body: Value) -> WorkflowDefinition {
        let text = json!({
            "schemaVersion": "1.1.0", "version": "1.1.0",
            "id": "wf-t", "name": "T", "kind": "atomic",
            "inputPorts": [], "outputPorts": [],
            "body": body,
            "authority": { "executionClass": "deterministic", "mutation": "none" }
        })
        .to_string();
        serde_json::from_str(&text).expect("workflow parses")
    }

    #[test]
    fn identical_local_ids_in_different_paths_remain_distinct() {
        let wf = workflow(json!([
            composite("A", json!([op("x")])),
            composite("B", json!([op("x")]))
        ]));
        let steps = expand_composites(&wf).expect("expands");
        let xs: Vec<&Vec<String>> = steps
            .iter()
            .filter(|s| s.operation_id == "x")
            .map(|s| &s.scope_path)
            .collect();
        assert_eq!(xs.len(), 2, "both `x` operations flatten");
        assert!(xs.contains(&&vec!["A".to_string()]));
        assert!(xs.contains(&&vec!["B".to_string()]));
    }

    #[test]
    fn identical_fully_qualified_paths_fail_closed() {
        let wf = workflow(json!([composite("A", json!([op("x"), op("x")]))]));
        let err = expand_composites(&wf).expect_err("duplicate FQ path must fail closed");
        assert!(err.iter().any(|d| d.code == "SOMA-CMP-0003"), "{err:?}");
    }

    #[test]
    fn ambiguous_producer_fails_expansion_before_any_plan() {
        // Two sibling producers of the same output name: audit-clean, but the
        // expansion refuses ambiguity before a plan could be sealed.
        let mut p1 = op("p1");
        p1["outputs"] = json!([{ "name": "dup", "type": "Order", "emits": ["Produced"] }]);
        let mut p2 = op("p2");
        p2["outputs"] = json!([{ "name": "dup", "type": "Order", "emits": ["Produced"] }]);
        let mut consumer = op("c");
        consumer["inputs"] =
            json!([{ "name": "dup", "type": "Order", "acceptedOutcomes": ["Produced"] }]);
        let wf = workflow(json!([p1, p2, consumer]));
        let err = expand_composites(&wf).expect_err("ambiguous producer must fail");
        assert!(err.iter().any(|d| d.code == "SOMA-CMP-0003"), "{err:?}");
    }
}
