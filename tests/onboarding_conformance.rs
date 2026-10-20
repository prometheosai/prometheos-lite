//! E6/I05 (#134) — repository onboarding and actionable diagnostics.
//!
//! Strict-TDD conformance: a supported repository receives a reviewable
//! onboarding manifest (a PROPOSAL that enables no authority); every
//! detection carries confidence and evidence; unsupported or conflicting
//! setups stop with typed, actionable diagnostics; user edits override
//! detection explicitly and are recorded; and onboarding never modifies
//! project files — the survey is read-only and the apply step requires
//! explicit approval before writing anything at all.

use std::path::Path;
use std::process::Command;

use prometheos_lite::workflow::onboarding::{
    ONBOARDING_SCHEMA_VERSION, OnboardingManifestV1, OnboardingSurveyV1, apply_user_manifest,
    manifest_from_survey, survey_repository,
};
use prometheos_lite::workflow::soma::Diagnostic;

fn git(root: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("git available in test environment");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Snapshot every file's content hash under the repo (recording writes).
fn snapshot(root: &Path) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, u64)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                walk(&path, base, out);
            } else {
                let bytes = std::fs::read(&path).unwrap();
                let mut h: u64 = 1469598103934665603;
                for b in bytes {
                    h ^= b as u64;
                    h = h.wrapping_mul(1099511628211);
                }
                out.push((
                    path.strip_prefix(base)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    h,
                ));
            }
        }
    }
    walk(root, root, &mut out);
    out.sort();
    out
}

fn init_git(root: &Path) {
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "ci@example.com"]);
    git(root, &["config", "user.name", "ci"]);
    git(root, &["config", "core.autocrlf", "false"]);
}

fn commit_all(root: &Path) {
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "initial"]);
}

const CARGO_MANIFEST: &str =
    "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n";

fn cargo_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_git(root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("Cargo.toml"), CARGO_MANIFEST).unwrap();
    std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    commit_all(root);
    dir
}

fn npm_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_git(root);
    std::fs::write(
        root.join("package.json"),
        r#"{ "name": "fixture", "scripts": { "test": "jest" } }"#,
    )
    .unwrap();
    std::fs::write(root.join("index.js"), "console.log(1);\n").unwrap();
    commit_all(root);
    dir
}

fn multi_system_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_git(root);
    std::fs::write(root.join("Cargo.toml"), CARGO_MANIFEST).unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{ "name": "fixture", "scripts": { "test": "jest" } }"#,
    )
    .unwrap();
    commit_all(root);
    dir
}

fn empty_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_git(root);
    std::fs::write(root.join("README.md"), "# fixture\n").unwrap();
    commit_all(root);
    dir
}

fn assert_actionable(diags: &[Diagnostic], code: &str) {
    assert!(
        diags.iter().any(|d| d.code == code),
        "expected {code}, got {diags:?}"
    );
    let hit = diags.iter().find(|d| d.code == code).unwrap();
    assert!(
        !hit.message.trim().is_empty(),
        "{code} must carry an explanatory message"
    );
    let remediation = hit
        .remediation
        .as_ref()
        .expect("actionable remediation attached");
    assert!(
        remediation.action.is_some() && !remediation.summary.trim().is_empty(),
        "{code} must carry a remediation action + summary"
    );
}

// ---------------------------------------------------------------------------
// Acceptance 1 + 2: reviewable manifest with confidence and evidence.
// ---------------------------------------------------------------------------

#[test]
fn supported_cargo_repository_receives_a_reviewable_manifest() {
    let dir = cargo_repo();
    let survey = survey_repository(dir.path()).expect("supported repository surveys cleanly");
    let manifest = manifest_from_survey(&survey);

    assert_eq!(manifest.schema_version, ONBOARDING_SCHEMA_VERSION);
    assert!(!manifest.approved, "a proposal is never pre-approved");
    assert!(manifest.overrides.is_empty());
    assert_eq!(manifest.survey.schema_version, ONBOARDING_SCHEMA_VERSION);
    assert!(!manifest.survey.revision.is_empty());
    assert!(!manifest.survey.dirty);
    assert_eq!(manifest.survey.rows.len(), survey.rows.len());

    let row = |field: &str| {
        survey
            .rows
            .iter()
            .find(|r| r.field == field)
            .unwrap_or_else(|| panic!("{field} row present"))
    };
    assert_eq!(row("package_systems").value, serde_json::json!(["cargo"]));
    assert_eq!(row("package_systems").confidence, "high");
    assert_eq!(row("package_systems").evidence, vec!["Cargo.toml"]);

    let commands = row("test_commands");
    assert_eq!(
        commands.value,
        serde_json::json!([{"system": "cargo", "command": ["cargo", "test"]}])
    );
    assert_eq!(commands.confidence, "medium");
    assert_eq!(commands.evidence, vec!["Cargo.toml"]);

    let protected = row("protected_paths");
    assert!(
        protected
            .value
            .as_array()
            .expect("paths array")
            .iter()
            .any(|p| p == ".git")
    );
    assert!(
        protected
            .value
            .as_array()
            .expect("paths array")
            .iter()
            .any(|p| p == ".prometheos-lite")
    );

    assert_eq!(row("languages").confidence, "high");
    assert!(
        row("languages")
            .value
            .as_array()
            .expect("languages array")
            .iter()
            .any(|l| l == "rust")
    );

    // The proposal suggests configuration but grants no authority: there
    // are no scopes, capabilities, or tools anywhere in the proposal.
    let proposal = serde_json::to_value(&manifest).unwrap();
    let text = proposal.to_string();
    assert!(!text.contains("readable:"), "no scopes in a proposal");
    assert!(!text.contains("writable:"), "no scopes in a proposal");
    assert!(!text.contains("authority"), "no authority in a proposal");
}

