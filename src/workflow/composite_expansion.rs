//! E4/X03 (#160) Slice B — deterministic recursive expansion of composite
//! bodies into a flat, plan-shaped step list.
//!
//! Pure function of the (already-audited) canonical `WorkflowDefinition`:
//! recursion is deterministic (per-scope topological order, then declaration
//! ordinal), composite identity is preserved as an explicit `scopePath`
//! (never a concatenated free-form id), and every flattened operation is
//! bound to its source scope path, typed boundary-port crossings derived
//! from real producer→consumer edges, and effective imported authority /
//! exported effects / visible context computed across the complete ancestor
//! chain (each ancestor can restrict a descendant, never widen it).
//!
//! Identity: an operation is identified by `scopePath + operationId`.
//! Local ids are unique within their enclosing scope (the audit gate
//! refuses duplicate siblings as `SOMA-EXP-0003`), while the same local
//! id in a DIFFERENT scope is legal and remains distinct through the
//! fully-qualified scope path (E4/I07 #253: reused local operation ids
//! execute as distinct identities, e.g. `A/x` and `B/x`). The
//! fully-qualified-path duplicate check below is defense in depth for
//! direct callers (same-scope duplicates are refused by the audit
//! first; distinct scopes cannot produce a duplicate fully-qualified
//! path).
//!
//! Expansion runs only after the audit gate. Checks the audit gate does
//! not perform (ambiguous producer, sibling type mismatch, ancestor-chain
//! authority/effects/context widening) run here for every composite
//! workflow; cycle and duplicate fully-qualified-path rejections are
//! defense in depth — the audit gate refuses those inputs first
//! (SOMA-EXP-0002 / SOMA-EXP-0003). The expansion metadata lives in the
//! separately versioned [`ExpansionPlanV1`] — the published v1.1
//! `ExecutionPlan` step shape (`key`/`operationId`) is never extended.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::workflow::execution_graph::topological_order_body;
use crate::workflow::soma::Diagnostic;
use crate::workflow::soma::contracts::{
    BodyItem, EffectExport, OperationDefinition, PortDefinition, WorkflowDefinition,
};

/// Format identifier of the Lite composite expansion plan (separately
/// versioned from the published SOMA v1.1 `ExecutionPlan`).
pub const EXPANSION_PLAN_FORMAT: &str = "lite-composite-expansion-plan";

/// Version of the Lite composite expansion plan format.
pub const EXPANSION_PLAN_FORMAT_VERSION: &str = "1.0.0";

/// The Lite composite expansion plan: the flattened step list plus the
/// binding to the source workflow digest. This is the only place composite
/// expansion metadata lives — the sealed v1.1 plan keeps its published
/// `{key, operationId}` step shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExpansionPlanV1 {
    pub format: String,
    pub format_version: String,
    /// Canonical digest of the source workflow (same rule as
    /// `SOMA-CMP-0004`): binds the expansion to the document it was
    /// derived from.
    pub workflow_digest: String,
    pub steps: Vec<ExpansionStep>,
}

/// One flattened operation plus its expansion metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExpansionStep {
    /// Published v1.1 step key: `s{index:04}:{operationId}` over the
    /// flattened order — unique within the plan.
    pub key: String,
    /// The workflow body operation this step executes.
    pub operation_id: String,
    /// Enclosing composite scope path (empty for root-level operations).
    pub scope_path: Vec<String>,
    /// Effective imported authority for this operation (sorted, deduped),
    /// computed across the complete ancestor chain.
    pub effective_authority: Vec<String>,
    /// Typed boundary-port crossings into the enclosing scope (sorted).
    pub boundary_inputs: Vec<BoundaryCrossing>,
    /// Typed boundary-port crossings out of the enclosing scope (sorted).
    pub boundary_outputs: Vec<BoundaryCrossing>,
    /// Effects this operation exports (sorted, deduped), restricted by the
    /// ancestor chain's effect exports.
    pub exported_effects: Vec<String>,
    /// Context visible to this operation inside its scope (sorted, deduped),
    /// restricted by the ancestor chain's disclosures.
    pub visible_context: Vec<String>,
}

/// Direction of a typed boundary-port crossing relative to the scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CrossingDirection {
    /// Data crossing into the scope through a declared input port.
    In,
    /// Data crossing out of the scope through a declared output port.
    Out,
}

/// A typed boundary-port crossing: a wanted (non-internal) name that
/// matches a declared port of the enclosing scope, with the scope path it
/// crossed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoundaryCrossing {
    pub name: String,
    pub ty: String,
    pub direction: CrossingDirection,
    /// The scope the port was declared on (empty for root).
    pub crossed_scope: Vec<String>,
}

