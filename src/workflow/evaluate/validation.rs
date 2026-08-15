use anyhow::{Context, Result, anyhow};
use std::path::Path;
use std::process::Stdio;

use tokio::process::Command as AsyncCommand;

use super::cancellation::CancellationToken;
use super::evidence::ValidationRecord;
use super::generation::load_proposal_from_repo;
use super::identity::{EvaluationState, now_iso};
use super::integrity::run_git_cmd;

// ---------------------------------------------------------------------------
// Validation (isolated worktree)
// ---------------------------------------------------------------------------

/// Run isolated validation, cooperatively cancellable.
///
/// The validation subprocess is a long-running external workload. This function
/// races the child wait against `token.cancelled()`: when cancellation is
/// requested the entire child process tree is terminated and the run returns an
/// error classified as a cooperative cancellation (it records no failed
/// validation, and recovery can later resume finalization without re-running
/// validation). Cancellation is a control-flow signal, NOT a validation
/// failure.
pub(super) async fn run_isolated_validation(
    repo: &Path,
    proposal_id: &str,
    validation_command: Option<&str>,
    evidence_dir: &Path,
    token: &CancellationToken,
) -> Result<ValidationRecord> {
    let proposal = load_proposal_from_repo(repo, proposal_id)?;
    let start_time = now_iso();

    let wt_root = std::env::temp_dir().join(format!("prometheos-eval-{proposal_id}"));
    // Clean any stale state.
    let _ = run_git_cmd(
        repo,
        &["worktree", "remove", "--force", wt_root.to_str().unwrap()],
    );
    let _ = std::fs::remove_dir_all(&wt_root);
    let _ = run_git_cmd(repo, &["worktree", "prune"]);

    let patch_file =
        std::env::temp_dir().join(format!("prometheos-eval-patch-{proposal_id}.patch"));
    std::fs::write(&patch_file, &proposal.patch)
        .context("failed to write patch file for validation")?;

    // Create detached worktree at base sha.
    run_git_cmd(
        repo,
        &[
            "worktree",
            "add",
            "--detach",
            wt_root.to_str().unwrap(),
            &proposal.base_sha,
        ],
    )
    .context("failed to create validation worktree")?;

    // Step 1: Check if patch applies cleanly.
    let patch_applies = run_git_cmd(
        &wt_root,
        &["apply", "--check", patch_file.to_str().unwrap()],
    )
    .is_ok();

    if !patch_applies {
        // Patch doesn't apply — record and clean up.
        let _ = run_git_cmd(
            repo,
            &["worktree", "remove", "--force", wt_root.to_str().unwrap()],
        );
        let _ = std::fs::remove_dir_all(&wt_root);
        let _ = std::fs::remove_file(&patch_file);

        let completion_time = now_iso();
        return Ok(ValidationRecord {
            validation_command: validation_command.map(|s| s.to_string()),
            exit_code: None,
            stdout_preview: String::new(),
            stderr_preview: "patch does not apply cleanly".to_string(),
            start_time,
            completion_time,
            test_discovered: false,
            test_executed: false,
            test_names_found: Vec::new(),
            test_count: 0,
            warnings: Vec::new(),
            failures: vec!["patch apply check failed".to_string()],
            patch_applies_cleanly: false,
            validation_passed: false,
        });
    }

    // Apply the patch.
    let _ = run_git_cmd(&wt_root, &["apply", patch_file.to_str().unwrap()]);

    // Step 2: Run validation command if present — cooperatively cancellable.
    let (exit_code, stdout, stderr) = match validation_command {
        Some(cmd) => {
            let child = validation_shell_async(cmd)
                .current_dir(&wt_root)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .context("failed to execute validation command")?;
            // Capture the pid up front so cancellation can signal the whole
            // process group even after the child is moved into the waiter task.
            let pid = child.id();
            let mut waiter = tokio::spawn(async move { child.wait_with_output().await });
            tokio::select! {
                res = &mut waiter => {
                    let output = res
                        .context("validation waiter task failed")?
                        .context("failed to wait on validation command")?;
                    (
                        Some(output.status.code().unwrap_or(-1)),
                        String::from_utf8_lossy(&output.stdout).to_string(),
                        String::from_utf8_lossy(&output.stderr).to_string(),
                    )
                }
                _ = token.cancelled() => {
                    // Terminate the whole validation subtree by pid, then stop
                    // waiting. `kill_on_drop` cleans up the spawned child too.
                    if let Some(pid) = pid {
                        kill_child_tree(pid).await;
                    }
                    waiter.abort();
                    return Err(anyhow!("validation cancelled by user request"));
                }
            }
        }
        None => (None, String::new(), String::new()),
    };

    let completion_time = now_iso();

    // Discover tests from output.
    let (test_discovered, test_executed, test_names, test_count, warnings, failures) =
        parse_test_evidence(&stdout, &stderr, exit_code);

    let validation_passed = exit_code.map(|c| c == 0).unwrap_or(true) && patch_applies;

    // Save raw logs.
    let stdout_path = evidence_dir.join("validation_stdout.log");
    let stderr_path = evidence_dir.join("validation_stderr.log");
    let _ = std::fs::write(&stdout_path, &stdout);
    let _ = std::fs::write(&stderr_path, &stderr);

    // Clean up worktree.
    let _ = run_git_cmd(
        repo,
        &["worktree", "remove", "--force", wt_root.to_str().unwrap()],
    );
    let _ = std::fs::remove_dir_all(&wt_root);
    let _ = std::fs::remove_file(&patch_file);

    Ok(ValidationRecord {
        validation_command: validation_command.map(|s| s.to_string()),
        exit_code,
        stdout_preview: truncate(&stdout, 4096),
        stderr_preview: truncate(&stderr, 4096),
        start_time,
        completion_time,
        test_discovered,
        test_executed,
        test_names_found: test_names,
        test_count,
        warnings,
        failures,
        patch_applies_cleanly: patch_applies,
        validation_passed,
    })
}
// ---------------------------------------------------------------------------
// Test evidence parsing
// ---------------------------------------------------------------------------