#[test]
fn detection_confidence_and_evidence_are_shown_for_every_row() {
    let dir = npm_repo();
    let survey = survey_repository(dir.path()).expect("surveys cleanly");
    assert!(!survey.rows.is_empty());
    for row in &survey.rows {
        assert!(
            row.confidence == "high" || row.confidence == "medium",
            "{}: confidence must be explicit, got {:?}",
            row.field,
            row.confidence
        );
        assert!(
            !row.evidence.is_empty(),
            "{}: evidence must be shown",
            row.field
        );
    }
    let commands = survey
        .rows
        .iter()
        .find(|r| r.field == "test_commands")
        .expect("test_commands row");
    assert_eq!(
        commands.value,
        serde_json::json!([{"system": "npm", "command": ["npm", "test"]}])
    );
    assert_eq!(commands.evidence, vec!["package.json scripts.test"]);
}

#[test]
fn multi_system_repository_records_both_without_refusing() {
    let dir = multi_system_repo();
    let survey = survey_repository(dir.path()).expect("multi-language repositories are supported");
    let systems = survey
        .rows
        .iter()
        .find(|r| r.field == "package_systems")
        .expect("package systems row");
    assert_eq!(systems.value, serde_json::json!(["cargo", "npm"]));
    let commands = survey
        .rows
        .iter()
        .find(|r| r.field == "test_commands")
        .expect("test commands row");
    assert_eq!(commands.value.as_array().expect("commands").len(), 2);
}

// ---------------------------------------------------------------------------
// Acceptance 3: unsupported or conflicting setups stop with actionable
// diagnostics.
// ---------------------------------------------------------------------------

#[test]
fn empty_repository_stops_with_actionable_diagnostics() {
    let dir = empty_repo();
    let diags = survey_repository(dir.path()).expect_err("no supported setup must refuse");
    assert_actionable(&diags, "SOMA-ONB-0001");
}

#[test]
fn non_git_directory_stops_with_actionable_diagnostics() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), CARGO_MANIFEST).unwrap();
    let diags = survey_repository(dir.path()).expect_err("a non-git directory must refuse");
    assert_actionable(&diags, "SOMA-ONB-0002");
}

#[test]
fn unparseable_package_manifest_stops_with_actionable_diagnostics() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_git(root);
    std::fs::write(root.join("Cargo.toml"), "not [ a toml at all").unwrap();
    commit_all(root);
    let diags = survey_repository(root).expect_err("a broken manifest must refuse");
    assert_actionable(&diags, "SOMA-ONB-0003");
}

// ---------------------------------------------------------------------------
// Acceptance 4 + 5: explicit overrides; never modify project files.
// ---------------------------------------------------------------------------

#[test]
fn user_edits_override_detection_explicitly() {
    let dir = cargo_repo();
    let survey = survey_repository(dir.path()).unwrap();
    let proposal = manifest_from_survey(&survey);

    // The user edits the proposed test commands in the reviewable file.
    let mut user: OnboardingManifestV1 =
        serde_json::from_str(&serde_json::to_string(&proposal).unwrap()).unwrap();
    user.proposed.test_commands = vec![prometheos_lite::workflow::onboarding::TestCommandV1 {
        system: "cargo".into(),
        command: vec!["cargo".into(), "nextest".into(), "run".into()],
    }];
    user.approved = true;

    let fresh = survey_repository(dir.path()).unwrap();
    let applied =
        apply_user_manifest(&proposal, &user, &fresh, true).expect("explicit user edits apply");
    assert!(applied.approved);
    assert_eq!(applied.overrides.len(), 1, "the override is recorded");
    let over = &applied.overrides[0];
    assert_eq!(over.field, "test_commands");
    assert_eq!(
        over.user,
        serde_json::json!([{"system": "cargo", "command": ["cargo", "nextest", "run"]}])
    );
    assert_eq!(
        over.detected,
        serde_json::json!([{"system": "cargo", "command": ["cargo", "test"]}])
    );
    // The user's value wins in the applied config.
    assert_eq!(
        applied.proposed.test_commands[0].command,
        vec![
            "cargo".to_string(),
            "nextest".to_string(),
            "run".to_string()
        ]
    );
}

