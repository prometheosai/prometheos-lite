//! E4/X03 (#160) Slice B — deterministic composite → execution-plan expansion.
//!
//! Exercises `compile_expansion_plan` (and the audit gate through
//! `compile_workflow_text`) on a two-level composite workflow: the
//! separately versioned expansion plan carries explicit scope paths,
//! typed boundary-port crossings derived from real producer→consumer
//! edges, and ancestor-chain restrictions; the sealed v1.1 plan keeps its
//! published `{key, operationId}` step shape; compilation is deterministic
//! and order-significant; and every discriminating negative control fails
//! before a plan is sealed.

use prometheos_lite::workflow::composite_expansion::{
    CrossingDirection, EXPANSION_PLAN_FORMAT, EXPANSION_PLAN_FORMAT_VERSION,
};
use prometheos_lite::workflow::governance_compiler::{
    compile_expansion_plan, compile_workflow_text,
};
use prometheos_lite::workflow::soma::contracts::WorkflowDefinition;
use prometheos_lite::workflow::soma::{supported_version, validate_artifact_text};
use serde_json::{Value, json};

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

/// Every mutation in this file must stay audit-clean so the asserted
/// refusal provably comes from the expansion layer / chain check, not
/// from a pre-expansion audit rule.
fn assert_audit_clean(text: &str) {
    let wf: WorkflowDefinition = serde_json::from_str(text).expect("parses");
    let diags = wf.audit(&supported_version());
    assert!(
        diags.is_empty(),
        "fixture mutation must stay audit-clean: {diags:?}"
    );
}

fn compile_err(text: &str) -> Vec<prometheos_lite::workflow::soma::Diagnostic> {
    compile_workflow_text(text).expect_err("must fail before a plan is sealed")
}

#[test]
fn composite_expansion_plan_carries_scope_paths_and_leaves_the_v1_1_plan_conformant() {
    let text = fixture_text();
    let plan = compile_workflow_text(&text).expect("composite workflow must compile");

    // The sealed v1.1 plan validates against the published ExecutionPlan
    // schema, whose steps deny additional members.
    let plan_text = serde_json::to_string(&plan).expect("plan serializes");
    let diags = validate_artifact_text("ExecutionPlan", &plan_text).expect("plan text validates");
    assert!(diags.is_empty(), "v1.1 plan must validate: {diags:?}");
    let value: Value = serde_json::from_str(&plan_text).expect("plan json");
    for step in value["steps"].as_array().expect("steps") {
        let mut keys: Vec<&str> = step
            .as_object()
            .expect("step object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["key", "operationId"],
            "published step shape must not grow members"
        );
    }

    // Composite scope metadata lives in the separately versioned
    // expansion plan, bound to the same source workflow digest.
    let expansion = compile_expansion_plan(&text).expect("expansion plan");
    assert_eq!(expansion.format, EXPANSION_PLAN_FORMAT);
    assert_eq!(expansion.format_version, EXPANSION_PLAN_FORMAT_VERSION);
    assert_eq!(expansion.workflow_digest, plan.workflow_digest);
    let leaf = expansion
        .steps
        .iter()
        .find(|s| s.operation_id == "leaf")
        .expect("leaf step present");
    assert_eq!(leaf.scope_path, ["flow", "inner"]);
    assert!(
        expansion
            .steps
            .iter()
            .any(|s| s.operation_id == "prep" && s.scope_path.is_empty()),
        "root-level operations stay in the root scope"
    );
    assert_eq!(
        expansion.steps.len(),
        3,
        "prep, leaf, and audit flatten; composites are scopes, not steps"
    );
}

