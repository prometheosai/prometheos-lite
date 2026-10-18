//! E4/I07 (#253) — governed sequential execution of expanded nested
//! composites.
//!
//! The deterministic `CompositeWorkflow` expansion (#160) is runtime
//! input: a reviewed `GovernancePermit` issues over the sealed expanded
//! plan, the flattened leaf operations carry fully-qualified
//! `scopePath` identity through execution, routing, evidence,
//! suspension, checkpoint, and resume, and the leaves execute
//! sequentially through the real `NodeRunner` gate pipeline into the
//! `GraphRunStateV1` durable law. The run manifest is the orchestrator
//! contract (as for atomic runs): its nodes are the permit's
//! fully-qualified step ids.
//!
//! Determinism note: the runner stamps wall-clock times into
//! `NodeResultV1`, so the driver supplies the durable attempt
//! references (per-step result digests and timestamps) exactly as an
//! orchestrator would; every identity this file compares across two
//! runs is then a pure function of the reviewed workflow text.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use prometheos_lite::workflow::governance_compiler::compile_workflow_text;
use prometheos_lite::workflow::governance_permit::GovernancePermit;
use prometheos_lite::workflow::graph_exec::{ExecutionLimits, route_after, run_complete};
use prometheos_lite::workflow::graph_state::{
    EdgeKind, GraphEdgeV1, GraphManifestV1, GraphNodeV1, GraphRunStateV1, NodeAttemptRecordV1,
    OutcomeCategory, RouteDecisionV1,
};
use prometheos_lite::workflow::graph_suspend::issue_resume_token;
use prometheos_lite::workflow::node_contracts::NodeManifestV1;
use prometheos_lite::workflow::node_runner::{
    Capability, CapabilityRegistry, NodeRunRequest, NodeRunner,
};
use prometheos_lite::workflow::policy::LocalRestrictions;
use prometheos_lite::workflow::projection::GraphDisclosurePolicy;
use prometheos_lite::workflow::projection::project_graph_json;
use prometheos_lite::workflow::soma::canonical::try_canonical_digest;
use prometheos_lite::workflow::soma::contracts::WorkflowDefinition;
use prometheos_lite::workflow::soma::supported_version;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/composite/nested-two-level.json"
);
const LEAF_FQ: &str = "flow/inner/leaf";
const NOW: &str = "2026-10-18T00:00:00Z";
const INPUTS_DIGEST: &str = "4444444444444444444444444444444444444444444444444444444444444444";

fn fixture_text() -> String {
    std::fs::read_to_string(FIXTURE).expect("read fixture")
}

fn reviewed_identity(text: &str) -> String {
    compile_workflow_text(text)
        .expect("audit-clean workflow compiles")
        .canonicalization
        .sha256
}

/// Audit-clean mutation helper: every mutated document must still pass
/// the published audit so a later refusal provably comes from the
/// expansion/chain/runtime layer.
fn assert_audit_clean(text: &str) {
    let wf: WorkflowDefinition = serde_json::from_str(text).expect("parses");
    let diags = wf.audit(&supported_version());
    assert!(
        diags.is_empty(),
        "mutation must stay audit-clean: {diags:?}"
    );
}

fn restrictions() -> LocalRestrictions {
    LocalRestrictions {
        readable_scopes: vec!["orders".into()],
        writable_scopes: vec!["shipping".into()],
        token_budget_ceiling: None,
        denied_providers: vec![],
        forbidden_paths: vec![],
        max_attempts: 1,
        escalation_target: "human-review".into(),
    }
}

fn manifest_for(fq: &str, readable: &[&str], writable: &[&str]) -> NodeManifestV1 {
    NodeManifestV1::parse_json(
        &serde_json::json!({
            "schemaVersion": "1.0.0",
            "nodeId": fq,
            "purpose": "governed nested leaf",
            "inputs": [],
            "outputs": [{"name": "result", "typeRef": "string"}],
            "readableScopes": readable,
            "writableScopes": writable,
            "retry": {"maxAttempts": 1, "retryableClasses": []}
        })
        .to_string(),
    )
    .unwrap()
}

