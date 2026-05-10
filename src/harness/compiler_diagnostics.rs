//! Compiler Diagnostics - P0-HARNESS-010
//!
//! Provides structured compiler diagnostic parsing for intelligent repair loops.
//! Supports Rust (cargo check), TypeScript (tsc), and Python (pytest/mypy/ruff).

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// P0-HARNESS-010: Structured compiler diagnostic for intelligent repair
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompilerDiagnostic {
    pub file: PathBuf,
    pub line: usize,
    pub column: usize,
    pub code: Option<String>,
    pub message: String,
    pub suggested_replacement: Option<String>,
    pub severity: DiagnosticSeverity,
    pub category: DiagnosticCategory,
}

/// P0-HARNESS-010: Diagnostic severity levels
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
    Note,
}

/// P0-HARNESS-010: Diagnostic categories for targeted repair
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DiagnosticCategory {
    Syntax,
    Type,
    Import,
    Unused,
    Borrowing,
    Lifetime,
    Trait,
    Macro,
    Format,
    Clippy,
    Test,
    Build,
    Dependency,
}

/// P0-HARNESS-010: Language-specific diagnostic parser
pub trait DiagnosticParser {
    fn parse(&self, output: &str, working_dir: &PathBuf) -> Result<Vec<CompilerDiagnostic>>;
}

/// P0-HARNESS-010: Rust diagnostic parser (cargo check --message-format=json)
pub struct RustDiagnosticParser;

impl DiagnosticParser for RustDiagnosticParser {
    fn parse(&self, output: &str, working_dir: &PathBuf) -> Result<Vec<CompilerDiagnostic>> {
        let mut diagnostics = Vec::new();

        for line in output.lines() {
            if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(line) {
                if let Some(diag) = parse_rust_diagnostic(&json_value, working_dir) {
                    diagnostics.push(diag);
                }
            }
        }

        Ok(diagnostics)
    }
}

/// P0-HARNESS-010: Parse individual Rust diagnostic from JSON
fn parse_rust_diagnostic(
    value: &serde_json::Value,
    working_dir: &PathBuf,
) -> Option<CompilerDiagnostic> {
    let message = value.get("message")?.as_str()?.to_string();
    let level = value.get("level")?.as_str()?;

    let severity = match level {
        "error" => DiagnosticSeverity::Error,
        "warning" => DiagnosticSeverity::Warning,
        "note" => DiagnosticSeverity::Note,
        _ => DiagnosticSeverity::Info,
    };

    let code = value
        .get("code")
        .and_then(|v| v.as_str().map(|s| s.to_string()));

    let spans = value.get("spans")?.as_array()?;
    let primary_span = spans.first()?;

    let line = primary_span.get("line_start")?.as_u64()? as usize;
    let column = primary_span.get("column_start")?.as_u64()? as usize;

    let file_path = primary_span.get("file_name")?.as_str()?;
    let file = if file_path.starts_with("src/") {
        working_dir.join(file_path)
    } else {
        PathBuf::from(file_path)
    };

    let suggested = value
        .get("children")
        .and_then(|children| children.as_array())
        .and_then(|arr| arr.first())
        .and_then(|child| child.get("rendered"))
        .and_then(|rendered| rendered.as_str())
        .map(|s| s.to_string());

    let category = categorize_rust_diagnostic(&message, &code);

    Some(CompilerDiagnostic {
        file,
        line,
        column,
        code,
        message,
        suggested_replacement: suggested,
        severity,
        category,
    })
}

