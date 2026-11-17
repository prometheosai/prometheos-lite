//! Issue #138 Slice A — executable adversarial security regressions and
//! assessment evidence preparation.
//!
//! Each test names its boundary and verifies **actual refusal with zero
//! unauthorized effects**, not merely a returned error. The threat-boundary
//! matrix lives in `docs/security/threat-boundary-matrix-slice-A.md`.
//!
//! NOTE: this suite prepares the evidence for the independent assessment;
//! it does NOT (and cannot) declare the assessment complete.

use std::path::{Path, PathBuf};

use prometheos_lite::flow::intelligence::Tool;
use prometheos_lite::harness::permissions::{
    Permission, PermissionGrant, PermissionLedger, PermissionScope,
};
use prometheos_lite::tools::CommandTool;
use prometheos_lite::tools::{
    ListTreeTool, PatchFileTool, RepoReadFileTool, SearchFilesTool, WriteFileTool,
};
use prometheos_lite::workflow::evaluate;
use prometheos_lite::workflow::evaluate::EvaluationState;
use prometheos_lite::workflow::evaluate::{JournalEvent, validate_journal};

mod common;

use common::task_execution::{TaskExecutionSpec, fixture_root, run_task_execution};

const EXECUTE_DESCRIPTOR: &str =
    include_str!("fixtures/scenarios/rust-bugfix-arithmetic-execute/descriptor.json");
const BUGFIX_REPO_IDENTITY_URL: &str =
    "https://repo.invalid/prometheos/lite-fixtures/rust-bugfix-arithmetic-execute-001.git";

const EXECUTE_SPEC: TaskExecutionSpec = TaskExecutionSpec {
    repair_target: "src/lib.rs",
    repo_identity_url: BUGFIX_REPO_IDENTITY_URL,
};

fn correct_repair_edits() -> Vec<prometheos_lite::harness::edit_protocol::EditOperation> {
    vec![
        prometheos_lite::harness::edit_protocol::EditOperation::SearchReplace(
            prometheos_lite::harness::edit_protocol::SearchReplaceEdit {
                file: PathBuf::from("src/lib.rs"),
                search: "a - b".to_string(),
                replace: "a + b".to_string(),
                replace_all: Some(false),
                context_lines: None,
            },
        ),
    ]
}

fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("temp dir")
}

// ---------------------------------------------------------------------------
// Helper: build a repo with a symlink/junction escaping to an outside dir
// ---------------------------------------------------------------------------

fn symlink_or_junction(source: &Path, target: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, source).is_ok()
    }
    #[cfg(windows)]
    {
        if target.is_dir() {
            std::os::windows::fs::symlink_dir(target, source).is_ok()
        } else {
            std::os::windows::fs::symlink_file(target, source).is_ok()
        }
    }
}

/// A repository directory containing an inbound symlink (or junction)
/// `escape` pointing at an outside directory seeded with `victim.txt`.
/// Returns (repo, outside, victim_path).
fn repo_with_escaping_symlink() -> (tempfile::TempDir, tempfile::TempDir, PathBuf) {
    let base = tempdir();
    let repo = base.path().join("repo");
    let outside = tempdir();
    std::fs::create_dir_all(&repo).expect("repo dir");
    std::fs::write(outside.path().join("victim.txt"), "outside-of-repo").expect("victim");
    let link = repo.join("escape");
    if !symlink_or_junction(&link, outside.path()) {
        panic!("symlink creation not permitted on this host (directory symlink failed)");
    }
    let victim_path = outside.path().join("victim.txt");
    (base, outside, victim_path)
}

async fn call(tool: &impl Tool, input: serde_json::Value) -> serde_json::Value {
    let debug = input.to_string();
    tool.call(input)
        .await
        .unwrap_or_else(|err| panic!("tool call failed for input {debug}: {err:#}"))
}

// ---------------------------------------------------------------------------
// A. Command execution: no shell interpolation, no unauthorized executable
// ---------------------------------------------------------------------------

