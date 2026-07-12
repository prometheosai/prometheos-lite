//! #78 — governed proposals through a `PatchProvider`.
//!
//! End-to-end tests for the provider-backed proposal path. Every test uses the
//! deterministic [`MockProposalProvider`] (no network, no model), proving the
//! generated patch is routed through the existing #77 governed workflow and is
//! treated as hostile input.

use prometheos_lite::harness::patch_provider::{
    MockProposalMode, MockProposalProvider, PatchProviderContext,
};
use prometheos_lite::workflow::{self, AuthorityLevel, GenerateScope, ProviderRouteInfo};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A throwaway git repo with one committed source file.
fn temp_repo() -> (TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().to_path_buf();
    git(&repo, &["init"]);
    git(&repo, &["config", "user.email", "t@t"]);
    git(&repo, &["config", "user.name", "t"]);
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src/main.rs"), "pub fn main() {}\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "init"]);
    (dir, repo)
}

fn safe_scope() -> GenerateScope {
    GenerateScope {
        allowed_paths: vec!["src/**".to_string()],
        forbidden_paths: vec![],
        allow_dependency_changes: false,
        max_files_changed: None,
        max_lines_changed: None,
    }
}

fn ctx(goal: &str) -> PatchProviderContext {
    PatchProviderContext {
        task: goal.to_string(),
        ..Default::default()
    }
}

async fn generate_safe(repo: &Path) -> workflow::GenerateResult {
    let provider = MockProposalProvider::safe();
    workflow::generate_proposal(
        repo,
        "add generated file",
        AuthorityLevel::Assist,
        &provider,
        ctx("add generated file"),
        &safe_scope(),
        None,
        None,
    )
    .await
    .expect("safe provider should generate a proposal")
}

// 1. mock provider generates a valid proposal consumed by existing dry-run/approval/apply flow
#[tokio::test]
async fn mock_provider_drives_existing_gates() {
    let (_d, repo) = temp_repo();
    let res = generate_safe(&repo).await;

    workflow::dry_run(&repo, &res.id, None).expect("dry-run should pass");
    workflow::approve(&repo, &res.id, &res.patch_hash, "op").expect("approve should pass");
    workflow::apply(&repo, &res.id, &res.patch_hash, None, true).expect("apply should pass");

    let content = std::fs::read_to_string(repo.join("src/generated_patch.rs")).unwrap();
    assert!(content.contains("generated"), "tree not patched: {content}");
}

// 2. provider output with path outside allowed scope is rejected
#[tokio::test]
async fn provider_out_of_scope_rejected() {
    let (_d, repo) = temp_repo();
    let provider = MockProposalProvider::with_mode(MockProposalMode::OutOfScope);
    let r = workflow::generate_proposal(
        &repo,
        "g",
        AuthorityLevel::Assist,
        &provider,
        ctx("g"),
        &safe_scope(),
        None,
        None,
    )
    .await;
    assert!(r.is_err(), "out-of-scope provider output must be rejected");
}

// 3. provider output touching forbidden path is rejected
#[tokio::test]
async fn provider_forbidden_path_rejected() {
    let (_d, repo) = temp_repo();
    let scope = GenerateScope {
        forbidden_paths: vec!["src/secrets/".to_string()],
        ..safe_scope()
    };
    let provider = MockProposalProvider::with_mode(MockProposalMode::Forbidden);
    let r = workflow::generate_proposal(
        &repo,
        "g",
        AuthorityLevel::Assist,
        &provider,
        ctx("g"),
        &scope,
        None,
        None,
    )
    .await;
    assert!(
        r.is_err(),
        "forbidden-path provider output must be rejected"
    );
}

// 4. dependency manifest change is rejected unless allowed
#[tokio::test]
async fn dependency_manifest_rejected_unless_allowed() {
    let (_d, repo) = temp_repo();
    let provider = MockProposalProvider::with_mode(MockProposalMode::Dependency);

    let blocked = workflow::generate_proposal(
        &repo,
        "g",
        AuthorityLevel::Assist,
        &provider,
        ctx("g"),
        &safe_scope(),
        None,
        None,
    )
    .await;
    assert!(
        blocked.is_err(),
        "dependency change must be rejected without permission"
    );

    let allowed = workflow::generate_proposal(
        &repo,
        "g",
        AuthorityLevel::Assist,
        &provider,
        ctx("g"),
        &GenerateScope {
            allowed_paths: vec![],
            forbidden_paths: vec![],
            allow_dependency_changes: true,
            max_files_changed: None,
            max_lines_changed: None,
        },
        None,
        None,
    )
    .await;
    assert!(
        allowed.is_ok(),
        "dependency change must be allowed when permitted"
    );
}