fn ship_runner(permit: GovernancePermit, counter: Arc<AtomicUsize>) -> NodeRunner {
    let mut reg = CapabilityRegistry::new();
    let hits = counter.clone();
    reg.declare(
        "ship",
        Capability::deterministic(&["payload"], move |a| {
            hits.fetch_add(1, Ordering::SeqCst);
            Ok(format!(
                "ship:{}",
                a.get("payload").and_then(|p| p.as_str()).unwrap_or("")
            ))
        }),
    );
    NodeRunner::new(reg, permit)
}

fn ship_request<'a>(manifest: &'a NodeManifestV1, r: &'a LocalRestrictions) -> NodeRunRequest<'a> {
    NodeRunRequest {
        manifest,
        local_restrictions: r,
        capability: "ship".into(),
        args: serde_json::json!({"payload": "order-1"}),
        idempotency_key: format!("{}:ship", manifest.node_id),
        known_secrets: vec![],
    }
}

/// The orchestrator's sequential run contract: nodes are the permit's
/// fully-qualified step ids plus one declared terminal exit, edges
/// follow the permit's flattened order into the exit, entry is the
/// first leaf (the terminal exit never enters the frontier, #120).
fn sequential_manifest(permit: &GovernancePermit) -> GraphManifestV1 {
    let ids: Vec<String> = permit
        .execution_graph()
        .steps
        .iter()
        .map(|s| s.operation_id.clone())
        .collect();
    assert!(ids.len() >= 2, "expanded run needs at least two leaves");
    let exit = "terminal-exit".to_string();
    let mut nodes: Vec<GraphNodeV1> = ids
        .iter()
        .map(|id| GraphNodeV1 {
            node_id: id.clone(),
            capability: "ship".into(),
            purpose: None,
            resources: vec![],
            join: None,
        })
        .collect();
    nodes.push(GraphNodeV1 {
        node_id: exit.clone(),
        capability: "ship".into(),
        purpose: None,
        resources: vec![],
        join: None,
    });
    let mut edges: Vec<GraphEdgeV1> = ids
        .windows(2)
        .map(|w| GraphEdgeV1 {
            from: w[0].clone(),
            to: w[1].clone(),
            kind: EdgeKind::Sequence,
            condition_label: None,
        })
        .collect();
    edges.push(GraphEdgeV1 {
        from: ids[ids.len() - 1].clone(),
        to: exit.clone(),
        kind: EdgeKind::Sequence,
        condition_label: None,
    });
    GraphManifestV1 {
        schema_version: "1.0.0".into(),
        graph_id: "nested-runtime".into(),
        version: "1.0.0".into(),
        nodes,
        edges,
        entry_points: vec![ids[0].clone()],
        terminal_exits: vec![exit],
        shared_state_keys: vec![],
        policy_digest: None,
        content_digest: None,
    }
    .sealed()
}

/// Deterministic durable attempt reference for one executed leaf, as an
/// orchestrator records it (the runner's wall-clock result is retained
/// in the runner's own journal; the durable graph attempt binds the
/// canonical digest of the governed step identity + output).
fn attempt(step: usize, fq: &str, output: &str) -> NodeAttemptRecordV1 {
    let result_digest = try_canonical_digest(&serde_json::json!({
        "node": fq,
        "output": output,
    }))
    .expect("attempt reference digests");
    NodeAttemptRecordV1 {
        attempt: step as u32,
        started_at: format!("t{step}"),
        completed_at: Some(format!("t{step}")),
        outcome: OutcomeCategory::Completed,
        result_digest,
    }
}

/// One complete governed sequential drive over the expanded leaves,
/// optionally suspending + resuming through the real permit ladder just
/// before one leaf executes. Every recorded identity is a pure function
/// of the workflow text.
#[derive(Debug, Clone, PartialEq)]
struct RunTrace {
    step_ids: Vec<String>,
    attempt_digests: Vec<String>,
    decisions: Vec<RouteDecisionV1>,
    suspension_digest: Option<String>,
    token_binding: Option<String>,
    resumed_frontier: Vec<String>,
    final_state_digest: String,
}

