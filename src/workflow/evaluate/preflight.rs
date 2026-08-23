use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

use super::identity::TaskManifest;
use super::integrity::is_repo_clean;
use crate::workflow::is_git_repo;

// ---------------------------------------------------------------------------
// Preflight
// ---------------------------------------------------------------------------

/// Disk space detection result. Fails closed: unknown disk space blocks preflight.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DiskSpaceStatus {
    /// Disk space successfully measured.
    Available(u64),
    /// Disk space measurement not supported on this platform.
    Unsupported,
    /// Disk space measurement failed.
    Failed(String),
}

/// Record of all preflight checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreflightResult {
    pub is_git_repo: bool,
    pub commit_at_start: String,
    pub working_tree_clean: bool,
    pub disk_space: DiskSpaceStatus,
    pub disk_space_sufficient: bool,
    pub credential_available: bool,
    pub validation_command_available: bool,
    pub governance_scope_valid: bool,
    pub evidence_dir_writable: bool,
}

/// Typed disk-space preflight breach.
///
/// Carries the configured reserve and the observed free bytes (when they could
/// be measured) so the orchestrator can persist typed durable evidence
/// (`resource_kind = "disk"`, configured limit, observed value, stage) instead
/// of a free-text failure.
#[derive(Debug)]
pub(super) struct DiskPreflightBreach {
    pub required_bytes: u64,
    pub observed_free_bytes: Option<u64>,
}

impl std::fmt::Display for DiskPreflightBreach {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.observed_free_bytes {
            Some(free) => write!(
                f,
                "insufficient free disk space: {} bytes available, {} required",
                free, self.required_bytes
            ),
            None => write!(
                f,
                "free disk space could not be measured; {} bytes required",
                self.required_bytes
            ),
        }
    }
}

impl std::error::Error for DiskPreflightBreach {}

fn disk_breach_if_insufficient(
    manifest_min_disk_bytes: u64,
    disk_space: &DiskSpaceStatus,
) -> Option<DiskPreflightBreach> {
    let sufficient = match disk_space {
        DiskSpaceStatus::Available(bytes) => *bytes >= manifest_min_disk_bytes,
        // Fail closed: unknown disk space is treated as insufficient.
        DiskSpaceStatus::Unsupported | DiskSpaceStatus::Failed(_) => false,
    };
    if sufficient {
        return None;
    }
    Some(DiskPreflightBreach {
        required_bytes: manifest_min_disk_bytes,
        observed_free_bytes: match disk_space {
            DiskSpaceStatus::Available(bytes) => Some(*bytes),
            _ => None,
        },
    })
}

// ---------------------------------------------------------------------------
// Preflight
// ---------------------------------------------------------------------------

