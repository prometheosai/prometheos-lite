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
use crate::workflow::soma::contracts::{AuthorityProfile, SecretGrant, WorkflowDefinition};
use crate::workflow::soma::types::MutationMode;

/// Schema version of this Lite-owned structure (semantically versioned;
/// never confused with the SOMA plan's `schemaVersion`).
pub const EXECUTION_GRAPH_SCHEMA_VERSION: &str = "lite.execution-graph.v1";

/// Deterministic topological order over workflow-body indices.
///
/// An edge `producer -> consumer` exists when a consumer input name matches
/// a producer output name (the reference compiler's dataflow rule). Returns
/// `None` when the graph contains a cycle — the reference compiler
/// `debug_assert`s acyclicity because its audit gate guarantees it; Lite
/// fails closed with a diagnostic instead of trusting that invariant at
/// every call site.
pub(crate) fn topological_order(wf: &WorkflowDefinition) -> Option<Vec<usize>> {
    let n = wf.body.len();
    let mut producers: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, u) in wf.body.iter().enumerate() {
        for o in &u.outputs {
            producers.entry(o.name.as_str()).or_default().push(i);
        }
    }
    let mut indegree = vec![0usize; n];
    let mut edges = vec![Vec::<usize>::new(); n];
    for (i, u) in wf.body.iter().enumerate() {
        let mut deps: Vec<usize> = Vec::new();
        for inp in &u.inputs {
            if let Some(producers_of_name) = producers.get(inp.name.as_str()) {
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
}

/// Compile the Lite execution graph for a workflow.
///
/// Callers must chain this after `compile_workflow_text` (the audit gate):
/// this entry point re-parses and enforces dataflow well-formedness plus
/// fail-closed authority reduction, but it is not a substitute for the
/// full audit. `plan_identity` is the sealed plan's
/// `canonicalization.sha256` from that same compilation.
pub fn compile_execution_graph(
    workflow_text: &str,
    plan_identity: &str,
) -> Result<CompiledExecutionGraphV1, Vec<Diagnostic>> {
    let model: WorkflowDefinition = serde_json::from_str(workflow_text)
        .map_err(|e| vec![Diagnostic::new("SOMA-CMP-0003", format!("schema violation: {e}"))])?;
    let workflow_digest = workflow_digest_of(&model)?;
    let Some(order) = topological_order(&model) else {
        return Err(vec![Diagnostic::new(
            "SOMA-EXP-0002",
            "workflow body has an operation-edge cycle; no topological sort",
        )]);
    };

    let mut producers: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, unit) in model.body.iter().enumerate() {
        for output in &unit.outputs {
            producers.entry(output.name.as_str()).or_default().push(i);
        }
    }

    let mut steps = Vec::with_capacity(order.len());
    for (position, &body_idx) in order.iter().enumerate() {
        let unit = &model.body[body_idx];
        let granted = reduce_unit_authority(&model, body_idx)?;
        let mut dependencies: BTreeSet<&str> = BTreeSet::new();
        for input in &unit.inputs {
            if let Some(producers_of_name) = producers.get(input.name.as_str()) {
                for &producer_idx in producers_of_name {
                    if producer_idx != body_idx {
                        dependencies.insert(model.body[producer_idx].id.as_str());
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
        });
    }

    Ok(CompiledExecutionGraphV1 {
        schema_version: EXECUTION_GRAPH_SCHEMA_VERSION.to_string(),
        workflow_digest,
        plan_identity: plan_identity.to_string(),
        steps,
    })
}

/// Reduce one unit's authority against the workflow ceiling (R8b).
///
/// Ported from the reference `reduce_authority` (soma-validate): every
/// capability, scope, and secret the unit declares must already appear in
/// the ceiling; absent dimensions are carried as `None` rather than copied
/// from the ceiling. Fail closed with `SOMA-AUTH-*` diagnostics on any
/// widening (unreachable when the caller has already passed the audit —
/// defense in depth for direct callers).
fn reduce_unit_authority(
    workflow: &WorkflowDefinition,
    unit_index: usize,
) -> Result<AuthorityProfile, Vec<Diagnostic>> {
    let unit = &workflow.body[unit_index];
    let ceiling = &workflow.authority;
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
                format!(
                    "operation {} declares an empty capability grant",
                    unit.id
                ),
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
        tools: if tools.is_empty() {
            None
        } else {
            Some(tools)
        },
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