fn drive_nested_run(
    permit: &GovernancePermit,
    suspend_before: Option<&str>,
) -> (RunTrace, GraphRunStateV1) {
    let m = sequential_manifest(permit);
    let mut run = GraphRunStateV1::open("r1", &m, "rev-1", "pws", "pd", "t0").unwrap();
    let mut runner = ship_runner(permit.clone(), Arc::new(AtomicUsize::new(0)));
    let mut trace = RunTrace {
        step_ids: vec![],
        attempt_digests: vec![],
        decisions: vec![],
        suspension_digest: None,
        token_binding: None,
        resumed_frontier: vec![],
        final_state_digest: String::new(),
    };
    for (i, step) in permit.execution_graph().steps.iter().enumerate() {
        let fq = step.operation_id.clone();
        trace.step_ids.push(fq.clone());
        if suspend_before == Some(fq.as_str()) {
            let suspension = permit
                .suspend_run(
                    &mut run,
                    &m,
                    "susp-1",
                    "human-review",
                    &fq,
                    INPUTS_DIGEST,
                    "t9",
                    None,
                    None,
                )
                .expect("permit-bound suspension at the expanded leaf");
            let token = issue_resume_token(&suspension, "tok-1", None).unwrap();
            let ckpt = run.export_checkpoint().unwrap();
            let resumed = permit
                .resume_run(&ckpt, &suspension, &token, &m, "pd", "rev-1", NOW, None)
                .expect("permit resumes the expanded leaf");
            trace.suspension_digest = Some(suspension.digest().unwrap());
            trace.token_binding = Some(token.binding_digest.clone());
            trace.resumed_frontier = resumed.state.frontier.clone();
            run = resumed.state;
        }
        let manifest = manifest_for(&fq, &["orders"], &[]);
        let outcome = runner
            .execute(ship_request(&manifest, &restrictions()))
            .expect("expanded leaf executes through the governed runner");
        let output = outcome.output.clone();
        let attempt = attempt(i + 1, &fq, &output);
        trace.attempt_digests.push(attempt.result_digest.clone());
        run.apply_node_completion(&fq, attempt, format!("t{i}"))
            .unwrap();
        // Route after every completion: the frontier is momentarily
        // empty here (#120 law), and the final route targets the
        // terminal exit, which never re-enters the frontier.
        let decision = route_after(
            &run,
            &m,
            ExecutionLimits::default(),
            &fq,
            OutcomeCategory::Completed,
            format!("t{i}"),
        )
        .expect("deterministic route to the next expanded leaf");
        run.record_route_decision(decision.clone(), &m).unwrap();
        trace.decisions.push(decision);
    }
    assert!(run_complete(&run), "expanded run reaches terminal");
    trace.final_state_digest = run.content_digest.clone().expect("sealed");
    (trace, run)
}

fn issue_permit(text: &str) -> GovernancePermit {
    GovernancePermit::issue(text, &reviewed_identity(text)).expect("permit issues")
}

// ---------------------------------------------------------------------------
// Structure: the permit binds the sealed expanded plan.
// ---------------------------------------------------------------------------

#[test]
fn nested_composite_issues_a_permit_over_the_expanded_plan() {
    let permit = issue_permit(&fixture_text());
    let expansion = permit
        .expansion_plan()
        .expect("composite carries expansion");
    let plan = compile_workflow_text(&fixture_text()).unwrap();
    assert_eq!(expansion.workflow_digest, plan.workflow_digest);

    // Fully-qualified identity: reused across execution, routing,
    // evidence, suspension, checkpoint, and resume.
    let ids: Vec<&str> = permit
        .execution_graph()
        .steps
        .iter()
        .map(|s| s.operation_id.as_str())
        .collect();
    assert_eq!(ids, ["prep", LEAF_FQ, "audit"], "flattened scope order");
    let steps = &permit.execution_graph().steps;
    assert_eq!(steps[0].scope_path, Vec::<String>::new());
    assert_eq!(
        steps[1].scope_path,
        vec!["flow".to_string(), "inner".to_string()]
    );
    assert_eq!(steps[2].scope_path, Vec::<String>::new());
    // Plan keys stay the published v1.1 shape over the flattened order.
    assert_eq!(steps[0].key, "s0000:prep");
    assert_eq!(steps[1].key, "s0001:leaf");
    assert_eq!(steps[2].key, "s0002:audit");
    // Every leaf's granted authority is reduced from its own declared
    // grants against the reviewed ceiling.
    for step in steps {
        assert!(
            step.granted.tool_keys().contains(&"ship"),
            "leaf {} keeps its reviewed capability",
            step.operation_id
        );
    }

    // Authority graph covers the fully-qualified leaves.
    let op_ids: Vec<&str> = permit
        .authority_graph()
        .operations
        .iter()
        .map(|o| o.op_id.as_str())
        .collect();
    assert_eq!(op_ids, ["prep", LEAF_FQ, "audit"]);

    // Atomic permits carry no expansion plan (existing behavior).
    let atomic = permit_for_atomic();
    assert!(atomic.expansion_plan().is_none());
}