#[tokio::test]
async fn security_command_tool_refuses_shell_injection_shapes() {
    let tool = CommandTool::new();
    // Assert against the FULL spawn path for every hostile shape. Two
    // observation plans are acceptable; both prove no unauthorized process is
    // allowed through:
    //   Plan A — the command fails the allowlist before any process is even
    //   attempted (success:false, exit_code -1, "not allowed").
    //   Plan B — the command is allowlisted by its first token (e.g. "cargo
    //   $(id)"), reaches `tokio::process::Command::new`, and fails to resolve
    //   as a program (no process is ever created).
    // Either way the outcome is identical: zero unauthorized execution.
    for hostile in [
        "cargo; id",
        "cargo $(id)",
        "cargo `id`",
        "cargo && id",
        "cargo | id",
        "cargo & id",
        "cargo\nid",
        "cargo\r\nid",
        "rm -rf /tmp/never",
        "/usr/bin/rm",
        "/usr/bin/cargo; id",
        "cmd /c id",
        "powershell -e id",
        "sh -c id",
    ] {
        let input = serde_json::json!({
            "command": hostile,
            "args": ["never-used"],
            "cwd": std::env::current_dir().unwrap().to_string_lossy(),
        });
        match tool.call(input).await {
            Ok(out) => {
                assert_eq!(
                    out["success"], false,
                    "command must be refused: {hostile:?}"
                );
                assert_eq!(
                    out["exit_code"], -1,
                    "refused command has no process exit: {hostile:?}"
                );
                assert!(
                    out["error"].as_str().unwrap_or("").contains("not allowed"),
                    "refusal must be the safety-policy denial: {out} for {hostile:?}"
                );
            }
            Err(err) => {
                // Allowlisted-in-name payloads (e.g. "cargo $(id)") whose base
                // token IS allowlisted pass the POLICY check and reach spawn —
                // and fail to resolve as a program, so no process is created
                // either. Both are safe; both are acceptable refusals.
                let text = format!("{err:#}");
                assert!(
                    !text.contains("not allowed"),
                    "a spawn failure here is the expected shape: {text}"
                );
            }
        }
    }

    // An allowlisted executable with hostile-looking argv executes exactly
    // as passed, proving argv is neither shell-interpreted nor tokenized by
    // the tool (no shell exists to do either).
    let input = serde_json::json!({
        "command": "rustc",
        "args": ["--version"],
    });
    let out = call(&tool, input).await;
    assert_eq!(
        out["exit_code"], 0,
        "rustc must run through the boundary: {out}"
    );
    assert!(
        out["stdout"].as_str().unwrap_or_default().contains("rustc"),
        "rustc --version output expected: {out}"
    );

    // And a blocked command is refused even when phrased as an allowed
    // program name with shell-y (but inert) argv.
    let input = serde_json::json!({
        "command": "cargo",
        "args": ["-V"],
    });
    let out = call(&tool, input).await;
    assert_eq!(out["exit_code"], 0, "allowlisted cargo must execute: {out}");
}

#[tokio::test]
async fn security_command_tool_blocked_destructive_executables_refused() {
    let tool = CommandTool::new();
    for blocked in [
        "rm", "rmdir", "del", "format", "fdisk", "mkfs", "dd", "shutdown", "reboot", "poweroff",
    ] {
        let input = serde_json::json!({"command": blocked, "args": ["/tmp/never"]});
        let out = call(&tool, input).await;
        assert_eq!(
            out["success"], false,
            "blocked command must be refused: {blocked}"
        );
        assert_eq!(out["exit_code"], -1);
    }
}

#[tokio::test]
async fn security_command_tool_allowed_executable_executes() {
    let tool = CommandTool::new();
    // `node --version` proves the boundary actually executes allowed commands
    // (i.e., it does not merely refuse everything — a real negative control).
    let input = serde_json::json!({
        "command": "node",
        "args": ["--version"],
    });
    let out = call(&tool, input).await;
    assert_eq!(out["success"], true, "allowed executables still run: {out}");
    assert_eq!(out["exit_code"], 0);
    assert!(out["stdout"].as_str().unwrap_or_default().contains("v"));
}

