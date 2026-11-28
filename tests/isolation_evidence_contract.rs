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
//!
//! Revision semantics (H1/H2 repair): `headSha`/`candidateHead` are the exact
//! checked-out PR #341 source; `workflowSha` is the observed `GITHUB_SHA` of
//! the dispatch and is labelled `OBSERVED_CLAIM_UNVERIFIED`; `workflowRevision`
//! is the historical revision where the reviewed workflow was introduced;
//! `workflowIntroductionDigest` is that ORIGINAL workflow's content digest and
//! is labelled `HISTORICAL_INTRODUCTION_ONLY`. It is NOT a verification of the
//! workflow that may execute this collector: the active workflow identity is
//! bound separately by the deterministic, Principal-reviewed repository policy
//! digest (`activeWorkflowBinding`), never asserted by this unprivileged
//! collector. A moving `main` therefore does not invalidate the historical pin,
//! and this collector never embeds the digest of a new workflow (no circular
//! collector/workflow digest dependency). The merged workflow cannot invoke
//! this collector because its checkout predates it; the historical assertion
//! is proven from an immutable fixture, not from the current checkout. Any
//! integration requires a separate DRAFT workflow PR with exact reviewed source
//! identity and a deliberate Principal digest re-pin (see
//! `validate_integration_plan`). No privileged acceptance is enabled here.

#[cfg(windows)]
use std::fs;
#[cfg(windows)]
use std::iter::once;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;
#[cfg(windows)]
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::process::Command;
#[cfg(windows)]
use std::ptr;

use serde_json::Value;
#[cfg(windows)]
use serde_json::json;
#[cfg(windows)]
use winapi::shared::minwindef::TRUE;
#[cfg(windows)]
use winapi::um::fileapi::{
    BY_HANDLE_FILE_INFORMATION, CreateFileW, GetFileInformationByHandle, OPEN_EXISTING,
};
#[cfg(windows)]
use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
#[cfg(windows)]
use winapi::um::winbase::FILE_FLAG_BACKUP_SEMANTICS;
#[cfg(windows)]
use winapi::um::winnt::{
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    GENERIC_READ,
};

const SCHEMA: &str = "prometheos.isolation-evidence.v1";
const REPOSITORY: &str = "prometheosai/prometheos-lite";
const APPROVED_WORKFLOW: &str = ".github/workflows/pr341-windows-acceptance.yml";
/// Digest of the workflow CONTENT as introduced at the historical revision
/// below (git blob `3ebbab5681def4b24e867df50a6f422d944b86be`). It is
/// immutable historical provenance only — NOT verification of any workflow
/// that executes this collector.
const WORKFLOW_INTRODUCTION_SHA256: &str =
    "b45b5ed1c912cef4d14251563fe2366f76c1d28287ce2e2291ef84f55d01b84c";
/// Marks the introduction digest as historical metadata, not an assertion
/// about the currently executing workflow.
const WORKFLOW_DIGEST_PROVENANCE: &str = "HISTORICAL_INTRODUCTION_ONLY";
/// The active (executing) workflow is bound by the separately reviewed,
/// deterministic repository policy digest pin. This collector neither knows
/// nor asserts that digest; doing so would create a circular dependency
/// between the collector commit and the workflow commit.
const ACTIVE_WORKFLOW_BINDING: &str = "REVIEWED_POLICY_DIGEST_NOT_ASSERTED_HERE";
/// The audited revision at which the reviewed workflow CONTENT was introduced.
/// It is a historical binding, NOT the live `GITHUB_SHA` of a dispatch. `main`
/// may advance without changing the workflow bytes; the content digest below
/// remains the authoritative pin.
const REVIEWED_MAIN_SHA: &str = "981c39955fb82b181202b932bfeaf2119b22e669";
/// Synthetic, full-length event SHA used to prove that `main` advancing does
/// not invalidate an unchanged, pinned workflow file.
const ADVANCED_MAIN_SHA: &str = "0123456789abcdef0123456789abcdef01234567";
/// Marks `workflowSha` as an observed claim. Run-to-workflow-revision binding
/// must be established externally; this contract never authenticates it.
const WORKFLOW_SHA_PROVENANCE: &str = "OBSERVED_CLAIM_UNVERIFIED";
const REVIEWED_PR341_SHA: &str = "37a7ae33ca21fc25ad0f70f20022cd4a95634bce";
const COLLECTOR_TEST_NAME: &str = "isolation_evidence_contract";
/// GitHub `GITHUB_WORKFLOW_REF` format is `{owner}/{repo}/{workflow_path}@{ref}`.
/// This is NOT `GITHUB_REF` (event ref) and NOT the checked-out commit SHA.
const EXPECTED_WORKFLOW_REF: &str =
    "prometheosai/prometheos-lite/.github/workflows/pr341-windows-acceptance.yml@refs/heads/main";