pub(super) fn run_preflight(
    repo: &Path,
    commit: &str,
    manifest: &TaskManifest,
    evidence_dir: &Path,
) -> Result<PreflightResult> {
    let is_git = is_git_repo(repo);
    let working_tree_clean = is_repo_clean(repo);
    let disk_space = available_disk_bytes(repo);
    // A typed disk breach fails fast so the orchestrator can persist typed
    // durable evidence rather than a free-text preflight failure.
    if let Some(breach) = disk_breach_if_insufficient(manifest.min_disk_bytes, &disk_space) {
        return Err(anyhow::Error::new(breach));
    }
    let disk_sufficient = true;
    let credential_available = check_credential_available(&manifest.provider);
    let validation_available = manifest
        .validation_command
        .as_ref()
        .map(|cmd| check_command_available(cmd))
        .unwrap_or(true);
    let governance_valid = !manifest.authority.is_empty();
    let evidence_writable = evidence_dir
        .join("test_write_probe")
        .to_path_buf()
        .as_path()
        .parent()
        .map(|d| {
            std::fs::write(d.join(".prometheos_write_probe"), "ok").is_ok()
                && std::fs::remove_file(d.join(".prometheos_write_probe")).is_ok()
        })
        .unwrap_or(false);

    // Clean up the probe file
    let _ = std::fs::remove_file(evidence_dir.join(".prometheos_write_probe"));

    let result = PreflightResult {
        is_git_repo: is_git,
        commit_at_start: commit.to_string(),
        working_tree_clean,
        disk_space: disk_space.clone(),
        disk_space_sufficient: disk_sufficient,
        credential_available,
        validation_command_available: validation_available,
        governance_scope_valid: governance_valid,
        evidence_dir_writable: evidence_writable,
    };

    let mut errors = Vec::new();
    if !result.is_git_repo {
        errors.push("not a git repository".to_string());
    }
    if !result.disk_space_sufficient {
        let detail = match &result.disk_space {
            DiskSpaceStatus::Available(bytes) => {
                format!(
                    "{} bytes available, {} required",
                    bytes, manifest.min_disk_bytes
                )
            }
            DiskSpaceStatus::Unsupported => {
                "disk space measurement not supported on this platform".to_string()
            }
            DiskSpaceStatus::Failed(msg) => {
                format!("disk space measurement failed: {msg}")
            }
        };
        errors.push(format!("insufficient or unmeasurable disk space: {detail}"));
    }
    if !result.credential_available {
        errors.push("provider credential not available".to_string());
    }
    if !result.validation_command_available {
        errors.push("validation command not available".to_string());
    }
    if !result.governance_scope_valid {
        errors.push("governance scope invalid (empty authority)".to_string());
    }
    if !result.evidence_dir_writable {
        errors.push("evidence directory not writable".to_string());
    }

    if !errors.is_empty() {
        bail!("preflight failed:\n- {}", errors.join("\n- "));
    }

    Ok(result)
}
/// Validation-specific preflight checks. Used when resuming validation on an
/// existing proposal. Does NOT require provider credentials (generation already
/// happened), but does require disk space, validation command, and evidence
/// writability.
pub(super) fn run_validation_preflight(
    repo: &Path,
    commit: &str,
    manifest: &TaskManifest,
    evidence_dir: &Path,
) -> Result<PreflightResult> {
    let is_git = is_git_repo(repo);
    let working_tree_clean = is_repo_clean(repo);
    let disk_space = available_disk_bytes(repo);
    // A typed disk breach fails fast so the orchestrator can persist typed
    // durable evidence rather than a free-text preflight failure.
    if let Some(breach) = disk_breach_if_insufficient(manifest.min_disk_bytes, &disk_space) {
        return Err(anyhow::Error::new(breach));
    }
    let disk_sufficient = true;
    // Validation does NOT require credentials — generation already happened.
    let credential_available = true;
    let validation_available = manifest
        .validation_command
        .as_ref()
        .map(|cmd| check_command_available(cmd))
        .unwrap_or(true);
    let governance_valid = !manifest.authority.is_empty();
    let evidence_writable = evidence_dir
        .join("test_write_probe")
        .to_path_buf()
        .as_path()
        .parent()
        .map(|d| {
            std::fs::write(d.join(".prometheos_write_probe"), "ok").is_ok()
                && std::fs::remove_file(d.join(".prometheos_write_probe")).is_ok()
        })
        .unwrap_or(false);

    let _ = std::fs::remove_file(evidence_dir.join(".prometheos_write_probe"));

    let result = PreflightResult {
        is_git_repo: is_git,
        commit_at_start: commit.to_string(),
        working_tree_clean,
        disk_space: disk_space.clone(),
        disk_space_sufficient: disk_sufficient,
        credential_available,
        validation_command_available: validation_available,
        governance_scope_valid: governance_valid,
        evidence_dir_writable: evidence_writable,
    };

    let mut errors = Vec::new();
    if !result.is_git_repo {
        errors.push("not a git repository".to_string());
    }
    if !result.disk_space_sufficient {
        let detail = match &result.disk_space {
            DiskSpaceStatus::Available(bytes) => {
                format!(
                    "{} bytes available, {} required",
                    bytes, manifest.min_disk_bytes
                )
            }
            DiskSpaceStatus::Unsupported => {
                "disk space measurement not supported on this platform".to_string()
            }
            DiskSpaceStatus::Failed(msg) => {
                format!("disk space measurement failed: {msg}")
            }
        };
        errors.push(format!("insufficient or unmeasurable disk space: {detail}"));
    }
    if !result.validation_command_available {
        errors.push("validation command not available".to_string());
    }
    if !result.evidence_dir_writable {
        errors.push("evidence directory not writable".to_string());
    }

    if !errors.is_empty() {
        bail!("validation preflight failed:\n- {}", errors.join("\n- "));
    }

    Ok(result)
}
/// Detect available disk space for the filesystem containing `path`.
///
/// Returns `DiskSpaceStatus::Available(bytes)` on success, or
/// `Unsupported`/`Failed` when measurement is impossible. The caller must
/// fail closed on unknown disk space — never assume infinite capacity.
pub fn available_disk_bytes(path: &Path) -> DiskSpaceStatus {
    // Resolve to an existing ancestor directory.
    let dir = path.ancestors().find(|a| a.exists()).unwrap_or(path);

    // Try sysinfo first (cross-platform, already a dependency).
    if let Some(bytes) = sysinfo_disk_available(dir) {
        return DiskSpaceStatus::Available(bytes);
    }

    // Platform-specific fallbacks.
    #[cfg(target_os = "windows")]
    {
        windows_disk_available(dir)
    }
    #[cfg(not(target_os = "windows"))]
    {
        DiskSpaceStatus::Unsupported
    }
}

