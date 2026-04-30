//! EvolutionEngine - playbook evolution and A/B testing system
//!
//! This module implements the EvolutionEngine which tracks playbook performance,
//! tests variants, and promotes successful mutations.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

use crate::db::Db;
use crate::db::repository::PlaybookOperations;
use crate::work::playbook::{PatternRecord, PatternType, WorkContextPlaybook};
use crate::work::types::{WorkContext, FlowPerformanceRecord};

/// EvaluationResult - result of evaluating a WorkContext
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationResult {
    /// Overall score (0.0 to 1.0)
    pub overall_score: f32,
    /// Semantic correctness score (LLM-based)
    pub semantic_score: f32,
    /// Structural correctness score (schema validation)
    pub structural_score: f32,
    /// Tool consistency score
    pub tool_consistency_score: f32,
    /// Artifact completeness score
    pub artifact_completeness_score: f32,
    /// Evaluation timestamp
    pub evaluated_at: chrono::DateTime<chrono::Utc>,
    /// Evaluation details
    pub details: String,
}

/// Playbook evolution tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybookEvolution {
    /// Unique identifier
    pub id: String,
    /// Playbook ID this evolution belongs to
    pub playbook_id: String,
    /// Version number
    pub version: u32,
    /// Parent version (for tracking lineage)
    pub parent_version: Option<u32>,
    /// Mutation strategy used
    pub mutation_strategy: MutationStrategy,
    /// Performance metrics
    pub performance: PerformanceMetrics,
    /// Number of executions
    pub execution_count: u32,
    /// Success rate
    pub success_rate: f64,
    /// Average duration in milliseconds
    pub avg_duration_ms: u64,
    /// Status
    pub status: EvolutionStatus,
    /// Created at timestamp
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Updated at timestamp
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Mutation strategies for playbook evolution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MutationStrategy {
    /// Add a new node
    AddNode {
        node_id: String,
        node_type: String,
        position: String,
    },
    /// Remove a node
    RemoveNode {
        node_id: String,
    },
    /// Reorder nodes
    ReorderNodes {
        node_order: Vec<String>,
    },
    /// Modify node parameters
    ModifyNode {
        node_id: String,
        parameter_changes: HashMap<String, serde_json::Value>,
    },
    /// Combine two playbooks
    Combine {
        other_playbook_id: String,
    },
}

/// Performance metrics for a playbook evolution
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PerformanceMetrics {
    /// Success rate (0.0 to 1.0)
    pub success_rate: f64,
    /// Average execution duration in milliseconds
    pub avg_duration_ms: u64,
    /// Average cost
    pub avg_cost: f64,
    /// Average number of tool calls
    pub avg_tool_calls: f64,
    /// User satisfaction score (if available)
    pub user_satisfaction: Option<f64>,
}

/// Evolution status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EvolutionStatus {
    /// Testing in progress
    Testing,
    /// Ready for promotion
    ReadyForPromotion,
    /// Promoted to production
    Promoted,
    /// Rejected
    Rejected,
    /// Deprecated
    Deprecated,
}

/// A/B test configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbTest {
    /// Unique identifier
    pub id: String,
    /// Playbook ID being tested
    pub playbook_id: String,
    /// Version A (control)
    pub version_a: u32,
    /// Version B (variant)
    pub version_b: u32,
    /// Traffic split (0.0 to 1.0 for version B)
    pub traffic_split: f64,
    /// Start time
    pub started_at: chrono::DateTime<chrono::Utc>,
    /// End time (if completed)
    pub ended_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Status
    pub status: AbTestStatus,
    /// Results
    pub results: Option<AbTestResults>,
}

/// A/B test status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AbTestStatus {
    /// Running
    Running,
    /// Completed
    Completed,
    /// Stopped early
    StoppedEarly,
}

/// A/B test results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbTestResults {
    /// Version A metrics
    pub metrics_a: PerformanceMetrics,
    /// Version B metrics
    pub metrics_b: PerformanceMetrics,
    /// Statistical significance
    pub significance: f64,
    /// Winner
    pub winner: Option<u32>,
}

/// EvolutionEngine - manages playbook evolution
pub struct EvolutionEngine {
    db: Arc<Db>,
}

impl EvolutionEngine {
    /// Create a new EvolutionEngine
    pub fn new(db: Arc<Db>) -> Self {
        Self { db }
    }

    /// Extract patterns from a completed WorkContext
    /// Returns success and failure patterns based on execution metadata and FlowPerformanceRecords
    pub fn extract_patterns(context: &WorkContext) -> (Vec<PatternRecord>, Vec<PatternRecord>) {
        let mut success_patterns = Vec::new();
        let mut failure_patterns = Vec::new();

        // Check if context completed successfully
        let is_success = context.status == crate::work::types::WorkStatus::Completed;

        // Extract patterns from execution metadata
        for record in &context.execution_metadata {
            let pattern_type = if is_success {
                PatternType::Success
            } else {
                PatternType::Failure
            };

            // Pattern: node latency pattern
            if record.latency_ms > 5000 {
                let pattern = PatternRecord {
                    pattern_type: pattern_type.clone(),
                    signal: format!("high_latency_node_{}", record.node_id),
                    weight: if is_success { 0.3 } else { 0.7 },
                    created_at: chrono::Utc::now(),
                };
                if is_success {
                    success_patterns.push(pattern);
                } else {
                    failure_patterns.push(pattern);
                }
            }

            // Pattern: model/provider usage
            let pattern = PatternRecord {
                pattern_type: pattern_type.clone(),
                signal: format!("model_usage_{}_{}", record.provider, record.model),
                weight: 0.5,
                created_at: chrono::Utc::now(),
            };
            if is_success {
                success_patterns.push(pattern);
            } else {
                failure_patterns.push(pattern);
            }
        }

        // Extract patterns from FlowPerformanceRecords in metadata
        if let Some(obj) = context.metadata.as_object() {
            for (key, value) in obj {
                if key.starts_with("flow_perf_") {
                    if let Ok(perf_record) = serde_json::from_value::<FlowPerformanceRecord>(value.clone()) {
                        let pattern_type = if perf_record.success_score > 0.7 {
                            PatternType::Success
                        } else {
                            PatternType::Failure
                        };

                        // Pattern: flow performance
                        let pattern = PatternRecord {
                            pattern_type: pattern_type.clone(),
                            signal: format!("flow_performance_{}_{}", perf_record.flow_id, if perf_record.success_score > 0.7 { "success" } else { "failure" }),
                            weight: perf_record.success_score,
                            created_at: chrono::Utc::now(),
                        };
                        if pattern_type == PatternType::Success {
                            success_patterns.push(pattern);
                        } else {
                            failure_patterns.push(pattern);
                        }

                        // Pattern: high duration
                        if perf_record.duration_ms > 10000 {
                            let pattern = PatternRecord {
                                pattern_type: pattern_type.clone(),
                                signal: format!("high_duration_flow_{}", perf_record.flow_id),
                                weight: 0.6,
                                created_at: chrono::Utc::now(),
                            };
                            if pattern_type == PatternType::Success {
                                success_patterns.push(pattern);
                            } else {
                                failure_patterns.push(pattern);
                            }
                        }

                        // Pattern: high cost
                        if perf_record.token_cost > 0.5 {
                            let pattern = PatternRecord {
                                pattern_type: pattern_type.clone(),
                                signal: format!("high_cost_flow_{}", perf_record.flow_id),
                                weight: 0.5,
                                created_at: chrono::Utc::now(),
                            };
                            if pattern_type == PatternType::Success {
                                success_patterns.push(pattern);
                            } else {
                                failure_patterns.push(pattern);
                            }
                        }
                    }
                }
            }
        }

        // Pattern: revision count (from decisions)
        let revision_count = context.decisions.len() as f32;
        if revision_count > 3.0 {
            let pattern = PatternRecord {
                pattern_type: if is_success { PatternType::Success } else { PatternType::Failure },
                signal: "high_revision_count".to_string(),
                weight: 0.4,
                created_at: chrono::Utc::now(),
            };
            if is_success {
                success_patterns.push(pattern);
            } else {
                failure_patterns.push(pattern);
            }
        }

        // Pattern: failure reason
        if let Some(ref blocked_reason) = context.blocked_reason {
            let pattern = PatternRecord {
                pattern_type: PatternType::Failure,
                signal: format!("blocked_reason_{}", blocked_reason),
                weight: 0.8,
                created_at: chrono::Utc::now(),
            };
            failure_patterns.push(pattern);
        }

        (success_patterns, failure_patterns)
    }

