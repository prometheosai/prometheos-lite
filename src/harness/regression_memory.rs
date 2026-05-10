//! Regression Memory - Issue #26
//! Learn from failures to prevent recurring issues

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FailurePattern {
    pub id: String,
    pub pattern_signature: String,
    pub failure_type: FailureType,
    pub context_hash: String,
    pub error_signature: String,
    pub file_path: Option<PathBuf>,
    pub line_number: Option<u32>,
    pub frequency: u32,
    pub first_seen: chrono::DateTime<chrono::Utc>,
    pub last_seen: chrono::DateTime<chrono::Utc>,
    pub successful_solutions: Vec<SuccessfulSolution>,
    pub unsuccessful_attempts: Vec<UnsuccessfulAttempt>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SuccessfulSolution {
    pub solution_id: String,
    pub description: String,
    pub approach: String,
    pub success_count: u32,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UnsuccessfulAttempt {
    pub attempt_id: String,
    pub description: String,
    pub failure_reason: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum FailureType {
    SyntaxError,
    TypeError,
    RuntimeError,
    TestFailure,
    CompilationError,
    LintError,
    LogicError,
    PerformanceIssue,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct RegressionMemory {
    patterns: HashMap<String, FailurePattern>,
    file_index: HashMap<PathBuf, Vec<String>>,
    type_index: HashMap<FailureType, Vec<String>>,
    access_log: Vec<MemoryAccess>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MemoryAccess {
    pattern_id: String,
    access_type: AccessType,
    timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum AccessType {
    Read,
    Write,
    Match,
    Learn,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PatternMatch {
    pub pattern: FailurePattern,
    pub similarity: f64,
    pub recommended_solutions: Vec<SuccessfulSolution>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LearningResult {
    pub new_pattern: bool,
    pub pattern_id: String,
    pub learned_solution: bool,
    pub confidence_improvement: f64,
}

impl RegressionMemory {
    pub fn new() -> Self {
        Self {
            patterns: HashMap::new(),
            file_index: HashMap::new(),
            type_index: HashMap::new(),
            access_log: Vec::new(),
        }
    }

    pub fn record_failure(
        &mut self,
        failure_type: FailureType,
        error_message: &str,
        file_path: Option<&Path>,
        line_number: Option<u32>,
        context: &str,
    ) -> String {
        let pattern_signature = self.generate_signature(error_message, file_path, line_number);
        let context_hash = self.hash_context(context);

        let pattern_id = if let Some(existing) = self.patterns.get_mut(&pattern_signature) {
            // Update existing pattern
            existing.frequency += 1;
            existing.last_seen = chrono::Utc::now();
            existing.pattern_signature.clone()
        } else {
            // Create new pattern
            let id = format!("pattern-{}", self.patterns.len() + 1);
            let pattern = FailurePattern {
                id: id.clone(),
                pattern_signature: pattern_signature.clone(),
                failure_type,
                context_hash,
                error_signature: error_message.to_string(),
                file_path: file_path.map(|p| p.to_path_buf()),
                line_number,
                frequency: 1,
                first_seen: chrono::Utc::now(),
                last_seen: chrono::Utc::now(),
                successful_solutions: Vec::new(),
                unsuccessful_attempts: Vec::new(),
            };

            self.patterns.insert(pattern_signature.clone(), pattern);

            // Index by file
            if let Some(path) = file_path {
                self.file_index
                    .entry(path.to_path_buf())
                    .or_insert_with(Vec::new)
                    .push(pattern_signature.clone());
            }

            // Index by type
            self.type_index
                .entry(failure_type)
                .or_insert_with(Vec::new)
                .push(pattern_signature.clone());

            pattern_signature
        };

        self.log_access(&pattern_id, AccessType::Write);
        pattern_id
    }

    pub fn find_similar_failures(
        &self,
        failure_type: FailureType,
        error_message: &str,
        file_path: Option<&Path>,
        context: &str,
    ) -> Vec<PatternMatch> {
        let mut matches = Vec::new();
        let context_hash = self.hash_context(context);

        // Search by type
        if let Some(pattern_ids) = self.type_index.get(&failure_type) {
            for id in pattern_ids {
                if let Some(pattern) = self.patterns.get(id) {
                    let similarity =
                        self.calculate_similarity(pattern, error_message, &context_hash, file_path);

                    if similarity > 0.7 {
                        let mut recommended = pattern.successful_solutions.clone();
                        recommended
                            .sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());

                        matches.push(PatternMatch {
                            pattern: pattern.clone(),
                            similarity,
                            recommended_solutions: recommended.into_iter().take(3).collect(),
                        });
                    }
                }
            }
        }

        // Search by file
        if let Some(path) = file_path {
            if let Some(pattern_ids) = self.file_index.get(path) {
                for id in pattern_ids {
                    if let Some(pattern) = self.patterns.get(id) {
                        // Skip if already added
                        if matches.iter().any(|m| m.pattern.id == pattern.id) {
                            continue;
                        }

                        let similarity = self.calculate_similarity(
                            pattern,
                            error_message,
                            &context_hash,
                            file_path,
                        );

                        if similarity > 0.5 {
                            matches.push(PatternMatch {
                                pattern: pattern.clone(),
                                similarity,
                                recommended_solutions: pattern.successful_solutions.clone(),
                            });
                        }
                    }
                }
            }
        }

        matches.sort_by(|a, b| b.similarity.partial_cmp(&a.similarity).unwrap());
        matches
    }

    pub fn learn_solution(
        &mut self,
        pattern_id: &str,
        solution_description: &str,
        solution_approach: &str,
        success: bool,
    ) -> LearningResult {
        let _now = chrono::Utc::now();

        if let Some(pattern) = self.patterns.get_mut(pattern_id) {
            if success {
                // Check if solution already exists
                if let Some(existing) = pattern
                    .successful_solutions
                    .iter_mut()
                    .find(|s| s.approach == solution_approach)
                {
                    existing.success_count += 1;
                    existing.confidence = (existing.confidence * 0.9) + 0.1; // Increase confidence
                } else {
                    pattern.successful_solutions.push(SuccessfulSolution {
                        solution_id: format!("sol-{}", pattern.successful_solutions.len() + 1),
                        description: solution_description.to_string(),
                        approach: solution_approach.to_string(),
                        success_count: 1,
                        confidence: 0.5, // Start with moderate confidence
                    });
                }

                self.log_access(pattern_id, AccessType::Learn);

                LearningResult {
                    new_pattern: false,
                    pattern_id: pattern_id.to_string(),
                    learned_solution: true,
                    confidence_improvement: 0.1,
                }
            } else {
                pattern.unsuccessful_attempts.push(UnsuccessfulAttempt {
                    attempt_id: format!("att-{}", pattern.unsuccessful_attempts.len() + 1),
                    description: solution_description.to_string(),
                    failure_reason: "Solution failed".to_string(),
                });

                LearningResult {
                    new_pattern: false,
                    pattern_id: pattern_id.to_string(),
                    learned_solution: false,
                    confidence_improvement: -0.05,
                }
            }
        } else {
            LearningResult {
                new_pattern: false,
                pattern_id: pattern_id.to_string(),
                learned_solution: false,
                confidence_improvement: 0.0,
            }
        }
    }

    pub fn get_hot_patterns(&self, limit: usize) -> Vec<FailurePattern> {
        let mut patterns: Vec<_> = self.patterns.values().cloned().collect();
        patterns.sort_by(|a, b| b.frequency.cmp(&a.frequency));
        patterns.into_iter().take(limit).collect()
    }

    pub fn get_patterns_for_file(&self, file_path: &Path) -> Vec<FailurePattern> {
        self.file_index
            .get(file_path)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.patterns.get(id).cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn get_memory_stats(&self) -> MemoryStats {
        let total_patterns = self.patterns.len();
        let total_solutions: usize = self
            .patterns
            .values()
            .map(|p| p.successful_solutions.len())
            .sum();
        let total_failures: u32 = self.patterns.values().map(|p| p.frequency).sum();

        let patterns_with_solutions = self
            .patterns
            .values()
            .filter(|p| !p.successful_solutions.is_empty())
            .count();

        MemoryStats {
            total_patterns,
            total_solutions,
            total_failures,
            patterns_with_solutions,
            average_solutions_per_pattern: if total_patterns > 0 {
                total_solutions as f64 / total_patterns as f64
            } else {
                0.0
            },
            solve_rate: if total_patterns > 0 {
                patterns_with_solutions as f64 / total_patterns as f64
            } else {
                0.0
            },
        }
    }

    pub fn export_memory(&self) -> Vec<FailurePattern> {
        self.patterns.values().cloned().collect()
    }

    pub fn import_memory(&mut self, patterns: Vec<FailurePattern>) {
        for pattern in patterns {
            self.patterns
                .insert(pattern.pattern_signature.clone(), pattern);
        }
    }

    fn generate_signature(
        &self,
        error_message: &str,
        file_path: Option<&Path>,
        line: Option<u32>,
    ) -> String {
        let file_part = file_path
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let line_part = line.map(|l| l.to_string()).unwrap_or_default();
        format!(
            "{}:{}:{}",
            self.hash_string(error_message),
            file_part,
            line_part
        )
    }

    fn hash_context(&self, context: &str) -> String {
        self.hash_string(context)
    }

    fn hash_string(&self, s: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        s.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    fn calculate_similarity(
        &self,
        pattern: &FailurePattern,
        error_message: &str,
        context_hash: &str,
        file_path: Option<&Path>,
    ) -> f64 {
        let mut score = 0.0;

        // Error message similarity (50%)
        if pattern.error_signature == error_message {
            score += 0.5;
        } else if error_message.contains(&pattern.error_signature)
            || pattern.error_signature.contains(error_message)
        {
            score += 0.3;
        }

        // Context similarity (30%)
        if pattern.context_hash == context_hash {
            score += 0.3;
        }

        // File path match (20%)
        if let Some(path) = file_path {
            if pattern.file_path.as_ref() == Some(&path.to_path_buf()) {
                score += 0.2;
            }
        }

        score
    }

    fn log_access(&mut self, pattern_id: &str, access_type: AccessType) {
        self.access_log.push(MemoryAccess {
            pattern_id: pattern_id.to_string(),
            access_type,
            timestamp: chrono::Utc::now(),
        });
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryStats {
    pub total_patterns: usize,
    pub total_solutions: usize,
    pub total_failures: u32,
    pub patterns_with_solutions: usize,
    pub average_solutions_per_pattern: f64,
    pub solve_rate: f64,
}

pub fn create_regression_memory() -> RegressionMemory {
    RegressionMemory::new()
}

pub fn format_memory_stats(stats: &MemoryStats) -> String {
    format!(
        r#"Regression Memory Statistics
=============================
Total Patterns: {}
Total Solutions: {}
Total Failures Recorded: {}
Patterns with Solutions: {} ({:.0}%)
Avg Solutions per Pattern: {:.2}
Solve Rate: {:.0}%
"#,
        stats.total_patterns,
        stats.total_solutions,
        stats.total_failures,
        stats.patterns_with_solutions,
        stats.solve_rate * 100.0,
        stats.average_solutions_per_pattern,
        stats.solve_rate * 100.0
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_and_find_failure() {
        let mut memory = RegressionMemory::new();

        let pattern_id = memory.record_failure(
            FailureType::SyntaxError,
            "unexpected token `}`",
            Some(Path::new("src/lib.rs")),
            Some(42),
            "fn foo() { bar() }",
        );

        assert!(!pattern_id.is_empty());

        let matches = memory.find_similar_failures(
            FailureType::SyntaxError,
            "unexpected token `}`",
            Some(Path::new("src/lib.rs")),
            "fn foo() { bar() }",
        );

        assert!(!matches.is_empty());
        assert_eq!(matches[0].pattern.pattern_signature, pattern_id);
    }

    #[test]
    fn test_learn_solution() {
        let mut memory = RegressionMemory::new();

        let pattern_id = memory.record_failure(
            FailureType::TestFailure,
            "assertion failed",
            Some(Path::new("tests/test.rs")),
            Some(10),
            "assert_eq!(result, 42)",
        );

        let result = memory.learn_solution(&pattern_id, "Fix off-by-one", "increment_fix", true);

        assert!(result.learned_solution);

        let stats = memory.get_memory_stats();
        assert_eq!(stats.total_solutions, 1);
    }

    #[test]
    fn test_get_hot_patterns() {
        let mut memory = RegressionMemory::new();

        // Record same failure multiple times
        for _ in 0..5 {
            memory.record_failure(
                FailureType::RuntimeError,
                "null pointer dereference",
                Some(Path::new("src/main.rs")),
                Some(20),
                "let x = *ptr",
            );
        }

        let hot_patterns = memory.get_hot_patterns(10);
        assert!(!hot_patterns.is_empty());
        assert_eq!(hot_patterns[0].frequency, 5);
    }
}