fn parse_test_evidence(
    stdout: &str,
    stderr: &str,
    exit_code: Option<i32>,
) -> (bool, bool, Vec<String>, usize, Vec<String>, Vec<String>) {
    let combined = format!("{stdout}\n{stderr}");

    // Discover test binary names.
    let test_names = extract_test_names(&combined);
    let test_discovered = !test_names.is_empty();

    // Detect test execution markers.
    let test_executed = combined.contains("test result:")
        || combined.contains("running")
        || combined.contains("test .")
        || combined.contains("FAILED")
        || combined.contains("ok")
        || combined.contains(".test.");

    // Count tests from "test result: ok. N passed" lines.
    let test_count = count_tests_from_output(&combined);

    // Extract warnings.
    let warnings = extract_patterns(&combined, &["warning:", "WARNING:", "warn:"]);

    // Extract failures.
    let failures = extract_patterns(
        &combined,
        &["FAILED", "error:", "ERROR:", "panicked", "failures:"],
    );

    // If exit code is non-zero and no specific failures found, add generic failure.
    if exit_code.map(|c| c != 0).unwrap_or(false) && failures.is_empty() {
        let mut f = Vec::new();
        f.push(format!(
            "validation exited with code {}",
            exit_code.unwrap()
        ));
        (
            test_discovered,
            test_executed,
            test_names,
            test_count,
            warnings,
            f,
        )
    } else {
        (
            test_discovered,
            test_executed,
            test_names,
            test_count,
            warnings,
            failures,
        )
    }
}

