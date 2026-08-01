use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;

use super::evidence::IntegrityRecord;
use super::generation::load_proposal_from_repo;

// ---------------------------------------------------------------------------
// Repository integrity
// ---------------------------------------------------------------------------

pub fn verify_repo_integrity(
    repo: &Path,
    expected_commit: &str,
    proposal_id: &str,
) -> IntegrityRecord {
    let current_commit = git_rev_parse_head(repo).unwrap_or_default();
    let original_commit_unchanged = current_commit == expected_commit;

    let status = run_git_cmd(repo, &["status", "--porcelain"]).unwrap_or_default();
    let no_tracked_modifications = status.lines().all(|line| {
        let path = line.get(3..).unwrap_or("").trim();
        path.starts_with(".prometheos/")
    });

    let staged = run_git_cmd(repo, &["diff", "--cached", "--name-only"]).unwrap_or_default();
    let no_staged_modifications = staged.trim().is_empty();

    // Check that the proposal was not applied.
    let proposal = load_proposal_from_repo(repo, proposal_id).ok();
    let proposal_not_applied = proposal.map(|p| p.applied != Some(true)).unwrap_or(true);

    // Candidate changes confined: no untracked files outside .prometheos/.
    let candidate_changes_confined = status.lines().all(|line| {
        let path = line.get(3..).unwrap_or("").trim();
        path.starts_with(".prometheos/")
    });

    IntegrityRecord {
        original_commit_unchanged,
        no_tracked_modifications,
        no_staged_modifications,
        candidate_changes_confined,
        proposal_not_applied,
    }
}
// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(super) fn git_rev_parse_head(repo: &Path) -> Result<String> {
    let out = run_git_cmd(repo, &["rev-parse", "HEAD"])?;
    Ok(out.trim().to_string())
}

pub(super) fn run_git_cmd(repo: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .context("failed to execute git")?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git {} failed: {}", args.join(" "), stderr.trim())
    }
}

pub(super) fn is_repo_clean(repo: &Path) -> bool {
    run_git_cmd(repo, &["status", "--porcelain"])
        .map(|s| s.trim().is_empty())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        let run = |args: &[&str]| {
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
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(repo.join("file.txt"), "hello").unwrap();
        run(&["add", "-A"]);
        run(&["commit", "-qm", "init"]);
        dir
    }

    fn head(repo: &Path) -> String {
        git_rev_parse_head(repo).unwrap()
    }

    #[test]
    fn clean_repository_accepted() {
        let dir = init_repo();
        let record = verify_repo_integrity(dir.path(), &head(dir.path()), "missing-proposal");
        assert!(record.original_commit_unchanged);
        assert!(record.no_tracked_modifications);
        assert!(record.no_staged_modifications);
        assert!(record.proposal_not_applied);
        assert!(record.candidate_changes_confined);
    }

    #[test]
    fn tracked_modification_detected() {
        let dir = init_repo();
        let expected = head(dir.path());
        std::fs::write(dir.path().join("file.txt"), "changed").unwrap();
        let record = verify_repo_integrity(dir.path(), &expected, "missing-proposal");
        assert!(!record.no_tracked_modifications);
    }

    #[test]
    fn prometheos_artifacts_remain_allowed() {
        let dir = init_repo();
        let expected = head(dir.path());
        // Untracked artifacts under .prometheos/ are allowed by the policy.
        std::fs::create_dir_all(dir.path().join(".prometheos/diagnostics")).unwrap();
        std::fs::write(dir.path().join(".prometheos/diagnostics/probe.log"), "x").unwrap();
        let record = verify_repo_integrity(dir.path(), &expected, "missing-proposal");
        assert!(record.original_commit_unchanged);
        assert!(record.no_tracked_modifications);
        assert!(record.candidate_changes_confined);
    }

    #[test]
    fn untracked_file_outside_prometheos_breaks_confined() {
        let dir = init_repo();
        let expected = head(dir.path());
        std::fs::write(dir.path().join("new_untracked.txt"), "x").unwrap();
        let record = verify_repo_integrity(dir.path(), &expected, "missing-proposal");
        assert!(!record.candidate_changes_confined);
    }
}