/// P0-HARNESS-010: Categorize Rust diagnostic for targeted repair
fn categorize_rust_diagnostic(message: &str, code: &Option<String>) -> DiagnosticCategory {
    let msg_lower = message.to_lowercase();

    if msg_lower.contains("borrow") || msg_lower.contains("move") || msg_lower.contains("ownership")
    {
        DiagnosticCategory::Borrowing
    } else if msg_lower.contains("lifetime") {
        DiagnosticCategory::Lifetime
    } else if msg_lower.contains("trait") || msg_lower.contains("impl") {
        DiagnosticCategory::Trait
    } else if msg_lower.contains("macro") {
        DiagnosticCategory::Macro
    } else if msg_lower.contains("unused") {
        DiagnosticCategory::Unused
    } else if msg_lower.contains("type") {
        DiagnosticCategory::Type
    } else if msg_lower.contains("import") || msg_lower.contains("use") {
        DiagnosticCategory::Import
    } else if msg_lower.contains("format") || msg_lower.contains("style") {
        DiagnosticCategory::Format
    } else if code.as_ref().map_or(false, |c| c.contains("clippy")) {
        DiagnosticCategory::Clippy
    } else if msg_lower.contains("test") {
        DiagnosticCategory::Test
    } else if msg_lower.contains("build") {
        DiagnosticCategory::Build
    } else {
        DiagnosticCategory::Syntax
    }
}

/// P0-HARNESS-010: TypeScript diagnostic parser (tsc --pretty false)
pub struct TypeScriptDiagnosticParser;

impl DiagnosticParser for TypeScriptDiagnosticParser {
    fn parse(&self, output: &str, working_dir: &PathBuf) -> Result<Vec<CompilerDiagnostic>> {
        let mut diagnostics = Vec::new();

        for line in output.lines() {
            if let Some(diag) = parse_typescript_diagnostic(line, working_dir) {
                diagnostics.push(diag);
            }
        }

        Ok(diagnostics)
    }
}

/// P0-HARNESS-010: Parse individual TypeScript diagnostic
fn parse_typescript_diagnostic(line: &str, working_dir: &PathBuf) -> Option<CompilerDiagnostic> {
    // TypeScript error format: file(line,column): error TS1234: message
    let ts_regex =
        regex::Regex::new(r"^(.+)\((\d+),(\d+)\):\s+(error|warning|info)\s+TS(\d+):\s+(.+)$")
            .ok()?;

    if let Some(caps) = ts_regex.captures(line) {
        let file = working_dir.join(&caps[1]);
        let line_num = caps[2].parse().ok()?;
        let column = caps[3].parse().ok()?;
        let severity = match &caps[4] {
            "error" => DiagnosticSeverity::Error,
            "warning" => DiagnosticSeverity::Warning,
            "info" => DiagnosticSeverity::Info,
            _ => DiagnosticSeverity::Note,
        };
        let code = Some(format!("TS{}", &caps[5]));
        let message = caps[6].to_string();

        let category = categorize_typescript_diagnostic(&message);

        Some(CompilerDiagnostic {
            file,
            line: line_num,
            column,
            code,
            message,
            suggested_replacement: None,
            severity,
            category,
        })
    } else {
        None
    }
}

/// P0-HARNESS-010: Categorize TypeScript diagnostic
fn categorize_typescript_diagnostic(message: &str) -> DiagnosticCategory {
    let msg_lower = message.to_lowercase();

    if msg_lower.contains("type") {
        DiagnosticCategory::Type
    } else if msg_lower.contains("import") || msg_lower.contains("module") {
        DiagnosticCategory::Import
    } else if msg_lower.contains("unused") {
        DiagnosticCategory::Unused
    } else {
        DiagnosticCategory::Syntax
    }
}

/// P0-HARNESS-010: Python diagnostic parser (pytest/mypy/ruff)
pub struct PythonDiagnosticParser;

impl DiagnosticParser for PythonDiagnosticParser {
    fn parse(&self, output: &str, working_dir: &PathBuf) -> Result<Vec<CompilerDiagnostic>> {
        let mut diagnostics = Vec::new();

        for line in output.lines() {
            if let Some(diag) = parse_python_diagnostic(line, working_dir) {
                diagnostics.push(diag);
            }
        }

        Ok(diagnostics)
    }
}

/// P0-HARNESS-010: Parse individual Python diagnostic
fn parse_python_diagnostic(line: &str, working_dir: &PathBuf) -> Option<CompilerDiagnostic> {
    // Python error format: file.py:line: error message
    let py_regex = regex::Regex::new(r"^(.+):(\d+):\s+(error|warning|info):\s+(.+)$").ok()?;

    if let Some(caps) = py_regex.captures(line) {
        let file = working_dir.join(&caps[1]);
        let line_num = caps[2].parse().ok()?;
        let severity = match &caps[3] {
            "error" => DiagnosticSeverity::Error,
            "warning" => DiagnosticSeverity::Warning,
            "info" => DiagnosticSeverity::Info,
            _ => DiagnosticSeverity::Note,
        };
        let message = caps[4].to_string();

        let category = categorize_python_diagnostic(&message);

        Some(CompilerDiagnostic {
            file,
            line: line_num,
            column: 1, // Python typically doesn't provide column info
            code: None,
            message,
            suggested_replacement: None,
            severity,
            category,
        })
    } else {
        None
    }
}