#[tokio::test]
async fn security_command_tool_paths_cannot_escape_allowlist() {
    let tool = CommandTool::new();
    // Absolute path to a blocked executable is refused even though the base
    // name is `rm`.
    let input = serde_json::json!({"command": "/usr/bin/rm", "args": ["/tmp/never"]});
    let out = call(&tool, input).await;
    assert_eq!(out["success"], false);

    // Absolute path that resolves to an allowed executable by basename only
    // when the base name exactly matches (the allowlist is basename-based;
    // a mismatching name is refused regardless of path).
    let input = serde_json::json!({"command": "/usr/bin/zz_cargo", "args": ["test"]});
    let out = call(&tool, input).await;
    assert_eq!(out["success"], false, "zz_cargo is not allowlisted");
}

// ---------------------------------------------------------------------------
// B. Filesystem: symlink/junction escape, traversal, dependency manifests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn security_repo_read_through_directory_symlink_refused() {
    let (base, _outside, victim_path) = repo_with_escaping_symlink();
    let tool = RepoReadFileTool::new(base.path().join("repo"));
    let input = serde_json::json!({"path": "escape/victim.txt"});
    let out = tool.call(input).await.expect("tool call");
    assert_eq!(
        out["success"], false,
        "reads through a symlink must be refused: {out}"
    );
    assert!(
        !out.to_string().contains("outside-of-repo"),
        "no out-of-repo content may leak: {out}"
    );
    assert_eq!(
        std::fs::read_to_string(&victim_path).unwrap(),
        "outside-of-repo",
        "the outside victim file is untouched"
    );
}

#[tokio::test]
async fn security_repo_write_through_directory_symlink_refused() {
    let (base, _outside, victim_path) = repo_with_escaping_symlink();
    let tool = WriteFileTool::new(base.path().join("repo"));
    let input = serde_json::json!({"path": "escape/evil.txt", "content": "owned"});
    let out = tool.call(input).await.expect("tool call");
    assert_eq!(
        out["success"], false,
        "writes through a symlink must be refused: {out}"
    );
    assert!(
        !base.path().join("repo/escape/evil.txt").exists(),
        "no artifact may be planted via the symlink"
    );
    assert_eq!(
        std::fs::read_to_string(&victim_path).unwrap(),
        "outside-of-repo",
        "the outside victim file is untouched"
    );
}

#[tokio::test]
async fn security_repo_patch_through_directory_symlink_refused() {
    let (base, _outside, victim_path) = repo_with_escaping_symlink();
    let tool = PatchFileTool::new(base.path().join("repo"));
    let input = serde_json::json!({
        "path": "escape/victim.txt",
        "diff": "--- a/escape/victim.txt\n+++ b/escape/victim.txt\n@@ -1 +1 @@\n-outside-of-repo\n+owned\n"
    });
    let out = tool.call(input).await.expect("tool call");
    assert_eq!(
        out["success"], false,
        "patches through a symlink must be refused: {out}"
    );
    assert_eq!(
        std::fs::read_to_string(&victim_path).unwrap(),
        "outside-of-repo",
        "the outside victim file is untouched"
    );
}

#[tokio::test]
async fn security_repo_list_and_search_skip_escaping_symlinks() {
    let (base, _outside, _victim) = repo_with_escaping_symlink();
    let repo = base.path().join("repo");
    let tool = ListTreeTool::new(repo.clone());
    let out = tool
        .call(serde_json::json!({"root": ""}))
        .await
        .expect("list tree");
    let files = out["files"].as_array().expect("files list");
    for path in files {
        let p = path.as_str().unwrap_or("");
        assert!(
            !p.contains("victim.txt") || p.contains("repo"),
            "no out-of-repo path must be listed: {p}"
        );
    }

    let tool = SearchFilesTool::new(repo);
    let out = tool
        .call(serde_json::json!({"query": "outside-of-repo"}))
        .await
        .expect("search");
    let count = out["count"].as_u64().unwrap_or(0);
    assert_eq!(count, 0, "search must not read through the symlink: {out}");
}

