//! GovernancePermit: the reviewed-plan binding that closes the structural
//! bypass (issue #163 correction 3).
//!
//! A GovernancePermit is the ONLY thing that may authorize a NodeRunner.
//! It is produced by GovernancePermit::issue from a single source of truth:
//! a workflow text that has been compiled into a sealed governance plan and
//! verified against a reviewed identity, and it binds two things together:
//!
//! 1. the execution graph (which units may run, in what order, with what
//!    reduced authority), and
//! 2. the reviewed identity the plan was sealed against.
//!
//! A runner built without a permit cannot exist: NodeRunner::default is
//! removed and every constructor takes a GovernancePermit, so any code path
//! that reaches an effect must have passed through permit.ensure_request
//! first. The permit's fields are private; there is no builder, no public
//! field access, and no Default - a permit can only come from issue.

use crate::workflow::execution_graph::CompiledExecutionGraphV1;
use crate::workflow::governance::CompiledAuthorityGraph;
use crate::workflow::governance_compiler::{compile_workflow_text, verify_reviewed_plan};
use crate::workflow::node_contracts::NodeManifestV1;
use crate::workflow::soma::Diagnostic;

/// A reviewed, sealed governance plan plus the execution graph it authorizes.
///
/// Constructed ONLY by GovernancePermit::issue. Fields are private on
/// purpose: a permit is a capability-bearing artifact and must not be
/// hand-assembled or mutated after issue.
#[derive(Debug, Clone)]
pub struct GovernancePermit {
    plan_identity: String,
    authority_graph: CompiledAuthorityGraph,
    execution_graph: CompiledExecutionGraphV1,
}

impl GovernancePermit {
    pub fn issue(workflow_text: &str, reviewed_identity: &str) -> Result<Self, Vec<Diagnostic>> {
        let plan = compile_workflow_text(workflow_text)?;
        // verify_reviewed_plan takes the compiled ExecutionPlan document
        // (schemaVersion/planVersion/workflowDigest/steps/canonicalization),
        // not the workflow text: it re-validates the plan shape, recomputes
        // the self-seal, and binds it to the reviewed identity.
        let plan_text = serde_json::to_string(&plan).map_err(|e| {
            vec![Diagnostic::new(
                "SOMA-CMP-0004",
                format!("plan cannot be re-serialized for verification ({e})"),
            )]
        })?;
        verify_reviewed_plan(&plan_text, reviewed_identity)?;
        let workflow_value: serde_json::Value =
            serde_json::from_str(workflow_text).map_err(|e| {
                vec![Diagnostic::new(
                    "SOMA-CMP-0003",
                    format!("schema violation: {e}"),
                )]
            })?;
        let authority_graph = crate::workflow::governance::compile_authority(&workflow_value)?;
        let execution_graph = crate::workflow::execution_graph::compile_execution_graph(
            workflow_text,
            &plan.canonicalization.sha256,
        )?;
        Ok(Self {
            plan_identity: plan.canonicalization.sha256,
            authority_graph,
            execution_graph,
        })
    }

    pub fn plan_identity(&self) -> &str {
        &self.plan_identity
    }

    pub fn authority_graph(&self) -> &CompiledAuthorityGraph {
        &self.authority_graph
    }

    pub fn execution_graph(&self) -> &CompiledExecutionGraphV1 {
        &self.execution_graph
    }

    /// Bind one concrete request to the compiled execution graph.
    ///
    /// The manifest's node id must name a step of the reviewed plan, the
    /// requested capability must be a key of that step's granted
    /// `tools`, and the manifest's readable/writable scopes must each be
    /// subsets of the step's granted scopes (`None` grants nothing).
    /// Refusals carry the catalogue code that matches the violation:
    /// SOMA-CMP-0002 (ungoverned node), SOMA-AUTH-0001 (capability used
    /// but not granted), SOMA-AUTH-0003 (scope outside the grant). This
    /// runs FIRST in every public effect path, so a request that
    /// substitutes a capability or widens its manifest past the reviewed
    /// authority is refused before capability resolution.
    pub fn ensure_request(
        &self,
        manifest: &NodeManifestV1,
        capability: &str,
    ) -> Result<(), Vec<Diagnostic>> {
        let step = self
            .execution_graph
            .steps
            .iter()
            .find(|step| step.operation_id == manifest.node_id);
        let Some(step) = step else {
            return Err(vec![Diagnostic::new(
                "SOMA-CMP-0002",
                format!(
                    "node {node_id:?} is not governed by the reviewed plan; \
                     the permit does not authorize this node",
                    node_id = manifest.node_id
                ),
            )]);
        };
        if !step.granted.tool_keys().contains(&capability) {
            return Err(vec![Diagnostic::new(
                "SOMA-AUTH-0001",
                format!(
                    "node {node_id:?} requested capability {capability:?}, which \
                     the reviewed step {step:?} does not grant",
                    node_id = manifest.node_id,
                    step = step.operation_id
                ),
            )]);
        }
        for scope in &manifest.readable_scopes {
            if !step.granted.readable_scope_set().contains(scope.as_str()) {
                return Err(vec![Diagnostic::new(
                    "SOMA-AUTH-0003",
                    format!(
                        "node {node_id:?} manifest declares readable scope {scope:?} \
                         outside the reviewed step grant",
                        node_id = manifest.node_id
                    ),
                )]);
            }
        }
        for scope in &manifest.writable_scopes {
            if !step.granted.writable_scope_set().contains(scope.as_str()) {
                return Err(vec![Diagnostic::new(
                    "SOMA-AUTH-0003",
                    format!(
                        "node {node_id:?} manifest declares writable scope {scope:?} \
                         outside the reviewed step grant",
                        node_id = manifest.node_id
                    ),
                )]);
            }
        }
        Ok(())
    }
}