/// P0-HARNESS-010: Categorize Python diagnostic
fn categorize_python_diagnostic(message: &str) -> DiagnosticCategory {
    let msg_lower = message.to_lowercase();

    if msg_lower.contains("import") || msg_lower.contains("module") {
        DiagnosticCategory::Import
    } else if msg_lower.contains("unused") {
        DiagnosticCategory::Unused
    } else if msg_lower.contains("type") {
        DiagnosticCategory::Type
    } else if msg_lower.contains("syntax") {
        DiagnosticCategory::Syntax
    } else if msg_lower.contains("test") {
        DiagnosticCategory::Test
    } else {
        DiagnosticCategory::Format
    }
}

/// P0-HARNESS-010: Diagnostic parser factory
pub struct DiagnosticParserFactory;

impl DiagnosticParserFactory {
    /// Create appropriate parser for language/tool
    pub fn create_for_command(command: &str) -> Box<dyn DiagnosticParser> {
        if command.contains("cargo check") {
            Box::new(RustDiagnosticParser)
        } else if command.contains("tsc") || command.contains("typescript") {
            Box::new(TypeScriptDiagnosticParser)
        } else if command.contains("pytest") || command.contains("mypy") || command.contains("ruff")
        {
            Box::new(PythonDiagnosticParser)
        } else {
            Box::new(GenericDiagnosticParser)
        }
    }
}

/// P0-HARNESS-010: Generic diagnostic parser fallback
pub struct GenericDiagnosticParser;

impl DiagnosticParser for GenericDiagnosticParser {
    fn parse(&self, output: &str, _working_dir: &PathBuf) -> Result<Vec<CompilerDiagnostic>> {
        // Simple line-based parsing for unknown tools
        let mut diagnostics = Vec::new();
        let mut line_num = 1;

        for line in output.lines() {
            if line.trim().is_empty() {
                continue;
            }

            // Try to extract severity from common patterns
            let (severity, message) = if let Some((sev, msg)) = extract_severity_and_message(line) {
                (sev, msg.to_string())
            } else {
                (DiagnosticSeverity::Info, line.to_string())
            };

            diagnostics.push(CompilerDiagnostic {
                file: PathBuf::from("unknown"), // Would need context for real file path
                line: line_num,
                column: 1,
                code: None,
                message,
                suggested_replacement: None,
                severity,
                category: DiagnosticCategory::Syntax,
            });

            line_num += 1;
        }

        Ok(diagnostics)
    }
}

/// P0-HARNESS-010: Extract severity and message from generic diagnostic line
fn extract_severity_and_message(line: &str) -> Option<(DiagnosticSeverity, &str)> {
    let patterns = [
        ("error:", DiagnosticSeverity::Error),
        ("warning:", DiagnosticSeverity::Warning),
        ("info:", DiagnosticSeverity::Info),
        ("note:", DiagnosticSeverity::Note),
    ];

    for (pattern, severity) in &patterns {
        if let Some(pos) = line.find(pattern) {
            let msg_start = pos + pattern.len();
            let message = line[msg_start..].trim();
            return Some((severity.clone(), message));
        }
    }

    None
}

/// P0-HARNESS-010: Convert severity string to enum
impl From<&str> for DiagnosticSeverity {
    fn from(s: &str) -> Self {
        match s {
            "error" => DiagnosticSeverity::Error,
            "warning" => DiagnosticSeverity::Warning,
            "info" => DiagnosticSeverity::Info,
            "note" => DiagnosticSeverity::Note,
            _ => DiagnosticSeverity::Info,
        }
    }
}