impl ExpansionStep {
    /// The fully-qualified runtime identity of this leaf: the enclosing
    /// scope path joined by `/`, then the local operation id. Root-scope
    /// leaves keep their bare local id, so atomic-workflow identity is
    /// unchanged. This is the identity the governed runtime uses across
    /// execution, routing, evidence, suspension, checkpoint, and resume
    /// (#253): reused local operation ids in different scopes can never
    /// collide.
    pub fn fully_qualified_id(&self) -> String {
        fully_qualified_id(&self.scope_path, &self.operation_id)
    }
}

/// The fully-qualified runtime id of a leaf at `scope_path` with local
/// `operation_id` (empty scope path ⇒ the bare local id).
pub fn fully_qualified_id(scope_path: &[String], operation_id: &str) -> String {
    if scope_path.is_empty() {
        operation_id.to_string()
    } else {
        format!("{}/{}", scope_path.join("/"), operation_id)
    }
}

/// Resolve the workflow-body operation a flattened expansion step
/// executes, plus the items of its enclosing scope, by walking the
/// scope path through the composites. Fail-closed (`PROJ-0001`) if the
/// path does not resolve — unreachable for expansions produced from an
/// audited document; defense in depth for direct callers.
pub(crate) fn leaf_operation<'a>(
    wf: &'a WorkflowDefinition,
    step: &ExpansionStep,
) -> Result<(&'a OperationDefinition, &'a [BodyItem]), Vec<Diagnostic>> {
    let mut items: &'a [BodyItem] = &wf.body;
    for scope in &step.scope_path {
        let Some(BodyItem::Composite(c)) = items.iter().find(|i| i.id() == scope.as_str()) else {
            return Err(vec![Diagnostic::new(
                "PROJ-0001",
                format!(
                    "expanded step {} references unknown scope {scope:?}",
                    step.fully_qualified_id()
                ),
            )]);
        };
        items = &c.body;
    }
    let Some(BodyItem::Operation(op)) = items.iter().find(|i| i.id() == step.operation_id.as_str())
    else {
        return Err(vec![Diagnostic::new(
            "PROJ-0001",
            format!(
                "expanded step {} references an unknown operation",
                step.fully_qualified_id()
            ),
        )]);
    };
    Ok((op, items))
}

/// One level of the ancestor chain: every level can restrict its
/// descendants' effective authority / exported effects / visible context.
struct ChainLevel<'a> {
    label: String,
    authority_imports: &'a [String],
    /// Root level is armed only for root-kind composites (`imports_armed`),
    /// mirroring the audit's AUTH-0002 semantics.
    imports_armed: bool,
    effect_exports: Option<&'a [EffectExport]>,
    discloses: Option<&'a [String]>,
}

/// Root-only ceilings (the audit's root environment, reduced to what the
/// expansion chain check needs).
struct Ceilings<'a> {
    tool_keys: BTreeSet<&'a str>,
    readable: BTreeSet<&'a str>,
    writable: BTreeSet<&'a str>,
}

fn refusal(code: &'static str, message: String) -> Vec<Diagnostic> {
    vec![Diagnostic::new(code, message)]
}

/// Flatten a workflow's (possibly nested) body into deterministic
/// [`ExpansionStep`]s, enforcing ancestor-chain restrictions and typed
/// boundary-port crossings.
pub fn expand_composites(wf: &WorkflowDefinition) -> Result<Vec<ExpansionStep>, Vec<Diagnostic>> {
    let ceilings = Ceilings {
        tool_keys: wf.authority.tool_keys().into_iter().collect(),
        readable: wf.authority.readable_scope_set(),
        writable: wf.authority.writable_scope_set(),
    };
    let mut chain = vec![ChainLevel {
        label: "<root>".to_string(),
        authority_imports: &wf.authority_imports,
        imports_armed: wf.is_composite(),
        effect_exports: wf.effect_exports.as_deref(),
        discloses: wf.context.as_ref().and_then(|c| c.discloses.as_deref()),
    }];
    let mut steps: Vec<ExpansionStep> = Vec::new();
    let mut seen: BTreeSet<Vec<String>> = BTreeSet::new();
    let mut index: usize = 0;
    expand_scope(
        &wf.body,
        &[],
        &wf.input_ports,
        &wf.output_ports,
        &ceilings,
        &mut chain,
        &mut steps,
        &mut seen,
        &mut index,
    )?;
    Ok(steps)
}

