use crate::harness::validation::ValidationResult;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum VerificationStrength {
    /// No verification performed
    None = 0,
    /// Only format checking (cargo fmt, prettier, etc.)
    FormatOnly = 1,
    /// Only static analysis without execution (cargo check, mypy, tsc --noEmit)
    StaticOnly = 2,
    /// Only linting (clippy, eslint, pylint)
    LintOnly = 3,
    /// Unit tests pass
    Tests = 4,
    /// Reproduction tests confirm fix
    Reproduction = 5,
    /// Full verification including integration tests, coverage, reproduction
    Full = 6,
}

impl VerificationStrength {
    pub fn description(&self) -> &'static str {
        match self {
            VerificationStrength::None => "No verification performed",
            VerificationStrength::FormatOnly => "Code formatting verified only",
            VerificationStrength::StaticOnly => {
                "Static analysis passed (compilation/type-checking)"
            }
            VerificationStrength::LintOnly => "Linting passed (style/best practices)",
            VerificationStrength::Tests => "Unit tests passed",
            VerificationStrength::Reproduction => "Fix verified with reproduction tests",
            VerificationStrength::Full => {
                "Full verification including integration tests and coverage"
            }
        }
    }

    pub fn requirements(&self) -> Vec<&'static str> {
        match self {
            VerificationStrength::None => vec![],
            VerificationStrength::FormatOnly => vec!["format_check"],
            VerificationStrength::StaticOnly => vec!["static_check"],
            VerificationStrength::LintOnly => vec!["lint_check"],
            VerificationStrength::Tests => vec!["unit_tests"],
            VerificationStrength::Reproduction => vec!["unit_tests", "reproduction_test"],
            VerificationStrength::Full => vec![
                "format_check",
                "static_check",
                "lint_check",
                "unit_tests",
                "integration_tests",
                "coverage_check",
                "reproduction_test",
            ],
        }
    }

    pub fn is_sufficient_for(&self, min_required: VerificationStrength) -> bool {
        *self >= min_required
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerificationAssessment {
    pub strength: VerificationStrength,
    pub achieved_levels: Vec<VerificationLevel>,
    pub missing_levels: Vec<VerificationLevel>,
    pub coverage_percent: Option<f32>,
    pub test_count: usize,
    pub passed_tests: usize,
    pub failed_tests: usize,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum VerificationLevel {
    FormatCheck,
    StaticCheck,
    LintCheck,
    UnitTests,
    IntegrationTests,
    CoverageCheck,
    ReproductionTest,
}

impl VerificationLevel {
    pub fn name(&self) -> &'static str {
        match self {
            VerificationLevel::FormatCheck => "Format Check",
            VerificationLevel::StaticCheck => "Static Check",
            VerificationLevel::LintCheck => "Lint Check",
            VerificationLevel::UnitTests => "Unit Tests",
            VerificationLevel::IntegrationTests => "Integration Tests",
            VerificationLevel::CoverageCheck => "Coverage Check",
            VerificationLevel::ReproductionTest => "Reproduction Test",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            VerificationLevel::FormatCheck => "Code formatting and style compliance",
            VerificationLevel::StaticCheck => "Compilation and type checking without execution",
            VerificationLevel::LintCheck => "Static analysis for code quality and best practices",
            VerificationLevel::UnitTests => "Isolated component testing",
            VerificationLevel::IntegrationTests => "Cross-component and system testing",
            VerificationLevel::CoverageCheck => "Code coverage measurement",
            VerificationLevel::ReproductionTest => "Verification that specific issue is fixed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct VerificationAssessor {
    level_mapping: HashMap<String, VerificationLevel>,
}

impl Default for VerificationAssessor {
    fn default() -> Self {
        Self::new()
    }
}

impl VerificationAssessor {
    pub fn new() -> Self {
        let mut level_mapping = HashMap::new();

        // Format commands
        level_mapping.insert("cargo fmt".to_string(), VerificationLevel::FormatCheck);
        level_mapping.insert("rustfmt".to_string(), VerificationLevel::FormatCheck);
        level_mapping.insert("prettier".to_string(), VerificationLevel::FormatCheck);
        level_mapping.insert("black --check".to_string(), VerificationLevel::FormatCheck);

        // Static checks
        level_mapping.insert("cargo check".to_string(), VerificationLevel::StaticCheck);
        level_mapping.insert("tsc --noEmit".to_string(), VerificationLevel::StaticCheck);
        level_mapping.insert("mypy".to_string(), VerificationLevel::StaticCheck);
        level_mapping.insert("go build".to_string(), VerificationLevel::StaticCheck);

        // Lint checks
        level_mapping.insert("cargo clippy".to_string(), VerificationLevel::LintCheck);
        level_mapping.insert("eslint".to_string(), VerificationLevel::LintCheck);
        level_mapping.insert("pylint".to_string(), VerificationLevel::LintCheck);
        level_mapping.insert("golint".to_string(), VerificationLevel::LintCheck);

        // Unit tests
        level_mapping.insert("cargo test".to_string(), VerificationLevel::UnitTests);
        level_mapping.insert("pytest".to_string(), VerificationLevel::UnitTests);
        level_mapping.insert("jest".to_string(), VerificationLevel::UnitTests);
        level_mapping.insert("go test".to_string(), VerificationLevel::UnitTests);

        // Integration tests (often same command with flags)
        level_mapping.insert(
            "cargo test --test integration".to_string(),
            VerificationLevel::IntegrationTests,
        );
        level_mapping.insert(
            "pytest tests/integration".to_string(),
            VerificationLevel::IntegrationTests,
        );

        Self { level_mapping }
    }

    pub fn assess(&self, result: Option<&ValidationResult>) -> VerificationAssessment {
        let mut achieved = vec![];
        let mut missing = vec![];
        let mut test_count = 0;
        let mut passed_tests = 0;
        let mut failed_tests = 0;
        let mut coverage_percent = None;
        let mut duration_ms = 0u64;

        if let Some(r) = result {
            // Analyze each command result
            for cmd_result in &r.command_results {
                let cmd_str = cmd_result.command.to_lowercase();
                duration_ms += cmd_result.duration_ms;

                // Check if this command maps to a verification level
                for (pattern, level) in &self.level_mapping {
                    if cmd_str.contains(&pattern.to_lowercase()) && !achieved.contains(level) {
                        if cmd_result.exit_code == Some(0) {
                            achieved.push(*level);
                        }
                    }
                }

                // Try to extract test counts from output
                if let Some((passed, failed, total)) = self.parse_test_results(&cmd_result.stdout) {
                    passed_tests += passed;
                    failed_tests += failed;
                    test_count += total;
                }

                // Try to extract coverage
                if let Some(coverage) = self.parse_coverage(&cmd_result.stdout) {
                    coverage_percent = Some(coverage);
                }
            }

            // Determine missing levels
            let all_levels = vec![
                VerificationLevel::FormatCheck,
                VerificationLevel::StaticCheck,
                VerificationLevel::LintCheck,
                VerificationLevel::UnitTests,
                VerificationLevel::IntegrationTests,
                VerificationLevel::CoverageCheck,
                VerificationLevel::ReproductionTest,
            ];

            for level in all_levels {
                if !achieved.contains(&level) {
                    missing.push(level);
                }
            }
        } else {
            // No validation result - all levels missing
            missing = vec![
                VerificationLevel::FormatCheck,
                VerificationLevel::StaticCheck,
                VerificationLevel::LintCheck,
                VerificationLevel::UnitTests,
                VerificationLevel::IntegrationTests,
                VerificationLevel::CoverageCheck,
                VerificationLevel::ReproductionTest,
            ];
        }

        let strength = self.calculate_strength(&achieved, coverage_percent);

        VerificationAssessment {
            strength,
            achieved_levels: achieved,
            missing_levels: missing,
            coverage_percent,
            test_count,
            passed_tests,
            failed_tests,
            duration_ms,
        }
    }

    fn calculate_strength(
        &self,
        achieved: &[VerificationLevel],
        coverage: Option<f32>,
    ) -> VerificationStrength {
        let has_format = achieved.contains(&VerificationLevel::FormatCheck);
        let has_static = achieved.contains(&VerificationLevel::StaticCheck);
        let has_lint = achieved.contains(&VerificationLevel::LintCheck);
        let has_unit_tests = achieved.contains(&VerificationLevel::UnitTests);
        let has_integration = achieved.contains(&VerificationLevel::IntegrationTests);
        let has_coverage = coverage.map(|c| c >= 70.0).unwrap_or(false);
        let has_reproduction = achieved.contains(&VerificationLevel::ReproductionTest);

        if has_format
            && has_static
            && has_lint
            && has_unit_tests
            && has_integration
            && has_coverage
            && has_reproduction
        {
            VerificationStrength::Full
        } else if has_unit_tests && has_reproduction {
            VerificationStrength::Reproduction
        } else if has_unit_tests {
            VerificationStrength::Tests
        } else if has_lint {
            VerificationStrength::LintOnly
        } else if has_static {
            VerificationStrength::StaticOnly
        } else if has_format {
            VerificationStrength::FormatOnly
        } else {
            VerificationStrength::None
        }
    }

    fn parse_test_results(&self, output: &str) -> Option<(usize, usize, usize)> {
        // Try to parse common test output formats
        // Rust: "test result: ok. 5 passed; 0 failed;"
        // Python: "5 passed, 2 failed"
        // JavaScript: "5 passing (10ms)"

        let rust_pattern =
            regex::Regex::new(r"test result:.*?(\d+)\s*passed.*?(\d+)\s*failed").ok()?;
        if let Some(cap) = rust_pattern.captures(output) {
            let passed = cap.get(1)?.as_str().parse().ok()?;
            let failed = cap.get(2)?.as_str().parse().ok()?;
            return Some((passed, failed, passed + failed));
        }

        let pytest_pattern = regex::Regex::new(r"(\d+)\s*passed").ok()?;
        let pytest_failed = regex::Regex::new(r"(\d+)\s*failed").ok()?;
        if let Some(cap) = pytest_pattern.captures(output) {
            let passed = cap.get(1)?.as_str().parse().ok()?;
            let failed = pytest_failed
                .captures(output)
                .and_then(|c| c.get(1))
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(0);
            return Some((passed, failed, passed + failed));
        }

        None
    }

    fn parse_coverage(&self, output: &str) -> Option<f32> {
        // Try to parse coverage percentages
        // "coverage: 85.4%"
        // "Coverage: 85%"
        let pattern = regex::Regex::new(r"[Cc]overage[:\s]+(\d+\.?\d*)%").ok()?;
        pattern.captures(output)?.get(1)?.as_str().parse().ok()
    }

    pub fn recommend_improvements(&self, assessment: &VerificationAssessment) -> Vec<String> {
        let mut recommendations = vec![];

        if assessment.strength == VerificationStrength::None {
            recommendations
                .push("No verification performed - add at least format checking".to_string());
        }

        if !assessment
            .achieved_levels
            .contains(&VerificationLevel::StaticCheck)
        {
            recommendations.push("Add static type checking (cargo check, tsc, mypy)".to_string());
        }

        if !assessment
            .achieved_levels
            .contains(&VerificationLevel::LintCheck)
        {
            recommendations.push("Add linting (clippy, eslint, pylint)".to_string());
        }

        if !assessment
            .achieved_levels
            .contains(&VerificationLevel::UnitTests)
        {
            recommendations.push("Add unit tests for critical code paths".to_string());
        }

        if assessment
            .coverage_percent
            .map(|c| c < 70.0)
            .unwrap_or(true)
        {
            recommendations.push("Improve test coverage to at least 70%".to_string());
        }

        if assessment.failed_tests > 0 {
            recommendations.push(format!("Fix {} failing tests", assessment.failed_tests));
        }

        recommendations
    }
}

pub fn assess_verification_strength(result: Option<&ValidationResult>) -> VerificationStrength {
    let assessor = VerificationAssessor::new();
    assessor.assess(result).strength
}

pub fn format_verification_assessment(assessment: &VerificationAssessment) -> String {
    let mut output = String::new();

    output.push_str("Verification Assessment\n");
    output.push_str("=======================\n\n");

    output.push_str(&format!("Overall Strength: {:?}\n", assessment.strength));
    output.push_str(&format!(
        "Description: {}\n\n",
        assessment.strength.description()
    ));

    output.push_str("Achieved Levels:\n");
    for level in &assessment.achieved_levels {
        output.push_str(&format!("  ✓ {}\n", level.name()));
    }

    if !assessment.missing_levels.is_empty() {
        output.push_str("\nMissing Levels:\n");
        for level in &assessment.missing_levels {
            output.push_str(&format!("  ✗ {} - {}\n", level.name(), level.description()));
        }
    }

    output.push_str(&format!(
        "\nTests: {} passed, {} failed ({} total)\n",
        assessment.passed_tests, assessment.failed_tests, assessment.test_count
    ));

    if let Some(coverage) = assessment.coverage_percent {
        output.push_str(&format!("Coverage: {:.1}%\n", coverage));
    }

    output.push_str(&format!("Duration: {}ms\n", assessment.duration_ms));

    output
}
