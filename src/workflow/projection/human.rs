//! Deterministic, non-normative human plan projection.
//!
//! Stability contract: golden fixtures in `tests/projection_golden/` pin the
//! grammar (spec §6). Body order is the compiler's topological order
//! (SPEC_002 / declaration ordinal) — never an ad-hoc sort.

use crate::workflow::execution_graph::topological_order;
use crate::workflow::governance_compiler::workflow_digest_of;
use crate::workflow::soma::contracts::{OperationDefinition, WorkflowDefinition};
use crate::workflow::soma::{Diagnostic, canonical::sha256_hex};

use super::envelope::{PROJECTION_VERSION_V1, VersionedProjectionEnvelope};
use super::validated_source;

/// Render the non-normative plan body (everything except the Disclosure
/// section, which depends on the disclosure policy and is appended later).
pub(crate) fn render_plan_body(
    wf: &WorkflowDefinition,
    source_digest: &str,
) -> Result<String, Vec<Diagnostic>> {
    let order = topological_order(wf).ok_or_else(|| {
        vec![Diagnostic::new(
            "PROJ-0001",
            "workflow body has an operation-edge cycle; projection refused",
        )]
    })?;
    let mut out = String::new();
    out.push_str(&format!(
        "NON-NORMATIVE VIEW — derived from source digest {source_digest}; not an executable contract.\n"
    ));
    out.push_str(&format!(
        "# Workflow: {} v{} ({})\n",
        wf.id, wf.version, wf.name
    ));
    out.push_str(&format!(
        "Schema: {}  Kind: {}\n",
        wf.schema_version,
        wf.kind.map(|k| k.as_str()).unwrap_or("atomic")
    ));
    if let Some(purpose) = &wf.purpose {
        out.push_str(&format!("Purpose: {purpose}\n"));
    }
    if let Some(ctx) = &wf.context {
        out.push_str(&format!("Discloses: {}\n", json_or_none(&ctx.discloses)));
        out.push_str(&format!("Requires: {}\n", json_or_none(&ctx.requires)));
    }
    out.push('\n');

    out.push_str("## Authority Ceiling\n");
    let a = &wf.authority;
    out.push_str(&format!("ExecutionClass: {}\n", a.execution_class.as_str()));
    out.push_str(&format!("Mutation: {}\n", a.mutation.as_str()));
    out.push_str(&format!(
        "Readable scopes: {}\n",
        json_or_none(&a.readable_scopes)
    ));
    out.push_str(&format!(
        "Writable scopes: {}\n",
        json_or_none(&a.writable_scopes)
    ));
    out.push_str(&format!("Tools: {}\n", json_or_none(&a.tools)));
    out.push_str(&format!("Network: {}\n", json_opt(&a.network_policy)));
    out.push_str(&format!("Provider: {}\n", json_opt(&a.provider_policy)));
    out.push_str(&format!("Secrets: {}\n", json_opt(&a.secrets)));
    out.push_str(&format!("Escalation: {}\n", json_opt(&a.escalation)));
    out.push_str(&format!("Review: {}\n", json_opt(&a.review)));
    out.push_str(&format!("Abstention: {}\n", json_opt(&a.abstention)));
    out.push_str(&format!("Budgets: {}\n", json_opt(&a.budgets)));
    out.push_str(&format!(
        "Content restrictions: {}\n",
        json_opt(&a.content_restrictions)
    ));
    out.push('\n');

    out.push_str("## Body (topological order)\n");
    let marker = if wf.is_composite() {
        "COMPOSITE"
    } else {
        "ATOMIC"
    };
    for (pos, &idx) in order.iter().enumerate() {
        let unit = &wf.body[idx];
        out.push_str(&format!("### s{pos:04}:{} [{marker}]\n", unit.id));
        out.push_str(&render_unit(unit));
    }
    out.push('\n');

    out.push_str("## Constraints\n");
    if wf.constraints.is_empty() {
        out.push_str("  none\n");
    } else {
        for c in &wf.constraints {
            out.push_str(&format!(
                "  {}: {} ({}, {}, eval={})\n",
                c.id,
                c.predicate,
                c.violation_category,
                c.kind.as_str(),
                c.evaluation_point.as_deref().unwrap_or("none")
            ));
        }
    }
    out.push('\n');

    out.push_str("## Evidence References\n");
    if wf.evidence.is_empty() {
        out.push_str("  none\n");
    } else {
        for e in &wf.evidence {
            out.push_str(&format!(
                "  {}: event={} artifact={} kind={} by={} at={}\n",
                e.id,
                e.event_digest.as_str(),
                e.artifact_digest.as_str(),
                e.artifact_kind,
                e.produced_by,
                e.produced_at.as_deref().unwrap_or("-")
            ));
        }
    }
    out.push('\n');
    Ok(out)
}

