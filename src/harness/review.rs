use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewReport {
    pub issues: Vec<ReviewIssue>,
    pub summary: ReviewSummary,
    pub passed: bool,
    pub critical_count: usize,
    pub high_count: usize,
    pub ast_analysis_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ReviewSummary {
    pub total_issues: usize,
    pub by_type: std::collections::HashMap<ReviewIssueType, usize>,
    pub by_severity: std::collections::HashMap<ReviewSeverity, usize>,
    pub files_reviewed: usize,
    pub files_with_issues: usize,
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
                    Regex::new(r"(?i)TODO.*security|FIXME.*security|XXX.*security").unwrap(),
                    ReviewSeverity::Medium,
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
                    Regex::new(r"(?i)unimplemented!").unwrap(),
                    ReviewSeverity::High,
                    "BUG004",
                ),
                (
                    Regex::new(r"(?i)todo!").unwrap(),
                    ReviewSeverity::Medium,
                    "BUG005",
                ),
                (
                    Regex::new(r"(?i)fixme|xxx|hack").unwrap(),
                    ReviewSeverity::Low,
                    "BUG006",
                ),
                (
                    Regex::new(r"(?i)clone\(\)").unwrap(),
                    ReviewSeverity::Low,
                    "BUG007",
                ),
                (
                    Regex::new(r"(?i)as_ptr\(\)").unwrap(),
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
        // Simplified AST parsing based on language
        // In a full implementation, this would use tree-sitter or similar
        match context.language {
            Language::Rust => self.parse_rust_ast(context),
            Language::Python => self.parse_python_ast(context),
            Language::JavaScript | Language::TypeScript => self.parse_js_ast(context),
            _ => None,
        }
    }

    fn parse_rust_ast(&self, context: &ReviewContext) -> Option<AstNode> {
        // Simplified: extract function signatures and structures
        let mut root = AstNode {
            kind: AstNodeKind::Unknown,
            name: None,
            line_start: 1,
            line_end: context.file_content.lines().count(),
            children: vec![],
        };

        let fn_pattern = Regex::new(r"(?m)^\s*(?:pub\s+)?(?:async\s+)?fn\s+(\w+)").ok()?;
        for cap in fn_pattern.captures_iter(&context.file_content) {
            if let Some(name) = cap.get(1) {
                let line = context.file_content[..cap.get(0).unwrap().start()]
                    .lines()
                    .count()
                    + 1;
                root.children.push(AstNode {
                    kind: AstNodeKind::Function,
                    name: Some(name.as_str().to_string()),
                    line_start: line,
                    line_end: line,
                    children: vec![],
                });
            }
        }

        Some(root)
    }

    fn parse_python_ast(&self, context: &ReviewContext) -> Option<AstNode> {
        let mut root = AstNode {
            kind: AstNodeKind::Unknown,
            name: None,
            line_start: 1,
            line_end: context.file_content.lines().count(),
            children: vec![],
        };

        let def_pattern = Regex::new(r"(?m)^\s*def\s+(\w+)").ok()?;
        for cap in def_pattern.captures_iter(&context.file_content) {
            if let Some(name) = cap.get(1) {
                let line = context.file_content[..cap.get(0).unwrap().start()]
                    .lines()
                    .count()
                    + 1;
                root.children.push(AstNode {
                    kind: AstNodeKind::Function,
                    name: Some(name.as_str().to_string()),
                    line_start: line,
                    line_end: line,
                    children: vec![],
                });
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

    ReviewReport {
        issues: all_issues,
        summary,
        passed,
        critical_count,
        high_count,
        ast_analysis_enabled: true,
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