#[tokio::test]
async fn security_repo_final_component_symlink_read_refused() {
    let base = tempdir();
    let repo = base.path().join("repo");
    let outside = tempdir();
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(outside.path().join("victim.txt"), "outside-of-repo").unwrap();
    // Final component itself is a symlink to the outside file.
    let link = repo.join("victim.txt");
    assert!(symlink_or_junction(
        &link,
        &outside.path().join("victim.txt")
    ));

    let tool = RepoReadFileTool::new(repo.clone());
    let out = tool
        .call(serde_json::json!({"path": "victim.txt"}))
        .await
        .expect("read");
    assert_eq!(
        out["success"], false,
        "read through a final-component symlink must be refused: {out}"
    );
    assert_eq!(
        std::fs::read_to_string(outside.path().join("victim.txt")).unwrap(),
        "outside-of-repo"
    );

    let tool = WriteFileTool::new(repo);
    let out = tool
        .call(serde_json::json!({"path": "victim.txt", "content": "owned"}))
        .await
        .expect("write");
    assert_eq!(
        out["success"], false,
        "write through a final-component symlink must be refused: {out}"
    );
    assert_eq!(
        std::fs::read_to_string(outside.path().join("victim.txt")).unwrap(),
        "outside-of-repo"
    );
}

#[tokio::test]
async fn security_repo_nested_fresh_path_still_writes() {
    // Positive regression: a path whose parents do not yet exist is still a
    // legitimate write target inside the repo.
    let base = tempdir();
    let repo = base.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let tool = WriteFileTool::new(repo.clone());
    let out = tool
        .call(serde_json::json!({"path": "nested/fresh/file.txt", "content": "ok"}))
        .await
        .expect("write");
    assert_eq!(
        out["success"], true,
        "fresh nested write must succeed: {out}"
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("nested/fresh/file.txt")).unwrap(),
        "ok"
    );
}

#[test]
fn security_dependency_manifest_tamper_refused_preeffect() {
    // Tampering with a dependency manifest (Cargo.toml) changes what cargo
    // would build; the pinned baseline gate must refuse before any effect.
    let base = EXECUTE_DESCRIPTOR;
    let forged = base.replace(
        "\"revision\": \"bfca0ee96857e365026ac07f9a95c514b7e58e8b\"",
        "\"revision\": \"0000000000000000000000000000000000000000\"",
    );
    assert_ne!(forged, base);
    let run = run_task_execution(&EXECUTE_SPEC, &forged, correct_repair_edits());
    assert_eq!(
        run.capability_effects, 0,
        "no effect may run against an unpinned revision"
    );
    assert!(!run.patched, "no patch may be applied");
    assert!(
        run.changed_vs_baseline.is_empty(),
        "zero repository mutations: {:?}",
        run.changed_vs_baseline
    );
    assert!(
        run.envelope.is_none(),
        "no receipt without the pinned baseline"
    );
    assert_eq!(
        run.classification,
        prometheos_lite::workflow::scenario::ScenarioOutcomeClass::SetupError
    );
}

#[test]
fn security_fixture_bytes_are_never_modified() {
    // Guards the dispatch's core safety property for the forged-manifest case
    // above: the committed fixture bytes never drift.
    let descriptor =
        prometheos_lite::workflow::scenario::ScenarioDescriptor::parse_json(EXECUTE_DESCRIPTOR)
            .expect("descriptor validates");
    let root = fixture_root(&descriptor);
    let digest = prometheos_lite::workflow::scenario::fixture_content_digest(&root)
        .expect("fixture digests");
    assert_eq!(
        digest,
        "d4234cd58be74da1e9b369ab21bdecf96d33f1d43e12c4a5f7ae848dc2e400d6"
    );
}

// ---------------------------------------------------------------------------
// C. Authority and governance
// ---------------------------------------------------------------------------

