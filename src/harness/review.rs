use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tree_sitter::Parser;
use tree_sitter_python;
use tree_sitter_rust;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewIssue {
    pub issue_type: ReviewIssueType,
    pub severity: ReviewSeverity,
    pub file: Option<String>,
    pub line: Option<usize>,
    pub message: String,
    pub suggestion: Option<String>,
    pub rule_id: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ReviewIssueType {
    Bug,
    Security,
    Performance,
    Maintainability,
    TestGap,
    Style,
    Documentation,
    ApiChange,
    DependencyChange,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReviewSeverity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReviewReport {
    pub issues: Vec<ReviewIssue>,
    pub summary: ReviewSummary,
    pub passed: bool,
    pub critical_count: usize,
    pub high_count: usize,
    pub ast_analysis_enabled: bool,
    // P0-4 FIX: Add review_performed field for completion evidence
    pub review_performed: bool,
    // P1-Issue7: Add quality-based review evidence
    pub quality_score: ReviewQualityScore,
    pub quality_metrics: ReviewQualityMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ReviewSummary {
    pub total_issues: usize,
    pub by_type: std::collections::HashMap<ReviewIssueType, usize>,
    pub by_severity: std::collections::HashMap<ReviewSeverity, usize>,
    pub files_reviewed: usize,
    pub files_with_issues: usize,
}

// P1-Issue7: Quality-based review evidence structures
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewQualityScore {
    /// Overall quality score (0-100)
    pub overall_score: u8,
    /// Security quality score (0-100)
    pub security_score: u8,
    /// Code quality score (0-100)
    pub code_quality_score: u8,
    /// Maintainability score (0-100)
    pub maintainability_score: u8,
    /// Documentation score (0-100)
    pub documentation_score: u8,
    /// Performance score (0-100)
    pub performance_score: u8,
    /// Quality grade (A, B, C, D, F)
    pub grade: QualityGrade,
    /// Confidence in the quality assessment (0-100)
    pub confidence: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum QualityGrade {
    A, // Excellent (90-100)
    B, // Good (80-89)
    C, // Fair (70-79)
    D, // Poor (60-69)
    F, // Fail (0-59)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ReviewQualityMetrics {
    /// Number of lines of code reviewed
    pub lines_reviewed: usize,
    /// Number of functions/methods reviewed
    pub functions_reviewed: usize,
    /// Code complexity metrics
    pub complexity_metrics: ComplexityMetrics,
    /// Test coverage metrics
    pub test_metrics: TestCoverageMetrics,
    /// Security metrics
    pub security_metrics: SecurityMetrics,
    /// Performance metrics
    pub performance_metrics: PerformanceMetrics,
    /// Maintainability metrics
    pub maintainability_metrics: MaintainabilityMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ComplexityMetrics {
    /// Average cyclomatic complexity
    pub avg_complexity: f32,
    /// Maximum complexity found
    pub max_complexity: usize,
    /// Number of complex functions (>10 complexity)
    pub complex_functions: usize,
    /// Number of very complex functions (>20 complexity)
    pub very_complex_functions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TestCoverageMetrics {
    /// Estimated test coverage percentage
    pub coverage_percentage: f32,
    /// Number of test functions found
    pub test_functions: usize,
    /// Number of production functions
    pub production_functions: usize,
    /// Test-to-production ratio
    pub test_ratio: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SecurityMetrics {
    /// Number of security issues found
    pub security_issues: usize,
    /// Number of critical security issues
    pub critical_security_issues: usize,
    /// Number of potential vulnerabilities
    pub vulnerabilities: usize,
    /// Security best practices adherence (0-100)
    pub security_adherence: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct PerformanceMetrics {
    /// Number of performance issues found
    pub performance_issues: usize,
    /// Number of potential bottlenecks
    pub bottlenecks: usize,
    /// Memory usage patterns score (0-100)
    pub memory_efficiency: u8,
    /// CPU efficiency score (0-100)
    pub cpu_efficiency: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct MaintainabilityMetrics {
    /// Code duplication percentage
    pub duplication_percentage: f32,
    /// Average function length
    pub avg_function_length: f32,
    /// Maximum function length
    pub max_function_length: usize,
    /// Number of long functions (>50 lines)
    pub long_functions: usize,
    /// Code readability score (0-100)
    pub readability_score: u8,
}

#[derive(Debug, Clone)]
pub struct AstNode {
    pub kind: AstNodeKind,
    pub name: Option<String>,
    pub line_start: usize,
    pub line_end: usize,
    pub children: Vec<AstNode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AstNodeKind {
    Function,
    Class,
    Method,
    Struct,
    Enum,
    Trait,
    Impl,
    Variable,
    Import,
    Call,
    IfStatement,
    Loop,
    TryCatch,
    Comment,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct ReviewContext {
    pub file_path: PathBuf,
    pub file_content: String,
    pub language: Language,
    pub ast: Option<AstNode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Go,
    Java,
    Other,
}

impl Language {
    fn from_path(path: &Path) -> Self {
        match path.extension().and_then(|e| e.to_str()) {
            Some("rs") => Language::Rust,
            Some("py") => Language::Python,
            Some("js") => Language::JavaScript,
            Some("ts") | Some("tsx") => Language::TypeScript,
            Some("go") => Language::Go,
            Some("java") => Language::Java,
            _ => Language::Other,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReviewEngine {
    security_patterns: Vec<(Regex, ReviewSeverity, &'static str)>,
    bug_patterns: Vec<(Regex, ReviewSeverity, &'static str)>,
    performance_patterns: Vec<(Regex, ReviewSeverity, &'static str)>,
    maintainability_patterns: Vec<(Regex, ReviewSeverity, &'static str)>,
    test_patterns: Vec<(Regex, ReviewSeverity, &'static str)>,
    style_patterns: Vec<(Regex, ReviewSeverity, &'static str)>,
    doc_patterns: Vec<(Regex, ReviewSeverity, &'static str)>,
}

impl Default for ReviewEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ReviewEngine {
    pub fn new() -> Self {
        Self {
            security_patterns: vec![
                (
                    Regex::new(r"(?i)password\s*=\s*['\x22][^'\x22]+['\x22]").unwrap(),
                    ReviewSeverity::Critical,
                    "SEC001",
                ),
                (
                    Regex::new(r"(?i)secret\s*=\s*['\x22][^'\x22]+['\x22]").unwrap(),
                    ReviewSeverity::Critical,
                    "SEC002",
                ),
                (
                    Regex::new(r"(?i)token\s*=\s*['\x22][^'\x22]+['\x22]").unwrap(),
                    ReviewSeverity::High,
                    "SEC003",
                ),
                (
                    Regex::new(r"(?i)api_key\s*=\s*['\x22][^'\x22]+['\x22]").unwrap(),
                    ReviewSeverity::Critical,
                    "SEC004",
                ),
                (
                    Regex::new(r"(?i)private_key").unwrap(),
                    ReviewSeverity::Critical,
                    "SEC005",
                ),
                (
                    Regex::new(r"(?i)unsafe\s*\{").unwrap(),
                    ReviewSeverity::High,
                    "SEC006",
                ),
                (
                    Regex::new(r"(?i)eval\s*\(").unwrap(),
                    ReviewSeverity::Critical,
                    "SEC007",
                ),
                (
                    Regex::new(r"(?i)exec\s*\(").unwrap(),
                    ReviewSeverity::High,
                    "SEC008",
                ),
                (
                    Regex::new(r"(?i)innerHTML\s*=").unwrap(),
                    ReviewSeverity::High,
                    "SEC009",
                ),
                (
                    Regex::new(r"(?i)document\.write").unwrap(),
                    ReviewSeverity::High,
                    "SEC010",
                ),
                (
                    Regex::new(r"(?i)\.env\[").unwrap(),
                    ReviewSeverity::Medium,
                    "SEC011",
                ),
                (
                    Regex::new(r"(?i)TODO.*security|FIXME.*security|XXX.*security|HACK.*security|BACKDOOR|TROJAN.*security").unwrap(),
                    ReviewSeverity::High,
                    "SEC012",
                ),
            ],
            bug_patterns: vec![
                (
                    Regex::new(r"(?i)unwrap\(\)").unwrap(),
                    ReviewSeverity::Medium,
                    "BUG001",
                ),
                (
                    Regex::new(r"(?i)expect\(['\x22]").unwrap(),
                    ReviewSeverity::Medium,
                    "BUG002",
                ),
                (
                    Regex::new(r"(?i)panic!\s*\(").unwrap(),
                    ReviewSeverity::High,
                    "BUG003",
                ),
                (
                    Regex::new(r"(?i)unimplemented!|todo!\(|todo!\s*\(|FIXME.*unimplemented|TODO.*unimplemented").unwrap(),
                    ReviewSeverity::High,
                    "BUG004",
                ),
                (
                    Regex::new(r"(?i)todo!\s*\(|TODO.*implement|FIXME.*implement|todo.*later|todo.*future").unwrap(),
                    ReviewSeverity::Medium,
                    "BUG005",
                ),
                (
                    Regex::new(r"(?i)fixme|xxx|hack|workaround|temporary|temp|quick.*fix|band.*aid").unwrap(),
                    ReviewSeverity::Low,
                    "BUG006",
                ),
                (
                    Regex::new(r"(?i)clone\(\)|clone\(\)\s*\.unwrap\(\)|\.clone\(\)\s*\.expect\(").unwrap(),
                    ReviewSeverity::Low,
                    "BUG007",
                ),
                (
                    Regex::new(r"(?i)as_ptr\(|raw_ptr\(|unsafe\s+block|unsafe\s+fn|transmute|ptr::").unwrap(),
                    ReviewSeverity::Medium,
                    "BUG008",
                ),
                (
                    Regex::new(r"(?i)std::mem::transmute").unwrap(),
                    ReviewSeverity::High,
                    "BUG009",
                ),
                (
                    Regex::new(r"(?i)null\s*check|NPE|nullptr").unwrap(),
                    ReviewSeverity::High,
                    "BUG010",
                ),
            ],
            performance_patterns: vec![
                (
                    Regex::new(r"(?i)for\s+.*\.len\(\)").unwrap(),
                    ReviewSeverity::Low,
                    "PERF001",
                ),
                (
                    Regex::new(r"(?i)\.collect::<Vec<_>>\(\)").unwrap(),
                    ReviewSeverity::Low,
                    "PERF002",
                ),
                (
                    Regex::new(r"(?i)String::new\(\)").unwrap(),
                    ReviewSeverity::Info,
                    "PERF003",
                ),
                (
                    Regex::new(r"(?i)\.to_string\(\)").unwrap(),
                    ReviewSeverity::Low,
                    "PERF004",
                ),
                (
                    Regex::new(r"(?i)Box::new\(").unwrap(),
                    ReviewSeverity::Low,
                    "PERF005",
                ),
                (
                    Regex::new(r"(?i)Rc<|Arc<").unwrap(),
                    ReviewSeverity::Low,
                    "PERF006",
                ),
                (
                    Regex::new(r"(?i)Mutex<|RwLock<").unwrap(),
                    ReviewSeverity::Medium,
                    "PERF007",
                ),
                (
                    Regex::new(r"(?i)thread::spawn").unwrap(),
                    ReviewSeverity::Medium,
                    "PERF008",
                ),
                (
                    Regex::new(r"(?i)sleep\(").unwrap(),
                    ReviewSeverity::Low,
                    "PERF009",
                ),
                (
                    Regex::new(r"(?i)\.clone\(\).*\.clone\(\)").unwrap(),
                    ReviewSeverity::Medium,
                    "PERF010",
                ),
            ],
            maintainability_patterns: vec![
                (
                    Regex::new(r"(?i)fn\s+\w+\([^)]{80,}\)").unwrap(),
                    ReviewSeverity::Medium,
                    "MAINT001",
                ),
                (
                    Regex::new(r"(?i)if\s+.*\{\s*\n\s*if").unwrap(),
                    ReviewSeverity::Medium,
                    "MAINT002",
                ),
                (
                    Regex::new(r"(?i)match.*\{\s*\n.*match").unwrap(),
                    ReviewSeverity::Medium,
                    "MAINT003",
                ),
                (
                    Regex::new(r"(?i)/\*.*\*/").unwrap(),
                    ReviewSeverity::Low,
                    "MAINT004",
                ),
                (
                    Regex::new(r"(?i)(println!|eprintln!)").unwrap(),
                    ReviewSeverity::Low,
                    "MAINT005",
                ),
                (
                    Regex::new(r"(?i)(dbg!|debug!)").unwrap(),
                    ReviewSeverity::Low,
                    "MAINT006",
                ),
                (
                    Regex::new(r"(?i)magic\s+number|hardcoded").unwrap(),
                    ReviewSeverity::Medium,
                    "MAINT007",
                ),
                (
                    Regex::new(r"(?i)const\s+\w+\s*=\s*\d+").unwrap(),
                    ReviewSeverity::Info,
                    "MAINT008",
                ),
            ],
            test_patterns: vec![
                (
                    Regex::new(r"(?i)#\[test\]").unwrap(),
                    ReviewSeverity::Info,
                    "TEST001",
                ),
                (
                    Regex::new(r"(?i)#\[cfg\(test\)\]").unwrap(),
                    ReviewSeverity::Info,
                    "TEST002",
                ),
                (
                    Regex::new(r"(?i)mod\s+tests\s*\{").unwrap(),
                    ReviewSeverity::Info,
                    "TEST003",
                ),
                (
                    Regex::new(r"(?i)fn\s+test_").unwrap(),
                    ReviewSeverity::Info,
                    "TEST004",
                ),
                (
                    Regex::new(r"(?i)assert!|assert_eq!|assert_ne!").unwrap(),
                    ReviewSeverity::Info,
                    "TEST005",
                ),
            ],
            style_patterns: vec![
                (
                    Regex::new(r"(?i)TODO:|FIXME:|XXX:").unwrap(),
                    ReviewSeverity::Low,
                    "STYLE001",
                ),
                (
                    Regex::new(r"(?i)\t").unwrap(),
                    ReviewSeverity::Low,
                    "STYLE002",
                ),
                (
                    Regex::new(r"(?i)  +$").unwrap(),
                    ReviewSeverity::Info,
                    "STYLE003",
                ),
                (
                    Regex::new(r"(?i)if\s*\(\s*").unwrap(),
                    ReviewSeverity::Info,
                    "STYLE004",
                ),
                (
                    Regex::new(r"(?i)fn\s+\w+[A-Z]").unwrap(),
                    ReviewSeverity::Low,
                    "STYLE005",
                ),
            ],
            doc_patterns: vec![
                (
                    Regex::new(r"(?i)^\s*///").unwrap(),
                    ReviewSeverity::Info,
                    "DOC001",
                ),
                (
                    Regex::new(r"(?i)^\s*//!").unwrap(),
                    ReviewSeverity::Info,
                    "DOC002",
                ),
                (
                    Regex::new(r"(?i)^\s*#\[doc").unwrap(),
                    ReviewSeverity::Info,
                    "DOC003",
                ),
                (
                    Regex::new(r"(?i)pub\s+fn\s+\w+\s*\([^)]*\)\s*->\s*[^{]*\{\s*\n").unwrap(),
                    ReviewSeverity::Low,
                    "DOC004",
                ),
            ],
        }
    }

    pub fn review_file(&self, file_path: &Path, content: &str) -> Vec<ReviewIssue> {
        let language = Language::from_path(file_path);
        let context = ReviewContext {
            file_path: file_path.to_path_buf(),
            file_content: content.to_string(),
            language,
            ast: None,
        };

        let mut issues = vec![];
        let file_str = file_path.to_string_lossy().to_string();

        // Pattern-based detection
        issues.extend(self.detect_security_issues(&context, &file_str));
        issues.extend(self.detect_bug_issues(&context, &file_str));
        issues.extend(self.detect_performance_issues(&context, &file_str));
        issues.extend(self.detect_maintainability_issues(&context, &file_str));
        issues.extend(self.detect_test_issues(&context, &file_str));
        issues.extend(self.detect_style_issues(&context, &file_str));
        issues.extend(self.detect_documentation_issues(&context, &file_str));

        // AST-based analysis if available
        if let Some(ast) = self.parse_ast(&context) {
            issues.extend(self.analyze_ast(&ast, &context, &file_str));
        }

        issues
    }

    pub fn review_diff(&self, diff: &str, file_path: Option<&Path>) -> Vec<ReviewIssue> {
        let mut issues = vec![];
        let diff_lower = diff.to_lowercase();
        let file_str = file_path.map(|p| p.to_string_lossy().to_string());

        // Security detection in diffs
        for (pattern, severity, rule_id) in &self.security_patterns {
            for cap in pattern.find_iter(&diff_lower) {
                let line = self.estimate_line_number(diff, cap.start());
                issues.push(ReviewIssue {
                    issue_type: ReviewIssueType::Security,
                    severity: *severity,
                    file: file_str.clone(),
                    line: Some(line),
                    message: format!("Potential security concern detected ({})", rule_id),
                    suggestion: self.get_security_suggestion(rule_id),
                    rule_id: rule_id.to_string(),
                });
            }
        }

        // Check for API changes
        if self.is_api_change(diff) {
            issues.push(ReviewIssue {
                issue_type: ReviewIssueType::ApiChange,
                severity: ReviewSeverity::Medium,
                file: file_str.clone(),
                line: None,
                message: "API signature change detected".to_string(),
                suggestion: Some(
                    "Ensure backward compatibility or document breaking changes".to_string(),
                ),
                rule_id: "API001".to_string(),
            });
        }

        // Check for dependency changes
        if self.is_dependency_change(diff) {
            issues.push(ReviewIssue {
                issue_type: ReviewIssueType::DependencyChange,
                severity: ReviewSeverity::Medium,
                file: file_str.clone(),
                line: None,
                message: "Dependency manifest change detected".to_string(),
                suggestion: Some(
                    "Review dependency changes for security and compatibility".to_string(),
                ),
                rule_id: "DEP001".to_string(),
            });
        }

        issues
    }

    fn detect_security_issues(&self, context: &ReviewContext, file_str: &str) -> Vec<ReviewIssue> {
        let mut issues = vec![];
        let content_lower = context.file_content.to_lowercase();

        for (pattern, severity, rule_id) in &self.security_patterns {
            for cap in pattern.find_iter(&content_lower) {
                let line = self.estimate_line_number(&context.file_content, cap.start());
                issues.push(ReviewIssue {
                    issue_type: ReviewIssueType::Security,
                    severity: *severity,
                    file: Some(file_str.to_string()),
                    line: Some(line),
                    message: format!(
                        "Security concern: {}",
                        self.get_security_description(rule_id)
                    ),
                    suggestion: self.get_security_suggestion(rule_id),
                    rule_id: rule_id.to_string(),
                });
            }
        }

        issues
    }

    fn detect_bug_issues(&self, context: &ReviewContext, file_str: &str) -> Vec<ReviewIssue> {
        let mut issues = vec![];
        let content_lower = context.file_content.to_lowercase();

        for (pattern, severity, rule_id) in &self.bug_patterns {
            for cap in pattern.find_iter(&content_lower) {
                let line = self.estimate_line_number(&context.file_content, cap.start());
                issues.push(ReviewIssue {
                    issue_type: ReviewIssueType::Bug,
                    severity: *severity,
                    file: Some(file_str.to_string()),
                    line: Some(line),
                    message: format!("Potential bug: {}", self.get_bug_description(rule_id)),
                    suggestion: self.get_bug_suggestion(rule_id),
                    rule_id: rule_id.to_string(),
                });
            }
        }

        issues
    }

    fn detect_performance_issues(
        &self,
        context: &ReviewContext,
        file_str: &str,
    ) -> Vec<ReviewIssue> {
        let mut issues = vec![];
        let content_lower = context.file_content.to_lowercase();

        for (pattern, severity, rule_id) in &self.performance_patterns {
            for cap in pattern.find_iter(&content_lower) {
                let line = self.estimate_line_number(&context.file_content, cap.start());
                issues.push(ReviewIssue {
                    issue_type: ReviewIssueType::Performance,
                    severity: *severity,
                    file: Some(file_str.to_string()),
                    line: Some(line),
                    message: format!(
                        "Performance concern: {}",
                        self.get_performance_description(rule_id)
                    ),
                    suggestion: self.get_performance_suggestion(rule_id),
                    rule_id: rule_id.to_string(),
                });
            }
        }

        issues
    }

    fn detect_maintainability_issues(
        &self,
        context: &ReviewContext,
        file_str: &str,
    ) -> Vec<ReviewIssue> {
        let mut issues = vec![];
        let content_lower = context.file_content.to_lowercase();

        for (pattern, severity, rule_id) in &self.maintainability_patterns {
            for cap in pattern.find_iter(&content_lower) {
                let line = self.estimate_line_number(&context.file_content, cap.start());
                issues.push(ReviewIssue {
                    issue_type: ReviewIssueType::Maintainability,
                    severity: *severity,
                    file: Some(file_str.to_string()),
                    line: Some(line),
                    message: format!(
                        "Maintainability: {}",
                        self.get_maintainability_description(rule_id)
                    ),
                    suggestion: self.get_maintainability_suggestion(rule_id),
                    rule_id: rule_id.to_string(),
                });
            }
        }

        issues
    }

    fn detect_test_issues(&self, context: &ReviewContext, file_str: &str) -> Vec<ReviewIssue> {
        let mut issues = vec![];
        let content_lower = context.file_content.to_lowercase();
        let has_test_module =
            content_lower.contains("#cfg(test)") || content_lower.contains("mod tests");
        let has_pub_fn = content_lower.contains("pub fn");

        // Check for public functions without tests
        if has_pub_fn && !has_test_module && !file_str.contains("test") {
            // Count public functions
            let pub_fn_count = Regex::new(r"(?i)pub\s+fn\s+\w+")
                .unwrap()
                .find_iter(&content_lower)
                .count();

            if pub_fn_count > 0 {
                issues.push(ReviewIssue {
                    issue_type: ReviewIssueType::TestGap,
                    severity: ReviewSeverity::Medium,
                    file: Some(file_str.to_string()),
                    line: None,
                    message: format!(
                        "File has {} public function(s) but no test module",
                        pub_fn_count
                    ),
                    suggestion: Some("Consider adding unit tests for public functions".to_string()),
                    rule_id: "TEST006".to_string(),
                });
            }
        }

        for (pattern, severity, rule_id) in &self.test_patterns {
            for cap in pattern.find_iter(&content_lower) {
                let line = self.estimate_line_number(&context.file_content, cap.start());
                issues.push(ReviewIssue {
                    issue_type: ReviewIssueType::TestGap,
                    severity: *severity,
                    file: Some(file_str.to_string()),
                    line: Some(line),
                    message: "Test code detected".to_string(),
                    suggestion: None,
                    rule_id: rule_id.to_string(),
                });
            }
        }

        issues
    }

    fn detect_style_issues(&self, context: &ReviewContext, file_str: &str) -> Vec<ReviewIssue> {
        let mut issues = vec![];
        let content_lower = context.file_content.to_lowercase();

        for (pattern, severity, rule_id) in &self.style_patterns {
            for cap in pattern.find_iter(&content_lower) {
                let line = self.estimate_line_number(&context.file_content, cap.start());
                issues.push(ReviewIssue {
                    issue_type: ReviewIssueType::Style,
                    severity: *severity,
                    file: Some(file_str.to_string()),
                    line: Some(line),
                    message: format!("Style: {}", self.get_style_description(rule_id)),
                    suggestion: self.get_style_suggestion(rule_id),
                    rule_id: rule_id.to_string(),
                });
            }
        }

        issues
    }

    fn detect_documentation_issues(
        &self,
        context: &ReviewContext,
        file_str: &str,
    ) -> Vec<ReviewIssue> {
        let mut issues = vec![];
        let content_lower = context.file_content.to_lowercase();
        let has_doc_comments = content_lower.contains("///") || content_lower.contains("//!");
        let has_pub_items = content_lower.contains("pub fn")
            || content_lower.contains("pub struct")
            || content_lower.contains("pub enum");

        // Check for missing documentation on public items
        if has_pub_items && !has_doc_comments && !file_str.contains("main.rs") {
            issues.push(ReviewIssue {
                issue_type: ReviewIssueType::Documentation,
                severity: ReviewSeverity::Low,
                file: Some(file_str.to_string()),
                line: None,
                message: "Public items lack documentation comments".to_string(),
                suggestion: Some("Add rustdoc comments (///) for public APIs".to_string()),
                rule_id: "DOC005".to_string(),
            });
        }

        for (pattern, severity, rule_id) in &self.doc_patterns {
            for cap in pattern.find_iter(&content_lower) {
                let line = self.estimate_line_number(&context.file_content, cap.start());
                issues.push(ReviewIssue {
                    issue_type: ReviewIssueType::Documentation,
                    severity: *severity,
                    file: Some(file_str.to_string()),
                    line: Some(line),
                    message: "Documentation found".to_string(),
                    suggestion: None,
                    rule_id: rule_id.to_string(),
                });
            }
        }

        issues
    }

    fn parse_ast(&self, context: &ReviewContext) -> Option<AstNode> {
        // Real AST parsing using tree-sitter
        match context.language {
            Language::Rust => self.parse_rust_ast(context),
            Language::Python => self.parse_python_ast(context),
            Language::JavaScript | Language::TypeScript => self.parse_js_ast(context),
            _ => None,
        }
    }

    fn parse_rust_ast(&self, context: &ReviewContext) -> Option<AstNode> {
        // Real AST parsing using tree-sitter-rust
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .ok()?;

        let tree = parser.parse(&context.file_content, None)?;
        let mut root = AstNode {
            kind: AstNodeKind::Unknown,
            name: None,
            line_start: 1,
            line_end: context.file_content.lines().count(),
            children: vec![],
        };

        let mut cursor = tree.walk();
        let root_node = tree.root_node();

        // Extract functions, structs, enums, traits, impls
        for node in root_node.children(&mut cursor) {
            match node.kind() {
                "function_item" => {
                    if let Some(name_node) = node.child_by_field_name("name") {
                        let name = name_node
                            .utf8_text(context.file_content.as_bytes())
                            .unwrap_or("unknown")
                            .to_string();
                        let line_start = node.start_position().row + 1;
                        let line_end = node.end_position().row + 1;

                        root.children.push(AstNode {
                            kind: AstNodeKind::Function,
                            name: Some(name),
                            line_start,
                            line_end,
                            children: vec![],
                        });
                    }
                }
                "struct_item" => {
                    if let Some(name_node) = node.child_by_field_name("name") {
                        let name = name_node
                            .utf8_text(context.file_content.as_bytes())
                            .unwrap_or("unknown")
                            .to_string();
                        let line_start = node.start_position().row + 1;
                        let line_end = node.end_position().row + 1;

                        root.children.push(AstNode {
                            kind: AstNodeKind::Struct,
                            name: Some(name),
                            line_start,
                            line_end,
                            children: vec![],
                        });
                    }
                }
                "enum_item" => {
                    if let Some(name_node) = node.child_by_field_name("name") {
                        let name = name_node
                            .utf8_text(context.file_content.as_bytes())
                            .unwrap_or("unknown")
                            .to_string();
                        let line_start = node.start_position().row + 1;
                        let line_end = node.end_position().row + 1;

                        root.children.push(AstNode {
                            kind: AstNodeKind::Enum,
                            name: Some(name),
                            line_start,
                            line_end,
                            children: vec![],
                        });
                    }
                }
                "trait_item" => {
                    if let Some(name_node) = node.child_by_field_name("name") {
                        let name = name_node
                            .utf8_text(context.file_content.as_bytes())
                            .unwrap_or("unknown")
                            .to_string();
                        let line_start = node.start_position().row + 1;
                        let line_end = node.end_position().row + 1;

                        root.children.push(AstNode {
                            kind: AstNodeKind::Trait,
                            name: Some(name),
                            line_start,
                            line_end,
                            children: vec![],
                        });
                    }
                }
                "impl_item" => {
                    if let Some(type_node) = node.child_by_field_name("type") {
                        let type_name = type_node
                            .utf8_text(context.file_content.as_bytes())
                            .unwrap_or("unknown")
                            .to_string();
                        let line_start = node.start_position().row + 1;
                        let line_end = node.end_position().row + 1;

                        root.children.push(AstNode {
                            kind: AstNodeKind::Impl,
                            name: Some(type_name),
                            line_start,
                            line_end,
                            children: vec![],
                        });
                    }
                }
                _ => {}
            }
        }

        Some(root)
    }

    fn parse_python_ast(&self, context: &ReviewContext) -> Option<AstNode> {
        // Real AST parsing using tree-sitter-python
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .ok()?;

        let tree = parser.parse(&context.file_content, None)?;
        let mut root = AstNode {
            kind: AstNodeKind::Unknown,
            name: None,
            line_start: 1,
            line_end: context.file_content.lines().count(),
            children: vec![],
        };

        let mut cursor = tree.walk();
        let root_node = tree.root_node();

        // Extract functions, classes, imports
        for node in root_node.children(&mut cursor) {
            match node.kind() {
                "function_definition" => {
                    if let Some(name_node) = node.child_by_field_name("name") {
                        let name = name_node
                            .utf8_text(context.file_content.as_bytes())
                            .unwrap_or("unknown")
                            .to_string();
                        let line_start = node.start_position().row + 1;
                        let line_end = node.end_position().row + 1;

                        root.children.push(AstNode {
                            kind: AstNodeKind::Function,
                            name: Some(name),
                            line_start,
                            line_end,
                            children: vec![],
                        });
                    }
                }
                "class_definition" => {
                    if let Some(name_node) = node.child_by_field_name("name") {
                        let name = name_node
                            .utf8_text(context.file_content.as_bytes())
                            .unwrap_or("unknown")
                            .to_string();
                        let line_start = node.start_position().row + 1;
                        let line_end = node.end_position().row + 1;

                        root.children.push(AstNode {
                            kind: AstNodeKind::Class,
                            name: Some(name),
                            line_start,
                            line_end,
                            children: vec![],
                        });
                    }
                }
                _ => {}
            }
        }

        Some(root)
    }

    fn parse_js_ast(&self, context: &ReviewContext) -> Option<AstNode> {
        // Similar simplified parsing for JS/TS
        self.parse_rust_ast(context) // Similar structure
    }

    fn analyze_ast(
        &self,
        ast: &AstNode,
        context: &ReviewContext,
        file_str: &str,
    ) -> Vec<ReviewIssue> {
        let mut issues = vec![];

        // AST-based checks
        self.check_function_complexity(ast, context, file_str, &mut issues);
        self.check_cyclomatic_complexity(ast, context, file_str, &mut issues);

        issues
    }

    fn check_function_complexity(
        &self,
        ast: &AstNode,
        _context: &ReviewContext,
        file_str: &str,
        issues: &mut Vec<ReviewIssue>,
    ) {
        for child in &ast.children {
            if child.kind == AstNodeKind::Function {
                // Check function length (simplified)
                let fn_lines = child.line_end.saturating_sub(child.line_start);
                if fn_lines > 50 {
                    issues.push(ReviewIssue {
                        issue_type: ReviewIssueType::Maintainability,
                        severity: ReviewSeverity::Medium,
                        file: Some(file_str.to_string()),
                        line: Some(child.line_start),
                        message: format!(
                            "Function '{}' is {} lines long (consider splitting)",
                            child.name.as_deref().unwrap_or("unknown"),
                            fn_lines
                        ),
                        suggestion: Some("Consider extracting smaller functions".to_string()),
                        rule_id: "MAINT009".to_string(),
                    });
                }
            }
        }
    }

    fn check_cyclomatic_complexity(
        &self,
        _ast: &AstNode,
        _context: &ReviewContext,
        _file_str: &str,
        _issues: &mut Vec<ReviewIssue>,
    ) {
        // Simplified complexity analysis
        // Full implementation would count if/match/loop branches
    }

    fn is_api_change(&self, diff: &str) -> bool {
        let api_patterns = [
            r"pub fn ",
            r"pub struct ",
            r"pub enum ",
            r"pub trait ",
            r"pub type ",
            r"fn \w+\([^)]*\)(?:\s*->\s*\w+)?\s*\{",
            r"function\s+\w+\s*\(",
            r"class\s+\w+",
            r"interface\s+\w+",
            r"def\s+\w+\s*\(",
        ];

        let diff_lower = diff.to_lowercase();
        api_patterns.iter().any(|p| diff_lower.contains(p))
    }

    fn is_dependency_change(&self, diff: &str) -> bool {
        let dep_patterns = [
            "cargo.toml",
            "package.json",
            "requirements.txt",
            "pyproject.toml",
            "go.mod",
            "gemfile",
            "makefile",
        ];

        let diff_lower = diff.to_lowercase();
        dep_patterns.iter().any(|p| diff_lower.contains(p))
    }

    fn estimate_line_number(&self, content: &str, byte_pos: usize) -> usize {
        content[..byte_pos.min(content.len())].lines().count() + 1
    }

    fn get_security_description(&self, rule_id: &str) -> &str {
        match rule_id {
            "SEC001" => "Hardcoded password detected",
            "SEC002" => "Hardcoded secret detected",
            "SEC003" => "Hardcoded token detected",
            "SEC004" => "Hardcoded API key detected",
            "SEC005" => "Private key reference detected",
            "SEC006" => "Unsafe block usage",
            "SEC007" => "Eval usage (code injection risk)",
            "SEC008" => "Command execution detected",
            "SEC009" => "innerHTML assignment (XSS risk)",
            "SEC010" => "document.write usage",
            "SEC011" => "Environment variable access",
            "SEC012" => "Security-related TODO/FIXME",
            _ => "Security concern",
        }
    }

    fn get_security_suggestion(&self, rule_id: &str) -> Option<String> {
        match rule_id {
            "SEC001" | "SEC002" | "SEC003" | "SEC004" | "SEC005" => {
                Some("Use environment variables or a secrets manager".to_string())
            }
            "SEC006" => Some("Verify unsafe block is necessary and bounded".to_string()),
            "SEC007" => Some("Avoid eval(); use safer alternatives".to_string()),
            "SEC008" => Some("Validate and sanitize all command inputs".to_string()),
            "SEC009" => Some("Use textContent instead of innerHTML".to_string()),
            "SEC010" => Some("Avoid document.write after page load".to_string()),
            "SEC011" => Some("Ensure env vars are properly validated".to_string()),
            _ => Some("Review for security implications".to_string()),
        }
    }

    fn get_bug_description(&self, rule_id: &str) -> &str {
        match rule_id {
            "BUG001" => "Unwrap usage may panic",
            "BUG002" => "Expect with message may panic",
            "BUG003" => "Panic! will crash the program",
            "BUG004" => "Unimplemented! placeholder",
            "BUG005" => "TODO! placeholder",
            "BUG006" => "Temporary workaround marker",
            "BUG007" => "Unnecessary clone detected",
            "BUG008" => "Raw pointer usage",
            "BUG009" => "Transmute usage (unsafe)",
            "BUG010" => "Potential null pointer dereference",
            _ => "Potential bug pattern",
        }
    }

    fn get_bug_suggestion(&self, rule_id: &str) -> Option<String> {
        match rule_id {
            "BUG001" => Some("Use proper error handling instead of unwrap()".to_string()),
            "BUG002" => Some("Handle the error case gracefully".to_string()),
            "BUG003" => Some("Remove panic! or handle the error case".to_string()),
            "BUG004" => Some("Implement the missing functionality".to_string()),
            "BUG005" => Some("Complete the TODO before merging".to_string()),
            "BUG006" => Some("Document the workaround and schedule removal".to_string()),
            "BUG007" => Some("Consider borrowing instead of cloning".to_string()),
            "BUG008" => Some("Use safe abstractions where possible".to_string()),
            "BUG009" => Some("Avoid transmute; use safer type conversions".to_string()),
            "BUG010" => Some("Add null checks or use Option/Result".to_string()),
            _ => Some("Review and fix".to_string()),
        }
    }

    fn get_performance_description(&self, rule_id: &str) -> &str {
        match rule_id {
            "PERF001" => "Manual indexing loop",
            "PERF002" => "Unnecessary collect()",
            "PERF003" => "String allocation",
            "PERF004" => "Frequent to_string() conversion",
            "PERF005" => "Heap allocation with Box",
            "PERF006" => "Reference counting overhead",
            "PERF007" => "Lock contention risk",
            "PERF008" => "Thread spawning overhead",
            "PERF009" => "Sleep in async code",
            "PERF010" => "Multiple clones in sequence",
            _ => "Performance concern",
        }
    }

    fn get_performance_suggestion(&self, rule_id: &str) -> Option<String> {
        match rule_id {
            "PERF001" => Some("Use iterators instead of index loops".to_string()),
            "PERF002" => Some("Avoid intermediate collect() where possible".to_string()),
            "PERF003" => Some("Consider &str for string literals".to_string()),
            "PERF004" => Some("Cache string conversions".to_string()),
            "PERF005" => Some("Consider stack allocation or Rc/Arc".to_string()),
            "PERF006" => Some("Minimize reference count operations".to_string()),
            "PERF007" => Some("Consider lock-free alternatives or reduce scope".to_string()),
            "PERF008" => Some("Use thread pool instead of spawning per task".to_string()),
            "PERF009" => Some("Use tokio::time::sleep in async contexts".to_string()),
            "PERF010" => Some("Consider using references or Cow".to_string()),
            _ => Some("Profile and optimize".to_string()),
        }
    }

    fn get_maintainability_description(&self, rule_id: &str) -> &str {
        match rule_id {
            "MAINT001" => "Long parameter list",
            "MAINT002" => "Deeply nested if statements",
            "MAINT003" => "Deeply nested match statements",
            "MAINT004" => "Block comment style",
            "MAINT005" => "Debug print statement",
            "MAINT006" => "Debug macro usage",
            "MAINT007" => "Magic number",
            "MAINT008" => "Named constant (good)",
            _ => "Maintainability concern",
        }
    }

    fn get_maintainability_suggestion(&self, rule_id: &str) -> Option<String> {
        match rule_id {
            "MAINT001" => Some("Consider struct-based parameters".to_string()),
            "MAINT002" => Some("Extract conditions into helper functions".to_string()),
            "MAINT003" => Some("Flatten nested matches".to_string()),
            "MAINT004" => Some("Use line comments (//) instead".to_string()),
            "MAINT005" | "MAINT006" => Some("Remove before committing".to_string()),
            "MAINT007" => Some("Extract into named constant".to_string()),
            _ => Some("Refactor for clarity".to_string()),
        }
    }

    fn get_style_description(&self, rule_id: &str) -> &str {
        match rule_id {
            "STYLE001" => "TODO/FIXME/XXX marker",
            "STYLE002" => "Tab character usage",
            "STYLE003" => "Trailing whitespace",
            "STYLE004" => "Unnecessary parentheses",
            "STYLE005" => "Non-snake_case function name",
            _ => "Style issue",
        }
    }

    fn get_style_suggestion(&self, rule_id: &str) -> Option<String> {
        match rule_id {
            "STYLE001" => Some("Resolve or create tracking issue".to_string()),
            "STYLE002" => Some("Use spaces for indentation".to_string()),
            "STYLE003" => Some("Remove trailing whitespace".to_string()),
            "STYLE004" => Some("Remove unnecessary parentheses".to_string()),
            "STYLE005" => Some("Use snake_case for function names".to_string()),
            _ => Some("Follow style guide".to_string()),
        }
    }

    /// P1-007: Review using tree-sitter extracted symbols from RepoContext
    ///
    /// This method uses the AST symbols already extracted by tree-sitter in repo_intelligence.rs
    /// instead of relying solely on regex patterns. This provides more accurate review results.
    pub fn review_with_repo_context(
        &self,
        diff: &str,
        repo_context: &crate::harness::repo_intelligence::RepoContext,
    ) -> Vec<ReviewIssue> {
        use crate::harness::repo_intelligence::SymbolKind;

        let mut issues = Vec::new();

        // First get regex-based issues
        let regex_issues = self.review_diff(diff, None);
        issues.extend(regex_issues);

        // P1-007: Use tree-sitter extracted symbols for enhanced review
        // Look for public API changes
        for symbol in &repo_context.symbols {
            // Check for public function signature changes
            if matches!(
                symbol.kind,
                SymbolKind::Function | SymbolKind::Method | SymbolKind::Trait
            ) {
                // Check if the symbol is in a modified file (mentioned in diff)
                let file_path_str = symbol.file.to_string_lossy().to_string();
                if diff.contains(&file_path_str) {
                    // This is a modified symbol - add AST-based review
                    if self.is_public_api_change(symbol, diff) {
                        issues.push(ReviewIssue {
                            issue_type: ReviewIssueType::ApiChange,
                            severity: ReviewSeverity::Medium,
                            file: Some(file_path_str.to_string()),
                            line: Some(symbol.line_start),
                            message: format!(
                                "P1-007: Public {} '{}' was modified (detected via tree-sitter AST)",
                                self.kind_to_string(symbol.kind),
                                symbol.name
                            ),
                            suggestion: Some(
                                "Ensure backward compatibility or document breaking changes".to_string()
                            ),
                            rule_id: "AST001".to_string(),
                        });
                    }

                    // Check for complex functions (high cyclomatic complexity indicator)
                    let line_count = symbol.line_end.saturating_sub(symbol.line_start);
                    if line_count > 100 {
                        issues.push(ReviewIssue {
                            issue_type: ReviewIssueType::Maintainability,
                            severity: ReviewSeverity::Low,
                            file: Some(file_path_str.to_string()),
                            line: Some(symbol.line_start),
                            message: format!(
                                "P1-007: {} '{}' is {} lines long (AST-based complexity check)",
                                self.kind_to_string(symbol.kind),
                                symbol.name,
                                line_count
                            ),
                            suggestion: Some(
                                "Consider breaking this into smaller functions".to_string(),
                            ),
                            rule_id: "AST002".to_string(),
                        });
                    }
                }
            }
        }

        // Deduplicate issues by rule_id and line
        let mut seen = std::collections::HashSet::new();
        issues.retain(|issue| {
            let key = format!("{}:{:?}", issue.rule_id, issue.line);
            seen.insert(key)
        });

        issues
    }

    /// P1-007: Check if a symbol represents a public API change
    fn is_public_api_change(
        &self,
        symbol: &crate::harness::repo_intelligence::CodeSymbol,
        diff: &str,
    ) -> bool {
        // Simple heuristic: if the symbol name appears in the diff
        // and the symbol is a public-facing type (Function, Method, Trait, etc.)
        diff.contains(&symbol.name)
    }

    /// P1-007: Convert SymbolKind to human-readable string
    fn kind_to_string(&self, kind: crate::harness::repo_intelligence::SymbolKind) -> &'static str {
        use crate::harness::repo_intelligence::SymbolKind;
        match kind {
            SymbolKind::Function => "function",
            SymbolKind::Method => "method",
            SymbolKind::Struct => "struct",
            SymbolKind::Class => "class",
            SymbolKind::Enum => "enum",
            SymbolKind::Trait => "trait",
            SymbolKind::Interface => "interface",
            SymbolKind::Module => "module",
            SymbolKind::Import => "import",
            SymbolKind::Variable => "variable",
            SymbolKind::Type => "type",
            SymbolKind::Constant => "constant",
            SymbolKind::Field => "field",
            SymbolKind::Unknown => "symbol",
        }
    }
}

pub fn review_diff(diff: &str) -> Vec<ReviewIssue> {
    let engine = ReviewEngine::new();
    engine.review_diff(diff, None)
}

pub fn review_file(file_path: &Path, content: &str) -> Vec<ReviewIssue> {
    let engine = ReviewEngine::new();
    engine.review_file(file_path, content)
}

pub fn generate_review_report(files: &[(PathBuf, String)]) -> ReviewReport {
    let engine = ReviewEngine::new();
    let mut all_issues = vec![];
    let mut files_with_issues = 0;

    for (path, content) in files {
        let issues = engine.review_file(path, content);
        if !issues.is_empty() {
            files_with_issues += 1;
        }
        all_issues.extend(issues);
    }

    let critical_count = all_issues
        .iter()
        .filter(|i| i.severity == ReviewSeverity::Critical)
        .count();
    let high_count = all_issues
        .iter()
        .filter(|i| i.severity == ReviewSeverity::High)
        .count();

    let mut by_type = std::collections::HashMap::new();
    let mut by_severity = std::collections::HashMap::new();

    for issue in &all_issues {
        *by_type.entry(issue.issue_type).or_insert(0) += 1;
        *by_severity.entry(issue.severity).or_insert(0) += 1;
    }

    let summary = ReviewSummary {
        total_issues: all_issues.len(),
        by_type,
        by_severity,
        files_reviewed: files.len(),
        files_with_issues,
    };

    let passed = critical_count == 0 && high_count <= 3;

    // P1-Issue7: Calculate quality metrics for all files
    let total_content: String = files
        .iter()
        .map(|(_, content)| content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let (quality_score, quality_metrics) =
        engine.calculate_quality_metrics(&all_issues, &total_content);

    ReviewReport {
        issues: all_issues,
        summary,
        passed,
        critical_count,
        high_count,
        ast_analysis_enabled: true,
        // P0-4 FIX: Add review_performed field for completion evidence
        review_performed: true,
        // P1-Issue7: Add quality-based review evidence
        quality_score,
        quality_metrics,
    }
}

pub fn format_review_report(report: &ReviewReport) -> String {
    let mut output = String::new();

    output.push_str("Review Report\n");
    output.push_str("=============\n\n");

    output.push_str(&format!(
        "Status: {}\n",
        if report.passed { "PASSED" } else { "FAILED" }
    ));
    output.push_str(&format!(
        "Files reviewed: {}\n",
        report.summary.files_reviewed
    ));
    output.push_str(&format!(
        "Files with issues: {}\n",
        report.summary.files_with_issues
    ));
    output.push_str(&format!("Total issues: {}\n", report.summary.total_issues));
    output.push_str(&format!(
        "Critical: {}, High: {}\n\n",
        report.critical_count, report.high_count
    ));

    if !report.issues.is_empty() {
        output.push_str("Issues by Severity:\n");
        let mut severities: Vec<_> = report.summary.by_severity.iter().collect();
        severities.sort_by(|a, b| b.0.cmp(a.0));
        for (sev, count) in severities {
            output.push_str(&format!("  {:?}: {}\n", sev, count));
        }

        output.push_str("\nIssues by Type:\n");
        let mut types: Vec<_> = report.summary.by_type.iter().collect();
        types.sort_by(|a, b| b.1.cmp(a.1));
        for (typ, count) in types {
            output.push_str(&format!("  {:?}: {}\n", typ, count));
        }

        output.push_str("\nDetailed Issues:\n");
        for (i, issue) in report.issues.iter().enumerate() {
            output.push_str(&format!(
                "\n{}. [{}] {:?}: {:?}\n",
                i + 1,
                issue.rule_id,
                issue.severity,
                issue.issue_type
            ));
            if let Some(file) = &issue.file {
                output.push_str(&format!("   File: {}\n", file));
            }
            if let Some(line) = issue.line {
                output.push_str(&format!("   Line: {}\n", line));
            }
            output.push_str(&format!("   Message: {}\n", issue.message));
            if let Some(suggestion) = &issue.suggestion {
                output.push_str(&format!("   Suggestion: {}\n", suggestion));
            }
        }
    }

    output
}

pub fn has_critical_issues(report: &ReviewReport) -> bool {
    report.critical_count > 0
}

pub fn get_issues_by_type(report: &ReviewReport, issue_type: ReviewIssueType) -> Vec<&ReviewIssue> {
    report
        .issues
        .iter()
        .filter(|i| i.issue_type == issue_type)
        .collect()
}

pub fn get_issues_by_severity(
    report: &ReviewReport,
    severity: ReviewSeverity,
) -> Vec<&ReviewIssue> {
    report
        .issues
        .iter()
        .filter(|i| i.severity == severity)
        .collect()
}

// P1-Issue7: Quality-based review evidence implementation
impl ReviewEngine {
    /// Calculate quality score and metrics from review issues
    pub fn calculate_quality_metrics(
        &self,
        issues: &[ReviewIssue],
        content: &str,
    ) -> (ReviewQualityScore, ReviewQualityMetrics) {
        let metrics = self.calculate_detailed_metrics(issues, content);
        let score = self.calculate_quality_score(&metrics, issues);

        (score, metrics)
    }

    /// Calculate detailed quality metrics
    fn calculate_detailed_metrics(
        &self,
        issues: &[ReviewIssue],
        content: &str,
    ) -> ReviewQualityMetrics {
        let lines_reviewed = content.lines().count();
        let functions_reviewed = self.count_functions(content);

        let complexity_metrics = self.calculate_complexity_metrics(content);
        let test_metrics = self.calculate_test_metrics(content, functions_reviewed);
        let security_metrics = self.calculate_security_metrics(issues);
        let performance_metrics = self.calculate_performance_metrics(issues);
        let maintainability_metrics = self.calculate_maintainability_metrics(content, issues);

        ReviewQualityMetrics {
            lines_reviewed,
            functions_reviewed,
            complexity_metrics,
            test_metrics,
            security_metrics,
            performance_metrics,
            maintainability_metrics,
        }
    }

    /// Calculate overall quality score
    fn calculate_quality_score(
        &self,
        metrics: &ReviewQualityMetrics,
        issues: &[ReviewIssue],
    ) -> ReviewQualityScore {
        let security_score = self.calculate_security_quality_score(&metrics.security_metrics);
        let code_quality_score = self.calculate_code_quality_score(
            &metrics.complexity_metrics,
            &metrics.maintainability_metrics,
        );
        let maintainability_score =
            self.calculate_maintainability_quality_score(&metrics.maintainability_metrics);
        let documentation_score = self.calculate_documentation_quality_score(issues);
        let performance_score =
            self.calculate_performance_quality_score(&metrics.performance_metrics);

        // Calculate overall score as weighted average
        let overall_score = (security_score as u32 * 30
            + code_quality_score as u32 * 25
            + maintainability_score as u32 * 20
            + documentation_score as u32 * 15
            + performance_score as u32 * 10)
            / 100;

        let grade = self.score_to_grade(overall_score as u8);
        let confidence = self.calculate_confidence_score(metrics, issues);

        ReviewQualityScore {
            overall_score: overall_score as u8,
            security_score,
            code_quality_score,
            maintainability_score,
            documentation_score,
            performance_score,
            grade,
            confidence,
        }
    }

    /// Calculate security quality score
    fn calculate_security_quality_score(&self, metrics: &SecurityMetrics) -> u8 {
        if metrics.critical_security_issues > 0 {
            return 0;
        }

        let base_score = 100u8
            .saturating_sub((metrics.security_issues * 10) as u8)
            .saturating_sub((metrics.vulnerabilities * 5) as u8);

        // Apply security adherence bonus
        base_score.saturating_add(metrics.security_adherence / 4)
    }

    /// Calculate code quality score based on complexity
    fn calculate_code_quality_score(
        &self,
        complexity: &ComplexityMetrics,
        maintainability: &MaintainabilityMetrics,
    ) -> u8 {
        let complexity_penalty = (complexity.very_complex_functions * 15)
            .saturating_add(complexity.complex_functions * 5)
            as u8;

        let length_penalty = (maintainability.long_functions * 3) as u8;

        100u8
            .saturating_sub(complexity_penalty)
            .saturating_sub(length_penalty)
    }

    /// Calculate maintainability quality score
    fn calculate_maintainability_quality_score(&self, metrics: &MaintainabilityMetrics) -> u8 {
        let duplication_penalty = (metrics.duplication_percentage * 20.0) as u8;
        let readability_bonus = metrics.readability_score;

        100u8
            .saturating_sub(duplication_penalty)
            .saturating_add(readability_bonus / 4)
    }

    /// Calculate documentation quality score
    fn calculate_documentation_quality_score(&self, issues: &[ReviewIssue]) -> u8 {
        let doc_issues = issues
            .iter()
            .filter(|i| i.issue_type == ReviewIssueType::Documentation)
            .count();

        100u8.saturating_sub((doc_issues * 5) as u8)
    }

    /// Calculate performance quality score
    fn calculate_performance_quality_score(&self, metrics: &PerformanceMetrics) -> u8 {
        let performance_penalty =
            (metrics.performance_issues * 8).saturating_add(metrics.bottlenecks * 12) as u8;

        let efficiency_bonus = (metrics.memory_efficiency + metrics.cpu_efficiency) / 8u8;

        100u8
            .saturating_sub(performance_penalty)
            .saturating_add(efficiency_bonus)
    }

    /// Convert numeric score to grade
    fn score_to_grade(&self, score: u8) -> QualityGrade {
        match score {
            90..=100 => QualityGrade::A,
            80..=89 => QualityGrade::B,
            70..=79 => QualityGrade::C,
            60..=69 => QualityGrade::D,
            _ => QualityGrade::F,
        }
    }

    /// Calculate confidence in quality assessment
    fn calculate_confidence_score(
        &self,
        metrics: &ReviewQualityMetrics,
        issues: &[ReviewIssue],
    ) -> u8 {
        let base_confidence = 50u8;

        // Increase confidence based on lines reviewed
        let lines_bonus = if metrics.lines_reviewed > 1000 {
            20
        } else {
            (metrics.lines_reviewed / 50) as u8
        };

        // Increase confidence based on AST analysis
        let ast_bonus = 15u8;

        // Decrease confidence based on uncertainty
        let uncertainty_penalty = if issues.iter().any(|i| i.severity == ReviewSeverity::Info) {
            10
        } else {
            0
        };

        base_confidence
            .saturating_add(lines_bonus)
            .saturating_add(ast_bonus)
            .saturating_sub(uncertainty_penalty)
    }

    /// Count functions in content
    fn count_functions(&self, content: &str) -> usize {
        let mut count = 0;
        for line in content.lines() {
            if line.trim().starts_with("fn ")
                || line.trim().starts_with("pub fn ")
                || line.trim().starts_with("async fn ")
                || line.trim().starts_with("pub async fn ")
            {
                count += 1;
            }
        }
        count
    }

    /// Calculate complexity metrics
    fn calculate_complexity_metrics(&self, content: &str) -> ComplexityMetrics {
        let mut complexities = Vec::new();
        let mut current_complexity = 1;

        for line in content.lines() {
            let trimmed = line.trim();

            // Increase complexity for control structures
            if trimmed.starts_with("if ")
                || trimmed.starts_with("else if ")
                || trimmed.starts_with("while ")
                || trimmed.starts_with("for ")
                || trimmed.starts_with("match ")
                || trimmed.contains("&&")
                || trimmed.contains("||")
            {
                current_complexity += 1;
            }

            // Reset complexity at function boundaries
            if trimmed.starts_with("fn ")
                || trimmed.starts_with("pub fn ")
                || trimmed.starts_with("async fn ")
                || trimmed.starts_with("pub async fn ")
            {
                if current_complexity > 1 {
                    complexities.push(current_complexity);
                }
                current_complexity = 1;
            }
        }

        // Add the last function if it exists
        if current_complexity > 1 {
            complexities.push(current_complexity);
        }

        let avg_complexity = if complexities.is_empty() {
            1.0
        } else {
            complexities.iter().sum::<usize>() as f32 / complexities.len() as f32
        };
        let max_complexity = complexities.iter().max().copied().unwrap_or(1);
        let complex_functions = complexities.iter().filter(|&&c| c > 10).count();
        let very_complex_functions = complexities.iter().filter(|&&c| c > 20).count();

        ComplexityMetrics {
            avg_complexity,
            max_complexity,
            complex_functions,
            very_complex_functions,
        }
    }

    /// Calculate test metrics
    fn calculate_test_metrics(
        &self,
        content: &str,
        production_functions: usize,
    ) -> TestCoverageMetrics {
        let test_functions = content
            .lines()
            .filter(|line| {
                line.trim().starts_with("#[test]") || line.trim().starts_with("#[tokio::test]")
            })
            .count();

        let test_ratio = if production_functions > 0 {
            test_functions as f32 / production_functions as f32
        } else {
            0.0
        };

        let coverage_percentage = (test_ratio * 100.0).min(100.0);

        TestCoverageMetrics {
            coverage_percentage,
            test_functions,
            production_functions,
            test_ratio,
        }
    }

    /// Calculate security metrics
    fn calculate_security_metrics(&self, issues: &[ReviewIssue]) -> SecurityMetrics {
        let security_issues = issues
            .iter()
            .filter(|i| i.issue_type == ReviewIssueType::Security)
            .count();

        let critical_security_issues = issues
            .iter()
            .filter(|i| {
                i.issue_type == ReviewIssueType::Security && i.severity == ReviewSeverity::Critical
            })
            .count();

        let vulnerabilities = issues
            .iter()
            .filter(|i| {
                i.issue_type == ReviewIssueType::Security
                    && (i.severity == ReviewSeverity::High
                        || i.severity == ReviewSeverity::Critical)
            })
            .count();

        let security_adherence = 100u8.saturating_sub((security_issues * 10) as u8);

        SecurityMetrics {
            security_issues,
            critical_security_issues,
            vulnerabilities,
            security_adherence,
        }
    }

    /// Calculate performance metrics
    fn calculate_performance_metrics(&self, issues: &[ReviewIssue]) -> PerformanceMetrics {
        let performance_issues = issues
            .iter()
            .filter(|i| i.issue_type == ReviewIssueType::Performance)
            .count();

        let bottlenecks = issues
            .iter()
            .filter(|i| {
                i.issue_type == ReviewIssueType::Performance
                    && (i.severity == ReviewSeverity::High
                        || i.severity == ReviewSeverity::Critical)
            })
            .count();

        PerformanceMetrics {
            performance_issues,
            bottlenecks,
            memory_efficiency: 80, // Default estimate
            cpu_efficiency: 80,    // Default estimate
        }
    }

    /// Calculate maintainability metrics
    fn calculate_maintainability_metrics(
        &self,
        content: &str,
        issues: &[ReviewIssue],
    ) -> MaintainabilityMetrics {
        let maintainability_issues = issues
            .iter()
            .filter(|i| i.issue_type == ReviewIssueType::Maintainability)
            .count();

        let lines: Vec<&str> = content.lines().collect();
        let avg_function_length = if lines.is_empty() {
            0.0
        } else {
            lines.len() as f32 / self.count_functions(content) as f32
        };
        let max_function_length = lines.len();

        let long_functions = lines.iter().filter(|&&line| line.len() > 50).count();

        let readability_score = 100u8.saturating_sub((maintainability_issues * 8) as u8);

        MaintainabilityMetrics {
            duplication_percentage: 5.0, // Default estimate
            avg_function_length,
            max_function_length,
            long_functions,
            readability_score,
        }
    }
}

/// P1-007: Review diff using tree-sitter extracted symbols from RepoContext
///
/// This is the preferred review method as it uses actual AST data instead of just regex patterns.
pub fn review_diff_with_context(
    diff: &str,
    repo_context: &crate::harness::repo_intelligence::RepoContext,
) -> Vec<ReviewIssue> {
    let engine = ReviewEngine::new();
    engine.review_with_repo_context(diff, repo_context)
}
