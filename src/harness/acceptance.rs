use crate::harness::environment::EnvironmentProfile;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AcceptanceCriterion {
    pub id: String,
    pub description: String,
    pub verification_method: VerificationMethod,
    pub status: CriterionStatus,
    pub priority: CriterionPriority,
    pub detected_tests: Vec<String>,
    pub detected_checks: Vec<String>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum VerificationMethod {
    TestCommand(String),
    StaticCheck(String),
    LintCommand(String),
    FormatCommand(String),
    Review,
    Manual,
    FileExists(PathBuf),
    ContentMatch { file: PathBuf, pattern: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CriterionStatus {
    Pending,
    Passed,
    Failed,
    NotApplicable,
    Blocked,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum CriterionPriority {
    Critical,
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompiledAcceptanceCriteria {
    pub criteria: Vec<AcceptanceCriterion>,
    pub test_commands: Vec<String>,
    pub static_checks: Vec<String>,
    pub lint_commands: Vec<String>,
    pub format_commands: Vec<String>,
    pub total_priority_score: u32,
    pub auto_detected: usize,
    pub manual_review_required: usize,
}

#[derive(Debug, Clone, Default)]
pub struct AcceptanceCompiler {
    test_patterns: Vec<Regex>,
    check_patterns: Vec<Regex>,
    lint_patterns: Vec<Regex>,
    format_patterns: Vec<Regex>,
}

impl AcceptanceCompiler {
    pub fn new() -> Self {
        Self {
            test_patterns: vec![
                Regex::new(r"(?i)test\s+(should|must|can|verify|ensure|check)").unwrap(),
                Regex::new(r"(?i)unit\s+test").unwrap(),
                Regex::new(r"(?i)integration\s+test").unwrap(),
                Regex::new(r"(?i)add\s+.*\s+test").unwrap(),
                Regex::new(r"(?i)test\s+(that|if|whether)").unwrap(),
                Regex::new(r"(?i)assert").unwrap(),
            ],
            check_patterns: vec![
                Regex::new(r"(?i)static\s+(check|analysis)").unwrap(),
                Regex::new(r"(?i)clippy|lint|eslint").unwrap(),
                Regex::new(r"(?i)compile|build\s+error").unwrap(),
                Regex::new(r"(?i)type\s+check").unwrap(),
            ],
            lint_patterns: vec![
                Regex::new(r"(?i)lint|linting").unwrap(),
                Regex::new(r"(?i)style\s+(check|guide)").unwrap(),
                Regex::new(r"(?i)formatting").unwrap(),
            ],
            format_patterns: vec![
                Regex::new(r"(?i)format|fmt").unwrap(),
                Regex::new(r"(?i)rustfmt|prettier").unwrap(),
                Regex::new(r"(?i)code\s+style").unwrap(),
            ],
        }
    }

    pub fn compile(&self, reqs: &[String], env: &EnvironmentProfile) -> CompiledAcceptanceCriteria {
        let mut criteria = vec![];
        let mut test_commands = vec![];
        let mut static_checks = vec![];
        let mut lint_commands = vec![];
        let mut format_commands = vec![];
        let mut auto_detected = 0;
        let mut manual_review_required = 0;

        for (i, req) in reqs.iter().enumerate() {
            let (criterion, detected_tests, detected_checks, detected_lints, detected_formats) =
                self.analyze_criterion(req, i, env);

            if !detected_tests.is_empty() || !detected_checks.is_empty() {
                auto_detected += 1;
                test_commands.extend(detected_tests.clone());
                static_checks.extend(detected_checks.clone());
                lint_commands.extend(detected_lints.clone());
                format_commands.extend(detected_formats.clone());
            } else if matches!(
                criterion.verification_method,
                VerificationMethod::Review | VerificationMethod::Manual
            ) {
                manual_review_required += 1;
            }

            criteria.push(criterion);
        }

        // Add environment-based verification commands
        test_commands.extend(env.test_commands.clone());
        static_checks.extend(env.lint_commands.clone());
        lint_commands.extend(env.lint_commands.clone());
        format_commands.extend(env.format_commands.clone());

        // Deduplicate
        test_commands = deduplicate(test_commands);
        static_checks = deduplicate(static_checks);
        lint_commands = deduplicate(lint_commands);
        format_commands = deduplicate(format_commands);

        let total_priority_score = criteria
            .iter()
            .map(|c| match c.priority {
                CriterionPriority::Critical => 4,
                CriterionPriority::High => 3,
                CriterionPriority::Medium => 2,
                CriterionPriority::Low => 1,
            })
            .sum();

        CompiledAcceptanceCriteria {
            criteria,
            test_commands,
            static_checks,
            lint_commands,
            format_commands,
            total_priority_score,
            auto_detected,
            manual_review_required,
        }
    }

    fn analyze_criterion(
        &self,
        req: &str,
        index: usize,
        env: &EnvironmentProfile,
    ) -> (
        AcceptanceCriterion,
        Vec<String>,
        Vec<String>,
        Vec<String>,
        Vec<String>,
    ) {
        let detected_tests = self.detect_test_commands(req, env);
        let detected_checks = self.detect_static_checks(req, env);
        let detected_lints = self.detect_lint_commands(req, env);
        let detected_formats = self.detect_format_commands(req, env);

        let priority = self.infer_priority(req);
        let confidence =
            self.calculate_detection_confidence(req, &detected_tests, &detected_checks);

        let verification_method = if !detected_tests.is_empty() {
            VerificationMethod::TestCommand(detected_tests[0].clone())
        } else if !detected_checks.is_empty() {
            VerificationMethod::StaticCheck(detected_checks[0].clone())
        } else if !detected_lints.is_empty() {
            VerificationMethod::LintCommand(detected_lints[0].clone())
        } else if !detected_formats.is_empty() {
            VerificationMethod::FormatCommand(detected_formats[0].clone())
        } else if self.is_file_exists_check(req) {
            let path = self
                .extract_file_path(req)
                .unwrap_or_else(|| PathBuf::from("unknown"));
            VerificationMethod::FileExists(path)
        } else {
            VerificationMethod::Review
        };

        let criterion = AcceptanceCriterion {
            id: format!("AC-{:03}", index + 1),
            description: req.to_string(),
            verification_method,
            status: CriterionStatus::Pending,
            priority,
            detected_tests: detected_tests.clone(),
            detected_checks: detected_checks.clone(),
            confidence,
        };

        (
            criterion,
            detected_tests,
            detected_checks,
            detected_lints,
            detected_formats,
        )
    }

    fn detect_test_commands(&self, req: &str, env: &EnvironmentProfile) -> Vec<String> {
        let mut tests = vec![];

        for pattern in &self.test_patterns {
            if pattern.is_match(req) {
                let cmd = match env.test_commands.first() {
                    Some(cmd) => cmd.clone(),
                    None => {
                        if env.languages.contains(&"rust".to_string()) {
                            "cargo test".into()
                        } else if env.languages.contains(&"python".to_string()) {
                            "pytest".into()
                        } else if env.languages.contains(&"javascript".to_string()) {
                            "npm test".into()
                        } else if env.languages.contains(&"typescript".to_string()) {
                            "npm test".into()
                        } else {
                            "test".into()
                        }
                    }
                };
                tests.push(cmd);
                break;
            }
        }

        tests
    }

    fn detect_static_checks(&self, req: &str, env: &EnvironmentProfile) -> Vec<String> {
        let mut checks = vec![];

        for pattern in &self.check_patterns {
            if pattern.is_match(req) {
                let cmd = match env.lint_commands.first() {
                    Some(cmd) => cmd.clone(),
                    None => {
                        if env.languages.contains(&"rust".to_string()) {
                            "cargo check".into()
                        } else if env.languages.contains(&"python".to_string()) {
                            "mypy".into()
                        } else if env.languages.contains(&"javascript".to_string())
                            || env.languages.contains(&"typescript".to_string())
                        {
                            "eslint".into()
                        } else {
                            "check".into()
                        }
                    }
                };
                checks.push(cmd);
                break;
            }
        }

        checks
    }

    fn detect_lint_commands(&self, req: &str, env: &EnvironmentProfile) -> Vec<String> {
        let mut lints = vec![];

        for pattern in &self.lint_patterns {
            if pattern.is_match(req) {
                let cmd = match env.lint_commands.first() {
                    Some(cmd) => cmd.clone(),
                    None => {
                        if env.languages.contains(&"rust".to_string()) {
                            "cargo clippy".into()
                        } else if env.languages.contains(&"python".to_string()) {
                            "pylint".into()
                        } else if env.languages.contains(&"javascript".to_string()) {
                            "eslint".into()
                        } else {
                            "lint".into()
                        }
                    }
                };
                lints.push(cmd);
                break;
            }
        }

        lints
    }

    fn detect_format_commands(&self, req: &str, env: &EnvironmentProfile) -> Vec<String> {
        let mut formats = vec![];

        for pattern in &self.format_patterns {
            if pattern.is_match(req) {
                let cmd = match env.format_commands.first() {
                    Some(cmd) => cmd.clone(),
                    None => {
                        if env.languages.contains(&"rust".to_string()) {
                            "cargo fmt".into()
                        } else if env.languages.contains(&"python".to_string()) {
                            "black --check".into()
                        } else if env.languages.contains(&"javascript".to_string())
                            || env.languages.contains(&"typescript".to_string())
                        {
                            "prettier --check".into()
                        } else {
                            "fmt".into()
                        }
                    }
                };
                formats.push(cmd);
                break;
            }
        }

        formats
    }

    fn infer_priority(&self, req: &str) -> CriterionPriority {
        let req_lower = req.to_lowercase();

        if req_lower.contains("critical")
            || req_lower.contains("security")
            || req_lower.contains("crash")
            || req_lower.contains("data loss")
        {
            CriterionPriority::Critical
        } else if req_lower.contains("must")
            || req_lower.contains("required")
            || req_lower.contains("important")
            || req_lower.contains("ensure")
        {
            CriterionPriority::High
        } else if req_lower.contains("should") || req_lower.contains("recommend") {
            CriterionPriority::Medium
        } else {
            CriterionPriority::Low
        }
    }

    fn calculate_detection_confidence(
        &self,
        req: &str,
        tests: &[String],
        checks: &[String],
    ) -> f32 {
        let mut confidence: f32 = 0.5;

        if !tests.is_empty() {
            confidence += 0.25;
        }
        if !checks.is_empty() {
            confidence += 0.15;
        }

        let specificity_score = if req.len() > 100 { 0.1 } else { 0.0 };
        confidence += specificity_score;

        confidence.min(1.0)
    }

    fn is_file_exists_check(&self, req: &str) -> bool {
        let req_lower = req.to_lowercase();
        req_lower.contains("create") && req_lower.contains("file")
            || req_lower.contains("add") && req_lower.contains("file")
            || req_lower.contains("exists") && req_lower.contains("file")
    }

    fn extract_file_path(&self, req: &str) -> Option<PathBuf> {
        let file_pattern =
            Regex::new(r"(?i)(?:create|add|file)\s+(?:`?)([\w./-]+\.\w+)(?:`?)").ok()?;
        file_pattern
            .captures(req)
            .and_then(|caps| caps.get(1))
            .map(|m| PathBuf::from(m.as_str()))
    }
}

fn deduplicate<T: Clone + Eq>(items: Vec<T>) -> Vec<T> {
    let mut seen = vec![];
    for item in items {
        if !seen.contains(&item) {
            seen.push(item);
        }
    }
    seen
}

pub fn compile_acceptance_criteria(reqs: &[String]) -> Vec<AcceptanceCriterion> {
    let compiler = AcceptanceCompiler::new();
    let env = EnvironmentProfile::default();
    let compiled = compiler.compile(reqs, &env);
    compiled.criteria
}

pub fn compile_acceptance_criteria_with_env(
    reqs: &[String],
    env: &EnvironmentProfile,
) -> CompiledAcceptanceCriteria {
    let compiler = AcceptanceCompiler::new();
    compiler.compile(reqs, env)
}

pub fn get_verification_summary(criteria: &[AcceptanceCriterion]) -> String {
    let total = criteria.len();
    let passed = criteria
        .iter()
        .filter(|c| matches!(c.status, CriterionStatus::Passed))
        .count();
    let failed = criteria
        .iter()
        .filter(|c| matches!(c.status, CriterionStatus::Failed))
        .count();
    let pending = criteria
        .iter()
        .filter(|c| matches!(c.status, CriterionStatus::Pending))
        .count();
    let critical = criteria
        .iter()
        .filter(|c| c.priority == CriterionPriority::Critical)
        .count();

    let auto_verifiable = criteria
        .iter()
        .filter(|c| {
            !matches!(
                c.verification_method,
                VerificationMethod::Review | VerificationMethod::Manual
            )
        })
        .count();

    format!(
        "{} criteria: {} passed, {} failed, {} pending ({} critical, {} auto-verifiable)",
        total, passed, failed, pending, critical, auto_verifiable
    )
}

pub fn update_criterion_status(
    criteria: &mut [AcceptanceCriterion],
    criterion_id: &str,
    status: CriterionStatus,
) -> bool {
    if let Some(criterion) = criteria.iter_mut().find(|c| c.id == criterion_id) {
        criterion.status = status;
        true
    } else {
        false
    }
}