const EXPECTED_EVENT_REF: &str = "refs/heads/main";
const PARTIAL_UNC_COVERAGE: &str = "PARTIAL_UNC_MAPPINGS_ONLY";
const ZEROED_STAGE2_ATTESTATION: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

/// Immutable historical fixture distilled from the reviewed workflow as
/// introduced at main `981c3995` (git blob `3ebbab56…`, LF-normalized
/// SHA-256 `b45b5ed1…`, both independently recomputed from the object store).
/// The pinned PR #341 checkout PREDATES that workflow file, so the historical
/// assertion must never read the current checkout's copy: doing so would both
/// fail in the pinned checkout and use mutable content as proof of history.
const HISTORICAL_WORKFLOW_EVIDENCE: &str = r#"{
  "sourceRevision": "981c39955fb82b181202b932bfeaf2119b22e669",
  "workflowBlob": "3ebbab5681def4b24e867df50a6f422d944b86be",
  "contentSha256": "b45b5ed1c912cef4d14251563fe2366f76c1d28287ce2e2291ef84f55d01b84c",
  "pinnedSource": "37a7ae33ca21fc25ad0f70f20022cd4a95634bce",
  "focusedTestTarget": "work::owner_registry",
  "invokesCollector": false
}"#;

fn historical_workflow_evidence() -> Value {
    serde_json::from_str(HISTORICAL_WORKFLOW_EVIDENCE)
        .expect("embedded immutable historical fixture must parse")
}

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
    // headSha is the CHECKED-OUT SOURCE commit (git rev-parse HEAD), not GITHUB_SHA.
    if req_str(doc, &["headSha"], &mut errors) != Some(REVIEWED_PR341_SHA) {
        errors.push("checked-out source commit is not the pinned PR 341 head".into());
    }
    if req_str(doc, &["candidateHead"], &mut errors) != Some(REVIEWED_PR341_SHA) {
        errors.push("candidate PR head binding mismatch".into());
    }
    // H1: `workflowSha` is the observed GITHUB_SHA of the dispatch, i.e. the
    // branch revision that hosted the run. It is NOT the pinned PR #341 source
    // and NOT the historical revision that introduced the workflow. It is
    // accepted as a well-formed claim, never compared to REVIEWED_MAIN_SHA.
    let workflow_sha = req_str(doc, &["workflowSha"], &mut errors);
    if workflow_sha
        .is_some_and(|sha| sha.len() != 40 || !sha.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        errors.push("workflowSha must be a full GitHub commit SHA claim".into());
    }
    if let (Some(workflow_sha), Some(head_sha)) =
        (workflow_sha, doc.get("headSha").and_then(Value::as_str))
        && workflow_sha == head_sha
    {
        errors.push("workflowSha must not be reused as the checked-out source SHA".into());
    }
    if req_str(doc, &["workflowShaProvenance"], &mut errors) != Some(WORKFLOW_SHA_PROVENANCE) {
        errors.push("workflowSha must be labelled as an unverified observed claim".into());
    }
    req_bool(doc, &["checkoutVerified"], true, &mut errors);
    if req_str(doc, &["workflowPath"], &mut errors) != Some(APPROVED_WORKFLOW) {
        errors.push("workflow path binding mismatch".into());
    }
    // Historical introduction digest: immutable provenance metadata, NOT a
    // verification of the workflow that may actually execute this collector.
    if req_str(doc, &["workflowIntroductionDigest"], &mut errors)
        != Some(WORKFLOW_INTRODUCTION_SHA256)
    {
        errors.push("historical workflow introduction digest mismatch".into());
    }
    if req_str(doc, &["workflowDigestProvenance"], &mut errors) != Some(WORKFLOW_DIGEST_PROVENANCE)
    {
        errors.push("introduction digest must be labelled historical only".into());
    }
    // The active workflow identity is bound by the separately reviewed,
    // deterministic repository policy digest. It is never asserted here.
    if req_str(doc, &["activeWorkflowBinding"], &mut errors) != Some(ACTIVE_WORKFLOW_BINDING) {
        errors.push("active workflow must be bound by a separately reviewed policy digest".into());
    }
    if field(doc, &["activeWorkflowDigest"]).is_some() {
        errors.push("collector must not assert the executing workflow digest".into());
    }
    // Historical introduction revision, not the live event SHA.
    if req_str(doc, &["workflowRevision"], &mut errors) != Some(REVIEWED_MAIN_SHA) {
        errors.push("workflow introduction revision binding mismatch".into());
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
    // GitHub sets `RUNNER_OS=Windows` (canonical casing) on windows-hosted
    // runners. Compare case-insensitively while preserving the raw claim.
    if req_str(doc, &["providerAttested", "runnerOs"], &mut errors)
        .is_none_or(|os| !os.eq_ignore_ascii_case("windows"))
    {
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

    // G3: UNC mappings are a PARTIAL observation. Complete mount inventory is
    // NOT available, so protectedMountCount must stay explicitly unknown.
    if req_str(doc, &["hostMounts", "coverage"], &mut errors) != Some(PARTIAL_UNC_COVERAGE) {
        errors.push("mount inventory coverage must be declared partial".into());
    }
    req_bool(
        doc,
        &["hostMounts", "inventoryComplete"],
        false,
        &mut errors,
    );
    req_u64(doc, &["hostMounts", "uncMappingsObserved"], 0, &mut errors);
    if field(doc, &["hostMounts", "protectedMountCount"]).is_some_and(|v| !v.is_null()) {
        errors.push(
            "protectedMountCount is unmeasured and must be null, not a fabricated number".into(),
        );
    }

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
        "candidateHead": REVIEWED_PR341_SHA,
        "workflowSha": ADVANCED_MAIN_SHA,
        "workflowShaProvenance": WORKFLOW_SHA_PROVENANCE,
        "checkoutVerified": true,
        "workflowPath": APPROVED_WORKFLOW,
        "workflowIntroductionDigest": WORKFLOW_INTRODUCTION_SHA256,
        "workflowDigestProvenance": WORKFLOW_DIGEST_PROVENANCE,
        "activeWorkflowBinding": ACTIVE_WORKFLOW_BINDING,
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
            "coverage": PARTIAL_UNC_COVERAGE,
            "inventoryComplete": false,
            "uncMappingsObserved": 0,
            "protectedMountCount": null
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

/// Deterministic regression for the hosted-runner casing defect: GitHub's
/// `RUNNER_OS` is `Windows`, while the fixture uses lowercase `windows`.
#[test]
fn github_hosted_runner_os_casing_is_accepted() {
    let mut doc = valid_document();
    doc["providerAttested"]["runnerOs"] = serde_json::json!("Windows");
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyValid,
        "canonical GitHub RUNNER_OS casing must be accepted"
    );

    let mut doc = valid_document();
    doc["providerAttested"]["runnerOs"] = serde_json::json!("WINDOWS");
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyValid
    );

    for bad in ["linux", "macOS", "windows-2022", ""] {
        let mut doc = valid_document();
        doc["providerAttested"]["runnerOs"] = serde_json::json!(bad);
        assert_eq!(
            validate_structure_only(&doc).structure,
            StructuralStatus::StructurallyInvalid,
            "runner OS {bad:?} must be refused"
        );
    }
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
    doc["workflowIntroductionDigest"] = serde_json::json!("ffff");
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyInvalid
    );

    let mut doc = valid_document();
    doc["workflowDigestProvenance"] = serde_json::json!("CURRENT_WORKFLOW_VERIFIED");
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyInvalid,
        "the historical digest must never be presented as current verification"
    );

    let mut doc = valid_document();
    doc["activeWorkflowBinding"] = serde_json::json!("SELF_ASSERTED");
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyInvalid
    );

    let mut doc = valid_document();
    // Synthetic value only: the collector must never embed any real workflow
    // digest, so this fixture is deliberately unrelated to every known digest.
    doc["activeWorkflowDigest"] =
        serde_json::json!("deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef");
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyInvalid,
        "the collector must not embed the executing workflow digest"
    );

    let mut doc = valid_document();
    doc["workflowRevision"] = serde_json::json!("deadbeef");
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyInvalid
    );
}