fn permit_for_atomic() -> GovernancePermit {
    let text = serde_json::json!({
        "schemaVersion": "1.1.0", "version": "1.1.0", "id": "atomic-only",
        "name": "Atomic", "kind": "atomic",
        "inputPorts": [{"name": "goal", "direction": "input", "type": "string",
                         "cardinality": "single", "requiredness": "required"}],
        "outputPorts": [{"name": "result", "direction": "output", "type": "string",
                          "cardinality": "single", "requiredness": "required"}],
        "body": [{
            "schemaVersion": "1.1.0", "version": "1.1.0", "id": "solo",
            "executionClass": "deterministic",
            "inputs": [{"name": "goal", "type": "string", "acceptedOutcomes": ["Produced"]}],
            "outputs": [{"name": "result", "type": "string", "emits": ["Produced"]}],
            "authority": ["ship"], "effects": [], "uses": [], "secrets": [], "context": []
        }],
        "authority": {
            "executionClass": "deterministic", "mutation": "none", "tools": {"ship": ["ship.v1"]},
            "readableScopes": ["orders"], "writableScopes": ["shipping"],
            "networkPolicy": {"default": "deny"}, "providerPolicy": {"allowlist": []}, "secrets": []
        }
    })
    .to_string();
    issue_permit(&text)
}

// ---------------------------------------------------------------------------
// End to end: two-level nested composite through the real runtime.
// ---------------------------------------------------------------------------

#[test]
fn nested_composite_executes_sequentially_end_to_end() {
    let permit = issue_permit(&runtime_fixture_text());
    let (trace, run) = drive_nested_run(&permit, None);
    assert_eq!(trace.step_ids, ["prep", LEAF_FQ, "audit"]);
    assert_eq!(
        trace.decisions.len(),
        3,
        "routes between leaves plus the terminal-exit route"
    );
    assert_eq!(
        trace.decisions[0].from_node, "prep",
        "routing carries fully-qualified identity"
    );
    assert_eq!(trace.decisions[0].to_node, LEAF_FQ);
    assert_eq!(trace.decisions[1].from_node, LEAF_FQ);
    assert_eq!(trace.decisions[1].to_node, "audit");
    assert_eq!(trace.decisions[2].from_node, "audit");
    assert_eq!(trace.decisions[2].to_node, "terminal-exit");
    // Evidence keys are the fully-qualified leaf ids.
    let keys: Vec<&str> = run.node_attempts.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        ["audit", LEAF_FQ, "prep"],
        "evidence keyed by scope identity"
    );
}

#[test]
fn same_workflow_produces_byte_identical_plan_route_and_evidence_identities() {
    let text = runtime_fixture_text();
    let permit_a = issue_permit(&text);
    let permit_b = issue_permit(&text);
    // Byte-identical expanded plan.
    let a = serde_json::to_string(permit_a.expansion_plan().unwrap()).unwrap();
    let b = serde_json::to_string(permit_b.expansion_plan().unwrap()).unwrap();
    assert_eq!(a, b);
    assert_eq!(permit_a.plan_identity(), permit_b.plan_identity());
    assert_eq!(
        permit_a.authority_snapshot_digest(),
        permit_b.authority_snapshot_digest()
    );
    // Identical route sequence, evidence identities, and (with the
    // suspension ladder exercised at the same leaf) identical
    // suspension and token identities.
    let (trace_a, _) = drive_nested_run(&permit_a, Some(LEAF_FQ));
    let (trace_b, _) = drive_nested_run(&permit_b, Some(LEAF_FQ));
    assert_eq!(trace_a, trace_b);
}

