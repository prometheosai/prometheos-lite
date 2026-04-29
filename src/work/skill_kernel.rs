//! SkillKernel - skill extraction and management system
//!
//! This module implements the SkillKernel which extracts reusable skills
//! from successful flow executions and matches them to new tasks.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

use crate::db::Db;

/// Skill - a reusable capability extracted from successful flow executions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    /// Unique identifier for the skill
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Description of what the skill does
    pub description: String,
    /// Capability signature (input/output patterns)
    pub capability_signature: CapabilitySignature,
    /// Example executions demonstrating the skill
    pub examples: Vec<SkillExample>,
    /// Success rate of this skill
    pub success_rate: f64,
    /// Average execution duration in milliseconds
    pub avg_duration_ms: u64,
    /// Number of times this skill has been used
    pub usage_count: u32,
    /// Metadata tags
    pub tags: Vec<String>,
    /// Created at timestamp
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Updated at timestamp
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Capability signature - describes input/output patterns for a skill
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilitySignature {
    /// Input schema (JSON Schema)
    pub input_schema: serde_json::Value,
    /// Output schema (JSON Schema)
    pub output_schema: serde_json::Value,
    /// Required tools for this skill
    pub required_tools: Vec<String>,
    /// Required permissions
    pub required_permissions: Vec<String>,
}

/// Example execution demonstrating a skill
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillExample {
    /// Unique identifier
    pub id: String,
    /// Flow run ID this example came from
    pub flow_run_id: String,
    /// Input for this example
    pub input: serde_json::Value,
    /// Output for this example
    pub output: serde_json::Value,
    /// Execution duration in milliseconds
    pub duration_ms: u64,
}

/// Skill extraction request
#[derive(Debug, Clone)]
pub struct SkillExtractionRequest {
    /// Flow run ID to extract skills from
    pub flow_run_id: String,
    /// Flow name
    pub flow_name: String,
    /// Execution metadata
    pub execution_metadata: HashMap<String, serde_json::Value>,
    /// Input for the execution
    pub input: serde_json::Value,
    /// Final output
    pub final_output: serde_json::Value,
    /// Execution duration in milliseconds
    pub duration_ms: u64,
}

/// Skill matching request
#[derive(Debug, Clone)]
pub struct SkillMatchingRequest {
    /// Task description
    pub task_description: String,
    /// Input data
    pub input: serde_json::Value,
    /// Required tools
    pub required_tools: Vec<String>,
    /// Maximum number of skills to return
    pub max_results: usize,
}

/// Skill matching result
#[derive(Debug, Clone)]
pub struct SkillMatch {
    /// The matched skill
    pub skill: Skill,
    /// Relevance score (0.0 to 1.0)
    pub relevance_score: f64,
    /// Reason for the match
    pub reason: String,
}

/// SkillKernel - manages skill extraction and matching
pub struct SkillKernel {
    db: Arc<Db>,
}

impl SkillKernel {
    /// Create a new SkillKernel
    pub fn new(db: Arc<Db>) -> Self {
        Self { db }
    }

    /// Extract skills from a successful flow execution
    pub fn extract_skills(&self, request: SkillExtractionRequest) -> Result<Vec<Skill>> {
        // Only extract from successful executions
        if request.duration_ms == 0 {
            return Ok(vec![]);
        }

        // Analyze execution metadata to identify patterns
        let mut skills = Vec::new();

        // Extract skill based on flow type
        let skill = self.create_skill_from_execution(&request)?;
        skills.push(skill);

        // Store skills in database
        for skill in &skills {
            self.store_skill(skill)?;
        }

        Ok(skills)
    }

    /// Create a skill from execution data
    fn create_skill_from_execution(&self, request: &SkillExtractionRequest) -> Result<Skill> {
        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now();

        // Infer capability signature from execution metadata
        let capability_signature = self.infer_capability_signature(request)?;

        // Create example from this execution
        let example = SkillExample {
            id: Uuid::new_v4().to_string(),
            flow_run_id: request.flow_run_id.clone(),
            input: request.input.clone(),
            output: request.final_output.clone(),
            duration_ms: request.duration_ms,
        };

        // Generate skill name and description from flow name
        let name = self.generate_skill_name(&request.flow_name);
        let description = format!(
            "Skill extracted from successful execution of {} flow",
            request.flow_name
        );

        Ok(Skill {
            id,
            name,
            description,
            capability_signature,
            examples: vec![example],
            success_rate: 1.0, // First example is successful
            avg_duration_ms: request.duration_ms,
            usage_count: 0,
            tags: self.generate_tags(&request.flow_name),
            created_at: now,
            updated_at: now,
        })
    }

    /// Infer capability signature from execution metadata
    fn infer_capability_signature(&self, request: &SkillExtractionRequest) -> Result<CapabilitySignature> {
        // Infer capability signature from execution
        let input_schema = self.infer_schema(&request.input);
        let output_schema = self.infer_schema(&request.final_output);

        // Extract required tools from execution metadata
        let required_tools = self.extract_required_tools(&request.execution_metadata);

        // Default to conservative permissions
        let required_permissions = vec!["FileRead".to_string()];

        Ok(CapabilitySignature {
            input_schema,
            output_schema,
            required_tools,
            required_permissions,
        })
    }