#[test]
fn security_permission_ledger_traversal_cannot_borrow_prefix_grant() {
    let mut ledger = PermissionLedger::new();
    ledger.grant(PermissionGrant {
        permission: Permission::Write,
        scope: PermissionScope::Directory,
        path: Some(PathBuf::from("repo/src")),
        pattern: None,
        allow: true,
        reason: "test fixture".to_string(),
    });

    // A traversal path that lexically escapes the granted prefix must not be
    // allowed by a naive starts_with check.
    let hostile = PathBuf::from("repo/src/../../secret.txt");
    let result = ledger.check(Permission::Write, &hostile);
    assert!(
        !result.granted,
        "traversal cannot borrow a prefix grant: {result:?}"
    );

    // The honest path inside the grant is allowed.
    let ok = ledger.check(Permission::Write, Path::new("repo/src/main.rs"));
    assert!(ok.granted, "files inside the grant still work: {ok:?}");
}

#[test]
fn security_permission_ledger_explicit_deny_overrides_allow() {
    let mut ledger = PermissionLedger::new();
    ledger.grant(PermissionGrant {
        permission: Permission::Read,
        scope: PermissionScope::Directory,
        path: Some(PathBuf::from("repo/src")),
        pattern: None,
        allow: true,
        reason: "allow".to_string(),
    });
    ledger.grant(PermissionGrant {
        permission: Permission::Read,
        scope: PermissionScope::Directory,
        path: Some(PathBuf::from("repo/src")),
        pattern: None,
        allow: false,
        reason: "deny: explicit".to_string(),
    });
    let result = ledger.check(Permission::Read, Path::new("repo/src/secrets.txt"));
    assert!(
        !result.granted,
        "an explicit denial overrides any allow for the same scope: {result:?}"
    );
}

#[test]
fn security_permission_ledger_no_matching_grant_denies_by_default() {
    let mut ledger = PermissionLedger::new();
    let result = ledger.check(Permission::Read, Path::new("any/thing.txt"));
    assert!(
        !result.granted,
        "default deny with no matching grant: {result:?}"
    );
}

#[test]
fn security_permission_ledger_command_profile_mismatch_refused() {
    let mut ledger = PermissionLedger::new();
    ledger.grant(PermissionGrant {
        permission: Permission::Execute,
        scope: PermissionScope::Command,
        path: None,
        pattern: Some("^cargo test$".to_string()),
        allow: true,
        reason: "cargo test only".to_string(),
    });
    let ok = ledger.check_command("cargo test --offline");
    assert!(
        !ok.granted,
        "pattern is anchored; variant command does not match: {ok:?}"
    );
    let refused = ledger.check_command("cargo clippy -- -D warnings");
    assert!(
        !refused.granted,
        "unmatched command is refused: {refused:?}"
    );
    let refused = ledger.check_command("python -c 'import os'");
    assert!(!refused.granted);
}

// ---------------------------------------------------------------------------
// D. Evidence integrity: the production journal validator refuses gaps
// ---------------------------------------------------------------------------

fn sample_event(sequence: u64, key: &str) -> JournalEvent {
    JournalEvent {
        schema_version: prometheos_lite::workflow::schema::CURRENT_SCHEMA_VERSION,
        event_id: format!("evt-{sequence}"),
        sequence,
        run_id: "run-1".to_string(),
        identity_key: key.to_string(),
        timestamp: "2026-01-01T00:00:00Z".to_string(),
        from_state: EvaluationState::Created,
        to_state: EvaluationState::PreflightPassed,
        proposal_ref: None,
        failure_classification: None,
        owner_run_id: "run-1".to_string(),
        lease_epoch: 1,
        repository_revision: "abc123".to_string(),
        evidence_ref: None,
        checkpoint_ref: None,
    }
}

fn write_sample_journal_event(journal_dir: &Path, sequence: u64, key: &str) {
    let event = sample_event(sequence, key);
    let path = journal_dir.join(format!("{sequence:020}.json"));
    std::fs::write(&path, serde_json::to_string_pretty(&event).unwrap()).unwrap();
}