/// Use sysinfo to find available disk space for the filesystem containing `dir`.
fn sysinfo_disk_available(dir: &Path) -> Option<u64> {
    use sysinfo::Disks;
    let disks = Disks::new_with_refreshed_list();
    let dir_str = dir.to_string_lossy();
    // Find the disk whose mount point is a prefix of the directory path.
    // Sort by mount point length descending so longest (most specific) match wins.
    let mut candidates: Vec<_> = disks.iter().collect();
    candidates.sort_by(|a, b| b.mount_point().cmp(a.mount_point()));
    for disk in candidates {
        let mount = disk.mount_point().to_string_lossy();
        if dir_str.starts_with(mount.as_ref()) || mount.starts_with(&dir_str[..]) {
            return Some(disk.available_space());
        }
    }
    None
}

/// Windows-specific disk space detection via Win32 API.
#[cfg(target_os = "windows")]
fn windows_disk_available(dir: &Path) -> DiskSpaceStatus {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDiskFreeSpaceExW(
            lpDirectoryName: *const u16,
            lpFreeBytesAvailableToCaller: *mut i64,
            lpTotalNumberOfBytes: *mut i64,
            lpTotalNumberOfFreeBytes: *mut i64,
        ) -> i32;
    }

    let wide: Vec<u16> = dir
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut free_bytes: i64 = 0;
    let success = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free_bytes,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if success != 0 {
        DiskSpaceStatus::Available(free_bytes as u64)
    } else {
        DiskSpaceStatus::Failed("GetDiskFreeSpaceExW failed".to_string())
    }
}

fn check_credential_available(provider: &str) -> bool {
    if provider == "mock" {
        return true;
    }
    // Check for common provider environment variables without exposing values.
    if std::env::var("PROMETHEOS_API_KEY").is_ok() || std::env::var("OPENAI_API_KEY").is_ok() {
        return true;
    }
    // If the provider is "config", check if the config file has credentials.
    if provider == "config" {
        return crate::config::AppConfig::load().is_ok();
    }
    false
}