    /// Infer JSON Schema from a value
    fn infer_schema(&self, value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::String(_) => serde_json::json!({"type": "string"}),
            serde_json::Value::Number(_) => serde_json::json!({"type": "number"}),
            serde_json::Value::Bool(_) => serde_json::json!({"type": "boolean"}),
            serde_json::Value::Array(_) => serde_json::json!({"type": "array"}),
            serde_json::Value::Object(_) => serde_json::json!({"type": "object"}),
            serde_json::Value::Null => serde_json::json!({"type": "null"}),
        }
    }

    /// Extract required tools from execution metadata
    fn extract_required_tools(&self, metadata: &HashMap<String, serde_json::Value>) -> Vec<String> {
        let mut tools = Vec::new();

        for (_node_id, metadata_value) in metadata {
            if let Some(tool_name) = metadata_value.get("tool_name").and_then(|v| v.as_str()) {
                if !tools.contains(&tool_name.to_string()) {
                    tools.push(tool_name.to_string());
                }
            }
        }

        tools
    }

    /// Generate skill name from flow name
    fn generate_skill_name(&self, flow_name: &str) -> String {
        // Convert flow name to skill name (e.g., "planning_flow" -> "Planning")
        flow_name
            .replace("_flow", "")
            .replace("_", " ")
            .split_whitespace()
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().chain(chars).collect(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Generate tags from flow name
    fn generate_tags(&self, flow_name: &str) -> Vec<String> {
        let mut tags = vec!["extracted".to_string()];

        // Add domain-specific tags based on flow name
        if flow_name.contains("plan") {
            tags.push("planning".to_string());
        }
        if flow_name.contains("code") || flow_name.contains("gen") {
            tags.push("code-generation".to_string());
        }
        if flow_name.contains("review") {
            tags.push("review".to_string());
        }

        tags
    }

    /// Store a skill in the database
    fn store_skill(&self, skill: &Skill) -> Result<()> {
        // Check if skills table exists, create if not
        self.init_skills_table()?;

        // Insert skill
        let skill_json = serde_json::to_string(skill).context("Failed to serialize skill")?;

        self.db.conn().execute(
            "INSERT OR REPLACE INTO skills (id, skill_json, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
            [
                &skill.id,
                &skill_json,
                &skill.created_at.to_rfc3339(),
                &skill.updated_at.to_rfc3339(),
            ],
        ).context("Failed to store skill")?;

        Ok(())
    }

    /// Initialize skills table
    fn init_skills_table(&self) -> Result<()> {
        self.db.conn().execute(
            "CREATE TABLE IF NOT EXISTS skills (
                id TEXT PRIMARY KEY,
                skill_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
            [],
        ).context("Failed to create skills table")?;

        Ok(())
    }

    /// List all skills
    pub fn list_skills(&self) -> Result<Vec<Skill>> {
        self.init_skills_table()?;

        let mut stmt = self.db.conn().prepare("SELECT skill_json FROM skills ORDER BY created_at DESC")?;
        let mut skills = Vec::new();

        let rows = stmt.query_map([], |row| {
            let skill_json: String = row.get(0)?;
            Ok(skill_json)
        })?;

        for row in rows {
            let skill_json = row?;
            let skill: Skill = serde_json::from_str(&skill_json).context("Failed to deserialize skill")?;
            skills.push(skill);
        }

        Ok(skills)
    }

    /// Get a skill by ID
    pub fn get_skill(&self, id: &str) -> Result<Option<Skill>> {
        self.init_skills_table()?;

        let mut stmt = self.db.conn().prepare("SELECT skill_json FROM skills WHERE id = ?1")?;
        let mut rows = stmt.query_map([id], |row| {
            let skill_json: String = row.get(0)?;
            Ok(skill_json)
        })?;

        if let Some(row) = rows.next() {
            let skill_json = row?;
            let skill: Skill = serde_json::from_str(&skill_json).context("Failed to deserialize skill")?;
            Ok(Some(skill))
        } else {
            Ok(None)
        }
    }

    /// Match skills to a task
    pub fn match_skills(&self, request: SkillMatchingRequest) -> Result<Vec<SkillMatch>> {
        let all_skills = self.list_skills()?;

        let mut matches = Vec::new();

        for skill in all_skills {
            let relevance_score = self.calculate_relevance(&skill, &request);

            if relevance_score > 0.3 {
                // Only include reasonably relevant skills
                matches.push(SkillMatch {
                    skill,
                    relevance_score,
                    reason: "Pattern match in capability signature".to_string(),
                });
            }
        }

        // Sort by relevance score
        matches.sort_by(|a, b| b.relevance_score.partial_cmp(&a.relevance_score).unwrap());

        // Limit results
        matches.truncate(request.max_results);

        Ok(matches)
    }

    /// Calculate relevance score for a skill
    fn calculate_relevance(&self, skill: &Skill, request: &SkillMatchingRequest) -> f64 {
        let mut score = 0.0;

        // Check tool overlap
        let tool_overlap: usize = skill
            .capability_signature
            .required_tools
            .iter()
            .filter(|tool| request.required_tools.contains(tool))
            .count();

        if !request.required_tools.is_empty() {
            score += (tool_overlap as f64) / (request.required_tools.len() as f64) * 0.5;
        }

        // Check success rate
        score += skill.success_rate * 0.3;

        // Check usage count (more used = more relevant)
        if skill.usage_count > 0 {
            score += (skill.usage_count as f64).min(10.0) / 10.0 * 0.2;
        }

        score.min(1.0)
    }

    /// Update skill usage statistics
    pub fn record_skill_usage(&self, skill_id: &str) -> Result<()> {
        if let Some(mut skill) = self.get_skill(skill_id)? {
            skill.usage_count += 1;
            skill.updated_at = chrono::Utc::now();
            self.store_skill(&skill)?;
        }

        Ok(())
    }
}
