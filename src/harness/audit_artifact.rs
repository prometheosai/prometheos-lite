//! P0-Audit-010: Machine-readable audit artifact output

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// P0-Audit-010: Machine-readable audit artifact for every harness run
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditArtifact {
    /// Unique work context identifier
    pub work_context_id: String,
    /// Trace ID for correlation
    pub trace_id: Option<String>,
    /// Timestamp when audit was created
    pub timestamp: u64,
    /// Repository path
    pub repo_path: PathBuf,
    /// Harness mode used
    pub mode: String,
    /// Task description
    pub task: String,
    /// RepoMap quality metrics
    pub repo_map_quality: RepoMapQuality,
    /// Patch identity verification
    pub patch_identity: PatchIdentity,
    /// Validation results
    pub validation: ValidationAudit,
    /// Review results
    pub review: ReviewAudit,
    /// Risk assessment
    pub risk: RiskAudit,
    /// Sandbox evidence
    pub sandbox: SandboxAudit,
    /// Final completion decision
    pub completion_decision: String,
    /// Remaining warnings
    pub remaining_warnings: Vec<String>,
    /// Execution duration in milliseconds
    pub execution_duration_ms: u64,
}

/// RepoMap quality metrics
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RepoMapQuality {
    /// Parser backend used
    pub parser_backend: String,
    /// Number of files parsed
    pub files_parsed: usize,
    /// Number of parse errors
    pub parse_errors: usize,
    /// Number of symbols extracted
    pub symbols_extracted: usize,
    /// Confidence score (0.0-1.0)
    pub confidence: f32,
}

/// Patch identity verification
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PatchIdentity {
    /// Hash of generated patch
    pub generated_hash: Option<String>,
    /// Hash of dry-run patch
    pub dry_run_hash: Option<String>,
    /// Hash of applied patch
    pub applied_hash: Option<String>,
    /// Whether hashes match
    pub hashes_match: bool,
    /// Patch size in bytes
    pub patch_size_bytes: usize,
}

/// Validation audit information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValidationAudit {
    /// Number of commands planned
    pub commands_planned: usize,
    /// Number of commands executed
    pub commands_executed: usize,
    /// Number of commands skipped
    pub commands_skipped: usize,
    /// Number of commands failed
    pub commands_failed: usize,
    /// Overall validation status
    pub status: String,
    /// Validation duration in milliseconds
    pub duration_ms: u64,
}

/// Review audit information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReviewAudit {
    /// Number of files reviewed
    pub files_reviewed: usize,
    /// Number of lines analyzed
    pub lines_analyzed: usize,
    /// Number of security patterns checked
    pub security_patterns_checked: usize,
    /// Number of issues found
    pub issues_found: usize,
    /// Review quality score
    pub quality_score: f32,
    /// AST analysis enabled
    pub ast_analysis_enabled: bool,
    /// Review duration in milliseconds
    pub duration_ms: u64,
}

/// Risk audit information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RiskAudit {
    /// Overall risk level
    pub risk_level: String,
    /// Number of security issues
    pub security_issues: usize,
    /// Number of API changes
    pub api_changes: usize,
    /// Number of dependency changes
    pub dependency_changes: usize,
    /// Risk approval required
    pub approval_required: bool,
    /// Risk assessment duration in milliseconds
    pub duration_ms: u64,
}

/// Sandbox audit information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SandboxAudit {
    /// Runtime kind used
    pub runtime_kind: String,
    /// Process isolation
    pub isolated_process: bool,
    /// Filesystem isolation
    pub isolated_filesystem: bool,
    /// Network disabled
    pub network_disabled: bool,
    /// CPU limited
    pub cpu_limited: bool,
    /// Memory limited
    pub memory_limited: bool,
    /// No new privileges
    pub no_new_privileges: bool,
    /// Capabilities dropped
    pub capabilities_dropped: bool,
    /// Seccomp enabled
    pub seccomp_enabled: bool,
    /// PIDs limit
    pub pids_limit: Option<u32>,
    /// Non-root user
    pub non_root_user: bool,
    /// Tmpfs protected
    pub tmpfs_protected: bool,
    /// Container ID if applicable
    pub container_id: Option<String>,
}

