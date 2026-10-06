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
    env.projection_digest = try_canonical_digest(&env.payload).expect("mutated payload digests");
    let err = verify_projection_against_source(&env, &base_wf())
        .expect_err("added authority must fail against the source AST");
    assert_eq!(err[0].code, "PROJ-0002", "got {err:?}");
}

// `compile_workflow_text` is already imported above.
use prometheos_lite::workflow::execution_graph::compile_execution_graph;
use prometheos_lite::workflow::projection::project_human_plan;

fn reorder_text() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/soma-golden/lite-reorder.json"
    ))
    .expect("lite-reorder fixture exists")
}

fn golden_path(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR")))
        .join("tests/projection_golden")
        .join(format!("{name}.human.txt"))
}

#[test]
fn human_order_matches_compiler_topological_order() {
    let text = reorder_text();
    let wf: WorkflowDefinition = serde_json::from_str(&text).expect("parses");
    let plan = compile_workflow_text(&text).expect("compiles");
    let graph =
        compile_execution_graph(&text, &plan.canonicalization.sha256).expect("graph compiles");

    let env = project_human_plan(&wf, None).expect("projects");
    let headers: Vec<String> = env
        .payload
        .lines()
        .filter(|l| l.starts_with("### s"))
        .map(|l| l.split(" [").next().expect("header shape").to_string())
        .collect();
    let expected: Vec<String> = graph
        .steps
        .iter()
        .map(|s| format!("### {}", s.key))
        .collect();
    assert_eq!(headers, expected, "human order must equal compiler order");
}

#[test]
fn human_plan_cannot_add_authority_or_canonical_fields() {
    let wf = base_wf();
    let env = project_human_plan(&wf, None).expect("projects");
    for forbidden in [
        "root.shell",
        "fs.write",
        "network.exfiltrate",
        "provider.override",
    ] {
        assert!(
            !env.payload.contains(forbidden),
            "human plan leaked forbidden capability {forbidden}"
        );
    }
    // The Tools line lists exactly the ceiling's declared tool keys.
    let tools_line = env
        .payload
        .lines()
        .find(|l| l.starts_with("Tools: "))
        .expect("Tools line");
    let tools: Value =
        serde_json::from_str(tools_line.trim_start_matches("Tools: ")).expect("json");
    let keys: Vec<&str> = tools
        .as_object()
        .expect("tools object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        wf.authority.tool_keys(),
        "tools must be the source set"
    );
    // No envelope field beyond the five defined ones.
    let value = serde_json::to_value(&env).expect("envelope serializes");
    assert_eq!(value.as_object().expect("object").len(), 5);
}

#[test]
fn human_golden_fixtures_match() {
    let cases: [(&str, String, &[&str]); 2] = [
        (
            "wf-base",
            base_text(),
            &[
                "NON-NORMATIVE VIEW — derived from source digest",
                "not an executable contract",
                "# Workflow: wf-base v1.1.0 (Base)",
                "Schema: 1.1.0  Kind: atomic",
                "## Authority Ceiling",
                "ExecutionClass: deterministic",
                "Mutation: none",
                "Escalation: none",
                "Review: none",
                "Tools: {\"ship\":[\"ship.v1\"]}",
                "### s0000:op1 [ATOMIC]",
                "Inputs: order: Order[Produced]",
                "Outputs: shipment: Shipment[Produced]",
                "Authority: [\"ship\"]",
                "## Disclosure",
                "Redactions: 0 Omissions: 0",
            ],
        ),
        (
            "lite-reorder",
            reorder_text(),
            &[
                "### s0000:op1 [",
                "### s0001:op2 [",
                "## Constraints",
                "## Disclosure",
            ],
        ),
    ];
    for (name, text, needles) in cases {
        let wf: WorkflowDefinition = serde_json::from_str(&text).expect("parses");
        let env = project_human_plan(&wf, None).expect("projects");
        let path = golden_path(name);
        if std::env::var("PROJECTION_GOLDEN_REGEN").is_ok() {
            std::fs::create_dir_all(path.parent().expect("parent")).expect("golden dir");
            std::fs::write(&path, &env.payload).expect("write golden");
        }
        let golden = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!("missing golden {name} ({e}); regenerate with PROJECTION_GOLDEN_REGEN=1")
        });
        assert_eq!(env.payload, golden, "golden drift for {name}");
        for needle in needles {
            assert!(
                golden.contains(needle),
                "{name} golden must cover {needle:?} (activation requires authority, gates, typed outcomes, composite markers)"
            );
        }
        // Every body header carries an explicit composite/atomic marker.
        for line in golden.lines().filter(|l| l.starts_with("### s")) {
            assert!(
                line.ends_with(" [ATOMIC]") || line.ends_with(" [COMPOSITE]"),
                "missing boundary marker in {line:?}"
            );
        }
    }
}

use prometheos_lite::workflow::projection::{
    RedactionPolicy, verify_human_against_source, verify_human_projection_bytes,
};
use prometheos_lite::workflow::redaction::{REDACTED_PLACEHOLDER, SECRET_CANARY};
use prometheos_lite::workflow::soma::canonical::sha256_hex;

fn canary_policy(omitted: &[&str]) -> RedactionPolicy {
    RedactionPolicy {
        known_secrets: vec![SECRET_CANARY.to_string()],
        omitted_fields: omitted.iter().map(|s| s.to_string()).collect(),
    }
}

#[test]
fn human_redaction_masks_canary_and_canonical_stays_unredacted() {
    let wf: WorkflowDefinition = serde_json::from_str(&redacted_seed_text()).expect("seed parses");
    let human = project_human_plan(&wf, Some(canary_policy(&[]))).expect("projects");
    assert!(
        !human.payload.contains(SECRET_CANARY),
        "known secret must be redacted from the human plan"
    );
    assert!(
        human.payload.contains(REDACTED_PLACEHOLDER),
        "placeholder must be present"
    );
    let expected_id = format!("secret-{}", &sha256_hex(SECRET_CANARY.as_bytes())[..12]);
    assert!(
        human
            .payload
            .contains(&format!("  redacted: {expected_id}\n")),
        "disclosure must record the redacted secret id, got:\n{}",
        human.payload
    );
    assert!(human.payload.contains("Redactions: 1 Omissions: 0"));

    // Binding rule: canonical JSON is NEVER redacted.
    let canon = project_canonical_json(&wf).expect("projects");
    let canon_text = serde_json::to_string(&canon.payload).expect("stringifies");
    assert!(
        canon_text.contains(SECRET_CANARY),
        "canonical projection must stay unredacted"
    );
}

#[test]
fn human_omission_suppresses_field_and_records_disclosure() {
    let wf: WorkflowDefinition = serde_json::from_str(&redacted_seed_text()).expect("seed parses");
    let human = project_human_plan(
        &wf,
        Some(RedactionPolicy {
            known_secrets: vec![],
            omitted_fields: vec!["Purpose".to_string()],
        }),
    )
    .expect("projects");
    assert!(
        human.payload.contains("Purpose: <omitted>\n"),
        "omitted field value must be suppressed"
    );
    assert!(!human.payload.contains(SECRET_CANARY));
    assert!(human.payload.contains("Redactions: 0 Omissions: 1"));
    assert!(human.payload.contains("  omitted: Purpose\n"));
}

#[test]
fn tampered_disclosure_metadata_fails_closed() {
    let wf: WorkflowDefinition = serde_json::from_str(&redacted_seed_text()).expect("seed parses");
    let env = project_human_plan(&wf, Some(canary_policy(&[]))).expect("projects");
    let raw = env.canonical_bytes().expect("bytes");
    let mut v: Value = serde_json::from_slice(&raw).expect("parses");
    v["payload"] = Value::String(
        v["payload"]
            .as_str()
            .expect("payload is text")
            .replace("Redactions: 1", "Redactions: 0")
            .to_string(),
    );
    let tampered = try_canonical_bytes(&v).expect("re-canonicalized");
    let err = verify_human_projection_bytes(&tampered)
        .expect_err("disclosure tamper must fail the payload digest");
    assert_eq!(err[0].code, "SOMA-CMP-0004", "got {err:?}");
}

