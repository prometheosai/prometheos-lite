//! Evidence Persistence - P0-HARNESS-009
//!
//! Provides explicit persistence contract for EvidenceLog entries and artifacts
//! into WorkContext/RunDb lifecycle as queryable audit trail.

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::harness::{
    artifacts::HarnessArtifact,
    evidence::{EvidenceEntry, EvidenceLog},
};

/// P0-HARNESS-009: Evidence persistence contract for audit trail
#[async_trait]
pub trait EvidenceSink {
    /// Persist a single evidence entry to durable storage
    async fn persist_entry(&self, work_context_id: &str, entry: EvidenceEntry) -> Result<()>;
    
    /// Persist an artifact to durable storage
    async fn persist_artifact(&self, work_context_id: &str, artifact: HarnessArtifact) -> Result<()>;
    
    /// Persist multiple evidence entries in batch
    async fn persist_batch(&self, work_context_id: &str, entries: Vec<EvidenceEntry>) -> Result<()> {
        for entry in entries {
            self.persist_entry(work_context_id, entry).await?;
        }
        Ok(())
    }
    
    /// Retrieve evidence entries for a work context
    async fn retrieve_entries(&self, work_context_id: &str) -> Result<Vec<EvidenceEntry>>;
    
    /// Retrieve artifacts for a work context
    async fn retrieve_artifacts(&self, work_context_id: &str) -> Result<Vec<HarnessArtifact>>;
}

/// P0-HARNESS-009: File-based evidence persistence implementation
pub struct FileEvidenceSink {
    base_path: PathBuf,
}

impl FileEvidenceSink {
    /// Create a new file-based evidence sink
    pub fn new(base_path: PathBuf) -> Self {
        Self { base_path }
    }
    
    /// Get the evidence file path for a work context
    fn evidence_file_path(&self, work_context_id: &str) -> PathBuf {
        self.base_path.join(format!("{}.evidence.json", work_context_id))
    }
    
    /// Get the artifacts directory path for a work context
    fn artifacts_dir_path(&self, work_context_id: &str) -> PathBuf {
        self.base_path.join(format!("{}.artifacts", work_context_id))
    }
}

#[async_trait]
impl EvidenceSink for FileEvidenceSink {
    async fn persist_entry(&self, work_context_id: &str, entry: EvidenceEntry) -> Result<()> {
        use tokio::fs;
        use std::collections::HashMap;
        
        let evidence_file = self.evidence_file_path(work_context_id);
        
        // Ensure parent directory exists
        if let Some(parent) = evidence_file.parent() {
            fs::create_dir_all(parent).await?;
        }
        
        // Read existing evidence log
        let mut entries = if evidence_file.exists() {
            let content = fs::read_to_string(&evidence_file).await?;
            serde_json::from_str(&content).unwrap_or_else(|_| Vec::new())
        } else {
            Vec::new()
        };
        
        // Add new entry
        let entry_id = entry.id.clone();
        entries.push(entry);
        
        // Write back to file
        let content = serde_json::to_string_pretty(&entries)?;
        fs::write(&evidence_file, content).await?;
        
        tracing::debug!("P0-HARNESS-009: Persisted evidence entry {} for work context {}", 
                   entry_id, work_context_id);
        
        Ok(())
    }
    
    async fn persist_artifact(&self, work_context_id: &str, artifact: HarnessArtifact) -> Result<()> {
        use tokio::fs;
        
        let artifacts_dir = self.artifacts_dir_path(work_context_id);
        
        // Ensure artifacts directory exists
        fs::create_dir_all(&artifacts_dir).await?;
        
        let artifact_file = artifacts_dir.join(format!("{}.json", artifact.id));
        let content = serde_json::to_string_pretty(&artifact)?;
        
        fs::write(&artifact_file, content).await?;
        
        tracing::debug!("P0-HARNESS-009: Persisted artifact {} for work context {}", 
                   artifact.id, work_context_id);
        
        Ok(())
    }
    
    async fn retrieve_entries(&self, work_context_id: &str) -> Result<Vec<EvidenceEntry>> {
        use tokio::fs;
        
        let evidence_file = self.evidence_file_path(work_context_id);
        
        if !evidence_file.exists() {
            return Ok(Vec::new());
        }
        
        let content = fs::read_to_string(&evidence_file).await?;
        let entries: Vec<EvidenceEntry> = serde_json::from_str(&content)?;
        
        Ok(entries)
    }
    