#[test]
fn workflow_event_sha_is_distinct_from_checked_out_source() {
    assert_eq!(
        validate_structure_only(&valid_document()).structure,
        StructuralStatus::StructurallyValid
    );
    let mut doc = valid_document();
    doc["workflowSha"] = serde_json::json!(REVIEWED_PR341_SHA);
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyInvalid,
        "event SHA is the main revision, not the checked-out PR source"
    );
    let mut doc = valid_document();
    doc["headSha"] = serde_json::json!(REVIEWED_MAIN_SHA);
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyInvalid,
        "checked-out source must be the pinned PR head"
    );
    let mut doc = valid_document();
    doc["checkoutVerified"] = serde_json::json!(false);
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyInvalid
    );
    let mut doc = valid_document();
    doc.as_object_mut().expect("object").remove("candidateHead");
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyInvalid
    );
}

#[test]
fn missing_git_checkout_identity_fails_closed() {
    let mut doc = valid_document();
    doc["headSha"] = Value::Null;
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyInvalid
    );
    let mut doc = valid_document();
    doc["checkoutVerified"] = serde_json::json!(false);
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
fn reparse_programdata_is_rejected() {
    let mut doc = valid_document();
    doc["programData"]["reparse"] = serde_json::json!(true);
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyInvalid
    );
}