#[test]
fn co_tampered_human_projection_fails_against_source() {
    let wf: WorkflowDefinition = serde_json::from_str(&redacted_seed_text()).expect("seed parses");
    let env = project_human_plan(&wf, Some(canary_policy(&[]))).expect("projects");
    let raw = env.canonical_bytes().expect("bytes");
    let mut v: Value = serde_json::from_slice(&raw).expect("parses");
    let edited = v["payload"]
        .as_str()
        .expect("payload is text")
        .replace("Redactions: 1", "Redactions: 0");
    v["payload"] = Value::String(edited.clone());
    v["projectionDigest"] = Value::String(sha256_hex(edited.as_bytes()));
    let tampered = try_canonical_bytes(&v).expect("re-canonicalized");
    // Structural verify passes (payload + digest are consistent) …
    let env = verify_human_projection_bytes(&tampered).expect("structurally consistent");
    // … but the source-equality layer refuses it.
    let err = verify_human_against_source(&env, &wf, Some(&canary_policy(&[])))
        .expect_err("co-tamper must fail against the source AST");
    assert_eq!(err[0].code, "PROJ-0002", "got {err:?}");
}

#[test]
fn verify_human_accepts_honest_bytes_and_source() {
    let wf: WorkflowDefinition = serde_json::from_str(&redacted_seed_text()).expect("seed parses");
    let policy = canary_policy(&[]);
    let env = project_human_plan(&wf, Some(policy.clone())).expect("projects");
    let raw = env.canonical_bytes().expect("bytes");
    let verified = verify_human_projection_bytes(&raw).expect("honest bytes verify");
    assert_eq!(verified, env);
    assert!(verify_human_against_source(&verified, &wf, Some(&policy)).is_ok());
}

#[test]
fn human_golden_redacted_fixture_matches() {
    let wf: WorkflowDefinition = serde_json::from_str(&redacted_seed_text()).expect("seed parses");
    let env = project_human_plan(&wf, Some(canary_policy(&["Purpose"]))).expect("projects");
    let path = golden_path("wf-base-redacted");
    if std::env::var("PROJECTION_GOLDEN_REGEN").is_ok() {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("golden dir");
        std::fs::write(&path, &env.payload).expect("write golden");
    }
    let golden = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("missing golden wf-base-redacted ({e}); regenerate with PROJECTION_GOLDEN_REGEN=1")
    });
    assert_eq!(env.payload, golden, "golden drift for wf-base-redacted");
    let secret_id = format!(
        "redacted: secret-{}",
        &sha256_hex(SECRET_CANARY.as_bytes())[..12]
    );
    for needle in [
        "Purpose: <omitted>",
        "Redactions: 1 Omissions: 1",
        "omitted: Purpose",
        secret_id.as_str(),
    ] {
        assert!(
            golden.contains(needle),
            "redacted golden must cover {needle:?}"
        );
    }
}

use prometheos_lite::workflow::soma::contracts::RetryPolicy;
use prometheos_lite::workflow::soma::types::Hex64;

/// C1-a: an honest AST carrying a correct `contentDigest` must survive the
/// whole canonical round-trip — project, bytes verify, verify against source.
#[test]
fn content_digest_bearing_ast_round_trips() {
    let mut wf = base_wf();
    let plan = compile_workflow_text(&base_text()).expect("base compiles");
    wf.content_digest = Some(Hex64::parse(&plan.workflow_digest).expect("valid hex64"));
    let env = project_canonical_json(&wf).expect("honest contentDigest AST projects");
    let bytes = env.canonical_bytes().expect("canonical bytes");
    let verified = verify_canonical_projection_bytes(&bytes).expect("honest bytes verify");
    verify_projection_against_source(&verified, &wf)
        .expect("honest contentDigest AST must verify against source");
}

/// C1-b: an attacker who strips `payload.contentDigest` and recomputes
/// `projectionDigest` survives the structural check but must fail closed
/// against the source AST.
#[test]
fn stripped_payload_tamper_fails_against_source() {
    let mut wf = base_wf();
    let plan = compile_workflow_text(&base_text()).expect("base compiles");
    wf.content_digest = Some(Hex64::parse(&plan.workflow_digest).expect("valid hex64"));
    let env = project_canonical_json(&wf).expect("honest contentDigest AST projects");
    let mut value = serde_json::to_value(&env).expect("envelope serializes");
    value["payload"]
        .as_object_mut()
        .expect("payload is an object")
        .remove("contentDigest");
    let digest = try_canonical_digest(&value["payload"]).expect("tampered payload digests");
    value["projectionDigest"] = Value::String(digest);
    let tampered = try_canonical_bytes(&value).expect("re-canonicalized");
    let env = verify_canonical_projection_bytes(&tampered).expect("structural verify passes");
    let err = verify_projection_against_source(&env, &wf)
        .expect_err("stripped-payload tamper must fail against the source AST");
    assert_eq!(err[0].code, "PROJ-0002", "got {err:?}");
}

/// I2 (spec §9): a raw, non-canonical number lexeme injected into honest
/// bytes must fail closed on read — the canonical re-render diverges.
#[test]
fn raw_number_lexeme_fails_closed_on_read() {
    // No shipped workflow fixture carries a JSON numeric literal, so the
    // test puts one in the AST: `retry.maxAttempts: 2` (the audit has no
    // retry rule, so the gate stays honest).
    let mut wf = base_wf();
    let prometheos_lite::workflow::soma::contracts::BodyItem::Operation(op) = &mut wf.body[0]
    else {
        panic!("base fixture body[0] is an operation");
    };
    op.retry = Some(RetryPolicy {
        max_attempts: 2,
        backoff: None,
    });
    let bytes = project_canonical_json(&wf)
        .expect("projects")
        .canonical_bytes()
        .expect("canonical bytes");
    let text = String::from_utf8(bytes).expect("utf-8");
    let needle = "\"maxAttempts\":2";
    assert_eq!(
        text.matches(needle).count(),
        1,
        "expected exactly one {needle} in honest bytes"
    );
    // Inject directly into honest bytes — NO re-key: re-canonicalizing would
    // normalize `2.0` back to `2` and heal the very lexeme under test.
    let injected = text.replace(needle, "\"maxAttempts\":2.0");
    let err = verify_canonical_projection_bytes(injected.as_bytes())
        .expect_err("non-canonical number lexeme must fail closed");
    assert_eq!(err[0].code, "PROJ-0001", "got {err:?}");
}

// ---------------------------------------------------------------------------
// Independent exact-head review repairs (B1/B2): top-level schemaVersion is
// bound on parse, and both public against-source verifiers independently
// validate complete envelope integrity before identity comparison.
// ---------------------------------------------------------------------------

/// B1 structural path: a forged top-level `schemaVersion` on an otherwise
/// honest canonical envelope must fail closed on read.
#[test]
fn forged_top_level_schema_version_fails_canonical_bytes() {
    let mut env = project_canonical_json(&base_wf()).expect("projects");
    env.schema_version = "9.9.9".to_string();
    let bytes = env.canonical_bytes().expect("canonical bytes");
    let err = verify_canonical_projection_bytes(&bytes)
        .expect_err("forged top-level schemaVersion must fail closed");
    assert_eq!(err[0].code, "SOMA-CMP-0001", "got {err:?}");
}

/// B1 structural path, human flavor: same forgery through the human byte
/// verifier must fail closed with the same version-refusal code.
#[test]
fn forged_top_level_schema_version_fails_human_bytes() {
    let mut env = project_human_plan(&base_wf(), None).expect("projects");
    env.schema_version = "9.9.9".to_string();
    let bytes = env.canonical_bytes().expect("canonical bytes");
    let err = verify_human_projection_bytes(&bytes)
        .expect_err("forged top-level schemaVersion must fail closed");
    assert_eq!(err[0].code, "SOMA-CMP-0001", "got {err:?}");
}

