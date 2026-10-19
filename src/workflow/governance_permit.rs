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
use crate::workflow::soma::contracts::WorkflowDefinition;

/// A reviewed, sealed governance plan plus the execution graph it authorizes.
///
/// Constructed ONLY by GovernancePermit::issue. Fields are private on
/// purpose: a permit is a capability-bearing artifact and must not be
/// hand-assembled or mutated after issue.
///
/// Slice B (P1-A, review repair): this is the only governed
/// suspension/resume path, and the boundary holds INSIDE the crate too —
/// the raw transition primitives in `workflow::graph_suspend` require a
/// real `&GovernancePermit` and read their digest material from it, so
/// no sibling module can forge plan/authority digests (no transition
/// accepts digest material at all). An in-crate policy regression
/// (`graph_suspend::tests::raw_transition_surface_stays_permit_gated_in_crate`)
/// fails if a forgeable binding constructor or raw digest-bearing
/// transition surface becomes production-visible again. Stated honestly:
/// no API/CLI suspension endpoint exists; the library permit boundary IS
/// the externally reachable governed transition surface.
///
/// Compile-fail proof (runs under `cargo test --doc --all-features`,
/// registered in BOTH evidence-runner specification points): an external
/// caller cannot bypass the reviewed permit by hand-assembling one:
///
/// ```compile_fail
/// use prometheos_lite::workflow::governance_permit::GovernancePermit;
/// let _permit = GovernancePermit {
///     plan_identity: "deadbeef".repeat(8),
///     authority_graph: loop {},
///     execution_graph: loop {},
///     expansion: loop {},
/// };
/// ```
#[derive(Debug, Clone)]
pub struct GovernancePermit {
    plan_identity: String,
    authority_graph: CompiledAuthorityGraph,
    execution_graph: CompiledExecutionGraphV1,
    /// The composite expansion plan this permit executes (E4/I07 #253);
    /// `None` for atomic bodies. Digest-bound to the same workflow as the
    /// sealed plan at issue time.
    expansion: Option<crate::workflow::composite_expansion::ExpansionPlanV1>,
}

impl GovernancePermit {
    pub fn issue(workflow_text: &str, reviewed_identity: &str) -> Result<Self, Vec<Diagnostic>> {
        let plan = compile_workflow_text(workflow_text)?;
        // E4/X03 (#160) → E4/I07 (#253): compilation expands nested
        // composites deterministically, and the expansion is now RUNTIME
        // INPUT — issue() binds the sealed expanded plan, the authority
        // graph over the fully-qualified leaves, and the expansion plan
        // (digest-bound to the same workflow) into one permit. Sequential
        // dispatch executes the flattened leaves in expansion order;
        // every leaf keeps its fully-qualified scope identity across
        // execution, routing, evidence, suspension, checkpoint, and
        // resume.
        let workflow: WorkflowDefinition = serde_json::from_str(workflow_text).map_err(|e| {
            vec![Diagnostic::new(
                "SOMA-CMP-0003",
                format!("schema violation: {e}"),
            )]
        })?;
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
        // E4/I07 (#253): composite workflows carry their expansion plan,
        // bound to the same reviewed workflow digest as the sealed plan —
        // a stale or incoherent expansion can never back a permit.
        let expansion = if workflow.contains_composite_body_item() {
            let expansion =
                crate::workflow::governance_compiler::compile_expansion_plan(workflow_text)?;
            if expansion.workflow_digest != plan.workflow_digest {
                return Err(vec![Diagnostic::new(
                    "SOMA-CMP-0004",
                    "expanded plan digest does not bind to the reviewed workflow",
                )]);
            }
            Some(expansion)
        } else {
            None
        };
        Ok(Self {
            plan_identity: plan.canonicalization.sha256,
            authority_graph,
            execution_graph,
            expansion,
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

    /// The composite expansion plan this permit executes (E4/I07 #253):
    /// the flattened, scope-identified leaf steps with typed boundary
    /// crossings and chain-narrowed effective authority. `None` for
    /// atomic bodies (their execution graph is already the full story).
    pub fn expansion_plan(&self) -> Option<&crate::workflow::composite_expansion::ExpansionPlanV1> {
        self.expansion.as_ref()
    }

    /// Canonical digest of the compiled authority snapshot this permit
    /// governs (`governance::CompiledAuthorityGraph::snapshot_digest`).
    /// Deterministic per issued permit; used to anchor durable suspension
    /// and resume-token bindings below.
    pub fn authority_snapshot_digest(&self) -> String {
        self.authority_graph
            .snapshot_digest()
            .expect("Lite-constructed authority graph digests")
    }

    /// Suspend a permit-governed graph run: bind the suspension to this
    /// permit's reviewed plan identity and authority snapshot (E4/X05
    /// contracts in [`crate::workflow::graph_suspend`]).
    ///
    /// Operation binding (P1-B repair): the supplied `manifest` must be
    /// the exact graph revision the run pinned at open, and
    /// `operation_id` must be (1) a step of this permit's reviewed plan,
    /// (2) a node of `manifest`, and (3) on the run's live frontier.
    /// The transition fails closed and journals nothing otherwise.
    ///
    /// `now` is the recorded-at clock; `revision`/`expiration` pass through
    /// the vendored-mapped optional members. Refuses a terminated run and
    /// malformed digest inputs (inherited write-boundary validation).
    #[allow(clippy::too_many_arguments)]
    pub fn suspend_run(
        &self,
        state: &mut crate::workflow::graph_state::GraphRunStateV1,
        manifest: &crate::workflow::graph_state::GraphManifestV1,
        suspension_id: &str,
        trigger: &str,
        operation_id: &str,
        inputs_digest: &str,
        now: &str,
        revision: Option<&str>,
        expiration: Option<&str>,
    ) -> anyhow::Result<crate::workflow::graph_suspend::SuspensionPointV1> {
        crate::workflow::graph_suspend::suspend_run(
            state,
            manifest,
            suspension_id,
            trigger,
            operation_id,
            inputs_digest,
            now,
            revision,
            expiration,
            self,
        )
    }

    /// Resume a suspended run under THIS permit: the suspension and its
    /// token are checked against this permit's reviewed identity and
    /// authority snapshot before the run law (`graph_resume`) executes.
    /// A token issued under any other plan identity or authority snapshot
    /// fails closed (`SOMA-RES-0001`); the X05 ladder and #123 gates are
    /// untouched.
    ///
    /// Operation binding (P1-B repair): the ladder revalidates the
    /// suspension's resumability operation against this permit (a step
    /// of its reviewed plan), the supplied manifest, and the imported
    /// run's live frontier — the same binding the creation gate
    /// enforced, re-proven at resume.
    #[allow(clippy::too_many_arguments)]
    pub fn resume_run(
        &self,
        checkpoint_text: &str,
        suspension: &crate::workflow::graph_suspend::SuspensionPointV1,
        token: &crate::workflow::graph_suspend::ResumeTokenV1,
        manifest: &crate::workflow::graph_state::GraphManifestV1,
        portable_digest: &str,
        repo_revision_now: &str,
        now: &str,
        replay: Option<crate::workflow::graph_resume::ReplayAuthorization>,
    ) -> Result<
        crate::workflow::graph_resume::ResumedRun,
        crate::workflow::graph_suspend::ResumeWithTokenError,
    > {
        crate::workflow::graph_suspend::resume_with_token(
            checkpoint_text,
            suspension,
            token,
            manifest,
            portable_digest,
            repo_revision_now,
            now,
            replay,
            self,
        )
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
