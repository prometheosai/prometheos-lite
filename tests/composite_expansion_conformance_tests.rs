//! E4/X03 (#160) Slice B — deterministic composite → execution-plan expansion.
//!
//! Exercises `compile_workflow_text` on a two-level composite workflow: the
//! plan seals with explicit scope paths, compilation is deterministic and
//! order-significant, and audit-clean-but-escalating inputs fail before a plan.

use prometheos_lite::workflow::governance_compiler::compile_workflow_text;
use serde_json::Value;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/composite/nested-two-level.json"
);

fn fixture_text() -> String {
    std::fs::read_to_string(FIXTURE).expect("read fixture")
}

fn fixture_value() -> Value {
    serde_json::from_str(&fixture_text()).expect("parse fixture")
}

#[test]
fn composite_workflow_compiles_and_binds_scope_paths() {
    let plan = compile_workflow_text(&fixture_text()).expect("composite workflow must compile");
    // Every flattened operation is present; the innermost `leaf` carries its
    // full composite scope path (never a concatenated free-form id).
    let leaf = plan
        .steps
        .iter()
        .find(|s| s.operation_id == "leaf")
        .expect("leaf step present");
    assert_eq!(
        leaf.scope_path,
        vec!["flow".to_string(), "inner".to_string()]
    );
    // Root-level operations stay in the root scope (empty path).
    assert!(
        plan.steps
            .iter()
            .any(|s| s.operation_id == "prep" && s.scope_path.is_empty())
    );
}

#[test]
fn repeated_compilation_is_byte_identical() {
    let a = compile_workflow_text(&fixture_text()).expect("compile a");
    let b = compile_workflow_text(&fixture_text()).expect("compile b");
    assert_eq!(
        serde_json::to_string(&a).expect("a"),
        serde_json::to_string(&b).expect("b"),
        "repeated compilation must produce byte-identical plans"
    );
    assert_eq!(a.canonicalization.sha256, b.canonicalization.sha256);
}

#[test]
fn sibling_reorder_changes_the_source_bound_digest() {
    // Stated rule: expansion is a pure function of the canonical source bytes;
    // sibling order is source-significant, so reordering yields a different
    // (source-bound) sealed digest.
    let original = compile_workflow_text(&fixture_text()).expect("original");
    let mut v = fixture_value();
    let mut body = v["body"].as_array().expect("array").clone();
    body.reverse();
    v["body"] = Value::Array(body);
    let reordered =
        compile_workflow_text(&serde_json::to_string(&v).expect("ser")).expect("reordered");
    assert_ne!(
        original.canonicalization.sha256, reordered.canonicalization.sha256,
        "sibling reorder must change the source-bound digest"
    );
}

#[test]
fn authority_escalation_fails_before_plan_is_sealed() {
    let mut v = fixture_value();
    let flow = &mut v["body"][1];
    let inner = &mut flow["body"][0];
    inner["body"][0]["authority"] = serde_json::json!(["exfiltrate"]);
    let err =
        compile_workflow_text(&serde_json::to_string(&v).expect("ser")).expect_err("must fail");
    assert!(
        err.iter()
            .any(|d| d.code == "SOMA-AUTH-0002" || d.code == "SOMA-AUTH-0001"),
        "{err:?}"
    );
}

#[test]
fn undeclared_context_escape_fails_before_plan_is_sealed() {
    let mut v = fixture_value();
    let flow = &mut v["body"][1];
    let inner = &mut flow["body"][0];
    inner["context"] = serde_json::json!({ "discloses": [] });
    inner["body"][0]["context"] = serde_json::json!(["private-note"]);
    let err =
        compile_workflow_text(&serde_json::to_string(&v).expect("ser")).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "SOMA-EXP-0007"), "{err:?}");
}