/// B2: a structurally plausible but wrong `projectionDigest` (valid hex64)
/// must be caught by `verify_projection_against_source` itself — no prior
/// byte-verification call required.
#[test]
fn forged_projection_digest_fails_projection_against_source() {
    let wf = base_wf();
    let mut env = project_canonical_json(&wf).expect("projects");
    env.projection_digest = "f".repeat(64);
    let err = verify_projection_against_source(&env, &wf)
        .expect_err("forged projectionDigest must fail against the source");
    assert_eq!(err[0].code, "SOMA-CMP-0004", "got {err:?}");
}

/// B2: an unsupported `projectionVersion` must be refused by the
/// against-source verifier itself.
#[test]
fn forged_projection_version_fails_projection_against_source() {
    let wf = base_wf();
    let mut env = project_canonical_json(&wf).expect("projects");
    env.projection_version = "projection.v99".to_string();
    let err = verify_projection_against_source(&env, &wf)
        .expect_err("forged projectionVersion must fail against the source");
    assert_eq!(err[0].code, "SOMA-CMP-0001", "got {err:?}");
}

/// B2: a forged top-level `schemaVersion` must be refused by the
/// against-source verifier itself (strict SemVer bound to supported).
#[test]
fn forged_schema_version_fails_projection_against_source() {
    let wf = base_wf();
    let mut env = project_canonical_json(&wf).expect("projects");
    env.schema_version = "9.9.9".to_string();
    let err = verify_projection_against_source(&env, &wf)
        .expect_err("forged schemaVersion must fail against the source");
    assert_eq!(err[0].code, "SOMA-CMP-0001", "got {err:?}");
}

/// B2, human flavor: forged `projectionDigest` caught by
/// `verify_human_against_source` without a prior byte-verification call.
#[test]
fn forged_projection_digest_fails_human_against_source() {
    let wf = base_wf();
    let mut env = project_human_plan(&wf, None).expect("projects");
    env.projection_digest = "f".repeat(64);
    let err = verify_human_against_source(&env, &wf, None)
        .expect_err("forged projectionDigest must fail against the source");
    assert_eq!(err[0].code, "SOMA-CMP-0004", "got {err:?}");
}

/// B2, human flavor: unsupported `projectionVersion` refused directly.
#[test]
fn forged_projection_version_fails_human_against_source() {
    let wf = base_wf();
    let mut env = project_human_plan(&wf, None).expect("projects");
    env.projection_version = "projection.v99".to_string();
    let err = verify_human_against_source(&env, &wf, None)
        .expect_err("forged projectionVersion must fail against the source");
    assert_eq!(err[0].code, "SOMA-CMP-0001", "got {err:?}");
}

/// B2, human flavor: forged top-level `schemaVersion` refused directly.
#[test]
fn forged_schema_version_fails_human_against_source() {
    let wf = base_wf();
    let mut env = project_human_plan(&wf, None).expect("projects");
    env.schema_version = "9.9.9".to_string();
    let err = verify_human_against_source(&env, &wf, None)
        .expect_err("forged schemaVersion must fail against the source");
    assert_eq!(err[0].code, "SOMA-CMP-0001", "got {err:?}");
}

// ---------------------------------------------------------------------------
// E4/X07 Slice 2 — Task 1: flat-fixture pins captured at main@84d44e4
// ---------------------------------------------------------------------------

const FLAT_PINS: [(&str, &str, &str); 3] = [
    (
        "fixtures/valid/wf-valid-base.json",
        "0d4e039e5d750307f52790068e687d879d5e4d124beffdeb06becd5abe39fd6d",
        "709bcd249f8977d7316c73d52834f209d1b4f78eb7f9a644f451ba7c07f5ae54",
    ),
    (
        "fixtures/valid/wf-valid-composite.json",
        "10190c09684303f6286ee86d8f4e7bea62b4bafeecbbf3c19790d46d6aeb87cf",
        "85c23e8c7bfa9044dd8f37dbeed4f0f25ad4ad9ee889a8668030d6ba1cd92974",
    ),
    (
        "fixtures/valid/wf-valid-gov.json",
        "bf337338252c7022282647bc20b9bb331f5b7997c5633ff8f012f329a8e3f1ee",
        "709b1ab15dcd1ad178c1ad05df69e33e3ee589be0f8ba5142d498025f1be44ac",
    ),
];

#[test]
fn flat_workflow_digests_remain_pinned() {
    for (rel, digest, _) in FLAT_PINS {
        let text = fixture(rel);
        let plan = compile_workflow_text(&text).unwrap_or_else(|e| panic!("{rel}: {e:?}"));
        assert_eq!(
            plan.workflow_digest, digest,
            "workflow digest drifted for {rel}"
        );
    }
}

