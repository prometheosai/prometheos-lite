use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use crate::harness::risk::RiskAssessment;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SemanticDiff {
    pub api_changes: Vec<ApiChange>,
    pub auth_changes: Vec<AuthChange>,
    pub database_changes: Vec<DatabaseChange>,
    pub dependency_changes: Vec<DependencyChange>,
    pub config_changes: Vec<ConfigChange>,
    pub file_changes: Vec<FileChange>,
    pub changed_files: Vec<FileChange>,
    pub risk_assessment: RiskAssessment,
    pub summary: SemanticSummary,
}

impl Default for SemanticDiff {
    fn default() -> Self {
        Self {
            api_changes: Vec::new(),
            auth_changes: Vec::new(),
            database_changes: Vec::new(),
            dependency_changes: Vec::new(),
            config_changes: Vec::new(),
            file_changes: Vec::new(),
            changed_files: Vec::new(),
            risk_assessment: RiskAssessment {
                level: crate::harness::risk::RiskLevel::None,
                reasons: Vec::new(),
                requires_approval: false,
                can_override: false,
                override_conditions: Vec::new(),
                assessed: false,
            },
            summary: SemanticSummary::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiChange {
    pub file: PathBuf,
    pub line: Option<usize>,
    pub change_type: ApiChangeType,
    pub signature: String,
    pub breaking: bool,
    pub description: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ApiChangeType {
    FunctionAdded,
    FunctionRemoved,
    FunctionModified,
    StructAdded,
    StructRemoved,
    StructModified,
    EnumAdded,
    EnumRemoved,
    EnumModified,
    TraitAdded,
    TraitRemoved,
    TraitModified,
    TypeAliasAdded,
    TypeAliasRemoved,
    TypeAliasModified,
    VisibilityChanged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthChange {
    pub file: PathBuf,
    pub line: Option<usize>,
    pub change_type: AuthChangeType,
    pub description: String,
    pub risk_level: RiskLevel,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AuthChangeType {
    Authentication,
    AuthenticationModified,
    Authorization,
    Permission,
    Token,
    Jwt,
    OAuth,
    Session,
    Credential,
    Secret,
    Hash,
    Login,
    Logout,
    SignOut,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatabaseChange {
    pub file: PathBuf,
    pub line: Option<usize>,
    pub change_type: DatabaseChangeType,
    pub description: String,
    pub migration_required: bool,
    pub breaking: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DatabaseChangeType {
    TableCreated,
    TableDropped,
    TableModified,
    TableAdded,
    TableRemoved,
    Migration,
    MigrationAdded,
    ColumnAdded,
    ColumnRemoved,
    ColumnModified,
    IndexAdded,
    IndexRemoved,
    IndexModified,
    ConstraintAdded,
    ConstraintRemoved,
    ConstraintModified,
    QueryModified,
    ViewCreated,
    ViewRemoved,
    ViewModified,
    TriggerCreated,
    TriggerRemoved,
    TriggerModified,
    StoredProcedureCreated,
    StoredProcedureRemoved,
    StoredProcedureModified,
    FunctionCreated,
    FunctionRemoved,
    FunctionModified,
    SchemaModified,
    SchemaRemoved,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DependencyChange {
    pub file: PathBuf,
    pub package_name: String,
    pub old_version: Option<String>,
    pub new_version: Option<String>,
    pub change_type: DependencyChangeType,
    pub risk_level: RiskLevel,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DependencyChangeType {
    Added,
    Removed,
    Upgraded,
    Downgraded,
    Modified,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigChange {
    pub file: PathBuf,
    pub config_key: String,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
    pub change_type: ConfigChangeType,
    pub environment: ConfigEnvironment,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConfigChangeType {
    Added,
    Removed,
    Modified,
    DefaultChanged,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConfigEnvironment {
    Development,
    Production,
    Test,
    All,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileChange {
    pub path: PathBuf,
    pub change_type: FileChangeType,
    pub lines_added: usize,
    pub lines_removed: usize,
    pub semantic_category: SemanticCategory,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FileChangeType {
    Added,
    Removed,
    Modified,
    Renamed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SemanticCategory {
    SourceCode,
    Test,
    Configuration,
    Documentation,
    Build,
    Dependency,
    Migration,
    Secret,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum RiskLevel {
    #[default]
    None,
    Low,
    Medium,
    High,
    Critical,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SemanticSummary {
    pub total_files_changed: usize,
    pub total_lines_added: usize,
    pub total_lines_removed: usize,
    pub breaking_changes: usize,
    pub api_surface_changes: usize,
    pub security_relevant_changes: usize,
    pub infrastructure_changes: usize,
}

#[derive(Debug, Clone)]
pub struct SemanticDiffAnalyzer {
    api_patterns: Vec<Regex>,
    auth_patterns: Vec<Regex>,
    db_patterns: Vec<Regex>,
    dep_patterns: Vec<Regex>,
    config_patterns: Vec<Regex>,
    breaking_patterns: Vec<Regex>,
    secret_patterns: Vec<Regex>,
}

impl Default for SemanticDiffAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl SemanticDiffAnalyzer {
    pub fn new() -> Self {
        Self {
            api_patterns: vec![
                Regex::new(r"(?i)pub\s+(?:async\s+)?fn\s+(\w+)").unwrap(),
                Regex::new(r"(?i)pub\s+struct\s+(\w+)").unwrap(),
                Regex::new(r"(?i)pub\s+enum\s+(\w+)").unwrap(),
                Regex::new(r"(?i)pub\s+trait\s+(\w+)").unwrap(),
                Regex::new(r"(?i)pub\s+type\s+(\w+)").unwrap(),
                Regex::new(r"(?i)pub\s+mod\s+(\w+)").unwrap(),
            ],
            auth_patterns: vec![
                Regex::new(r"(?i)(?:auth|login|password|token|jwt|session|credential|secret|hash|bcrypt|argon2|pbkdf2|scrypt)").unwrap(),
                Regex::new(r"(?i)(?:oauth|bearer|basic|digest)").unwrap(),
                Regex::new(r"(?i)(?:signin|signout|logout|permission|role|access)").unwrap(),
            ],
            db_patterns: vec![
                Regex::new(r"(?i)(?:create|drop|alter|table|column|index|constraint|schema|migration)").unwrap(),
                Regex::new(r"(?i)(?:insert|update|delete|select|from|where|join)").unwrap(),
                Regex::new(r"(?i)(?:sql|query|database|db|pool|connection)").unwrap(),
            ],
            dep_patterns: vec![
                Regex::new(r"(?i)(?:add|remove|upgrade|downgrade|dependency|package|module|import)").unwrap(),
                Regex::new(r"(?i)(?:npm|pip|cargo|yarn|composer|maven|gradle)").unwrap(),
                Regex::new(r"(?i)(?:require|include|use|import|from)").unwrap(),
            ],
            config_patterns: vec![
                Regex::new(r"(?i)(?:config|setting|option|parameter|variable)").unwrap(),
                Regex::new(r"(?i)(?:env|environment|dev|prod|test)").unwrap(),
                Regex::new(r"(?i)(?:toml|yaml|json|xml|ini|cfg)").unwrap(),
            ],
            breaking_patterns: vec![
                Regex::new(r"(?i)(?:remove|delete|drop|break|breaking|deprecate)").unwrap(),
                Regex::new(r"(?i)(?:signature|interface|api|contract)").unwrap(),
                Regex::new(r"(?i)(?:major|version|v[2-9])").unwrap(),
            ],
            secret_patterns: vec![
                Regex::new(r"(?i)(?:secret|password|token|key|credential|private)").unwrap(),
                Regex::new(r"(?i)(?:api[_-]?key|access[_-]?token|secret[_-]?key)").unwrap(),
                Regex::new(r"(?i)(?:aws|azure|gcp|github|gitlab)").unwrap(),
            ],
        }
    }

    // Additional helper methods for semantic analysis
    fn infer_api_change_type(&self, diff: &str, name: &str) -> ApiChangeType {
        match name {
            "fn" => {
                if self.is_addition(&diff, name) {
                    ApiChangeType::FunctionAdded
                } else if self.is_removal(&diff, name) {
                    ApiChangeType::FunctionRemoved
                } else {
                    ApiChangeType::FunctionModified
                }
            }
            "struct" => {
                if self.is_addition(&diff, name) {
                    ApiChangeType::StructAdded
                } else if self.is_removal(&diff, name) {
                    ApiChangeType::StructRemoved
                } else {
                    ApiChangeType::StructModified
                }
            }
            "enum" => {
                if self.is_addition(&diff, name) {
                    ApiChangeType::EnumAdded
                } else if self.is_removal(&diff, name) {
                    ApiChangeType::EnumRemoved
                } else {
                    ApiChangeType::EnumModified
                }
            }
            "trait" => {
                if self.is_addition(&diff, name) {
                    ApiChangeType::TraitAdded
                } else if self.is_removal(&diff, name) {
                    ApiChangeType::TraitRemoved
                } else {
                    ApiChangeType::TraitModified
                }
            }
            "type" => {
                if self.is_addition(&diff, name) {
                    ApiChangeType::TypeAliasAdded
                } else if self.is_removal(&diff, name) {
                    ApiChangeType::TypeAliasRemoved
                } else {
                    ApiChangeType::TypeAliasModified
                }
            }
            _ => ApiChangeType::FunctionModified,
        }
    }

    fn infer_auth_change_type(&self, diff: &str, pattern: &str) -> AuthChangeType {
        match pattern {
            p if p.contains("auth") => AuthChangeType::Authentication,
            p if p.contains("authoriz") => AuthChangeType::Authorization,
            p if p.contains("permission") => AuthChangeType::Permission,
            p if p.contains("token") => AuthChangeType::Token,
            p if p.contains("jwt") => AuthChangeType::Jwt,
            p if p.contains("oauth") => AuthChangeType::OAuth,
            p if p.contains("session") => AuthChangeType::Session,
            p if p.contains("credential") => AuthChangeType::Credential,
            p if p.contains("secret") => AuthChangeType::Secret,
            p if p.contains("hash") => AuthChangeType::Hash,
            p if p.contains("bcrypt") => AuthChangeType::Hash,
            p if p.contains("argon2") => AuthChangeType::Hash,
            p if p.contains("pbkdf2") => AuthChangeType::Hash,
            p if p.contains("scrypt") => AuthChangeType::Hash,
            p if p.contains("login") => AuthChangeType::Login,
            p if p.contains("logout") => AuthChangeType::Logout,
            p if p.contains("signin") => AuthChangeType::Login,
            p if p.contains("signout") => AuthChangeType::SignOut,
            _ => AuthChangeType::Authentication,
        }
    }

    fn infer_db_change_type(&self, diff: &str, pattern: &str) -> DatabaseChangeType {
        match pattern {
            p if p.contains("create") => DatabaseChangeType::TableCreated,
            p if p.contains("drop") => DatabaseChangeType::TableDropped,
            p if p.contains("alter") => DatabaseChangeType::TableModified,
            p if p.contains("migration") => DatabaseChangeType::Migration,
            _ => DatabaseChangeType::TableModified,
        }
    }

    fn infer_config_change_type(&self, diff: &str, pattern: &str) -> ConfigChangeType {
        match pattern {
            p if p.contains("add") => ConfigChangeType::Added,
            p if p.contains("remove") => ConfigChangeType::Removed,
            p if p.contains("modify") => ConfigChangeType::Modified,
            p if p.contains("default") => ConfigChangeType::DefaultChanged,
            _ => ConfigChangeType::Modified,
        }
    }

    fn infer_config_environment(&self, diff: &str, pattern: &str) -> ConfigEnvironment {
        match pattern {
            p if p.contains("prod") => ConfigEnvironment::Production,
            p if p.contains("production") => ConfigEnvironment::Production,
            p if p.contains("test") => ConfigEnvironment::Test,
            p if p.contains("dev") => ConfigEnvironment::Development,
            _ => ConfigEnvironment::Development,
        }
    }

    fn infer_dep_change_type(&self, diff: &str, pattern: &str) -> DependencyChangeType {
        match pattern {
            p if p.contains("add") => DependencyChangeType::Added,
            p if p.contains("remove") => DependencyChangeType::Removed,
            p if p.contains("upgrade") => DependencyChangeType::Upgraded,
            p if p.contains("downgrade") => DependencyChangeType::Downgraded,
            _ => DependencyChangeType::Modified,
        }
    }

    fn infer_file_change_type(&self, diff: &str, file: &Path) -> FileChangeType {
        // Look at the diff to determine change type
        if diff.contains(&format!("+++ b/{}", file.to_string_lossy())) {
            FileChangeType::Added
        } else if diff.contains(&format!("--- a/{}", file.to_string_lossy())) {
            FileChangeType::Removed
        } else if diff.contains(&format!("rename from")) {
            FileChangeType::Renamed
        } else {
            FileChangeType::Modified
        }
    }

    fn categorize_file(&self, file: &Path) -> SemanticCategory {
        let path_str = file.to_string_lossy().to_lowercase();
        let extension = file.extension().and_then(|s| s.to_str()).unwrap_or("");

        match extension {
            "rs" | "py" | "js" | "ts" | "go" | "java" => SemanticCategory::SourceCode,
            "toml" | "yaml" | "json" | "xml" | "ini" | "cfg" => SemanticCategory::Configuration,
            "md" | "rst" | "txt" | "adoc" => SemanticCategory::Documentation,
            "dockerfile" | "Dockerfile" | "docker-compose" | "containerfile" => SemanticCategory::Build,
            "sql" | "migration" | "seed" => SemanticCategory::Migration,
            _ => SemanticCategory::Unknown,
        }
    }

    fn is_addition(&self, diff: &str, name: &str) -> bool {
        diff.contains(&format!("+ {}", name)) || diff.contains(&format!("pub {}", name))
    }

    fn is_removal(&self, diff: &str, name: &str) -> bool {
        diff.contains(&format!("- {}", name)) || diff.contains(&format!("pub use {}", name))
    }

    fn is_breaking_change(&self, diff: &str, name: &str) -> bool {
        // Check for breaking change indicators
        let breaking_patterns = [
            "pub fn ",
            "pub struct ",
            "pub enum ",
            "pub trait ",
            "#[deprecated(",
        ];

        breaking_patterns.iter().any(|pattern| {
            diff.contains(pattern)
        })
    }

    fn requires_migration(&self, diff: &str, pattern: &str) -> bool {
        pattern.contains("migration") || pattern.contains("create") || pattern.contains("drop")
    }

    fn assess_auth_risk(&self, diff: &str, pattern: &str) -> RiskLevel {
        match pattern {
            p if p.contains("secret") || p.contains("hash") || p.contains("bcrypt") || p.contains("argon2") || p.contains("pbkdf2") || p.contains("scrypt") => RiskLevel::High,
            p if p.contains("jwt") || p.contains("oauth") || p.contains("session") => RiskLevel::Medium,
            p if p.contains("credential") => RiskLevel::High,
            _ => RiskLevel::Low,
        }
    }

    fn assess_dep_risk(&self, diff: &str, pattern: &str) -> RiskLevel {
        match pattern {
            p if p.contains("major") => RiskLevel::High,
            p if p.contains("minor") => RiskLevel::Medium,
            _ => RiskLevel::Low,
        }
    }

    // Missing extract_* methods
    fn extract_api_changes(&self, diff: &str, files: &[PathBuf]) -> Vec<ApiChange> {
        self.detect_api_changes(diff, files)
    }

    fn extract_auth_changes(&self, diff: &str, files: &[PathBuf]) -> Vec<AuthChange> {
        self.detect_auth_changes(diff, files)
    }

    fn extract_database_changes(&self, diff: &str, files: &[PathBuf]) -> Vec<DatabaseChange> {
        self.detect_database_changes(diff, files)
    }

    fn extract_config_changes(&self, diff: &str, files: &[PathBuf]) -> Vec<ConfigChange> {
        self.detect_config_changes(diff, files)
    }

    fn extract_dependency_changes(&self, diff: &str, files: &[PathBuf]) -> Vec<DependencyChange> {
        self.detect_dependency_changes(diff, files)
    }

    fn extract_file_changes(&self, diff: &str, files: &[PathBuf]) -> Vec<FileChange> {
        self.analyze_file_changes(diff, files)
    }

    
    pub fn analyze(&self, diff: &str, changed_files: &[PathBuf]) -> SemanticDiff {
        let api_changes = self.detect_api_changes(diff, changed_files);
        let auth_changes = self.detect_auth_changes(diff, changed_files);
        let database_changes = self.detect_database_changes(diff, changed_files);
        let dependency_changes = self.detect_dependency_changes(diff, changed_files);
        let config_changes = self.detect_config_changes(diff, changed_files);
        let file_changes = self.analyze_file_changes(diff, changed_files);

        let summary = self.generate_summary(
            &api_changes,
            &auth_changes,
            &database_changes,
            &config_changes,
            &dependency_changes,
            &file_changes,
        );
        let risk_assessment = self.assess_risk(
            &api_changes,
            &auth_changes,
            &database_changes,
            &dependency_changes,
            &config_changes,
        );

        let changed_files = file_changes.clone();
        SemanticDiff {
            api_changes,
            auth_changes,
            database_changes,
            dependency_changes,
            config_changes,
            file_changes,
            changed_files,
            risk_assessment,
            summary,
        }
    }

    fn detect_api_changes(&self, diff: &str, files: &[PathBuf]) -> Vec<ApiChange> {
        let mut changes = vec![];
        let diff_lower = diff.to_lowercase();

        for file in files {
            let is_source = file
                .extension()
                .map(|e| {
                    matches!(
                        e.to_str(),
                        Some("rs")
                            | Some("py")
                            | Some("js")
                            | Some("ts")
                            | Some("go")
                            | Some("java")
                    )
                })
                .unwrap_or(false);

            if !is_source {
                continue;
            }

            // Detect function signature changes
            for pattern in &self.api_patterns {
                for cap in pattern.captures_iter(&diff) {
                    if let Some(name) = cap.get(1) {
                        let change_type = self.infer_api_change_type(&diff, name.as_str());
                        let line = self.estimate_line(diff, cap.get(0).unwrap().start());
                        let breaking = self.is_breaking_change(&diff, name.as_str());

                        changes.push(ApiChange {
                            file: file.clone(),
                            line: Some(line),
                            change_type,
                            signature: name.as_str().to_string(),
                            breaking,
                            description: format!("API {:?} for {}", change_type, name.as_str()),
                        });
                    }
                }
            }
        }

        changes
    }

    fn detect_auth_changes(&self, diff: &str, files: &[PathBuf]) -> Vec<AuthChange> {
        let mut changes = vec![];
        let diff_lower = diff.to_lowercase();

        for pattern in &self.auth_patterns {
            for cap in pattern.find_iter(&diff_lower) {
                let line = self.estimate_line(diff, cap.start());
                let matched_text = cap.as_str();
                let change_type = self.infer_auth_change_type(&diff, matched_text);
                let risk_level = self.assess_auth_risk(&diff, matched_text);
                let description = format!("Authentication/authorization change at line {}", line);

                for file in files.iter().take(1) {
                    changes.push(AuthChange {
                        file: file.clone(),
                        line: Some(line),
                        change_type,
                        description: description.clone(),
                        risk_level,
                    });
                }
            }
        }

        changes
    }

    fn detect_database_changes(&self, diff: &str, files: &[PathBuf]) -> Vec<DatabaseChange> {
        let mut changes = vec![];
        let diff_lower = diff.to_lowercase();

        for file in files {
            let is_db_related = file.to_string_lossy().to_lowercase().contains("migration")
                || file.to_string_lossy().to_lowercase().contains("schema")
                || file.extension().map(|e| e == "sql").unwrap_or(false);

            // SQL migration detection
            if is_db_related || file.extension().map(|e| e == "sql").unwrap_or(false) {
                for pattern in &self.db_patterns {
                    for cap in pattern.find_iter(&diff_lower) {
                        let line = self.estimate_line(diff, cap.start());
                        let matched_text = cap.as_str();
                        let change_type = self.infer_db_change_type(&diff, matched_text);
                        let migration_required = matches!(
                            change_type,
                            DatabaseChangeType::SchemaModified
                                | DatabaseChangeType::TableModified
                                | DatabaseChangeType::ColumnModified
                                | DatabaseChangeType::MigrationAdded
                        );
                        let breaking = matches!(
                            change_type,
                            DatabaseChangeType::TableRemoved
                                | DatabaseChangeType::ColumnRemoved
                                | DatabaseChangeType::SchemaRemoved
                        );

                        changes.push(DatabaseChange {
                            file: file.clone(),
                            line: Some(line),
                            change_type,
                            description: format!("Database {:?} detected", change_type),
                            migration_required,
                            breaking,
                        });
                    }
                }
            }
        }

        changes
    }

    fn detect_dependency_changes(&self, diff: &str, files: &[PathBuf]) -> Vec<DependencyChange> {
        let mut changes = vec![];

        for file in files {
            let is_dep_file = self
                .dep_patterns
                .iter()
                .any(|p| p.is_match(&file.to_string_lossy()));

            if is_dep_file {
                // Parse dependency changes based on file type
                let file_changes = match file.file_name().and_then(|n| n.to_str()) {
                    Some("Cargo.toml") => self.parse_cargo_toml_changes(diff),
                    Some("package.json") => self.parse_package_json_changes(diff),
                    Some("requirements.txt") => self.parse_requirements_txt_changes(diff),
                    _ => vec![],
                };

                changes.extend(file_changes);
            }
        }

        changes
    }

    fn detect_config_changes(&self, diff: &str, files: &[PathBuf]) -> Vec<ConfigChange> {
        let mut changes = vec![];
        let diff_lower = diff.to_lowercase();

        for file in files {
            let is_config = self
                .config_patterns
                .iter()
                .any(|p| p.is_match(&file.to_string_lossy()) || p.is_match(&diff_lower));

            if is_config {
                let environment = self.infer_config_environment(&diff, file.to_string_lossy().as_ref());

                // Detect key-value changes
                let key_pattern = Regex::new(r"(?i)([a-z_][a-z0-9_]*)\s*=\s*([^\n]+)").unwrap();
                for cap in key_pattern.captures_iter(&diff) {
                    if let (Some(key), Some(value)) = (cap.get(1), cap.get(2)) {
                        let change_type = self.infer_config_change_type(&diff, key.as_str());

                        changes.push(ConfigChange {
                            file: file.clone(),
                            config_key: key.as_str().to_string(),
                            old_value: None, // Would need before/after comparison
                            new_value: Some(value.as_str().to_string()),
                            change_type,
                            environment,
                        });
                    }
                }
            }
        }

        changes
    }

    fn analyze_file_changes(&self, diff: &str, files: &[PathBuf]) -> Vec<FileChange> {
        let mut changes = vec![];

        for file in files {
            let (lines_added, lines_removed) = self.count_lines_changed(diff, file);
            let change_type = self.infer_file_change_type(diff, file);
            let semantic_category = self.categorize_file(file);

            changes.push(FileChange {
                path: file.clone(),
                change_type,
                lines_added,
                lines_removed,
                semantic_category,
            });
        }

        changes
    }

    fn parse_cargo_toml_changes(&self, diff: &str) -> Vec<DependencyChange> {
        let mut changes = vec![];
        let dep_pattern =
            Regex::new(r"(?i)^[+-]\s*(\w+)\s*=\s*['\x22]?([^'\x22\n]+)['\x22]?").unwrap();

        for cap in dep_pattern.captures_iter(diff) {
            if let (Some(name), Some(version)) = (cap.get(1), cap.get(2)) {
                let line = cap.get(0).unwrap().as_str();
                let change_type = if line.starts_with('+') {
                    DependencyChangeType::Added
                } else if line.starts_with('-') {
                    DependencyChangeType::Removed
                } else {
                    DependencyChangeType::Modified
                };

                let risk_level = self.assess_dep_risk(&diff, name.as_str());

                changes.push(DependencyChange {
                    file: PathBuf::from("Cargo.toml"),
                    package_name: name.as_str().to_string(),
                    old_version: None,
                    new_version: Some(version.as_str().to_string()),
                    change_type,
                    risk_level,
                });
            }
        }

        changes
    }

    fn parse_package_json_changes(&self, diff: &str) -> Vec<DependencyChange> {
        let mut changes = vec![];
        let dep_pattern = Regex::new(r#"(?i)^[+-]\s*"([^"]+)":\s*"([^"]+)""#).unwrap();

        for cap in dep_pattern.captures_iter(diff) {
            if let (Some(name), Some(version)) = (cap.get(1), cap.get(2)) {
                let line = cap.get(0).unwrap().as_str();
                let change_type = if line.starts_with('+') {
                    DependencyChangeType::Added
                } else if line.starts_with('-') {
                    DependencyChangeType::Removed
                } else {
                    DependencyChangeType::Modified
                };

                let risk_level = self.assess_dep_risk(&diff, name.as_str());

                changes.push(DependencyChange {
                    file: PathBuf::from("package.json"),
                    package_name: name.as_str().to_string(),
                    old_version: None,
                    new_version: Some(version.as_str().to_string()),
                    change_type,
                    risk_level,
                });
            }
        }

        changes
    }

    fn parse_requirements_txt_changes(&self, diff: &str) -> Vec<DependencyChange> {
        let mut changes = vec![];
        let dep_pattern = Regex::new(r"(?i)^[+-]\s*([a-z0-9_-]+)([=<>!~]+)?([0-9.]+)?").unwrap();

        for cap in dep_pattern.captures_iter(diff) {
            if let Some(name) = cap.get(1) {
                let line = cap.get(0).unwrap().as_str();
                let change_type = if line.starts_with('+') {
                    DependencyChangeType::Added
                } else if line.starts_with('-') {
                    DependencyChangeType::Removed
                } else {
                    DependencyChangeType::Modified
                };

                let risk_level = self.assess_dep_risk(&diff, name.as_str());

                changes.push(DependencyChange {
                    file: PathBuf::from("requirements.txt"),
                    package_name: name.as_str().to_string(),
                    old_version: None,
                    new_version: cap.get(3).map(|v| v.as_str().to_string()),
                    change_type,
                    risk_level,
                });
            }
        }

        changes
    }

    
    
    
    
    
    
    
    
    
    
    fn assess_risk(
        &self,
        api: &[ApiChange],
        auth: &[AuthChange],
        db: &[DatabaseChange],
        deps: &[DependencyChange],
        config: &[ConfigChange],
    ) -> RiskAssessment {
        let api_risk = if api.iter().any(|a| a.breaking) {
            RiskLevel::High
        } else if !api.is_empty() {
            RiskLevel::Medium
        } else {
            RiskLevel::Low
        };

        let auth_risk = auth
            .iter()
            .map(|a| a.risk_level)
            .max()
            .unwrap_or(RiskLevel::None);
        let db_risk = if db.iter().any(|d| d.breaking) {
            RiskLevel::High
        } else if db.iter().any(|d| d.migration_required) {
            RiskLevel::Medium
        } else if !db.is_empty() {
            RiskLevel::Low
        } else {
            RiskLevel::None
        };

        let dep_risk = deps
            .iter()
            .map(|d| d.risk_level)
            .max()
            .unwrap_or(RiskLevel::None);
        let config_risk = if config
            .iter()
            .any(|c| c.environment == ConfigEnvironment::Production)
        {
            RiskLevel::High
        } else if !config.is_empty() {
            RiskLevel::Medium
        } else {
            RiskLevel::Low
        };

        let overall_risk = *[api_risk, auth_risk, db_risk, dep_risk, config_risk]
            .iter()
            .max()
            .unwrap_or(&RiskLevel::None);

        let requires_review = matches!(
            overall_risk,
            RiskLevel::Medium | RiskLevel::High | RiskLevel::Critical
        );
        let requires_approval = matches!(overall_risk, RiskLevel::High | RiskLevel::Critical);

        let mut reasons = vec![];
        if api_risk >= RiskLevel::Medium {
            reasons.push(format!("API changes detected with {:?} risk", api_risk));
        }
        if auth_risk >= RiskLevel::Medium {
            reasons.push(format!(
                "Authentication/authorization changes with {:?} risk",
                auth_risk
            ));
        }
        if db_risk >= RiskLevel::Medium {
            reasons.push(format!("Database changes with {:?} risk", db_risk));
        }
        if dep_risk >= RiskLevel::Medium {
            reasons.push(format!("Dependency changes with {:?} risk", dep_risk));
        }
        if config_risk >= RiskLevel::Medium {
            reasons.push(format!("Configuration changes with {:?} risk", config_risk));
        }

        let risk_reasons: Vec<crate::harness::risk::RiskReason> = reasons.into_iter().map(|s| crate::harness::risk::RiskReason {
            category: crate::harness::risk::RiskCategory::ApiBreaking,
            description: s,
            severity: crate::harness::risk::RiskSeverity::Medium,
            mitigation: None,
        }).collect();

        RiskAssessment {
            level: match overall_risk {
            RiskLevel::None => crate::harness::risk::RiskLevel::None,
            RiskLevel::Low => crate::harness::risk::RiskLevel::Low,
            RiskLevel::Medium => crate::harness::risk::RiskLevel::Medium,
            RiskLevel::High => crate::harness::risk::RiskLevel::High,
            RiskLevel::Critical => crate::harness::risk::RiskLevel::Critical,
        },
            reasons: risk_reasons,
            requires_approval,
            can_override: false,
            override_conditions: vec![],
            assessed: true,
        }
    }

    
    fn generate_summary(
        &self,
        api: &[ApiChange],
        auth: &[AuthChange],
        db: &[DatabaseChange],
        config: &[ConfigChange],
        deps: &[DependencyChange],
        files: &[FileChange],
    ) -> SemanticSummary {
        let total_files_changed = files.len();
        let total_lines_added: usize = files.iter().map(|f| f.lines_added).sum();
        let total_lines_removed: usize = files.iter().map(|f| f.lines_removed).sum();
        let breaking_changes =
            api.iter().filter(|a| a.breaking).count() + db.iter().filter(|d| d.breaking).count();
        let api_surface_changes = api.len();
        let security_relevant_changes = auth.len();
        let infrastructure_changes = db.len() + config.len() + deps.len();

        SemanticSummary {
            total_files_changed,
            total_lines_added,
            total_lines_removed,
            breaking_changes,
            api_surface_changes,
            security_relevant_changes,
            infrastructure_changes,
        }
    }

    fn estimate_line(&self, content: &str, byte_pos: usize) -> usize {
        content[..byte_pos.min(content.len())].lines().count() + 1
    }

    fn count_lines_changed(&self, diff: &str, file: &Path) -> (usize, usize) {
        let file_str = file.to_string_lossy();
        let mut added = 0;
        let mut removed = 0;
        let mut in_file_section = false;

        for line in diff.lines() {
            if line.contains(&format!("--- a/{}", file_str))
                || line.contains(&format!("+++ b/{}", file_str))
            {
                in_file_section = true;
            } else if line.starts_with("---") || line.starts_with("+++") {
                in_file_section = false;
            }

            if in_file_section {
                if line.starts_with('+') && !line.starts_with("+++") {
                    added += 1;
                } else if line.starts_with('-') && !line.starts_with("---") {
                    removed += 1;
                }
            }
        }

        (added, removed)
    }
}

pub fn analyze_semantic_diff(diff: &str) -> SemanticDiff {
    let analyzer = SemanticDiffAnalyzer::new();
    let changed_files = extract_changed_files(diff);
    analyzer.analyze(diff, &changed_files)
}

pub fn analyze_semantic_diff_with_files(diff: &str, files: &[PathBuf]) -> SemanticDiff {
    let analyzer = SemanticDiffAnalyzer::new();
    analyzer.analyze(diff, files)
}

fn extract_changed_files(diff: &str) -> Vec<PathBuf> {
    let mut files = HashSet::new();
    let file_pattern = Regex::new(r"^[+-]{3}\s+(?:[ab]/)?(.+)$").unwrap();

    for line in diff.lines() {
        if let Some(cap) = file_pattern.captures(line) {
            if let Some(file) = cap.get(1) {
                let path = file.as_str();
                if path != "/dev/null" && !path.is_empty() {
                    files.insert(PathBuf::from(path));
                }
            }
        }
    }

    files.into_iter().collect()
}

pub fn format_semantic_diff_report(diff: &SemanticDiff) -> String {
    let mut output = String::new();

    output.push_str("Semantic Diff Analysis\n");
    output.push_str("=====================\n\n");

    output.push_str("Summary:\n");
    output.push_str(&format!(
        "  Files changed: {} (+{} -{})\n",
        diff.summary.total_files_changed,
        diff.summary.total_lines_added,
        diff.summary.total_lines_removed
    ));
    output.push_str(&format!(
        "  Breaking changes: {}\n",
        diff.summary.breaking_changes
    ));
    output.push_str(&format!(
        "  API surface changes: {}\n",
        diff.summary.api_surface_changes
    ));
    output.push_str(&format!(
        "  Security-relevant changes: {}\n",
        diff.summary.security_relevant_changes
    ));
    output.push_str(&format!(
        "  Infrastructure changes: {}\n\n",
        diff.summary.infrastructure_changes
    ));

    output.push_str("Risk Assessment:\n");
    output.push_str(&format!(
        "  Overall risk: {:?}\n",
        diff.risk_assessment.level
    ));
    output.push_str(&format!(
        "  Requires approval: {:?}\n",
        diff.risk_assessment.requires_approval
    ));
    output.push_str(&format!(
        "  Can override: {:?}\n",
        diff.risk_assessment.can_override
    ));
    output.push_str(&format!(
        "  Assessed: {:?}\n",
        diff.risk_assessment.assessed
    ));
    output.push_str(&format!(
        "  Requires approval: {}\n\n",
        diff.risk_assessment.requires_approval
    ));

    if !diff.risk_assessment.reasons.is_empty() {
        output.push_str("Risk Reasons:\n");
        for reason in &diff.risk_assessment.reasons {
            output.push_str(&format!("  - {}\n", reason.description));
        }
        output.push('\n');
    }

    if !diff.api_changes.is_empty() {
        output.push_str("API Changes:\n");
        for change in &diff.api_changes {
            output.push_str(&format!(
                "  {}: {:?} {} (breaking: {})\n",
                change.file.display(),
                change.change_type,
                change.signature,
                change.breaking
            ));
        }
        output.push('\n');
    }

    if !diff.auth_changes.is_empty() {
        output.push_str("Auth Changes:\n");
        for change in &diff.auth_changes {
            output.push_str(&format!(
                "  {}: {:?} ({:?})\n",
                change.file.display(),
                change.change_type,
                change.risk_level
            ));
        }
        output.push('\n');
    }

    if !diff.database_changes.is_empty() {
        output.push_str("Database Changes:\n");
        for change in &diff.database_changes {
            output.push_str(&format!(
                "  {}: {:?} (migration: {}, breaking: {})\n",
                change.file.display(),
                change.change_type,
                change.migration_required,
                change.breaking
            ));
        }
        output.push('\n');
    }

    if !diff.dependency_changes.is_empty() {
        output.push_str("Dependency Changes:\n");
        for change in &diff.dependency_changes {
            output.push_str(&format!(
                "  {}: {} {:?} ({:?})\n",
                change.file.display(),
                change.package_name,
                change.change_type,
                change.risk_level
            ));
        }
        output.push('\n');
    }

    if !diff.config_changes.is_empty() {
        output.push_str("Config Changes:\n");
        for change in &diff.config_changes {
            output.push_str(&format!(
                "  {}: {} {:?} ({:?})\n",
                change.file.display(),
                change.config_key,
                change.change_type,
                change.environment
            ));
        }
    }

    output
}

pub fn has_breaking_changes(diff: &SemanticDiff) -> bool {
    diff.summary.breaking_changes > 0 || diff.risk_assessment.level >= crate::harness::risk::RiskLevel::High
}

pub fn requires_approval(diff: &SemanticDiff) -> bool {
    diff.risk_assessment.requires_approval
}

pub fn requires_security_review(diff: &SemanticDiff) -> bool {
    diff.risk_assessment.level >= crate::harness::risk::RiskLevel::High
        || diff.summary.security_relevant_changes > 0
        || diff.risk_assessment.level == crate::harness::risk::RiskLevel::Critical
}
