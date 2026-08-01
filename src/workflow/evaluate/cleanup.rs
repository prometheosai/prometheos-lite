use std::path::Path;

use super::evidence::CleanupRecord;
use super::integrity::run_git_cmd;

// ---------------------------------------------------------------------------
// Cleanup
// ---------------------------------------------------------------------------

pub(super) fn cleanup_worktree(repo: &Path, proposal_id: &str) -> CleanupRecord {
    let wt_root = std::env::temp_dir().join(format!("prometheos-eval-{proposal_id}"));
    let patch_file =
        std::env::temp_dir().join(format!("prometheos-eval-patch-{proposal_id}.patch"));

    let worktree_removed = run_git_cmd(
        repo,
        &["worktree", "remove", "--force", wt_root.to_str().unwrap()],
    )
    .is_ok()
        || !wt_root.exists();

    let _ = std::fs::remove_dir_all(&wt_root);
    let _ = std::fs::remove_file(&patch_file);

    // Evidence is preserved in the evidence directory, not in the worktree.
    CleanupRecord {
        worktree_removed,
        evidence_preserved: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_worktree_treated_as_removed_and_evidence_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        let proposal_id = format!("unique-proposal-{}", uuid::Uuid::new_v4());

        let record = cleanup_worktree(&repo, &proposal_id);
        assert!(record.worktree_removed);
        assert!(record.evidence_preserved);

        // Temp patch path is absent afterward.
        let patch = std::env::temp_dir().join(format!("prometheos-eval-patch-{proposal_id}.patch"));
        assert!(!patch.exists());
    }
}
