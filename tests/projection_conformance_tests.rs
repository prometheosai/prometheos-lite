//! Conformance for the deterministic projection subsystem (E4/X07 Slice 1).

use prometheos_lite::workflow::projection::{PROJECTION_VERSION_V1, VersionedProjectionEnvelope};

#[test]
fn envelope_exposes_projection_v1_and_nests_payload() {
    assert_eq!(PROJECTION_VERSION_V1, "projection.v1");
    let env = VersionedProjectionEnvelope {
        projection_version: PROJECTION_VERSION_V1.to_string(),
        schema_version: "1.1.0".to_string(),
        source_digest: "a".repeat(64),
        projection_digest: "b".repeat(64),
        payload: serde_json::json!({"id": "wf-base"}),
    };
    let value = serde_json::to_value(&env).expect("envelope serializes");
    let obj = value.as_object().expect("envelope is a JSON object");
    for key in [
        "projectionVersion",
        "schemaVersion",
        "sourceDigest",
        "projectionDigest",
        "payload",
    ] {
        assert!(obj.contains_key(key), "missing envelope key {key}");
    }
    assert_eq!(
        obj.len(),
        5,
        "payload must be one nested key, not flattened"
    );
    assert_eq!(
        obj["payload"]["id"],
        serde_json::json!("wf-base"),
        "payload must nest its value under `payload`"
    );
}

#[test]
fn envelope_rejects_unknown_fields() {
    let mut value = serde_json::json!({
        "projectionVersion": "projection.v1",
        "schemaVersion": "1.1.0",
        "sourceDigest": "a".repeat(64),
        "projectionDigest": "b".repeat(64),
        "payload": null
    });
    value["extra"] = serde_json::json!(1);
    let parsed = serde_json::from_value::<VersionedProjectionEnvelope<serde_json::Value>>(value);
    assert!(parsed.is_err(), "unknown envelope field must fail closed");
}

use prometheos_lite::workflow::governance_compiler::compile_workflow_text;
use prometheos_lite::workflow::projection::{project_canonical_json, validated_source};
use prometheos_lite::workflow::soma::contracts::WorkflowDefinition;

const VENDORED: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/vendored/soma/v1.1");

