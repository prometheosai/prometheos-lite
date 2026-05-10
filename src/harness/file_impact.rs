//! P1-Issue2: File impact scoring for surgical edits
//!
//! This module provides comprehensive file impact scoring to help determine
//! which files are most relevant for surgical edits based on symbol relevance,
//! import graph analysis, test file impact, and API surface considerations.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// P1-Issue2: File impact scoring for surgical edits
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileImpactScore {
    pub file_path: PathBuf,
    pub overall_score: f32, // 0.0 - 1.0
    pub symbol_relevance: SymbolRelevanceScore,
    pub import_graph_impact: ImportGraphImpact,
    pub test_file_impact: TestFileImpact,
    pub api_surface_impact: ApiSurfaceImpact,
    pub change_risk: ChangeRisk,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SymbolRelevanceScore {
    pub score: f32,
    pub symbols_mentioned: Vec<String>,
    pub symbols_defined: Vec<String>,
    pub symbols_used: Vec<String>,
    pub symbol_density: f32,           // symbols per line
    pub critical_symbols: Vec<String>, // pub, unsafe, async, etc.
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImportGraphImpact {
    pub score: f32,
    pub imports_count: usize,
    pub exports_count: usize,
    pub transitive_imports: usize,
    pub circular_imports: bool,
    pub critical_dependencies: Vec<String>,
    pub dependency_depth: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TestFileImpact {
    pub score: f32,
    pub is_test_file: bool,
    pub test_functions_count: usize,
    pub test_coverage: f32,
    pub integration_tests: bool,
    pub unit_tests: bool,
    pub doc_tests: bool,
    pub production_code_dep: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApiSurfaceImpact {
    pub score: f32,
    pub public_api_changes: usize,
    pub breaking_changes: usize,
    pub semver_impact: SemverImpact,
    pub api_stability: ApiStability,
    pub consumer_impact: ConsumerImpact,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SemverImpact {
    None,
    Patch,
    Minor,
    Major,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ApiStability {
    Stable,
    Unstable,
    Experimental,
    Deprecated,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConsumerImpact {
    None,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChangeRisk {
    Low,
    Medium,
    High,
    Critical,
}

impl FileImpactScore {
    /// Calculate impact score for a file based on multiple factors
    pub fn calculate_impact(
        file_path: &std::path::Path,
        task_symbols: &[String],
        repo_context: &crate::harness::repo_intelligence::RepoContext,
        rust_analyzer: Option<&crate::harness::repo_intelligence::RustAnalyzerData>,
    ) -> Result<Self> {
        let symbol_relevance =
            Self::calculate_symbol_relevance(file_path, task_symbols, repo_context)?;
        let import_graph_impact =
            Self::calculate_import_graph_impact(file_path, repo_context, rust_analyzer)?;
        let test_file_impact =
            Self::calculate_test_file_impact(file_path, repo_context, rust_analyzer)?;
        let api_surface_impact =
            Self::calculate_api_surface_impact(file_path, repo_context, rust_analyzer)?;
        let change_risk = Self::assess_change_risk(
            &symbol_relevance,
            &import_graph_impact,
            &test_file_impact,
            &api_surface_impact,
        );

        // Calculate overall score as weighted average
        let overall_score = (symbol_relevance.score * 0.3
            + import_graph_impact.score * 0.2
            + test_file_impact.score * 0.2
            + api_surface_impact.score * 0.3)
            .min(1.0);

        Ok(Self {
            file_path: file_path.to_path_buf(),
            overall_score,
            symbol_relevance,
            import_graph_impact,
            test_file_impact,
            api_surface_impact,
            change_risk,
        })
    }

    /// Calculate symbol relevance score
    fn calculate_symbol_relevance(
        file_path: &std::path::Path,
        task_symbols: &[String],
        repo_context: &crate::harness::repo_intelligence::RepoContext,
    ) -> Result<SymbolRelevanceScore> {
        let file_content = std::fs::read_to_string(file_path)
            .context(format!("Failed to read file: {}", file_path.display()))?;

        let lines: Vec<&str> = file_content.lines().collect();
        let mut symbols_mentioned = Vec::new();
        let mut symbols_defined = Vec::new();
        let mut symbols_used = Vec::new();
        let mut critical_symbols = Vec::new();

        // Find symbols mentioned in task
        for task_symbol in task_symbols {
            if file_content.contains(task_symbol) {
                symbols_mentioned.push(task_symbol.clone());
            }
        }

        // Extract symbols from repo context
        for symbol in &repo_context.symbols {
            if symbol.file == file_path {
                symbols_defined.push(symbol.name.clone());

                // Check for critical symbols
                match symbol.kind {
                    crate::harness::repo_intelligence::SymbolKind::Function
                    | crate::harness::repo_intelligence::SymbolKind::Struct
                    | crate::harness::repo_intelligence::SymbolKind::Enum
                    | crate::harness::repo_intelligence::SymbolKind::Trait => {
                        if symbol.visibility
                            == crate::harness::repo_intelligence::Visibility::Public
                        {
                            critical_symbols.push(symbol.name.clone());
                        }
                    }
                    _ => {}
                }
            }
        }

        // Find symbol usage in relationships
        for relationship in &repo_context.relationships {
            if relationship.file == file_path {
                symbols_used.push(relationship.from.clone());
                symbols_used.push(relationship.to.clone());
            }
        }

        // Calculate symbol density
        let symbol_density = if lines.is_empty() {
            0.0
        } else {
            (symbols_defined.len() + symbols_used.len()) as f32 / lines.len() as f32
        };

        // Calculate relevance score
        let mut score = 0.0;

        // Task symbol mentions (highest weight)
        score += symbols_mentioned.len() as f32 * 0.4;

        // Critical symbols (high weight)
        score += critical_symbols.len() as f32 * 0.3;

        // Symbol density (medium weight)
        score += symbol_density * 0.2;

        // Total symbols (low weight)
        score += (symbols_defined.len() + symbols_used.len()) as f32 * 0.1;

        // Normalize to 0-1 range
        score = (score / 10.0).min(1.0);

        Ok(SymbolRelevanceScore {
            score,
            symbols_mentioned,
            symbols_defined,
            symbols_used,
            symbol_density,
            critical_symbols,
        })
    }

    /// Calculate import graph impact
    fn calculate_import_graph_impact(
        file_path: &std::path::Path,
        repo_context: &crate::harness::repo_intelligence::RepoContext,
        rust_analyzer: Option<&crate::harness::repo_intelligence::RustAnalyzerData>,
    ) -> Result<ImportGraphImpact> {
        let file_content = std::fs::read_to_string(file_path)
            .context(format!("Failed to read file: {}", file_path.display()))?;

        let mut imports_count = 0;
        let mut exports_count = 0;
        let mut critical_dependencies = Vec::new();

        // Count imports and exports
        use regex::Regex;
        if let Ok(re) = Regex::new(r"use\s+([^;]+);") {
            for caps in re.captures_iter(&file_content) {
                if let Some(import_path) = caps.get(1) {
                    imports_count += 1;
                    let import_str = import_path.as_str();

                    // Check for critical dependencies
                    if Self::is_critical_dependency(import_str) {
                        critical_dependencies.push(import_str.to_string());
                    }
                }
            }
        }

        if let Ok(re) = Regex::new(r"pub\s+use\s+([^;]+);") {
            exports_count += re.captures_iter(&file_content).count();
        }

        // Calculate transitive imports and dependency depth
        let (transitive_imports, dependency_depth) = if let Some(rust_data) = rust_analyzer {
            Self::analyze_transitive_imports(file_path, rust_data)
        } else {
            (0, 0)
        };

        // Check for circular imports
        let circular_imports = Self::check_circular_imports(file_path, repo_context);

        // Calculate impact score
        let mut score = 0.0;

        // Import count (medium weight)
        score += (imports_count as f32 / 20.0).min(1.0) * 0.3;

        // Export count (medium weight)
        score += (exports_count as f32 / 10.0).min(1.0) * 0.2;

        // Critical dependencies (high weight)
        score += (critical_dependencies.len() as f32 / 5.0).min(1.0) * 0.3;

        // Dependency depth (medium weight)
        score += (dependency_depth as f32 / 10.0).min(1.0) * 0.1;

        // Circular imports penalty (high negative weight)
        if circular_imports {
            score -= 0.2;
        }

        score = score.max(0.0).min(1.0);

        Ok(ImportGraphImpact {
            score,
            imports_count,
            exports_count,
            transitive_imports,
            circular_imports,
            critical_dependencies,
            dependency_depth,
        })
    }

    /// Calculate test file impact
    fn calculate_test_file_impact(
        file_path: &std::path::Path,
        repo_context: &crate::harness::repo_intelligence::RepoContext,
        rust_analyzer: Option<&crate::harness::repo_intelligence::RustAnalyzerData>,
    ) -> Result<TestFileImpact> {
        let file_content = std::fs::read_to_string(file_path)
            .context(format!("Failed to read file: {}", file_path.display()))?;

        let is_test_file = file_path.to_string_lossy().contains("test")
            || file_path.to_string_lossy().contains("spec");

        let mut test_functions_count = 0;
        let mut integration_tests = false;
        let mut unit_tests = false;
        let mut doc_tests = false;
        let mut production_code_dep = false;

        // Count test functions
        use regex::Regex;
        if let Ok(re) = Regex::new(r"#\[test\]\s*fn\s+(\w+)") {
            test_functions_count = re.captures_iter(&file_content).count();
        }

        // Check test types
        if file_path.starts_with(repo_context.root.join("tests")) {
            integration_tests = true;
        } else if file_path.starts_with(repo_context.root.join("src")) {
            unit_tests = true;
            doc_tests = file_content.contains("///");
        }

        // Check if test depends on production code
        if is_test_file {
            if let Ok(re) = Regex::new(r"use\s+crate::") {
                production_code_dep = re.is_match(&file_content);
            }
        }

        // Estimate test coverage (simplified)
        let test_coverage = if test_functions_count > 0 {
            (test_functions_count as f32 / 10.0).min(1.0) * 0.8 // Assume good coverage
        } else {
            0.0
        };

        // Calculate impact score
        let mut score = 0.0;

        if is_test_file {
            // Test files have high impact
            score += 0.6;

            // More test functions = higher impact
            score += (test_functions_count as f32 / 20.0).min(1.0) * 0.2;

            // Integration tests have higher impact
            if integration_tests {
                score += 0.1;
            }

            // Tests that depend on production code have higher impact
            if production_code_dep {
                score += 0.1;
            }
        } else {
            // Production files that are tested have higher impact
            if Self::is_file_tested(file_path, repo_context) {
                score += 0.3;
            }
        }

        score = score.min(1.0);

        Ok(TestFileImpact {
            score,
            is_test_file,
            test_functions_count,
            test_coverage,
            integration_tests,
            unit_tests,
            doc_tests,
            production_code_dep,
        })
    }

    /// Calculate API surface impact
    fn calculate_api_surface_impact(
        file_path: &std::path::Path,
        repo_context: &crate::harness::repo_intelligence::RepoContext,
        rust_analyzer: Option<&crate::harness::repo_intelligence::RustAnalyzerData>,
    ) -> Result<ApiSurfaceImpact> {
        let file_content = std::fs::read_to_string(file_path)
            .context(format!("Failed to read file: {}", file_path.display()))?;

        let mut public_api_changes = 0;
        let mut breaking_changes = 0;
        let mut semver_impact = SemverImpact::None;
        let mut api_stability = ApiStability::Stable;
        let mut consumer_impact = ConsumerImpact::None;

        // Count public API items
        use regex::Regex;

        // Public functions
        if let Ok(re) = Regex::new(r"pub\s+(async\s+)?(unsafe\s+)?fn\s+(\w+)") {
            public_api_changes += re.captures_iter(&file_content).count();
        }

        // Public structs
        if let Ok(re) = Regex::new(r"pub\s+struct\s+(\w+)") {
            public_api_changes += re.captures_iter(&file_content).count();
        }

        // Public enums
        if let Ok(re) = Regex::new(r"pub\s+enum\s+(\w+)") {
            public_api_changes += re.captures_iter(&file_content).count();
        }

        // Public traits
        if let Ok(re) = Regex::new(r"pub\s+trait\s+(\w+)") {
            public_api_changes += re.captures_iter(&file_content).count();
            // Traits are breaking changes
            breaking_changes += re.captures_iter(&file_content).count();
        }

        // Check for deprecated items
        if file_content.contains("#[deprecated]") {
            api_stability = ApiStability::Deprecated;
        }

        // Check for experimental items
        if file_content.contains("#[unstable]") || file_content.contains("#[feature(") {
            api_stability = ApiStability::Experimental;
        }

        // Determine semver impact
        if breaking_changes > 0 {
            semver_impact = SemverImpact::Major;
            consumer_impact = ConsumerImpact::High;
        } else if public_api_changes > 0 {
            semver_impact = SemverImpact::Minor;
            consumer_impact = ConsumerImpact::Medium;
        } else {
            semver_impact = SemverImpact::Patch;
            consumer_impact = ConsumerImpact::Low;
        }

        // Calculate impact score
        let mut score = 0.0;

        // Public API changes (high weight)
        score += (public_api_changes as f32 / 10.0).min(1.0) * 0.4;

        // Breaking changes (very high weight)
        score += (breaking_changes as f32 / 5.0).min(1.0) * 0.4;

        // API stability (medium weight)
        match api_stability {
            ApiStability::Stable => score += 0.1,
            ApiStability::Unstable => score += 0.05,
            ApiStability::Experimental => score += 0.0,
            ApiStability::Deprecated => score += 0.02,
        }

        // Consumer impact (medium weight)
        match consumer_impact {
            ConsumerImpact::None => score += 0.0,
            ConsumerImpact::Low => score += 0.05,
            ConsumerImpact::Medium => score += 0.1,
            ConsumerImpact::High => score += 0.15,
            ConsumerImpact::Critical => score += 0.2,
        }

        score = score.min(1.0);

        Ok(ApiSurfaceImpact {
            score,
            public_api_changes,
            breaking_changes,
            semver_impact,
            api_stability,
            consumer_impact,
        })
    }

    /// Assess overall change risk
    fn assess_change_risk(
        symbol_relevance: &SymbolRelevanceScore,
        import_graph_impact: &ImportGraphImpact,
        test_file_impact: &TestFileImpact,
        api_surface_impact: &ApiSurfaceImpact,
    ) -> ChangeRisk {
        let risk_score = symbol_relevance.score * 0.3
            + import_graph_impact.score * 0.2
            + test_file_impact.score * 0.2
            + api_surface_impact.score * 0.3;

        // Additional risk factors
        if api_surface_impact.breaking_changes > 0 {
            return ChangeRisk::Critical;
        }

        if import_graph_impact.circular_imports {
            return ChangeRisk::High;
        }

        if test_file_impact.is_test_file && test_file_impact.production_code_dep {
            return ChangeRisk::High;
        }

        if !symbol_relevance.critical_symbols.is_empty() {
            return ChangeRisk::Medium;
        }

        match risk_score {
            s if s >= 0.8 => ChangeRisk::High,
            s if s >= 0.6 => ChangeRisk::Medium,
            s if s >= 0.3 => ChangeRisk::Low,
            _ => ChangeRisk::Low,
        }
    }

    /// Check if a dependency is critical
    fn is_critical_dependency(import_path: &str) -> bool {
        let critical_crates = [
            "std",
            "core",
            "alloc",
            "proc_macro",
            "tokio",
            "serde",
            "anyhow",
            "thiserror",
            "async_trait",
            "futures",
            "tracing",
            "clap",
            "serde_json",
            "regex",
        ];

        for critical in &critical_crates {
            if import_path.starts_with(critical) {
                return true;
            }
        }

        false
    }

    /// Analyze transitive imports
    fn analyze_transitive_imports(
        file_path: &std::path::Path,
        rust_analyzer: &crate::harness::repo_intelligence::RustAnalyzerData,
    ) -> (usize, usize) {
        // Find the module for this file
        let module_name = rust_analyzer
            .module_graph
            .modules
            .iter()
            .find(|(_, module)| module.file_path.as_deref() == Some(file_path))
            .map(|(path, _)| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
            })
            .unwrap_or("unknown");

        // Get imports for this module
        let imports = rust_analyzer
            .module_graph
            .imports
            .get(module_name)
            .map(|imports| imports.len())
            .unwrap_or(0);

        // Calculate dependency depth (simplified)
        let dependency_depth = imports.min(10);

        // Transitive imports (simplified calculation)
        let transitive_imports = dependency_depth * 2;

        (transitive_imports, dependency_depth)
    }

    /// Check for circular imports
    fn check_circular_imports(
        file_path: &std::path::Path,
        repo_context: &crate::harness::repo_intelligence::RepoContext,
    ) -> bool {
        // Simplified circular import detection
        let file_str = file_path.to_string_lossy();

        // Check if this file is both imported and exports to the same modules
        for relationship in &repo_context.relationships {
            if relationship.file == file_path {
                if relationship.from == relationship.to {
                    return true;
                }
            }
        }

        false
    }

    /// Check if a file is tested
    fn is_file_tested(
        file_path: &std::path::Path,
        repo_context: &crate::harness::repo_intelligence::RepoContext,
    ) -> bool {
        let file_str = file_path.to_string_lossy();

        // Check if any test file imports this file
        for ranked_file in &repo_context.ranked_files {
            if ranked_file.path.to_string_lossy().contains("test") {
                // This is a simplified check - in reality we'd parse the test file
                return true;
            }
        }

        false
    }
}