// ---------------------------------------------------------------------------
// Suspension / resume preserve the exact expanded operation + scope.
// ---------------------------------------------------------------------------

#[test]
fn suspension_and_resume_preserve_expanded_scope_identity() {
    let permit = issue_permit(&runtime_fixture_text());
    let m = sequential_manifest(&permit);
    let mut run = GraphRunStateV1::open("r1", &m, "rev-1", "pws", "pd", "t0").unwrap();
    advance_to_nested_leaf(&mut run, &m);

    let suspension = permit
        .suspend_run(
            &mut run,
            &m,
            "susp-1",
            "human-review",
            LEAF_FQ,
            INPUTS_DIGEST,
            "t1",
            None,
            None,
        )
        .expect("suspends at the nested leaf");
    assert_eq!(suspension.resumability.operation_id, LEAF_FQ);
    assert_eq!(suspension.plan_digest, permit.plan_identity());
    assert_eq!(
        suspension.authority_snapshot_digest,
        permit.authority_snapshot_digest()
    );
    let token = issue_resume_token(&suspension, "tok-1", None).unwrap();
    let ckpt = run.export_checkpoint().unwrap();
    let resumed = permit
        .resume_run(&ckpt, &suspension, &token, &m, "pd", "rev-1", NOW, None)
        .expect("checkpointed resume re-enters the same expanded leaf");
    assert_eq!(resumed.state.frontier, vec![LEAF_FQ.to_string()]);

    // The P1-B binding holds on fully-qualified identity: the bare local
    // id is NOT a permit step, and a foreign scope path is refused
    // (both fire at the permit-step gate, before the live-frontier gate).
    let mut fresh = GraphRunStateV1::open("r2", &m, "rev-1", "pws", "pd", "t0").unwrap();
    let err = permit
        .suspend_run(
            &mut fresh,
            &m,
            "s-x",
            "human-review",
            "leaf",
            INPUTS_DIGEST,
            "t1",
            None,
            None,
        )
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("not governed by the reviewed plan"),
        "local id without scope must not bind: {err}"
    );
    let mut fresh = GraphRunStateV1::open("r3", &m, "rev-1", "pws", "pd", "t0").unwrap();
    let err = permit
        .suspend_run(
            &mut fresh,
            &m,
            "s-x",
            "human-review",
            "flow/inner/ghost",
            INPUTS_DIGEST,
            "t1",
            None,
            None,
        )
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("not governed by the reviewed plan"),
        "unknown scope path must not bind: {err}"
    );
}

#[test]
fn stale_expanded_plan_permit_refuses_resume() {
    // A suspension issued under one workflow's expanded plan refuses
    // resume under a permit whose reviewed workflow moved (stale plan
    // identity, SOMA-RES-0001) — the ladder's staleness gate binds the
    // expanded plan identity at resume.
    let permit_a = issue_permit(&runtime_fixture_text());
    let permit_b = issue_permit(&fixture_text());
    assert_ne!(permit_a.plan_identity(), permit_b.plan_identity());
    let m = sequential_manifest(&permit_a);
    let mut run = GraphRunStateV1::open("r1", &m, "rev-1", "pws", "pd", "t0").unwrap();
    advance_to_nested_leaf(&mut run, &m);
    let suspension = permit_a
        .suspend_run(
            &mut run,
            &m,
            "susp-1",
            "human-review",
            LEAF_FQ,
            INPUTS_DIGEST,
            "t1",
            None,
            None,
        )
        .unwrap();
    let token = issue_resume_token(&suspension, "tok-1", None).unwrap();
    let ckpt = run.export_checkpoint().unwrap();
    let err = permit_b
        .resume_run(&ckpt, &suspension, &token, &m, "pd", "rev-1", NOW, None)
        .unwrap_err();
    let codes: Vec<String> = match &err {
        prometheos_lite::workflow::graph_suspend::ResumeWithTokenError::Refusal(d) => {
            d.iter().map(|x| x.code.clone()).collect()
        }
        prometheos_lite::workflow::graph_suspend::ResumeWithTokenError::Underlying(e) => {
            panic!("expected a typed refusal, got {e}")
        }
    };
    assert_eq!(codes, vec!["SOMA-RES-0001".to_string()]);
}