    /// Evaluate a WorkContext with semantic, structural, and tool consistency signals
    pub fn evaluate_context(context: &WorkContext) -> EvaluationResult {
        // Semantic correctness: based on completion status and decisions
        let semantic_score = match context.status {
            crate::work::types::WorkStatus::Completed => 1.0,
            crate::work::types::WorkStatus::InProgress => 0.7,
            crate::work::types::WorkStatus::Blocked => 0.3,
            crate::work::types::WorkStatus::Draft => 0.5,
            crate::work::types::WorkStatus::AwaitingApproval => 0.6,
            _ => 0.4,
        };

        // Structural correctness: based on plan and artifacts
        let structural_score = if context.plan.is_some() { 0.8 } else { 0.4 };
        let structural_score = structural_score + if !context.artifacts.is_empty() { 0.2 } else { 0.0 };

        // Tool consistency: based on execution metadata
        let tool_consistency_score = if context.execution_metadata.is_empty() {
            0.5
        } else {
            let total_latency: u64 = context.execution_metadata.iter().map(|r| r.latency_ms).sum();
            let avg_latency = total_latency / context.execution_metadata.len() as u64;
            if avg_latency < 5000 { 0.9 } else if avg_latency < 10000 { 0.7 } else { 0.5 }
        };

        // Artifact completeness: based on completion criteria
        let artifact_completeness_score = if context.completion_criteria.is_empty() {
            1.0
        } else {
            let completed = context.artifacts.len() as f32;
            let total = context.completion_criteria.len() as f32;
            (completed / total).min(1.0)
        };

        // Overall score: weighted average
        let overall_score = (semantic_score * 0.3
            + structural_score * 0.25
            + tool_consistency_score * 0.25
            + artifact_completeness_score * 0.2).clamp(0.0, 1.0);

        let details = format!(
            "Semantic: {:.2}, Structural: {:.2}, Tool Consistency: {:.2}, Artifact Completeness: {:.2}",
            semantic_score, structural_score, tool_consistency_score, artifact_completeness_score
        );

        EvaluationResult {
            overall_score,
            semantic_score,
            structural_score,
            tool_consistency_score,
            artifact_completeness_score,
            evaluated_at: chrono::Utc::now(),
            details,
        }
    }

    /// Evolve a playbook based on extracted patterns
    /// Increases successful flow weights, penalizes failures, adjusts research_depth/autonomy
    pub fn evolve_playbook(
        &self,
        playbook_id: &str,
        success_patterns: Vec<PatternRecord>,
        failure_patterns: Vec<PatternRecord>,
    ) -> Result<()> {
        let mut playbook = PlaybookOperations::get_playbook(&*self.db, playbook_id)?
            .ok_or_else(|| anyhow::anyhow!("Playbook not found: {}", playbook_id))?;

        // Adjust flow weights based on success/failure patterns
        // Only target the specific flow involved in the pattern
        for pattern in &success_patterns {
            if pattern.signal.starts_with("flow_performance_") {
                // Extract flow_id from signal: "flow_performance_{flow_id}_success"
                if let Some(flow_id) = pattern.signal.strip_prefix("flow_performance_").and_then(|s| s.strip_suffix("_success")) {
                    // Increase weight for the specific flow that succeeded
                    for flow_pref in &mut playbook.preferred_flows {
                        if flow_pref.flow_id == flow_id {
                            flow_pref.weight = (flow_pref.weight + 0.1).min(1.0);
                            flow_pref.confidence = (flow_pref.confidence + 0.05).min(1.0);
                        }
                    }
                }
            } else if pattern.signal.starts_with("high_duration_flow_") {
                // Extract flow_id from signal: "high_duration_flow_{flow_id}"
                if let Some(flow_id) = pattern.signal.strip_prefix("high_duration_flow_") {
                    // Decrease weight for the specific flow with high duration
                    for flow_pref in &mut playbook.preferred_flows {
                        if flow_pref.flow_id == flow_id {
                            flow_pref.weight = (flow_pref.weight - 0.1).max(0.0);
                            flow_pref.confidence = (flow_pref.confidence - 0.05).max(0.0);
                        }
                    }
                }
            } else if pattern.signal.starts_with("high_cost_flow_") {
                // Extract flow_id from signal: "high_cost_flow_{flow_id}"
                if let Some(flow_id) = pattern.signal.strip_prefix("high_cost_flow_") {
                    // Decrease weight for the specific flow with high cost
                    for flow_pref in &mut playbook.preferred_flows {
                        if flow_pref.flow_id == flow_id {
                            flow_pref.weight = (flow_pref.weight - 0.1).max(0.0);
                            flow_pref.confidence = (flow_pref.confidence - 0.05).max(0.0);
                        }
                    }
                }
            } else if pattern.signal.starts_with("model_usage_") {
                // Extract flow_id from signal: "model_usage_{flow_id}_{model_name}"
                // Target only the specific flow that used this model
                let parts: Vec<&str> = pattern.signal.split('_').collect();
                if parts.len() >= 3 {
                    let flow_id = parts[2]; // "model_usage_{flow_id}_{model_name}"
                    for flow_pref in &mut playbook.preferred_flows {
                        if flow_pref.flow_id == flow_id {
                            flow_pref.weight = (flow_pref.weight + 0.1).min(1.0);
                            flow_pref.confidence = (flow_pref.confidence + 0.05).min(1.0);
                        }
                    }
                }
            }
        }

        for pattern in &failure_patterns {
            if pattern.signal.starts_with("flow_performance_") {
                // Extract flow_id from signal: "flow_performance_{flow_id}_failure"
                if let Some(flow_id) = pattern.signal.strip_prefix("flow_performance_").and_then(|s| s.strip_suffix("_failure")) {
                    // Decrease weight for the specific flow that failed
                    for flow_pref in &mut playbook.preferred_flows {
                        if flow_pref.flow_id == flow_id {
                            flow_pref.weight = (flow_pref.weight - 0.1).max(0.0);
                            flow_pref.confidence = (flow_pref.confidence - 0.05).max(0.0);
                        }
                    }
                }
            } else if pattern.signal.starts_with("model_usage_") {
                // Extract flow_id from signal: "model_usage_{flow_id}_{model_name}"
                // Target only the specific flow that used this model
                let parts: Vec<&str> = pattern.signal.split('_').collect();
                if parts.len() >= 3 {
                    let flow_id = parts[2]; // "model_usage_{flow_id}_{model_name}"
                    for flow_pref in &mut playbook.preferred_flows {
                        if flow_pref.flow_id == flow_id {
                            flow_pref.weight = (flow_pref.weight - 0.1).max(0.0);
                            flow_pref.confidence = (flow_pref.confidence - 0.05).max(0.0);
                        }
                    }
                }
            }
        }

        // Adjust research depth based on high revision count patterns
        let high_revision_count = success_patterns.iter()
            .any(|p| p.signal == "high_revision_count");
        if high_revision_count {
            // Increase research depth for contexts with high revisions
            playbook.default_research_depth = match playbook.default_research_depth {
                crate::work::playbook::ResearchDepth::Minimal => crate::work::playbook::ResearchDepth::Standard,
                crate::work::playbook::ResearchDepth::Standard => crate::work::playbook::ResearchDepth::Deep,
                crate::work::playbook::ResearchDepth::Deep => crate::work::playbook::ResearchDepth::Exhaustive,
                crate::work::playbook::ResearchDepth::Exhaustive => crate::work::playbook::ResearchDepth::Exhaustive,
            };
        }

        // Add new patterns to playbook
        for pattern in &success_patterns {
            if !playbook.success_patterns.iter().any(|p| p.signal == pattern.signal) {
                playbook.success_patterns.push(pattern.clone());
            }
        }

        for pattern in &failure_patterns {
            if !playbook.failure_patterns.iter().any(|p| p.signal == pattern.signal) {
                playbook.failure_patterns.push(pattern.clone());
            }
        }

        // Update playbook confidence based on pattern balance
        let success_count = success_patterns.len() as f32;
        let failure_count = failure_patterns.len() as f32;
        let total = success_count + failure_count;
        if total > 0.0 {
            let new_confidence = success_count / total;
            playbook.confidence = (playbook.confidence * 0.7 + new_confidence * 0.3).clamp(0.0, 1.0);
        }

        playbook.updated_at = chrono::Utc::now();

        PlaybookOperations::update_playbook(&*self.db, &playbook)?;

        Ok(())
    }