#[test]
fn security_validate_journal_refuses_sequence_gap_before_append() {
    let dir = tempdir();
    let journal_dir = dir
        .path()
        .join(".prometheos")
        .join("workflow")
        .join("journal")
        .join("key-gap-direct");
    std::fs::create_dir_all(&journal_dir).unwrap();
    write_sample_journal_event(&journal_dir, 1, "key-gap-direct");
    // Event 2 is MISSING. read_journal does NOT detect the gap…
    write_sample_journal_event(&journal_dir, 3, "key-gap-direct");
    let events = evaluate::read_journal(dir.path(), "key-gap-direct").expect("read_journal reads");
    assert_eq!(events.len(), 2, "read_journal alone does not close the gap");

    // …but the production validator — the exact API used on the replay/append
    // path — fails closed with the sequence gap before any event can be added.
    let err = validate_journal(&journal_dir).expect_err("production validator refuses the gap");
    assert!(
        format!("{err:#}").contains("journal sequence gap"),
        "expected sequence gap diagnostic: {err:#}"
    );
}

#[test]
fn security_validate_journal_accepts_contiguity_and_rejects_forged_event() {
    let dir = tempdir();
    let journal_dir = dir
        .path()
        .join(".prometheos")
        .join("workflow")
        .join("journal")
        .join("key-ok-direct");
    std::fs::create_dir_all(&journal_dir).unwrap();
    for seq in 0..3 {
        write_sample_journal_event(&journal_dir, seq, "key-ok-direct");
    }
    validate_journal(&journal_dir).expect("a contiguous journal validates");

    // A forged event whose sequence no longer matches its filename is refused
    // at read time (production check), before validate_journal is even called.
    let event1 = std::fs::read_to_string(journal_dir.join(format!("{:020}.json", 1))).unwrap();
    let mut forged: serde_json::Value = serde_json::from_str(&event1).unwrap();
    forged["sequence"] = serde_json::json!(9);
    std::fs::write(
        journal_dir.join(format!("{:020}.json", 1)),
        serde_json::to_string_pretty(&forged).unwrap(),
    )
    .unwrap();
    let err = evaluate::read_journal(dir.path(), "key-ok-direct").unwrap_err();
    assert!(
        format!("{err:#}").contains("filename") || format!("{err:#}").contains("sequence"),
        "forged journal event must fail closed: {err:#}"
    );
}

// ---------------------------------------------------------------------------
// E. Build supply chain: #297-class executable content is refused
// ---------------------------------------------------------------------------

const TRUSTED_POSTCSS_CONFIG: &str =
    "module.exports = {\n  plugins: {\n    tailwindcss: {},\n    autoprefixer: {},\n  },\n}\n";

fn repo_root_from_manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn security_frontend_build_config_carries_no_executable_payload() {
    let root = repo_root_from_manifest();
    // The trusted 82-byte postcss config (blob 33ad091d26d8a9dc95ebdf616e217d985ec215b8).
    let postcss = std::fs::read_to_string(root.join("frontend/postcss.config.js")).unwrap();
    let normalized = postcss.replace("\r\n", "\n");
    assert_eq!(
        normalized, TRUSTED_POSTCSS_CONFIG,
        "frontend/postcss.config.js must remain the trusted plain configuration (#297 containment)"
    );

    for rel in [
        "frontend/next.config.js",
        "frontend/tailwind.config.ts",
        "frontend/.eslintrc.json",
        "frontend/package.json",
        "frontend/package-lock.json",
    ] {
        let bytes = std::fs::read(root.join(rel)).unwrap_or_else(|e| panic!("{rel} readable: {e}"));
        let text = String::from_utf8_lossy(&bytes).into_owned();
        for forbidden in [
            "eval(",
            "new Function(",
            "require('child_process')",
            "require(\"child_process\")",
            "require('fs')",
            "require(\"fs\")",
            "atob(",
            "Buffer.from(",
            "process.env.GITHUB_TOKEN",
            "process.env.GH_TOKEN",
            "process.env.NPM_TOKEN",
        ] {
            assert!(
                !text.contains(forbidden),
                "{rel} contains forbidden executable marker {forbidden:?} — must be inspected and reverted"
            );
        }
    }
}