fn fixture(rel: &str) -> String {
    std::fs::read_to_string(format!("{VENDORED}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn base_text() -> String {
    fixture("fixtures/valid/wf-valid-base.json")
}

fn base_wf() -> WorkflowDefinition {
    serde_json::from_str(&base_text()).expect("base fixture parses")
}

/// Same AST content built from JSON texts whose `tools` object keys appear
/// in seed-dependent textual insertion orders; serde's BTreeMap-backed Map
/// must converge them to one canonical projection.
fn seed_wf(seed: usize) -> WorkflowDefinition {
    let mut value: serde_json::Value = serde_json::from_str(&base_text()).expect("base parses");
    let tools_text = match seed % 3 {
        0 => r#"{"ship":["ship.v1"],"archive":["archive.v1"],"alpha":["alpha.v1"]}"#,
        1 => r#"{"alpha":["alpha.v1"],"ship":["ship.v1"],"archive":["archive.v1"]}"#,
        _ => r#"{"archive":["archive.v1"],"alpha":["alpha.v1"],"ship":["ship.v1"]}"#,
    };
    value["authority"]["tools"] = serde_json::from_str(tools_text).expect("tools value");
    serde_json::from_value(value).expect("seeded workflow parses")
}

/// Base fixture with a purpose that embeds the secret canary (redaction seed).
/// Consumed by the disclosure-policy task of this slice; allowed dead code
/// until that task appends its tests.
#[allow(dead_code)]
fn redacted_seed_text() -> String {
    let mut value: serde_json::Value = serde_json::from_str(&base_text()).expect("base parses");
    value["purpose"] = serde_json::json!(format!(
        "handle orders for {}",
        prometheos_lite::workflow::redaction::SECRET_CANARY
    ));
    serde_json::to_string(&value).expect("seed serializes")
}

#[test]
fn canonical_projection_is_byte_deterministic_across_seeds() {
    let mut ref_bytes: Option<Vec<u8>> = None;
    let mut ref_digests: Option<(String, String)> = None;
    for seed in 0..10usize {
        let wf = seed_wf(seed);
        let env = project_canonical_json(&wf).expect("seed projects");
        let bytes = env.canonical_bytes().expect("canonical bytes");
        match (&ref_bytes, &ref_digests) {
            (None, None) => {
                ref_bytes = Some(bytes);
                ref_digests = Some((env.source_digest.clone(), env.projection_digest.clone()));
            }
            (Some(rb), Some((sd, pd))) => {
                assert_eq!(&bytes, rb, "seed {seed} produced different envelope bytes");
                assert_eq!(&env.source_digest, sd, "seed {seed} source digest differs");
                assert_eq!(
                    &env.projection_digest, pd,
                    "seed {seed} projection digest differs"
                );
            }
            _ => unreachable!("both set together"),
        }
    }
}

#[test]
fn one_byte_semantic_change_changes_both_digests() {
    let a = base_wf();
    let mut b = base_wf();
    b.name = "Bass".to_string(); // "Base" -> "Bass": exactly one byte differs
    let ea = project_canonical_json(&a).expect("a projects");
    let eb = project_canonical_json(&b).expect("b projects");
    assert_ne!(
        ea.source_digest, eb.source_digest,
        "source digest must change"
    );
    assert_ne!(
        ea.projection_digest, eb.projection_digest,
        "projection digest must change"
    );
}

#[test]
fn source_digest_matches_workflow_digest_of() {
    let plan = compile_workflow_text(&base_text()).expect("base compiles");
    let env = project_canonical_json(&base_wf()).expect("projects");
    assert_eq!(env.source_digest, plan.workflow_digest);
    assert_eq!(env.projection_version, "projection.v1");
    assert_eq!(env.schema_version, "1.1.0");
}

#[test]
fn projection_input_requires_validated_ast() {
    // Wrong schema version: audit reports SOMA-CMP-0001, gate refuses.
    let mut bad = base_wf();
    bad.schema_version = "0.9.0".to_string();
    let err = project_canonical_json(&bad).expect_err("wrong schema must be refused");
    assert!(
        err.iter()
            .any(|d| d.code == "SOMA-CMP-0001" || d.code == "PROJ-0001"),
        "expected a version refusal, got {err:?}"
    );
    assert!(
        validated_source(&bad).is_err(),
        "gate must refuse directly too"
    );

    // Broken contentDigest: audit reports SOMA-CMP-0004, gate refuses.
    let mut value: serde_json::Value = serde_json::from_str(&base_text()).unwrap();
    value["contentDigest"] = serde_json::json!("0".repeat(64));
    let bad: WorkflowDefinition = serde_json::from_value(value).expect("parses");
    let err = project_canonical_json(&bad).expect_err("bad contentDigest must be refused");
    assert!(
        err.iter().any(|d| d.code == "SOMA-CMP-0004"),
        "expected a digest refusal, got {err:?}"
    );

    // Sanity: the honest fixture passes the gate.
    assert!(validated_source(&base_wf()).is_ok());
}

use prometheos_lite::workflow::projection::{
    verify_canonical_projection_bytes, verify_projection_against_source,
};
use prometheos_lite::workflow::soma::canonical::{try_canonical_bytes, try_canonical_digest};
use serde_json::Value;

fn honest_canonical_bytes() -> Vec<u8> {
    project_canonical_json(&base_wf())
        .expect("projects")
        .canonical_bytes()
        .expect("canonical bytes")
}

fn rekeyed(raw: &[u8], mutate: impl FnOnce(&mut Value)) -> Vec<u8> {
    let mut value: Value = serde_json::from_slice(raw).expect("parses");
    mutate(&mut value);
    try_canonical_bytes(&value).expect("re-canonicalized")
}

#[test]
fn verify_accepts_honest_canonical_bytes() {
    let bytes = honest_canonical_bytes();
    let verified = verify_canonical_projection_bytes(&bytes).expect("honest bytes verify");
    let env = project_canonical_json(&base_wf()).expect("projects");
    assert_eq!(verified, env);
    assert!(verify_projection_against_source(&verified, &base_wf()).is_ok());
}

#[test]
fn tampered_payload_fails_projection_digest_check() {
    let bytes = honest_canonical_bytes();
    let tampered = rekeyed(&bytes, |v| {
        v["payload"]["name"] = Value::String("Bast".to_string());
    });
    let err =
        verify_canonical_projection_bytes(&tampered).expect_err("payload tamper must fail closed");
    assert_eq!(err[0].code, "SOMA-CMP-0004", "got {err:?}");
}

#[test]
fn tampered_source_identity_fails_against_source() {
    let bytes = honest_canonical_bytes();
    let tampered = rekeyed(&bytes, |v| {
        v["sourceDigest"] = Value::String("f".repeat(64));
    });
    // Structurally valid: the payload digest still matches.
    let env = verify_canonical_projection_bytes(&tampered)
        .expect("identity tamper survives structural checks");
    let err = verify_projection_against_source(&env, &base_wf())
        .expect_err("identity tamper must fail against the source AST");
    assert_eq!(err[0].code, "PROJ-0002", "got {err:?}");
}

#[test]
fn co_tampered_canonical_payload_fails_against_source() {
    // Attacker edits payload AND recomputes projectionDigest: structural
    // verify passes; the source-equality check still fails closed.
    let bytes = honest_canonical_bytes();
    let tampered = rekeyed(&bytes, |v| {
        v["payload"]["name"] = Value::String("Bast".to_string());
        let digest = try_canonical_digest(&v["payload"]).expect("tampered payload digests");
        v["projectionDigest"] = Value::String(digest);
    });
    let env = verify_canonical_projection_bytes(&tampered).expect("structurally consistent");
    let err = verify_projection_against_source(&env, &base_wf())
        .expect_err("payload divergence must fail against the source AST");
    assert_eq!(err[0].code, "PROJ-0002", "got {err:?}");
}

#[test]
fn tampered_version_fails_closed() {
    // Built honestly at the envelope level, but with an unsupported version.
    let mut env = project_canonical_json(&base_wf()).expect("projects");
    env.projection_version = "projection.v2".to_string();
    let bytes = env.canonical_bytes().expect("bytes");
    let err =
        verify_canonical_projection_bytes(&bytes).expect_err("projection.v2 must fail closed");
    assert_eq!(err[0].code, "SOMA-CMP-0001", "got {err:?}");
}

#[test]
fn non_canonical_bytes_fail_closed_on_read() {
    let mut raw = honest_canonical_bytes();
    let idx = raw.iter().position(|&c| c == b'{').expect("object opens");
    raw.insert(idx + 1, b' '); // whitespace the canonical renderer never emits
    let err =
        verify_canonical_projection_bytes(&raw).expect_err("non-canonical bytes must fail closed");
    assert_eq!(err[0].code, "PROJ-0001", "got {err:?}");
}

#[test]
fn duplicate_keys_fail_closed_on_read() {
    let text = String::from_utf8(honest_canonical_bytes()).expect("utf-8");
    let injected = text.replace("\"payload\"", "\"schemaVersion\":\"1.1.0\",\"payload\"");
    let err = verify_canonical_projection_bytes(injected.as_bytes())
        .expect_err("duplicate key must fail closed");
    assert_eq!(err[0].code, "PROJ-0001", "got {err:?}");
}

#[test]
fn unknown_fields_fail_closed_on_read() {
    let text = String::from_utf8(honest_canonical_bytes()).expect("utf-8");
    let injected = text.replace("\"payload\"", "\"extra\":1,\"payload\"");
    let err = verify_canonical_projection_bytes(injected.as_bytes())
        .expect_err("unknown field must fail closed");
    assert_eq!(err[0].code, "PROJ-0001", "got {err:?}");
}

#[test]
fn projection_data_cannot_add_canonical_fields() {
    // Negative: projection data cannot smuggle authority or fields the
    // source AST does not declare.
    let bytes = honest_canonical_bytes();
    let tampered = rekeyed(&bytes, |v| {
        v["payload"]["authority"] = serde_json::json!({"tools": {"root.shell": ["root.v1"]}});
    });
    let err = verify_canonical_projection_bytes(&tampered)
        .expect_err("added payload field must fail the digest check");
    assert_eq!(err[0].code, "SOMA-CMP-0004", "got {err:?}");

    // And even with a consistent digest, the source equality refuses it.
    let env = verify_canonical_projection_bytes(&honest_canonical_bytes()).expect("honest");
    let mut env = env;
    env.payload["authority"] = serde_json::json!({"tools": {"root.shell": ["root.v1"]}});
    let err = verify_projection_against_source(&env, &base_wf())
        .expect_err("added authority must fail against the source AST");
    assert_eq!(err[0].code, "PROJ-0002", "got {err:?}");
}