#[test]
fn incomplete_or_fabricated_mount_inventory_is_rejected() {
    let mut doc = valid_document();
    doc["hostMounts"]["inventoryComplete"] = serde_json::json!(true);
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyInvalid
    );

    let mut doc = valid_document();
    doc["hostMounts"]["protectedMountCount"] = serde_json::json!(0);
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyInvalid,
        "an unmeasured count must not be fabricated as zero"
    );

    let mut doc = valid_document();
    doc["hostMounts"]
        .as_object_mut()
        .expect("object")
        .remove("coverage");
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyInvalid
    );

    let mut doc = valid_document();
    doc.as_object_mut().expect("object").remove("hostMounts");
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyInvalid,
        "missing inventory must fail closed"
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

#[cfg(windows)]
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
#[cfg(windows)]
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

#[cfg(windows)]
fn path_is_reparse(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|meta| meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
        .unwrap_or(false)
}

#[cfg(windows)]
fn env_claim(name: &str) -> Value {
    std::env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .map(Value::String)
        .unwrap_or(Value::Null)
}

/// Collects only nonprivileged facts from the current Windows host. Provider
/// values are copied verbatim from GitHub-set environment variables as CLAIMS.
#[cfg(windows)]
fn git_head_commit() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(sha)
    } else {
        None
    }
}