#[test]
fn boundary_crossings_carry_scope_direction_and_suppress_internal_edges() {
    let text = fixture_text();
    let expansion = compile_expansion_plan(&text).expect("expansion plan");
    let step = |id: &str| {
        expansion
            .steps
            .iter()
            .find(|s| s.operation_id == id)
            .unwrap_or_else(|| panic!("{id} step present"))
    };

    // Root-boundary crossings carry an empty crossed_scope; nested ones
    // carry the declaring composite path.
    let prep_in = &step("prep").boundary_inputs;
    assert_eq!(prep_in.len(), 1);
    assert_eq!(prep_in[0].name, "order");
    assert_eq!(prep_in[0].ty, "Order");
    assert_eq!(prep_in[0].direction, CrossingDirection::In);
    assert!(prep_in[0].crossed_scope.is_empty());

    let audit_out = &step("audit").boundary_outputs;
    assert_eq!(audit_out.len(), 1);
    assert_eq!(audit_out[0].name, "shipment");
    assert_eq!(audit_out[0].direction, CrossingDirection::Out);
    assert!(audit_out[0].crossed_scope.is_empty());

    let leaf = step("leaf");
    assert_eq!(leaf.boundary_inputs.len(), 1);
    assert_eq!(leaf.boundary_inputs[0].name, "staged");
    assert_eq!(leaf.boundary_inputs[0].direction, CrossingDirection::In);
    assert_eq!(leaf.boundary_inputs[0].crossed_scope, ["flow", "inner"]);
    assert_eq!(leaf.boundary_outputs.len(), 1);
    assert_eq!(leaf.boundary_outputs[0].name, "ready");
    assert_eq!(leaf.boundary_outputs[0].direction, CrossingDirection::Out);
    assert_eq!(leaf.boundary_outputs[0].crossed_scope, ["flow", "inner"]);

    // Internal edges sharing a boundary-port name must not masquerade as
    // crossings: prep's `staged` output is consumed by flow in the same
    // scope even when the root declares a matching port.
    assert!(
        step("prep").boundary_outputs.is_empty(),
        "internal edge to flow's staged input must not masquerade as a crossing"
    );
    let mut v = fixture_value();
    let mut staged = v["outputPorts"][0].clone();
    staged["name"] = json!("staged");
    staged["direction"] = json!("output");
    staged["type"] = json!("Order");
    v["outputPorts"].as_array_mut().expect("array").push(staged);
    let mutated = serde_json::to_string(&v).expect("ser");
    assert_audit_clean(&mutated);
    let expansion = compile_expansion_plan(&mutated).expect("expansion plan");
    let prep = expansion
        .steps
        .iter()
        .find(|s| s.operation_id == "prep")
        .expect("prep");
    assert!(
        prep.boundary_outputs.iter().all(|c| c.name != "staged"),
        "a same-scope consumer suppresses the crossing even with a declared port"
    );
    assert!(
        prep.boundary_outputs.is_empty(),
        "prep declares no other outputs that could cross"
    );
}

#[test]
fn repeated_compilation_is_byte_identical() {
    let a = compile_expansion_plan(&fixture_text()).expect("compile a");
    let b = compile_expansion_plan(&fixture_text()).expect("compile b");
    assert_eq!(
        serde_json::to_string(&a).expect("a"),
        serde_json::to_string(&b).expect("b"),
        "repeated expansion must produce byte-identical plans"
    );
    let pa = compile_workflow_text(&fixture_text()).expect("plan a");
    let pb = compile_workflow_text(&fixture_text()).expect("plan b");
    assert_eq!(pa.canonicalization.sha256, pb.canonicalization.sha256);
}

#[test]
fn sibling_reorder_changes_the_source_bound_digest() {
    // Stated rule: expansion is a pure function of the canonical source
    // bytes; sibling order is source-significant, so reordering yields a
    // different source-bound digest.
    let original = compile_expansion_plan(&fixture_text()).expect("original");
    let mut v = fixture_value();
    let mut body = v["body"].as_array().expect("array").clone();
    body.reverse();
    v["body"] = Value::Array(body);
    let reordered =
        compile_expansion_plan(&serde_json::to_string(&v).expect("ser")).expect("reordered");
    assert_ne!(
        original.workflow_digest, reordered.workflow_digest,
        "sibling reorder must change the source-bound digest"
    );
}

#[test]
fn grandparent_imports_restrict_the_leaf_authority() {
    // `flow` is the leaf's grandparent (direct parent `inner` still imports
    // "ship"); empty grandparent imports must still restrict the leaf.
    let mut v = fixture_value();
    v["body"][1]["authorityImports"] = Value::Array(vec![]);
    let text = serde_json::to_string(&v).expect("ser");
    assert_audit_clean(&text);
    let err = compile_err(&text);
    assert!(
        err.iter().any(|d| d.code == "SOMA-AUTH-0002"),
        "grandparent imports must restrict the leaf: {err:?}"
    );
    assert!(
        err.iter().any(|d| d.message.contains("scope flow")),
        "message must name the restricting scope: {err:?}"
    );
}