#[test]
fn flat_canonical_bytes_unchanged() {
    for (rel, _, bytes_sha) in FLAT_PINS {
        let wf: WorkflowDefinition =
            serde_json::from_str(&fixture(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
        let env =
            project_canonical_json(&wf).unwrap_or_else(|e| panic!("{rel} must project: {e:?}"));
        let bytes = env
            .canonical_bytes()
            .unwrap_or_else(|e| panic!("{rel}: {e:?}"));
        assert_eq!(
            sha256_hex(&bytes),
            bytes_sha,
            "canonical envelope bytes drifted for {rel}"
        );
        let back = verify_canonical_projection_bytes(&bytes)
            .unwrap_or_else(|e| panic!("{rel} bytes must round-trip: {e:?}"));
        let again = back.canonical_bytes().expect("re-render");
        assert_eq!(
            sha256_hex(&again),
            bytes_sha,
            "round-trip bytes differ for {rel}"
        );
    }
}

// ---------------------------------------------------------------------------
// E4/X07 Slice 2 — Tasks 1-2: nested composite contract + fail-closed surfaces
// ---------------------------------------------------------------------------

const NESTED_FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/slice2/wf-nested.json"
);

fn nested_text() -> String {
    std::fs::read_to_string(NESTED_FIXTURE).expect("wf-nested fixture reads")
}

fn nested_wf() -> WorkflowDefinition {
    serde_json::from_str(&nested_text()).expect("wf-nested parses")
}

/// Spec §12 item 3 — RED at main@84d44e4 (nested body items fail to parse),
/// GREEN after Task 1. Uses only types that exist at base so the RED is a
/// runtime failure, not a compile failure; depth is proven through `Value`.
#[test]
fn nested_composite_document_parses() {
    let wf: WorkflowDefinition =
        serde_json::from_str(&nested_text()).expect("nested composite document must parse");
    assert_eq!(wf.id, "wf-nested");
    assert_eq!(wf.body.len(), 3);
    let v: Value = serde_json::from_str(&nested_text()).expect("fixture is valid JSON");
    assert_eq!(v["body"][1]["id"], "flow");
    assert_eq!(v["body"][1]["body"][0]["id"], "inner");
    assert_eq!(v["body"][1]["body"][0]["body"][0]["id"], "leaf");
}

/// Spec §12 item 4 — parse negatives. Each rejection must carry the BodyItem
/// oneOf refusal phrasing (base emits plain serde "unknown field"/"missing
/// field" errors, so this assertion is RED at main@84d44e4).
#[test]
fn body_item_one_of_negatives() {
    let base: Value = serde_json::from_str(&nested_text()).expect("fixture is valid JSON");

    let mut collision = base["body"][0].clone();
    collision["inputPorts"] = serde_json::json!([]);
    collision["outputPorts"] = serde_json::json!([]);
    collision["body"] = serde_json::json!([]);

    let mut no_exec = base["body"][0].clone();
    no_exec
        .as_object_mut()
        .expect("operation object")
        .remove("executionClass");

    let mut no_ports = base["body"][1].clone();
    {
        let obj = no_ports.as_object_mut().expect("composite object");
        obj.remove("inputPorts");
        obj.remove("body");
    }

    let mut malformed = base["body"][1].clone();
    malformed["body"] = serde_json::json!([{ "notA": "bodyItem" }]);

    for (name, item) in [
        ("discriminator-collision", collision),
        ("missing-executionClass", no_exec),
        ("missing-inputPorts-body", no_ports),
        ("malformed-nesting", malformed),
    ] {
        let mut doc = base.clone();
        doc["body"] = serde_json::json!([item]);
        let err = serde_json::from_str::<WorkflowDefinition>(&doc.to_string())
            .expect_err(name)
            .to_string();
        assert!(
            err.contains("body-item"),
            "{name}: expected a BodyItem oneOf refusal, got: {err}"
        );
    }
}

/// Spec §12 item 5 (final form, Task 2): the shared fixture audits clean,
/// each nested fault fails the gate with its exact code, and no projection
/// succeeds for any fault. RED against the Task 1 state (audit returns the
/// blanket SOMA-CMP-0003 refusal).
#[test]
fn recursive_audit_rejects_nested_faults() {
    let supported = prometheos_lite::workflow::soma::supported_version();

    let clean = nested_wf();
    assert!(
        clean.audit(&supported).is_empty(),
        "wf-nested must audit clean once recursion lands: {:?}",
        clean.audit(&supported)
    );

    #[allow(clippy::type_complexity)]
    let faults: [(&str, fn(&mut Value)); 3] = [
        ("duplicate-across-scopes", |v| {
            v["body"][1]["body"][0]["body"][0]["id"] = serde_json::json!("inner");
        }),
        ("unfed-nested-input", |v| {
            v["body"][1]["body"][0]["body"][0]["inputs"][0]["name"] = serde_json::json!("stray");
        }),
        ("nested-authority-import", |v| {
            v["body"][1]["body"][0]["body"][0]["authority"] = serde_json::json!(["ship"]);
        }),
    ];
    let expected: [&str; 3] = ["SOMA-EXP-0003", "SOMA-EXP-0005", "SOMA-AUTH-0002"];

    for ((name, mutate), code) in faults.into_iter().zip(expected) {
        let mut v: Value = serde_json::from_str(&nested_text()).expect("fixture is valid JSON");
        mutate(&mut v);
        let wf: WorkflowDefinition =
            serde_json::from_value(v).expect("mutated fixture still parses");
        let diags = wf.audit(&supported);
        assert!(
            diags.iter().any(|d| d.code == code),
            "{name}: expected {code}, got {diags:?}"
        );
        assert!(
            project_canonical_json(&wf).is_err(),
            "{name}: projection must be refused by the gate"
        );
    }
}

/// Spec §2.4 consumer fail-closed matrix (final form, Task 2).
#[test]
fn nested_surfaces_fail_closed_matrix() {
    let text = nested_text();

    // Governance plan compiler: exact SOMA-CMP-0003 via the explicit scan.
    let err =
        compile_workflow_text(&text).expect_err("plan compilation must refuse nested composites");
    assert!(
        err.iter().any(|d| d.code == "SOMA-CMP-0003"),
        "expected SOMA-CMP-0003 from compile_workflow_text, got {err:?}"
    );

    // Authority compiler: audit-sourced (scan after parse) — stable since Task 1.
    let v: Value = serde_json::from_str(&text).expect("nested parses");
    let err = prometheos_lite::workflow::governance::compile_authority(&v)
        .expect_err("authority compilation must refuse nested composites");
    assert!(
        err.iter().any(|d| d.code == "SOMA-CMP-0003"),
        "expected SOMA-CMP-0003 from compile_authority, got {err:?}"
    );

    // Permit chains compile_workflow_text first — inherits SOMA-CMP-0003.
    let err = prometheos_lite::workflow::governance_permit::GovernancePermit::issue(
        &text,
        "reviewed-identity",
    )
    .expect_err("permit issuance must refuse nested composites");
    assert!(
        err.iter().any(|d| d.code == "SOMA-CMP-0003"),
        "expected SOMA-CMP-0003 from GovernancePermit::issue, got {err:?}"
    );

    // Execution surface: explicit PROJ-0001 scan (lands Task 1, unchanged).
    let err =
        prometheos_lite::workflow::execution_graph::compile_execution_graph(&text, &"0".repeat(64))
            .expect_err("execution graph must refuse nested composites");
    assert!(
        err.iter().any(|d| d.code == "PROJ-0001"),
        "expected PROJ-0001 from compile_execution_graph, got {err:?}"
    );

    // Canonical projection: nested ASTs now parse AND audit clean, so the
    // canonical view succeeds (Slice-1 behavior for anything that parses).
    let wf = nested_wf();
    let env = project_canonical_json(&wf)
        .expect("canonical projection accepts nested documents after Task 2");
    let bytes = env.canonical_bytes().expect("canonical bytes");
    let text = String::from_utf8(bytes).expect("canonical bytes are UTF-8");
    assert!(text.contains("inner") && text.contains("leaf"));

    // Human projection: PROJ-0001 with a composite-specific message (never
    // the cycle wording — that is a distinct failure mode).
    let err =
        project_human_plan(&wf, None).expect_err("human projection must refuse nested composites");
    let proj: Vec<&str> = err
        .iter()
        .filter(|d| d.code == "PROJ-0001")
        .map(|d| d.message.as_str())
        .collect();
    assert!(
        !proj.is_empty(),
        "expected PROJ-0001 from human projection, got {err:?}"
    );
    assert!(
        proj.iter().any(|m| m.contains("composite")),
        "human refusal must say 'composite', got {proj:?}"
    );
}
// ---------------------------------------------------------------------------
// E4/X07 Slice 2 — Task 4: graph projection with disclosure boundaries
// (spec §12 tests 6-21, 23, 24; tests 22, 26-28 arrive with the verifiers
// in Task 5, test 29 with Task 6)
// ---------------------------------------------------------------------------

use std::collections::{BTreeMap, BTreeSet};

use prometheos_lite::workflow::projection::graph::GRAPH_SCHEMA_VERSION;
use prometheos_lite::workflow::projection::{GraphDisclosurePolicy, project_graph_json};

/// Caller-side policy builder (§6 shape: rendered node ids only).
fn graph_policy(authorized: &[&str], counts: &[&str]) -> GraphDisclosurePolicy {
    GraphDisclosurePolicy {
        authorized_boundaries: authorized.iter().map(|s| s.to_string()).collect(),
        count_authorization: counts.iter().map(|s| s.to_string()).collect(),
    }
}

/// wf-nested projected under an optional disclosure policy.
fn nested_graph(policy: Option<GraphDisclosurePolicy>) -> VersionedProjectionEnvelope<Value> {
    project_graph_json(&nested_wf(), policy.as_ref()).expect("wf-nested projects")
}

/// Repair `contentDigest` after a Value-level mutation: the audit's
/// `SOMA-CMP-0004` gate hashes the *serialization* of the AST minus this
/// field (audit_workflow.rs), and that serialization drops what the raw
/// document still carries (the fixtures' empty `effects`/`uses`/`secrets`/
/// `context` arrays), so hashing the raw `Value` disagrees with the pinned
/// digests (raw `512c74f7…` vs pinned `0d4e039e…` for wf-valid-base).
/// Parse first, digest the re-serialized value, then store — parity with
/// the gate is then exact.
fn parse_with_content_digest(value: &mut Value) -> WorkflowDefinition {
    let parsed: WorkflowDefinition =
        serde_json::from_value(value.clone()).expect("mutated fixture parses");
    let mut hashed = serde_json::to_value(&parsed).expect("workflow serializes to an object");
    if let Some(obj) = hashed.as_object_mut() {
        obj.remove("contentDigest");
    }
    let digest = try_canonical_digest(&hashed).expect("canonical digest of mutated fixture");
    value["contentDigest"] = Value::String(digest);
    serde_json::from_value(value.clone()).expect("mutated fixture parses")
}

/// Spec §12 test 6 — default withholding: composites render as boundary
/// nodes only; no child ids, no `children`/`internalEdges`, no counts.
#[test]
fn default_withholding() {
    let env = nested_graph(None);
    let bytes = env.canonical_bytes().expect("canonical bytes");
    let text = String::from_utf8(bytes).expect("bytes are UTF-8");
    for leaked in [
        "s0000:inner",
        "s0000:leaf",
        "\"inner\"",
        "\"leaf\"",
        "\"children\"",
        "\"internalEdges\"",
        "hiddenNodes",
        "hiddenEdges",
    ] {
        assert!(!text.contains(leaked), "default view leaked {leaked:?}");
    }
    let nodes = env.payload["nodes"].as_array().expect("nodes array");
    assert_eq!(nodes.len(), 3);
    assert_eq!(env.payload["graphSchemaVersion"], GRAPH_SCHEMA_VERSION);
    assert_eq!(nodes[0]["kind"], "atomic");
    assert_eq!(nodes[2]["kind"], "atomic");
    let flow = &nodes[1];
    assert_eq!(flow["kind"], "composite");
    assert_eq!(flow["id"], "s0001:flow");
    assert!(flow.get("children").is_none());
    assert!(flow.get("internalEdges").is_none());
    assert_eq!(flow["disclosure"]["withheld"], serde_json::json!(true));
    assert_eq!(flow["disclosure"]["reason"], "private-composite-default");
    assert_eq!(flow["disclosure"]["category"], "composite-boundary");
    assert!(flow["disclosure"].get("hiddenNodes").is_none());
    assert!(flow["disclosure"].get("hiddenEdges").is_none());
    let digest = flow["childSubgraphDigest"].as_str().expect("digest");
    assert_eq!(digest.len(), 64);
}

/// Spec §12 test 7 — authorized selective reveal: direct contents render;
/// each nested boundary keeps its own disclosure state.
#[test]
fn authorized_selective_reveal() {
    let env = nested_graph(Some(graph_policy(&["s0001:flow"], &[])));
    let nodes = env.payload["nodes"].as_array().expect("nodes");
    let flow = &nodes[1];
    assert!(
        flow.get("disclosure").is_none(),
        "an authorized boundary carries no disclosure block"
    );
    let children = flow["children"].as_array().expect("flow children render");
    assert_eq!(children.len(), 1);
    let inner = &children[0];
    assert_eq!(inner["id"], "s0000:inner");
    assert_eq!(inner["kind"], "composite");
    assert_eq!(inner["disclosure"]["withheld"], serde_json::json!(true));
    assert!(
        inner.get("children").is_none(),
        "non-cascading: inner keeps its own state"
    );
    assert!(inner.get("internalEdges").is_none());
    assert!(inner["disclosure"].get("hiddenNodes").is_none());
    let edges = flow["internalEdges"]
        .as_array()
        .expect("flow internalEdges");
    assert_eq!(edges.len(), 2, "R1 supply edge + R2 pass-through edge");
    assert_eq!(nodes.len(), 3, "parent structure untouched");
}

/// Spec §12 test 8 — non-cascading authorization: a parent entry never
/// reveals descendants; every nested boundary needs its own entry.
#[test]
fn non_cascading_authorization() {
    let parent_only = nested_graph(Some(graph_policy(&["s0001:flow"], &[])));
    let inner = &parent_only.payload["nodes"][1]["children"][0];
    assert_eq!(inner["disclosure"]["withheld"], serde_json::json!(true));
    assert!(inner.get("children").is_none());
    assert!(inner.get("internalEdges").is_none());

    let both = nested_graph(Some(graph_policy(&["s0000:inner", "s0001:flow"], &[])));
    let inner = &both.payload["nodes"][1]["children"][0];
    assert!(inner.get("disclosure").is_none());
    let leaf = inner["children"].as_array().expect("inner children render");
    assert_eq!(leaf.len(), 1);
    assert_eq!(leaf[0]["id"], "s0000:leaf");
    assert_eq!(leaf[0]["kind"], "atomic");
}

/// Spec §12 test 9 — ancestry traversability (§6 rule 4): a descendant
/// boundary without its ancestors is refused; full ancestry succeeds.
#[test]
fn ancestry_traversal_enforced() {
    let wf = nested_wf();
    let err = project_graph_json(&wf, Some(&graph_policy(&["s0000:inner"], &[])))
        .expect_err("descendant without ancestry must be refused");
    assert!(
        err.iter()
            .any(|d| d.code == "PROJ-0003" && d.message.contains("without its ancestor")),
        "expected an ancestry refusal, got {err:?}"
    );
    assert!(
        project_graph_json(
            &wf,
            Some(&graph_policy(&["s0001:flow", "s0000:inner"], &[]))
        )
        .is_ok()
    );
}

/// Spec §12 test 10 — policy validation negatives (§6 rules 1–3).
#[test]
fn policy_validation_negatives() {
    let wf = nested_wf();
    let refuse =
        |p: GraphDisclosurePolicy| project_graph_json(&wf, Some(&p)).expect_err("must refuse");

    let unknown = refuse(graph_policy(&["s9999:ghost"], &[]));
    assert!(
        unknown
            .iter()
            .any(|d| d.code == "PROJ-0003" && d.message.contains("unknown node id")),
        "{unknown:?}"
    );

    let atomic = refuse(graph_policy(&["s0000:prep"], &[]));
    assert!(
        atomic
            .iter()
            .any(|d| d.code == "PROJ-0003" && d.message.contains("non-composite")),
        "{atomic:?}"
    );

    let dup = refuse(graph_policy(&["s0001:flow", "s0001:flow"], &[]));
    assert!(
        dup.iter()
            .any(|d| d.code == "PROJ-0003"
                && d.message.contains("duplicate id in authorizedBoundaries")),
        "{dup:?}"
    );
}

/// Spec §12 test 11 — counts are independent of content authorization:
/// integers only, never content; content authorization never carries counts.
#[test]
fn count_authorization_independent() {
    let counted = nested_graph(Some(graph_policy(&[], &["s0001:flow"])));
    let flow = &counted.payload["nodes"][1];
    assert_eq!(flow["disclosure"]["withheld"], serde_json::json!(true));
    assert_eq!(flow["disclosure"]["hiddenNodes"], serde_json::json!(2));
    assert_eq!(flow["disclosure"]["hiddenEdges"], serde_json::json!(4));
    assert!(flow.get("children").is_none());
    assert!(flow.get("internalEdges").is_none());
    let text =
        String::from_utf8(counted.canonical_bytes().expect("bytes")).expect("bytes are UTF-8");
    assert!(
        !text.contains("s0000:inner"),
        "counts must never reveal content"
    );
    assert!(!text.contains("s0000:leaf"));
    assert!(!text.contains("\"children\""));

    // Same id in both arrays: content authorization wins its own node —
    // a revealed boundary renders no disclosure block at all.
    let both = nested_graph(Some(graph_policy(&["s0001:flow"], &["s0001:flow"])));
    let flow = &both.payload["nodes"][1];
    assert!(flow.get("children").is_some());
    assert!(flow.get("disclosure").is_none());
}

/// Spec §12 test 12 — `None` and `Some(empty)` are byte-identical (§6).
#[test]
fn none_equals_empty_policy() {
    let none = project_graph_json(&nested_wf(), None).expect("None projects");
    let empty = project_graph_json(&nested_wf(), Some(&GraphDisclosurePolicy::default()))
        .expect("empty projects");
    assert_eq!(
        none.canonical_bytes().expect("none bytes"),
        empty.canonical_bytes().expect("empty bytes"),
        "None and Some(empty) must be byte-identical"
    );
    assert_eq!(none.projection_digest, empty.projection_digest);
    assert_eq!(none.source_digest, empty.source_digest);
    assert_eq!(none.payload["policyDigest"], empty.payload["policyDigest"]);
}

/// Spec §12 test 13 — policy input order never changes output bytes.
#[test]
fn policy_order_irrelevant() {
    let a = nested_graph(Some(graph_policy(&["s0001:flow", "s0000:inner"], &[])));
    let b = nested_graph(Some(graph_policy(&["s0000:inner", "s0001:flow"], &[])));
    assert_eq!(
        a.canonical_bytes().expect("a"),
        b.canonical_bytes().expect("b"),
        "authorizedBoundaries order must not change bytes"
    );

    let c = nested_graph(Some(graph_policy(&[], &["s0001:flow", "s0000:inner"])));
    let d = nested_graph(Some(graph_policy(&[], &["s0000:inner", "s0001:flow"])));
    assert_eq!(
        c.canonical_bytes().expect("c"),
        d.canonical_bytes().expect("d"),
        "countAuthorization order must not change bytes"
    );
    assert_eq!(
        c.payload["nodes"][1]["disclosure"]["hiddenNodes"],
        serde_json::json!(2)
    );
}

/// Spec §12 test 14 — distinct disclosure views share `sourceDigest` and
/// differ in `projectionDigest` (explicit).
#[test]
fn disclosure_views_share_source_digest() {
    let default = nested_graph(None);
    let revealed = nested_graph(Some(graph_policy(&["s0001:flow", "s0000:inner"], &[])));
    assert_eq!(default.source_digest, revealed.source_digest);
    assert_ne!(default.projection_digest, revealed.projection_digest);
}

/// Every path at which two JSON values differ (missing keys included).
fn diff_paths(a: &Value, b: &Value, path: &str, out: &mut Vec<String>) {
    if a == b {
        return;
    }
    match (a, b) {
        (Value::Object(ma), Value::Object(mb)) => {
            let mut keys: Vec<&String> = ma.keys().chain(mb.keys()).collect();
            keys.sort();
            keys.dedup();
            for k in keys {
                match (ma.get(k), mb.get(k)) {
                    (Some(va), Some(vb)) => diff_paths(va, vb, &format!("{path}/{k}"), out),
                    _ => out.push(format!("{path}/{k}")),
                }
            }
        }
        (Value::Array(aa), Value::Array(ab)) => {
            if aa.len() != ab.len() {
                out.push(format!("{path}: array length {} vs {}", aa.len(), ab.len()));
                return;
            }
            for (i, (va, vb)) in aa.iter().zip(ab.iter()).enumerate() {
                diff_paths(va, vb, &format!("{path}/{i}"), out);
            }
        }
        _ => out.push(path.to_string()),
    }
}

/// All `(node id, childSubgraphDigest)` pairs in a payload, document order.
fn child_digests_in(payload: &Value) -> Vec<(String, String)> {
    fn walk(node: &Value, out: &mut Vec<(String, String)>) {
        if let Some(digest) = node.get("childSubgraphDigest").and_then(|v| v.as_str()) {
            out.push((
                node["id"].as_str().expect("node id").to_string(),
                digest.to_string(),
            ));
        }
        if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
            for child in children {
                walk(child, out);
            }
        }
    }
    let mut found = Vec::new();
    for node in payload["nodes"].as_array().expect("nodes") {
        walk(node, &mut found);
    }
    found
}

