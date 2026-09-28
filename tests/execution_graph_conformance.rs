//! Conformance for the Lite-owned execution graph (review correction 1).
//!
//! Topology, dependency, and reduced-authority data live in this versioned
//! Lite structure — never in the SOMA v1.1 plan shape, whose canonical bytes
//! stay byte-identical to the published schema.

use prometheos_lite::workflow::execution_graph::{
    EXECUTION_GRAPH_SCHEMA_VERSION, compile_execution_graph,
};
use prometheos_lite::workflow::governance_compiler::compile_workflow_text;
use prometheos_lite::workflow::soma::contracts::AuthorityProfile;
use serde_json::Value;

const VENDORED: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/vendored/soma/v1.1");

fn fixture(rel: &str) -> String {
    std::fs::read_to_string(format!("{VENDORED}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn reorder_text() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/soma-golden/lite-reorder.json"
    ))
    .expect("lite-reorder fixture exists")
}

/// The base workflow with a per-unit patch applied before graph compilation.
fn base_with_unit_patch(patch: impl FnOnce(&mut Value)) -> String {
    let mut value: Value =
        serde_json::from_str(&fixture("fixtures/valid/wf-valid-base.json")).expect("base parses");
    let unit = value["body"]
        .as_array_mut()
        .expect("body is an array")
        .first_mut()
        .expect("body has a unit");
    patch(unit);
    serde_json::to_string(&value).expect("patched workflow serializes")
}

#[test]
fn graph_binds_workflow_digest_and_plan_identity() {
    let text = reorder_text();
    let plan = compile_workflow_text(&text).expect("fixture compiles");
    let graph =
        compile_execution_graph(&text, &plan.canonicalization.sha256).expect("graph compiles");

    assert_eq!(graph.schema_version, EXECUTION_GRAPH_SCHEMA_VERSION);
    assert_eq!(graph.schema_version, "lite.execution-graph.v1");
    assert_eq!(graph.workflow_digest, plan.workflow_digest);
    assert_eq!(graph.plan_identity, plan.canonicalization.sha256);
}

#[test]
fn graph_steps_carry_order_dependencies_keys_and_granted() {
    let text = reorder_text();
    let plan = compile_workflow_text(&text).expect("fixture compiles");
    let graph =
        compile_execution_graph(&text, &plan.canonicalization.sha256).expect("graph compiles");

    assert_eq!(graph.steps.len(), 2);

    // op2 is declared first but consumes `shipment`, so op1 must run first.
    let first = &graph.steps[0];
    assert_eq!(first.operation_id, "op1");
    assert_eq!(first.key, "s0000:op1");
    assert_eq!(first.order, 0);
    assert!(first.dependencies.is_empty());

    let second = &graph.steps[1];
    assert_eq!(second.operation_id, "op2");
    assert_eq!(second.key, "s0001:op2");
    assert_eq!(second.order, 1);
    assert_eq!(second.dependencies, ["op1"]);

    // Granted authority is the unit reduction against the workflow ceiling:
    // the `ship` capability with its ceiling-declared versions, no scope
    // grants (the unit declares none), no secrets (the unit declares none).
    let mut expected_tools = std::collections::BTreeMap::new();
    expected_tools.insert("ship".to_string(), vec!["ship.v1".to_string()]);
    for step in &graph.steps {
        assert_eq!(step.granted.tools.as_ref(), Some(&expected_tools));
        assert!(step.granted.readable_scopes.is_none());
        assert!(step.granted.writable_scopes.is_none());
        assert!(step.granted.secrets.is_none());
        assert!(step.granted.network_policy.is_none());
        assert!(step.granted.provider_policy.is_none());
        assert!(step.granted.escalation.is_none());
        assert!(step.granted.review.is_none());
        assert!(step.granted.abstention.is_none());
        assert!(step.granted.budgets.is_none());
        assert!(step.granted.content_restrictions.is_none());
    }
}

