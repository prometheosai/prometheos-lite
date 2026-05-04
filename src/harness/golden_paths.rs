//! Golden Paths - Issue #27
//! Predefined workflow templates for common development tasks

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GoldenPath {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: PathCategory,
    pub complexity: PathComplexity,
    pub steps: Vec<PathStep>,
    pub validation_rules: Vec<ValidationRule>,
    pub estimated_duration_ms: u64,
    pub required_context: Vec<String>,
    pub success_criteria: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PathCategory {
    BugFix,
    FeatureImplementation,
    Refactoring,
    Testing,
    Documentation,
    Configuration,
    Migration,
    Optimization,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PathComplexity {
    Simple,
    Moderate,
    Complex,
    Expert,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PathStep {
    pub id: String,
    pub name: String,
    pub description: String,
    pub step_type: StepType,
    pub tool_invocations: Vec<ToolInvocation>,
    pub validation: Option<StepValidation>,
    pub rollback_point: bool,
    pub optional: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum StepType {
    Analysis,
    Generation,
    Validation,
    Testing,
    Review,
    Documentation,
    Commit,
    Deploy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolInvocation {
    pub tool_id: String,
    pub args: Vec<String>,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StepValidation {
    pub condition: String,
    pub required_outcome: String,
    pub retry_on_failure: bool,
    pub max_retries: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValidationRule {
    pub rule_type: RuleType,
    pub condition: String,
    pub error_message: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RuleType {
    FileMustExist,
    FileMustNotExist,
    TestMustPass,
    LintMustPass,
    BuildMustSucceed,
    CoverageMinimum,
}

#[derive(Debug, Clone)]
pub struct GoldenPathRegistry {
    paths: HashMap<String, GoldenPath>,
    execution_history: Vec<PathExecution>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathExecution {
    pub path_id: String,
    pub context_id: String,
    pub start_time: chrono::DateTime<chrono::Utc>,
    pub end_time: Option<chrono::DateTime<chrono::Utc>>,
    pub steps_completed: Vec<String>,
    pub current_step: Option<String>,
    pub success: bool,
    pub execution_log: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PathMatch {
    pub path: GoldenPath,
    pub match_score: f64,
    pub reason: String,
}

impl GoldenPathRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            paths: HashMap::new(),
            execution_history: Vec::new(),
        };
        registry.register_default_paths();
        registry
    }

    fn register_default_paths(&mut self) {
        // Bug Fix Path
        self.register(GoldenPath {
            id: "bug-fix".to_string(),
            name: "Standard Bug Fix".to_string(),
            description: "Systematic approach to fixing bugs".to_string(),
            category: PathCategory::BugFix,
            complexity: PathComplexity::Moderate,
            steps: vec![
                PathStep {
                    id: "analyze".to_string(),
                    name: "Analyze Issue".to_string(),
                    description: "Understand the bug and its context".to_string(),
                    step_type: StepType::Analysis,
                    tool_invocations: vec![],
                    validation: None,
                    rollback_point: true,
                    optional: false,
                },
                PathStep {
                    id: "reproduce".to_string(),
                    name: "Create Reproduction Test".to_string(),
                    description: "Write a test that fails with the bug".to_string(),
                    step_type: StepType::Testing,
                    tool_invocations: vec![ToolInvocation {
                        tool_id: "cargo-test".to_string(),
                        args: vec!["--test".to_string()],
                        required: true,
                    }],
                    validation: Some(StepValidation {
                        condition: "test_fails".to_string(),
                        required_outcome: "Test should initially fail".to_string(),
                        retry_on_failure: false,
                        max_retries: 0,
                    }),
                    rollback_point: true,
                    optional: false,
                },
                PathStep {
                    id: "fix".to_string(),
                    name: "Implement Fix".to_string(),
                    description: "Fix the bug with minimal changes".to_string(),
                    step_type: StepType::Generation,
                    tool_invocations: vec![],
                    validation: None,
                    rollback_point: true,
                    optional: false,
                },
                PathStep {
                    id: "validate".to_string(),
                    name: "Validate Fix".to_string(),
                    description: "Run tests to confirm fix works".to_string(),
                    step_type: StepType::Validation,
                    tool_invocations: vec![
                        ToolInvocation {
                            tool_id: "cargo-test".to_string(),
                            args: vec![],
                            required: true,
                        },
                        ToolInvocation {
                            tool_id: "cargo-check".to_string(),
                            args: vec![],
                            required: true,
                        },
                    ],
                    validation: Some(StepValidation {
                        condition: "tests_pass".to_string(),
                        required_outcome: "All tests must pass".to_string(),
                        retry_on_failure: true,
                        max_retries: 3,
                    }),
                    rollback_point: false,
                    optional: false,
                },
                PathStep {
                    id: "review".to_string(),
                    name: "Review Changes".to_string(),
                    description: "Review the fix for quality".to_string(),
                    step_type: StepType::Review,
                    tool_invocations: vec![ToolInvocation {
                        tool_id: "clippy".to_string(),
                        args: vec![],
                        required: false,
                    }],
                    validation: None,
                    rollback_point: false,
                    optional: true,
                },
            ],
            validation_rules: vec![
                ValidationRule {
                    rule_type: RuleType::TestMustPass,
                    condition: "all_tests".to_string(),
                    error_message: "All tests must pass after fix".to_string(),
                },
                ValidationRule {
                    rule_type: RuleType::BuildMustSucceed,
                    condition: "cargo_build".to_string(),
                    error_message: "Project must build successfully".to_string(),
                },
            ],
            estimated_duration_ms: 300000, // 5 minutes
            required_context: vec!["bug_description".to_string(), "repro_steps".to_string()],
            success_criteria: vec!["test_passes".to_string(), "no_regression".to_string()],
        });

        // Feature Implementation Path
        self.register(GoldenPath {
            id: "feature-impl".to_string(),
            name: "Feature Implementation".to_string(),
            description: "Standard approach to implementing new features".to_string(),
            category: PathCategory::FeatureImplementation,
            complexity: PathComplexity::Complex,
            steps: vec![
                PathStep {
                    id: "design".to_string(),
                    name: "Design Feature".to_string(),
                    description: "Plan the implementation approach".to_string(),
                    step_type: StepType::Analysis,
                    tool_invocations: vec![],
                    validation: None,
                    rollback_point: true,
                    optional: false,
                },
                PathStep {
                    id: "tests".to_string(),
                    name: "Write Tests First".to_string(),
                    description: "Create tests defining expected behavior".to_string(),
                    step_type: StepType::Testing,
                    tool_invocations: vec![],
                    validation: Some(StepValidation {
                        condition: "tests_compile".to_string(),
                        required_outcome: "Tests should compile but fail".to_string(),
                        retry_on_failure: false,
                        max_retries: 0,
                    }),
                    rollback_point: true,
                    optional: false,
                },
                PathStep {
                    id: "implement".to_string(),
                    name: "Implement Feature".to_string(),
                    description: "Write the feature implementation".to_string(),
                    step_type: StepType::Generation,
                    tool_invocations: vec![],
                    validation: None,
                    rollback_point: true,
                    optional: false,
                },
                PathStep {
                    id: "integrate".to_string(),
                    name: "Integrate and Test".to_string(),
                    description: "Integrate with existing code and run tests".to_string(),
                    step_type: StepType::Validation,
                    tool_invocations: vec![
                        ToolInvocation {
                            tool_id: "cargo-test".to_string(),
                            args: vec![],
                            required: true,
                        },
                        ToolInvocation {
                            tool_id: "cargo-check".to_string(),
                            args: vec![],
                            required: true,
                        },
                    ],
                    validation: Some(StepValidation {
                        condition: "tests_pass".to_string(),
                        required_outcome: "All tests must pass".to_string(),
                        retry_on_failure: true,
                        max_retries: 3,
                    }),
                    rollback_point: false,
                    optional: false,
                },
                PathStep {
                    id: "document".to_string(),
                    name: "Document Feature".to_string(),
                    description: "Add documentation for the new feature".to_string(),
                    step_type: StepType::Documentation,
                    tool_invocations: vec![],
                    validation: None,
                    rollback_point: false,
                    optional: true,
                },
            ],
            validation_rules: vec![
                ValidationRule {
                    rule_type: RuleType::TestMustPass,
                    condition: "all_tests".to_string(),
                    error_message: "All tests must pass".to_string(),
                },
                ValidationRule {
                    rule_type: RuleType::LintMustPass,
                    condition: "clippy".to_string(),
                    error_message: "Linting must pass".to_string(),
                },
            ],
            estimated_duration_ms: 600000, // 10 minutes
            required_context: vec![
                "feature_spec".to_string(),
                "acceptance_criteria".to_string(),
            ],
            success_criteria: vec![
                "tests_pass".to_string(),
                "lints_pass".to_string(),
                "documented".to_string(),
            ],
        });

        // Refactoring Path
        self.register(GoldenPath {
            id: "refactor".to_string(),
            name: "Safe Refactoring".to_string(),
            description: "Systematic refactoring with safety checks".to_string(),
            category: PathCategory::Refactoring,
            complexity: PathComplexity::Moderate,
            steps: vec![
                PathStep {
                    id: "identify".to_string(),
                    name: "Identify Refactoring Target".to_string(),
                    description: "Determine what needs refactoring and why".to_string(),
                    step_type: StepType::Analysis,
                    tool_invocations: vec![],
                    validation: None,
                    rollback_point: true,
                    optional: false,
                },
                PathStep {
                    id: "ensure_tests".to_string(),
                    name: "Ensure Test Coverage".to_string(),
                    description: "Verify existing tests cover the refactoring area".to_string(),
                    step_type: StepType::Testing,
                    tool_invocations: vec![ToolInvocation {
                        tool_id: "cargo-test".to_string(),
                        args: vec![],
                        required: true,
                    }],
                    validation: Some(StepValidation {
                        condition: "tests_exist".to_string(),
                        required_outcome: "Tests must exist and pass".to_string(),
                        retry_on_failure: false,
                        max_retries: 0,
                    }),
                    rollback_point: true,
                    optional: false,
                },
                PathStep {
                    id: "small_steps".to_string(),
                    name: "Apply Small Changes".to_string(),
                    description: "Make small, incremental refactoring changes".to_string(),
                    step_type: StepType::Generation,
                    tool_invocations: vec![],
                    validation: None,
                    rollback_point: true,
                    optional: false,
                },
                PathStep {
                    id: "validate_each".to_string(),
                    name: "Validate Each Step".to_string(),
                    description: "Run tests after each change".to_string(),
                    step_type: StepType::Validation,
                    tool_invocations: vec![ToolInvocation {
                        tool_id: "cargo-test".to_string(),
                        args: vec![],
                        required: true,
                    }],
                    validation: Some(StepValidation {
                        condition: "tests_pass".to_string(),
                        required_outcome: "Tests must pass after each change".to_string(),
                        retry_on_failure: true,
                        max_retries: 1,
                    }),
                    rollback_point: true,
                    optional: false,
                },
            ],
            validation_rules: vec![ValidationRule {
                rule_type: RuleType::TestMustPass,
                condition: "all_tests".to_string(),
                error_message: "All tests must pass throughout refactoring".to_string(),
            }],
            estimated_duration_ms: 450000, // 7.5 minutes
            required_context: vec!["refactoring_goal".to_string()],
            success_criteria: vec!["tests_pass".to_string(), "code_improved".to_string()],
        });

        // Documentation Path
        self.register(GoldenPath {
            id: "docs".to_string(),
            name: "Documentation Update".to_string(),
            description: "Add or update documentation".to_string(),
            category: PathCategory::Documentation,
            complexity: PathComplexity::Simple,
            steps: vec![
                PathStep {
                    id: "identify_docs".to_string(),
                    name: "Identify Documentation Needs".to_string(),
                    description: "Determine what needs documentation".to_string(),
                    step_type: StepType::Analysis,
                    tool_invocations: vec![],
                    validation: None,
                    rollback_point: false,
                    optional: false,
                },
                PathStep {
                    id: "write_docs".to_string(),
                    name: "Write Documentation".to_string(),
                    description: "Add doc comments and README updates".to_string(),
                    step_type: StepType::Documentation,
                    tool_invocations: vec![],
                    validation: None,
                    rollback_point: false,
                    optional: false,
                },
                PathStep {
                    id: "validate_docs".to_string(),
                    name: "Validate Documentation".to_string(),
                    description: "Check doc tests and links".to_string(),
                    step_type: StepType::Validation,
                    tool_invocations: vec![ToolInvocation {
                        tool_id: "cargo-test".to_string(),
                        args: vec!["--doc".to_string()],
                        required: true,
                    }],
                    validation: Some(StepValidation {
                        condition: "doc_tests_pass".to_string(),
                        required_outcome: "Documentation tests must pass".to_string(),
                        retry_on_failure: true,
                        max_retries: 2,
                    }),
                    rollback_point: false,
                    optional: false,
                },
            ],
            validation_rules: vec![ValidationRule {
                rule_type: RuleType::TestMustPass,
                condition: "doc_tests".to_string(),
                error_message: "Documentation tests must pass".to_string(),
            }],
            estimated_duration_ms: 120000, // 2 minutes
            required_context: vec!["documentation_scope".to_string()],
            success_criteria: vec!["doc_tests_pass".to_string()],
        });
    }

    pub fn register(&mut self, path: GoldenPath) {
        self.paths.insert(path.id.clone(), path);
    }

    pub fn get(&self, path_id: &str) -> Option<&GoldenPath> {
        self.paths.get(path_id)
    }

    pub fn list(&self) -> Vec<&GoldenPath> {
        self.paths.values().collect()
    }

    pub fn list_by_category(&self, category: PathCategory) -> Vec<&GoldenPath> {
        self.paths
            .values()
            .filter(|p| p.category == category)
            .collect()
    }

    pub fn find_matching_paths(
        &self,
        context: &str,
        category: Option<PathCategory>,
    ) -> Vec<PathMatch> {
        let mut matches = Vec::new();
        let context_lower = context.to_lowercase();

        for path in self.paths.values() {
            // Check category filter
            if let Some(cat) = category {
                if path.category != cat {
                    continue;
                }
            }

            // Calculate match score
            let mut score: f64 = 0.0;
            let mut reasons = Vec::new();

            // Check keywords in context
            let description_lower = path.description.to_lowercase();
            let keywords: Vec<_> = description_lower
                .split_whitespace()
                .filter(|w| w.len() > 4)
                .collect();

            for keyword in &keywords {
                if context_lower.contains(keyword) {
                    score += 0.2;
                    reasons.push(format!("Matches keyword: {}", keyword));
                }
            }

            // Check category hints
            let category_hint = match path.category {
                PathCategory::BugFix => "bug fix error issue",
                PathCategory::FeatureImplementation => "feature implement add new",
                PathCategory::Refactoring => "refactor improve clean",
                PathCategory::Testing => "test coverage",
                PathCategory::Documentation => "document doc readme",
                PathCategory::Configuration => "config setup",
                PathCategory::Migration => "migrate upgrade",
                PathCategory::Optimization => "optimize performance speed",
            };

            for hint in category_hint.split_whitespace() {
                if context_lower.contains(hint) {
                    score += 0.3;
                    reasons.push(format!("Category hint: {}", hint));
                    break;
                }
            }

            // Cap score at 1.0
            score = score.min(1.0);

            if score >= 0.3 {
                matches.push(PathMatch {
                    path: path.clone(),
                    match_score: score,
                    reason: reasons.join(", "),
                });
            }
        }

        matches.sort_by(|a, b| b.match_score.partial_cmp(&a.match_score).unwrap());
        matches
    }

    pub fn start_execution(&mut self, path_id: &str, context_id: &str) -> Result<usize> {
        let path = self
            .paths
            .get(path_id)
            .ok_or_else(|| anyhow::anyhow!("Path '{}' not found", path_id))?;

        let execution = PathExecution {
            path_id: path_id.to_string(),
            context_id: context_id.to_string(),
            start_time: chrono::Utc::now(),
            end_time: None,
            steps_completed: Vec::new(),
            current_step: None,
            success: false,
            execution_log: vec![format!("Started golden path: {}", path.name)],
        };

        self.execution_history.push(execution);
        Ok(self.execution_history.len() - 1)
    }

    pub fn record_step_completion(
        &mut self,
        execution_idx: usize,
        step_id: &str,
        success: bool,
    ) -> Result<()> {
        if let Some(execution) = self.execution_history.get_mut(execution_idx) {
            execution.steps_completed.push(step_id.to_string());
            execution.execution_log.push(format!(
                "Step {} completed: {}",
                step_id,
                if success { "success" } else { "failure" }
            ));
            Ok(())
        } else {
            bail!("Invalid execution index")
        }
    }

    pub fn complete_execution(&mut self, execution_idx: usize, success: bool) -> Result<()> {
        if let Some(execution) = self.execution_history.get_mut(execution_idx) {
            execution.end_time = Some(chrono::Utc::now());
            execution.success = success;
            execution.execution_log.push(format!(
                "Execution completed: {}",
                if success { "success" } else { "failure" }
            ));
            Ok(())
        } else {
            bail!("Invalid execution index")
        }
    }

    pub fn get_execution_history(&self) -> &[PathExecution] {
        &self.execution_history
    }

    pub fn get_path_stats(&self, path_id: &str) -> Option<PathStats> {
        let executions: Vec<_> = self
            .execution_history
            .iter()
            .filter(|e| e.path_id == path_id)
            .collect();

        if executions.is_empty() {
            return None;
        }

        let total = executions.len();
        let successful = executions.iter().filter(|e| e.success).count();

        let avg_duration = executions
            .iter()
            .filter_map(|e| {
                e.end_time
                    .map(|end| (end - e.start_time).num_milliseconds() as u64)
            })
            .sum::<u64>()
            / total as u64;

        Some(PathStats {
            path_id: path_id.to_string(),
            total_executions: total as u32,
            successful_executions: successful as u32,
            failed_executions: (total - successful) as u32,
            average_duration_ms: avg_duration,
            success_rate: successful as f64 / total as f64,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathStats {
    pub path_id: String,
    pub total_executions: u32,
    pub successful_executions: u32,
    pub failed_executions: u32,
    pub average_duration_ms: u64,
    pub success_rate: f64,
}

pub fn create_golden_path_registry() -> GoldenPathRegistry {
    GoldenPathRegistry::new()
}

pub fn format_path_match(path_match: &PathMatch) -> String {
    format!(
        r#"{} (Score: {:.0}%)
   {}
   Complexity: {:?}
   Match Reason: {}
"#,
        path_match.path.name,
        path_match.match_score * 100.0,
        path_match.path.description,
        path_match.path.complexity,
        path_match.reason
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_and_get_path() {
        let registry = GoldenPathRegistry::new();
        assert!(registry.get("bug-fix").is_some());
        assert_eq!(
            registry.get("bug-fix").unwrap().category,
            PathCategory::BugFix
        );
    }

    #[test]
    fn test_list_by_category() {
        let registry = GoldenPathRegistry::new();
        let bug_fix_paths = registry.list_by_category(PathCategory::BugFix);
        assert!(!bug_fix_paths.is_empty());
        assert!(
            bug_fix_paths
                .iter()
                .all(|p| p.category == PathCategory::BugFix)
        );
    }

    #[test]
    fn test_find_matching_paths() {
        let registry = GoldenPathRegistry::new();
        let matches = registry.find_matching_paths("I need to fix a bug", None);
        assert!(!matches.is_empty());
        assert!(matches[0].path.id == "bug-fix");
    }

    #[test]
    fn test_path_execution() {
        let mut registry = GoldenPathRegistry::new();
        let idx = registry.start_execution("bug-fix", "ctx-123").unwrap();

        registry
            .record_step_completion(idx, "analyze", true)
            .unwrap();
        registry
            .record_step_completion(idx, "reproduce", true)
            .unwrap();
        registry.complete_execution(idx, true).unwrap();

        let history = registry.get_execution_history();
        assert_eq!(history.len(), 1);
        assert!(history[0].success);
    }
}