/// Spec §12 test 15 — structural no-leakage diff: excluding exactly
/// `{envelope.projectionDigest (payloads only), payload.policyDigest}`, the
/// default and revealed views differ only inside the authorized region;
/// `childSubgraphDigest` values are view-invariant across both views (§8.1).
#[test]
fn structural_no_leakage_diff() {
    let default = nested_graph(None);
    let revealed = nested_graph(Some(graph_policy(&["s0001:flow", "s0000:inner"], &[])));

    // §8.1 view-invariance, compared before any exclusion.
    let dd = child_digests_in(&default.payload);
    let rd = child_digests_in(&revealed.payload);
    assert_eq!(
        dd.len(),
        1,
        "default view exposes the boundary commitment only"
    );
    assert_eq!(rd.len(), 2, "revealed view exposes both commitments");
    assert_eq!(dd[0].0, "s0001:flow");
    assert_eq!(rd[0].0, "s0001:flow");
    assert_eq!(dd[0], rd[0], "childSubgraphDigest must be view-invariant");

    // Comparing payloads (not envelopes) already excludes the envelope's
    // projectionDigest; drop the payload's policyDigest explicitly.
    let mut a = default.payload.clone();
    let mut b = revealed.payload.clone();
    a.as_object_mut()
        .expect("payload object")
        .remove("policyDigest");
    b.as_object_mut()
        .expect("payload object")
        .remove("policyDigest");

    let mut diffs = Vec::new();
    diff_paths(&a, &b, "", &mut diffs);
    assert!(!diffs.is_empty(), "the views must differ somewhere");
    for path in &diffs {
        assert!(
            path.starts_with("/nodes/1"),
            "diff outside the authorized region: {path}"
        );
    }
}