#[cfg(windows)]
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
    let checkout = git_head_commit();
    let checkout_verified = checkout.as_deref() == Some(REVIEWED_PR341_SHA);

    Ok(json!({
        "schemaVersion": SCHEMA,
        "event": env_claim("GITHUB_EVENT_NAME"),
        "repository": env_claim("GITHUB_REPOSITORY"),
        "headSha": checkout.map(Value::String).unwrap_or(Value::Null),
        "candidateHead": REVIEWED_PR341_SHA,
        "workflowSha": env_claim("GITHUB_SHA"),
        "workflowShaProvenance": WORKFLOW_SHA_PROVENANCE,
        "checkoutVerified": checkout_verified,
        "workflowPath": APPROVED_WORKFLOW,
        "workflowIntroductionDigest": WORKFLOW_INTRODUCTION_SHA256,
        "workflowDigestProvenance": WORKFLOW_DIGEST_PROVENANCE,
        "activeWorkflowBinding": ACTIVE_WORKFLOW_BINDING,
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
            "coverage": PARTIAL_UNC_COVERAGE,
            "inventoryComplete": false,
            "uncMappingsObserved": unc_mounts,
            "protectedMountCount": Value::Null
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

#[cfg(windows)]
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
    assert_eq!(doc["candidateHead"], REVIEWED_PR341_SHA);
    assert_eq!(doc["hostMounts"]["coverage"], PARTIAL_UNC_COVERAGE);
    assert_eq!(
        doc["hostMounts"]["inventoryComplete"],
        serde_json::json!(false)
    );
    assert!(
        doc["hostMounts"]["protectedMountCount"].is_null(),
        "unmeasured protected mounts must remain unknown"
    );
    if let Some(sha) = doc["headSha"].as_str() {
        assert_eq!(sha.len(), 40, "checkout SHA must be a full git commit id");
    }

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
fn main_advances_without_workflow_content_change_remains_structurally_valid() {
    // H1: a future workflow_dispatch runs on whatever `main` revision is
    // current. The workflow BYTES and their content digest are unchanged, so
    // a different GITHUB_SHA must not invalidate the document.
    let mut doc = valid_document();
    doc["workflowSha"] = serde_json::json!(ADVANCED_MAIN_SHA);
    doc["workflowRevision"] = serde_json::json!(REVIEWED_MAIN_SHA);
    doc["workflowIntroductionDigest"] = serde_json::json!(WORKFLOW_INTRODUCTION_SHA256);
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyValid,
        "a moving event SHA must not break an unchanged historical pin"
    );
    // The historical content pin is NOT loosened by that allowance.
    doc["workflowIntroductionDigest"] = serde_json::json!("ffff");
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyInvalid
    );
    // Nor is a malformed / reused event SHA.
    let mut doc = valid_document();
    doc["workflowSha"] = serde_json::json!("not-a-sha");
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyInvalid
    );
    let mut doc = valid_document();
    doc["workflowSha"] = serde_json::json!(REVIEWED_PR341_SHA);
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyInvalid,
        "event SHA must not be the checked-out source SHA"
    );
}

#[test]
fn workflow_sha_provenance_label_is_required() {
    let mut doc = valid_document();
    doc.as_object_mut()
        .expect("object")
        .remove("workflowShaProvenance");
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyInvalid
    );
    let mut doc = valid_document();
    doc["workflowShaProvenance"] = serde_json::json!("PROVIDER_ATTESTED");
    assert_eq!(
        validate_structure_only(&doc).structure,
        StructuralStatus::StructurallyInvalid,
        "a structural contract cannot authenticate the event SHA"
    );
}

/// H2 integration strategy, enforced as a document check. The current merged
/// workflow pins PR #341 source, which predates this collector, so it cannot
/// invoke it. Before any workflow change: acquire the collector from an exact
/// reviewed source SHA, keep the workflow content pin, use a separate DRAFT
/// workflow PR with a deliberate re-pin, and never enable privileged
/// acceptance.
fn validate_integration_plan(plan: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    if plan
        .get("workflowIntroductionDigest")
        .and_then(Value::as_str)
        != Some(WORKFLOW_INTRODUCTION_SHA256)
    {
        errors.push("historical workflow introduction digest must stay pinned".into());
    }
    if plan
        .get("workflowIntroductionRevision")
        .and_then(Value::as_str)
        != Some(REVIEWED_MAIN_SHA)
    {
        errors.push("historical workflow introduction revision must stay pinned".into());
    }
    // The active workflow is bound by the reviewed policy digest, never by a
    // digest asserted in this collector or self-approved here.
    if plan.get("activeWorkflowBinding").and_then(Value::as_str) != Some(ACTIVE_WORKFLOW_BINDING) {
        errors.push("active workflow binding must be the reviewed policy digest".into());
    }
    if plan.get("selfApprovedDigest").and_then(Value::as_bool) != Some(false) {
        errors.push("the executing workflow digest must not be self-approved".into());
    }
    if plan
        .get("currentWorkflowCanInvokeCollector")
        .and_then(Value::as_bool)
        != Some(false)
    {
        errors.push("current pinned workflow cannot invoke the collector".into());
    }
    if plan
        .get("collectorSourceSha")
        .and_then(Value::as_str)
        .is_none_or(|sha| {
            sha.len() != 40
                || !sha.bytes().all(|byte| byte.is_ascii_hexdigit())
                || sha == REVIEWED_PR341_SHA
        })
    {
        errors.push("collector must be acquired from an exact reviewed SHA other than the pinned PR 341 head".into());
    }
    if plan.get("separateDraftWorkflowPr").and_then(Value::as_bool) != Some(true) {
        errors.push("workflow integration requires a separate DRAFT PR".into());
    }
    if plan.get("deliberateDigestRepin").and_then(Value::as_bool) != Some(true) {
        errors.push("workflow content change requires a deliberate digest re-pin".into());
    }
    if plan.get("privilegedAcceptance").and_then(Value::as_bool) != Some(false) {
        errors.push("no privileged acceptance may be enabled".into());
    }
    errors
}

