//! Conformance suite for the Lite→SOMA governance compiler (issue #163).
//!
//! Proves: versioned mapping (req1), compile-before-expose (req2),
//! enriched diagnostics (req3), no-plan negatives per invalid family (req4),
//! digest binding (req5), and deterministic two-run output (req6).

use prometheos_lite::workflow::governance_compiler::{compile_workflow_text, verify_reviewed_plan};
use prometheos_lite::workflow::soma::Diagnostic;
use prometheos_lite::workflow::soma::contracts::WorkflowDefinition;
use prometheos_lite::workflow::soma::try_canonical_digest;
use serde_json::{Value, json};

const VENDORED: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/vendored/soma/v1.1");

fn fixture(rel: &str) -> String {
    std::fs::read_to_string(format!("{VENDORED}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// The `workflowDigest` rule: canonical digest of the serialized
/// `WorkflowDefinition` with `contentDigest` removed — the same rule the
/// audit uses for SOMA-CMP-0004.
fn expected_workflow_digest(text: &str) -> String {
    let model: WorkflowDefinition = serde_json::from_str(text).expect("valid fixture parses");
    let mut value = serde_json::to_value(&model).expect("model serializes");
    value
        .as_object_mut()
        .expect("workflow is an object")
        .remove("contentDigest");
    try_canonical_digest(&value).expect("fixtures satisfy the number policy")
}

#[test]
fn diagnostic_enrichment_serializes_and_round_trips() {
    let d = Diagnostic::new("SOMA-AUTH-0001", "authority denied")
        .with_source("wf-1", Some("related[0]".to_string()))
        .with_remediation("restrict-authority", "tighten the authority profile");
    let json = serde_json::to_value(&d).expect("serializes");
    assert_eq!(json["source"]["path"], "wf-1");
    assert_eq!(json["source"]["subject"], "related[0]");
    assert_eq!(json["remediation"]["action"], "restrict-authority");
    assert_eq!(
        json["remediation"]["summary"],
        "tighten the authority profile"
    );
    let back: Diagnostic = serde_json::from_value(json).expect("deserializes");
    assert_eq!(back, d);
}

#[test]
fn diagnostic_enrichment_is_omitted_by_default() {
    let d = Diagnostic::new("SOMA-CMP-0003", "schema violation: unknown field");
    let json = serde_json::to_value(&d).expect("serializes");
    assert!(json.get("source").is_none(), "source omitted when unset");
    assert!(
        json.get("remediation").is_none(),
        "remediation omitted when unset"
    );
    // Old diagnostics (no enrichment) still deserialize into the new struct.
    let legacy = serde_json::json!({
        "code": "SOMA-CMP-0003",
        "severity": "error",
        "category": "canonical_integrity",
        "message": "schema violation: unknown field"
    });
    let parsed: Diagnostic = serde_json::from_value(legacy).expect("legacy deserializes");
    assert!(parsed.source.is_none());
    assert!(parsed.remediation.is_none());
}

#[test]
fn valid_workflow_fixtures_compile_to_sealed_plans() {
    for rel in [
        "fixtures/valid/wf-valid-base.json",
        "fixtures/valid/wf-valid-composite.json",
        "fixtures/valid/wf-valid-gov.json",
    ] {
        let text = fixture(rel);
        let plan = compile_workflow_text(&text)
            .unwrap_or_else(|diags| panic!("{rel} must compile, got {diags:?}"));

        assert_eq!(plan.schema_version, "1.1.0", "{rel} schemaVersion");
        assert_eq!(plan.plan_version, "1.0.0", "{rel} planVersion");
        assert_eq!(
            plan.canonicalization.version, "1.0.0",
            "{rel} canonicalization.version"
        );

        // Steps mirror the workflow body in order.
        let wf: Value = serde_json::from_str(&text).expect("fixture parses");
        let body = wf["body"].as_array().expect("body is an array");
        assert_eq!(plan.steps.len(), body.len(), "{rel} step count");
        for (i, step) in plan.steps.iter().enumerate() {
            assert_eq!(step.key, format!("step-{i}"), "{rel} step key");
            assert_eq!(
                step.operation_id,
                body[i]["id"].as_str().expect("operation id"),
                "{rel} operationId"
            );
        }

        // workflowDigest binds the plan to the workflow artifact.
        assert_eq!(
            plan.workflow_digest,
            expected_workflow_digest(&text),
            "{rel} workflowDigest"
        );

        // Self-seal: canonicalization.sha256 is the digest of the plan
        // without its canonicalization member.
        let mut plan_value = serde_json::to_value(&plan).expect("plan serializes");
        plan_value
            .as_object_mut()
            .expect("plan is an object")
            .remove("canonicalization");
        let seal = try_canonical_digest(&plan_value).expect("plan satisfies the number policy");
        assert_eq!(plan.canonicalization.sha256, seal, "{rel} self-seal");
    }
}

#[test]
fn compile_is_byte_identical_across_two_runs() {
    let text = fixture("fixtures/valid/wf-valid-composite.json");
    let first = serde_json::to_string(&compile_workflow_text(&text).expect("compiles"))
        .expect("serializes");
    let second = serde_json::to_string(&compile_workflow_text(&text).expect("compiles"))
        .expect("serializes");
    assert_eq!(first, second, "two runs must produce identical plan JSON");
}

// ---------------------------------------------------------------------------
// req4: no-plan proofs for each invalid family (contract rows 1–5, 7)
// ---------------------------------------------------------------------------

/// Compile must fail with every expected code — and produce no plan.
fn assert_compile_fails_with(rel: &str, expected_codes: &[&str]) -> Vec<Diagnostic> {
    let text = fixture(rel);
    let diags = match compile_workflow_text(&text) {
        Ok(plan) => panic!("{rel} must NOT compile; got plan {plan:?}"),
        Err(diags) => diags,
    };
    for expected in expected_codes {
        assert!(
            diags.iter().any(|d| &d.code == expected),
            "{rel}: missing expected code {expected}; got {:?}",
            diags.iter().map(|d| &d.code).collect::<Vec<_>>()
        );
    }
    diags
}

#[test]
fn invalid_authority_family_produces_no_plan() {
    assert_compile_fails_with("fixtures/invalid/wf-auth-0001.json", &["SOMA-AUTH-0001"]);
    assert_compile_fails_with("fixtures/invalid/wf-auth-0003.json", &["SOMA-AUTH-0003"]);
    assert_compile_fails_with("fixtures/invalid/wf-auth-0005.json", &["SOMA-AUTH-0005"]);
    assert_compile_fails_with("fixtures/invalid/wf-auth-0006.json", &["SOMA-AUTH-0006"]);
}

#[test]
fn invalid_provider_family_produces_no_plan() {
    assert_compile_fails_with("fixtures/invalid/wf-auth-0004.json", &["SOMA-AUTH-0004"]);
}

#[test]
fn invalid_review_family_produces_no_plan() {
    assert_compile_fails_with("fixtures/invalid/wf-auth-0007.json", &["SOMA-AUTH-0007"]);
    assert_compile_fails_with("fixtures/invalid/wf-auth-0010.json", &["SOMA-AUTH-0010"]);
}

#[test]
fn invalid_recovery_family_produces_no_plan() {
    assert_compile_fails_with("fixtures/invalid/wf-auth-0008.json", &["SOMA-AUTH-0008"]);
}

#[test]
fn invalid_composite_family_produces_no_plan() {
    assert_compile_fails_with("fixtures/invalid/wf-auth-0002.json", &["SOMA-AUTH-0002"]);
    assert_compile_fails_with("fixtures/invalid/wf-exp-0007.json", &["SOMA-EXP-0007"]);
}

#[test]
fn malformed_input_refuses_with_cmp_0003_and_no_plan() {
    let diags = match compile_workflow_text("{not json") {
        Ok(plan) => panic!("malformed input must NOT compile; got plan {plan:?}"),
        Err(diags) => diags,
    };
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code, "SOMA-CMP-0003");
    assert_eq!(diags[0].severity, "error");
    assert!(
        diags[0].source.is_none(),
        "workflow id is unextractable from malformed input"
    );
    assert!(
        diags[0].remediation.is_some(),
        "remediation always attached"
    );
}

// ---------------------------------------------------------------------------
// req6 (mutation): a mutated valid workflow must fail to compile
// ---------------------------------------------------------------------------

#[test]
fn mutated_valid_workflow_fails_to_compile() {
    let text = fixture("fixtures/valid/wf-valid-base.json");

    // Mutation A: inject an unknown member (deny_unknown_fields).
    let mut wf: Value = serde_json::from_str(&text).expect("fixture parses");
    wf.as_object_mut()
        .expect("object")
        .insert("bogusMember".into(), json!(1));
    let mutated = serde_json::to_string(&wf).expect("serializes");
    let diags = match compile_workflow_text(&mutated) {
        Ok(plan) => panic!("unknown member must NOT compile; got plan {plan:?}"),
        Err(diags) => diags,
    };
    assert!(
        diags.iter().any(|d| d.code == "SOMA-CMP-0003"),
        "got {:?}",
        diags.iter().map(|d| &d.code).collect::<Vec<_>>()
    );

    // Mutation B: drop a required member.
    let mut wf: Value = serde_json::from_str(&text).expect("fixture parses");
    wf.as_object_mut().expect("object").remove("name");
    let mutated = serde_json::to_string(&wf).expect("serializes");
    let diags = match compile_workflow_text(&mutated) {
        Ok(plan) => panic!("missing required member must NOT compile; got plan {plan:?}"),
        Err(diags) => diags,
    };
    assert!(
        diags.iter().any(|d| d.code == "SOMA-CMP-0003"),
        "got {:?}",
        diags.iter().map(|d| &d.code).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// req3: every diagnostic on a failed compile carries code, severity,
// explanation, source, and remediation
// ---------------------------------------------------------------------------

#[test]
fn failed_compiles_carry_full_diagnostics() {
    let rel = "fixtures/invalid/wf-auth-0001.json";
    let text = fixture(rel);
    let workflow_id = serde_json::from_str::<Value>(&text).expect("fixture parses")["id"]
        .as_str()
        .expect("id")
        .to_string();
    let diags = match compile_workflow_text(&text) {
        Ok(plan) => panic!("must NOT compile; got plan {plan:?}"),
        Err(diags) => diags,
    };
    assert!(!diags.is_empty());
    for d in &diags {
        assert!(!d.code.is_empty(), "stable code present");
        assert_eq!(d.severity, "error");
        assert!(!d.message.is_empty(), "explanation present");
        let source = d.source.as_ref().expect("source attached");
        assert_eq!(source.path, workflow_id, "source path = workflow id");
        // Operation-level subject mirrors the diagnostic's related entry.
        assert_eq!(
            source.subject,
            d.related.first().cloned(),
            "subject = first related entry"
        );
        let remediation = d.remediation.as_ref().expect("remediation attached");
        assert!(!remediation.summary.is_empty(), "remediation summary");
    }
}

#[test]
fn every_invalid_workflow_fixture_fails_to_compile_with_pinned_codes() {
    #[derive(serde::Deserialize)]
    struct FixturePin {
        path: String,
        kind: String,
        expected_codes: Vec<String>,
        artifact: String,
    }
    #[derive(serde::Deserialize)]
    struct FixtureManifest {
        fixtures: Vec<FixturePin>,
    }
    let manifest: FixtureManifest =
        serde_json::from_str(&fixture("fixtures/manifest.json")).expect("manifest parses");
    let mut invalid_checked = 0usize;
    for fx in &manifest.fixtures {
        if fx.artifact != "WorkflowDefinition" || fx.kind != "invalid" {
            continue;
        }
        invalid_checked += 1;
        let diags = assert_compile_fails_with(&fx.path, &[]);
        for pinned in &fx.expected_codes {
            assert!(
                diags.iter().any(|d| &d.code == pinned),
                "{}: missing pinned code {pinned}; got {:?}",
                fx.path,
                diags.iter().map(|d| &d.code).collect::<Vec<_>>()
            );
        }
    }
    assert!(invalid_checked >= 25, "corpus shrank: {invalid_checked}");
}

#[test]
fn emitted_diagnostics_match_the_published_catalogue() {
    // Catalogue ground truth: every emitted code must exist in the pinned
    // diagnostics.json and carry the published severity (all are "error").
    let catalogue: Value =
        serde_json::from_str(&fixture("diagnostics.json")).expect("catalogue parses");
    let published: std::collections::HashMap<&str, &str> = catalogue["codes"]
        .as_array()
        .expect("catalogue.codes is an array")
        .iter()
        .map(|e| {
            (
                e["code"].as_str().expect("code"),
                e["severity"].as_str().expect("severity"),
            )
        })
        .collect();

    #[derive(serde::Deserialize)]
    struct FixturePin {
        path: String,
        kind: String,
        artifact: String,
    }
    #[derive(serde::Deserialize)]
    struct FixtureManifest {
        fixtures: Vec<FixturePin>,
    }
    let manifest: FixtureManifest =
        serde_json::from_str(&fixture("fixtures/manifest.json")).expect("manifest parses");

    let mut failures = Vec::new();
    let mut checked = 0usize;
    for fx in &manifest.fixtures {
        if fx.artifact != "WorkflowDefinition" || fx.kind != "invalid" {
            continue;
        }
        let text = fixture(&fx.path);
        let diags = match compile_workflow_text(&text) {
            Ok(plan) => panic!("{}: must NOT compile; got {plan:?}", fx.path),
            Err(diags) => diags,
        };
        for d in &diags {
            checked += 1;
            match published.get(d.code.as_str()) {
                None => failures.push(format!("{}: {} not in catalogue", fx.path, d.code)),
                Some(severity) if *severity != d.severity => failures.push(format!(
                    "{}: {} severity {} != published {severity}",
                    fx.path, d.code, d.severity
                )),
                Some(_) => {}
            }
            if d.remediation.is_none() {
                failures.push(format!("{}: {} missing remediation", fx.path, d.code));
            }
        }
    }
    assert_eq!(
        failures,
        Vec::<String>::new(),
        "catalogue conformance failures over {checked} diagnostics:\n{}",
        failures.join("\n")
    );
    assert!(checked >= 25, "diagnostic corpus shrank: {checked}");
}

// ---------------------------------------------------------------------------
// req1: versioned Lite → SOMA authority mapping
// ---------------------------------------------------------------------------

fn snapshot_with_escalation(
    escalation_target: &str,
) -> prometheos_lite::workflow::policy::EffectiveExecutionSnapshotV1 {
    prometheos_lite::workflow::policy::EffectiveExecutionSnapshotV1 {
        schema_version: "1.0.0".into(),
        snapshot_id: "snap-1".into(),
        node_id: "node-1".into(),
        readable_scopes: vec!["specs".into(), "docs".into()],
        writable_scopes: vec!["specs/drafts".into()],
        token_budget: Some(50_000),
        denied_providers: vec!["external".into()],
        forbidden_paths: vec![".env".into()],
        max_attempts: 3,
        escalation_target: escalation_target.into(),
        recorded_at: "2026-09-26T00:00:00Z".into(),
    }
}

#[test]
fn mapping_matrix_reflects_authority_level() {
    use prometheos_lite::workflow::AuthorityLevel;
    use prometheos_lite::workflow::governance_compiler::{MAPPING_VERSION, map_lite_authority};
    use prometheos_lite::workflow::soma::types::{ExecutionClass, MutationMode};

    let snapshot = snapshot_with_escalation("ops-oncall");
    for (level, expect_explicit) in [
        (AuthorityLevel::Review, false),
        (AuthorityLevel::Propose, false),
        (AuthorityLevel::Assist, true),
        (AuthorityLevel::Execute, true),
    ] {
        let mapping = map_lite_authority(level, &snapshot, ExecutionClass::Deterministic);
        assert_eq!(mapping.version, MAPPING_VERSION, "{level:?} versioned");
        let expected = if expect_explicit {
            MutationMode::Explicit
        } else {
            MutationMode::None_
        };
        assert_eq!(
            mapping.authority.mutation, expected,
            "{level:?} mutation mode"
        );
        assert_eq!(
            mapping.authority.execution_class,
            ExecutionClass::Deterministic
        );
        assert_eq!(
            mapping.authority.readable_scopes.as_ref(),
            Some(&snapshot.readable_scopes),
            "readable scopes transfer"
        );
        assert_eq!(
            mapping.authority.writable_scopes.as_ref(),
            Some(&snapshot.writable_scopes),
            "writable scopes transfer"
        );
        assert_eq!(
            mapping.authority.escalation.as_ref().map(|e| &e.to),
            Some(&snapshot.escalation_target),
            "escalation target transfers"
        );
    }
}

#[test]
fn mapping_omits_escalation_when_no_target_declared() {
    use prometheos_lite::workflow::AuthorityLevel;
    use prometheos_lite::workflow::governance_compiler::map_lite_authority;
    use prometheos_lite::workflow::soma::types::ExecutionClass;

    let snapshot = snapshot_with_escalation("");
    let mapping = map_lite_authority(
        AuthorityLevel::Execute,
        &snapshot,
        ExecutionClass::Deterministic,
    );
    assert!(
        mapping.authority.escalation.is_none(),
        "no target -> no escalation policy"
    );
}

#[test]
fn mapping_discloses_lite_enforced_restrictions() {
    use prometheos_lite::workflow::AuthorityLevel;
    use prometheos_lite::workflow::governance_compiler::{LITE_ENFORCED_ONLY, map_lite_authority};
    use prometheos_lite::workflow::soma::types::ExecutionClass;

    let snapshot = snapshot_with_escalation("ops-oncall");
    let mapping = map_lite_authority(
        AuthorityLevel::Execute,
        &snapshot,
        ExecutionClass::ModelAssisted,
    );
    let disclosed: Vec<&str> = mapping
        .lite_enforced_only
        .iter()
        .map(String::as_str)
        .collect();
    assert_eq!(
        disclosed, LITE_ENFORCED_ONLY,
        "no restriction silently dropped"
    );
    // The restrictions must actually be present on the snapshot.
    assert_eq!(snapshot.denied_providers, ["external"]);
    assert_eq!(snapshot.forbidden_paths, [".env"]);
    assert_eq!(snapshot.max_attempts, 3);
    assert_eq!(snapshot.token_budget, Some(50_000));
}

#[test]
fn mapping_serializes_in_camel_case_and_round_trips() {
    use prometheos_lite::workflow::AuthorityLevel;
    use prometheos_lite::workflow::governance_compiler::{LiteToSomaMappingV1, map_lite_authority};
    use prometheos_lite::workflow::soma::types::ExecutionClass;

    let snapshot = snapshot_with_escalation("ops-oncall");
    let mapping = map_lite_authority(
        AuthorityLevel::Assist,
        &snapshot,
        ExecutionClass::Deterministic,
    );
    let value = serde_json::to_value(&mapping).expect("serializes");
    assert!(value.get("liteEnforcedOnly").is_some(), "camelCase key");
    assert!(value.get("version").is_some());
    assert!(value.get("authority").is_some());

    let parsed: LiteToSomaMappingV1 =
        serde_json::from_value(value).expect("strict round-trip (deny_unknown_fields)");
    assert_eq!(parsed, mapping);
}

// ---------------------------------------------------------------------------
// req5: digest binding — execution refuses plans whose compiled identity
// differs from the reviewed plan
// ---------------------------------------------------------------------------

/// Compile a valid workflow and return the sealed plan as JSON text.
fn compiled_plan_text(rel: &str) -> String {
    let plan = compile_workflow_text(&fixture(rel)).expect("valid fixture compiles");
    serde_json::to_string(&plan).expect("plan serializes")
}

fn verify_err(plan_text: &str, reviewed_identity: &str) -> Vec<Diagnostic> {
    match verify_reviewed_plan(plan_text, reviewed_identity) {
        Ok(()) => panic!("plan must be refused"),
        Err(diags) => diags,
    }
}

fn assert_code(diags: &[Diagnostic], expected: &str) {
    assert!(
        diags.iter().any(|d| d.code == expected),
        "expected {expected}; got {:?}",
        diags
            .iter()
            .map(|d| format!("{}: {}", d.code, d.message))
            .collect::<Vec<_>>()
    );
}

/// Recompute and overwrite `canonicalization.sha256` on a plan object
/// (mirrors the compiler's sealing rule).
fn reseal(plan: &mut Value) {
    let mut for_digest = plan.clone();
    for_digest
        .as_object_mut()
        .expect("plan is an object")
        .remove("canonicalization");
    let seal = try_canonical_digest(&for_digest).expect("number policy holds");
    plan["canonicalization"]["sha256"] = json!(seal);
}

#[test]
fn reviewed_plan_identity_is_accepted() {
    let plan_text = compiled_plan_text("fixtures/valid/wf-valid-base.json");
    let plan: Value = serde_json::from_str(&plan_text).expect("parses");
    let reviewed_identity = plan["canonicalization"]["sha256"].as_str().expect("sha256");
    verify_reviewed_plan(&plan_text, reviewed_identity).expect("matching identity accepted");
}

#[test]
fn plan_with_broken_seal_is_refused_as_integrity_failure() {
    let plan_text = compiled_plan_text("fixtures/valid/wf-valid-base.json");
    let mut plan: Value = serde_json::from_str(&plan_text).expect("parses");
    let reviewed_identity = plan["canonicalization"]["sha256"]
        .as_str()
        .expect("sha256")
        .to_string();
    // Tamper with a step but leave the stale seal in place.
    plan["steps"][0]["operationId"] = json!("tampered-op");
    let tampered = serde_json::to_string(&plan).expect("serializes");
    let diags = verify_err(&tampered, &reviewed_identity);
    assert_code(&diags, "SOMA-CMP-0004");
    assert!(
        diags.iter().any(|d| d.code == "SOMA-CMP-0004"
            && d.remediation.is_some()
            && d.message.contains("reviewed")),
        "tamper refusal mentions the reviewed-plan binding; got {diags:?}"
    );
}

#[test]
fn tampered_and_resealed_plan_is_still_refused() {
    let plan_text = compiled_plan_text("fixtures/valid/wf-valid-composite.json");
    let mut plan: Value = serde_json::from_str(&plan_text).expect("parses");
    let reviewed_identity = plan["canonicalization"]["sha256"]
        .as_str()
        .expect("sha256")
        .to_string();
    // Attacker changes plan content AND recomputes the seal.
    plan["steps"][0]["operationId"] = json!("attacker-op");
    reseal(&mut plan);
    let tampered = serde_json::to_string(&plan).expect("serializes");
    let diags = verify_err(&tampered, &reviewed_identity);
    assert_code(&diags, "SOMA-CMP-0004");
    assert!(
        diags.iter().any(|d| d
            .remediation
            .as_ref()
            .is_some_and(|r| r.summary.contains("reviewed"))),
        "remediation points at the reviewed identity; got {diags:?}"
    );
}

#[test]
fn malformed_plan_refused_with_cmp_0003() {
    let diags = verify_err("{not a plan", "whatever");
    assert_code(&diags, "SOMA-CMP-0003");
}

#[test]
fn unsupported_plan_version_refused_with_cmp_0001() {
    let plan_text = compiled_plan_text("fixtures/valid/wf-valid-base.json");
    let mut plan: Value = serde_json::from_str(&plan_text).expect("parses");
    plan["planVersion"] = json!("2.0.0");
    reseal(&mut plan);
    let tampered = serde_json::to_string(&plan).expect("serializes");
    let identity = plan["canonicalization"]["sha256"]
        .as_str()
        .expect("sha256")
        .to_string();
    let diags = verify_err(&tampered, &identity);
    assert_code(&diags, "SOMA-CMP-0001");
}

#[test]
fn wrong_canonicalization_version_refused_with_cmp_0001() {
    let plan_text = compiled_plan_text("fixtures/valid/wf-valid-base.json");
    let mut plan: Value = serde_json::from_str(&plan_text).expect("parses");
    plan["canonicalization"]["version"] = json!("9.9.9");
    let tampered = serde_json::to_string(&plan).expect("serializes");
    let identity = plan["canonicalization"]["sha256"]
        .as_str()
        .expect("sha256")
        .to_string();
    let diags = verify_err(&tampered, &identity);
    assert_code(&diags, "SOMA-CMP-0001");
}