#[test]
fn root_composite_imports_restrict_descendants() {
    // Root-kind composite with empty imports: every descendant grant is
    // restricted by the root level (`<root>`).
    let mut v = fixture_value();
    v["kind"] = json!("composite");
    v["authorityImports"] = json!([]);
    v["authority"]["tools"] = json!({ "ship": ["ship.v1"], "forge": ["forge.v1"] });
    let composite_a = json!({
        "schemaVersion": "1.1.0", "version": "1.1.0", "id": "A",
        "inputPorts": [
            { "name": "order", "direction": "input", "type": "Order", "cardinality": "single", "requiredness": "required" }
        ],
        "outputPorts": [
            { "name": "shipment", "direction": "output", "type": "Shipment", "cardinality": "single", "requiredness": "required" }
        ],
        "authorityImports": ["forge"],
        "body": [ {
            "schemaVersion": "1.1.0", "version": "1.1.0", "id": "leaf",
            "executionClass": "deterministic",
            "inputs": [
                { "name": "order", "type": "Order", "acceptedOutcomes": ["Produced"] }
            ],
            "outputs": [
                { "name": "shipment", "type": "Shipment", "emits": ["Produced"] }
            ],
            "authority": ["forge"], "effects": [], "uses": [], "secrets": [], "context": []
        } ]
    });
    v["body"] = json!([composite_a]);
    let text = serde_json::to_string(&v).expect("ser");
    assert_audit_clean(&text);
    let err = compile_err(&text);
    assert!(
        err.iter().any(|d| d.code == "SOMA-AUTH-0002"),
        "root composite imports must restrict descendants: {err:?}"
    );
    assert!(
        err.iter().any(|d| d.message.contains("<root>")),
        "message must name the root scope: {err:?}"
    );
}

#[test]
fn grandparent_effect_export_restricts_the_leaf() {
    let mut v = fixture_value();
    v["body"][1]["effectExports"] = json!([]);
    v["body"][1]["body"][0]["body"][0]["effects"] = json!([{ "name": "emitX" }]);
    let text = serde_json::to_string(&v).expect("ser");
    assert_audit_clean(&text);
    let err = compile_err(&text);
    assert!(
        err.iter().any(|d| d.code == "SOMA-EXP-0007"),
        "grandparent effect exports must restrict the leaf: {err:?}"
    );
    assert!(
        err.iter()
            .any(|d| d.message.contains("emitX") && d.message.contains("scope flow")),
        "message must name the effect and the restricting scope: {err:?}"
    );
}

#[test]
fn grandparent_context_disclosure_restricts_the_leaf() {
    let mut v = fixture_value();
    v["body"][1]["context"] = json!({ "discloses": [] });
    v["body"][1]["body"][0]["body"][0]["context"] = json!(["private-note"]);
    let text = serde_json::to_string(&v).expect("ser");
    assert_audit_clean(&text);
    let err = compile_err(&text);
    assert!(
        err.iter().any(|d| d.code == "SOMA-EXP-0007"),
        "grandparent disclosures must restrict the leaf: {err:?}"
    );
    assert!(
        err.iter()
            .any(|d| d.message.contains("private-note") && d.message.contains("scope flow")),
        "message must name the context and the restricting scope: {err:?}"
    );
}

#[test]
fn sibling_type_mismatch_is_refused_by_expansion() {
    // `Order` producer feeding a `Shipment` consumer inside a composite
    // (the expansion only runs for composite bodies).
    let workflow = json!({
        "schemaVersion": "1.1.0", "version": "1.1.0",
        "id": "wf-sib", "name": "Sib", "kind": "atomic",
        "inputPorts": [], "outputPorts": [],
        "body": [ {
            "schemaVersion": "1.1.0", "version": "1.1.0", "id": "C",
            "inputPorts": [], "outputPorts": [], "authorityImports": [],
            "body": [
                {
                    "schemaVersion": "1.1.0", "version": "1.1.0", "id": "p",
                    "executionClass": "deterministic",
                    "inputs": [],
                    "outputs": [{ "name": "x", "type": "Order", "emits": ["Produced"] }],
                    "authority": [], "effects": [], "uses": [], "secrets": [], "context": []
                },
                {
                    "schemaVersion": "1.1.0", "version": "1.1.0", "id": "c",
                    "executionClass": "deterministic",
                    "inputs": [{ "name": "x", "type": "Shipment", "acceptedOutcomes": ["Produced"] }],
                    "outputs": [],
                    "authority": [], "effects": [], "uses": [], "secrets": [], "context": []
                }
            ]
        } ],
        "authority": { "executionClass": "deterministic", "mutation": "none" }
    });
    let text = workflow.to_string();
    assert_audit_clean(&text);
    let err = compile_err(&text);
    assert!(
        err.iter().any(|d| d.code == "SOMA-CMP-0005"),
        "sibling type mismatch must fail closed: {err:?}"
    );
    assert!(
        err.iter()
            .any(|d| d.message.contains("sibling type mismatch")),
        "message must name the refusal: {err:?}"
    );
}