/// P0-HARNESS-010: Intelligent repair strategy based on diagnostics
pub struct RepairStrategy {
    pub category: DiagnosticCategory,
    pub strategy: RepairStrategyType,
    pub confidence: f32,
}

/// P0-HARNESS-010: Repair strategy types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RepairStrategyType {
    /// Replace problematic code with suggested fix
    ReplaceWithSuggestion,
    /// Add missing import or use statement
    AddImport,
    /// Fix type annotation or conversion
    FixType,
    /// Remove unused code
    RemoveUnused,
    /// Restructure borrowing/lifetime
    RestructureOwnership,
    /// Implement missing trait method
    ImplementTrait,
    /// Add macro attributes or fix macro usage
    FixMacro,
    /// Apply formatting rules
    ApplyFormatting,
    /// Reorder code structure
    ReorderCode,
}

/// P0-HARNESS-010: Generate repair strategy from diagnostic
pub fn generate_repair_strategy(diagnostic: &CompilerDiagnostic) -> RepairStrategy {
    let (strategy, confidence) = match (&diagnostic.category, &diagnostic.message.to_lowercase()) {
        (DiagnosticCategory::Borrowing, msg) if msg.contains("borrowed") => {
            (RepairStrategyType::RestructureOwnership, 0.8)
        }
        (DiagnosticCategory::Borrowing, msg) if msg.contains("move") => {
            (RepairStrategyType::RestructureOwnership, 0.8)
        }
        (DiagnosticCategory::Lifetime, _) => (RepairStrategyType::RestructureOwnership, 0.7),
        (DiagnosticCategory::Trait, msg) if msg.contains("missing") => {
            (RepairStrategyType::ImplementTrait, 0.9)
        }
        (DiagnosticCategory::Import, msg) if msg.contains("not found") => {
            (RepairStrategyType::AddImport, 0.9)
        }
        (DiagnosticCategory::Unused, msg) if msg.contains("unused") => {
            (RepairStrategyType::RemoveUnused, 0.8)
        }
        (DiagnosticCategory::Type, msg)
            if msg.contains("mismatch") || msg.contains("cannot find type") =>
        {
            (RepairStrategyType::FixType, 0.7)
        }
        (DiagnosticCategory::Format, _) => (RepairStrategyType::ApplyFormatting, 0.6),
        (DiagnosticCategory::Clippy, msg) if msg.contains("must") => {
            (RepairStrategyType::ReplaceWithSuggestion, 0.8)
        }
        _ => (RepairStrategyType::ReorderCode, 0.5),
    };

    RepairStrategy {
        category: diagnostic.category.clone(),
        strategy,
        confidence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rust_diagnostic_parsing() {
        let parser = RustDiagnosticParser {};
        let working_dir = std::path::PathBuf::from("/workspace");
        let output = r#"{"message":"cannot find type `NonExistentType` in this scope","code":"E0412","level":"error","spans":[{"file_name":"src/main.rs","line_start":10,"column_start":20,"line_end":10,"column_end":35}]}"#;

        let diagnostics = parser.parse(output, &working_dir).unwrap();
        assert_eq!(diagnostics.len(), 1);

        let diag = &diagnostics[0];
        assert_eq!(diag.file, working_dir.join("src/main.rs"));
        assert_eq!(diag.line, 10);
        assert_eq!(diag.column, 20);
        assert_eq!(diag.code, Some("E0412".to_string()));
        assert_eq!(diag.severity, DiagnosticSeverity::Error);
        assert_eq!(diag.category, DiagnosticCategory::Type);
    }

    #[test]
    fn test_repair_strategy_generation() {
        let diagnostic = CompilerDiagnostic {
            file: std::path::PathBuf::from("test.rs"),
            line: 15,
            column: 10,
            code: Some("E0412".to_string()),
            message: "cannot find type `NonExistentType` in this scope".to_string(),
            suggested_replacement: None,
            severity: DiagnosticSeverity::Error,
            category: DiagnosticCategory::Type,
        };

        let strategy = generate_repair_strategy(&diagnostic);
        assert_eq!(strategy.category, DiagnosticCategory::Type);
        assert_eq!(strategy.strategy, RepairStrategyType::FixType);
        assert!(strategy.confidence > 0.5);
    }
}