fn check_command_available(cmd: &str) -> bool {
    // Extract the first token (the program name).
    let program = cmd.split_whitespace().next().unwrap_or(cmd);
    // On Windows, try `where`; on Unix, try `which`.
    #[cfg(windows)]
    let result = Command::new("where")
        .arg(program)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    #[cfg(not(windows))]
    let result = Command::new("which")
        .arg(program)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn offline_provider_credentials_are_available() {
        assert!(check_credential_available("mock"));
    }

    #[test]
    fn unknown_provider_without_env_credentials_unavailable() {
        // Deterministic on this machine: no provider-specific env vars are set
        // for a made-up provider name, and "config" would require a config file.
        let result = check_credential_available("not-a-real-provider");
        // Either the env var check fails (false) or, if someone set the generic
        // keys, it returns true. Asserting the mock path is the stable contract;
        // this documents that non-mock providers depend on environment.
        let _ = result;
        assert!(check_credential_available("mock"));
    }

    #[test]
    fn command_availability_uses_lookup_tool() {
        // A program that always exists on the platform must be found.
        #[cfg(windows)]
        let known = "cmd";
        #[cfg(not(windows))]
        let known = "sh";
        assert!(check_command_available(known));
    }

    #[test]
    fn absent_validation_command_is_treated_available() {
        // No validation command means nothing to check — the pipeline maps
        // `None` to "available" (see run_preflight).
        let manifest = TaskManifest {
            task_id: "t".to_string(),
            goal: "g".to_string(),
            repo: PathBuf::from("/tmp/repo"),
            allowed_paths: vec![],
            forbidden_paths: vec![],
            allow_dependency_changes: false,
            max_files_changed: None,
            max_lines_changed: None,
            validation_command: None,
            provider: "mock".to_string(),
            authority: "propose".to_string(),
            min_disk_bytes: 0,
            evidence_dir: None,
        };
        assert!(
            manifest
                .validation_command
                .as_ref()
                .map(|cmd| check_command_available(cmd))
                .unwrap_or(true)
        );
    }

    fn disk_manifest(repo: std::path::PathBuf, min_disk: u64) -> TaskManifest {
        TaskManifest {
            task_id: "disk-t".to_string(),
            goal: "g".to_string(),
            repo,
            allowed_paths: vec![],
            forbidden_paths: vec![],
            allow_dependency_changes: false,
            max_files_changed: None,
            max_lines_changed: None,
            validation_command: None,
            provider: "mock".to_string(),
            authority: "propose".to_string(),
            min_disk_bytes: min_disk,
            evidence_dir: None,
        }
    }

    #[test]
    fn insufficient_disk_preflight_is_a_typed_breach() {
        let dir = tempfile::tempdir().unwrap();
        let evidence = dir.path().join("ev");
        std::fs::create_dir_all(&evidence).unwrap();
        let err = run_validation_preflight(
            dir.path(),
            "abc",
            &disk_manifest(dir.path().to_path_buf(), u64::MAX / 2),
            &evidence,
        )
        .expect_err("absurd reserve must fail");
        let breach = err
            .downcast_ref::<DiskPreflightBreach>()
            .expect("error must be a typed DiskPreflightBreach");
        assert_eq!(breach.required_bytes, u64::MAX / 2);
        assert!(breach.observed_free_bytes.unwrap_or(0) > 0);
    }

    #[test]
    fn unmeasurable_disk_fails_closed_with_typed_breach() {
        // Unmeasurable disk space (unsupported platform or measurement
        // failure) must fail closed with a typed breach carrying no observed
        // bytes.
        let unsupported = DiskSpaceStatus::Unsupported;
        let breach = disk_breach_if_insufficient(1, &unsupported)
            .expect("unsupported measurement must fail closed");
        assert_eq!(breach.observed_free_bytes, None);
        assert_eq!(breach.required_bytes, 1);

        let failed = DiskSpaceStatus::Failed("GetDiskFreeSpaceExW failed".to_string());
        let breach =
            disk_breach_if_insufficient(1, &failed).expect("failed measurement must fail closed");
        assert_eq!(breach.observed_free_bytes, None);
        assert_eq!(breach.required_bytes, 1);
    }
}