// 5. absolute path and `../` traversal are rejected
#[tokio::test]
async fn absolute_and_traversal_paths_rejected() {
    let (_d, repo) = temp_repo();

    let absolute = MockProposalProvider::with_mode(MockProposalMode::Absolute);
    let r_abs = workflow::generate_proposal(
        &repo,
        "g",
        AuthorityLevel::Assist,
        &absolute,
        ctx("g"),
        &safe_scope(),
        None,
        None,
    )
    .await;
    assert!(r_abs.is_err(), "absolute path must be rejected");

    let traversal = MockProposalProvider::with_mode(MockProposalMode::Traversal);
    let r_trav = workflow::generate_proposal(
        &repo,
        "g",
        AuthorityLevel::Assist,
        &traversal,
        ctx("g"),
        &safe_scope(),
        None,
        None,
    )
    .await;
    assert!(r_trav.is_err(), "'..' traversal must be rejected");
}

// 6. malformed/non-diff provider output is rejected
#[tokio::test]
async fn malformed_provider_output_rejected() {
    let (_d, repo) = temp_repo();
    let provider = MockProposalProvider::with_mode(MockProposalMode::Malformed);
    let r = workflow::generate_proposal(
        &repo,
        "g",
        AuthorityLevel::Assist,
        &provider,
        ctx("g"),
        &safe_scope(),
        None,
        None,
    )
    .await;
    assert!(r.is_err(), "malformed/unsupported patch must be rejected");
}

// 7. provider metadata cannot override internally derived hash/files/line counts
#[tokio::test]
async fn derived_metadata_not_overridable() {
    let (_d, repo) = temp_repo();
    let res = generate_safe(&repo).await;

    let report = workflow::report(&repo, &res.id).expect("report should succeed");
    let value: serde_json::Value = serde_json::from_str(&report).expect("valid json");

    // Hash is recomputed internally from the patch, never taken from the provider.
    let stored_hash = value["patch_hash"].as_str().expect("patch_hash present");
    let mut hasher = Sha256::new();
    hasher.update(res.patch.as_bytes());
    let computed = format!("{:x}", hasher.finalize());
    assert_eq!(
        stored_hash, computed,
        "patch hash must be derived internally"
    );

    // Files/lines are also derived from the patch itself.
    let files = value["changed_files"]
        .as_array()
        .expect("changed_files present");
    assert!(
        files
            .iter()
            .any(|f| f.as_str() == Some("src/generated_patch.rs")),
        "changed files must be derived from the patch"
    );
    assert!(value["added_lines"].as_u64().unwrap_or(0) > 0);
}

// 8. `review` authority cannot generate a modifying patch
#[tokio::test]
async fn review_authority_cannot_generate() {
    let (_d, repo) = temp_repo();
    let provider = MockProposalProvider::safe();
    let r = workflow::generate_proposal(
        &repo,
        "g",
        AuthorityLevel::Review,
        &provider,
        ctx("g"),
        &safe_scope(),
        None,
        None,
    )
    .await;
    assert!(r.is_err(), "review authority must not generate a patch");
}

// 9. `propose` authority cannot apply
#[tokio::test]
async fn propose_authority_cannot_apply() {
    let (_d, repo) = temp_repo();
    let provider = MockProposalProvider::safe();
    let res = workflow::generate_proposal(
        &repo,
        "g",
        AuthorityLevel::Propose,
        &provider,
        ctx("g"),
        &safe_scope(),
        None,
        None,
    )
    .await
    .expect("propose authority may generate");

    workflow::dry_run(&repo, &res.id, None).expect("dry-run should pass");
    workflow::approve(&repo, &res.id, &res.patch_hash, "op").expect("approve should pass");
    let applied = workflow::apply(&repo, &res.id, &res.patch_hash, None, true);
    assert!(applied.is_err(), "propose authority cannot apply");
}

// 10. provider failure creates no valid proposal artifact
#[tokio::test]
async fn provider_failure_creates_no_artifact() {
    let (_d, repo) = temp_repo();
    let provider = MockProposalProvider::with_mode(MockProposalMode::Failing);
    let r = workflow::generate_proposal(
        &repo,
        "g",
        AuthorityLevel::Assist,
        &provider,
        ctx("g"),
        &safe_scope(),
        None,
        None,
    )
    .await;
    assert!(r.is_err(), "provider failure must surface as error");
    assert!(
        !repo.join(".prometheos").join("workflow").exists(),
        "no proposal artifact must be created on provider failure"
    );
}

