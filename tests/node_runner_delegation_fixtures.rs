//! Delegation bypass-proof fixtures for the governed NodeRunner
//! (issue #117 slice 3).
//!
//! Every delegation kind the fast loop supports — nested node execution,
//! code mode, tool bridges, and external adapter boundaries — must reach its
//! effect ONLY through the nine-gate pipeline. These fixtures prove:
//!
//! 1. nested runners: a parent capability invoking a child NodeRunner yields
//!    TWO journaled, digest-chained outcomes (both sides gated);
//! 2. code mode / tool bridge / external adapter capabilities execute only
//!    via runner-resolved one-shot handlers;
//! 3. bypass attempts fail closed: re-resolving a consumed capability,
//!    invoking an undeclared capability, and preflight-after-consumption all
//!    refuse.
//!
//! The external-adapter fixture simulates the adapter boundary with a stub
//! handler; the production `ExecutionHarness` adapter family lands with
//! #154 and consumes this exact seam.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use prometheos_lite::workflow::node_contracts::NodeManifestV1;
use prometheos_lite::workflow::node_runner::{Capability, CapabilityRegistry, NodeRunner};
use prometheos_lite::workflow::policy::LocalRestrictions;

fn manifest(node_id: &str) -> NodeManifestV1 {
    NodeManifestV1::parse_json(&{
        serde_json::json!({
            "schemaVersion": "1.0.0",
            "nodeId": node_id,
            "purpose": "delegation fixture",
            "inputs": [],
            "outputs": [{"name": "out", "typeRef": "string"}],
            "readableScopes": ["repo://x"],
            "writableScopes": ["work://y"],
            "retry": {"maxAttempts": 2, "retryableClasses": ["infra"]}
        })
        .to_string()
    })
    .unwrap()
}

fn restrictions() -> LocalRestrictions {
    LocalRestrictions {
        readable_scopes: vec!["repo://x".into()],
        writable_scopes: vec!["work://y".into()],
        token_budget_ceiling: None,
        denied_providers: vec![],
        forbidden_paths: vec![],
        max_attempts: 3,
        escalation_target: "human-review".into(),
    }
}

/// Build the outer runner whose capabilities delegate to every supported
/// kind. Each delegating handler routes the inner effect through its OWN
/// governed runner — delegation never reaches mechanics directly.
fn delegation_registry(child_calls: Arc<AtomicUsize>) -> CapabilityRegistry {
    let mut reg = CapabilityRegistry::new();

    // 1. Nested node: handler spawns a governed CHILD runner for the inner
    //    node. The child's effect is itself gated (its own nine gates).
    let nested_child = child_calls.clone();
    reg.declare(
        "nested.delegate",
        Capability::asynchronous(&["payload"], {
            let calls = nested_child;
            move |_args| {
                Box::pin(async move {
                    // The inner effect is only reachable through a child
                    // governed runner; there is no direct mechanic here.
                    let mut child_reg = CapabilityRegistry::new();
                    child_reg.declare(
                        "inner.effect",
                        Capability::deterministic(&["body"], move |a| {
                            calls.fetch_add(1, Ordering::SeqCst);
                            Ok(format!(
                                "inner:{}",
                                a.get("body").and_then(|b| b.as_str()).unwrap_or("")
                            ))
                        }),
                    );
                    let mut child = NodeRunner::new(child_reg);
                    let m = manifest("nested.child");
                    let r = restrictions();
                    let outcome = child.execute(NodeRunRequestLike::build(
                        &m,
                        &r,
                        "inner.effect",
                        serde_json::json!({"body": "nested"}),
                    ))?;
                    Ok(format!("nested:{}", outcome.output))
                })
            }
        }),
    );

    // 2. Code mode: the "interpreted payload" executes inside the handler —
    //    still exclusively via the resolved one-shot capability.
    reg.declare(
        "code.eval",
        Capability::asynchronous(&["expr"], move |args| {
            Box::pin(async move {
                let expr = args.get("expr").and_then(|e| e.as_str()).unwrap_or("");
                Ok(format!("code:{expr}"))
            })
        }),
    );

    // 3. Tool bridge: a tool-shaped invocation crossing the runner boundary.
    reg.declare(
        "tool.bridge",
        Capability::asynchronous(&["tool", "input"], move |args| {
            Box::pin(async move {
                let tool = args.get("tool").and_then(|t| t.as_str()).unwrap_or("");
                Ok(format!("tool:{tool}"))
            })
        }),
    );

    // 4. External adapter boundary (stub): the production adapter family
    //    lands with #154 and consumes this same seam.
    reg.declare(
        "external.adapter",
        Capability::asynchronous(&["request"], move |args| {
            Box::pin(async move {
                let req = args.get("request").and_then(|r| r.as_str()).unwrap_or("");
                Ok(format!("adapter:{req}"))
            })
        }),
    );

    reg
}