/// Spec §12 test 16 — byte determinism across seed-dependent map insertion
/// orders (Slice-1 technique) and repeat projections of the nested fixture.
#[test]
fn graph_deterministic_across_seeds() {
    let mut reference: Option<(Vec<u8>, String, String)> = None;
    for seed in 0..10usize {
        let env = project_graph_json(&seed_wf(seed), None)
            .unwrap_or_else(|e| panic!("seed {seed} must project: {e:?}"));
        let bytes = env.canonical_bytes().expect("canonical bytes");
        match &reference {
            None => {
                reference = Some((
                    bytes,
                    env.source_digest.clone(),
                    env.projection_digest.clone(),
                ));
            }
            Some((rb, rs, rp)) => {
                assert_eq!(&bytes, rb, "seed {seed} produced different envelope bytes");
                assert_eq!(&env.source_digest, rs, "seed {seed} sourceDigest differs");
                assert_eq!(
                    &env.projection_digest, rp,
                    "seed {seed} projectionDigest differs"
                );
            }
        }
    }
    let x = nested_graph(None);
    let y = nested_graph(None);
    assert_eq!(
        x.canonical_bytes().expect("x"),
        y.canonical_bytes().expect("y"),
        "the nested fixture must project byte-identically too"
    );
}

/// Spec §12 test 16a — the §8.1 construction is acyclic, deterministic,
/// and transitively bound: recompute from the public preimage, mutate a
/// deep leaf, and watch every ancestor commitment move.
#[test]
fn nested_digest_construction_acyclic_and_deterministic() {
    let policy = graph_policy(&["s0000:inner", "s0001:flow"], &[]);
    let env = project_graph_json(&nested_wf(), Some(&policy)).expect("projects");
    let again = project_graph_json(&nested_wf(), Some(&policy)).expect("projects again");
    assert_eq!(env.payload, again.payload, "digests must be deterministic");
    assert_eq!(env.projection_digest, again.projection_digest);

    // The subject's own `childSubgraphDigest` field is excluded from its
    // preimage; every other rendered field (incl. descendant digests) is
    // embedded exactly as §8.1 specifies.
    let strip = |node: &Value| {
        let mut v = node.clone();
        v.as_object_mut()
            .expect("node is an object")
            .remove("childSubgraphDigest");
        v
    };
    let preimage = |boundary_id: &str, content: Value| {
        serde_json::json!({
            "domain": "projection.graph.child-subgraph.v1",
            "graphSchemaVersion": env.payload["graphSchemaVersion"],
            "rootSourceDigest": env.source_digest,
            "boundaryId": boundary_id,
            "content": content,
        })
    };

    let flow_value = &env.payload["nodes"][1];
    let inner_value = &flow_value["children"][0];
    let inner_recomputed =
        try_canonical_digest(&preimage("s0000:inner", strip(inner_value))).expect("preimage");
    assert_eq!(
        inner_value["childSubgraphDigest"], inner_recomputed,
        "embedded inner digest must equal the public recomputation"
    );
    let flow_recomputed =
        try_canonical_digest(&preimage("s0001:flow", strip(flow_value))).expect("preimage");
    assert_eq!(
        flow_value["childSubgraphDigest"], flow_recomputed,
        "embedded flow digest must equal the public recomputation"
    );

    // Deep-leaf mutation (re-hashed so the gate stays honest): every
    // ancestor commitment must move — transitive binding, no self-reference.
    let mut raw: Value = serde_json::from_str(&nested_text()).expect("fixture");
    raw["body"][1]["body"][0]["body"][0]["outputs"]
        .as_array_mut()
        .expect("leaf outputs")
        .push(serde_json::json!({
            "name": "trace",
            "type": "Order",
            "emits": ["Produced"]
        }));
    let changed = parse_with_content_digest(&mut raw);
    let mutated = project_graph_json(&changed, Some(&policy)).expect("mutated leaf projects");
    let m_nodes = &mutated.payload["nodes"];
    assert_ne!(
        m_nodes[1]["childSubgraphDigest"], flow_value["childSubgraphDigest"],
        "flow commitment must move when a deep leaf changes"
    );
    assert_ne!(
        m_nodes[1]["children"][0]["childSubgraphDigest"], inner_value["childSubgraphDigest"],
        "inner commitment must move when a deep leaf changes"
    );
}