#[allow(clippy::too_many_arguments)]
fn expand_scope<'a>(
    items: &'a [BodyItem],
    scope_path: &[String],
    scope_inputs: &'a [PortDefinition],
    scope_outputs: &'a [PortDefinition],
    ceilings: &Ceilings<'a>,
    chain: &mut Vec<ChainLevel<'a>>,
    steps: &mut Vec<ExpansionStep>,
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

    // Per-scope producers (with declared types), consumers, and their name
    // sets — shared by the binding checks and the crossing derivation.
    let mut producer_types: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut consumers: BTreeSet<&str> = BTreeSet::new();
    for item in items {
        match item {
            BodyItem::Operation(op) => {
                for o in &op.outputs {
                    producer_types
                        .entry(o.name.as_str())
                        .or_default()
                        .push(o.ty.as_str());
                }
                consumers.extend(op.inputs.iter().map(|i| i.name.as_str()));
            }
            BodyItem::Composite(c) => {
                for p in &c.output_ports {
                    producer_types
                        .entry(p.name.as_str())
                        .or_default()
                        .push(p.ty.as_str());
                }
                consumers.extend(c.input_ports.iter().map(|p| p.name.as_str()));
            }
        }
    }
    let producer_names: BTreeSet<&str> = producer_types.keys().copied().collect();

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
                    check_input_binding(&inp.name, &inp.ty, &producer_types, scope_path)?;
                }
                check_chain(op, ceilings, chain)?;

                let mut authority = op.authority.clone();
                authority.sort();
                authority.dedup();
                let mut context = op.context.clone();
                context.sort();
                context.dedup();
                let mut effects: Vec<String> = op.effects.iter().map(|e| e.name.clone()).collect();
                effects.sort();
                effects.dedup();

                let input_names: Vec<String> = op.inputs.iter().map(|i| i.name.clone()).collect();
                let output_names: Vec<String> = op.outputs.iter().map(|o| o.name.clone()).collect();

                steps.push(ExpansionStep {
                    key: format!("s{index:04}:{}", op.id),
                    operation_id: op.id.clone(),
                    scope_path: scope_path.to_vec(),
                    effective_authority: authority,
                    boundary_inputs: crossings(
                        &input_names,
                        scope_inputs,
                        &producer_names,
                        CrossingDirection::In,
                        scope_path,
                    ),
                    boundary_outputs: crossings(
                        &output_names,
                        scope_outputs,
                        &consumers,
                        CrossingDirection::Out,
                        scope_path,
                    ),
                    exported_effects: effects,
                    visible_context: context,
                });
                *index += 1;
            }
            BodyItem::Composite(c) => {
                for p in &c.input_ports {
                    check_input_binding(&p.name, &p.ty, &producer_types, scope_path)?;
                }
                if c.authority_imports.iter().any(|a| a.trim().is_empty()) {
                    return Err(refusal(
                        "SOMA-AUTH-0002",
                        format!("composite {} has an empty authority import", c.id),
                    ));
                }
                let mut child = scope_path.to_vec();
                child.push(c.id.clone());
                chain.push(ChainLevel {
                    label: scope_label(&child),
                    authority_imports: &c.authority_imports,
                    imports_armed: true,
                    effect_exports: c.effect_exports.as_deref(),
                    discloses: c.context.as_ref().and_then(|x| x.discloses.as_deref()),
                });
                expand_scope(
                    &c.body,
                    &child,
                    &c.input_ports,
                    &c.output_ports,
                    ceilings,
                    chain,
                    steps,
                    seen,
                    index,
                )?;
                chain.pop();
            }
        }
    }
    Ok(())
}

