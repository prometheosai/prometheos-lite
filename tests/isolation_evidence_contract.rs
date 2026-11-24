//! Machine-readable isolation evidence contract for the merged manual Windows
//! acceptance workflow (PR #346, main `981c3995`).
//!
//! This target is NONPRIVILEGED and deterministic. It VALIDATES evidence
//! documents; it never creates, signs, or approves one. A validator pass means
//! only "the document has the required, distinguishable fields" — it is not
//! proof of runner isolation and not Stage 2 security acceptance.
//!
//! Trust boundary:
//! - Provider-attested: values emitted by the GitHub Actions runner for THIS
//!   run (run id, attempt, repository, ref, sha, actor, runner environment).
//!   They are provider-controlled, not self-asserted by test code, but are
//!   meaningful only with the reviewed workflow that does not overwrite them.
//! - OS-derived: values read from kernel/registry APIs inside the runner
//!   (OS build, ProgramData volume/file identity, drive types).
//! - Self-asserted: ordinary environment variables or test-supplied strings.
//!   These can never satisfy the isolation gate alone.
//! - Unprovable from inside the runner: image disposability and the complete
//!   absence of host-mounted protected resources. These stay PROVIDER_ASSERTED
//!   / UNPROVABLE and must never be promoted to independently attested.

use serde_json::Value;

const SCHEMA: &str = "prometheos.isolation-evidence.v1";
const REPOSITORY: &str = "prometheosai/prometheos-lite";
const APPROVED_WORKFLOW: &str = ".github/workflows/pr341-windows-acceptance.yml";
const REVIEWED_WORKFLOW_SHA256: &str =
    "b45b5ed1c912cef4d14251563fe2366f76c1d28287ce2e2291ef84f55d01b84c";
const REVIEWED_MAIN_SHA: &str = "981c39955fb82b181202b932bfeaf2119b22e669";
const REVIEWED_PR341_SHA: &str = "37a7ae33ca21fc25ad0f70f20022cd4a95634bce";
const ZEROED_STAGE2_ATTESTATION: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

fn field<'a>(root: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = root;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current)
}

fn req_str<'a>(root: &'a Value, path: &[&str], out: &mut Vec<String>) -> Option<&'a str> {
    match field(root, path).and_then(Value::as_str) {
        Some(value) if !value.is_empty() => Some(value),
        _ => {
            out.push(format!("missing or empty {}", path.join(".")));
            None
        }
    }
}

fn req_bool(root: &Value, path: &[&str], expected: bool, out: &mut Vec<String>) {
    match field(root, path).and_then(Value::as_bool) {
        Some(value) if value == expected => {}
        Some(_) => out.push(format!("{} must be {expected}", path.join("."))),
        None => out.push(format!("missing boolean {}", path.join("."))),
    }
}

fn req_u64(root: &Value, path: &[&str], minimum: u64, out: &mut Vec<String>) {
    match field(root, path).and_then(Value::as_u64) {
        Some(value) if value >= minimum => {}
        Some(_) => out.push(format!("{} below {minimum}", path.join("."))),
        None => out.push(format!("missing integer {}", path.join("."))),
    }
}

fn req_zero(root: &Value, path: &[&str], out: &mut Vec<String>) {
    match field(root, path).and_then(Value::as_u64) {
        Some(0) => {}
        Some(_) => out.push(format!("{} must be zero", path.join("."))),
        None => out.push(format!("missing integer {}", path.join("."))),
    }
}