/// Spec §12 test 17 — every vendored flat `wf-*` valid fixture projects
/// cleanly and byte-deterministically (§7.4 parity guard). Task 5 extends
/// this body with `verify_graph_projection_bytes` + against-source calls.
#[test]
fn all_vendored_valid_fixtures_project() {
    let dir = format!("{VENDORED}/fixtures/valid");
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{dir}: {e}"))
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("wf-") && n.ends_with(".json"))
        })
        .collect();
    files.sort();
    assert_eq!(files.len(), 3, "expected the three wf-* valid fixtures");

    for path in &files {
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
        let wf: WorkflowDefinition =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path:?} must parse: {e}"));
        let env = project_graph_json(&wf, None)
            .unwrap_or_else(|e| panic!("{path:?} must project: {e:?}"));
        let bytes = env.canonical_bytes().expect("canonical bytes");
        let again = project_graph_json(&wf, None).expect("second projection");
        assert_eq!(
            bytes,
            again.canonical_bytes().expect("canonical bytes"),
            "{path:?} is not byte-stable"
        );
    }
}

/// Spec §12 test 18 — authority expansion is impossible: every rendered
/// id, port, type, outcome, label, and disclosure string traces back to an
/// AST declaration or the fixed disclosure vocabulary.
#[test]
fn authority_expansion_impossible() {
    fn scan(
        item: &Value,
        ids: &mut BTreeSet<String>,
        ports: &mut BTreeSet<String>,
        words: &mut BTreeSet<String>,
    ) {
        ids.insert(item["id"].as_str().expect("declared id").to_string());
        for key in ["inputs", "outputs", "inputPorts", "outputPorts"] {
            if let Some(list) = item.get(key).and_then(|v| v.as_array()) {
                for p in list {
                    ports.insert(p["name"].as_str().expect("port name").to_string());
                    words.insert(p["type"].as_str().expect("port type").to_string());
                    for outcomes in ["acceptedOutcomes", "emits"] {
                        if let Some(list) = p.get(outcomes).and_then(|v| v.as_array()) {
                            for o in list {
                                words.insert(o.as_str().expect("outcome").to_string());
                            }
                        }
                    }
                }
            }
        }
        if let Some(body) = item.get("body").and_then(|v| v.as_array()) {
            for child in body {
                scan(child, ids, ports, words);
            }
        }
    }
    let raw: Value = serde_json::from_str(&nested_text()).expect("fixture");
    let mut raw_ids = BTreeSet::new();
    let mut port_names = BTreeSet::new();
    let mut words = BTreeSet::new();
    scan(&raw, &mut raw_ids, &mut port_names, &mut words);
    for key in ["inputPorts", "outputPorts"] {
        for p in raw[key].as_array().expect("root ports") {
            port_names.insert(p["name"].as_str().expect("port name").to_string());
            words.insert(p["type"].as_str().expect("port type").to_string());
        }
    }

    fn node_ports(node: &Value) -> BTreeSet<String> {
        let mut ports = BTreeSet::new();
        for key in ["inputs", "outputs"] {
            if let Some(list) = node.get(key).and_then(|v| v.as_array()) {
                for p in list {
                    ports.insert(p["name"].as_str().expect("port name").to_string());
                }
            }
        }
        ports
    }

    fn check_scope(
        nodes: &[Value],
        container: Option<&Value>,
        edges: &Value,
        raw_ids: &BTreeSet<String>,
        port_names: &BTreeSet<String>,
        words: &BTreeSet<String>,
    ) {
        let mut ports_of: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        if let Some(c) = container {
            ports_of.insert(
                c["id"].as_str().expect("container id").to_string(),
                node_ports(c),
            );
        }
        for node in nodes {
            let id = node["id"].as_str().expect("node id").to_string();
            let suffix = id.split(':').nth(1).expect("sNNNN:id grammar");
            assert!(raw_ids.contains(suffix), "invented node id {id}");
            match node["kind"].as_str().expect("kind") {
                "atomic" => {
                    assert!(
                        node.get("children").is_none() && node.get("disclosure").is_none(),
                        "{id}: atomic nodes carry no disclosure state"
                    );
                }
                "composite" => {
                    let digest = node["childSubgraphDigest"].as_str().expect("digest");
                    assert_eq!(digest.len(), 64);
                    assert!(
                        digest
                            .chars()
                            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
                        "childSubgraphDigest must be lowercase 64-hex"
                    );
                    let has_children = node.get("children").is_some();
                    let has_disclosure = node.get("disclosure").is_some();
                    assert!(
                        has_children != has_disclosure,
                        "{id}: exactly one disclosure state must render"
                    );
                    if let Some(d) = node.get("disclosure") {
                        assert_eq!(d["withheld"], serde_json::json!(true));
                        assert_eq!(d["reason"], "private-composite-default");
                        assert_eq!(d["category"], "composite-boundary");
                    }
                }
                other => panic!("unknown node kind {other}"),
            }
            for key in ["inputs", "outputs"] {
                if let Some(list) = node.get(key).and_then(|v| v.as_array()) {
                    for p in list {
                        let name = p["name"].as_str().expect("port name");
                        assert!(port_names.contains(name), "invented port {name}");
                        let ty = p["type"].as_str().expect("port type");
                        assert!(words.contains(ty), "invented type {ty}");
                        for outcomes in ["acceptedOutcomes", "emits"] {
                            if let Some(list) = p.get(outcomes).and_then(|v| v.as_array()) {
                                for o in list {
                                    let o = o.as_str().expect("outcome");
                                    assert!(words.contains(o), "invented outcome {o}");
                                }
                            }
                        }
                    }
                }
            }
            ports_of.insert(id, node_ports(node));
        }
        for edge in edges.as_array().expect("edges") {
            let label = edge["label"].as_array().expect("label");
            assert!(!label.is_empty(), "edge labels are never empty");
            for word in label {
                let w = word.as_str().expect("label word");
                assert!(words.contains(w), "invented label {w}");
            }
            for end in ["from", "to"] {
                let owner = edge[end]["node"].as_str().expect("endpoint node");
                let ports = ports_of
                    .get(owner)
                    .unwrap_or_else(|| panic!("endpoint {owner} is outside this scope"));
                let port = edge[end]["port"].as_str().expect("endpoint port");
                assert!(ports.contains(port), "{owner} declares no port {port}");
            }
        }
        for node in nodes {
            if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
                check_scope(
                    children,
                    Some(node),
                    &node["internalEdges"],
                    raw_ids,
                    port_names,
                    words,
                );
            }
        }
    }

    let env = nested_graph(Some(graph_policy(&["s0000:inner", "s0001:flow"], &[])));
    let nodes = env.payload["nodes"].as_array().expect("nodes");
    check_scope(
        nodes,
        None,
        &env.payload["edges"],
        &raw_ids,
        &port_names,
        &words,
    );
    for key in ["inputPorts", "outputPorts"] {
        for p in env.payload[key].as_array().expect("root ports") {
            let name = p["name"].as_str().expect("port name");
            assert!(port_names.contains(name), "invented root port {name}");
            let ty = p["type"].as_str().expect("port type");
            assert!(words.contains(ty), "invented root type {ty}");
        }
    }
}

