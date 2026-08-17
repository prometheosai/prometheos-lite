use anyhow::{Context, Result, anyhow, bail};
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use tokio::io::AsyncReadExt;
use tokio::process::Command as AsyncCommand;

use crate::workflow::redaction::Redactor;

use super::cancellation::CancellationToken;
use super::evidence::ValidationRecord;
use super::generation::load_proposal_from_repo;
use super::identity::{EvaluationState, now_iso};
use super::integrity::run_git_cmd;
use super::resource::{
    CLASSIFICATION_CPU, CLASSIFICATION_DISK, CLASSIFICATION_MEMORY, CLASSIFICATION_OUTPUT,
    CLASSIFICATION_TIMEOUT, ResourceLimits,
};

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
    resource_limits: &ResourceLimits,
    known_secrets: &[String],
) -> Result<ValidationRecord> {
    let proposal = load_proposal_from_repo(repo, proposal_id)?;
    let start_time = now_iso();

    // Diagnostics (including command, stdout, stderr) are redacted at the
    // persistence boundary. The command is still *executed* verbatim.
    let redactor = Redactor::new().with_known_secrets(known_secrets);

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
            validation_command: validation_command.map(|s| redactor.redact(s)),
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

    // Step 2: Run validation command if present — cooperatively cancellable and
    // resource-bounded (timeout + output cap). Cancellation wins over resource
    // violation (it is a control-flow signal, not a failure).
    let (exit_code, stdout, stderr) = match validation_command {
        Some(cmd) => {
            let (code, raw_out, raw_err) =
                bounded_run(cmd, &wt_root, resource_limits, token).await?;
            // Redact diagnostics before they become persisted evidence.
            let out = redactor.redact(&raw_out);
            let err = redactor.redact(&raw_err);
            (code, out, err)
        }
        None => (None, String::new(), String::new()),
    };

    let completion_time = now_iso();

    // Discover tests from output.
    let (test_discovered, test_executed, test_names, test_count, warnings, failures) =
        parse_test_evidence(&stdout, &stderr, exit_code);

    let validation_passed = exit_code.map(|c| c == 0).unwrap_or(true) && patch_applies;

    // Save raw logs (redacted, with checksum sidecar).
    let stdout_path = evidence_dir.join("validation_stdout.log");
    let stderr_path = evidence_dir.join("validation_stderr.log");
    crate::workflow::artifact_integrity::publish_with_integrity(
        repo,
        &stdout_path,
        stdout.as_bytes(),
        crate::workflow::artifact_integrity::ArtifactKind::RawLog,
    )
    .context("failed to write redacted validation stdout log")?;
    crate::workflow::artifact_integrity::publish_with_integrity(
        repo,
        &stderr_path,
        stderr.as_bytes(),
        crate::workflow::artifact_integrity::ArtifactKind::RawLog,
    )
    .context("failed to write redacted validation stderr log")?;

    // Clean up worktree.
    let _ = run_git_cmd(
        repo,
        &["worktree", "remove", "--force", wt_root.to_str().unwrap()],
    );
    let _ = std::fs::remove_dir_all(&wt_root);
    let _ = std::fs::remove_file(&patch_file);

    Ok(ValidationRecord {
        validation_command: validation_command.map(|s| redactor.redact(s)),
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

/// Run `command` in `cwd` as one shell expression, capturing stdout/stderr with
/// a bounded total budget and an optional wall-clock timeout. The entire
/// process tree is terminated on cancellation, timeout, or output overflow, and
/// the run fails closed with a `resource_*` classification.
///
/// Returns the exit code and the (still unredacted) captured output. Redaction
/// happens after this returns, so the executed command is never altered.
async fn bounded_run(
    command: &str,
    cwd: &Path,
    limits: &ResourceLimits,
    token: &CancellationToken,
) -> Result<(Option<i32>, String, String)> {
    let mut child = validation_shell_async(command)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("failed to execute validation command")?;
    let pid = child.id();
    let stdout = child
        .stdout
        .take()
        .context("validation stdout pipe unavailable")?;
    let stderr = child
        .stderr
        .take()
        .context("validation stderr pipe unavailable")?;

    let total = Arc::new(AtomicU64::new(0));
    let exceeded = Arc::new(AtomicBool::new(false));
    let cap = limits.max_output_bytes;

    // Bounded reader: accumulate up to a hard safety cap, and flag (and kill)
    // when the configured budget is exceeded.
    async fn read_bounded(
        stream: impl AsyncReadExt + Unpin,
        total: Arc<AtomicU64>,
        exceeded: Arc<AtomicBool>,
        cap: Option<u64>,
        pid: Option<u32>,
    ) -> String {
        let mut stream = stream;
        let mut s = String::new();
        let mut buf = [0u8; 8192];
        loop {
            match stream.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    if let Some(c) = cap {
                        let cur = total.fetch_add(n as u64, Ordering::SeqCst) + n as u64;
                        if cur > c && !exceeded.swap(true, Ordering::SeqCst) {
                            // Stop the workload; the main select observes
                            // `exceeded` once the process tree exits.
                            if let Some(pid) = pid {
                                kill_child_tree(pid).await;
                            }
                        }
                    }
                    // Never grow past a hard safety cap regardless of config.
                    if s.len() < 64 * 1024 * 1024 {
                        s.push_str(&String::from_utf8_lossy(&buf[..n]));
                    }
                }
                Err(_) => break,
            }
        }
        s
    }

    let out_task = {
        let total = total.clone();
        let exceeded = exceeded.clone();
        tokio::spawn(read_bounded(stdout, total, exceeded, cap, pid))
    };
    let err_task = {
        let total = total.clone();
        let exceeded = exceeded.clone();
        tokio::spawn(read_bounded(stderr, total, exceeded, cap, pid))
    };

    // Race the process exit against cancellation and the wall-clock timeout.
    let timeout_arm = limits.validation_timeout.map(|t| tokio::time::sleep(t));

    tokio::select! {
        status = child.wait() => {
            let code = status
                .context("failed to wait on validation command")?
                .code();
            let out = out_task.await.unwrap_or_default();
            let err = err_task.await.unwrap_or_default();
            if exceeded.load(Ordering::SeqCst) {
                bail!(
                    "{}: validation output exceeded the configured cap",
                    CLASSIFICATION_OUTPUT
                );
            }
            Ok((code, out, err))
        }
        _ = token.cancelled() => {
            if let Some(pid) = pid {
                kill_child_tree(pid).await;
            }
            out_task.abort();
            err_task.abort();
            Err(anyhow!("validation cancelled by user request"))
        }
        _ = maybe_timeout(timeout_arm), if timeout_arm.is_some() => {
            if let Some(pid) = pid {
                kill_child_tree(pid).await;
            }
            out_task.abort();
            err_task.abort();
            bail!(
                "{}: validation exceeded the configured wall-clock timeout",
                CLASSIFICATION_TIMEOUT
            );
        }
    }
}

/// Helper that awaits an optional timeout future (resolves immediately to
/// `()` when `None`). Used to conditionally arm the timeout select branch.
async fn maybe_timeout(f: Option<tokio::time::Sleep>) {
    if let Some(s) = f {
        s.await;
    }
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
    if msg.starts_with(CLASSIFICATION_TIMEOUT)
        || msg.starts_with(CLASSIFICATION_OUTPUT)
        || msg.starts_with(CLASSIFICATION_CPU)
        || msg.starts_with(CLASSIFICATION_MEMORY)
        || msg.starts_with(CLASSIFICATION_DISK)
    {
        // Resource exhaustion is infrastructure failure.
        msg.split(':').next().unwrap_or("infra_blocked").to_string()
    } else if msg.contains("disk") || msg.contains("ENOSPC") {
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
        CLASSIFICATION_TIMEOUT
        | CLASSIFICATION_OUTPUT
        | CLASSIFICATION_CPU
        | CLASSIFICATION_MEMORY
        | CLASSIFICATION_DISK => EvaluationState::InfraBlocked,
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

    // Leak-safety gate: a known secret echoed by the validation command must
    // never reach persisted diagnostics or raw logs. This drives the real
    // `run_isolated_validation` persistence path.
    #[tokio::test]
    async fn known_secret_is_redacted_from_persisted_validation() {
        use crate::workflow::artifact_integrity::{ArtifactKind, publish_with_integrity};
        use crate::workflow::evaluate::cancellation::CancellationToken;
        use crate::workflow::evaluate::integrity::run_git_cmd;
        use crate::workflow::redaction::SECRET_CANARY;
        use crate::workflow::{AuthorityLevel, ProposalArtifact, ScopeContract};

        let dir = std::env::temp_dir().join(format!("prometheos-canary-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        run_git_cmd(&dir, &["init"]).unwrap();
        run_git_cmd(&dir, &["config", "user.email", "t@example.com"]).unwrap();
        run_git_cmd(&dir, &["config", "user.name", "t"]).unwrap();
        std::fs::write(dir.join("seed.txt"), "x").unwrap();
        run_git_cmd(&dir, &["add", "."]).unwrap();
        run_git_cmd(&dir, &["commit", "-m", "init"]).unwrap();
        let base_sha = run_git_cmd(&dir, &["rev-parse", "HEAD"]).unwrap().trim().to_string();

        let id = "canary-run";
        let proposal = ProposalArtifact {
            id: id.to_string(),
            repo: dir.to_string_lossy().to_string(),
            base_sha: base_sha.clone(),
            goal: "g".to_string(),
            authority: AuthorityLevel::Propose,
            scope: ScopeContract {
                goal: "g".to_string(),
                authority: AuthorityLevel::Propose,
                allowed_paths: vec![],
                forbidden_paths: vec![],
                allow_dependency_changes: false,
                max_files_changed: None,
                max_lines_changed: None,
            },
            patch: "diff --git a/new.txt b/new.txt\nnew file mode 100644\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+hello\n".to_string(),
            patch_hash: "h".to_string(),
            changed_files: vec![],
            added_lines: 0,
            removed_lines: 0,
            approved: None,
            dry_run_passed: None,
            applied: None,
            validation_command: None,
            provider_provenance: None,
            dry_run_validation: None,
            apply_validation: None,
            checkpoint_ref: None,
            rollback_status: None,
        };
        let prop_path = dir
            .join(".prometheos")
            .join("workflow")
            .join(id)
            .join("proposal.json");
        std::fs::create_dir_all(prop_path.parent().unwrap()).unwrap();
        let bytes = serde_json::to_vec(&proposal).unwrap();
        publish_with_integrity(&dir, &prop_path, &bytes, ArtifactKind::Proposal).unwrap();

        let evidence_dir = dir.join(".prometheos").join("workflow").join(id);
        std::fs::create_dir_all(&evidence_dir).unwrap();

        let token = CancellationToken::new();
        let canary = SECRET_CANARY.to_string();
        let known = vec![canary.clone()];
        let record = run_isolated_validation(
            &dir,
            id,
            Some(&format!("echo {canary}")),
            &evidence_dir,
            &token,
            &ResourceLimits::default(),
            &known,
        )
        .await
        .expect("validation run should succeed");

        assert!(
            record.patch_applies_cleanly,
            "patch must apply for the validation command to run"
        );
        assert!(
            !record.stdout_preview.contains(&canary),
            "stdout preview leaked the known secret"
        );
        assert!(
            !record
                .validation_command
                .as_deref()
                .unwrap_or("")
                .contains(&canary),
            "validation command leaked the known secret"
        );

        let raw = std::fs::read_to_string(evidence_dir.join("validation_stdout.log"))
            .expect("raw stdout log must be persisted");
        assert!(
            !raw.contains(&canary),
            "raw stdout log leaked the known secret"
        );
    }
}
