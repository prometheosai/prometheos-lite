use anyhow::{Context, Result, anyhow, bail};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

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

    // Fail closed on absurd or invalid resource configuration. A misconfigured
    // limit must never silently weaken enforcement.
    resource_limits
        .validate()
        .context("invalid resource limits configuration")?;

    // Fail closed on patch tampering: the applied patch MUST match the recorded
    // patch hash (and any recorded approval hash). This rejects a swapped or
    // mutated patch before it ever touches the worktree.
    verify_patch_integrity(&proposal)?;

    // Reject patches that embed a known secret. Secrets must never be written
    // into a proposed patch; this fails closed before the patch touches the
    // worktree.
    verify_patch_free_of_secrets(&proposal, known_secrets)?;

    // Disk-pressure preflight: refuse to run if the filesystem hosting the repo
    // cannot satisfy the configured free-space reserve. Fails closed.
    if let Some(required) = resource_limits.min_free_disk_bytes {
        match super::preflight::available_disk_bytes(repo) {
            super::preflight::DiskSpaceStatus::Available(free) if free >= required => {}
            super::preflight::DiskSpaceStatus::Available(free) => bail!(
                "{}: disk pressure: {} free bytes available < {} required",
                CLASSIFICATION_DISK,
                free,
                required
            ),
            _ => bail!(
                "{}: could not determine free disk space before validation",
                CLASSIFICATION_DISK
            ),
        }
    }

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
        // Patch doesn't apply â€” record and clean up.
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
            failure_classification: None,
            resource_kind: None,
            configured_limit: None,
            observed_value: None,
            stage: None,
            event_timestamp: None,
        });
    }

    // Apply the patch.
    let _ = run_git_cmd(&wt_root, &["apply", patch_file.to_str().unwrap()]);

    // Step 2: Run validation command if present â€” cooperatively cancellable and
    // resource-bounded (timeout + output cap + aggregate CPU/memory/disk). A
    // resource breach returns a durable, classified `ValidationRecord` carrying the
    // captured (redacted) diagnostics; cancellation is a control-flow signal, not a
    // failure and is surfaced as a fatal error.
    let (exit_code, stdout, stderr) = match validation_command {
        Some(cmd) => match bounded_run(cmd, &wt_root, resource_limits, token).await {
            Ok((code, raw_out, raw_err)) => {
                // Enforce the output cap before persisting anything unbounded.
                check_output_budget(&raw_out, &raw_err, resource_limits)?;
                // Redact diagnostics before they become persisted evidence.
                let out = redactor.redact(&raw_out);
                let err = redactor.redact(&raw_err);
                (code, out, err)
            }
            Err(e) => match *e {
                // Build a durable, classified record with the captured (redacted)
                // diagnostics and typed resource evidence, then return it as a
                // completed failure so recovery never re-derives the classification.
                BoundedRunError::ResourceExceeded(re) => {
                    let out = redactor.redact(&re.stdout);
                    let err = redactor.redact(&re.stderr);
                    let completion_time = now_iso();
                    let base = ValidationRecord::resource_failure(
                        validation_command.map(|s| redactor.redact(s)),
                        re.classification,
                        &format!(
                            "{}: validation breached {} budget",
                            re.classification,
                            re.kind.unwrap_or("resource")
                        ),
                        start_time.clone(),
                        completion_time.clone(),
                        re.kind,
                        re.configured_limit.as_deref(),
                        re.observed_value.as_deref(),
                        Some(re.stage),
                    );
                    let rec = ValidationRecord {
                        stdout_preview: truncate(&out, 4096),
                        stderr_preview: truncate(&err, 4096),
                        failure_classification: Some(re.classification.to_string()),
                        exit_code: re.code,
                        ..base
                    };
                    // Persist the redacted raw logs so recovery/audit can inspect them.
                    let _ = crate::workflow::artifact_integrity::publish_with_integrity(
                        repo,
                        &evidence_dir.join("validation_stdout.log"),
                        out.as_bytes(),
                        crate::workflow::artifact_integrity::ArtifactKind::RawLog,
                    );
                    let _ = crate::workflow::artifact_integrity::publish_with_integrity(
                        repo,
                        &evidence_dir.join("validation_stderr.log"),
                        err.as_bytes(),
                        crate::workflow::artifact_integrity::ArtifactKind::RawLog,
                    );
                    // Clean up the validation worktree before returning.
                    let _ = run_git_cmd(
                        repo,
                        &["worktree", "remove", "--force", wt_root.to_str().unwrap()],
                    );
                    let _ = std::fs::remove_dir_all(&wt_root);
                    let _ = std::fs::remove_file(&patch_file);
                    return Ok(rec);
                }
                BoundedRunError::Fatal(e) => return Err(e),
            },
        },
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
        failure_classification: None,
        resource_kind: None,
        configured_limit: None,
        observed_value: None,
        stage: None,
        event_timestamp: None,
    })
}

