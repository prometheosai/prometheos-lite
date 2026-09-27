//! Cross-implementation golden tests for sealed governance plans.
//!
//! The goldens are emitted by the schema-matched oracle (soma-core
//! `022142b`, whose v1.1 `ExecutionPlan.schema.json` is byte-identical to
//! the vendored schema). Lite's sealed plan must be byte-identical to
//! those canonical bytes, and its seal digest must equal the oracle's.

use prometheos_lite::workflow::governance_compiler::compile_workflow_text;
use prometheos_lite::workflow::soma::canonical::try_canonical_bytes;
use serde_json::Value;

const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/soma-golden");

/// Golden name + workflow input path for each oracle case.
const CASES: &[(&str, &str)] = &[
    (
        "wf-valid-base",
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/vendored/soma/v1.1/fixtures/valid/wf-valid-base.json"
        ),
    ),
    (
        "wf-valid-composite",
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/vendored/soma/v1.1/fixtures/valid/wf-valid-composite.json"
        ),
    ),
    (
        "wf-valid-gov",
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/vendored/soma/v1.1/fixtures/valid/wf-valid-gov.json"
        ),
    ),
    (
        "lite-reorder",
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/soma-golden/lite-reorder.json"
        ),
    ),
];

fn read_golden_bytes(name: &str) -> Vec<u8> {
    std::fs::read(format!("{GOLDEN}/{name}.plan.canonical.json")).unwrap_or_else(|e| {
        panic!("{name}: golden missing ({e}) — generate with soma-cli@022142b `compile --out`")
    })
}

fn read_golden_digest(name: &str) -> String {
    std::fs::read_to_string(format!("{GOLDEN}/{name}.plan.sha256"))
        .unwrap_or_else(|e| {
            panic!("{name}: golden digest missing ({e}) — record the oracle digest")
        })
        .trim()
        .to_string()
}

#[test]
fn lite_plan_bytes_are_byte_identical_to_oracle_goldens() {
    for (name, workflow_path) in CASES {
        let text = std::fs::read_to_string(workflow_path)
            .unwrap_or_else(|e| panic!("{name}: workflow input missing ({e})"));
        let plan = compile_workflow_text(&text)
            .unwrap_or_else(|d| panic!("{name}: workflow does not compile: {d:?}"));
        let value = serde_json::to_value(&plan).expect("plan serializes");
        let actual = try_canonical_bytes(&value).expect("plan canonicalizes");
        let golden = read_golden_bytes(name);
        assert_eq!(
            actual, golden,
            "{name}: Lite plan bytes diverge from the oracle golden"
        );
        assert_eq!(
            plan.canonicalization.sha256,
            read_golden_digest(name),
            "{name}: Lite seal digest diverges from the oracle digest"
        );
    }
}

#[test]
fn oracle_goldens_are_canonical_and_self_consistent() {
    for (name, _) in CASES {
        let bytes = read_golden_bytes(name);
        let value: Value =
            serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            value["canonicalization"]["sha256"].as_str(),
            Some(read_golden_digest(name).as_str()),
            "{name}: recorded digest does not match the golden's seal field"
        );
        let recanonicalized = try_canonical_bytes(&value)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            recanonicalized, bytes,
            "{name}: oracle golden is not canonical bytes (byte-compare would be unsound)"
        );
    }
}