// 11. secrets/configuration values are absent from persisted provenance and reports
#[tokio::test]
async fn no_secrets_in_provenance_or_report() {
    let (_d, repo) = temp_repo();
    let res = generate_safe(&repo).await;
    let report = workflow::report(&repo, &res.id).expect("report should succeed");

    let lower = report.to_lowercase();
    assert!(
        !lower.contains("sk-"),
        "API key shape must not appear in report"
    );
    assert!(
        !lower.contains("authorization"),
        "authorization header must not appear in report"
    );
    assert!(
        !lower.contains("bearer"),
        "bearer token must not appear in report"
    );

    let value: serde_json::Value = serde_json::from_str(&report).expect("valid json");
    let prov = &value["provider_provenance"];
    assert_eq!(prov["implementation"], "mock");
    // Mock provider has no model/route; provenance stays free of secrets.
    assert!(prov["model"].is_null());
    assert!(prov["route"].is_null());
}

// 12. HEAD drift remains blocked
#[tokio::test]
async fn head_drift_remains_blocked() {
    let (_d, repo) = temp_repo();
    let res = generate_safe(&repo).await;
    workflow::dry_run(&repo, &res.id, None).expect("dry-run should pass");
    workflow::approve(&repo, &res.id, &res.patch_hash, "op").expect("approve should pass");

    // Move the repository HEAD away from the validated base.
    std::fs::write(repo.join("unrelated.rs"), "fn u() {}\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "move head"]);

    let applied = workflow::apply(&repo, &res.id, &res.patch_hash, None, true);
    assert!(applied.is_err(), "apply must be blocked after HEAD drift");
}

// 13. approval remains bound to the generated patch hash
#[tokio::test]
async fn approval_bound_to_generated_hash() {
    let (_d, repo) = temp_repo();
    let res = generate_safe(&repo).await;
    workflow::dry_run(&repo, &res.id, None).expect("dry-run should pass");

    let wrong = workflow::approve(&repo, &res.id, "deadbeef", "op");
    assert!(wrong.is_err(), "approval must reject a wrong patch hash");

    workflow::approve(&repo, &res.id, &res.patch_hash, "op").expect("approval with correct hash");
}

// 14. end-to-end mock-provider smoke passes without network access
#[tokio::test]
async fn end_to_end_mock_provider_offline() {
    let (_d, repo) = temp_repo();
    let before = std::fs::read_to_string(repo.join("src/main.rs")).unwrap();

    let res = generate_safe(&repo).await;

    // Fixture must remain unchanged before approval (only .prometheos metadata added).
    let after = std::fs::read_to_string(repo.join("src/main.rs")).unwrap();
    assert_eq!(
        before, after,
        "fixture source must be unchanged before approval"
    );
    assert!(
        !repo.join("src/generated_patch.rs").exists(),
        "patch must not be applied before approval"
    );

    workflow::dry_run(&repo, &res.id, None).expect("dry-run should pass in worktree");
    workflow::approve(&repo, &res.id, &res.patch_hash, "op").expect("approve should pass");
    workflow::apply(&repo, &res.id, &res.patch_hash, None, true).expect("apply should pass");

    let report = workflow::report(&repo, &res.id).expect("report should succeed");
    let value: serde_json::Value = serde_json::from_str(&report).expect("valid json");
    assert_eq!(value["applied"], true);
    assert_eq!(value["provider_provenance"]["implementation"], "mock");
    assert!(value["provider_provenance"]["patch_hash"].as_str() == Some(res.patch_hash.as_str()));
}

// --- Additional governance tests required by the blocking review ---

// 15. Windows drive-absolute provider paths are rejected (even on Linux CI)
#[tokio::test]
async fn windows_drive_path_rejected() {
    let (_d, repo) = temp_repo();
    let provider = MockProposalProvider::with_mode(MockProposalMode::WindowsDrive);
    let r = workflow::generate_proposal(
        &repo,
        "g",
        AuthorityLevel::Assist,
        &provider,
        ctx("g"),
        &safe_scope(),
        None,
        None,
    )
    .await;
    assert!(
        r.is_err(),
        "Windows drive path must be rejected on every platform"
    );
}

// 16. UNC provider paths are rejected (even on Linux CI)
#[tokio::test]
async fn unc_path_rejected() {
    let (_d, repo) = temp_repo();
    let provider = MockProposalProvider::with_mode(MockProposalMode::Unc);
    let r = workflow::generate_proposal(
        &repo,
        "g",
        AuthorityLevel::Assist,
        &provider,
        ctx("g"),
        &safe_scope(),
        None,
        None,
    )
    .await;
    assert!(r.is_err(), "UNC path must be rejected on every platform");
}

// 17. plain non-diff text is rejected before any proposal artifact is created
#[tokio::test]
async fn plain_text_rejected_before_artifact() {
    let (_d, repo) = temp_repo();
    let provider = MockProposalProvider::with_mode(MockProposalMode::PlainText);
    let r = workflow::generate_proposal(
        &repo,
        "g",
        AuthorityLevel::Assist,
        &provider,
        ctx("g"),
        &safe_scope(),
        None,
        None,
    )
    .await;
    assert!(r.is_err(), "plain text must not be persisted as a proposal");
    assert!(
        !repo.join(".prometheos").join("workflow").exists(),
        "no proposal artifact must be created for plain text"
    );
}

// 18. a deliberately secret-bearing route is sanitized in provenance
#[tokio::test]
async fn secret_bearing_route_is_sanitized() {
    let (_d, repo) = temp_repo();
    let provider = MockProposalProvider::safe();
    let route = ProviderRouteInfo {
        model: Some("gpt-4".to_string()),
        route: Some("https://sk-SECRETKEY123@api.example.com/v1/models?key=zzz#frag".to_string()),
    };
    let res = workflow::generate_proposal(
        &repo,
        "g",
        AuthorityLevel::Assist,
        &provider,
        ctx("g"),
        &safe_scope(),
        Some(route),
        None,
    )
    .await
    .expect("generate should succeed");

    let report = workflow::report(&repo, &res.id).expect("report should succeed");
    let value: serde_json::Value = serde_json::from_str(&report).expect("valid json");
    let route_json = value["provider_provenance"]["route"]
        .as_str()
        .expect("route present")
        .to_string();
    assert_eq!(
        route_json, "https://api.example.com",
        "route must be scheme://host only"
    );
    assert!(
        !route_json.contains("sk-SECRETKEY123"),
        "userinfo secret must be stripped"
    );
    assert!(!route_json.contains("/v1"), "path must be stripped");
    assert!(!route_json.contains("key=zzz"), "query must be stripped");
    assert!(!route_json.contains("#frag"), "fragment must be stripped");
    assert!(
        !report.to_lowercase().contains("sk-secretkey123"),
        "no secret anywhere in report"
    );
}

// 19. sanitize_provider_route keeps only scheme://host[:port]
#[test]
fn sanitize_provider_route_strips_secrets() {
    assert_eq!(
        workflow::sanitize_provider_route("https://sk-x@host:8080/p?q=1#f"),
        Some("https://host:8080".to_string())
    );
    assert_eq!(
        workflow::sanitize_provider_route("http://example.com"),
        Some("http://example.com".to_string())
    );
    assert_eq!(workflow::sanitize_provider_route("not-a-url"), None);
    assert_eq!(workflow::sanitize_provider_route("://nohost"), None);
}

// 20. structured lifecycle evidence appears in the report (validation + checkpoint + rollback)
#[tokio::test]
async fn report_exposes_lifecycle_evidence() {
    let (_d, repo) = temp_repo();
    let res = generate_safe(&repo).await;
    workflow::dry_run(&repo, &res.id, Some("true")).expect("dry-run should pass");
    workflow::approve(&repo, &res.id, &res.patch_hash, "op").expect("approve should pass");
    workflow::apply(&repo, &res.id, &res.patch_hash, Some("true"), true)
        .expect("apply should pass");

    let report = workflow::report(&repo, &res.id).expect("report should succeed");
    let value: serde_json::Value = serde_json::from_str(&report).expect("valid json");
    assert_eq!(value["dry_run_validation"], "true");
    assert_eq!(value["apply_validation"], "true");
    assert!(
        value["checkpoint_ref"]
            .as_str()
            .unwrap_or("")
            .starts_with("prometheos/checkpoint-")
    );
    assert_eq!(value["rollback_status"], "clean");
    assert_eq!(value["applied"], true);
}

// 21. rollback outcome is recorded as lifecycle evidence
#[tokio::test]
async fn rollback_outcome_recorded() {
    let (_d, repo) = temp_repo();
    let res = generate_safe(&repo).await;
    workflow::dry_run(&repo, &res.id, None).expect("dry-run should pass");
    workflow::approve(&repo, &res.id, &res.patch_hash, "op").expect("approve should pass");

    // Apply with a validation command that fails -> must roll back and record status.
    let applied = workflow::apply(&repo, &res.id, &res.patch_hash, Some("false"), true);
    assert!(applied.is_err(), "apply must fail validation and roll back");

    let report = workflow::report(&repo, &res.id).expect("report should succeed");
    let value: serde_json::Value = serde_json::from_str(&report).expect("valid json");
    assert_eq!(value["rollback_status"], "rolled_back");
    assert!(
        value["checkpoint_ref"]
            .as_str()
            .unwrap_or("")
            .starts_with("prometheos/checkpoint-")
    );
    // Tree reverted to original (no generated file).
    assert!(!repo.join("src/generated_patch.rs").exists());
}