/// Fail-closed validation. An empty error list means the document is
/// structurally complete; it is NOT an isolation or acceptance proof.
fn validate_evidence(doc: &Value) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    if req_str(doc, &["schemaVersion"], &mut errors) != Some(SCHEMA) {
        errors.push("unknown schemaVersion".into());
    }
    if req_str(doc, &["event"], &mut errors) != Some("workflow_dispatch") {
        errors.push("workflow trigger is not workflow_dispatch".into());
    }
    if req_str(doc, &["repository"], &mut errors) != Some(REPOSITORY) {
        errors.push("repository binding mismatch".into());
    }
    if req_str(doc, &["headSha"], &mut errors) != Some(REVIEWED_PR341_SHA) {
        errors.push("candidate revision binding mismatch".into());
    }
    if req_str(doc, &["workflowPath"], &mut errors) != Some(APPROVED_WORKFLOW) {
        errors.push("workflow path binding mismatch".into());
    }
    if req_str(doc, &["workflowSha256"], &mut errors) != Some(REVIEWED_WORKFLOW_SHA256) {
        errors.push("reviewed workflow digest mismatch".into());
    }
    if req_str(doc, &["workflowRevision"], &mut errors) != Some(REVIEWED_MAIN_SHA) {
        errors.push("workflow revision binding mismatch".into());
    }

    if let Some(run_id) = req_str(doc, &["runId"], &mut errors)
        && !run_id.bytes().all(|b| b.is_ascii_digit())
    {
        errors.push("runId must be a GitHub run number".into());
    }
    req_u64(doc, &["runAttempt"], 1, &mut errors);

    if req_str(doc, &["providerAttested", "runnerEnvironment"], &mut errors)
        != Some("github-hosted")
    {
        errors.push("runner is not provider-attested github-hosted".into());
    }
    if req_str(doc, &["providerAttested", "runnerOs"], &mut errors) != Some("windows") {
        errors.push("runner OS is not provider-attested windows".into());
    }
    req_str(doc, &["providerAttested", "workflowRef"], &mut errors);
    req_str(doc, &["providerAttested", "actor"], &mut errors);

    // OS-derived identity, not environment variables.
    req_str(doc, &["osDerived", "buildLabEx"], &mut errors);
    req_str(doc, &["osDerived", "installationType"], &mut errors);
    if req_str(doc, &["programData", "path"], &mut errors)
        .is_some_and(|p| !p.starts_with("C:\\") && !p.starts_with("D:\\"))
    {
        errors.push("ProgramData path is not an absolute fixed drive path".into());
    }
    req_bool(doc, &["programData", "reparse"], false, &mut errors);
    req_u64(doc, &["programData", "volumeSerial"], 1, &mut errors);
    req_u64(doc, &["programData", "fileIndex"], 1, &mut errors);
    req_bool(
        doc,
        &["programData", "prometheosPathAbsentBeforeRun"],
        true,
        &mut errors,
    );

    req_bool(doc, &["hostMounts", "enumerated"], true, &mut errors);
    req_zero(doc, &["hostMounts", "remoteCount"], &mut errors);
    req_zero(doc, &["hostMounts", "protectedMountCount"], &mut errors);

    // Self-asserted claims can never satisfy the gate.
    req_bool(doc, &["selfAssertedOnly"], false, &mut errors);

    // Keep the Stage 2 gate closed: no attestation may be self-approved here.
    if req_str(doc, &["stage2AttestationSha256"], &mut errors) != Some(ZEROED_STAGE2_ATTESTATION) {
        errors.push("Stage 2 attestation digest must remain zeroed".into());
    }
    req_bool(doc, &["stage2Executed"], false, &mut errors);

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClaimClass {
    ProviderAttested,
    OsDerived,
    SelfAsserted,
    Unprovable,
    NotRun,
}

fn claim_class(claim: &str) -> ClaimClass {
    match claim {
        "provider.run_id" | "provider.environment" | "provider.os" | "provider.actor" => {
            ClaimClass::ProviderAttested
        }
        "os.build" | "programdata.identity" | "hostmounts.drive_types" => ClaimClass::OsDerived,
        "self.env_vars" | "self.test_strings" => ClaimClass::SelfAsserted,
        "image.disposable" | "hostmounts.no_protected_resources_anywhere" => ClaimClass::Unprovable,
        "stage2.privileged" | "production.writer_lock" | "crash.retry_recovery" => {
            ClaimClass::NotRun
        }
        _ => ClaimClass::NotRun,
    }
}