#[test]
fn graph_is_deterministic_and_round_trips() {
    let text = reorder_text();
    let plan = compile_workflow_text(&text).expect("fixture compiles");
    let identity = plan.canonicalization.sha256.as_str();

    let first = compile_execution_graph(&text, identity).expect("graph compiles");
    let second = compile_execution_graph(&text, identity).expect("graph compiles");

    let bytes_a = serde_json::to_string(&first).expect("serializes");
    let bytes_b = serde_json::to_string(&second).expect("serializes");
    assert_eq!(bytes_a, bytes_b, "two compiles serialize identically");

    let parsed: prometheos_lite::workflow::execution_graph::CompiledExecutionGraphV1 =
        serde_json::from_str(&bytes_a).expect("deserializes");
    assert_eq!(parsed, first, "round trip preserves the graph");
}

#[test]
fn graph_refuses_scope_grants_outside_the_ceiling() {
    let text = base_with_unit_patch(|unit| {
        unit["authority"] = serde_json::json!(["ship", "readable:warehouse"]);
    });
    let err = compile_execution_graph(&text, &"ab".repeat(32))
        .expect_err("undeclared scope must fail closed");
    assert_eq!(err[0].code, "SOMA-AUTH-0003");
    assert!(
        err[0].message.contains("readable scope"),
        "{}",
        err[0].message
    );
}

#[test]
fn graph_refuses_empty_capability_grants() {
    let text = base_with_unit_patch(|unit| {
        unit["authority"] = serde_json::json!(["ship", ""]);
    });
    let err =
        compile_execution_graph(&text, &"ab".repeat(32)).expect_err("empty grant must fail closed");
    assert_eq!(err[0].code, "SOMA-AUTH-0001");
    assert!(
        err[0].message.contains("empty capability"),
        "{}",
        err[0].message
    );
}

#[test]
fn graph_refuses_undeclared_secrets() {
    let text = base_with_unit_patch(|unit| {
        unit["secrets"] = serde_json::json!(["ghost"]);
    });
    let err = compile_execution_graph(&text, &"ab".repeat(32))
        .expect_err("undeclared secret must fail closed");
    assert_eq!(err[0].code, "SOMA-AUTH-0005");
    assert!(err[0].message.contains("secret"), "{}", err[0].message);
}

#[test]
fn graph_refuses_cyclic_dataflow() {
    let text = fixture("fixtures/valid/wf-valid-base.json");
    let mut value: Value = serde_json::from_str(&text).expect("base parses");
    value["body"] = serde_json::json!([
        {
            "schemaVersion": "1.1.0",
            "version": "1.1.0",
            "id": "a",
            "executionClass": "deterministic",
            "inputs": [
                {"name": "flow-b", "type": "T", "acceptedOutcomes": ["Produced"]}
            ],
            "outputs": [
                {"name": "flow-a", "type": "T", "emits": ["Produced"]}
            ]
        },
        {
            "schemaVersion": "1.1.0",
            "version": "1.1.0",
            "id": "b",
            "executionClass": "deterministic",
            "inputs": [
                {"name": "flow-a", "type": "T", "acceptedOutcomes": ["Produced"]}
            ],
            "outputs": [
                {"name": "flow-b", "type": "T", "emits": ["Produced"]}
            ]
        }
    ]);
    let cyclic = serde_json::to_string(&value).expect("serializes");
    let err =
        compile_execution_graph(&cyclic, &"ab".repeat(32)).expect_err("a cycle must fail closed");
    assert_eq!(err[0].code, "SOMA-EXP-0002");
}

#[test]
fn reduced_grant_profiles_answer_is_empty_like_the_reference() {
    let empty: AuthorityProfile =
        serde_json::from_str(r#"{"executionClass":"deterministic","mutation":"none"}"#)
            .expect("minimal profile parses");
    assert!(empty.is_empty(), "no grants means an empty reduction");

    let text = reorder_text();
    let plan = compile_workflow_text(&text).expect("fixture compiles");
    let graph =
        compile_execution_graph(&text, &plan.canonicalization.sha256).expect("graph compiles");
    assert!(
        !graph.steps[0].granted.is_empty(),
        "the reduced profile carries the ceiling-declared tool versions"
    );
}