impl AuditArtifact {
    /// Create a new audit artifact
    pub fn new(
        work_context_id: String,
        trace_id: Option<String>,
        repo_path: PathBuf,
        mode: String,
        task: String,
    ) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Self {
            work_context_id,
            trace_id,
            timestamp,
            repo_path,
            mode,
            task,
            repo_map_quality: RepoMapQuality::default(),
            patch_identity: PatchIdentity::default(),
            validation: ValidationAudit::default(),
            review: ReviewAudit::default(),
            risk: RiskAudit::default(),
            sandbox: SandboxAudit::default(),
            completion_decision: "Unknown".to_string(),
            remaining_warnings: Vec::new(),
            execution_duration_ms: 0,
        }
    }

    /// Export audit artifact to JSON file
    pub async fn export_to_file(&self, output_dir: &PathBuf) -> Result<PathBuf> {
        use tokio::fs;

        // Create output directory if it doesn't exist
        fs::create_dir_all(output_dir).await?;

        // Generate filename with timestamp
        let filename = format!("audit-{}.json", self.timestamp);
        let output_path = output_dir.join(filename);

        // Write JSON to file
        let json_content = serde_json::to_string_pretty(self)?;
        fs::write(&output_path, json_content).await?;

        Ok(output_path)
    }

    /// Export audit artifact to console
    pub fn export_to_console(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| {
            "Failed to serialize audit artifact".to_string()
        })
    }

    /// Validate audit artifact completeness
    pub fn validate_completeness(&self) -> Vec<String> {
        let mut warnings = Vec::new();

        // Check critical fields
        if self.work_context_id.is_empty() {
            warnings.push("Missing work context ID".to_string());
        }

        if self.repo_map_quality.confidence < 0.5 {
            warnings.push("Low RepoMap confidence".to_string());
        }

        if self.validation.commands_executed == 0 {
            warnings.push("Zero validation commands executed".to_string());
        }

        if self.review.files_reviewed == 0 {
            warnings.push("Zero files reviewed".to_string());
        }

        if self.patch_identity.generated_hash.is_none() {
            warnings.push("Missing patch hash".to_string());
        }

        if self.sandbox.runtime_kind == "Local" && self.mode == "Autonomous" {
            warnings.push("Local runtime used in autonomous mode".to_string());
        }

        if !self.sandbox.isolated_process && self.mode == "Autonomous" {
            warnings.push("Process isolation not enforced in autonomous mode".to_string());
        }

        warnings
    }
}

impl Default for RepoMapQuality {
    fn default() -> Self {
        Self {
            parser_backend: "Unknown".to_string(),
            files_parsed: 0,
            parse_errors: 0,
            symbols_extracted: 0,
            confidence: 0.0,
        }
    }
}

impl Default for PatchIdentity {
    fn default() -> Self {
        Self {
            generated_hash: None,
            dry_run_hash: None,
            applied_hash: None,
            hashes_match: false,
            patch_size_bytes: 0,
        }
    }
}

impl Default for ValidationAudit {
    fn default() -> Self {
        Self {
            commands_planned: 0,
            commands_executed: 0,
            commands_skipped: 0,
            commands_failed: 0,
            status: "Unknown".to_string(),
            duration_ms: 0,
        }
    }
}

impl Default for ReviewAudit {
    fn default() -> Self {
        Self {
            files_reviewed: 0,
            lines_analyzed: 0,
            security_patterns_checked: 0,
            issues_found: 0,
            quality_score: 0.0,
            ast_analysis_enabled: false,
            duration_ms: 0,
        }
    }
}

impl Default for RiskAudit {
    fn default() -> Self {
        Self {
            risk_level: "Unknown".to_string(),
            security_issues: 0,
            api_changes: 0,
            dependency_changes: 0,
            approval_required: false,
            duration_ms: 0,
        }
    }
}

impl Default for SandboxAudit {
    fn default() -> Self {
        Self {
            runtime_kind: "Unknown".to_string(),
            isolated_process: false,
            isolated_filesystem: false,
            network_disabled: false,
            cpu_limited: false,
            memory_limited: false,
            no_new_privileges: false,
            capabilities_dropped: false,
            seccomp_enabled: false,
            pids_limit: None,
            non_root_user: false,
            tmpfs_protected: false,
            container_id: None,
        }
    }
}