fn valid_document() -> Value {
    serde_json::json!({
        "schemaVersion": SCHEMA,
        "event": "workflow_dispatch",
        "repository": REPOSITORY,
        "headSha": REVIEWED_PR341_SHA,
        "workflowPath": APPROVED_WORKFLOW,
        "workflowSha256": REVIEWED_WORKFLOW_SHA256,
        "workflowRevision": REVIEWED_MAIN_SHA,
        "runId": "123456789",
        "runAttempt": 1,
        "providerAttested": {
            "runnerEnvironment": "github-hosted",
            "runnerOs": "windows",
            "workflowRef": "refs/heads/main",
            "actor": "diegorhoger"
        },
        "osDerived": {
            "buildLabEx": "20348.1.amd64fre",
            "installationType": "Server"
        },
        "programData": {
            "path": "C:\\ProgramData",
            "reparse": false,
            "volumeSerial": 123456789,
            "fileIndex": 42,
            "prometheosPathAbsentBeforeRun": true
        },
        "hostMounts": {
            "enumerated": true,
            "remoteCount": 0,
            "protectedMountCount": 0
        },
        "selfAssertedOnly": false,
        "stage2AttestationSha256": ZEROED_STAGE2_ATTESTATION,
        "stage2Executed": false
    })
}

#[test]
fn valid_evidence_document_passes_fail_closed_validation() {
    assert!(validate_evidence(&valid_document()).is_ok());
}

#[test]
fn self_asserted_environment_only_is_rejected() {
    let mut doc = valid_document();
    doc["selfAssertedOnly"] = serde_json::json!(true);
    assert!(validate_evidence(&doc).is_err());
}

#[test]
fn revision_and_workflow_binding_are_enforced() {
    let mut doc = valid_document();
    doc["headSha"] = serde_json::json!("deadbeef");
    assert!(validate_evidence(&doc).is_err());

    let mut doc = valid_document();
    doc["workflowSha256"] = serde_json::json!("ffff");
    assert!(validate_evidence(&doc).is_err());

    let mut doc = valid_document();
    doc["workflowRevision"] = serde_json::json!("deadbeef");
    assert!(validate_evidence(&doc).is_err());
}

#[test]
fn missing_os_derived_attestation_is_rejected() {
    let mut doc = valid_document();
    doc.as_object_mut().expect("object").remove("osDerived");
    assert!(validate_evidence(&doc).is_err());
}

#[test]
fn reparse_programdata_and_remote_mounts_are_rejected() {
    let mut doc = valid_document();
    doc["programData"]["reparse"] = serde_json::json!(true);
    assert!(validate_evidence(&doc).is_err());

    let mut doc = valid_document();
    doc["hostMounts"]["remoteCount"] = serde_json::json!(1);
    assert!(validate_evidence(&doc).is_err());
}

#[test]
fn zeroed_stage2_attestation_is_required() {
    let mut doc = valid_document();
    doc["stage2AttestationSha256"] = serde_json::json!("abcd");
    assert!(validate_evidence(&doc).is_err());
    let mut doc = valid_document();
    doc["stage2Executed"] = serde_json::json!(true);
    assert!(validate_evidence(&doc).is_err());
}

#[test]
fn evidence_classes_are_explicit_and_unprovable_claims_stay_unprovable() {
    assert_eq!(
        claim_class("provider.environment"),
        ClaimClass::ProviderAttested
    );
    assert_eq!(claim_class("os.build"), ClaimClass::OsDerived);
    assert_eq!(claim_class("self.env_vars"), ClaimClass::SelfAsserted);
    assert_eq!(claim_class("image.disposable"), ClaimClass::Unprovable);
    assert_eq!(
        claim_class("hostmounts.no_protected_resources_anywhere"),
        ClaimClass::Unprovable
    );
    assert_eq!(claim_class("stage2.privileged"), ClaimClass::NotRun);
}
