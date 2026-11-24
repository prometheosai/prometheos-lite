//! Machine-readable isolation evidence contract for the merged manual Windows
//! acceptance workflow (PR #346, main `981c3995`).
//!
//! This target is NONPRIVILEGED and deterministic. It checks DOCUMENT SHAPE
//! only. It never creates, signs, authenticates, approves or accepts evidence.
//! A pass means only "the caller-supplied document has the required,
//! distinguishable fields" — `STRUCTURAL_ONLY / UNVERIFIED_PROVENANCE`.
//! All provider/OS values in such a document are CLAIMS until sourced through
//! a separately trusted channel. There is no promotion path to authentication
//! or Stage 2 acceptance in this prototype.
//!
//! Trust boundary:
//! - Claimed provider fields (run id, attempt, repository, refs, sha, actor,
//!   runner environment): values ASSERTED by the document. This prototype has
//!   no trusted channel to GitHub, so they are `Claimed`, never authenticated.
//! - OS-derived: values read from kernel/registry APIs inside the runner
//!   (OS build, ProgramData volume/file identity, drive types).
//! - Self-asserted: ordinary environment variables or test-supplied strings.
//!   These can never satisfy the isolation gate alone.
//! - Unprovable from inside the runner: image disposability and the complete
//!   absence of host-mounted protected resources. These stay PROVIDER_ASSERTED
//!   / UNPROVABLE and must never be promoted to independently attested.

use std::fs;
use std::iter::once;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr;

use serde_json::{Value, json};
use winapi::shared::minwindef::TRUE;
use winapi::um::fileapi::{
    BY_HANDLE_FILE_INFORMATION, CreateFileW, GetFileInformationByHandle, OPEN_EXISTING,
};
use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
use winapi::um::winbase::FILE_FLAG_BACKUP_SEMANTICS;
use winapi::um::winnt::{
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    GENERIC_READ,
};

const SCHEMA: &str = "prometheos.isolation-evidence.v1";
const REPOSITORY: &str = "prometheosai/prometheos-lite";
const APPROVED_WORKFLOW: &str = ".github/workflows/pr341-windows-acceptance.yml";
const REVIEWED_WORKFLOW_SHA256: &str =
    "b45b5ed1c912cef4d14251563fe2366f76c1d28287ce2e2291ef84f55d01b84c";
const REVIEWED_MAIN_SHA: &str = "981c39955fb82b181202b932bfeaf2119b22e669";
const REVIEWED_PR341_SHA: &str = "37a7ae33ca21fc25ad0f70f20022cd4a95634bce";
/// GitHub `GITHUB_WORKFLOW_REF` format is `{owner}/{repo}/{workflow_path}@{ref}`.
/// This is NOT `GITHUB_REF` (event ref) and NOT the checked-out commit SHA.
const EXPECTED_WORKFLOW_REF: &str =
    "prometheosai/prometheos-lite/.github/workflows/pr341-windows-acceptance.yml@refs/heads/main";
const EXPECTED_EVENT_REF: &str = "refs/heads/main";
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StructuralStatus {
    StructurallyValid,
    StructurallyInvalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProvenanceStatus {
    /// No trusted channel exists in this prototype. Always unverified.
    Unverified,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StructuralOnlyResult {
    structure: StructuralStatus,
    provenance: ProvenanceStatus,
    issues: Vec<String>,
}

/// Deliberately impossible: a structural check cannot authenticate a caller-
/// supplied document. No signature, issuer or credential is introduced here.
fn promote_to_authenticated(_: &StructuralOnlyResult) -> Result<(), &'static str> {
    Err("AUTHENTICATION_NOT_AVAILABLE: structural contract cannot authenticate provenance")
}

/// Deliberately impossible: this prototype produces no security acceptance.
fn isolation_acceptance(_: &StructuralOnlyResult) -> Result<(), &'static str> {
    Err("ISOLATION_ACCEPTANCE_NOT_AVAILABLE: no trusted isolation evidence producer or consumer")
}