/// Audit-clean runtime fixture: the two-level fixture with every leaf
/// declaring its reviewed scopes (the nested leaf additionally imports
/// the writable shipping scope through its full ancestor chain — the
/// chain can only restrict, never widen).
fn runtime_fixture_text() -> String {
    let mut v: serde_json::Value = serde_json::from_str(&fixture_text()).expect("fixture parses");
    v["body"][0]["authority"] = serde_json::json!(["ship", "readable:orders"]);
    v["body"][1]["authorityImports"] =
        serde_json::json!(["ship", "readable:orders", "writable:shipping"]);
    v["body"][1]["body"][0]["authorityImports"] =
        serde_json::json!(["ship", "readable:orders", "writable:shipping"]);
    v["body"][1]["body"][0]["body"][0]["authority"] =
        serde_json::json!(["ship", "readable:orders", "writable:shipping"]);
    v["body"][2]["authority"] = serde_json::json!(["ship", "readable:orders"]);
    let text = serde_json::to_string(&v).unwrap();
    assert_audit_clean(&text);
    text
}

/// Advance a run past the first leaf so the nested leaf is on the live
/// frontier (suspension is a live-node transition).
fn advance_to_nested_leaf(run: &mut GraphRunStateV1, m: &GraphManifestV1) {
    run.apply_node_completion("prep", attempt(1, "prep", "ship:order-1"), "t0")
        .unwrap();
    let decision = route_after(
        run,
        m,
        ExecutionLimits::default(),
        "prep",
        OutcomeCategory::Completed,
        "t0",
    )
    .expect("prep routes to the nested leaf");
    run.record_route_decision(decision, m).unwrap();
    assert_eq!(run.frontier, vec![LEAF_FQ.to_string()]);
}

// ---------------------------------------------------------------------------
// Negative controls: nothing runs past the gates.
// ---------------------------------------------------------------------------

#[test]
fn ungoverned_node_is_refused_before_effects() {
    let permit = issue_permit(&runtime_fixture_text());
    let counter = Arc::new(AtomicUsize::new(0));
    let mut runner = ship_runner(permit, counter.clone());
    let m = manifest_for("rogue/leaf", &["orders"], &[]);
    let err = runner
        .execute(ship_request(&m, &restrictions()))
        .unwrap_err()
        .to_string();
    assert!(err.contains("SOMA-CMP-0002"), "{err}");
    assert_eq!(counter.load(Ordering::SeqCst), 0, "no effect ran");
}

#[test]
fn effective_authority_only_narrows_across_nesting_at_runtime() {
    let permit = issue_permit(&runtime_fixture_text());

    // The nested leaf imported the writable scope through its full
    // ancestor chain, so its request runs with it.
    let counter = Arc::new(AtomicUsize::new(0));
    let mut runner = ship_runner(permit.clone(), counter.clone());
    let m = manifest_for(LEAF_FQ, &["orders"], &["shipping"]);
    runner
        .execute(ship_request(&m, &restrictions()))
        .expect("chain-imported writable scope is effective for the nested leaf");
    assert_eq!(counter.load(Ordering::SeqCst), 1);

    // The root sibling never declared the writable scope: its request
    // is refused before any effect with SOMA-AUTH-0003.
    let m = manifest_for("prep", &["orders"], &["shipping"]);
    let err = runner
        .execute(ship_request(&m, &restrictions()))
        .unwrap_err()
        .to_string();
    assert!(err.contains("SOMA-AUTH-0003"), "{err}");
    assert!(err.contains("shipping"), "scope named: {err}");
    assert_eq!(counter.load(Ordering::SeqCst), 1, "no further effect ran");
}