/// Enforce the audit's root ceilings plus every armed ancestor level's
/// imports / effect exports / disclosures for one operation; on success
/// return its effective (sorted, deduped) declarations.
fn check_chain(
    op: &OperationDefinition,
    ceilings: &Ceilings<'_>,
    chain: &[ChainLevel<'_>],
) -> Result<(), Vec<Diagnostic>> {
    for grant in &op.authority {
        match grant.split_once(':') {
            Some(("readable", scope)) => {
                if !ceilings.readable.contains(scope) {
                    return Err(refusal(
                        "SOMA-AUTH-0003",
                        format!(
                            "readable scope {scope:?} not declared for operation {}",
                            op.id
                        ),
                    ));
                }
            }
            Some(("writable", scope)) => {
                if !ceilings.writable.contains(scope) {
                    return Err(refusal(
                        "SOMA-AUTH-0003",
                        format!(
                            "writable scope {scope:?} not declared for operation {}",
                            op.id
                        ),
                    ));
                }
            }
            _ => {
                if !ceilings.tool_keys.contains(grant.as_str()) {
                    return Err(refusal(
                        "SOMA-AUTH-0001",
                        format!(
                            "capability {grant:?} used by operation {} is not granted",
                            op.id
                        ),
                    ));
                }
            }
        }
        // Ancestor chain: every armed level must import the grant
        // verbatim (including scope-prefixed grants; the prefix exemption
        // exists only in AUTH-0001).
        for level in chain {
            if level.imports_armed
                && !level
                    .authority_imports
                    .iter()
                    .any(|a| a == grant || a.as_str() == grant.as_str())
            {
                return Err(refusal(
                    "SOMA-AUTH-0002",
                    format!(
                        "operation {} exceeds imported authority of scope {}: {grant:?} not imported",
                        op.id, level.label
                    ),
                ));
            }
        }
    }
    for eff in &op.effects {
        for level in chain {
            if let Some(exports) = level.effect_exports
                && !exports.iter().any(|e| e.name == eff.name)
            {
                return Err(refusal(
                    "SOMA-EXP-0007",
                    format!(
                        "undeclared effect crossing: {} is not exported by scope {}",
                        eff.name, level.label
                    ),
                ));
            }
        }
    }
    for c in &op.context {
        for level in chain {
            if let Some(discloses) = level.discloses
                && !discloses.iter().any(|d| d == c)
            {
                return Err(refusal(
                    "SOMA-EXP-0007",
                    format!(
                        "undeclared context crossing: {c:?} is not disclosed by scope {}",
                        level.label
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Same-scope binding check for one input name: boundary-fed (no sibling
/// producer) is allowed; one producer must type-match; more than one is
/// ambiguous. Applied to operation inputs and composite input ports.
fn check_input_binding(
    name: &str,
    ty: &str,
    producer_types: &BTreeMap<&str, Vec<&str>>,
    scope_path: &[String],
) -> Result<(), Vec<Diagnostic>> {
    let Some(decls) = producer_types.get(name) else {
        return Ok(());
    };
    if decls.len() > 1 {
        return Err(refusal(
            "SOMA-CMP-0003",
            format!(
                "ambiguous producer for input {name:?} in scope {}",
                scope_label(scope_path)
            ),
        ));
    }
    if decls[0] != ty {
        return Err(refusal(
            "SOMA-CMP-0005",
            format!(
                "sibling type mismatch for input {name:?} in scope {}: producer declares {:?}, consumer expects {:?}",
                scope_label(scope_path),
                decls[0],
                ty
            ),
        ));
    }
    Ok(())
}

/// Typed boundary-port crossings: wanted (non-internal) names that match
/// declared ports of the enclosing scope, sorted by name. An internal edge
/// sharing a boundary-port name must not masquerade as a crossing, so the
/// per-scope producer / consumer name sets filter first.
fn crossings(
    names: &[String],
    ports: &[PortDefinition],
    internal: &BTreeSet<&str>,
    direction: CrossingDirection,
    scope_path: &[String],
) -> Vec<BoundaryCrossing> {
    let wanted: BTreeSet<&str> = names
        .iter()
        .map(String::as_str)
        .filter(|n| !internal.contains(*n))
        .collect();
    let mut out: Vec<BoundaryCrossing> = ports
        .iter()
        .filter(|p| wanted.contains(p.name.as_str()))
        .map(|p| BoundaryCrossing {
            name: p.name.clone(),
            ty: p.ty.clone(),
            direction,
            crossed_scope: scope_path.to_vec(),
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn scope_label(scope_path: &[String]) -> String {
    if scope_path.is_empty() {
        "<root>".to_string()
    } else {
        scope_path.join("/")
    }
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
    fn identical_fully_qualified_paths_fail_closed() {
        // Defense in depth: the audit gate (SOMA-EXP-0003) refuses
        // document-wide duplicate ids before expansion runs, so this
        // path is unreachable through `compile_workflow_text`; it guards
        // direct callers of `expand_composites`.
        let wf = workflow(json!([composite("A", json!([op("x"), op("x")]))]));
        let err = expand_composites(&wf).expect_err("duplicate FQ path must fail closed");
        assert!(err.iter().any(|d| d.code == "SOMA-CMP-0003"), "{err:?}");
    }

    #[test]
    fn operation_edge_cycle_fails_expansion() {
        // Defense in depth: `compile_workflow_text` never reaches expansion
        // with a cycle — the audit gate refuses SOMA-EXP-0002 first; this
        // exercises the expansion-layer refusal directly.
        let mut a = op("a");
        a["inputs"] = json!([{ "name": "n", "type": "Order", "acceptedOutcomes": ["Produced"] }]);
        a["outputs"] = json!([{ "name": "m", "type": "Order", "emits": ["Produced"] }]);
        let mut b = op("b");
        b["inputs"] = json!([{ "name": "m", "type": "Order", "acceptedOutcomes": ["Produced"] }]);
        b["outputs"] = json!([{ "name": "n", "type": "Order", "emits": ["Produced"] }]);
        let wf = workflow(json!([a, b]));
        let err = expand_composites(&wf).expect_err("cycle must fail closed");
        assert!(err.iter().any(|d| d.code == "SOMA-EXP-0002"), "{err:?}");
    }
}