/// Spec §12 test 19 — R1: a revealed container's input port supplies its
/// children with a shared-type `boundary-value` edge; a type mismatch fails
/// closed with `PROJ-0001`. The audit DOES compare boundary types per name
/// (rule 2), so the brief's literal craft — mutating `inner`'s boundary
/// input type — is refused as `SOMA-CMP-0005` before the builder runs.
/// The same R1 mismatch is produced by a conflicting *producer* instead:
/// an input-less operation supplies `staged` as `Shipment` while `inner`
/// consumes it as `Order`. Entry point and assertions are unchanged.
#[test]
fn composite_input_supplies_child_inputs() {
    let env = nested_graph(Some(graph_policy(&["s0001:flow"], &[])));
    let edges = env.payload["nodes"][1]["internalEdges"]
        .as_array()
        .expect("flow internalEdges");
    let r1 = edges
        .iter()
        .find(|e| e["to"]["node"] == "s0000:inner")
        .expect("R1 supply edge renders");
    assert_eq!(r1["kind"], "boundary-value");
    assert_eq!(r1["from"]["node"], "s0001:flow");
    assert_eq!(r1["from"]["port"], "staged");
    assert_eq!(r1["to"]["port"], "staged");
    assert_eq!(r1["label"], serde_json::json!(["Order"]));

    let mut raw: Value = serde_json::from_str(&nested_text()).expect("fixture");
    raw["body"][1]["body"]
        .as_array_mut()
        .expect("flow body array")
        .push(serde_json::json!({
            "schemaVersion": "1.1.0",
            "version": "1.1.0",
            "id": "mix",
            "executionClass": "deterministic",
            "inputs": [],
            "outputs": [
                { "name": "staged", "type": "Shipment", "emits": ["Produced"] }
            ],
            "effects": [],
            "uses": [],
            "secrets": [],
            "context": []
        }));
    let mismatch = parse_with_content_digest(&mut raw);
    let err = project_graph_json(&mismatch, None)
        .expect_err("incompatible boundary supply must be refused");
    assert!(
        err.iter()
            .any(|d| d.code == "PROJ-0001" && d.message.contains("incompatible boundary supply")),
        "expected an incompatible-boundary refusal, got {err:?}"
    );
}

/// Spec §12 test 20 — R2: a child output passes through to the container's
/// declared output port as the single-producer `boundary-value` edge.
#[test]
fn child_output_feeds_composite_output() {
    let env = nested_graph(Some(graph_policy(&["s0001:flow"], &[])));
    let edges = env.payload["nodes"][1]["internalEdges"]
        .as_array()
        .expect("flow internalEdges");
    let r2 = edges
        .iter()
        .find(|e| e["to"]["node"] == "s0001:flow")
        .expect("R2 pass-through edge renders");
    assert_eq!(r2["kind"], "boundary-value");
    assert_eq!(r2["from"]["node"], "s0000:inner");
    assert_eq!(r2["from"]["port"], "ready");
    assert_eq!(r2["to"]["port"], "ready");
    assert_eq!(r2["label"], serde_json::json!(["Shipment"]));
}

/// Spec §12 test 21 — R2 is a single-producer rule: zero or two internal
/// producers for a declared boundary output fail closed. Both crafts stay
/// audit-green (OUT rules are operations-only), so the refusal must come
/// from the graph builder.
#[test]
fn boundary_output_absent_or_ambiguous_fails() {
    let mut zero: Value = serde_json::from_str(&nested_text()).expect("fixture");
    zero["body"][1]["body"][0]["outputPorts"][0]["name"] = serde_json::json!("prepared");
    let zero = parse_with_content_digest(&mut zero);
    let err = project_graph_json(&zero, None).expect_err("absent producer must be refused");
    assert!(
        err.iter().any(|d| d.code == "PROJ-0001"
            && d.message
                .contains("absent producer for declared boundary output")),
        "expected an absent-producer refusal, got {err:?}"
    );

    let mut two: Value = serde_json::from_str(&nested_text()).expect("fixture");
    two["body"][1]["body"]
        .as_array_mut()
        .expect("flow body array")
        .push(serde_json::json!({
            "schemaVersion": "1.1.0",
            "version": "1.1.0",
            "id": "dup",
            "executionClass": "deterministic",
            "inputs": [
                { "name": "staged", "type": "Order", "acceptedOutcomes": ["Produced"] }
            ],
            "outputs": [
                { "name": "ready", "type": "Shipment", "emits": ["Produced"] }
            ],
            "effects": [],
            "uses": [],
            "secrets": [],
            "context": []
        }));
    let two = parse_with_content_digest(&mut two);
    let err = project_graph_json(&two, None).expect_err("ambiguous output must be refused");
    assert!(
        err.iter()
            .any(|d| d.code == "PROJ-0001" && d.message.contains("ambiguous boundary output")),
        "expected an ambiguous-output refusal, got {err:?}"
    );
}

/// Spec §12 test 23 — withheld rendering never rewires: parent adjacency
/// and any revealed container's internal edge set are disclosure-
/// independent, and parent edges terminate at container ports only.
#[test]
fn withheld_no_rewiring() {
    let default = nested_graph(None);
    let partial = nested_graph(Some(graph_policy(&["s0001:flow"], &[])));
    let full = nested_graph(Some(graph_policy(&["s0000:inner", "s0001:flow"], &[])));

    assert_eq!(
        default.payload["edges"], partial.payload["edges"],
        "parent adjacency must not depend on disclosure"
    );
    assert_eq!(
        default.payload["edges"], full.payload["edges"],
        "parent adjacency must not depend on disclosure"
    );
    assert_eq!(
        partial.payload["nodes"][1]["internalEdges"], full.payload["nodes"][1]["internalEdges"],
        "revealing a descendant must not rewire its container"
    );

    for edge in default.payload["edges"].as_array().expect("edges") {
        for end in ["from", "to"] {
            let id = edge[end]["node"].as_str().expect("endpoint node");
            assert!(
                id == "s0000:prep" || id == "s0001:flow" || id == "s0002:audit",
                "parent edge escaped to {id}"
            );
        }
    }
}

/// Spec §12 test 24 — a matched connection without `emits` data fails
/// closed ("rather than invent labels"); a declared-but-unused output is
/// legal (R7).
#[test]
fn matched_without_emits_fails_unused_output_passes() {
    let mut no_emits: Value = serde_json::from_str(&nested_text()).expect("fixture");
    no_emits["body"][0]["outputs"][0]
        .as_object_mut()
        .expect("output object")
        .remove("emits");
    let no_emits = parse_with_content_digest(&mut no_emits);
    let err = project_graph_json(&no_emits, None)
        .expect_err("matched output without emits must be refused");
    assert!(
        err.iter()
            .any(|d| d.code == "PROJ-0001" && d.message.contains("no derivable outcome label")),
        "expected a missing-label refusal, got {err:?}"
    );

    let mut unused: Value = serde_json::from_str(&nested_text()).expect("fixture");
    unused["body"][0]["outputs"]
        .as_array_mut()
        .expect("prep outputs")
        .push(serde_json::json!({
            "name": "orph",
            "type": "Order",
            "emits": ["Produced"]
        }));
    let unused = parse_with_content_digest(&mut unused);
    assert!(
        project_graph_json(&unused, None).is_ok(),
        "declared-but-unused outputs are legal (R7)"
    );
}