#[test]
fn chain_authority_widening_refuses_a_runnable_permit() {
    // The leaf declares a writable scope an ancestor does not import:
    // the published audit gate itself refuses (SOMA-AUTH-0002) — no
    // permit can issue at all, so no runnable permit and no durable
    // side effects can exist. The identity argument is irrelevant:
    // issuance refuses inside plan compilation, before identity
    // verification.
    let mut v: serde_json::Value = serde_json::from_str(&fixture_text()).expect("fixture parses");
    v["body"][1]["authorityImports"] = serde_json::json!(["ship", "readable:orders"]);
    v["body"][1]["body"][0]["authorityImports"] = serde_json::json!(["ship", "readable:orders"]);
    v["body"][1]["body"][0]["body"][0]["authority"] =
        serde_json::json!(["ship", "readable:orders", "writable:shipping"]);
    let text = serde_json::to_string(&v).unwrap();
    let diags = GovernancePermit::issue(
        &text,
        "0000000000000000000000000000000000000000000000000000000000000000",
    )
    .expect_err("widening past an ancestor import must refuse issuance");
    assert!(
        diags.iter().any(|d| d.code == "SOMA-AUTH-0002"),
        "chain narrowing refusal: {diags:?}"
    );
}

#[test]
fn reused_local_operation_ids_are_refused_by_the_audit_and_scoped_at_runtime() {
    // The audit gate refuses document-wide duplicate ids (SOMA-EXP-0003)
    // before any permit exists. The identity argument is irrelevant —
    // issuance refuses inside plan compilation.
    let mut v: serde_json::Value = serde_json::from_str(&fixture_text()).expect("fixture parses");
    v["body"][1]["body"][0]["body"][0]["id"] = serde_json::json!("prep");
    let text = serde_json::to_string(&v).unwrap();
    let diags = GovernancePermit::issue(
        &text,
        "0000000000000000000000000000000000000000000000000000000000000000",
    )
    .expect_err("duplicate local id must refuse issuance");
    assert!(
        diags.iter().any(|d| d.code == "SOMA-EXP-0003"),
        "duplicate identity refusal: {diags:?}"
    );
    // Fully-qualified runtime identity keeps distinct scopes distinct
    // even for equal local ids (defense-in-depth control lives in the
    // in-crate execution-graph tests; the public identity format is
    // pinned here).
    let permit = issue_permit(&fixture_text());
    assert!(
        permit
            .execution_graph()
            .steps
            .iter()
            .any(|s| s.operation_id == LEAF_FQ)
    );
}

// ---------------------------------------------------------------------------
// Projections: private internals stay unflattened by default.
// ---------------------------------------------------------------------------

#[test]
fn public_graph_projection_keeps_private_internals_unflattened_by_default() {
    let wf: WorkflowDefinition = serde_json::from_str(&fixture_text()).expect("fixture parses");

    // Default policy: composite boundaries are withheld — no children,
    // no internal edges, disclosure block instead.
    let envelope = project_graph_json(&wf, None).expect("default projection");
    let value = serde_json::to_value(&envelope).expect("envelope serializes");
    let nodes = value["payload"]["nodes"].as_array().expect("nodes");
    let flow = nodes
        .iter()
        .find(|n| n["id"] == serde_json::json!("s0001:flow"))
        .expect("flow node renders");
    assert!(
        flow.get("children").is_none(),
        "private internals must not flatten by default"
    );
    assert_eq!(flow["disclosure"]["withheld"], serde_json::json!(true));

    // Authorized boundaries reveal contents (outer first, then inner).
    let policy = GraphDisclosurePolicy {
        authorized_boundaries: vec!["s0001:flow".into(), "s0000:inner".into()],
        count_authorization: vec![],
    };
    let envelope = project_graph_json(&wf, Some(&policy)).expect("authorized projection");
    let value = serde_json::to_value(&envelope).expect("envelope serializes");
    let nodes = value["payload"]["nodes"].as_array().expect("nodes");
    let flow = nodes
        .iter()
        .find(|n| n["id"] == serde_json::json!("s0001:flow"))
        .expect("flow node renders");
    let children = flow["children"]
        .as_array()
        .expect("authorized boundary reveals children");
    assert!(
        children
            .iter()
            .any(|c| c["id"] == serde_json::json!("s0000:inner"))
    );
}
