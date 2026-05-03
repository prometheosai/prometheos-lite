use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::{debug, error, info, warn};

/// A complete trajectory recording of a harness execution
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Trajectory {
    pub id: String,
    pub work_context_id: String,
    pub steps: Vec<TrajectoryStep>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<TrajectoryMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TrajectoryMetadata {
    pub repo_root: Option<String>,
    pub task_description: Option<String>,
    pub model_used: Option<String>,
    pub harness_version: Option<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrajectoryStep {
    pub step_id: String,
    pub phase: String,
    pub tool_calls: Vec<ToolCallRecord>,
    pub tool_results: Vec<ToolResultRecord>,
    pub errors: Vec<String>,
    pub tokens: Option<u32>,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorded_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCallRecord {
    pub tool: String,
    pub input_summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_input: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolResultRecord {
    pub tool: String,
    pub success: bool,
    pub output_summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_details: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct TrajectoryStats {
    pub total_steps: usize,
    pub total_duration_ms: u64,
    pub total_tokens: u64,
    pub total_tool_calls: usize,
    pub successful_tool_calls: usize,
    pub failed_tool_calls: usize,
    pub total_errors: usize,
    pub phases_used: Vec<String>,
    pub average_step_duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReplayConfig {
    pub max_steps: Option<usize>,
    pub skip_phases: Vec<String>,
    pub step_delay_ms: u64,
    pub simulate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayResult {
    pub trajectory_id: String,
    pub steps_replayed: usize,
    pub steps_skipped: usize,
    pub steps_failed: usize,
    pub total_duration_ms: u64,
    pub divergence_detected: bool,
    pub divergence_details: Vec<String>,
}

pub struct TrajectoryStore {
    storage_path: PathBuf,
}

impl Trajectory {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            work_context_id: id.into(),
            steps: vec![],
            started_at: Utc::now(),
            completed_at: None,
            metadata: None,
        }
    }

    pub fn with_metadata(mut self, metadata: TrajectoryMetadata) -> Self {
        self.metadata = Some(metadata);
        self
    }

    pub fn record_step(&mut self, phase: impl Into<String>, duration_ms: u64, errors: Vec<String>) {
        self.steps.push(TrajectoryStep {
            step_id: uuid::Uuid::new_v4().to_string(),
            phase: phase.into(),
            tool_calls: vec![],
            tool_results: vec![],
            errors,
            tokens: None,
            duration_ms,
            recorded_at: Some(Utc::now()),
        })
    }

    pub fn record_step_with_tools(
        &mut self,
        phase: impl Into<String>,
        duration_ms: u64,
        tool_calls: Vec<ToolCallRecord>,
        tool_results: Vec<ToolResultRecord>,
        errors: Vec<String>,
        tokens: Option<u32>,
    ) {
        self.steps.push(TrajectoryStep {
            step_id: uuid::Uuid::new_v4().to_string(),
            phase: phase.into(),
            tool_calls,
            tool_results,
            errors,
            tokens,
            duration_ms,
            recorded_at: Some(Utc::now()),
        })
    }

    pub fn complete(&mut self) {
        self.completed_at = Some(Utc::now())
    }

    pub fn compute_stats(&self) -> TrajectoryStats {
        let total_steps = self.steps.len();
        let total_duration_ms: u64 = self.steps.iter().map(|s| s.duration_ms).sum();
        let total_tokens: u64 = self
            .steps
            .iter()
            .filter_map(|s| s.tokens.map(|t| t as u64))
            .sum();
        let total_tool_calls: usize = self.steps.iter().map(|s| s.tool_calls.len()).sum();
        let successful_tool_calls: usize = self
            .steps
            .iter()
            .flat_map(|s| &s.tool_results)
            .filter(|r| r.success)
            .count();
        let failed_tool_calls = total_tool_calls.saturating_sub(successful_tool_calls);
        let total_errors: usize = self.steps.iter().map(|s| s.errors.len()).sum();

        let mut phases: std::collections::HashSet<String> =
            self.steps.iter().map(|s| s.phase.clone()).collect();
        let phases_used: Vec<String> = phases.drain().collect();

        let average_step_duration_ms = if total_steps > 0 {
            total_duration_ms / total_steps as u64
        } else {
            0
        };

        TrajectoryStats {
            total_steps,
            total_duration_ms,
            total_tokens,
            total_tool_calls,
            successful_tool_calls,
            failed_tool_calls,
            total_errors,
            phases_used,
            average_step_duration_ms,
        }
    }

    pub fn get_steps_by_phase(&self, phase: &str) -> Vec<&TrajectoryStep> {
        self.steps.iter().filter(|s| s.phase == phase).collect()
    }

    pub fn find_first_error(&self) -> Option<&TrajectoryStep> {
        self.steps.iter().find(|s| !s.errors.is_empty())
    }

    pub fn is_successful(&self) -> bool {
        self.completed_at.is_some()
            && self.steps.iter().all(|s| s.errors.is_empty())
    }

    pub fn total_elapsed_ms(&self) -> u64 {
        let end = self.completed_at.unwrap_or_else(|| Utc::now());
        (end - self.started_at).num_milliseconds() as u64
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).context("Failed to serialize trajectory to JSON")
    }

    pub fn from_json(json: &str) -> Result<Self> {
        serde_json::from_str(json).context("Failed to deserialize trajectory from JSON")
    }
}

impl TrajectoryStore {
    pub fn new(storage_path: impl AsRef<Path>) -> Self {
        Self {
            storage_path: storage_path.as_ref().to_path_buf(),
        }
    }

    pub async fn init(&self) -> Result<()> {
        fs::create_dir_all(&self.storage_path)
            .await
            .context("Failed to create trajectory storage directory")?;
        Ok(())
    }

    pub async fn save(&self, trajectory: &Trajectory) -> Result<PathBuf> {
        self.init().await?;

        let filename = format!("{}_{}.json", trajectory.work_context_id, trajectory.id);
        let filepath = self.storage_path.join(filename);

        let json = trajectory.to_json()?;
        fs::write(&filepath, json)
            .await
            .context("Failed to write trajectory to file")?;

        info!(
            trajectory_id = %trajectory.id,
            path = %filepath.display(),
            "Saved trajectory to storage"
        );

        Ok(filepath)
    }

    pub async fn load(&self, trajectory_id: &str) -> Result<Trajectory> {
        let mut entries = fs::read_dir(&self.storage_path).await.context("Failed to read storage directory")?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if let Some(filename) = path.file_name() {
                let filename_str = filename.to_string_lossy();
                if filename_str.ends_with(&format!("_{}.json", trajectory_id)) {
                    let content = fs::read_to_string(&path).await.context("Failed to read trajectory file")?;
                    return Trajectory::from_json(&content);
                }
            }
        }

        anyhow::bail!("Trajectory not found: {}", trajectory_id)
    }

    pub async fn list_all(&self) -> Result<Vec<Trajectory>> {
        let mut trajectories = Vec::new();

        if !self.storage_path.exists() {
            return Ok(trajectories);
        }

        let mut entries = fs::read_dir(&self.storage_path).await.context("Failed to read storage directory")?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "json") {
                match fs::read_to_string(&path).await {
                    Ok(content) => {
                        if let Ok(trajectory) = Trajectory::from_json(&content) {
                            trajectories.push(trajectory);
                        } else {
                            warn!(path = %path.display(), "Failed to parse trajectory file");
                        }
                    }
                    Err(e) => {
                        warn!(path = %path.display(), error = %e, "Failed to read trajectory file");
                    }
                }
            }
        }

        trajectories.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        Ok(trajectories)
    }

    pub async fn list_by_work_context(&self, work_context_id: &str) -> Result<Vec<Trajectory>> {
        let all = self.list_all().await?;
        let filtered: Vec<_> = all
            .into_iter()
            .filter(|t| t.work_context_id == work_context_id)
            .collect();
        Ok(filtered)
    }

    pub async fn delete(&self, trajectory_id: &str) -> Result<bool> {
        let mut entries = fs::read_dir(&self.storage_path).await.context("Failed to read storage directory")?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if let Some(filename) = path.file_name() {
                let filename_str = filename.to_string_lossy();
                if filename_str.ends_with(&format!("_{}.json", trajectory_id)) {
                    fs::remove_file(&path).await.context("Failed to delete trajectory file")?;
                    info!(trajectory_id = %trajectory_id, "Deleted trajectory from storage");
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    pub async fn stats(&self) -> Result<StorageStats> {
        let trajectories = self.list_all().await?;
        let total_size: u64 = {
            let mut entries = fs::read_dir(&self.storage_path).await?;
            let mut size = 0u64;
            while let Some(entry) = entries.next_entry().await? {
                let metadata = entry.metadata().await?;
                size += metadata.len();
            }
            size
        };

        Ok(StorageStats {
            total_trajectories: trajectories.len(),
            total_size_bytes: total_size,
            completed_trajectories: trajectories.iter().filter(|t| t.completed_at.is_some()).count(),
            failed_trajectories: trajectories.iter().filter(|t| !t.is_successful()).count(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageStats {
    pub total_trajectories: usize,
    pub total_size_bytes: u64,
    pub completed_trajectories: usize,
    pub failed_trajectories: usize,
}

pub async fn replay_trajectory(
    trajectory: &Trajectory,
    config: &ReplayConfig,
) -> Result<ReplayResult> {
    info!(
        trajectory_id = %trajectory.id,
        steps = trajectory.steps.len(),
        "Starting trajectory replay"
    );

    let start = std::time::Instant::now();
    let mut steps_replayed = 0;
    let mut steps_skipped = 0;
    let mut steps_failed = 0;
    let mut divergence_details = Vec::new();

    for (idx, step) in trajectory.steps.iter().enumerate() {
        if let Some(max) = config.max_steps {
            if idx >= max {
                debug!("Reached max steps limit ({})", max);
                break;
            }
        }

        if config.skip_phases.contains(&step.phase) {
            debug!(step_idx = idx, phase = %step.phase, "Skipping phase");
            steps_skipped += 1;
            continue;
        }

        if config.step_delay_ms > 0 {
            tokio::time::sleep(tokio::time::Duration::from_millis(config.step_delay_ms)).await;
        }

        if config.simulate {
            debug!(
                step_idx = idx,
                phase = %step.phase,
                tool_calls = step.tool_calls.len(),
                "Simulating step"
            );
            steps_replayed += 1;
        } else {
            warn!("Non-simulated replay not yet implemented - treating as simulation");
            steps_replayed += 1;
        }

        if !step.errors.is_empty() {
            divergence_details.push(format!(
                "Step {} ({}) had {} errors in original",
                idx,
                step.phase,
                step.errors.len()
            ));
        }
    }

    let total_duration_ms = start.elapsed().as_millis() as u64;

    let result = ReplayResult {
        trajectory_id: trajectory.id.clone(),
        steps_replayed,
        steps_skipped,
        steps_failed,
        total_duration_ms,
        divergence_detected: !divergence_details.is_empty(),
        divergence_details,
    };

    info!(
        trajectory_id = %trajectory.id,
        steps_replayed = result.steps_replayed,
        "Completed trajectory replay"
    );

    Ok(result)
}

pub mod export {
    use super::*;

    pub fn to_text_report(trajectory: &Trajectory) -> String {
        let mut report = String::new();
        
        report.push_str(&format!("Trajectory Report\n"));
        report.push_str(&format!("==================\n\n"));
        report.push_str(&format!("ID: {}\n", trajectory.id));
        report.push_str(&format!("Work Context: {}\n", trajectory.work_context_id));
        report.push_str(&format!("Started: {}\n", trajectory.started_at.format("%Y-%m-%d %H:%M:%S UTC")));
        if let Some(completed) = trajectory.completed_at {
            report.push_str(&format!("Completed: {}\n", completed.format("%Y-%m-%d %H:%M:%S UTC")));
            let duration = trajectory.total_elapsed_ms();
            report.push_str(&format!("Duration: {}ms\n", duration));
        } else {
            report.push_str("Status: In Progress\n");
        }
        
        report.push_str(&format!("\nSteps: {}\n", trajectory.steps.len()));
        report.push_str(&format!("Successful: {}\n\n", if trajectory.is_successful() { "Yes" } else { "No" }));
        
        let stats = trajectory.compute_stats();
        report.push_str(&format!("Statistics\n"));
        report.push_str(&format!("----------\n"));
        report.push_str(&format!("Total Steps: {}\n", stats.total_steps));
        report.push_str(&format!("Total Duration: {}ms\n", stats.total_duration_ms));
        report.push_str(&format!("Average Step Duration: {}ms\n", stats.average_step_duration_ms));
        report.push_str(&format!("Total Tool Calls: {}\n", stats.total_tool_calls));
        report.push_str(&format!("Successful Tool Calls: {}\n", stats.successful_tool_calls));
        report.push_str(&format!("Failed Tool Calls: {}\n", stats.failed_tool_calls));
        report.push_str(&format!("Total Errors: {}\n", stats.total_errors));
        report.push_str(&format!("Phases Used: {}\n\n", stats.phases_used.join(", ")));
        
        report.push_str(&format!("Step Details\n"));
        report.push_str(&format!("------------\n"));
        for (idx, step) in trajectory.steps.iter().enumerate() {
            report.push_str(&format!("\n[Step {}] {}\n", idx + 1, step.phase));
            report.push_str(&format!("  Duration: {}ms\n", step.duration_ms));
            if let Some(tokens) = step.tokens {
                report.push_str(&format!("  Tokens: {}\n", tokens));
            }
            if !step.tool_calls.is_empty() {
                report.push_str(&format!("  Tool Calls: {}\n", step.tool_calls.len()));
                for call in &step.tool_calls {
                    report.push_str(&format!("    - {}: {}\n", call.tool, call.input_summary));
                }
            }
            if !step.errors.is_empty() {
                report.push_str(&format!("  Errors:\n"));
                for error in &step.errors {
                    report.push_str(&format!("    ! {}\n", error));
                }
            }
        }
        
        report
    }

    pub fn to_markdown(trajectory: &Trajectory) -> String {
        let mut md = String::new();
        
        md.push_str(&format!("# Trajectory Report: {}\n\n", trajectory.id));
        md.push_str(&format!("**Work Context:** {}\n\n", trajectory.work_context_id));
        md.push_str(&format!("**Started:** {}\n", trajectory.started_at.format("%Y-%m-%d %H:%M:%S UTC")));
        if let Some(completed) = trajectory.completed_at {
            md.push_str(&format!("**Completed:** {}\n", completed.format("%Y-%m-%d %H:%M:%S UTC")));
        }
        md.push_str(&format!("**Status:** {}\n\n", if trajectory.is_successful() { "✅ Successful" } else { "❌ Failed/Incomplete" }));
        
        let stats = trajectory.compute_stats();
        md.push_str("## Statistics\n\n");
        md.push_str("| Metric | Value |\n");
        md.push_str("|--------|-------|\n");
        md.push_str(&format!("| Total Steps | {} |\n", stats.total_steps));
        md.push_str(&format!("| Total Duration | {}ms |\n", stats.total_duration_ms));
        md.push_str(&format!("| Tool Calls | {} |\n", stats.total_tool_calls));
        md.push_str(&format!("| Successful Calls | {} |\n", stats.successful_tool_calls));
        md.push_str(&format!("| Failed Calls | {} |\n", stats.failed_tool_calls));
        md.push_str(&format!("| Errors | {} |\n\n", stats.total_errors));
        
        md.push_str("## Steps\n\n");
        for (idx, step) in trajectory.steps.iter().enumerate() {
            let status = if step.errors.is_empty() { "✅" } else { "❌" };
            md.push_str(&format!("### Step {}: {} {}\n\n", idx + 1, step.phase, status));
            md.push_str(&format!("- **Duration:** {}ms\n", step.duration_ms));
            if let Some(tokens) = step.tokens {
                md.push_str(&format!("- **Tokens:** {}\n", tokens));
            }
            
            if !step.tool_calls.is_empty() {
                md.push_str("\n**Tool Calls:**\n\n");
                for call in &step.tool_calls {
                    md.push_str(&format!("- `{}`: {}\n", call.tool, call.input_summary));
                }
            }
            
            if !step.errors.is_empty() {
                md.push_str("\n**Errors:**\n\n");
                for error in &step.errors {
                    md.push_str(&format!("- ⚠️ {}\n", error));
                }
            }
            md.push('\n');
        }
        
        md
    }

    pub fn to_csv(trajectory: &Trajectory) -> String {
        let mut csv = String::new();
        csv.push_str("step_id,phase,duration_ms,tokens,tool_calls,errors,success\n");
        
        for step in &trajectory.steps {
            let success = if step.errors.is_empty() { "true" } else { "false" };
            let tokens = step.tokens.map_or(String::new(), |t| t.to_string());
            csv.push_str(&format!(
                "{},{},{},{},{},{},{}\n",
                step.step_id,
                step.phase,
                step.duration_ms,
                tokens,
                step.tool_calls.len(),
                step.errors.len(),
                success
            ));
        }
        
        csv
    }
}

pub mod analysis {
    use super::*;

    pub fn compare_trajectories(a: &Trajectory, b: &Trajectory) -> TrajectoryComparison {
        let stats_a = a.compute_stats();
        let stats_b = b.compute_stats();
        
        let duration_diff_ms = stats_b.total_duration_ms as i64 - stats_a.total_duration_ms as i64;
        let step_count_diff = stats_b.total_steps as i64 - stats_a.total_steps as i64;
        let error_count_diff = stats_b.total_errors as i64 - stats_a.total_errors as i64;
        
        let phases_a: std::collections::HashSet<_> = a.steps.iter().map(|s| &s.phase).collect();
        let phases_b: std::collections::HashSet<_> = b.steps.iter().map(|s| &s.phase).collect();
        let common_phases: Vec<_> = phases_a.intersection(&phases_b).cloned().cloned().collect();
        
        TrajectoryComparison {
            trajectory_a_id: a.id.clone(),
            trajectory_b_id: b.id.clone(),
            duration_diff_ms,
            step_count_diff,
            error_count_diff,
            common_phases,
            is_regression: error_count_diff > 0 || duration_diff_ms > 1000,
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct TrajectoryComparison {
        pub trajectory_a_id: String,
        pub trajectory_b_id: String,
        pub duration_diff_ms: i64,
        pub step_count_diff: i64,
        pub error_count_diff: i64,
        pub common_phases: Vec<String>,
        pub is_regression: bool,
    }

    pub fn aggregate_stats(trajectories: &[Trajectory]) -> AggregatedStats {
        if trajectories.is_empty() {
            return AggregatedStats::default();
        }
        
        let total_trajectories = trajectories.len();
        let completed = trajectories.iter().filter(|t| t.completed_at.is_some()).count();
        let successful = trajectories.iter().filter(|t| t.is_successful()).count();
        
        let total_duration_ms: u64 = trajectories.iter().map(|t| t.total_elapsed_ms()).sum();
        let avg_duration_ms = total_duration_ms / total_trajectories as u64;
        
        let total_steps: usize = trajectories.iter().map(|t| t.steps.len()).sum();
        let avg_steps = total_steps / total_trajectories;
        
        let total_errors: usize = trajectories
            .iter()
            .map(|t| t.steps.iter().map(|s| s.errors.len()).sum::<usize>())
            .sum();
        
        AggregatedStats {
            total_trajectories,
            completed_trajectories: completed,
            successful_trajectories: successful,
            failed_trajectories: total_trajectories - successful,
            total_duration_ms,
            average_duration_ms: avg_duration_ms,
            total_steps,
            average_steps: avg_steps,
            total_errors,
            success_rate: successful as f64 / total_trajectories as f64,
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    pub struct AggregatedStats {
        pub total_trajectories: usize,
        pub completed_trajectories: usize,
        pub successful_trajectories: usize,
        pub failed_trajectories: usize,
        pub total_duration_ms: u64,
        pub average_duration_ms: u64,
        pub total_steps: usize,
        pub average_steps: usize,
        pub total_errors: usize,
        pub success_rate: f64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_trajectory_creation() {
        let mut traj = Trajectory::new("test-work-context");
        assert_eq!(traj.work_context_id, "test-work-context");
        assert!(traj.steps.is_empty());
        assert!(traj.completed_at.is_none());
        
        traj.record_step("test.phase", 100, vec![]);
        assert_eq!(traj.steps.len(), 1);
        assert_eq!(traj.steps[0].phase, "test.phase");
        
        traj.complete();
        assert!(traj.completed_at.is_some());
    }

    #[test]
    fn test_trajectory_stats() {
        let mut traj = Trajectory::new("test");
        traj.record_step("phase1", 100, vec![]);
        traj.record_step("phase2", 200, vec!["error1".to_string()]);
        
        let stats = traj.compute_stats();
        assert_eq!(stats.total_steps, 2);
        assert_eq!(stats.total_duration_ms, 300);
        assert_eq!(stats.total_errors, 1);
        assert_eq!(stats.average_step_duration_ms, 150);
    }

    #[test]
    fn test_trajectory_serialization() {
        let mut traj = Trajectory::new("test");
        traj.record_step("phase", 100, vec![]);
        
        let json = traj.to_json().unwrap();
        let deserialized = Trajectory::from_json(&json).unwrap();
        
        assert_eq!(traj.id, deserialized.id);
        assert_eq!(traj.steps.len(), deserialized.steps.len());
    }

    #[test]
    fn test_text_report_generation() {
        let mut traj = Trajectory::new("test-context");
        traj.record_step("execution", 150, vec![]);
        traj.complete();
        
        let report = export::to_text_report(&traj);
        assert!(report.contains("Trajectory Report"));
        assert!(report.contains("test-context"));
        assert!(report.contains("execution"));
    }

    #[test]
    fn test_markdown_export() {
        let mut traj = Trajectory::new("test");
        traj.record_step("step1", 100, vec![]);
        
        let md = export::to_markdown(&traj);
        assert!(md.contains("# Trajectory Report"));
        assert!(md.contains("Step 1"));
    }

    #[test]
    fn test_csv_export() {
        let mut traj = Trajectory::new("test");
        traj.record_step("phase", 100, vec![]);
        
        let csv = export::to_csv(&traj);
        assert!(csv.contains("step_id,phase,duration_ms"));
        assert!(csv.contains("phase"));
    }

    #[test]
    fn test_trajectory_comparison() {
        let mut traj_a = Trajectory::new("a");
        traj_a.record_step("phase", 100, vec![]);
        
        let mut traj_b = Trajectory::new("b");
        traj_b.record_step("phase", 200, vec!["error".to_string()]);
        
        let comparison = analysis::compare_trajectories(&traj_a, &traj_b);
        assert_eq!(comparison.duration_diff_ms, 100);
        assert_eq!(comparison.error_count_diff, 1);
        assert!(comparison.is_regression);
    }

    #[test]
    fn test_aggregated_stats() {
        let mut traj1 = Trajectory::new("1");
        traj1.record_step("p", 100, vec![]);
        traj1.complete();
        
        let mut traj2 = Trajectory::new("2");
        traj2.record_step("p", 200, vec!["err".to_string()]);
        
        let agg = analysis::aggregate_stats(&[traj1, traj2]);
        assert_eq!(agg.total_trajectories, 2);
        assert_eq!(agg.completed_trajectories, 1);
        assert_eq!(agg.total_errors, 1);
    }

    #[tokio::test]
    async fn test_trajectory_store() {
        let temp_dir = std::env::temp_dir().join(format!("trajectory_test_{}", uuid::Uuid::new_v4()));
        let store = TrajectoryStore::new(&temp_dir);
        
        let mut traj = Trajectory::new("test-context");
        traj.record_step("phase", 100, vec![]);
        let path = store.save(&traj).await.unwrap();
        assert!(path.exists());
        
        let loaded = store.load(&traj.id).await.unwrap();
        assert_eq!(loaded.id, traj.id);
        
        let all = store.list_all().await.unwrap();
        assert_eq!(all.len(), 1);
        
        let deleted = store.delete(&traj.id).await.unwrap();
        assert!(deleted);
        
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }

    #[tokio::test]
    async fn test_trajectory_replay() {
        let mut traj = Trajectory::new("test");
        traj.record_step("phase1", 100, vec![]);
        traj.record_step("phase2", 200, vec![]);
        
        let config = ReplayConfig {
            max_steps: None,
            skip_phases: vec![],
            step_delay_ms: 0,
            simulate: true,
        };
        
        let result = replay_trajectory(&traj, &config).await.unwrap();
        assert_eq!(result.steps_replayed, 2);
        assert_eq!(result.steps_skipped, 0);
    }
}
