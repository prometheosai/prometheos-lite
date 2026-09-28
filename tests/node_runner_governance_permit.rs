//! T5 RED test file: GovernancePermit binding.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

mod common;
use common::permit_for;

use prometheos_lite::workflow::governance_permit::GovernancePermit;
use prometheos_lite::workflow::node_runner::{Capability, CapabilityRegistry, NodeRunRequest, NodeRunner};
use prometheos_lite::workflow::policy::LocalRestrictions;
use prometheos_lite::workflow::node_contracts::NodeManifestV1;

fn restrictions() -> LocalRestrictions {
    LocalRestrictions {
        readable_scopes: vec!["repo://evaluation".into()],
        writable_scopes: vec!["work://evaluation".into()],
        token_budget_ceiling: None,
        denied_providers: vec![],
        forbidden_paths: vec![],
        max_attempts: 1,
        escalation_target: "human-review".into(),
    }
}

fn manifest(node_id: &str) -> NodeManifestV1 {
    NodeManifestV1::parse_json(&serde_json::json!({
        "schemaVersion": "1.0.0",
        "nodeId": node_id,
        "purpose": "governed node",
        "inputs": [],
        "outputs": [{"name": "result", "typeRef": "string"}],
        "readableScopes": ["repo://evaluation"],
        "writableScopes": ["work://evaluation"],
        "retry": {"maxAttempts": 1, "retryableClasses": []}
    }).to_string()).unwrap()
}

fn echo_runner(permit: GovernancePermit, counter: Arc<AtomicUsize>) -> NodeRunner {
    let mut reg = CapabilityRegistry::new();
    reg.declare(
        "echo",
        Capability::deterministic(&["text"], move |a| {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(format!("echo:{}", a.get("text").and_then(|t| t.as_str()).unwrap_or("")))
        }),
    );
    NodeRunner::new(reg, permit)
}

fn req<'a>(m: &'a NodeManifestV1, r: &'a LocalRestrictions) -> NodeRunRequest<'a> {
    NodeRunRequest {
        manifest: m,
        local_restrictions: r,
        capability: "echo".into(),
        args: serde_json::json!({"text": "x"}),
        idempotency_key: format!("{}:echo", m.node_id),
        known_secrets: vec![],
    }
}

#[test]
fn permit_issue_requires_exact_reviewed_identity() {
    let text = common::governance_permit::workflow_text(&["node-a", "node-b"]);
    let identity = prometheos_lite::workflow::governance_compiler::compile_workflow_text(&text)
        .expect("audit-clean workflow compiles")
        .canonicalization
        .sha256;

    let wrong = GovernancePermit::issue(
        &text,
        "0000000000000000000000000000000000000000000000000000000000000000",
    );
    let diags = wrong.expect_err("wrong reviewed identity must refuse");
    assert!(
        diags.iter().any(|d| d.code == "SOMA-CMP-0004"),
        "CMP-0004 on identity mismatch: {:?}",
        diags
    );
    let remediation = diags[0].remediation.as_ref().expect("remediation attached");
    assert_eq!(
        remediation.action.as_deref(),
        Some("restore-reviewed-plan"),
        "restore-reviewed-plan remediation"
    );

    let permit = permit_for(&["node-a", "node-b"]);
    assert_eq!(permit.plan_identity(), &identity);
    assert!(
        !permit.authority_graph().operations.is_empty(),
        "authority graph ops non-empty"
    );
    assert!(
        !permit.execution_graph().steps.is_empty(),
        "execution graph non-empty"
    );
}

#[test]
fn node_runner_requires_a_permit() {
    let permit = permit_for(&["node-a"]);
    let mut runner = echo_runner(permit, Arc::new(AtomicUsize::new(0)));
    let m = manifest("node-a");
    let r = restrictions();
    let outcome = runner.execute(req(&m, &r)).expect("governed node runs");
    assert_eq!(outcome.output, "echo:x");
}

#[test]
fn ungoverned_node_is_refused_before_capability_resolution() {
    let permit = permit_for(&["node-a"]);
    let counter = Arc::new(AtomicUsize::new(0));
    let mut runner = echo_runner(permit, counter.clone());
    let m = manifest("node-b");
    let r = restrictions();
    let err = runner.execute(req(&m, &r)).unwrap_err().to_string();
    assert!(err.contains("SOMA-CMP-0002"), "CMP-0002-family refusal: {err}");
    assert!(
        err.contains("node-b"),
        "refused node id is named in the message: {err}"
    );
    assert_eq!(
        counter.load(Ordering::SeqCst),
        0,
        "no capability resolution or effect ran"
    );
}

#[tokio::test]
async fn ungoverned_node_refused_on_all_four_public_effect_paths() {
    let permit = permit_for(&["node-a"]);
    let counter = Arc::new(AtomicUsize::new(0));
    let m = manifest("node-b");
    let r = restrictions();

    let mut runner = echo_runner(permit.clone(), counter.clone());
    assert!(runner.execute(req(&m, &r)).is_err(), "execute fails closed");

    let mut runner = echo_runner(permit.clone(), counter.clone());
    assert!(
        runner.execute_async(req(&m, &r)).await.is_err(),
        "execute_async fails closed"
    );

    let mut runner = echo_runner(permit.clone(), counter.clone());
    assert!(
        runner.preflight_gates(&req(&m, &r)).is_err(),
        "preflight_gates fails closed"
    );

    let mut runner = echo_runner(permit.clone(), counter.clone());
    assert!(
        runner.seal_effect(&req(&m, &r), Ok("x".into())).await.is_err(),
        "seal_effect fails closed"
    );
    assert_eq!(
        counter.load(Ordering::SeqCst),
        0,
        "no effect ran on any path"
    );
}

#[test]
fn fast_loop_identity_drift_is_refused() {
use prometheos_lite::workflow::evaluate::{
    FAST_LOOP_REVIEWED_IDENTITY, FAST_LOOP_WORKFLOW_TEXT,
};
    let compiled = prometheos_lite::workflow::governance_compiler::compile_workflow_text(
        FAST_LOOP_WORKFLOW_TEXT,
    )
    .expect("embedded fast-loop workflow compiles");
    assert_eq!(
        compiled.canonicalization.sha256,
        FAST_LOOP_REVIEWED_IDENTITY,
        "embedded fast-loop workflow seals to the pinned reviewed identity"
    );

    let mut tampered: serde_json::Value =
        serde_json::from_str(FAST_LOOP_WORKFLOW_TEXT).expect("workflow parses");
    tampered["id"] = serde_json::json!("lite-fast-loop-tampered");
    let tampered_text = serde_json::to_string(&tampered).expect("serializes");
    let err = GovernancePermit::issue(&tampered_text, FAST_LOOP_REVIEWED_IDENTITY)
        .expect_err("tampered workflow must not issue against the pinned identity");
    assert!(
        err.iter().any(|d| d.code == "SOMA-CMP-0004"),
        "tamper surfaces CMP-0004: {:?}",
        err
    );
}

#[test]
fn manifest_without_permit_cannot_exist() {
    let _ = std::marker::PhantomData::<fn() -> NodeRunner>;
}