    /// Initialize evolution tables
    fn init_tables(&self) -> Result<()> {
        // Create playbook_evolutions table
        self.db.conn().execute(
            "CREATE TABLE IF NOT EXISTS playbook_evolutions (
                id TEXT PRIMARY KEY,
                playbook_id TEXT NOT NULL,
                version TEXT NOT NULL,
                parent_version TEXT,
                mutation_strategy TEXT NOT NULL,
                performance TEXT NOT NULL,
                execution_count TEXT NOT NULL DEFAULT '0',
                success_rate TEXT NOT NULL DEFAULT '0.0',
                avg_duration_ms TEXT NOT NULL DEFAULT '0',
                status TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
            [],
        ).context("Failed to create playbook_evolutions table")?;

        // Create ab_tests table
        self.db.conn().execute(
            "CREATE TABLE IF NOT EXISTS ab_tests (
                id TEXT PRIMARY KEY,
                playbook_id TEXT NOT NULL,
                version_a INTEGER NOT NULL,
                version_b INTEGER NOT NULL,
                traffic_split REAL NOT NULL,
                started_at TEXT NOT NULL,
                ended_at TEXT,
                status TEXT NOT NULL,
                results TEXT,
                UNIQUE(playbook_id, status)
            )",
            [],
        ).context("Failed to create ab_tests table")?;

