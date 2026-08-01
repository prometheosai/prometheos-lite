use anyhow::{Context, Result};
use std::path::Path;

use crate::workflow::ProposalArtifact;

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
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read proposal {id} at {}", path.display()))?;
    serde_json::from_str(&text).context("failed to parse proposal artifact")
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