#[test]
fn composite_input_port_type_mismatch_is_refused_by_expansion() {
    // Same discrimination against a composite INPUT PORT, not only an
    // operation input.
    let workflow = json!({
        "schemaVersion": "1.1.0", "version": "1.1.0",
        "id": "wf-sib-port", "name": "SibPort", "kind": "atomic",
        "inputPorts": [], "outputPorts": [],
        "body": [ {
            "schemaVersion": "1.1.0", "version": "1.1.0", "id": "Outer",
            "inputPorts": [], "outputPorts": [], "authorityImports": [],
            "body": [
                {
                    "schemaVersion": "1.1.0", "version": "1.1.0", "id": "p",
                    "executionClass": "deterministic",
                    "inputs": [],
                    "outputs": [{ "name": "x", "type": "Order", "emits": ["Produced"] }],
                    "authority": [], "effects": [], "uses": [], "secrets": [], "context": []
                },
                {
                    "schemaVersion": "1.1.0", "version": "1.1.0", "id": "D",
                    "inputPorts": [
                        { "name": "x", "direction": "input", "type": "Shipment", "cardinality": "single", "requiredness": "required" }
                    ],
                    "outputPorts": [], "authorityImports": [],
                    "body": [ {
                        "schemaVersion": "1.1.0", "version": "1.1.0", "id": "c",
                        "executionClass": "deterministic",
                        "inputs": [{ "name": "x", "type": "Shipment", "acceptedOutcomes": ["Produced"] }],
                        "outputs": [],
                        "authority": [], "effects": [], "uses": [], "secrets": [], "context": []
                    } ]
                }
            ]
        } ],
        "authority": { "executionClass": "deterministic", "mutation": "none" }
    });
    let text = workflow.to_string();
    assert_audit_clean(&text);
    let err = compile_err(&text);
    assert!(
        err.iter().any(|d| d.code == "SOMA-CMP-0005"),
        "composite input port type mismatch must fail closed: {err:?}"
    );
}

#[test]
fn ambiguous_producer_is_refused_by_the_public_compiler() {
    // Two sibling producers of the same name: audit-clean, refused by the
    // expansion (reachable through the public compiler entrypoint).
    let workflow = json!({
        "schemaVersion": "1.1.0", "version": "1.1.0",
        "id": "wf-amb", "name": "Amb", "kind": "atomic",
        "inputPorts": [], "outputPorts": [],
        "body": [ {
            "schemaVersion": "1.1.0", "version": "1.1.0", "id": "C",
            "inputPorts": [], "outputPorts": [], "authorityImports": [],
            "body": [
                {
                    "schemaVersion": "1.1.0", "version": "1.1.0", "id": "p1",
                    "executionClass": "deterministic",
                    "inputs": [],
                    "outputs": [{ "name": "dup", "type": "Order", "emits": ["Produced"] }],
                    "authority": [], "effects": [], "uses": [], "secrets": [], "context": []
                },
                {
                    "schemaVersion": "1.1.0", "version": "1.1.0", "id": "p2",
                    "executionClass": "deterministic",
                    "inputs": [],
                    "outputs": [{ "name": "dup", "type": "Order", "emits": ["Produced"] }],
                    "authority": [], "effects": [], "uses": [], "secrets": [], "context": []
                },
                {
                    "schemaVersion": "1.1.0", "version": "1.1.0", "id": "c",
                    "executionClass": "deterministic",
                    "inputs": [{ "name": "dup", "type": "Order", "acceptedOutcomes": ["Produced"] }],
                    "outputs": [],
                    "authority": [], "effects": [], "uses": [], "secrets": [], "context": []
                }
            ]
        } ],
        "authority": { "executionClass": "deterministic", "mutation": "none" }
    });
    let text = workflow.to_string();
    assert_audit_clean(&text);
    let err = compile_err(&text);
    assert!(
        err.iter().any(|d| d.code == "SOMA-CMP-0003"),
        "ambiguous producer must fail closed: {err:?}"
    );
    assert!(
        err.iter().any(|d| d.message.contains("ambiguous producer")),
        "message must name the refusal: {err:?}"
    );
}