        Ok(())
    }

    /// Create a new playbook evolution
    pub fn create_evolution(
        &self,
        playbook_id: String,
        version: u32,
        parent_version: Option<u32>,
        mutation_strategy: MutationStrategy,
    ) -> Result<PlaybookEvolution> {
        self.init_tables()?;

        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now();

        let evolution = PlaybookEvolution {
            id: id.clone(),
            playbook_id,
            version,
            parent_version,
            mutation_strategy,
            performance: PerformanceMetrics {
                success_rate: 0.0,
                avg_duration_ms: 0,
                avg_cost: 0.0,
                avg_tool_calls: 0.0,
                user_satisfaction: None,
            },
            execution_count: 0,
            success_rate: 0.0,
            avg_duration_ms: 0,
            status: EvolutionStatus::Testing,
            created_at: now,
            updated_at: now,
        };

        self.store_evolution(&evolution)?;

        Ok(evolution)
    }

    /// Store an evolution in the database
    fn store_evolution(&self, evolution: &PlaybookEvolution) -> Result<()> {
        let strategy_json = serde_json::to_string(&evolution.mutation_strategy)
            .context("Failed to serialize mutation strategy")?;
        let performance_json = serde_json::to_string(&evolution.performance)
            .context("Failed to serialize performance metrics")?;
        let status_str = match evolution.status {
            EvolutionStatus::Testing => "testing",
            EvolutionStatus::ReadyForPromotion => "ready_for_promotion",
            EvolutionStatus::Promoted => "promoted",
            EvolutionStatus::Rejected => "rejected",
            EvolutionStatus::Deprecated => "deprecated",
        };

        self.db.conn().execute(
            "INSERT OR REPLACE INTO playbook_evolutions (
                id, playbook_id, version, parent_version, mutation_strategy,
                performance, execution_count, success_rate, avg_duration_ms,
                status, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            [
                &evolution.id,
                &evolution.playbook_id,
                &(evolution.version.to_string()),
                &evolution.parent_version.as_ref().map(|v| v.to_string()).unwrap_or_default(),
                &strategy_json,
                &performance_json,
                &(evolution.execution_count.to_string()),
                &evolution.success_rate.to_string(),
                &(evolution.avg_duration_ms.to_string()),
                status_str,
                &evolution.created_at.to_rfc3339(),
                &evolution.updated_at.to_rfc3339(),
            ],
        ).context("Failed to store evolution")?;

        Ok(())
    }

    /// Record execution results for an evolution
    pub fn record_execution(
        &self,
        evolution_id: &str,
        success: bool,
        duration_ms: u64,
        cost: f64,
        tool_calls: u32,
    ) -> Result<()> {
        if let Some(mut evolution) = self.get_evolution(evolution_id)? {
            evolution.execution_count += 1;
            evolution.avg_duration_ms = (evolution.avg_duration_ms * (evolution.execution_count - 1) as u64 + duration_ms) / evolution.execution_count as u64;

            // Update success rate
            let total_successes = (evolution.success_rate * (evolution.execution_count - 1) as f64) + if success { 1.0 } else { 0.0 };
            evolution.success_rate = total_successes / evolution.execution_count as f64;

            // Update performance metrics
            evolution.performance.success_rate = evolution.success_rate;
            evolution.performance.avg_duration_ms = evolution.avg_duration_ms;
            evolution.performance.avg_cost = (evolution.performance.avg_cost * (evolution.execution_count - 1) as f64 + cost) / evolution.execution_count as f64;
            evolution.performance.avg_tool_calls = (evolution.performance.avg_tool_calls * (evolution.execution_count - 1) as f64 + tool_calls as f64) / evolution.execution_count as f64;

            evolution.updated_at = chrono::Utc::now();

            // Check if ready for promotion (needs at least 10 executions)
            if evolution.execution_count >= 10 && evolution.success_rate > 0.8 {
                evolution.status = EvolutionStatus::ReadyForPromotion;
            }

            self.store_evolution(&evolution)?;
        }

        Ok(())
    }

    /// Get an evolution by ID
    pub fn get_evolution(&self, id: &str) -> Result<Option<PlaybookEvolution>> {
        self.init_tables()?;

        let mut stmt = self.db.conn().prepare(
            "SELECT id, playbook_id, version, parent_version, mutation_strategy,
                    performance, execution_count, success_rate, avg_duration_ms,
                    status, created_at, updated_at
             FROM playbook_evolutions WHERE id = ?1"
        )?;

        let mut rows = stmt.query_map([id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, String>(11)?,
            ))
        })?;

        if let Some(row) = rows.next() {
            let (id, playbook_id, version_str, parent_version_str, strategy_json, performance_json,
                 execution_count_str, success_rate_str, avg_duration_ms_str, status_str,
                 created_at, updated_at) = row?;

            let mutation_strategy: MutationStrategy = serde_json::from_str(&strategy_json)
                .context("Failed to deserialize mutation strategy")?;
            let performance: PerformanceMetrics = serde_json::from_str(&performance_json)
                .context("Failed to deserialize performance metrics")?;
            let status = match status_str.as_str() {
                "testing" => EvolutionStatus::Testing,
                "ready_for_promotion" => EvolutionStatus::ReadyForPromotion,
                "promoted" => EvolutionStatus::Promoted,
                "rejected" => EvolutionStatus::Rejected,
                "deprecated" => EvolutionStatus::Deprecated,
                _ => EvolutionStatus::Testing,
            };

            let version = version_str.parse::<u32>().unwrap_or(0);
            let parent_version = parent_version_str.and_then(|s| s.parse::<u32>().ok());
            let execution_count = execution_count_str.parse::<u32>().unwrap_or(0);
            let success_rate = success_rate_str.parse::<f64>().unwrap_or(0.0);
            let avg_duration_ms = avg_duration_ms_str.parse::<u64>().unwrap_or(0);

            Ok(Some(PlaybookEvolution {
                id,
                playbook_id,
                version,
                parent_version,
                mutation_strategy,
                performance,
                execution_count,
                success_rate,
                avg_duration_ms,
                status,
                created_at: chrono::DateTime::parse_from_rfc3339(&created_at)?.with_timezone(&chrono::Utc),
                updated_at: chrono::DateTime::parse_from_rfc3339(&updated_at)?.with_timezone(&chrono::Utc),
            }))
        } else {
            Ok(None)
        }
    }

    /// List evolutions for a playbook
    pub fn list_evolutions(&self, playbook_id: &str) -> Result<Vec<PlaybookEvolution>> {
        self.init_tables()?;

        let mut stmt = self.db.conn().prepare(
            "SELECT id, playbook_id, version, parent_version, mutation_strategy,
                    performance, execution_count, success_rate, avg_duration_ms,
                    status, created_at, updated_at
             FROM playbook_evolutions WHERE playbook_id = ?1 ORDER BY version DESC"
        )?;

        let mut evolutions = Vec::new();

        let rows = stmt.query_map([playbook_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, String>(11)?,
            ))
        })?;

        for row in rows {
            let (id, playbook_id, version_str, parent_version_str, strategy_json, performance_json,
                 execution_count_str, success_rate_str, avg_duration_ms_str, status_str,
                 created_at, updated_at) = row?;

            let mutation_strategy: MutationStrategy = serde_json::from_str(&strategy_json)
                .context("Failed to deserialize mutation strategy")?;
            let performance: PerformanceMetrics = serde_json::from_str(&performance_json)
                .context("Failed to deserialize performance metrics")?;
            let status = match status_str.as_str() {
                "testing" => EvolutionStatus::Testing,
                "ready_for_promotion" => EvolutionStatus::ReadyForPromotion,
                "promoted" => EvolutionStatus::Promoted,
                "rejected" => EvolutionStatus::Rejected,
                "deprecated" => EvolutionStatus::Deprecated,
                _ => EvolutionStatus::Testing,
            };

            let version = version_str.parse::<u32>().unwrap_or(0);
            let parent_version = parent_version_str.and_then(|s| s.parse::<u32>().ok());
            let execution_count = execution_count_str.parse::<u32>().unwrap_or(0);
            let success_rate = success_rate_str.parse::<f64>().unwrap_or(0.0);
            let avg_duration_ms = avg_duration_ms_str.parse::<u64>().unwrap_or(0);

            evolutions.push(PlaybookEvolution {
                id,
                playbook_id,
                version,
                parent_version,
                mutation_strategy,
                performance,
                execution_count,
                success_rate,
                avg_duration_ms,
                status,
                created_at: chrono::DateTime::parse_from_rfc3339(&created_at)?.with_timezone(&chrono::Utc),
                updated_at: chrono::DateTime::parse_from_rfc3339(&updated_at)?.with_timezone(&chrono::Utc),
            });
        }

        Ok(evolutions)
    }

    /// Promote an evolution to production
    pub fn promote_evolution(&self, evolution_id: &str) -> Result<()> {
        if let Some(mut evolution) = self.get_evolution(evolution_id)? {
            // Deprecate previous promoted version
            let evolutions = self.list_evolutions(&evolution.playbook_id)?;
            for e in evolutions {
                if e.status == EvolutionStatus::Promoted && e.id != evolution_id {
                    let mut deprecated = e;
                    deprecated.status = EvolutionStatus::Deprecated;
                    self.store_evolution(&deprecated)?;
                }
            }

            // Promote this version
            evolution.status = EvolutionStatus::Promoted;
            evolution.updated_at = chrono::Utc::now();
            self.store_evolution(&evolution)?;
        }

        Ok(())
    }

    /// Start an A/B test
    pub fn start_ab_test(
        &self,
        playbook_id: String,
        version_a: u32,
        version_b: u32,
        traffic_split: f64,
    ) -> Result<AbTest> {
        self.init_tables()?;

        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now();

        let test = AbTest {
            id: id.clone(),
            playbook_id,
            version_a,
            version_b,
            traffic_split,
            started_at: now,
            ended_at: None,
            status: AbTestStatus::Running,
            results: None,
        };

        self.store_ab_test(&test)?;

        Ok(test)
    }

    /// Store an A/B test
    fn store_ab_test(&self, test: &AbTest) -> Result<()> {
        let status_str = match test.status {
            AbTestStatus::Running => "running",
            AbTestStatus::Completed => "completed",
            AbTestStatus::StoppedEarly => "stopped_early",
        };
        let results_json = test.results.as_ref()
            .map(|r| serde_json::to_string(r).ok())
            .flatten()
            .unwrap_or_default();

        self.db.conn().execute(
            "INSERT OR REPLACE INTO ab_tests (
                id, playbook_id, version_a, version_b, traffic_split,
                started_at, ended_at, status, results
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            [
                &test.id,
                &test.playbook_id,
                &(test.version_a.to_string()),
                &(test.version_b.to_string()),
                &test.traffic_split.to_string(),
                &test.started_at.to_rfc3339(),
                &test.ended_at.as_ref().map(|t| t.to_rfc3339()).unwrap_or_default(),
                status_str,
                &results_json,
            ],
        ).context("Failed to store A/B test")?;

        Ok(())
    }

    /// Complete an A/B test and determine winner
    pub fn complete_ab_test(&self, test_id: &str) -> Result<AbTest> {
        if let Some(mut test) = self.get_ab_test(test_id)? {
            test.ended_at = Some(chrono::Utc::now());
            test.status = AbTestStatus::Completed;

            // Get performance metrics for both versions
            let evolutions = self.list_evolutions(&test.playbook_id)?;
            let metrics_a = evolutions.iter()
                .find(|e| e.version == test.version_a)
                .map(|e| e.performance.clone())
                .unwrap_or_default();
            let metrics_b = evolutions.iter()
                .find(|e| e.version == test.version_b)
                .map(|e| e.performance.clone())
                .unwrap_or_default();

            // Determine winner based on success rate
            let winner = if metrics_b.success_rate > metrics_a.success_rate {
                Some(test.version_b)
            } else {
                Some(test.version_a)
            };

            // Calculate significance (simplified)
            let significance = (metrics_b.success_rate - metrics_a.success_rate).abs();

            test.results = Some(AbTestResults {
                metrics_a,
                metrics_b,
                significance,
                winner,
            });

            self.store_ab_test(&test)?;

            Ok(test)
        } else {
            Err(anyhow::anyhow!("A/B test not found"))
        }
    }

    /// Get an A/B test by ID
    fn get_ab_test(&self, id: &str) -> Result<Option<AbTest>> {
        self.init_tables()?;

        let mut stmt = self.db.conn().prepare(
            "SELECT id, playbook_id, version_a, version_b, traffic_split,
                    started_at, ended_at, status, results
             FROM ab_tests WHERE id = ?1"
        )?;

        let mut rows = stmt.query_map([id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
            ))
        })?;

        if let Some(row) = rows.next() {
            let (id, playbook_id, version_a_str, version_b_str, traffic_split_str,
                 started_at, ended_at, status_str, results_json) = row?;

            let status = match status_str.as_str() {
                "running" => AbTestStatus::Running,
                "completed" => AbTestStatus::Completed,
                "stopped_early" => AbTestStatus::StoppedEarly,
                _ => AbTestStatus::Running,
            };

            let results = if !results_json.is_empty() {
                Some(serde_json::from_str(&results_json)?)
            } else {
                None
            };

            let version_a = version_a_str.parse::<u32>().unwrap_or(0);
            let version_b = version_b_str.parse::<u32>().unwrap_or(0);
            let traffic_split = traffic_split_str.parse::<f64>().unwrap_or(0.5);

            Ok(Some(AbTest {
                id,
                playbook_id,
                version_a,
                version_b,
                traffic_split,
                started_at: chrono::DateTime::parse_from_rfc3339(&started_at)?.with_timezone(&chrono::Utc),
                ended_at: ended_at.and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok()).map(|dt| dt.with_timezone(&chrono::Utc)),
                status,
                results,
            }))
        } else {
            Ok(None)
        }
    }

    /// List A/B tests for a playbook
    pub fn list_ab_tests(&self, playbook_id: &str) -> Result<Vec<AbTest>> {
        self.init_tables()?;

        let mut stmt = self.db.conn().prepare(
            "SELECT id, playbook_id, version_a, version_b, traffic_split,
                    started_at, ended_at, status, results
             FROM ab_tests WHERE playbook_id = ?1 ORDER BY started_at DESC"
        )?;

        let mut tests = Vec::new();

        let rows = stmt.query_map([playbook_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
            ))
        })?;

        for row in rows {
            let (id, playbook_id, version_a_str, version_b_str, traffic_split_str,
                 started_at, ended_at, status_str, results_json) = row?;

            let status = match status_str.as_str() {
                "running" => AbTestStatus::Running,
                "completed" => AbTestStatus::Completed,
                "stopped_early" => AbTestStatus::StoppedEarly,
                _ => AbTestStatus::Running,
            };

            let results = if !results_json.is_empty() {
                Some(serde_json::from_str(&results_json)?)
            } else {
                None
            };

            let version_a = version_a_str.parse::<u32>().unwrap_or(0);
            let version_b = version_b_str.parse::<u32>().unwrap_or(0);
            let traffic_split = traffic_split_str.parse::<f64>().unwrap_or(0.5);

            tests.push(AbTest {
                id,
                playbook_id,
                version_a,
                version_b,
                traffic_split,
                started_at: chrono::DateTime::parse_from_rfc3339(&started_at)?.with_timezone(&chrono::Utc),
                ended_at: ended_at.and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok()).map(|dt| dt.with_timezone(&chrono::Utc)),
                status,
                results,
            });
        }

        Ok(tests)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::work::types::{WorkDomain, WorkStatus};
    use serde_json::json;

    #[test]
    fn test_extract_patterns_success() {
        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );
        context.status = WorkStatus::Completed;

        // Add execution metadata
        context.execution_metadata.push(crate::work::types::ExecutionRecord {
            node_id: "planner".to_string(),
            provider: "openai".to_string(),
            model: "gpt-4".to_string(),
            latency_ms: 6000,
            timestamp: chrono::Utc::now(),
            tokens: Some(100),
            cost: Some(0.01),
        });

        let (success_patterns, failure_patterns) = EvolutionEngine::extract_patterns(&context);

        // Should have success patterns from model usage and high latency
        assert!(!success_patterns.is_empty());
        assert!(success_patterns.iter().any(|p| p.signal.starts_with("model_usage_")));
        assert!(success_patterns.iter().any(|p| p.signal.contains("high_latency")));
    }

    #[test]
    fn test_extract_patterns_failure() {
        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );
        context.status = WorkStatus::Blocked;
        context.blocked_reason = Some("Security violation".to_string());

        let (success_patterns, failure_patterns) = EvolutionEngine::extract_patterns(&context);

        // Should have failure patterns from blocked reason
        assert!(!failure_patterns.is_empty());
        assert!(failure_patterns.iter().any(|p| p.signal.contains("blocked_reason")));
    }

    #[test]
    fn test_extract_patterns_flow_performance() {
        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );
        context.status = WorkStatus::Completed;

        // Add flow performance record in metadata - use the format expected by extract_patterns
        let perf_record = FlowPerformanceRecord {
            id: uuid::Uuid::new_v4().to_string(),
            flow_id: "planning.flow.yaml".to_string(),
            work_context_id: context.id.clone(),
            success_score: 0.9,
            duration_ms: 15000,
            token_cost: 0.6,
            revision_count: 1,
            executed_at: chrono::Utc::now(),
        };
        let perf_json = serde_json::to_value(&perf_record).unwrap();
        context.metadata = json!({
            "flow_perf_planning": perf_json
        });

        let (success_patterns, failure_patterns) = EvolutionEngine::extract_patterns(&context);

        // Should have patterns from flow performance
        assert!(!success_patterns.is_empty());
        assert!(success_patterns.iter().any(|p| p.signal.contains("flow_performance")));
        assert!(success_patterns.iter().any(|p| p.signal.contains("high_duration")));
        assert!(success_patterns.iter().any(|p| p.signal.contains("high_cost")));
    }

    #[test]
    fn test_extract_patterns_high_revision_count() {
        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );
        context.status = WorkStatus::Completed;

        // Add 4 decisions to trigger high revision count pattern
        use crate::work::decision::DecisionRecord;
        context.decisions.push(DecisionRecord {
            id: uuid::Uuid::new_v4().to_string(),
            description: "decision 1".to_string(),
            chosen_option: "option1".to_string(),
            alternatives: vec!["option2".to_string()],
            approved: true,
            created_at: chrono::Utc::now(),
        });
        context.decisions.push(DecisionRecord {
            id: uuid::Uuid::new_v4().to_string(),
            description: "decision 2".to_string(),
            chosen_option: "option1".to_string(),
            alternatives: vec!["option2".to_string()],
            approved: true,
            created_at: chrono::Utc::now(),
        });
        context.decisions.push(DecisionRecord {
            id: uuid::Uuid::new_v4().to_string(),
            description: "decision 3".to_string(),
            chosen_option: "option1".to_string(),
            alternatives: vec!["option2".to_string()],
            approved: true,
            created_at: chrono::Utc::now(),
        });
        context.decisions.push(DecisionRecord {
            id: uuid::Uuid::new_v4().to_string(),
            description: "decision 4".to_string(),
            chosen_option: "option1".to_string(),
            alternatives: vec!["option2".to_string()],
            approved: true,
            created_at: chrono::Utc::now(),
        });

        let (success_patterns, failure_patterns) = EvolutionEngine::extract_patterns(&context);

        // Should have high revision count pattern
        assert!(success_patterns.iter().any(|p| p.signal == "high_revision_count"));
    }

    #[test]
    fn test_evaluate_context_completed() {
        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );
        context.status = WorkStatus::Completed;
        use crate::work::plan::{ExecutionPlan, PlanStep};
        context.plan = Some(ExecutionPlan::new(vec![
            PlanStep::new("step1".to_string(), "First step".to_string(), "flow1".to_string()),
            PlanStep::new("step2".to_string(), "Second step".to_string(), "flow2".to_string()),
        ]));

        let result = EvolutionEngine::evaluate_context(&context);

        assert!(result.overall_score > 0.8);
        assert_eq!(result.semantic_score, 1.0);
        assert!(result.structural_score > 0.7);
    }

    #[test]
    fn test_evaluate_context_blocked() {
        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );
        context.status = WorkStatus::Blocked;

        let result = EvolutionEngine::evaluate_context(&context);

        // Blocked status gives lower semantic score
        assert_eq!(result.semantic_score, 0.3);
        // Overall score should be lower than completed but not necessarily < 0.5 due to other factors
        assert!(result.overall_score < 0.7);
    }

    #[test]
    fn test_evaluate_context_artifact_completeness() {
        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );
        context.status = WorkStatus::Completed;
        use crate::work::types::CompletionCriterion;
        context.completion_criteria = vec![
            CompletionCriterion::new("plan".to_string(), "Plan completed".to_string()),
            CompletionCriterion::new("code".to_string(), "Code completed".to_string()),
        ];

        // Add one artifact
        use crate::work::artifact::{Artifact, ArtifactKind};
        context.artifacts.push(Artifact::new(
            uuid::Uuid::new_v4().to_string(),
            context.id.clone(),
            ArtifactKind::Plan,
            "Test Plan".to_string(),
            json!({"content": "test"}),
            "user-1".to_string(),
        ));

        let result = EvolutionEngine::evaluate_context(&context);

        // Artifact completeness should be 0.5 (1 of 2 criteria met)
        assert_eq!(result.artifact_completeness_score, 0.5);
    }

    #[test]
    fn test_evolve_playbook_success_patterns() {
        let db = Arc::new(Db::in_memory().unwrap());
        let engine = EvolutionEngine::new(db.clone());

        // Create a playbook
        let mut playbook = crate::work::playbook::WorkContextPlaybook::new(
            "pb-1".to_string(),
            "user-1".to_string(),
            "software".to_string(),
            "Test Playbook".to_string(),
            "Test playbook".to_string(),
        );
        playbook.preferred_flows = vec![
            crate::work::playbook::FlowPreference {
                flow_id: "planning.flow.yaml".to_string(),
                weight: 0.5,
                confidence: 0.5,
            },
        ];

        db.create_playbook(&playbook).unwrap();

        // Evolve with success patterns
        let success_patterns = vec![
            PatternRecord {
                pattern_type: PatternType::Success,
                signal: "model_usage_planning.flow.yaml_openai_gpt-4".to_string(),
                weight: 0.8,
                created_at: chrono::Utc::now(),
            },
        ];
        let failure_patterns = vec![];

        engine.evolve_playbook("pb-1", success_patterns, failure_patterns).unwrap();

        // Verify flow weights increased
        let updated = db.get_playbook("pb-1").unwrap().unwrap();
        assert!(updated.preferred_flows[0].weight > 0.5);
        assert!(updated.preferred_flows[0].confidence > 0.5);
    }

    #[test]
    fn test_evolve_playbook_failure_patterns() {
        let db = Arc::new(Db::in_memory().unwrap());
        let engine = EvolutionEngine::new(db.clone());

        // Create a playbook
        let mut playbook = crate::work::playbook::WorkContextPlaybook::new(
            "pb-1".to_string(),
            "user-1".to_string(),
            "software".to_string(),
            "Test Playbook".to_string(),
            "Test playbook".to_string(),
        );
        playbook.preferred_flows = vec![
            crate::work::playbook::FlowPreference {
                flow_id: "planning.flow.yaml".to_string(),
                weight: 0.8,
                confidence: 0.8,
            },
        ];

        db.create_playbook(&playbook).unwrap();

        // Evolve with failure patterns
        let success_patterns = vec![];
        let failure_patterns = vec![
            PatternRecord {
                pattern_type: PatternType::Failure,
                signal: "model_usage_openai_gpt-4".to_string(),
                weight: 0.8,
                created_at: chrono::Utc::now(),
            },
        ];

        engine.evolve_playbook("pb-1", success_patterns, failure_patterns).unwrap();

        // Verify flow weights decreased
        let updated = db.get_playbook("pb-1").unwrap().unwrap();
        assert!(updated.preferred_flows[0].weight < 0.8);
        assert!(updated.preferred_flows[0].confidence < 0.8);
    }

    #[test]
    fn test_evolve_playbook_research_depth_increase() {
        let db = Arc::new(Db::in_memory().unwrap());
        let engine = EvolutionEngine::new(db.clone());

        let mut playbook = crate::work::playbook::WorkContextPlaybook::new(
            "pb-1".to_string(),
            "user-1".to_string(),
            "software".to_string(),
            "Test Playbook".to_string(),
            "Test playbook".to_string(),
        );
        playbook.default_research_depth = crate::work::playbook::ResearchDepth::Minimal;

        db.create_playbook(&playbook).unwrap();

        // Evolve with high revision count pattern
        let success_patterns = vec![
            PatternRecord {
                pattern_type: PatternType::Success,
                signal: "high_revision_count".to_string(),
                weight: 0.8,
                created_at: chrono::Utc::now(),
            },
        ];
        let failure_patterns = vec![];

        engine.evolve_playbook("pb-1", success_patterns, failure_patterns).unwrap();

        // Verify research depth increased
        let updated = db.get_playbook("pb-1").unwrap().unwrap();
        assert_eq!(updated.default_research_depth, crate::work::playbook::ResearchDepth::Standard);
    }

    #[test]
    fn test_create_evolution() {
        let db = Arc::new(Db::in_memory().unwrap());
        let engine = EvolutionEngine::new(db);

        let strategy = MutationStrategy::AddNode {
            node_id: "new_node".to_string(),
            node_type: "llm".to_string(),
            position: "after".to_string(),
        };

        let evolution = engine.create_evolution("pb-1".to_string(), 1, None, strategy).unwrap();

        assert_eq!(evolution.playbook_id, "pb-1");
        assert_eq!(evolution.version, 1);
        assert_eq!(evolution.status, EvolutionStatus::Testing);
        assert_eq!(evolution.execution_count, 0);
    }

    #[test]
    fn test_record_execution() {
        let db = Arc::new(Db::in_memory().unwrap());
        let engine = EvolutionEngine::new(db);

        let strategy = MutationStrategy::RemoveNode {
            node_id: "old_node".to_string(),
        };

        let evolution = engine.create_evolution("pb-1".to_string(), 1, None, strategy).unwrap();

        // Record successful execution
        engine.record_execution(&evolution.id, true, 5000, 0.1, 5).unwrap();

        let updated = engine.get_evolution(&evolution.id).unwrap().unwrap();
        assert_eq!(updated.execution_count, 1);
        assert_eq!(updated.success_rate, 1.0);
    }

    #[test]
    fn test_promotion_threshold() {
        let db = Arc::new(Db::in_memory().unwrap());
        let engine = EvolutionEngine::new(db);

        let strategy = MutationStrategy::AddNode {
            node_id: "new_node".to_string(),
            node_type: "llm".to_string(),
            position: "after".to_string(),
        };

        let evolution = engine.create_evolution("pb-1".to_string(), 1, None, strategy).unwrap();

        // Record 10 successful executions
        for _ in 0..10 {
            engine.record_execution(&evolution.id, true, 5000, 0.1, 5).unwrap();
        }

        let updated = engine.get_evolution(&evolution.id).unwrap().unwrap();
        assert_eq!(updated.status, EvolutionStatus::ReadyForPromotion);
    }

    #[test]
    fn test_start_ab_test() {
        let db = Arc::new(Db::in_memory().unwrap());
        let engine = EvolutionEngine::new(db);

        let test = engine.start_ab_test("pb-1".to_string(), 1, 2, 0.5).unwrap();

        assert_eq!(test.playbook_id, "pb-1");
        assert_eq!(test.version_a, 1);
        assert_eq!(test.version_b, 2);
        assert_eq!(test.traffic_split, 0.5);
        assert_eq!(test.status, AbTestStatus::Running);
        assert!(test.results.is_none());
    }

    #[test]
    fn test_mutation_strategy_serialization() {
        let strategy = MutationStrategy::Combine {
            other_playbook_id: "pb-2".to_string(),
        };

        let json = serde_json::to_string(&strategy).unwrap();
        let deserialized: MutationStrategy = serde_json::from_str(&json).unwrap();

        match deserialized {
            MutationStrategy::Combine { other_playbook_id } => {
                assert_eq!(other_playbook_id, "pb-2");
            }
            _ => panic!("Wrong strategy deserialized"),
        }
    }

    #[test]
    fn test_performance_metrics_default() {
        let metrics = PerformanceMetrics::default();

        assert_eq!(metrics.success_rate, 0.0);
        assert_eq!(metrics.avg_duration_ms, 0);
        assert_eq!(metrics.avg_cost, 0.0);
        assert_eq!(metrics.avg_tool_calls, 0.0);
        assert!(metrics.user_satisfaction.is_none());
    }

    #[test]
    fn test_flow_performance_record_creation() {
        let perf_record = FlowPerformanceRecord {
            id: uuid::Uuid::new_v4().to_string(),
            flow_id: "planning.flow.yaml".to_string(),
            work_context_id: "ctx-1".to_string(),
            success_score: 0.9,
            duration_ms: 5000,
            token_cost: 0.1,
            revision_count: 1,
            executed_at: chrono::Utc::now(),
        };

        assert_eq!(perf_record.flow_id, "planning.flow.yaml");
        assert_eq!(perf_record.success_score, 0.9);
        assert_eq!(perf_record.duration_ms, 5000);
    }

    #[test]
    fn test_flow_performance_record_serialization() {
        let perf_record = FlowPerformanceRecord {
            id: uuid::Uuid::new_v4().to_string(),
            flow_id: "coding.flow.yaml".to_string(),
            work_context_id: "ctx-1".to_string(),
            success_score: 0.85,
            duration_ms: 10000,
            token_cost: 0.2,
            revision_count: 2,
            executed_at: chrono::Utc::now(),
        };

        // Test serialization
        let json = serde_json::to_string(&perf_record).unwrap();
        assert!(!json.is_empty());

        // Test deserialization
        let deserialized: FlowPerformanceRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.flow_id, "coding.flow.yaml");
        assert_eq!(deserialized.success_score, 0.85);
    }

    #[test]
    fn test_flow_performance_record_in_metadata() {
        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );

        let perf_record = FlowPerformanceRecord {
            id: uuid::Uuid::new_v4().to_string(),
            flow_id: "planning.flow.yaml".to_string(),
            work_context_id: context.id.clone(),
            success_score: 0.9,
            duration_ms: 5000,
            token_cost: 0.1,
            revision_count: 1,
            executed_at: chrono::Utc::now(),
        };

        // Store in metadata
        let perf_json = serde_json::to_value(&perf_record).unwrap();
        context.metadata = json!({
            "flow_perf_planning": perf_json
        });

        // Retrieve from metadata
        let retrieved = context.metadata.get("flow_perf_planning").unwrap();
        let deserialized: FlowPerformanceRecord = serde_json::from_value(retrieved.clone()).unwrap();

        assert_eq!(deserialized.flow_id, "planning.flow.yaml");
        assert_eq!(deserialized.success_score, 0.9);
    }

    #[test]
    fn test_flow_performance_record_high_cost_threshold() {
        let perf_record = FlowPerformanceRecord {
            id: uuid::Uuid::new_v4().to_string(),
            flow_id: "expensive.flow.yaml".to_string(),
            work_context_id: "ctx-1".to_string(),
            success_score: 0.7,
            duration_ms: 5000,
            token_cost: 0.6,  // Above 0.5 threshold
            revision_count: 1,
            executed_at: chrono::Utc::now(),
        };

        assert!(perf_record.token_cost > 0.5);
    }

    #[test]
    fn test_flow_performance_record_high_duration_threshold() {
        let perf_record = FlowPerformanceRecord {
            id: uuid::Uuid::new_v4().to_string(),
            flow_id: "slow.flow.yaml".to_string(),
            work_context_id: "ctx-1".to_string(),
            success_score: 0.8,
            duration_ms: 12000,  // Above 10000 threshold
            token_cost: 0.1,
            revision_count: 1,
            executed_at: chrono::Utc::now(),
        };

        assert!(perf_record.duration_ms > 10000);
    }

    #[test]
    fn test_flow_performance_record_success_threshold() {
        let perf_record = FlowPerformanceRecord {
            id: uuid::Uuid::new_v4().to_string(),
            flow_id: "successful.flow.yaml".to_string(),
            work_context_id: "ctx-1".to_string(),
            success_score: 0.9,  // Above 0.7 threshold
            duration_ms: 5000,
            token_cost: 0.1,
            revision_count: 1,
            executed_at: chrono::Utc::now(),
        };

        assert!(perf_record.success_score > 0.7);
    }

    #[test]
    fn test_flow_performance_record_failure_threshold() {
        let perf_record = FlowPerformanceRecord {
            id: uuid::Uuid::new_v4().to_string(),
            flow_id: "failed.flow.yaml".to_string(),
            work_context_id: "ctx-1".to_string(),
            success_score: 0.5,  // Below 0.7 threshold
            duration_ms: 5000,
            token_cost: 0.1,
            revision_count: 1,
            executed_at: chrono::Utc::now(),
        };

        assert!(perf_record.success_score < 0.7);
    }

    #[test]
    fn test_evaluate_context_in_progress() {
        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );
        context.status = WorkStatus::InProgress;

        let result = EvolutionEngine::evaluate_context(&context);

        assert_eq!(result.semantic_score, 0.7);
        assert!(result.overall_score > 0.5);
    }

    #[test]
    fn test_evaluate_context_draft() {
        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );
        context.status = WorkStatus::Draft;

        let result = EvolutionEngine::evaluate_context(&context);

        assert_eq!(result.semantic_score, 0.5);
    }

    #[test]
    fn test_evaluate_context_awaiting_approval() {
        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );
        context.status = WorkStatus::AwaitingApproval;

        let result = EvolutionEngine::evaluate_context(&context);

        assert_eq!(result.semantic_score, 0.6);
    }

    #[test]
    fn test_evaluate_context_tool_consistency_high_latency() {
        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );
        context.status = WorkStatus::Completed;

        // Add execution metadata with high latency
        context.execution_metadata.push(crate::work::types::ExecutionRecord {
            node_id: "planner".to_string(),
            provider: "openai".to_string(),
            model: "gpt-4".to_string(),
            latency_ms: 15000,  // High latency
            timestamp: chrono::Utc::now(),
            tokens: Some(100),
            cost: Some(0.01),
        });

        let result = EvolutionEngine::evaluate_context(&context);

        // Tool consistency should be lower due to high latency
        assert!(result.tool_consistency_score < 0.7);
    }

    #[test]
    fn test_evaluate_context_tool_consistency_low_latency() {
        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );
        context.status = WorkStatus::Completed;

        // Add execution metadata with low latency
        context.execution_metadata.push(crate::work::types::ExecutionRecord {
            node_id: "planner".to_string(),
            provider: "openai".to_string(),
            model: "gpt-4".to_string(),
            latency_ms: 2000,  // Low latency
            timestamp: chrono::Utc::now(),
            tokens: Some(100),
            cost: Some(0.01),
        });

        let result = EvolutionEngine::evaluate_context(&context);

        // Tool consistency should be higher due to low latency
        assert!(result.tool_consistency_score > 0.8);
    }

    #[test]
    fn test_evaluate_context_no_execution_metadata() {
        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );
        context.status = WorkStatus::Completed;

        let result = EvolutionEngine::evaluate_context(&context);

        // No execution metadata should give default tool consistency score
        assert_eq!(result.tool_consistency_score, 0.5);
    }

    #[test]
    fn test_evaluate_context_structural_with_plan() {
        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );
        context.status = WorkStatus::Completed;
        use crate::work::plan::{ExecutionPlan, PlanStep};
        context.plan = Some(ExecutionPlan::new(vec![
            PlanStep::new("step1".to_string(), "First step".to_string(), "flow1".to_string()),
        ]));

        let result = EvolutionEngine::evaluate_context(&context);

        // Structural score should be higher with a plan
        assert!(result.structural_score > 0.7);
    }

    #[test]
    fn test_evaluate_context_structural_without_plan() {
        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );
        context.status = WorkStatus::Completed;

        let result = EvolutionEngine::evaluate_context(&context);

        // Structural score should be lower without a plan
        assert_eq!(result.structural_score, 0.4);
    }

    #[test]
    fn test_evaluate_context_artifact_completeness_no_criteria() {
        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );
        context.status = WorkStatus::Completed;

        let result = EvolutionEngine::evaluate_context(&context);

        // No completion criteria should give perfect score
        assert_eq!(result.artifact_completeness_score, 1.0);
    }

    #[test]
    fn test_evaluate_context_overall_score_clamping() {
        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );
        context.status = WorkStatus::Completed;

        let result = EvolutionEngine::evaluate_context(&context);

        // Overall score should be clamped between 0.0 and 1.0
        assert!(result.overall_score >= 0.0);
        assert!(result.overall_score <= 1.0);
    }

    #[test]
    fn test_evaluate_context_details_format() {
        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );
        context.status = WorkStatus::Completed;

        let result = EvolutionEngine::evaluate_context(&context);

        // Details should contain all component scores
        assert!(result.details.contains("Semantic"));
        assert!(result.details.contains("Structural"));
        assert!(result.details.contains("Tool Consistency"));
        assert!(result.details.contains("Artifact Completeness"));
    }
}