fn valid_integration_plan() -> Value {
    serde_json::json!({
        "workflowIntroductionDigest": WORKFLOW_INTRODUCTION_SHA256,
        "workflowDigestProvenance": WORKFLOW_DIGEST_PROVENANCE,
        "activeWorkflowBinding": ACTIVE_WORKFLOW_BINDING,
        "workflowIntroductionRevision": REVIEWED_MAIN_SHA,
        "currentPinnedSource": REVIEWED_PR341_SHA,
        "collectorTest": COLLECTOR_TEST_NAME,
        "currentWorkflowCanInvokeCollector": false,
        "collectorSourceSha": ADVANCED_MAIN_SHA,
        "separateDraftWorkflowPr": true,
        "deliberateDigestRepin": true,
        "selfApprovedDigest": false,
        "privilegedAcceptance": false
    })
}

#[test]
fn current_workflow_cannot_invoke_collector_negative_control() {
    // Historical assertion is proven from the immutable fixture captured at
    // the reviewed introduction revision. It deliberately does NOT read the
    // current checkout's workflow: PR #341 predates that file, and mutable
    // current content is not evidence of historical behavior.
    let historical = historical_workflow_evidence();
    assert_eq!(historical["sourceRevision"], REVIEWED_MAIN_SHA);
    assert_eq!(
        historical["workflowBlob"],
        "3ebbab5681def4b24e867df50a6f422d944b86be"
    );
    assert_eq!(historical["contentSha256"], WORKFLOW_INTRODUCTION_SHA256);
    assert_eq!(historical["pinnedSource"], REVIEWED_PR341_SHA);
    assert_eq!(historical["focusedTestTarget"], "work::owner_registry");
    assert_eq!(historical["invokesCollector"], false);
    assert!(
        !HISTORICAL_WORKFLOW_EVIDENCE.contains(COLLECTOR_TEST_NAME),
        "the historical reviewed workflow must not claim to invoke the collector"
    );
    assert!(
        validate_integration_plan(&valid_integration_plan()).is_empty(),
        "the bounded integration strategy must be internally consistent"
    );
}

#[test]
fn collector_integration_plan_refuses_unsafe_shortcuts() {
    let mut plan = valid_integration_plan();
    plan["currentWorkflowCanInvokeCollector"] = serde_json::json!(true);
    assert!(
        !validate_integration_plan(&plan).is_empty(),
        "claiming the current workflow can collect is refused"
    );

    let mut plan = valid_integration_plan();
    plan["collectorSourceSha"] = serde_json::json!(REVIEWED_PR341_SHA);
    assert!(
        !validate_integration_plan(&plan).is_empty(),
        "the pinned PR 341 checkout cannot be the collector source"
    );

    let mut plan = valid_integration_plan();
    plan["separateDraftWorkflowPr"] = serde_json::json!(false);
    plan["deliberateDigestRepin"] = serde_json::json!(false);
    assert!(!validate_integration_plan(&plan).is_empty());

    let mut plan = valid_integration_plan();
    plan["workflowIntroductionDigest"] = serde_json::json!("ffff");
    assert!(!validate_integration_plan(&plan).is_empty());

    let mut plan = valid_integration_plan();
    plan["selfApprovedDigest"] = serde_json::json!(true);
    assert!(
        !validate_integration_plan(&plan).is_empty(),
        "self-approving the new workflow digest is refused"
    );

    let mut plan = valid_integration_plan();
    plan["activeWorkflowBinding"] = serde_json::json!("COLLECTOR_ASSERTED");
    assert!(!validate_integration_plan(&plan).is_empty());

    let mut plan = valid_integration_plan();
    plan["privilegedAcceptance"] = serde_json::json!(true);
    assert!(!validate_integration_plan(&plan).is_empty());
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