#[test]
fn public_cycle_is_refused_by_the_audit_gate() {
    // A cycle reaches the audit gate first (SOMA-EXP-0002) through the
    // public compiler entrypoint; the expansion-layer refusal is exercised
    // as a unit test in `composite_expansion`.
    let workflow = json!({
        "schemaVersion": "1.1.0", "version": "1.1.0",
        "id": "wf-cycle", "name": "Cycle", "kind": "atomic",
        "inputPorts": [], "outputPorts": [],
        "body": [
            {
                "schemaVersion": "1.1.0", "version": "1.1.0", "id": "a",
                "executionClass": "deterministic",
                "inputs": [{ "name": "n", "type": "Order", "acceptedOutcomes": ["Produced"] }],
                "outputs": [{ "name": "m", "type": "Order", "emits": ["Produced"] }],
                "authority": [], "effects": [], "uses": [], "secrets": [], "context": []
            },
            {
                "schemaVersion": "1.1.0", "version": "1.1.0", "id": "b",
                "executionClass": "deterministic",
                "inputs": [{ "name": "m", "type": "Order", "acceptedOutcomes": ["Produced"] }],
                "outputs": [{ "name": "n", "type": "Order", "emits": ["Produced"] }],
                "authority": [], "effects": [], "uses": [], "secrets": [], "context": []
            }
        ],
        "authority": { "executionClass": "deterministic", "mutation": "none" }
    });
    let text = workflow.to_string();
    let wf: WorkflowDefinition = serde_json::from_str(&text).expect("parses");
    let audit = wf.audit(&supported_version());
    assert!(
        audit.iter().any(|d| d.code == "SOMA-EXP-0002"),
        "audit must refuse the cycle: {audit:?}"
    );
    let err = compile_err(&text);
    assert!(
        err.iter().any(|d| d.code == "SOMA-EXP-0002"),
        "public compiler must refuse the cycle: {err:?}"
    );
}

#[test]
fn public_duplicate_id_is_refused_by_the_audit_gate() {
    // Duplicate ids anywhere in the document are refused by the audit
    // (SOMA-EXP-0003) before expansion; local-id reuse across scopes is
    // explicitly not part of the expansion contract.
    let workflow = json!({
        "schemaVersion": "1.1.0", "version": "1.1.0",
        "id": "wf-dup", "name": "Dup", "kind": "atomic",
        "inputPorts": [], "outputPorts": [],
        "body": [
            {
                "schemaVersion": "1.1.0", "version": "1.1.0", "id": "x",
                "executionClass": "deterministic",
                "inputs": [], "outputs": [],
                "authority": [], "effects": [], "uses": [], "secrets": [], "context": []
            },
            {
                "schemaVersion": "1.1.0", "version": "1.1.0", "id": "x",
                "executionClass": "deterministic",
                "inputs": [], "outputs": [],
                "authority": [], "effects": [], "uses": [], "secrets": [], "context": []
            }
        ],
        "authority": { "executionClass": "deterministic", "mutation": "none" }
    });
    let text = workflow.to_string();
    let wf: WorkflowDefinition = serde_json::from_str(&text).expect("parses");
    let audit = wf.audit(&supported_version());
    assert!(
        audit.iter().any(|d| d.code == "SOMA-EXP-0003"),
        "audit must refuse the duplicate id: {audit:?}"
    );
    let err = compile_err(&text);
    assert!(
        err.iter().any(|d| d.code == "SOMA-EXP-0003"),
        "public compiler must refuse the duplicate id: {err:?}"
    );
}

#[test]
fn atomic_workflow_has_no_expansion_plan() {
    let workflow = json!({
        "schemaVersion": "1.1.0", "version": "1.1.0",
        "id": "wf-atomic", "name": "Atomic", "kind": "atomic",
        "inputPorts": [
            { "name": "seed", "direction": "input", "type": "Order", "cardinality": "single", "requiredness": "required" }
        ],
        "outputPorts": [],
        "body": [ {
            "schemaVersion": "1.1.0", "version": "1.1.0", "id": "only",
            "executionClass": "deterministic",
            "inputs": [{ "name": "seed", "type": "Order", "acceptedOutcomes": ["Produced"] }],
            "outputs": [],
            "authority": [], "effects": [], "uses": [], "secrets": [], "context": []
        } ],
        "authority": { "executionClass": "deterministic", "mutation": "none" }
    });
    let text = workflow.to_string();
    compile_workflow_text(&text).expect("atomic workflow compiles");
    let err = compile_expansion_plan(&text)
        .expect_err("expansion metadata exists only for composite bodies");
    assert!(err.iter().any(|d| d.code == "SOMA-CMP-0003"), "{err:?}");
}

#[test]
fn authority_escalation_fails_before_plan_is_sealed() {
    let mut v = fixture_value();
    let flow = &mut v["body"][1];
    let inner = &mut flow["body"][0];
    inner["body"][0]["authority"] = serde_json::json!(["exfiltrate"]);
    let err = compile_err(&serde_json::to_string(&v).expect("ser"));
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
    let err = compile_err(&serde_json::to_string(&v).expect("ser"));
    assert!(err.iter().any(|d| d.code == "SOMA-EXP-0007"), "{err:?}");
}