fn extract_test_names(output: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in output.lines() {
        // Rust test output: "test module::test_name ... ok"
        if line.starts_with("test ")
            && let Some(name) = line.split_whitespace().nth(1)
            && name != "result"
            && !names.contains(&name.to_string())
        {
            names.push(name.to_string());
        }
        // cargo test output: "Running target/..."
        if line.starts_with("Running ")
            && let Some(path) = line.strip_prefix("Running ")
        {
            let name = path.split('/').next_back().unwrap_or(path).to_string();
            if !name.is_empty() && !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

fn count_tests_from_output(output: &str) -> usize {
    let mut count = 0usize;
    for line in output.lines() {
        // "test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out"
        if let Some(rest) = line.strip_prefix("test result:") {
            for part in rest.split(';') {
                let part = part.trim();
                if let Some(passed_part) = part.strip_suffix("passed") {
                    // Format: "N passed" or "ok. N passed"
                    let passed_part = passed_part.trim();
                    // Extract the number: could be "5" or "ok. 5"
                    if let Some(n_str) = passed_part.split_whitespace().last()
                        && let Ok(v) = n_str.parse::<usize>()
                    {
                        count += v;
                    }
                }
            }
        }
    }
    count
}

fn extract_patterns(output: &str, patterns: &[&str]) -> Vec<String> {
    let mut results = Vec::new();
    for line in output.lines() {
        for pat in patterns {
            if line.contains(pat) {
                let trimmed = line.trim().to_string();
                if !trimmed.is_empty() && !results.contains(&trimmed) {
                    results.push(trimmed);
                }
                break;
            }
        }
    }
    results
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.len() <= max_chars {
        s.to_string()
    } else {
        format!("{}…[truncated, {} bytes total]", &s[..max_chars], s.len())
    }
}
pub(super) fn classify_dry_run_error(msg: &str) -> String {
    if msg.contains("disk") || msg.contains("ENOSPC") {
        "infra_blocked".to_string()
    } else if msg.contains("compiler") || msg.contains("cargo") || msg.contains("rustc") {
        "candidate_compile_failed".to_string()
    } else if msg.contains("worktree") || msg.contains("git") {
        "infra_blocked".to_string()
    } else if msg.contains("validation command failed") {
        "validation_failed".to_string()
    } else if msg.contains("patch does not apply") {
        "candidate_compile_failed".to_string()
    } else {
        "validation_failed".to_string()
    }
}

pub fn classify_validation_failure(vr: &ValidationRecord) -> String {
    // Infrastructure classification must be supported by concrete evidence.
    let stderr = &vr.stderr_preview;
    let lower = stderr.to_lowercase();
    if lower.contains("disk full") || lower.contains("enospc") {
        return "infra_blocked".to_string();
    }
    if lower.contains("no space left") {
        return "infra_blocked".to_string();
    }
    if lower.contains("compiler not found") || lower.contains("cargo: not found") {
        return "infra_blocked".to_string();
    }

    // Compilation failures are NOT infrastructure.
    if stderr.contains("error[") || stderr.contains("could not compile") {
        return "candidate_compile_failed".to_string();
    }

    // Test failures.
    if !vr.failures.is_empty() {
        return "candidate_test_failed".to_string();
    }

    // Validation command failure (non-zero exit, no specific classification).
    if vr.exit_code.map(|c| c != 0).unwrap_or(false) {
        return "validation_failed".to_string();
    }

    "validation_failed".to_string()
}

pub(super) fn failure_to_terminal_state(classification: &str) -> EvaluationState {
    match classification {
        "preflight_blocked" => EvaluationState::PreflightBlocked,
        "generation_failed" => EvaluationState::GenerationFailed,
        "governance_rejected" => EvaluationState::GovernanceRejected,
        "candidate_compile_failed" => EvaluationState::CandidateCompileFailed,
        "candidate_test_failed" => EvaluationState::CandidateTestFailed,
        "validation_failed" => EvaluationState::ValidationFailed,
        "infra_blocked" => EvaluationState::InfraBlocked,
        "integrity_failed" => EvaluationState::IntegrityFailed,
        "validation_passed_review_required" => EvaluationState::ReviewGate,
        _ => EvaluationState::InternalError,
    }
}

/// Async, platform-aware shell for validation commands. On unix the child is
/// placed in its own process group so the entire subtree can be signalled when
/// the run is cancelled.
fn validation_shell_async(command: &str) -> AsyncCommand {
    #[cfg(windows)]
    {
        let mut cmd = AsyncCommand::new("cmd");
        cmd.arg("/C").arg(command);
        cmd
    }
    #[cfg(not(windows))]
    {
        let mut cmd = AsyncCommand::new("sh");
        cmd.arg("-c").arg(command);
        // Own process group so cancellation can signal the whole subtree.
        cmd.process_group(0);
        cmd
    }
}

/// Terminate a validation child process tree given its pid, and wait a moment
/// for the kill to take effect.
///
/// On unix the child was spawned in its own process group, so signalling the
/// negative pid reaches the shell and every descendant. On windows `taskkill
/// /T` kills the tree. `kill_on_drop` on the spawned child cleans up the
/// `Child` handle itself once its waiter task is aborted.
async fn kill_child_tree(pid: u32) {
    #[cfg(unix)]
    {
        // SAFETY: kill with SIGKILL on a process group we just created.
        unsafe {
            let _ = libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .output();
    }
    // Give the OS a brief moment to reap the tree before the caller continues.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classify_dry_run_error_compile() {
        assert_eq!(
            classify_dry_run_error("compiler error"),
            "candidate_compile_failed"
        );
    }
    #[test]
    fn classify_dry_run_error_infra() {
        assert_eq!(
            classify_dry_run_error("disk full during worktree"),
            "infra_blocked"
        );
    }
    #[test]
    fn classify_validation_failure_compile_error() {
        let vr = ValidationRecord {
            validation_command: None,
            exit_code: Some(1),
            stdout_preview: String::new(),
            stderr_preview: "error[E0308]: could not compile".to_string(),
            start_time: String::new(),
            completion_time: String::new(),
            test_discovered: false,
            test_executed: false,
            test_names_found: Vec::new(),
            test_count: 0,
            warnings: Vec::new(),
            failures: Vec::new(),
            patch_applies_cleanly: true,
            validation_passed: false,
        };
        assert_eq!(classify_validation_failure(&vr), "candidate_compile_failed");
    }
    #[test]
    fn classify_validation_failure_not_infra() {
        let vr = ValidationRecord {
            validation_command: None,
            exit_code: Some(1),
            stdout_preview: String::new(),
            stderr_preview: "assertion `left == right` failed".to_string(),
            start_time: String::new(),
            completion_time: String::new(),
            test_discovered: false,
            test_executed: false,
            test_names_found: Vec::new(),
            test_count: 0,
            warnings: Vec::new(),
            failures: vec!["assertion failed".to_string()],
            patch_applies_cleanly: true,
            validation_passed: false,
        };
        assert_ne!(classify_validation_failure(&vr), "infra_blocked");
    }
    #[test]
    fn parse_test_evidence_rust_output() {
        let stdout = "running 1\ntest tests::it_works ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored";
        let (discovered, executed, names, count, _warnings, _failures) =
            parse_test_evidence(stdout, "", Some(0));
        assert!(discovered);
        assert!(executed);
        assert!(names.contains(&"tests::it_works".to_string()));
        assert_eq!(count, 1);
    }
    #[test]
    fn parse_test_evidence_no_tests() {
        let (discovered, _executed, names, count, _, _) =
            parse_test_evidence("no tests here", "", Some(0));
        assert!(!discovered);
        assert!(!names.is_empty() || !discovered);
        assert_eq!(count, 0);
    }
    #[test]
    fn truncate_short() {
        assert_eq!(truncate("hello", 10), "hello");
    }
    #[test]
    fn truncate_long() {
        let s = "a".repeat(100);
        let t = truncate(&s, 10);
        assert!(t.len() < 100);
        assert!(t.contains("truncated"));
    }
    #[test]
    fn test_extract_test_names() {
        let output = "test foo::bar ... ok\ntest baz::qux ... FAILED";
        let names = extract_test_names(output);
        assert!(names.contains(&"foo::bar".to_string()));
        assert!(names.contains(&"baz::qux".to_string()));
    }
    #[test]
    fn test_count_tests_from_output() {
        let output = "test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out";
        assert_eq!(count_tests_from_output(output), 5);
    }
    #[test]
    fn failure_to_terminal_state_mapping() {
        assert_eq!(
            failure_to_terminal_state("infra_blocked"),
            EvaluationState::InfraBlocked
        );
        assert_eq!(
            failure_to_terminal_state("candidate_compile_failed"),
            EvaluationState::CandidateCompileFailed
        );
        assert_eq!(
            failure_to_terminal_state("validation_passed_review_required"),
            EvaluationState::ReviewGate
        );
    }
}