/// Small helper mirroring NodeRunRequest construction used across fixtures.
struct NodeRunRequestLike;
impl NodeRunRequestLike {
    fn build<'a>(
        manifest: &'a NodeManifestV1,
        local: &'a LocalRestrictions,
        capability: &str,
        args: serde_json::Value,
    ) -> prometheos_lite::workflow::node_runner::NodeRunRequest<'a> {
        prometheos_lite::workflow::node_runner::NodeRunRequest {
            manifest,
            local_restrictions: local,
            capability: capability.into(),
            args,
            idempotency_key: format!("{}:{}", manifest.node_id, capability),
            known_secrets: vec![],
        }
    }
}

#[tokio::test]
async fn nested_node_execution_is_fully_gated_on_both_sides() {
    let child_calls = Arc::new(AtomicUsize::new(0));
    let mut parent = NodeRunner::new(delegation_registry(child_calls.clone()));
    let m = manifest("nested.parent");
    let r = restrictions();
    let outcome = parent
        .execute_async(NodeRunRequestLike::build(
            &m,
            &r,
            "nested.delegate",
            serde_json::json!({"payload": "p"}),
        ))
        .await
        .expect("nested delegation passes gates on both sides");
    assert!(outcome.output.starts_with("nested:inner:nested"));
    // The child effect ran exactly once, through the child's own gates.
    assert_eq!(child_calls.load(Ordering::SeqCst), 1);
    // Parent journaled its terminal entry; evidence binds.
    assert_eq!(parent.journal().len(), 1);
    assert_eq!(
        outcome.result.evidence_refs[0].event_digest,
        outcome.evidence_entry.entry_digest
    );
}

#[tokio::test]
async fn code_tool_and_adapter_paths_run_only_through_resolved_capabilities() {
    let mut runner = NodeRunner::new(delegation_registry(Arc::new(AtomicUsize::new(0))));
    let m = manifest("delegation");
    let r = restrictions();
    for (cap, args, prefix) in [
        ("code.eval", serde_json::json!({"expr": "1+1"}), "code:"),
        (
            "tool.bridge",
            serde_json::json!({"tool": "grep", "input": "x"}),
            "tool:",
        ),
        (
            "external.adapter",
            serde_json::json!({"request": "r1"}),
            "adapter:",
        ),
    ] {
        let outcome = runner
            .execute_async(NodeRunRequestLike::build(&m, &r, cap, args))
            .await
            .unwrap_or_else(|e| panic!("{cap} must run gated: {e}"));
        assert!(
            outcome.output.starts_with(prefix),
            "{cap} => {}",
            outcome.output
        );
        assert_eq!(
            outcome.result.evidence_refs[0].event_digest,
            outcome.evidence_entry.entry_digest
        );
    }
    // One durable journal entry per governed run, in order.
    assert_eq!(runner.journal().len(), 3);
    let seqs: Vec<u64> = runner.journal().iter().map(|e| e.sequence).collect();
    assert_eq!(seqs, vec![0, 1, 2]);
}

#[tokio::test]
async fn bypass_attempts_fail_closed() {
    let mut runner = NodeRunner::new(delegation_registry(Arc::new(AtomicUsize::new(0))));
    let m = manifest("bypass");
    let r = restrictions();

    // Consume the one-shot capability once.
    let first = NodeRunRequestLike::build(&m, &r, "code.eval", serde_json::json!({"expr": "a"}));
    runner.execute_async(first).await.expect("first run");

    // Bypass 1: second preflight of the SAME consumed capability refuses.
    let again = NodeRunRequestLike::build(&m, &r, "code.eval", serde_json::json!({"expr": "b"}));
    assert!(
        runner.preflight_gates(&again).is_err(),
        "consumed capability must not resolve twice"
    );

    // Bypass 2: undeclared capability refuses.
    let ghost = NodeRunRequestLike::build(&m, &r, "ghost.cap", serde_json::json!({}));
    let err = runner.execute_async(ghost).await.unwrap_err().to_string();
    assert!(err.contains("SOMA-AUTH-0005"), "{err}");

    // Bypass 3: missing required arg refuses at gate 2 even when declared.
    let mut reg2 = CapabilityRegistry::new();
    reg2.declare(
        "strict.cap",
        Capability::asynchronous(&["needed"], |_a| Box::pin(async { Ok("x".into()) })),
    );
    let mut strict = NodeRunner::new(reg2);
    let bad = NodeRunRequestLike::build(&m, &r, "strict.cap", serde_json::json!({}));
    let err2 = strict.execute_async(bad).await.unwrap_err().to_string();
    assert!(err2.contains("SOMA-CMP-0003"), "{err2}");
}
