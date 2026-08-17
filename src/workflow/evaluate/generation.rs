use anyhow::{Context, Result};
use std::path::Path;

use crate::workflow::ProposalArtifact;
use crate::workflow::artifact_integrity::{ArtifactKind, read_verified_or_legacy};

// ---------------------------------------------------------------------------
// Failure classification
// ---------------------------------------------------------------------------

pub(super) fn classify_generation_error(msg: &str) -> String {
    if msg.contains("disk")
        || msg.contains("ENOSPC")
        || msg.contains("credential")
        || msg.contains("API key")
        || msg.contains("401")
        || msg.contains("network")
        || msg.contains("timeout")
        || msg.contains("ECONNREFUSED")
    {
        "infra_blocked".to_string()
    } else {
        "generation_failed".to_string()
    }
}
pub(super) fn load_proposal_from_repo(repo: &Path, id: &str) -> Result<ProposalArtifact> {
    let path = repo
        .join(".prometheos")
        .join("workflow")
        .join(id)
        .join("proposal.json");
    // Trusted read: verify the #115-format checksum sidecar before the proposal
    // becomes input to recovery. Genuinely legacy artifacts are tolerated.
    let bytes = read_verified_or_legacy(repo, &path, ArtifactKind::Proposal)
        .with_context(|| format!("cannot read proposal {id} at {}", path.display()))?;
    serde_json::from_slice(&bytes).context("failed to parse proposal artifact")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classify_generation_error_infra() {
        assert_eq!(classify_generation_error("disk full"), "infra_blocked");
        assert_eq!(
            classify_generation_error("credential not found"),
            "infra_blocked"
        );
        assert_eq!(
            classify_generation_error("network timeout"),
            "infra_blocked"
        );
    }
    #[test]
    fn classify_generation_error_not_infra() {
        assert_eq!(
            classify_generation_error("provider returned no edits"),
            "generation_failed"
        );
    }
}