#[test]
fn survey_block_edits_are_refused_as_detection_owned() {
    let dir = cargo_repo();
    let survey = survey_repository(dir.path()).unwrap();
    let proposal = manifest_from_survey(&survey);
    let mut user = proposal.clone();
    user.approved = true;
    // Tampering with detection evidence: flip the recorded dirty flag.
    user.survey.dirty = true;

    let fresh = survey_repository(dir.path()).unwrap();
    let diags =
        apply_user_manifest(&proposal, &user, &fresh, true).expect_err("survey edits refuse");
    assert_actionable(&diags, "SOMA-ONB-0005");
}

#[test]
fn repository_drift_refuses_apply_until_regenerated() {
    let dir = cargo_repo();
    let survey = survey_repository(dir.path()).unwrap();
    let proposal = manifest_from_survey(&survey);
    let mut user = proposal.clone();
    user.approved = true;

    // The repository moves after the proposal was generated.
    std::fs::write(dir.path().join("src/extra.rs"), "pub fn extra() {}\n").unwrap();
    commit_all(dir.path());

    let fresh = survey_repository(dir.path()).unwrap();
    let diags = apply_user_manifest(&proposal, &user, &fresh, true).expect_err("drift must refuse");
    assert_actionable(&diags, "SOMA-ONB-0004");
}

#[test]
fn apply_requires_explicit_approval() {
    let dir = cargo_repo();
    let survey = survey_repository(dir.path()).unwrap();
    let proposal = manifest_from_survey(&survey);
    let mut user = proposal.clone();
    user.approved = false; // not approved in the reviewed document

    let fresh = survey_repository(dir.path()).unwrap();
    let diags =
        apply_user_manifest(&proposal, &user, &fresh, true).expect_err("unapproved apply refuses");
    assert_actionable(&diags, "SOMA-ONB-0006");

    // And the caller must pass the approval flag too.
    user.approved = true;
    let diags =
        apply_user_manifest(&proposal, &user, &fresh, false).expect_err("flag-less apply refuses");
    assert_actionable(&diags, "SOMA-ONB-0006");
}

#[test]
fn onboarding_never_modifies_project_files() {
    let dir = cargo_repo();
    let before = snapshot(dir.path());
    assert!(
        !dir.path().join(".prometheos-lite").exists(),
        "survey writes nothing"
    );

    let survey = survey_repository(dir.path()).unwrap();
    let _proposal = manifest_from_survey(&survey);
    assert_eq!(
        snapshot(dir.path()),
        before,
        "survey + proposal write nothing"
    );

    // A refused apply also writes nothing (checked by the caller writing
    // only after success — proven here by the state remaining identical).
    let mut user =
        serde_json::from_str::<OnboardingManifestV1>(&serde_json::to_string(&_proposal).unwrap())
            .unwrap();
    user.approved = false;
    let fresh = survey_repository(dir.path()).unwrap();
    assert!(apply_user_manifest(&_proposal, &user, &fresh, true).is_err());
    assert_eq!(
        snapshot(dir.path()),
        before,
        "a refused apply writes nothing"
    );
}

// ---------------------------------------------------------------------------
// Determinism: the same repository surveys identically.
// ---------------------------------------------------------------------------

#[test]
fn onboarding_is_deterministic() {
    let dir = cargo_repo();
    let a = survey_repository(dir.path()).unwrap();
    let b = survey_repository(dir.path()).unwrap();
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        serde_json::to_string(&b).unwrap()
    );
    assert_eq!(a.survey_digest, b.survey_digest);
    assert_eq!(
        a.survey_digest.len(),
        64,
        "digest is 64-hex over the canonical survey"
    );
    let pa = manifest_from_survey(&a);
    let pb = manifest_from_survey(&b);
    assert_eq!(
        serde_json::to_string(&pa).unwrap(),
        serde_json::to_string(&pb).unwrap()
    );
}

#[test]
fn unsupported_codes_are_scoped_to_onboarding() {
    // The pinned extension codes resolve to their categories and nothing
    // upstream shadows them (mirror-pinned in emitted_diagnostics_conformance).
    for (code, category) in [
        ("SOMA-ONB-0001", "onboarding_unsupported"),
        ("SOMA-ONB-0002", "onboarding_unsupported"),
        ("SOMA-ONB-0003", "onboarding_conflict"),
        ("SOMA-ONB-0004", "onboarding_state"),
        ("SOMA-ONB-0005", "onboarding_conflict"),
        ("SOMA-ONB-0006", "onboarding_state"),
    ] {
        let emitted = Diagnostic::new(code, "category guard");
        assert_eq!(
            emitted.category, category,
            "{code} must resolve through the Lite extension registry"
        );
    }
}

// Silence unused warnings for the survey type used in signatures above.
#[allow(dead_code)]
fn _survey_type_witness(_: Option<&OnboardingSurveyV1>) {}
