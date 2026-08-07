//! Evaluation checkpoint document.
//!
//! A checkpoint is a compact, versioned snapshot of where an evaluation run
//! currently stands. It is written after the corresponding journal event is
//! durable, so recovery can trust either the journal (source of truth) or the
//! most recent checkpoint (fast path). Checkpoints are NOT the full
//! `PortableWorkState` (issue #151); they contain no proposal bytes and no
//! evidence — only references.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::identity::EvaluationState;
use super::schema::{
    CURRENT_SCHEMA_VERSION, DocumentType, SchemaVersion, validate_version, version_diagnostic,
};

/// Compact durable snapshot of an evaluation's position.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationCheckpoint {
    /// Schema version of this checkpoint document.
    pub schema_version: SchemaVersion,
    /// Deterministic identity key.
    pub identity_key: String,
    /// Run that owns this checkpoint.
    pub run_id: String,
    /// Repository path at checkpoint time.
    pub repo: String,
    /// Repository revision at checkpoint time.
    pub repo_pin: String,
    /// Current evaluation state.
    pub state: EvaluationState,
    /// Highest journal sequence persisted when this checkpoint was written.
    pub last_journal_sequence: u64,
    /// Proposal reference, when one exists.
    pub proposal_ref: Option<String>,
    /// Evidence directory reference (relative to repo).
    pub evidence_ref: Option<String>,
    /// Owner run id + lease epoch that protected this checkpoint.
    pub owner_run_id: String,
    pub lease_epoch: u64,
    /// RFC3339 timestamp.
    pub updated_at: String,
}

/// Checkpoint file for an identity.
pub fn checkpoint_path_for(repo: &Path, identity_key: &str) -> PathBuf {
    repo.join(".prometheos")
        .join("workflow")
        .join("checkpoint")
        .join(format!("{identity_key}.json"))
}

/// Write a checkpoint atomically (temp file + fsync + rename + dir fsync).
pub fn write_checkpoint(repo: &Path, checkpoint: &EvaluationCheckpoint) -> Result<()> {
    let path = checkpoint_path_for(repo, &checkpoint.identity_key);
    super::durable::atomic_write_json(&path, checkpoint)
        .with_context(|| format!("failed to commit checkpoint {}", path.display()))
}

/// Read and validate a checkpoint for an identity, if present.
pub fn read_checkpoint(repo: &Path, identity_key: &str) -> Result<Option<EvaluationCheckpoint>> {
    let path = checkpoint_path_for(repo, identity_key);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read checkpoint {}", path.display()))?;
    let cp: EvaluationCheckpoint = serde_json::from_str(&text)
        .with_context(|| format!("corrupt checkpoint {}", path.display()))?;
    match validate_version(DocumentType::EvaluationCheckpoint, cp.schema_version)? {
        super::schema::VersionStatus::Unsupported => bail!(
            "{}",
            version_diagnostic(DocumentType::EvaluationCheckpoint, cp.schema_version)
                .migration_action
        ),
        _ => Ok(Some(cp)),
    }
}

/// Build a checkpoint for the given position.
pub fn build_checkpoint(
    repo: &Path,
    identity_key: &str,
    run_id: &str,
    repo_pin: &str,
    state: EvaluationState,
    last_journal_sequence: u64,
    proposal_ref: Option<String>,
    evidence_ref: Option<String>,
    owner_run_id: &str,
    lease_epoch: u64,
) -> EvaluationCheckpoint {
    EvaluationCheckpoint {
        schema_version: CURRENT_SCHEMA_VERSION,
        identity_key: identity_key.to_string(),
        run_id: run_id.to_string(),
        repo: repo.display().to_string(),
        repo_pin: repo_pin.to_string(),
        state,
        last_journal_sequence,
        proposal_ref,
        evidence_ref,
        owner_run_id: owner_run_id.to_string(),
        lease_epoch,
        updated_at: super::identity::now_iso(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_dir() -> PathBuf {
        tempfile::tempdir().unwrap().path().to_path_buf()
    }

    fn sample(repo: &Path, state: EvaluationState, last_seq: u64) -> EvaluationCheckpoint {
        build_checkpoint(
            repo,
            "key-1",
            "run-1",
            "abc123",
            state,
            last_seq,
            Some("proposal-1".to_string()),
            Some("evidence/run-1".to_string()),
            "run-1",
            0,
        )
    }

    #[test]
    fn checkpoint_round_trips() {
        let repo = repo_dir();
        let cp = sample(&repo, EvaluationState::Validating, 4);
        write_checkpoint(&repo, &cp).unwrap();
        let loaded = read_checkpoint(&repo, "key-1").unwrap().unwrap();
        assert_eq!(loaded.identity_key, "key-1");
        assert_eq!(loaded.state, EvaluationState::Validating);
        assert_eq!(loaded.last_journal_sequence, 4);
        assert_eq!(loaded.proposal_ref.as_deref(), Some("proposal-1"));
        assert_eq!(loaded.schema_version, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn missing_checkpoint_returns_none() {
        let repo = repo_dir();
        assert!(read_checkpoint(&repo, "key-1").unwrap().is_none());
    }

    #[test]
    fn corrupt_checkpoint_fails_closed() {
        let repo = repo_dir();
        let path = checkpoint_path_for(&repo, "key-1");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "not json").unwrap();
        assert!(read_checkpoint(&repo, "key-1").is_err());
    }

    #[test]
    fn unsupported_checkpoint_version_fails_closed() {
        let repo = repo_dir();
        let path = checkpoint_path_for(&repo, "key-1");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut cp = sample(&repo, EvaluationState::Validating, 0);
        cp.schema_version = SchemaVersion::new(99, 0, 0);
        std::fs::write(&path, serde_json::to_string_pretty(&cp).unwrap()).unwrap();
        assert!(read_checkpoint(&repo, "key-1").is_err());
    }
}