/// Fail-closed STRUCTURAL check only. Never an authentication, isolation or
/// Stage 2 acceptance result.
fn validate_structure_only(doc: &Value) -> StructuralOnlyResult {
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
    if req_str(doc, &["providerAttested", "workflowRef"], &mut errors)
        != Some(EXPECTED_WORKFLOW_REF)
    {
        errors.push("workflowRef is not the exact reviewed workflow identity".into());
    }
    // GITHUB_REF (event ref) is a different value from GITHUB_WORKFLOW_REF.
    if req_str(doc, &["providerAttested", "eventRef"], &mut errors) != Some(EXPECTED_EVENT_REF) {
        errors.push("eventRef is not the reviewed dispatch branch".into());
    }
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

    StructuralOnlyResult {
        structure: if errors.is_empty() {
            StructuralStatus::StructurallyValid
        } else {
            StructuralStatus::StructurallyInvalid
        },
        provenance: ProvenanceStatus::Unverified,
        issues: errors,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClaimClass {
    Claimed,
    OsDerived,
    SelfAsserted,
    Unprovable,
    NotRun,
}

fn claim_class(claim: &str) -> ClaimClass {
    match claim {
        "provider.run_id"
        | "provider.environment"
        | "provider.os"
        | "provider.actor"
        | "provider.workflow_ref" => ClaimClass::Claimed,
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
            "workflowRef": EXPECTED_WORKFLOW_REF,
            "eventRef": EXPECTED_EVENT_REF,
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
fn valid_evidence_document_is_only_structurally_valid() {
    let result = validate_structure_only(&valid_document());
    assert_eq!(result.structure, StructuralStatus::StructurallyValid);
    assert_eq!(result.provenance, ProvenanceStatus::Unverified);
    assert!(result.issues.is_empty());
}

#[test]
fn self_asserted_environment_only_is_rejected() {
    let mut doc = valid_document();
    doc["selfAssertedOnly"] = serde_json::json!(true);
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyInvalid
    );
}

#[test]
fn revision_and_workflow_binding_are_enforced() {
    let mut doc = valid_document();
    doc["headSha"] = serde_json::json!("deadbeef");
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyInvalid
    );

    let mut doc = valid_document();
    doc["workflowSha256"] = serde_json::json!("ffff");
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyInvalid
    );

    let mut doc = valid_document();
    doc["workflowRevision"] = serde_json::json!("deadbeef");
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyInvalid
    );
}

#[test]
fn missing_os_derived_attestation_is_rejected() {
    let mut doc = valid_document();
    doc.as_object_mut().expect("object").remove("osDerived");
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyInvalid
    );
}

#[test]
fn reparse_programdata_and_remote_mounts_are_rejected() {
    let mut doc = valid_document();
    doc["programData"]["reparse"] = serde_json::json!(true);
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyInvalid
    );

    let mut doc = valid_document();
    doc["hostMounts"]["remoteCount"] = serde_json::json!(1);
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyInvalid
    );
}

#[test]
fn zeroed_stage2_attestation_is_required() {
    let mut doc = valid_document();
    doc["stage2AttestationSha256"] = serde_json::json!("abcd");
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyInvalid
    );
    let mut doc = valid_document();
    doc["stage2Executed"] = serde_json::json!(true);
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyInvalid
    );
}

#[test]
fn evidence_classes_are_explicit_and_unprovable_claims_stay_unprovable() {
    assert_eq!(claim_class("provider.environment"), ClaimClass::Claimed);
    assert_eq!(claim_class("os.build"), ClaimClass::OsDerived);
    assert_eq!(claim_class("self.env_vars"), ClaimClass::SelfAsserted);
    assert_eq!(claim_class("image.disposable"), ClaimClass::Unprovable);
    assert_eq!(
        claim_class("hostmounts.no_protected_resources_anywhere"),
        ClaimClass::Unprovable
    );
    assert_eq!(claim_class("stage2.privileged"), ClaimClass::NotRun);
}

/// A document an untrusted caller could author: every required field is
/// present and pinned correctly, but the provider/OS values are fabricated.
fn forged_document() -> Value {
    let mut doc = valid_document();
    doc["runId"] = serde_json::json!("999999999");
    doc["providerAttested"]["actor"] = serde_json::json!("forged-actor");
    doc["osDerived"]["buildLabEx"] = serde_json::json!("forged-build-label");
    doc["programData"]["volumeSerial"] = serde_json::json!(999);
    doc["programData"]["fileIndex"] = serde_json::json!(777);
    doc
}

#[test]
fn forged_but_structurally_valid_document_remains_unverified() {
    let result = validate_structure_only(&forged_document());
    assert_eq!(
        result.structure,
        StructuralStatus::StructurallyValid,
        "shape checks cannot detect a forged caller-supplied document"
    );
    assert_eq!(result.provenance, ProvenanceStatus::Unverified);
    assert!(
        promote_to_authenticated(&result).is_err(),
        "no promotion to authenticated evidence is possible"
    );
    assert!(
        isolation_acceptance(&result).is_err(),
        "no isolation acceptance is possible"
    );
    let doc = forged_document();
    assert_eq!(doc["stage2AttestationSha256"], ZEROED_STAGE2_ATTESTATION);
    assert_eq!(doc["stage2Executed"], serde_json::json!(false));
}