/// Run `command` in `cwd` as one shell expression, capturing stdout/stderr with
/// a bounded total budget and an optional wall-clock timeout. The entire
/// process tree is terminated on cancellation, timeout, or output overflow, and
/// the run fails closed with a `resource_*` classification.
///
/// Returns the exit code and the (still unredacted) captured output. Redaction
/// happens after this returns, so the executed command is never altered.
/// Authoritative resource-breach evidence produced by [`bounded_run`]. Carries the
/// captured (still-unredacted) stdout/stderr so the caller can durably persist
/// redacted diagnostics instead of discarding them on a resource failure.
pub(super) struct ResourceExceeded {
    pub classification: &'static str,
    pub kind: Option<&'static str>,
    pub configured_limit: Option<String>,
    pub observed_value: Option<String>,
    pub stage: &'static str,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Error returned by [`bounded_run`]: either a resource breach (with typed
/// evidence) or a fatal control-flow error (cancellation, spawn failure, disk
/// preflight). The resource variant is matched by the caller so it can build a
/// durable, classified `ValidationRecord`.
pub(super) enum BoundedRunError {
    ResourceExceeded(ResourceExceeded),
    Fatal(anyhow::Error),
}

/// `bounded_run`'s result type: the error is boxed because the resource variant
/// carries captured diagnostics and would otherwise make every result frame
/// carry a very large `Err` payload (clippy::result_large_err).
pub(super) type BoundedRunResult = Result<(Option<i32>, String, String), Box<BoundedRunError>>;

impl From<anyhow::Error> for Box<BoundedRunError> {
    fn from(e: anyhow::Error) -> Self {
        Box::new(BoundedRunError::Fatal(e))
    }
}

async fn bounded_run(
    command: &str,
    cwd: &Path,
    limits: &ResourceLimits,
    token: &CancellationToken,
) -> BoundedRunResult {
    let mut cmd = validation_shell_async(command);
    cmd.current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // OS-enforced CPU cap (kernel hard limit on the validated command
    // process): POSIX `RLIMIT_CPU` with soft < hard. The kernel delivers
    // SIGXCPU at the soft limit and the default disposition terminates the
    // child — a deterministic, attributable verdict (`stage = "rlimit_cpu"`).
    // The hard limit sits above the soft one purely as a backstop; SIGKILL
    // and shell-relayed exit 137 are deliberately NOT attributed to the cap,
    // because ordinary failures, external kills, and OOM would be
    // indistinguishable from it.
    //
    // `RLIMIT_CPU` is per process (each descendant receives its own
    // allowance), so the AGGREGATE tree budget is enforced and classified by
    // the monitor below. Memory/disk deliberately have NO child-side rlimit:
    // `RLIMIT_AS` makes large allocations fail with an ordinary non-zero exit
    // (no classifiable signal, and on macOS it fails outright).
    #[cfg(unix)]
    {
        let cpu = limits.max_cpu_time.map(|d| d.as_secs());
        // SAFETY: `pre_exec` runs in the child immediately after fork and before
        // exec; we only call async-signal-safe libc setters. If the configured
        // cap cannot be applied the spawn fails (fail closed: a resource cap we
        // cannot enforce must not run unbounded).
        unsafe {
            cmd.pre_exec(move || {
                if let Some(secs) = cpu {
                    let rlim = libc::rlimit {
                        rlim_cur: secs,
                        rlim_max: secs + 5, // soft fires SIGXCPU first; hard is a backstop
                    };
                    if libc::setrlimit(libc::RLIMIT_CPU, &rlim) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        // Start the shell SUSPENDED so no descendant can be created before the
        // Job Object is assigned: children inherit the job, so anything spawned
        // after assignment is covered by tree-wide termination, and a fast
        // pre-assignment grandchild (the spawn/assign race) cannot exist.
        const CREATE_SUSPENDED: u32 = 0x0000_0004;
        cmd.creation_flags(CREATE_SUSPENDED);
    }
    let mut child = cmd
        .spawn()
        .context("failed to execute validation command")?;
    let pid = child.id();
    // OS-enforced memory/CPU caps via a Job Object on Windows (aggregate across the
    // whole process tree). Fail closed: if limits are configured but cannot be
    // applied, the validation must not run unbounded. The job handle is retained
    // so the aggregate monitor can poll it; the monitor is the authoritative
    // resource-breach detector on Windows (it sets `monitor_kind` precisely and
    // never infers a breach from an ordinary non-zero exit code).
    let monitor_done = Arc::new(AtomicBool::new(false));
    let monitor_kind = Arc::new(AtomicU8::new(0));
    let mut monitor_handle: Option<std::thread::JoinHandle<()>> = None;

    #[cfg(windows)]
    {
        if (limits.max_memory_bytes.is_some() || limits.max_cpu_time.is_some())
            && let Some(win_pid) = pid
        {
            let job = match apply_job_limits(win_pid, limits) {
                Ok(job) => job,
                Err(e) => {
                    let err = e.context(
                        "failed to apply Windows Job Object resource limits (CPU/memory enforcement unavailable)",
                    );
                    // Fail closed. The shell is still suspended (it never ran,
                    // so it has no descendants); kill it synchronously â€” never
                    // across an await while the raw job-pointer scrutinee is
                    // live, which would make this future !Send.
                    let _ = std::process::Command::new("taskkill")
                        .args(["/F", "/PID", &win_pid.to_string()])
                        .output();
                    return Err(Box::new(BoundedRunError::Fatal(err)));
                }
            };
            // Ownership of the job handle is transferred to the monitor thread,
            // which closes it on exit. The async future therefore never holds a
            // raw pointer across an await (keeps it `Send` for `tokio::spawn`).
            monitor_handle = Some(spawn_resource_monitor_win(
                job as isize,
                *limits,
                monitor_done.clone(),
                monitor_kind.clone(),
            ));
        }
        // The shell was created suspended; every descendant it spawns from now
        // on inherits the already-assigned Job Object, so tree-wide termination
        // cannot be escaped by a pre-assignment grandchild.
        //
        // Fail-closed on resume failure: a child left suspended would burn the
        // whole wall-clock timeout and be misrecorded as a timeout breach.
        // Stop the monitor first (its thread closes the Job handle), kill the
        // still-suspended tree, and surface the error.
        if let Some(win_pid) = pid
            && let Err(e) = resume_suspended_process_windows(win_pid)
        {
            let err = e.context("failed to resume suspended validation process");
            monitor_done.store(true, Ordering::SeqCst);
            if let Some(h) = monitor_handle.take() {
                let _ = h.join();
            }
            let _ = std::process::Command::new("taskkill")
                .args(["/F", "/T", "/PID", &win_pid.to_string()])
                .output();
            return Err(Box::new(BoundedRunError::Fatal(err)));
        }
    }

    // Aggregate, process-tree resource enforcement on Unix: a monitor thread walks
    // the validation process subtree and kills the entire tree the moment the
    // aggregate CPU time, aggregate RSS, or free disk space crosses a budget. This
    // is true process-tree accounting AND the authoritative breach classifier: it
    // sets `monitor_kind` precisely (cpu/memory/disk) so the caller can persist
    // typed durable evidence instead of inferring a breach from an exit code.
    #[cfg(unix)]
    {
        if (limits.max_cpu_time.is_some()
            || limits.max_memory_bytes.is_some()
            || limits.min_free_disk_bytes.is_some())
            && let Some(pid) = pid
        {
            monitor_handle = Some(spawn_resource_monitor(
                pid,
                *limits,
                cwd.to_path_buf(),
                monitor_done.clone(),
                monitor_kind.clone(),
            ));
        }
    }
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
    #[cfg(unix)]
    let killed_by_sigxcpu = Arc::new(AtomicBool::new(false));
    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;

    let status_result: Result<Option<i32>, Box<BoundedRunError>> = tokio::select! {
        status = child.wait() => {
            let status = status.context("failed to wait on validation command")?;
            // SIGXCPU at the soft RLIMIT_CPU limit is the ONLY unambiguous,
            // attributable kernel CPU-cap verdict. Ordinary signals (SIGKILL,
            // OOM, external kills) and shell-relayed exit codes fall through
            // untouched so they can never be misclassified as CPU exhaustion.
            #[cfg(unix)]
            if limits.max_cpu_time.is_some()
                && let Some(sig) = status.signal()
                && sig == libc::SIGXCPU
            {
                killed_by_sigxcpu.store(true, Ordering::SeqCst);
            }
            Ok(status.code())
        }
        _ = token.cancelled() => {
            if let Some(pid) = pid {
                kill_child_tree(pid).await;
            }
            Err(Box::new(BoundedRunError::Fatal(anyhow!(
                "validation cancelled by user request"
            ))))
        }
        _ = maybe_timeout(timeout_arm), if timeout_arm.is_some() => {
            if let Some(pid) = pid {
                kill_child_tree(pid).await;
            }
            let secs = limits.validation_timeout.map(|d| d.as_secs()).unwrap_or(0);
            Err(Box::new(BoundedRunError::ResourceExceeded(ResourceExceeded {
                classification: CLASSIFICATION_TIMEOUT,
                kind: Some("timeout"),
                configured_limit: Some(format!("{}s", secs)),
                observed_value: None,
                stage: "wall_clock_timeout",
                code: None,
                stdout: String::new(),
                stderr: String::new(),
            })))
        }
    };

    // Stop the aggregate monitor and collect its verdict.
    monitor_done.store(true, Ordering::SeqCst);
    if let Some(h) = monitor_handle {
        let _ = h.join();
    }
    let monitor_kind = monitor_kind.load(Ordering::SeqCst);

    // Drain the bounded readers so partial output survives resource breaches and
    // timeouts (the pipes close once the tree is killed). Diagnostics are
    // redacted later by the caller before they become persisted evidence.
    let out = out_task.await.unwrap_or_default();
    let err = err_task.await.unwrap_or_default();

    if monitor_kind != 0 {
        // The aggregate monitor caught a process-tree breach; report it precisely
        // (cpu/memory/disk) with the captured diagnostics so recovery maps it to
        // InfraBlocked and the durable record carries typed evidence.
        let (cls, kind, configured, observed) = match monitor_kind {
            1 => (
                CLASSIFICATION_CPU,
                Some("cpu"),
                limits.max_cpu_time.map(|d| format!("{}s", d.as_secs())),
                Some(format!(
                    ">={}s aggregate",
                    limits.max_cpu_time.unwrap_or_default().as_secs()
                )),
            ),
            2 => (
                CLASSIFICATION_MEMORY,
                Some("memory"),
                limits.max_memory_bytes.map(|b| b.to_string()),
                Some(format!(
                    ">={}b aggregate",
                    limits.max_memory_bytes.unwrap_or_default()
                )),
            ),
            3 => (
                CLASSIFICATION_DISK,
                Some("disk"),
                limits.min_free_disk_bytes.map(|b| b.to_string()),
                Some("free space below reserve".to_string()),
            ),
            _ => (CLASSIFICATION_OUTPUT, Some("resource"), None, None),
        };
        return Err(Box::new(BoundedRunError::ResourceExceeded(
            ResourceExceeded {
                classification: cls,
                kind,
                configured_limit: configured,
                observed_value: observed,
                stage: "aggregate_monitor",
                code: None,
                stdout: out,
                stderr: err,
            },
        )));
    }

    // Kernel-enforced CPU verdict: the DIRECT child died by SIGXCPU at the
    // soft RLIMIT_CPU limit. Deterministic, attributable kernel evidence —
    // never inferred from SIGKILL or exit codes (those stay ordinary
    // failures). Aggregate tree CPU is enforced/classified by the monitor.
    #[cfg(unix)]
    if limits.max_cpu_time.is_some() && killed_by_sigxcpu.load(Ordering::SeqCst) {
        return Err(Box::new(BoundedRunError::ResourceExceeded(
            ResourceExceeded {
                classification: CLASSIFICATION_CPU,
                kind: Some("cpu"),
                configured_limit: Some(format!(
                    "{}s",
                    limits.max_cpu_time.unwrap_or_default().as_secs()
                )),
                observed_value: Some("SIGXCPU at RLIMIT_CPU soft limit".to_string()),
                stage: "rlimit_cpu",
                code: None,
                stdout: out,
                stderr: err,
            },
        )));
    }

    match status_result {
        Ok(code) => {
            if exceeded.load(Ordering::SeqCst) {
                let cap = limits.max_output_bytes.unwrap_or(0);
                return Err(Box::new(BoundedRunError::ResourceExceeded(
                    ResourceExceeded {
                        classification: CLASSIFICATION_OUTPUT,
                        kind: Some("output"),
                        configured_limit: Some(cap.to_string()),
                        observed_value: Some(format!(">{}b", cap)),
                        stage: "output_cap",
                        code,
                        stdout: out,
                        stderr: err,
                    },
                )));
            }
            Ok((code, out, err))
        }
        Err(e) => match *e {
            // The wall-clock timeout arm is raised before the readers are drained;
            // attach the captured diagnostics so the durable record carries them.
            BoundedRunError::ResourceExceeded(mut re) => {
                re.stdout = out;
                re.stderr = err;
                Err(Box::new(BoundedRunError::ResourceExceeded(re)))
            }
            other => Err(Box::new(other)),
        },
    }
}

/// Helper that awaits an optional timeout future (resolves immediately to
/// `()` when `None`). Used to conditionally arm the timeout select branch.
async fn maybe_timeout(f: Option<tokio::time::Sleep>) {
    if let Some(s) = f {
        s.await;
    }
}

/// Fail closed if the proposal's patch does not match its recorded hash (or the
/// hash recorded in its approval). A mismatch means the patch was swapped or
/// mutated after the proposal/approval was written, and it must never be applied
/// to the validation worktree.
/// Reject validation output that exceeds the configured cap. This is the
/// enforcement counterpart to the preview-only `truncate`: it guarantees we
/// never persist (or feed into evidence) unbounded validation output, and that
/// a runaway command is surfaced as a resource violation rather than silently
/// accepted.
fn check_output_budget(stdout: &str, stderr: &str, limits: &ResourceLimits) -> Result<()> {
    if let Some(cap) = limits.max_output_bytes {
        let total = stdout.len() + stderr.len();
        if total > cap as usize {
            bail!(
                "{}: validation produced {} bytes exceeding cap {}",
                CLASSIFICATION_OUTPUT,
                total,
                cap
            );
        }
    }
    Ok(())
}

/// Reject a proposal whose recorded patch embeds a known secret. This is an
/// independent control from patch-hash integrity: it prevents operator secrets
/// from entering the codebase via a proposed diff, even when the hash matches.
fn verify_patch_free_of_secrets(
    proposal: &crate::workflow::ProposalArtifact,
    known_secrets: &[String],
) -> Result<()> {
    for secret in known_secrets {
        if secret.is_empty() {
            continue;
        }
        if proposal.patch.contains(secret) {
            bail!(
                "patch rejected: contains a known secret; secrets must not be embedded in patches"
            );
        }
    }
    Ok(())
}

fn verify_patch_integrity(proposal: &crate::workflow::ProposalArtifact) -> Result<()> {
    let computed = crate::workflow::artifact_integrity::sha256_hex(proposal.patch.as_bytes());
    if !proposal.patch_hash.is_empty() && proposal.patch_hash != computed {
        bail!(
            "patch integrity verification failed: patch does not match recorded patch_hash \
             (possible tampering or corruption)"
        );
    }
    if let Some(ref approval) = proposal.approved
        && !approval.patch_hash.is_empty()
        && approval.patch_hash != computed
    {
        bail!(
            "patch integrity verification failed: patch does not match approval patch_hash \
             (candidate may be stale or tampered)"
        );
    }
    Ok(())
}

/// Apply OS-enforced CPU/memory caps to a freshly spawned validation process via
/// a Windows Job Object. Fails closed: callers reject the run if limits are
/// configured but cannot be applied, so a resource cap is never silently
/// unenforced.
///
/// Limits are applied at the **job** (process-tree aggregate) level:
/// - `JobMemoryLimit` caps the total committed memory of the whole job
///   (aggregate across all child processes), not per-process.
/// - `PerJobUserTimeLimit` caps the aggregate user-mode CPU time of the job.
///
/// Returns the live job `HANDLE` so the aggregate monitor can poll it. The caller
/// closes the handle after the monitor has joined.
#[cfg(windows)]
fn apply_job_limits(pid: u32, limits: &ResourceLimits) -> Result<*mut winapi::ctypes::c_void> {
    use std::mem;
    use winapi::um::handleapi::CloseHandle;
    use winapi::um::jobapi2::{
        AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject,
    };
    use winapi::um::processthreadsapi::OpenProcess;
    use winapi::um::winnt::{
        JOB_OBJECT_LIMIT_JOB_MEMORY, JOB_OBJECT_LIMIT_JOB_TIME,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        PROCESS_ALL_ACCESS,
    };

    unsafe {
        let job = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null_mut());
        if job.is_null() {
            bail!("CreateJobObjectW failed");
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = mem::zeroed();
        let mut flags: u32 = 0;
        if let Some(mem) = limits.max_memory_bytes {
            // Aggregate job memory limit (total committed memory across the tree).
            info.JobMemoryLimit = mem as usize;
            flags |= JOB_OBJECT_LIMIT_JOB_MEMORY;
        }
        if let Some(cpu) = limits.max_cpu_time {
            // JOBOBJECT BasicLimitInformation.PerJobUserTimeLimit is in 100ns units.
            *info
                .BasicLimitInformation
                .PerJobUserTimeLimit
                .QuadPart_mut() = (cpu.as_secs() * 10_000_000) as i64;
            flags |= JOB_OBJECT_LIMIT_JOB_TIME;
        }
        info.BasicLimitInformation.LimitFlags = flags;
        let ok = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *mut _,
            mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if ok == 0 {
            CloseHandle(job);
            bail!("SetInformationJobObject failed");
        }
        let hproc = OpenProcess(PROCESS_ALL_ACCESS, 0, pid);
        if hproc.is_null() {
            CloseHandle(job);
            bail!("OpenProcess failed");
        }
        let assigned = AssignProcessToJobObject(job, hproc);
        CloseHandle(hproc);
        if assigned == 0 {
            CloseHandle(job);
            bail!("AssignProcessToJobObject failed");
        }
        Ok(job)
    }
}

/// Resume the primary (and any other) threads of a process that was created
/// with `CREATE_SUSPENDED`. The validation shell is spawned suspended so the
/// Job Object can be assigned before it executes a single instruction; this
/// closes the spawn→assign race where a fast grandchild could escape
/// tree-wide job termination.
///
/// Fail-closed: returns the number of threads successfully resumed, and an
/// error when none could be resumed (no matching thread found, OpenThread
/// failure, or `ResumeThread` returning `(u32::MAX)`). A child left
/// suspended would otherwise silently burn the whole wall-clock timeout and
/// be misrecorded as a timeout breach.
#[cfg(windows)]
fn resume_suspended_process_windows(pid: u32) -> Result<usize> {
    use std::mem;
    use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
    use winapi::um::processthreadsapi::{OpenThread, ResumeThread};
    use winapi::um::tlhelp32::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use winapi::um::winnt::THREAD_SUSPEND_RESUME;

    let mut resumed = 0usize;
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snap == INVALID_HANDLE_VALUE {
            bail!("CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD) failed");
        }
        let mut entry: THREADENTRY32 = mem::zeroed();
        entry.dwSize = mem::size_of::<THREADENTRY32>() as u32;
        if Thread32First(snap, &mut entry) == 0 {
            CloseHandle(snap);
            bail!("Thread32First failed");
        }
        loop {
            if entry.th32OwnerProcessID == pid {
                let h = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                if h.is_null() {
                    continue;
                }
                let prev = ResumeThread(h);
                CloseHandle(h);
                if prev == u32::MAX {
                    let err = std::io::Error::last_os_error();
                    CloseHandle(snap);
                    bail!("ResumeThread failed for tid {}: {err}", entry.th32ThreadID);
                }
                resumed += 1;
            }
            if Thread32Next(snap, &mut entry) == 0 {
                break;
            }
        }
        CloseHandle(snap);
    }
    // The freshly-spawned suspended shell has exactly one thread (its primary)
    // before executing anything, so "at least one" is equivalent to "the
    // primary thread was opened and resumed successfully".
    if resumed == 0 {
        bail!("no thread could be resumed for suspended validation process {pid}");
    }
    Ok(resumed)
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
        format!("{}â€¦[truncated, {} bytes total]", &s[..max_chars], s.len())
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
    // Trust an authoritative classification carried by the durable record so
    // recovery/replay never re-derives a resource rejection as a candidate test
    // failure.
    if let Some(ref fc) = vr.failure_classification {
        return fc.clone();
    }
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

/// Spawn the aggregate, process-tree resource monitor for a Unix validation run.
///
/// The monitor enumerates the validation process SUBTREE (parentâ†’child walk from
/// the shell pid; robust to `setpgid` behaviour in containers/CI) and, on each
/// tick, sums the **aggregate** user+system CPU time and **aggregate** RSS across
/// the whole tree. If the aggregate CPU, aggregate RSS, or free disk space crosses
/// the configured budget it kills the entire subtree and records which resource
/// was breached in `kind` (1=cpu, 2=memory, 3=disk). CPU additionally carries a
/// kernel hard cap via the child's `RLIMIT_CPU` (`pre_exec`); memory/disk are
/// enforced by this monitor. Platform-specific enumeration differs (Linux
/// `/proc` vs macOS `proc_pidinfo`).
#[cfg(unix)]
fn spawn_resource_monitor(
    pid: u32,
    limits: ResourceLimits,
    cwd: std::path::PathBuf,
    done: Arc<AtomicBool>,
    kind: Arc<AtomicU8>,
) -> std::thread::JoinHandle<()> {
    #[cfg(target_os = "linux")]
    {
        spawn_resource_monitor_linux(pid, limits, cwd, done, kind)
    }
    #[cfg(target_os = "macos")]
    {
        spawn_resource_monitor_macos(pid, limits, cwd, done, kind)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (pid, limits, cwd, done, kind);
        std::thread::spawn(move || {})
    }
}

#[cfg(target_os = "linux")]
fn spawn_resource_monitor_linux(
    pid: u32,
    limits: ResourceLimits,
    cwd: std::path::PathBuf,
    done: Arc<AtomicBool>,
    kind: Arc<AtomicU8>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let root = pid as i32;
        // Clock ticks per second, for converting /proc stat times to seconds.
        let tick = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        let tick = if tick <= 0 { 100 } else { tick as u64 };
        while !done.load(Ordering::SeqCst) {
            let tree = process_tree_subtree(root);
            let mut cpu_ticks: u64 = 0;
            let mut rss_bytes: u64 = 0;
            for &p in &tree {
                let proc_path = std::path::Path::new("/proc").join(p.to_string());
                if let Ok(stat) = std::fs::read_to_string(proc_path.join("stat"))
                    && let Some((ut, st)) = proc_cpu_ticks(&stat)
                {
                    cpu_ticks += ut + st;
                }
                if let Some(rss) = proc_rss(&proc_path) {
                    rss_bytes += rss;
                }
            }
            if let Some(secs) = limits.max_cpu_time
                && cpu_ticks / tick >= secs.as_secs()
            {
                kind.store(1, Ordering::SeqCst);
                kill_process_tree(root);
                return;
            }
            if let Some(mem) = limits.max_memory_bytes
                && rss_bytes >= mem
            {
                kind.store(2, Ordering::SeqCst);
                kill_process_tree(root);
                return;
            }
            if let Some(min_free) = limits.min_free_disk_bytes
                && let super::preflight::DiskSpaceStatus::Available(free) =
                    super::preflight::available_disk_bytes(&cwd)
                && free < min_free
            {
                kind.store(3, Ordering::SeqCst);
                kill_process_tree(root);
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    })
}

#[cfg(target_os = "macos")]
fn spawn_resource_monitor_macos(
    pid: u32,
    limits: ResourceLimits,
    cwd: std::path::PathBuf,
    done: Arc<AtomicBool>,
    kind: Arc<AtomicU8>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let root = pid as i32;
        while !done.load(Ordering::SeqCst) {
            let tree = process_tree_subtree(root);
            let mut cpu_us: u64 = 0;
            let mut rss_bytes: u64 = 0;
            for &p in &tree {
                let mut task: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
                let sz = unsafe {
                    libc::proc_pidinfo(
                        p,
                        libc::PROC_PIDTASKINFO,
                        0,
                        &mut task as *mut _ as *mut libc::c_void,
                        std::mem::size_of::<libc::proc_taskinfo>() as i32,
                    )
                };
                if sz > 0 {
                    // proc_taskinfo user/system times are in microseconds.
                    cpu_us += task.pti_total_user + task.pti_total_system;
                    rss_bytes += task.pti_resident_size;
                }
            }
            if let Some(secs) = limits.max_cpu_time
                && cpu_us / 1_000_000 >= secs.as_secs()
            {
                kind.store(1, Ordering::SeqCst);
                kill_process_tree(root);
                return;
            }
            if let Some(mem) = limits.max_memory_bytes
                && rss_bytes >= mem
            {
                kind.store(2, Ordering::SeqCst);
                kill_process_tree(root);
                return;
            }
            if let Some(min_free) = limits.min_free_disk_bytes
                && let super::preflight::DiskSpaceStatus::Available(free) =
                    super::preflight::available_disk_bytes(&cwd)
                && free < min_free
            {
                kind.store(3, Ordering::SeqCst);
                kill_process_tree(root);
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    })
}

#[cfg(windows)]
fn spawn_resource_monitor_win(
    job: isize,
    limits: ResourceLimits,
    done: Arc<AtomicBool>,
    kind: Arc<AtomicU8>,
) -> std::thread::JoinHandle<()> {
    use std::mem;
    use winapi::um::handleapi::CloseHandle;
    use winapi::um::jobapi2::{QueryInformationJobObject, TerminateJobObject};
    use winapi::um::winnt::{
        JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
    };
    std::thread::spawn(move || {
        let job_handle = job as *mut winapi::ctypes::c_void;
        while !done.load(Ordering::SeqCst) {
            unsafe {
                // Memory: compare the job's peak committed memory against the limit.
                if limits.max_memory_bytes.is_some() {
                    let mut ext: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = mem::zeroed();
                    let ok = QueryInformationJobObject(
                        job_handle as *mut _,
                        JobObjectExtendedLimitInformation,
                        &mut ext as *mut _ as *mut winapi::ctypes::c_void,
                        mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                        std::ptr::null_mut(),
                    );
                    if ok != 0
                        && ext.JobMemoryLimit > 0
                        && ext.PeakJobMemoryUsed >= ext.JobMemoryLimit
                    {
                        kind.store(2, Ordering::SeqCst);
                        TerminateJobObject(job_handle, 0xC000_0142u32);
                        return;
                    }
                }
                // CPU: compare the job's aggregate user time against the limit.
                if limits.max_cpu_time.is_some() {
                    let mut acct: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = mem::zeroed();
                    let ok = QueryInformationJobObject(
                        job_handle as *mut _,
                        JobObjectBasicAccountingInformation,
                        &mut acct as *mut _ as *mut winapi::ctypes::c_void,
                        mem::size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                        std::ptr::null_mut(),
                    );
                    if ok != 0
                        && let Some(cpu) = limits.max_cpu_time
                        && (*acct.TotalUserTime.QuadPart() as u64) >= cpu.as_secs() * 10_000_000
                    {
                        kind.store(1, Ordering::SeqCst);
                        TerminateJobObject(job_handle, 0xC000_0142u32);
                        return;
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        // Ownership transferred here: close the job handle now that the tree is
        // stopped and the caller has joined.
        unsafe {
            CloseHandle(job_handle);
        }
    })
}

/// Enumerate the validation process subtree rooted at `root` by walking the OS
/// parent->child map. PID-based enumeration is robust to how the shell assigns
/// process groups, which is fragile inside containers/CI, and therefore reliably
/// captures every descendant regardless of `setpgid` behaviour.
#[cfg(unix)]
fn process_tree_subtree(root: i32) -> Vec<i32> {
    #[cfg(target_os = "linux")]
    let children = linux_ppid_map();
    #[cfg(target_os = "macos")]
    let children = macos_ppid_map();
    let mut visited: Vec<i32> = Vec::new();
    let mut queue = vec![root];
    while let Some(p) = queue.pop() {
        if visited.contains(&p) {
            continue;
        }
        visited.push(p);
        if let Some(kids) = children.get(&p) {
            for k in kids {
                queue.push(*k);
            }
        }
    }
    visited
}

#[cfg(target_os = "linux")]
fn linux_ppid_map() -> HashMap<i32, Vec<i32>> {
    let mut children: HashMap<i32, Vec<i32>> = HashMap::new();
    if let Ok(entries) = std::fs::read_dir("/proc") {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            let p = match name.parse::<i32>() {
                Ok(p) => p,
                Err(_) => continue,
            };
            let stat = match std::fs::read_to_string(e.path().join("stat")) {
                Ok(s) => s,
                Err(_) => continue,
            };
            if let Some(pp) = stat_ppid(&stat) {
                children.entry(pp).or_default().push(p);
            }
        }
    }
    children
}

#[cfg(target_os = "macos")]
fn macos_ppid_map() -> HashMap<i32, Vec<i32>> {
    // `proc_listallpids(NULL, 0)` returns the number of PIDs in the table
    // (NOT a byte count). Size the buffer for that many PIDs and pass its byte
    // capacity on the second call; then honor the second call's returned PID
    // count, which can be lower than the probe if processes exited meanwhile.
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        return HashMap::new();
    }
    let mut pids: Vec<i32> = vec![0; count as usize];
    let written = unsafe {
        libc::proc_listallpids(
            pids.as_mut_ptr() as *mut libc::c_void,
            (count as usize)
                .saturating_mul(std::mem::size_of::<i32>())
                .min(i32::MAX as usize) as i32,
        )
    };
    if written <= 0 {
        return HashMap::new();
    }
    let mut children: HashMap<i32, Vec<i32>> = HashMap::new();
    for &p in &pids[..written as usize] {
        if p <= 0 {
            continue;
        }
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let sz = unsafe {
            libc::proc_pidinfo(
                p,
                libc::PROC_PIDTBSDINFO,
                0,
                &mut info as *mut _ as *mut libc::c_void,
                std::mem::size_of::<libc::proc_bsdinfo>() as i32,
            )
        };
        if sz <= 0 {
            continue;
        }
        children.entry(info.pbi_ppid as i32).or_default().push(p);
    }
    children
}

/// Parse the parent pid from a `/proc/<pid>/stat` line (robust to the comm field
/// containing spaces or parentheses).
#[cfg(target_os = "linux")]
fn stat_ppid(stat: &str) -> Option<i32> {
    let idx = stat.rfind(')')?;
    let rest = &stat[idx + 1..];
    let mut it = rest.split_whitespace();
    let _state = it.next();
    it.next().and_then(|p| p.parse::<i32>().ok())
}

/// Kill the whole validation process subtree (rooted at the validation child) by
/// sending SIGKILL to every descendant. Robust to process-group assignment
/// quirks because it enumerates the PID tree directly.
#[cfg(unix)]
fn kill_process_tree(root: i32) {
    for p in process_tree_subtree(root) {
        unsafe {
            let _ = libc::kill(p, libc::SIGKILL);
        }
    }
}

/// Parse the (utime, stime) pair (in clock ticks) from `/proc/<pid>/stat`.
#[cfg(target_os = "linux")]
fn proc_cpu_ticks(stat: &str) -> Option<(u64, u64)> {
    let idx = stat.rfind(')')?;
    let rest = &stat[idx + 1..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    // state(0) ppid(1) pgrp(2) session(3) tty(4) tpgid(5) flags(6) minflt(7)
    // cminflt(8) majflt(9) cmajflt(10) utime(11) stime(12)
    let utime = fields.get(11)?.parse::<u64>().ok()?;
    let stime = fields.get(12)?.parse::<u64>().ok()?;
    Some((utime, stime))
}

/// Parse the resident set size (bytes) from `/proc/<pid>/status` (`VmRSS`).
#[cfg(target_os = "linux")]
fn proc_rss(proc_path: &Path) -> Option<u64> {
    let status = std::fs::read_to_string(proc_path.join("status")).ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb = rest.split_whitespace().next()?.parse::<u64>().ok()?;
            return Some(kb * 1024);
        }
    }
    None
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
            failure_classification: None,
            resource_kind: None,
            configured_limit: None,
            observed_value: None,
            stage: None,
            event_timestamp: None,
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
            failure_classification: None,
            resource_kind: None,
            configured_limit: None,
            observed_value: None,
            stage: None,
            event_timestamp: None,
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
        let base_sha = run_git_cmd(&dir, &["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_string();

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
            patch_hash: crate::workflow::artifact_integrity::sha256_hex(
                b"diff --git a/new.txt b/new.txt\nnew file mode 100644\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+hello\n",
            )
            .to_string(),
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

        // Recursively assert the canary never reaches ANY persisted `.prometheos`
        // file (journal, evidence, raw logs, registry, checkpoint, ...).
        let prom_dir = dir.join(".prometheos");
        let mut leaked = false;
        if prom_dir.exists() {
            let mut stack = vec![prom_dir];
            while let Some(p) = stack.pop() {
                let md = std::fs::metadata(&p).expect("metadata readable");
                if md.is_dir() {
                    for e in std::fs::read_dir(&p).expect("dir readable") {
                        stack.push(e.expect("entry").path());
                    }
                } else if let Ok(content) = std::fs::read_to_string(&p)
                    && content.contains(&canary)
                {
                    leaked = true;
                    eprintln!("LEAK: canary found in {}", p.display());
                }
            }
        }
        assert!(
            !leaked,
            "known secret canary leaked into a persisted .prometheos file"
        );
    }

    // A resource breach must produce a DURABLE, classified ValidationRecord whose
    // typed evidence survives recovery: classify_validation_failure must honor the
    // authoritative classification (InfraBlocked) rather than re-deriving it from
    // the (redacted) stderr as a candidate test failure.
    #[tokio::test]
    async fn resource_failure_record_is_durable_and_maps_to_infra_blocked() {
        use crate::workflow::evaluate::evidence::ValidationRecord;

        let rec = ValidationRecord::resource_failure(
            None,
            CLASSIFICATION_CPU,
            "aggregate cpu time exceeded the configured budget",
            "start".to_string(),
            "end".to_string(),
            Some("cpu"),
            Some("5s"),
            Some(">=5s aggregate"),
            Some("aggregate_monitor"),
        );
        assert_eq!(
            rec.failure_classification.as_deref(),
            Some(CLASSIFICATION_CPU)
        );
        assert_eq!(rec.resource_kind.as_deref(), Some("cpu"));
        assert_eq!(rec.configured_limit.as_deref(), Some("5s"));
        assert_eq!(rec.observed_value.as_deref(), Some(">=5s aggregate"));
        assert_eq!(rec.stage.as_deref(), Some("aggregate_monitor"));
        // Authoritative classification is preserved verbatim across recovery/replay.
        assert_eq!(classify_validation_failure(&rec), CLASSIFICATION_CPU);
        // ...and recovery maps it to InfraBlocked (never a candidate test failure).
        assert_eq!(
            failure_to_terminal_state(CLASSIFICATION_CPU),
            crate::workflow::evaluate::identity::EvaluationState::InfraBlocked
        );
    }

    // The output cap breach must be caught and reported as a classified resource
    // failure carrying typed evidence, independent of the OS, so recovery maps it
    // to InfraBlocked (never a candidate test failure).
    #[tokio::test]
    async fn output_cap_breach_is_durable_and_classified() {
        use crate::workflow::artifact_integrity::{ArtifactKind, publish_with_integrity};
        use crate::workflow::evaluate::cancellation::CancellationToken;
        use crate::workflow::evaluate::integrity::run_git_cmd;
        use crate::workflow::evaluate::resource::ResourceLimits;
        use crate::workflow::{AuthorityLevel, ProposalArtifact, ScopeContract};

        let dir = std::env::temp_dir().join(format!("prometheos-outcap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        run_git_cmd(&dir, &["init"]).unwrap();
        run_git_cmd(&dir, &["config", "user.email", "t@example.com"]).unwrap();
        run_git_cmd(&dir, &["config", "user.name", "t"]).unwrap();
        std::fs::write(dir.join("seed.txt"), "x").unwrap();
        run_git_cmd(&dir, &["add", "."]).unwrap();
        run_git_cmd(&dir, &["commit", "-m", "init"]).unwrap();
        let base_sha = run_git_cmd(&dir, &["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_string();

        let id = "outcap-run";
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
            patch_hash: crate::workflow::artifact_integrity::sha256_hex(
                b"diff --git a/new.txt b/new.txt\nnew file mode 100644\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+hello\n",
            )
            .to_string(),
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

        // Generate a file whose contents far exceed the output cap.
        let big = dir.join("big.txt");
        std::fs::write(&big, "a".repeat(200_000)).unwrap();
        let dump = if cfg!(windows) {
            format!("type {}", big.display())
        } else {
            format!("cat {}", big.display())
        };

        let token = CancellationToken::new();
        let limits = ResourceLimits {
            max_output_bytes: Some(64),
            ..ResourceLimits::default()
        };
        let record =
            run_isolated_validation(&dir, id, Some(&dump), &evidence_dir, &token, &limits, &[])
                .await
                .expect("validation run should complete");

        assert_eq!(
            record.failure_classification.as_deref(),
            Some(CLASSIFICATION_OUTPUT),
            "output breach must be classified as a resource failure"
        );
        assert_eq!(record.resource_kind.as_deref(), Some("output"));
        assert_eq!(classify_validation_failure(&record), CLASSIFICATION_OUTPUT);
        assert_eq!(
            failure_to_terminal_state(CLASSIFICATION_OUTPUT),
            crate::workflow::evaluate::identity::EvaluationState::InfraBlocked
        );
    }

    use crate::workflow::artifact_integrity::{ArtifactKind, publish_with_integrity, sha256_hex};
    use crate::workflow::evaluate::cancellation::CancellationToken;
    use crate::workflow::evaluate::integrity::run_git_cmd;
    use crate::workflow::evaluate::resource::ResourceLimits;
    use crate::workflow::{ApprovalRecord, AuthorityLevel, ProposalArtifact, ScopeContract};

    const PATCH: &str = "diff --git a/new.txt b/new.txt\nnew file mode 100644\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+hello\n";

    fn init_test_repo(name: &str) -> (std::path::PathBuf, String) {
        let dir = std::env::temp_dir().join(format!(
            "prometheos-valtest-{}-{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        run_git_cmd(&dir, &["init"]).unwrap();
        run_git_cmd(&dir, &["config", "user.email", "t@example.com"]).unwrap();
        run_git_cmd(&dir, &["config", "user.name", "t"]).unwrap();
        std::fs::write(dir.join("seed.txt"), "x").unwrap();
        run_git_cmd(&dir, &["add", "."]).unwrap();
        run_git_cmd(&dir, &["commit", "-m", "init"]).unwrap();
        let base = run_git_cmd(&dir, &["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_string();
        (dir, base)
    }

    #[allow(clippy::too_many_arguments)]
    fn write_proposal(
        dir: &std::path::Path,
        id: &str,
        base_sha: &str,
        patch: &str,
        patch_hash: &str,
        approved: Option<ApprovalRecord>,
    ) {
        let proposal = ProposalArtifact {
            id: id.to_string(),
            repo: dir.to_string_lossy().to_string(),
            base_sha: base_sha.to_string(),
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
            patch: patch.to_string(),
            patch_hash: patch_hash.to_string(),
            changed_files: vec![],
            added_lines: 0,
            removed_lines: 0,
            approved,
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
        publish_with_integrity(dir, &prop_path, &bytes, ArtifactKind::Proposal).unwrap();
    }

    #[tokio::test]
    async fn validation_timeout_is_enforced_as_resource_violation() {
        let (dir, base) = init_test_repo("timeout");
        let id = "timeout-run";
        write_proposal(&dir, id, &base, PATCH, &sha256_hex(PATCH.as_bytes()), None);
        let evidence_dir = dir.join(".prometheos").join("workflow").join(id);
        std::fs::create_dir_all(&evidence_dir).unwrap();
        let token = CancellationToken::new();
        let sleep_cmd = if cfg!(windows) {
            "ping -n 31 127.0.0.1 >nul"
        } else {
            "sleep 30"
        };
        let limits = ResourceLimits {
            validation_timeout: Some(std::time::Duration::from_millis(300)),
            ..ResourceLimits::default()
        };
        let rec = run_isolated_validation(
            &dir,
            id,
            Some(sleep_cmd),
            &evidence_dir,
            &token,
            &limits,
            &[],
        )
        .await
        .expect("timeout returns a durable, classified record");
        assert_eq!(
            rec.failure_classification.as_deref(),
            Some(CLASSIFICATION_TIMEOUT),
            "expected timeout classification, got: {rec:?}"
        );
        assert_eq!(
            rec.resource_kind.as_deref(),
            Some("timeout"),
            "expected timeout resource kind, got: {rec:?}"
        );
    }

    #[tokio::test]
    async fn disk_preflight_blocks_validation_when_unsatisfied() {
        let (dir, base) = init_test_repo("disk");
        let id = "disk-run";
        write_proposal(&dir, id, &base, PATCH, &sha256_hex(PATCH.as_bytes()), None);
        let evidence_dir = dir.join(".prometheos").join("workflow").join(id);
        std::fs::create_dir_all(&evidence_dir).unwrap();
        let token = CancellationToken::new();
        // Require far more free space than any real filesystem can provide.
        let limits = ResourceLimits {
            min_free_disk_bytes: Some(u64::MAX),
            ..ResourceLimits::default()
        };
        let err = run_isolated_validation(
            &dir,
            id,
            Some("echo ok"),
            &evidence_dir,
            &token,
            &limits,
            &[],
        )
        .await
        .expect_err("disk preflight must fail validation");
        assert!(
            err.to_string().contains(CLASSIFICATION_DISK),
            "expected disk classification, got: {err}"
        );
    }

    #[test]
    fn check_output_budget_enforces_cap() {
        let capped = ResourceLimits {
            max_output_bytes: Some(10),
            ..ResourceLimits::default()
        };
        assert!(check_output_budget("0123456789", "", &capped).is_ok());
        assert!(check_output_budget("", "0123456789", &capped).is_ok());
        assert!(check_output_budget("0123456789A", "", &capped).is_err());
        assert!(check_output_budget("a", &"a".repeat(100), &capped).is_err());
        // With the default 8 MiB cap, a 1 MiB payload is allowed.
        assert!(
            check_output_budget(&"a".repeat(1_000_000), "", &ResourceLimits::default()).is_ok()
        );
    }

    #[tokio::test]
    async fn patch_integrity_rejects_tampered_patch_hash() {
        let (dir, base) = init_test_repo("patch");
        let id = "patch-run";
        // Deliberately wrong recorded hash.
        write_proposal(&dir, id, &base, PATCH, "deadbeefdeadbeef", None);
        let evidence_dir = dir.join(".prometheos").join("workflow").join(id);
        std::fs::create_dir_all(&evidence_dir).unwrap();
        let token = CancellationToken::new();
        let err = run_isolated_validation(
            &dir,
            id,
            Some("echo ok"),
            &evidence_dir,
            &token,
            &ResourceLimits::default(),
            &[],
        )
        .await
        .expect_err("tampered patch_hash must fail validation");
        assert!(
            err.to_string().contains("patch integrity"),
            "expected patch integrity error, got: {err}"
        );
    }

    #[tokio::test]
    async fn patch_integrity_rejects_mismatched_approval_hash() {
        let (dir, base) = init_test_repo("patch-approval");
        let id = "patch-run-approval";
        let approved = ApprovalRecord {
            approver: "lead".to_string(),
            approved_at: "now".to_string(),
            patch_hash: "wrongapproval".to_string(),
        };
        write_proposal(
            &dir,
            id,
            &base,
            PATCH,
            &sha256_hex(PATCH.as_bytes()),
            Some(approved),
        );
        let evidence_dir = dir.join(".prometheos").join("workflow").join(id);
        std::fs::create_dir_all(&evidence_dir).unwrap();
        let token = CancellationToken::new();
        let err = run_isolated_validation(
            &dir,
            id,
            Some("echo ok"),
            &evidence_dir,
            &token,
            &ResourceLimits::default(),
            &[],
        )
        .await
        .expect_err("mismatched approval patch_hash must fail validation");
        assert!(
            err.to_string().contains("patch integrity"),
            "expected patch integrity error, got: {err}"
        );
    }

    #[tokio::test]
    async fn patch_containing_known_secret_is_rejected() {
        let (dir, base) = init_test_repo("secret-patch");
        let id = "secret-patch-run";
        let secret = "SUPERSECRETXYZ";
        let patch =
            format!("diff --git a/x.rs b/x.rs\n--- a/x.rs\n+++ b/x.rs\n@@ -1 +1 @@\n+{secret}\n");
        write_proposal(&dir, id, &base, &patch, &sha256_hex(patch.as_bytes()), None);
        let evidence_dir = dir.join(".prometheos").join("workflow").join(id);
        std::fs::create_dir_all(&evidence_dir).unwrap();
        let token = CancellationToken::new();
        let err = run_isolated_validation(
            &dir,
            id,
            Some("echo ok"),
            &evidence_dir,
            &token,
            &ResourceLimits::default(),
            &[secret.to_string()],
        )
        .await
        .expect_err("secret-bearing patch must be rejected");
        assert!(
            err.to_string().contains("known secret"),
            "expected secret rejection, got: {err}"
        );
    }

    // CPU/memory enforcement via the aggregate process-tree monitor (Unix) /
    // Job Objects (Windows). These are exercised on platforms that support
    // them; the wall-clock timeout must not mask the resource kill, so it is
    // set generously here.
    #[cfg(unix)]
    #[tokio::test]
    async fn cpu_limit_kills_runaway_process() {
        let (dir, base) = init_test_repo("cpu");
        let id = "cpu-run";
        write_proposal(&dir, id, &base, PATCH, &sha256_hex(PATCH.as_bytes()), None);
        let evidence_dir = dir.join(".prometheos").join("workflow").join(id);
        std::fs::create_dir_all(&evidence_dir).unwrap();
        let token = CancellationToken::new();
        let limits = ResourceLimits {
            validation_timeout: Some(std::time::Duration::from_secs(90)),
            max_cpu_time: Some(std::time::Duration::from_secs(1)),
            ..ResourceLimits::default()
        };
        let err = run_isolated_validation(
            &dir,
            id,
            // The shell exec-optimizes this simple command, so the capped
            // process IS the direct child and SIGXCPU lands on it.
            Some("yes > /dev/null"),
            &evidence_dir,
            &token,
            &limits,
            &[],
        )
        .await
        .expect("cpu limit must be enforced");
        assert_eq!(
            err.failure_classification.as_deref(),
            Some(CLASSIFICATION_CPU),
            "cpu breach must be classified as a resource failure"
        );
        assert_eq!(err.resource_kind.as_deref(), Some("cpu"));
        assert_eq!(classify_validation_failure(&err), CLASSIFICATION_CPU);
        assert_eq!(
            failure_to_terminal_state(CLASSIFICATION_CPU),
            crate::workflow::evaluate::identity::EvaluationState::InfraBlocked
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn memory_limit_kills_runaway_process() {
        // Skip when python3 is unavailable in this environment.
        if std::process::Command::new("python3")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let (dir, base) = init_test_repo("mem");
        let id = "mem-run";
        write_proposal(&dir, id, &base, PATCH, &sha256_hex(PATCH.as_bytes()), None);
        let evidence_dir = dir.join(".prometheos").join("workflow").join(id);
        std::fs::create_dir_all(&evidence_dir).unwrap();
        let token = CancellationToken::new();
        // No disk budget here so only the memory breach is observed.
        let limits = ResourceLimits {
            validation_timeout: Some(std::time::Duration::from_secs(90)),
            max_memory_bytes: Some(64 * 1024 * 1024),
            min_free_disk_bytes: None,
            ..ResourceLimits::default()
        };
        let err = run_isolated_validation(
            &dir,
            id,
            Some("python3 -c 'import time; b=bytearray(256*1024*1024); [b.__setitem__(i,1) for i in range(0,len(b),4096)]; time.sleep(10)'"),
            &evidence_dir,
            &token,
            &limits,
            &[],
        )
        .await
        .expect("memory limit must be enforced");
        assert_eq!(
            err.failure_classification.as_deref(),
            Some(CLASSIFICATION_MEMORY),
            "memory breach must be classified as a resource failure"
        );
        assert_eq!(err.resource_kind.as_deref(), Some("memory"));
        assert_eq!(classify_validation_failure(&err), CLASSIFICATION_MEMORY);
        assert_eq!(
            failure_to_terminal_state(CLASSIFICATION_MEMORY),
            crate::workflow::evaluate::identity::EvaluationState::InfraBlocked
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn cpu_limit_kills_runaway_process_windows() {
        let (dir, base) = init_test_repo("wincpu");
        let id = "wincpu-run";
        write_proposal(&dir, id, &base, PATCH, &sha256_hex(PATCH.as_bytes()), None);
        let evidence_dir = dir.join(".prometheos").join("workflow").join(id);
        std::fs::create_dir_all(&evidence_dir).unwrap();
        let token = CancellationToken::new();
        let limits = ResourceLimits {
            validation_timeout: Some(std::time::Duration::from_secs(90)),
            max_cpu_time: Some(std::time::Duration::from_secs(1)),
            min_free_disk_bytes: None,
            ..ResourceLimits::default()
        };
        let err = run_isolated_validation(
            &dir,
            id,
            Some("for /l %i in (0,0,1) do @rem"),
            &evidence_dir,
            &token,
            &limits,
            &[],
        )
        .await
        .expect("cpu limit must be enforced via Windows Job Object");
        assert_eq!(
            err.failure_classification.as_deref(),
            Some(CLASSIFICATION_CPU),
            "cpu breach must be classified as a resource failure (Job Object setup must succeed)"
        );
        assert_eq!(err.resource_kind.as_deref(), Some("cpu"));
        assert_eq!(classify_validation_failure(&err), CLASSIFICATION_CPU);
        assert_eq!(
            failure_to_terminal_state(CLASSIFICATION_CPU),
            crate::workflow::evaluate::identity::EvaluationState::InfraBlocked
        );
    }

    #[cfg(windows)]
    #[test]
    fn resume_fails_closed_for_process_without_threads() {
        // Injected failure: a pid with no threads in the toolhelp snapshot
        // (nothing to resume) must be an ERROR, not a silent Ok — a child left
        // suspended would otherwise be misrecorded as a timeout breach.
        let err = resume_suspended_process_windows(0x0FFF_FFFF)
            .expect_err("resuming a thread-less pid must fail closed");
        assert!(
            format!("{err:#}").contains("no thread could be resumed"),
            "unexpected error: {err:#}"
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn memory_limit_kills_runaway_process_windows() {
        // Skip when PowerShell is unavailable in this environment.
        if std::process::Command::new("powershell")
            .arg("-NoProfile")
            .arg("-Command")
            .arg("$null")
            .output()
            .is_err()
        {
            return;
        }
        let (dir, base) = init_test_repo("winmem");
        let id = "winmem-run";
        write_proposal(&dir, id, &base, PATCH, &sha256_hex(PATCH.as_bytes()), None);
        let evidence_dir = dir.join(".prometheos").join("workflow").join(id);
        std::fs::create_dir_all(&evidence_dir).unwrap();
        let token = CancellationToken::new();
        let limits = ResourceLimits {
            validation_timeout: Some(std::time::Duration::from_secs(60)),
            max_memory_bytes: Some(64 * 1024 * 1024),
            min_free_disk_bytes: None,
            ..ResourceLimits::default()
        };
        // Write the burner as a script file: this sidesteps every layer of
        // cmd/PowerShell quoting ambiguity in the wrapped command string.
        let ps1 = dir.join("mem_burn.ps1");
        std::fs::write(
            &ps1,
            "$b=New-Object byte[] (300MB); $b[0]=1; Start-Sleep 30\r\n",
        )
        .unwrap();
        let rec = run_isolated_validation(
            &dir,
            id,
            Some(&format!(
                // Bare path: temp paths have no spaces, and cmd/PS quoting
                // layers mangle embedded quotes.
                "powershell -NoProfile -ExecutionPolicy Bypass -File {}",
                ps1.display()
            )),
            &evidence_dir,
            &token,
            &limits,
            &[],
        )
        .await
        .expect("memory limit must be enforced");
        assert_eq!(
            rec.failure_classification.as_deref(),
            Some(CLASSIFICATION_MEMORY),
            "memory breach must be classified as a resource failure: {rec:?}"
        );
        assert_eq!(rec.resource_kind.as_deref(), Some("memory"));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn descendant_tree_is_cleaned_after_output_cap_breach_windows() {
        use std::time::{Duration, Instant};
        let (dir, base) = init_test_repo("windesc");
        let id = "windesc-run";
        write_proposal(&dir, id, &base, PATCH, &sha256_hex(PATCH.as_bytes()), None);
        let evidence_dir = dir.join(".prometheos").join("workflow").join(id);
        std::fs::create_dir_all(&evidence_dir).unwrap();
        let token = CancellationToken::new();
        let limits = ResourceLimits {
            validation_timeout: Some(std::time::Duration::from_secs(60)),
            max_output_bytes: Some(1024),
            ..ResourceLimits::default()
        };
        // The command spawns a long-lived background descendant and then floods
        // stdout past the cap. The whole tree must be terminated; the marker
        // descendant must not outlive the run.
        let cmd = "start /b cmd /c \"ping -n 60 10.255.255.1 -w 1000 >nul\" & for /L %i in (1,1,200) do @echo aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let rec = run_isolated_validation(&dir, id, Some(cmd), &evidence_dir, &token, &limits, &[])
            .await
            .expect("output cap must be enforced");
        assert_eq!(
            rec.failure_classification.as_deref(),
            Some(CLASSIFICATION_OUTPUT),
            "output-cap breach must be classified as a resource failure: {rec:?}"
        );
        // Poll briefly for the descendant to be reaped by the tree kill.
        let mut sys = sysinfo::System::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            sys.refresh_processes();
            let alive = sys
                .processes()
                .values()
                .any(|p| p.cmd().iter().any(|c| c.contains("10.255.255.1")));
            if !alive || Instant::now() > deadline {
                assert!(!alive, "background descendant survived the tree-wide kill");
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_subtree_enumeration_includes_descendants() {
        // Regression guard for proc_listallpids sizing: spawn a shell that
        // forks two children, then verify the enumerated subtree contains more
        // than the root alone. The previous implementation probed the table
        // size as a PID count but sized the buffer as PIDs/4, inspecting only
        // a fraction of the table.
        use std::process::{Command, Stdio};
        let mut child = Command::new("/bin/sh")
            .arg("-c")
            .arg("sleep 30 & sleep 30; wait")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn sh");
        let root = child.id() as i32;
        // Give the shell a moment to fork its children.
        std::thread::sleep(std::time::Duration::from_millis(400));
        let tree = process_tree_subtree(root);
        kill_process_tree(root);
        let _ = child.wait();
        assert!(
            tree.len() >= 3,
            "subtree must include the shell and both sleep descendants, got {tree:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cpu_limit_sigxcpu_is_kernel_classified() {
        // RLIMIT_CPU is a kernel hard cap: even if the aggregate monitor never
        // polls, a death by SIGXCPU must classify the run as
        // resource_cpu_exhausted with typed evidence (stage rlimit_cpu).
        let (dir, base) = init_test_repo("sigxcpu");
        let id = "sigxcpu-run";
        write_proposal(&dir, id, &base, PATCH, &sha256_hex(PATCH.as_bytes()), None);
        let evidence_dir = dir.join(".prometheos").join("workflow").join(id);
        std::fs::create_dir_all(&evidence_dir).unwrap();
        let token = CancellationToken::new();
        let limits = ResourceLimits {
            validation_timeout: Some(std::time::Duration::from_secs(60)),
            max_cpu_time: Some(std::time::Duration::from_secs(1)),
            ..ResourceLimits::default()
        };
        let rec = run_isolated_validation(
            &dir,
            id,
            // The shell exec-optimizes this simple command, so the capped
            // process IS the direct child and SIGXCPU lands on it.
            Some("yes > /dev/null"),
            &evidence_dir,
            &token,
            &limits,
            &[],
        )
        .await
        .expect("cpu limit must be enforced");
        assert_eq!(
            rec.failure_classification.as_deref(),
            Some(CLASSIFICATION_CPU),
            "kernel SIGXCPU death must classify as cpu exhaustion: {rec:?}"
        );
        // Either the kernel verdict (RLIMIT_CPU/SIGXCPU) or the monitor's
        // aggregate observation wins the race; both are deterministic,
        // OS-derived evidence.
        assert!(
            matches!(
                rec.stage.as_deref(),
                Some("rlimit_cpu") | Some("aggregate_monitor")
            ),
            "unexpected stage: {rec:?}"
        );
        assert_eq!(rec.configured_limit.as_deref(), Some("1s"));
        assert_eq!(rec.resource_kind.as_deref(), Some("cpu"));
        assert_eq!(
            failure_to_terminal_state(CLASSIFICATION_CPU),
            crate::workflow::evaluate::identity::EvaluationState::InfraBlocked
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn immediate_exit_137_is_not_cpu_exhaustion() {
        // A validation command that exits 137 on its own must stay an ordinary
        // failure — never be attributed to the CPU cap (regression for the
        // shell-relayed SIGKILL heuristic).
        let (dir, base) = init_test_repo("exit137");
        let id = "exit137-run";
        write_proposal(&dir, id, &base, PATCH, &sha256_hex(PATCH.as_bytes()), None);
        let evidence_dir = dir.join(".prometheos").join("workflow").join(id);
        std::fs::create_dir_all(&evidence_dir).unwrap();
        let token = CancellationToken::new();
        let limits = ResourceLimits {
            validation_timeout: Some(std::time::Duration::from_secs(30)),
            max_cpu_time: Some(std::time::Duration::from_secs(60)),
            ..ResourceLimits::default()
        };
        let rec = run_isolated_validation(
            &dir,
            id,
            Some("sh -c 'exit 137'"),
            &evidence_dir,
            &token,
            &limits,
            &[],
        )
        .await
        .expect("ordinary exit must complete");
        assert_eq!(rec.exit_code, Some(137));
        assert_ne!(
            rec.failure_classification.as_deref(),
            Some(CLASSIFICATION_CPU),
            "exit 137 must never classify as cpu exhaustion: {rec:?}"
        );
        assert_eq!(rec.resource_kind, None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn external_sigkill_is_not_cpu_exhaustion() {
        // A command that SIGKILLs itself is an ordinary signal death — with a
        // CPU limit configured it still must not be classified as CPU
        // exhaustion (only SIGXCPU is the kernel verdict).
        let (dir, base) = init_test_repo("sigkill");
        let id = "sigkill-run";
        write_proposal(&dir, id, &base, PATCH, &sha256_hex(PATCH.as_bytes()), None);
        let evidence_dir = dir.join(".prometheos").join("workflow").join(id);
        std::fs::create_dir_all(&evidence_dir).unwrap();
        let token = CancellationToken::new();
        let limits = ResourceLimits {
            validation_timeout: Some(std::time::Duration::from_secs(30)),
            max_cpu_time: Some(std::time::Duration::from_secs(60)),
            ..ResourceLimits::default()
        };
        let rec = run_isolated_validation(
            &dir,
            id,
            Some("sh -c 'kill -9 $$'"),
            &evidence_dir,
            &token,
            &limits,
            &[],
        )
        .await
        .expect("signal death must complete");
        assert_ne!(
            rec.failure_classification.as_deref(),
            Some(CLASSIFICATION_CPU),
            "self-SIGKILL must never classify as cpu exhaustion: {rec:?}"
        );
        assert_eq!(rec.resource_kind, None);
        assert_ne!(rec.stage.as_deref(), Some("rlimit_cpu"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn multi_process_aggregate_cpu_burner_is_monitor_classified() {
        // RLIMIT_CPU is per process: two concurrent burners each get their own
        // allowance, so the KERNEL cap alone cannot see the aggregate. The
        // aggregate budget must be enforced and classified by the monitor.
        let (dir, base) = init_test_repo("aggcpu");
        let id = "aggcpu-run";
        write_proposal(&dir, id, &base, PATCH, &sha256_hex(PATCH.as_bytes()), None);
        let evidence_dir = dir.join(".prometheos").join("workflow").join(id);
        std::fs::create_dir_all(&evidence_dir).unwrap();
        let token = CancellationToken::new();
        let limits = ResourceLimits {
            validation_timeout: Some(std::time::Duration::from_secs(60)),
            max_cpu_time: Some(std::time::Duration::from_secs(2)),
            ..ResourceLimits::default()
        };
        let rec = run_isolated_validation(
            &dir,
            id,
            // Two burners: ~2s CPU per second of wall time in aggregate, while
            // each stays under its individual 2s allowance when the tree is
            // killed at the 2s aggregate mark.
            Some("sh -c 'yes > /dev/null & yes > /dev/null; wait'"),
            &evidence_dir,
            &token,
            &limits,
            &[],
        )
        .await
        .expect("aggregate cpu limit must be enforced");
        assert_eq!(
            rec.failure_classification.as_deref(),
            Some(CLASSIFICATION_CPU),
            "{rec:?}"
        );
        assert_eq!(
            rec.stage.as_deref(),
            Some("aggregate_monitor"),
            "multi-process burn must be classified by the aggregate monitor: {rec:?}"
        );
        assert_eq!(rec.configured_limit.as_deref(), Some("2s"));
    }
}