    async fn retrieve_artifacts(&self, work_context_id: &str) -> Result<Vec<HarnessArtifact>> {
        use tokio::fs;
        
        let artifacts_dir = self.artifacts_dir_path(work_context_id);
        
        if !artifacts_dir.exists() {
            return Ok(Vec::new());
        }
        
        let mut artifacts = Vec::new();
        let mut entries = fs::read_dir(&artifacts_dir).await?;
        
        while let Some(entry) = entries.next_entry().await {
            let entry = entry?;
            let path = entry.path();
            
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                let content = fs::read_to_string(&path).await?;
                let artifact: HarnessArtifact = serde_json::from_str(&content)?;
                artifacts.push(artifact);
            }
        }
        
        Ok(artifacts)
    }
}

/// P0-HARNESS-009: Evidence persistence manager
pub struct EvidencePersistenceManager {
    sink: Box<dyn EvidenceSink + Send + Sync>,
}

impl EvidencePersistenceManager {
    /// Create a new evidence persistence manager
    pub fn new(sink: Box<dyn EvidenceSink + Send + Sync>) -> Self {
        Self { sink }
    }
    
    /// Persist an entire evidence log for a work context
    pub async fn persist_evidence_log(&self, work_context_id: &str, evidence_log: &EvidenceLog) -> Result<()> {
        tracing::info!("P0-HARNESS-009: Persisting {} evidence entries for work context {}", 
                    evidence_log.entries.len(), work_context_id);
        
        self.sink.persist_batch(work_context_id, evidence_log.entries.clone()).await?;
        
        // Persist completion verification: every side effect must have a persisted evidence entry
        for entry in &evidence_log.entries {
            if matches!(entry.kind, 
                crate::harness::evidence::EvidenceEntryKind::PatchGenerated |
                crate::harness::evidence::EvidenceEntryKind::PatchApplied |
                crate::harness::evidence::EvidenceEntryKind::ValidationCompleted |
                crate::harness::evidence::EvidenceEntryKind::SandboxBackendUsed
            ) {
                tracing::debug!("P0-HARNESS-009: Verified side effect evidence persisted: {}", entry.id);
            } else {
                tracing::warn!("P0-HARNESS-009: Non-side-effect entry found: {}", entry.id);
            }
        }
        
        Ok(())
    }
    
    /// Retrieve evidence log for a work context
    pub async fn retrieve_evidence_log(&self, work_context_id: &str) -> Result<EvidenceLog> {
        let entries = self.sink.retrieve_entries(work_context_id).await?;
        Ok(EvidenceLog {
            execution_id: work_context_id.to_string(),
            entries,
            started_at: Some(chrono::Utc::now()),
            completed_at: None,
        })
    }
    
    /// Get a reference to the underlying sink
    pub fn sink(&self) -> &dyn EvidenceSink {
        self.sink.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::evidence::{EvidenceEntry, EvidenceEntryKind};
    use std::path::PathBuf;
    
    #[tokio::test]
    async fn test_file_evidence_sink_persistence() -> Result<()> {
        let temp_dir = std::env::temp_dir();
        let sink = FileEvidenceSink::new(temp_dir.join("test_evidence"));
        let work_context_id = "test-context";
        
        let entry = EvidenceEntry {
            id: "test-entry".to_string(),
            timestamp: chrono::Utc::now(),
            kind: EvidenceEntryKind::PatchGenerated,
            description: "Test evidence entry".to_string(),
            input_summary: std::collections::HashMap::new(),
            output_summary: std::collections::HashMap::new(),
            command: Some("test-command".to_string()),
            command_result: None,
            files_touched: vec![],
            trace_id: Some("test-trace".to_string()),
            span_id: None,
            duration_ms: 100,
            success: true,
        };
        
        // Persist the entry
        sink.persist_entry(work_context_id, entry).await?;
        
        // Retrieve and verify
        let retrieved = sink.retrieve_entries(work_context_id).await?;
        assert_eq!(retrieved.len(), 1);
        assert_eq!(retrieved[0].id, "test-entry");
        
        Ok(())
    }
}