fn render_unit(unit: &OperationDefinition) -> String {
    let mut s = String::new();
    let inputs = unit
        .inputs
        .iter()
        .map(|i| {
            let mut line = format!("{}: {}", i.name, i.ty);
            if !i.accepted_outcomes.is_empty() {
                let outs: Vec<&str> = i.accepted_outcomes.iter().map(|o| o.as_str()).collect();
                line.push('[');
                line.push_str(&outs.join(","));
                line.push(']');
            }
            line
        })
        .collect::<Vec<_>>()
        .join(", ");
    s.push_str(&format!(
        "  Inputs: {}\n",
        if inputs.is_empty() { "none" } else { &inputs }
    ));
    let outputs = unit
        .outputs
        .iter()
        .map(|o| {
            let mut line = format!("{}: {}", o.name, o.ty);
            if let Some(emits) = &o.emits
                && !emits.is_empty()
            {
                let vals: Vec<&str> = emits.iter().map(|e| e.as_str()).collect();
                line.push('[');
                line.push_str(&vals.join(","));
                line.push(']');
            }
            line
        })
        .collect::<Vec<_>>()
        .join(", ");
    s.push_str(&format!(
        "  Outputs: {}\n",
        if outputs.is_empty() { "none" } else { &outputs }
    ));
    s.push_str(&format!("  Authority: {}\n", list_or_none(&unit.authority)));
    let effects = unit
        .effects
        .iter()
        .map(|e| {
            let mut line = e.name.clone();
            if e.review == Some(true) {
                line.push_str(" [review]");
            }
            if e.irreversible == Some(true) {
                line.push_str(" [irreversible]");
            }
            line
        })
        .collect::<Vec<_>>()
        .join(", ");
    s.push_str(&format!(
        "  Effects: {}\n",
        if effects.is_empty() { "none" } else { &effects }
    ));
    s.push_str(&format!("  Uses: {}\n", list_or_none(&unit.uses)));
    s.push_str(&format!("  Secrets: {}\n", list_or_none(&unit.secrets)));
    s.push_str(&format!("  Context: {}\n", list_or_none(&unit.context)));
    s
}

fn json_opt<T: serde::Serialize>(v: &Option<T>) -> String {
    match v {
        Some(v) => serde_json::to_string(v).unwrap_or_else(|_| "none".to_string()),
        None => "none".to_string(),
    }
}

fn json_or_none<T: serde::Serialize>(v: &Option<T>) -> String {
    json_opt(v)
}

fn list_or_none(v: &[String]) -> String {
    if v.is_empty() {
        "none".to_string()
    } else {
        let quoted: Vec<String> = v.iter().map(|s| format!("{s:?}")).collect();
        format!("[{}]", quoted.join(", "))
    }
}

/// Non-normative human plan projection of the validated AST. Disclosure
/// counts are always rendered; Slice 1 without a policy records zeros.
pub fn project_human_plan(
    wf: &WorkflowDefinition,
) -> Result<VersionedProjectionEnvelope<String>, Vec<Diagnostic>> {
    validated_source(wf)?;
    let source_digest = workflow_digest_of(wf)?;
    let mut text = render_plan_body(wf, &source_digest)?;
    text.push_str("## Disclosure\n");
    text.push_str("  Redactions: 0 Omissions: 0\n");
    let projection_digest = sha256_hex(text.as_bytes());
    Ok(VersionedProjectionEnvelope {
        projection_version: PROJECTION_VERSION_V1.to_string(),
        schema_version: wf.schema_version.clone(),
        source_digest,
        projection_digest,
        payload: text,
    })
}