// ---------------------------------------------------------------------------
// Nonprivileged real-facts collector.
//
// Run (no privileged provisioning, no workflow change):
//   PR341_EVIDENCE_OUT=<path> cargo test --locked --test \
//     isolation_evidence_contract test_collector_collects_real_nonprivileged_facts -- \
//     --exact --nocapture
//
// Without `PR341_EVIDENCE_OUT` the collector runs read-only and writes nothing.
// The emitted document is CLAIMED data, never an authenticated attestation.
// ---------------------------------------------------------------------------

fn ps_line(command: &str) -> Result<String, String> {
    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", command])
        .output()
        .map_err(|e| format!("powershell unavailable: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "OS probe failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Kernel identity of an OS object (volume serial + file index), via the same
/// Win32 API family used by the reviewed provisioning code.
fn kernel_identity(path: &Path) -> Result<(u32, u64), String> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(once(0u16)).collect();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null_mut(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err("kernel object handle unavailable".into());
    }
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    let queried = unsafe { GetFileInformationByHandle(handle, &mut info) };
    unsafe { CloseHandle(handle) };
    if queried != TRUE {
        return Err("kernel object identity query failed".into());
    }
    Ok((
        info.dwVolumeSerialNumber,
        ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
    ))
}

fn path_is_reparse(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|meta| meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
        .unwrap_or(false)
}

fn env_claim(name: &str) -> Value {
    std::env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .map(Value::String)
        .unwrap_or(Value::Null)
}

/// Collects only nonprivileged facts from the current Windows host. Provider
/// values are copied verbatim from GitHub-set environment variables as CLAIMS.
fn collect_nonprivileged_windows_facts() -> Result<Value, String> {
    let known_folder = ps_line("[Environment]::GetFolderPath('CommonApplicationData')")?;
    if known_folder.is_empty() || !PathBuf::from(&known_folder).is_absolute() {
        return Err("ProgramData Known Folder is not an absolute path".into());
    }
    let program_data = PathBuf::from(&known_folder);
    let (volume_serial, file_index) = kernel_identity(&program_data)
        .map_err(|e| format!("ProgramData kernel identity unavailable: {e}"))?;
    let reparse = path_is_reparse(&program_data);

    let product_type = ps_line("(Get-CimInstance Win32_OperatingSystem).ProductType")?;
    let installation_type = match product_type.as_str() {
        "1" => "Client",
        "2" => "DomainController",
        "3" => "Server",
        other => other,
    };
    let caption = ps_line("(Get-CimInstance Win32_OperatingSystem).Caption")?;
    let build = ps_line("(Get-CimInstance Win32_OperatingSystem).BuildNumber")?;
    let token_sid = ps_line("[Security.Principal.WindowsIdentity]::GetCurrent().User.Value")?;
    let is_admin = ps_line(
        "([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)",
    )?;
    let unc_mounts = ps_line(
        "@(Get-PSDrive -PSProvider FileSystem | Where-Object { $_.DisplayRoot -like '\\\\*' }).Count",
    )?;
    let unc_mounts: u64 = unc_mounts
        .parse()
        .map_err(|e| format!("mount inventory malformed: {e}"))?;
    let registry_absent = ps_line(&format!(
        "Test-Path -LiteralPath '{}'",
        program_data
            .join("PrometheOS/Lite/owner-registry/registry.json")
            .to_string_lossy()
    ))?;
    let registry_absent = registry_absent.eq_ignore_ascii_case("False");
    let collected_at = ps_line("(Get-Date).ToUniversalTime().ToString('o')")?;

    Ok(json!({
        "schemaVersion": SCHEMA,
        "event": env_claim("GITHUB_EVENT_NAME"),
        "repository": env_claim("GITHUB_REPOSITORY"),
        "headSha": env_claim("GITHUB_SHA"),
        "workflowPath": APPROVED_WORKFLOW,
        "workflowSha256": REVIEWED_WORKFLOW_SHA256,
        "workflowRevision": REVIEWED_MAIN_SHA,
        "runId": env_claim("GITHUB_RUN_ID"),
        "runAttempt": std::env::var("GITHUB_RUN_ATTEMPT")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map(Value::from)
            .unwrap_or(Value::Null),
        "providerAttested": {
            "runnerEnvironment": env_claim("RUNNER_ENVIRONMENT"),
            "runnerOs": env_claim("RUNNER_OS"),
            "workflowRef": env_claim("GITHUB_WORKFLOW_REF"),
            "eventRef": env_claim("GITHUB_REF"),
            "actor": env_claim("GITHUB_ACTOR")
        },
        "osDerived": {
            "buildLabEx": format!("{caption} build {build}"),
            "installationType": installation_type.to_owned(),
            "tokenSid": token_sid,
            "tokenIsAdministrator": is_admin.eq_ignore_ascii_case("True")
        },
        "programData": {
            "path": known_folder,
            "reparse": reparse,
            "volumeSerial": volume_serial,
            "fileIndex": file_index,
            "prometheosPathAbsentBeforeRun": registry_absent
        },
        "hostMounts": {
            "enumerated": true,
            "remoteCount": unc_mounts,
            "protectedMountCount": unc_mounts
        },
        "selfAssertedOnly": false,
        "stage2AttestationSha256": ZEROED_STAGE2_ATTESTATION,
        "stage2Executed": false,
        "provenance": {
            "classification": "UNVERIFIED_PROVENANCE",
            "producer": "nonprivileged-windows-facts-collector-v1",
            "hostKind": if std::env::var_os("GITHUB_ACTIONS").is_some() {
                "github-hosted-claimed"
            } else {
                "unknown-local"
            },
            "collectedAtUtc": collected_at,
            "limitation": "provider fields are copied from environment; OS facts are locally observed; no trusted channel authenticates either"
        }
    }))
}

#[test]
fn test_collector_collects_real_nonprivileged_facts() {
    let doc = collect_nonprivileged_windows_facts().expect("collector must run read-only");
    assert_eq!(doc["provenance"]["classification"], "UNVERIFIED_PROVENANCE");
    assert_eq!(doc["stage2AttestationSha256"], ZEROED_STAGE2_ATTESTATION);
    assert_eq!(doc["stage2Executed"], serde_json::json!(false));
    assert!(
        doc["osDerived"]["tokenSid"]
            .as_str()
            .is_some_and(|sid| sid.starts_with("S-1-")),
        "real Windows token SID must be collected"
    );
    assert!(
        doc["programData"]["volumeSerial"]
            .as_u64()
            .is_some_and(|v| v > 0),
        "real kernel volume identity must be collected"
    );
    assert_eq!(doc["hostMounts"]["enumerated"], serde_json::json!(true));

    let result = validate_structure_only(&doc);
    let on_runner = std::env::var_os("GITHUB_ACTIONS").is_some();
    if on_runner {
        assert_eq!(result.structure, StructuralStatus::StructurallyValid);
    } else {
        assert_eq!(
            result.structure,
            StructuralStatus::StructurallyInvalid,
            "off-runner collection must fail closed on missing provider claims"
        );
        assert!(!result.issues.is_empty());
    }
    assert_eq!(result.provenance, ProvenanceStatus::Unverified);
    assert!(promote_to_authenticated(&result).is_err());
    assert!(isolation_acceptance(&result).is_err());

    if let Some(path) = std::env::var_os("PR341_EVIDENCE_OUT") {
        let rendered = serde_json::to_string_pretty(&doc).expect("serialize evidence");
        fs::write(PathBuf::from(path), rendered).expect("write evidence artifact");
    }
}

#[test]
fn workflow_ref_is_bound_to_exact_reviewed_identity() {
    assert_eq!(
        validate_structure_only(&valid_document()).structure,
        StructuralStatus::StructurallyValid
    );

    let mutations = [
        "refs/heads/attacker", // event ref only
        REVIEWED_PR341_SHA,    // checkout commit
        "prometheosai/prometheos-lite/.github/workflows/other.yml@refs/heads/main",
        "evil/prometheos-lite/.github/workflows/pr341-windows-acceptance.yml@refs/heads/main",
        "prometheosai/prometheos-lite/.github/workflows/pr341-windows-acceptance.yml@refs/heads/attacker",
        "prometheosai/prometheos-lite/.github/workflows/pr341-windows-acceptance.yml", // missing @ref
    ];
    for bad_ref in mutations {
        let mut doc = valid_document();
        doc["providerAttested"]["workflowRef"] = serde_json::json!(bad_ref);
        assert_eq!(
            validate_structure_only(&doc).structure,
            StructuralStatus::StructurallyInvalid,
            "workflowRef {bad_ref} must be rejected"
        );
    }

    let mut doc = valid_document();
    doc["providerAttested"]["eventRef"] = serde_json::json!("refs/heads/attacker");
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyInvalid,
        "the dispatch event ref is bound separately from the workflow ref"
    );
}
