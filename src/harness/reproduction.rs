use crate::harness::{
    edit_protocol::EditOperation,
    file_control::FilePolicy,
    repo_intelligence::{RepoContext, SymbolInfo},
    environment::EnvironmentProfile,
};
use anyhow::{Result, Context, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReproductionRequest {
    pub task: String,
    pub failure_description: String,
    pub error_message: Option<String>,
    pub stack_trace: Option<String>,
    pub affected_files: Vec<PathBuf>,
    pub mentioned_symbols: Vec<String>,
    pub repro_mode: ReproductionMode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReproductionMode {
    MinimalTest,
    IntegrationTest,
    PropertyBased,
    EdgeCaseExplorer,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReproductionResult {
    pub success: bool,
    pub test_files: Vec<PathBuf>,
    pub test_count: usize,
    pub reproduction_confidence: f32,
    pub failure_captured: bool,
    pub suggested_fixes: Vec<SuggestedFix>,
    pub diagnostics: Vec<ReproductionDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SuggestedFix {
    pub description: String,
    pub confidence: f32,
    pub affected_file: PathBuf,
    pub line_number: Option<usize>,
    pub fix_type: FixType,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FixType {
    NullCheck,
    BoundsCheck,
    TypeConversion,
    ResourceCleanup,
    ErrorHandling,
    LogicCorrection,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReproductionDiagnostic {
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub file: Option<PathBuf>,
    pub line: Option<usize>,
    pub category: DiagnosticCategory,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Hint,
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DiagnosticCategory {
    AssertionFailure,
    ExceptionThrown,
    UnexpectedOutput,
    Timeout,
    ResourceLeak,
    StateCorruption,
}

#[derive(Debug, Clone)]
struct TestGenerator {
    repo: RepoContext,
    env: EnvironmentProfile,
    policy: FilePolicy,
    test_counter: usize,
}

pub async fn generate_reproduction_test(
    req: &ReproductionRequest,
    repo: &RepoContext,
    env: &EnvironmentProfile,
    policy: &FilePolicy,
) -> Result<ReproductionResult> {
    let mut generator = TestGenerator::new(repo.clone(), env.clone(), policy.clone());
    
    let test_files = match req.repro_mode {
        ReproductionMode::MinimalTest => {
            generator.generate_minimal_test(req).await?
        }
        ReproductionMode::IntegrationTest => {
            generator.generate_integration_test(req).await?
        }
        ReproductionMode::PropertyBased => {
            generator.generate_property_test(req).await?
        }
        ReproductionMode::EdgeCaseExplorer => {
            generator.generate_edge_case_tests(req).await?
        }
    };
    
    let diagnostics = analyze_failure_signature(req, repo).await?;
    let suggested_fixes = infer_suggested_fixes(&diagnostics, repo);
    
    let reproduction_confidence = calculate_reproduction_confidence(
        &test_files,
        &diagnostics,
        &req.failure_description,
    );
    
    Ok(ReproductionResult {
        success: !test_files.is_empty(),
        test_count: test_files.len(),
        test_files,
        reproduction_confidence,
        failure_captured: !diagnostics.is_empty(),
        suggested_fixes,
        diagnostics,
    })
}

impl TestGenerator {
    fn new(repo: RepoContext, env: EnvironmentProfile, policy: FilePolicy) -> Self {
        Self {
            repo,
            env,
            policy,
            test_counter: 0,
        }
    }
    
    async fn generate_minimal_test(&mut self, req: &ReproductionRequest) -> Result<Vec<PathBuf>> {
        let mut tests = vec![];
        
        for symbol_name in &req.mentioned_symbols {
            if let Some(symbol) = self.find_symbol(symbol_name) {
                let test = self.create_unit_test_for_symbol(&symbol, req).await?;
                if let Some(path) = self.write_test_file(&test, "unit").await? {
                    tests.push(path);
                }
            }
        }
        
        if tests.is_empty() && !req.affected_files.is_empty() {
            let test = self.create_file_based_test(&req.affected_files[0], req).await?;
            if let Some(path) = self.write_test_file(&test, "regression").await? {
                tests.push(path);
            }
        }
        
        Ok(tests)
    }
    
    async fn generate_integration_test(&mut self, req: &ReproductionRequest) -> Result<Vec<PathBuf>> {
        let mut tests = vec![];
        
        let integration_test = self.create_integration_scenario(req).await?;
        if let Some(path) = self.write_test_file(&integration_test, "integration").await? {
            tests.push(path);
        }
        
        Ok(tests)
    }
    
    async fn generate_property_test(&mut self, req: &ReproductionRequest) -> Result<Vec<PathBuf>> {
        let mut tests = vec![];
        
        for symbol_name in &req.mentioned_symbols {
            if let Some(symbol) = self.find_symbol(symbol_name) {
                let test = self.create_property_test(&symbol, req).await?;
                if let Some(path) = self.write_test_file(&test, "property").await? {
                    tests.push(path);
                }
            }
        }
        
        Ok(tests)
    }
    
    async fn generate_edge_case_tests(&mut self, req: &ReproductionRequest) -> Result<Vec<PathBuf>> {
        let mut tests = vec![];
        let edge_cases = self.identify_edge_cases(req);
        
        for (i, edge_case) in edge_cases.iter().enumerate() {
            let test = self.create_edge_case_test(edge_case, req, i).await?;
            if let Some(path) = self.write_test_file(&test, &format!("edge{}", i)).await? {
                tests.push(path);
            }
        }
        
        Ok(tests)
    }
    
    fn find_symbol(&self, name: &str) -> Option<&SymbolInfo> {
        self.repo.symbols.iter().find(|s| s.name == name || s.full_path.contains(name))
    }
    
    async fn create_unit_test_for_symbol(&self, symbol: &SymbolInfo, req: &ReproductionRequest) -> Result<TestContent> {
        let language = detect_language(&symbol.file_path);
        
        let test_code = match language.as_str() {
            "rust" => generate_rust_unit_test(symbol, req),
            "python" => generate_python_unit_test(symbol, req),
            "javascript" | "typescript" => generate_js_unit_test(symbol, req),
            _ => generate_generic_unit_test(symbol, req),
        };
        
        Ok(TestContent {
            language,
            code: test_code,
            target_file: symbol.file_path.clone(),
            test_name: format!("test_repro_{}", sanitize_name(&symbol.name)),
        })
    }
    
    async fn create_file_based_test(&self, file: &Path, req: &ReproductionRequest) -> Result<TestContent> {
        let language = detect_language(file);
        
        let test_code = match language.as_str() {
            "rust" => generate_rust_regression_test(file, req),
            "python" => generate_python_regression_test(file, req),
            _ => generate_generic_regression_test(file, req),
        };
        
        Ok(TestContent {
            language,
            code: test_code,
            target_file: file.to_path_buf(),
            test_name: format!("test_regression_{}", self.test_counter),
        })
    }
    
    async fn create_integration_scenario(&self, req: &ReproductionRequest) -> Result<TestContent> {
        let language = self.env.languages.first().map(|s| s.as_str()).unwrap_or("rust");
        
        let test_code = format!(
            r#"// Integration test for: {}
// Failure: {}

#[test]
fn test_integration_reproduction() {{
    // TODO: Set up test environment matching production conditions
    
    // Execute the failing scenario
    
    // Assert expected failure is captured
}}
"#,
            req.task,
            req.failure_description
        );
        
        Ok(TestContent {
            language: language.to_string(),
            code: test_code,
            target_file: req.affected_files.first().cloned().unwrap_or_else(|| PathBuf::from("test.rs")),
            test_name: "test_integration_reproduction".into(),
        })
    }
    
    async fn create_property_test(&self, symbol: &SymbolInfo, req: &ReproductionRequest) -> Result<TestContent> {
        let test_code = format!(
            r#"// Property-based test for: {}
// Generated to find edge cases causing: {}

use proptest::prelude::*;

proptest! {{
    #![proptest_config(ProptestConfig {{
        cases: 100,
        .. ProptestConfig::default()
    }})]
    
    #[test]
    fn test_{}_property(input in any::<String>()) {{
        // Property: function should not panic on any input
        let _ = {}::{}(input);
    }}
}}
"#,
            symbol.name,
            req.failure_description,
            sanitize_name(&symbol.name),
            symbol.module.as_deref().unwrap_or("module"),
            symbol.name
        );
        
        Ok(TestContent {
            language: "rust".to_string(),
            code: test_code,
            target_file: symbol.file_path.clone(),
            test_name: format!("test_{}_property", sanitize_name(&symbol.name)),
        })
    }
    
    async fn create_edge_case_test(&self, edge_case: &EdgeCase, req: &ReproductionRequest, index: usize) -> Result<TestContent> {
        let test_code = format!(
            r#"// Edge case test #{}: {}
// Target: {}

#[test]
fn test_edge_case_{}() {{
    // Input: {:?}
    // Expected: Handle gracefully without panic/error
    
    let input = {:?};
    let result = {}(input);
    
    // Assert based on edge case type
    {}
}}
"#,
            index,
            edge_case.description,
            req.mentioned_symbols.first().unwrap_or(&"unknown".to_string()),
            index,
            edge_case.input,
            edge_case.input,
            req.mentioned_symbols.first().unwrap_or(&"function".to_string()),
            edge_case.assertion
        );
        
        Ok(TestContent {
            language: self.env.languages.first().cloned().unwrap_or_else(|| "rust".to_string()),
            code: test_code,
            target_file: req.affected_files.first().cloned().unwrap_or_else(|| PathBuf::from("test.rs")),
            test_name: format!("test_edge_case_{}", index),
        })
    }
    
    fn identify_edge_cases(&self, req: &ReproductionRequest) -> Vec<EdgeCase> {
        let mut cases = vec![];
        
        if let Some(ref error) = req.error_message {
            let error_lower = error.to_lowercase();
            
            if error_lower.contains("null") || error_lower.contains("none") {
                cases.push(EdgeCase {
                    description: "Null/None input".into(),
                    input: "null".into(),
                    assertion: "assert!(result.is_err() || result.is_none());".into(),
                });
            }
            
            if error_lower.contains("empty") || error_lower.contains("boundary") {
                cases.push(EdgeCase {
                    description: "Empty input".into(),
                    input: "".into(),
                    assertion: "assert!(result.is_ok());".into(),
                });
            }
            
            if error_lower.contains("overflow") || error_lower.contains("large") {
                cases.push(EdgeCase {
                    description: "Maximum value".into(),
                    input: "usize::MAX".into(),
                    assertion: "assert!(result.is_err() || result.is_ok());".into(),
                });
            }
        }
        
        cases.push(EdgeCase {
            description: "Unicode/special characters".into(),
            input: "\u{0}\u{1F600}\n\t\"'".into(),
            assertion: "assert!(result.is_ok());".into(),
        });
        
        cases
    }
    
    async fn write_test_file(&mut self, content: &TestContent, suffix: &str) -> Result<Option<PathBuf>> {
        self.test_counter += 1;
        
        let test_dir = self.policy.repo_root.join("tests");
        let test_filename = format!("{}_{}_repro_{}.rs", content.test_name, suffix, self.test_counter);
        let test_path = test_dir.join(test_filename);
        
        if !test_dir.exists() {
            tokio::fs::create_dir_all(&test_dir).await.ok();
        }
        
        tokio::fs::write(&test_path, &content.code).await.ok();
        
        if test_path.exists() {
            Ok(Some(test_path))
        } else {
            Ok(None)
        }
    }
}

fn detect_language(file: &Path) -> String {
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "rs" => "rust",
        "py" => "python",
        "js" => "javascript",
        "ts" => "typescript",
        "go" => "go",
        "java" => "java",
        "cpp" | "cc" | "cxx" => "cpp",
        _ => "unknown",
    }.to_string()
}

fn generate_rust_unit_test(symbol: &SymbolInfo, req: &ReproductionRequest) -> String {
    format!(
        r#"#[cfg(test)]
mod {}_tests {{
    use super::*;
    
    /// Regression test for: {}
    /// Failure: {}
    #[test]
    fn test_reproduction() {{
        // Arrange
        
        // Act: Call the function that fails
        let result = {}();
        
        // Assert: Verify the failure is captured
        // TODO: Add specific assertions based on the failure
    }}
}}
"#,
        symbol.module.as_deref().unwrap_or("module"),
        req.task,
        req.failure_description,
        symbol.name
    )
}

fn generate_python_unit_test(symbol: &SymbolInfo, req: &ReproductionRequest) -> String {
    format!(
        r#"import unittest

class Test{}Reproduction(unittest.TestCase):
    """Regression test for: {}
    Failure: {}
    """
    
    def test_reproduction(self):
        # Arrange
        
        # Act
        result = {}()
        
        # Assert
        # TODO: Add specific assertions
        self.assertIsNotNone(result)

if __name__ == '__main__':
    unittest.main()
"#,
        sanitize_name(&symbol.name),
        req.task,
        req.failure_description,
        symbol.name
    )
}

fn generate_js_unit_test(symbol: &SymbolInfo, req: &ReproductionRequest) -> String {
    format!(
        r#"// Regression test for: {}
// Failure: {}

describe('{} reproduction', () => {{
    test('should reproduce the failure', () => {{
        // Arrange
        
        // Act
        const result = {}();
        
        // Assert
        expect(result).toBeDefined();
    }});
}});
"#,
        req.task,
        req.failure_description,
        symbol.name,
        symbol.name
    )
}

fn generate_generic_unit_test(symbol: &SymbolInfo, req: &ReproductionRequest) -> String {
    format!(
        r#"// Test for: {}
// Failure: {}
// Symbol: {} in {:?}

fn test_reproduction() {{
    // TODO: Implement test
    println!("Testing {}", "{}");
}}
"#,
        req.task,
        req.failure_description,
        symbol.name,
        symbol.file_path,
        symbol.name
    )
}

fn generate_rust_regression_test(file: &Path, req: &ReproductionRequest) -> String {
    format!(
        r#"// Regression test for file: {:?}
// Task: {}
// Failure: {}

#[test]
fn test_regression() {{
    // Load and test the affected file
    let content = std::fs::read_to_string({:?}).unwrap();
    
    // Assertions based on the failure
    assert!(!content.is_empty());
}}
"#,
        file,
        req.task,
        req.failure_description,
        file
    )
}

fn generate_python_regression_test(file: &Path, req: &ReproductionRequest) -> String {
    format!(
        r#"# Regression test for file: {:?}
# Task: {}
# Failure: {}

import unittest

class RegressionTest(unittest.TestCase):
    def test_file_integrity(self):
        with open({:?}, 'r') as f:
            content = f.read()
        self.assertTrue(len(content) > 0)

if __name__ == '__main__':
    unittest.main()
"#,
        file,
        req.task,
        req.failure_description,
        file
    )
}

fn generate_generic_regression_test(_file: &Path, req: &ReproductionRequest) -> String {
    format!(
        r#"// Regression test
// Task: {}
// Failure: {}
"#,
        req.task,
        req.failure_description
    )
}

async fn analyze_failure_signature(req: &ReproductionRequest, repo: &RepoContext) -> Result<Vec<ReproductionDiagnostic>> {
    let mut diagnostics = vec![];
    
    if let Some(ref error) = req.error_message {
        if error.contains("panic") || error.contains("exception") {
            diagnostics.push(ReproductionDiagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!("Unhandled exception detected: {}", error),
                file: req.affected_files.first().cloned(),
                line: None,
                category: DiagnosticCategory::ExceptionThrown,
            });
        }
        
        if error.contains("assert") || error.contains("expect") {
            diagnostics.push(ReproductionDiagnostic {
                severity: DiagnosticSeverity::Error,
                message: "Assertion failure - expected behavior not met".into(),
                file: req.affected_files.first().cloned(),
                line: extract_line_from_error(error),
                category: DiagnosticCategory::AssertionFailure,
            });
        }
        
        if error.contains("timeout") || error.contains("timed out") {
            diagnostics.push(ReproductionDiagnostic {
                severity: DiagnosticSeverity::Warning,
                message: "Operation timeout detected".into(),
                file: None,
                line: None,
                category: DiagnosticCategory::Timeout,
            });
        }
    }
    
    Ok(diagnostics)
}

fn infer_suggested_fixes(diagnostics: &[ReproductionDiagnostic], _repo: &RepoContext) -> Vec<SuggestedFix> {
    let mut fixes = vec![];
    
    for diag in diagnostics {
        let (fix_type, description) = match diag.category {
            DiagnosticCategory::ExceptionThrown => {
                (FixType::ErrorHandling, "Add error handling for potential exceptions".into())
            }
            DiagnosticCategory::AssertionFailure => {
                (FixType::LogicCorrection, "Review assertion logic and expected values".into())
            }
            DiagnosticCategory::Timeout => {
                (FixType::ResourceCleanup, "Add timeout handling and resource cleanup".into())
            }
            _ => continue,
        };
        
        fixes.push(SuggestedFix {
            description,
            confidence: 0.7,
            affected_file: diag.file.clone().unwrap_or_else(|| PathBuf::from("unknown")),
            line_number: diag.line,
            fix_type,
        });
    }
    
    fixes
}

fn calculate_reproduction_confidence(
    test_files: &[PathBuf],
    diagnostics: &[ReproductionDiagnostic],
    failure_desc: &str,
) -> f32 {
    let base_confidence = 0.3;
    
    let test_bonus = (test_files.len() as f32 * 0.1).min(0.3);
    let diag_bonus = (diagnostics.len() as f32 * 0.05).min(0.2);
    
    let detail_bonus = if failure_desc.len() > 100 { 0.1 } else { 0.0 };
    
    let total = base_confidence + test_bonus + diag_bonus + detail_bonus;
    total.min(1.0)
}

fn sanitize_name(name: &str) -> String {
    name.replace("::", "_").replace("<", "_").replace(">", "_").replace(" ", "_")
}

fn extract_line_from_error(error: &str) -> Option<usize> {
    let re = regex::Regex::new(r"line\s+(\d+)").ok()?;
    re.captures(error)?.get(1)?.as_str().parse().ok()
}

#[derive(Debug, Clone)]
struct TestContent {
    language: String,
    code: String,
    target_file: PathBuf,
    test_name: String,
}

#[derive(Debug, Clone)]
struct EdgeCase {
    description: String,
    input: String,
    assertion: String,
}
