use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SemanticDiff {
    pub api_changes: Vec<ApiChange>,
    pub auth_changes: Vec<AuthChange>,
    pub database_changes: Vec<DatabaseChange>,
    pub dependency_changes: Vec<DependencyChange>,
    pub config_changes: Vec<ConfigChange>,
    pub changed_files: Vec<FileChange>,
    pub risk_assessment: RiskAssessment,
    pub summary: SemanticSummary,
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
    AuthenticationAdded,
    AuthenticationRemoved,
    AuthenticationModified,
    AuthorizationAdded,
    AuthorizationRemoved,
    AuthorizationModified,
    PermissionCheckAdded,
    PermissionCheckRemoved,
    PermissionCheckModified,
    TokenHandlingAdded,
    TokenHandlingRemoved,
    TokenHandlingModified,
    SecretAccessAdded,
    SecretAccessRemoved,
    SecretAccessModified,
    CredentialValidationAdded,
    CredentialValidationRemoved,
    CredentialValidationModified,
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
    SchemaAdded,
    SchemaRemoved,
    SchemaModified,
    MigrationAdded,
    MigrationModified,
    TableAdded,
    TableRemoved,
    TableModified,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum RiskLevel {
    None,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RiskAssessment {
    pub overall_risk: RiskLevel,
    pub api_risk: RiskLevel,
    pub auth_risk: RiskLevel,
    pub database_risk: RiskLevel,
    pub dependency_risk: RiskLevel,
    pub config_risk: RiskLevel,
    pub requires_review: bool,
    pub requires_approval: bool,
    pub reasons: Vec<String>,
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
                Regex::new(r"(?i)pub\s+use\s+").unwrap(),
                Regex::new(r"(?i)fn\s+(\w+)\s*\([^)]*\)\s*(?:->\s*[^\{]+)?\s*\{").unwrap(),
            ],
            auth_patterns: vec![
                Regex::new(r"(?i)auth").unwrap(),
                Regex::new(r"(?i)authenticat").unwrap(),
                Regex::new(r"(?i)authoriz").unwrap(),
                Regex::new(r"(?i)permission").unwrap(),
                Regex::new(r"(?i)token").unwrap(),
                Regex::new(r"(?i)jwt").unwrap(),
                Regex::new(r"(?i)oauth").unwrap(),
                Regex::new(r"(?i)session").unwrap(),
                Regex::new(r"(?i)credential").unwrap(),
                Regex::new(r"(?i)password").unwrap(),
                Regex::new(r"(?i)secret").unwrap(),
                Regex::new(r"(?i)hash").unwrap(),
                Regex::new(r"(?i)bcrypt|argon2|pbkdf2|scrypt").unwrap(),
                Regex::new(r"(?i)login|logout|signin|signout").unwrap(),
            ],
            db_patterns: vec![
                Regex::new(r"(?i)schema").unwrap(),
                Regex::new(r"(?i)migration").unwrap(),
                Regex::new(r"(?i)CREATE\s+TABLE").unwrap(),
                Regex::new(r"(?i)DROP\s+TABLE").unwrap(),
                Regex::new(r"(?i)ALTER\s+TABLE").unwrap(),
                Regex::new(r"(?i)CREATE\s+INDEX").unwrap(),
                Regex::new(r"(?i)DROP\s+INDEX").unwrap(),
                Regex::new(r"(?i)INSERT\s+INTO").unwrap(),
                Regex::new(r"(?i)UPDATE\s+").unwrap(),
                Regex::new(r"(?i)DELETE\s+FROM").unwrap(),
                Regex::new(r"(?i)SELECT\s+.*\s+FROM").unwrap(),
                Regex::new(r"(?i)\.sql").unwrap(),
                Regex::new(r"(?i)diesel::|sqlx::|sea_orm::").unwrap(),
                Regex::new(r"(?i)db\.").unwrap(),
                Regex::new(r"(?i)database|postgres|mysql|sqlite").unwrap(),
            ],
            dep_patterns: vec![
                Regex::new(r"(?i)cargo\.toml").unwrap(),
                Regex::new(r"(?i)package\.json").unwrap(),
                Regex::new(r"(?i)requirements\.txt").unwrap(),
                Regex::new(r"(?i)pyproject\.toml").unwrap(),
                Regex::new(r"(?i)go\.mod").unwrap(),
                Regex::new(r"(?i)gemfile").unwrap(),
                Regex::new(r"(?i)gemfile\.lock").unwrap(),
                Regex::new(r"(?i)composer\.json").unwrap(),
                Regex::new(r"(?i)pom\.xml").unwrap(),
                Regex::new(r"(?i)build\.gradle").unwrap(),
                Regex::new(r"(?i)\.lock$").unwrap(),
            ],
            config_patterns: vec![
                Regex::new(r"(?i)\.env").unwrap(),
                Regex::new(r"(?i)config").unwrap(),
                Regex::new(r"(?i)settings").unwrap(),
                Regex::new(r"(?i)\.toml$").unwrap(),
                Regex::new(r"(?i)\.yaml$").unwrap(),
                Regex::new(r"(?i)\.yml$").unwrap(),
                Regex::new(r"(?i)\.json$").unwrap(),
                Regex::new(r"(?i)production").unwrap(),
                Regex::new(r"(?i)staging").unwrap(),
                Regex::new(r"(?i)docker").unwrap(),
                Regex::new(r"(?i)kubernetes|k8s").unwrap(),
                Regex::new(r"(?i)\.ini$").unwrap(),
            ],
            breaking_patterns: vec![
                Regex::new(r"(?i)breaking|BREAKING").unwrap(),
                Regex::new(r"(?i)removed|deleted").unwrap(),
                Regex::new(r"(?i)renamed").unwrap(),
                Regex::new(r"(?i)changed\s+signature").unwrap(),
                Regex::new(r"(?i)deprecated").unwrap(),
                Regex::new(r"(?i)#[\s]*\[deprecated").unwrap(),
            ],
            secret_patterns: vec![
                Regex::new(r"(?i)password\s*=").unwrap(),
                Regex::new(r"(?i)secret\s*=").unwrap(),
                Regex::new(r"(?i)api_key\s*=").unwrap(),
                Regex::new(r"(?i)private_key").unwrap(),
                Regex::new(r"(?i)\.pem$").unwrap(),
                Regex::new(r"(?i)\.key$").unwrap(),
                Regex::new(r"(?i)id_rsa").unwrap(),
                Regex::new(r"(?i)id_ed25519").unwrap(),
                Regex::new(r"(?i)\.env\.local").unwrap(),
            ],
        }
    }
    
    pub fn analyze(&self, diff: &str, changed_files: &[PathBuf]) -> SemanticDiff {
        let api_changes = self.detect_api_changes(diff, changed_files);
        let auth_changes = self.detect_auth_changes(diff, changed_files);
        let database_changes = self.detect_database_changes(diff, changed_files);
        let dependency_changes = self.detect_dependency_changes(diff, changed_files);
        let config_changes = self.detect_config_changes(diff, changed_files);
        let file_changes = self.analyze_file_changes(diff, changed_files);
        
        let summary = self.generate_summary(&api_changes, &auth_changes, &database_changes, 
                                           &dependency_changes, &config_changes, &file_changes);
        let risk_assessment = self.assess_risk(&api_changes, &auth_changes, &database_changes,
                                              &dependency_changes, &config_changes);
        
        SemanticDiff {
            api_changes,
            auth_changes,
            database_changes,
            dependency_changes,
            config_changes,
            changed_files: file_changes,
            risk_assessment,
            summary,
        }
    }
    
    fn detect_api_changes(&self, diff: &str, files: &[PathBuf]) -> Vec<ApiChange> {
        let mut changes = vec![];
        let diff_lower = diff.to_lowercase();
        
        for file in files {
            let is_source = file.extension()
                .map(|e| matches!(e.to_str(), Some("rs") | Some("py") | Some("js") | Some("ts") | Some("go") | Some("java")))
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
                let change_type = self.infer_auth_change_type(&diff, cap.start());
                let risk_level = self.assess_auth_risk(&diff, cap.start());
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
            let is_db_related = file.to_string_lossy().to_lowercase().contains("migration") ||
                              file.to_string_lossy().to_lowercase().contains("schema") ||
                              file.extension().map(|e| e == "sql").unwrap_or(false);
            
            // SQL migration detection
            if is_db_related || file.extension().map(|e| e == "sql").unwrap_or(false) {
                for pattern in &self.db_patterns {
                    for cap in pattern.find_iter(&diff_lower) {
                        let line = self.estimate_line(diff, cap.start());
                        let change_type = self.infer_db_change_type(&diff, cap.start());
                        let migration_required = matches!(change_type, 
                            DatabaseChangeType::SchemaModified |
                            DatabaseChangeType::TableModified |
                            DatabaseChangeType::ColumnModified |
                            DatabaseChangeType::MigrationAdded);
                        let breaking = matches!(change_type,
                            DatabaseChangeType::TableRemoved |
                            DatabaseChangeType::ColumnRemoved |
                            DatabaseChangeType::SchemaRemoved);
                        
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
            let is_dep_file = self.dep_patterns.iter().any(|p| {
                p.is_match(&file.to_string_lossy())
            });
            
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
            let is_config = self.config_patterns.iter().any(|p| {
                p.is_match(&file.to_string_lossy()) || p.is_match(&diff_lower)
            });
            
            if is_config {
                let environment = self.infer_config_environment(&diff, file);
                
                // Detect key-value changes
                let key_pattern = Regex::new(r"(?i)([a-z_][a-z0-9_]*)\s*=\s*([^\n]+)").unwrap();
                for cap in key_pattern.captures_iter(&diff) {
                    if let (Some(key), Some(value)) = (cap.get(1), cap.get(2)) {
                        let change_type = self.infer_config_change_type(&diff, cap.start());
                        
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
        let dep_pattern = Regex::new(r"(?i)^[+-]\s*(\w+)\s*=\s*['\"]?([^'\"\n]+)['\"]?").unwrap();
        
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
                
                let risk_level = self.assess_dep_risk(name.as_str());
                
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
                
                let risk_level = self.assess_dep_risk(name.as_str());
                
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
                
                let risk_level = self.assess_dep_risk(name.as_str());
                
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
    
    fn infer_api_change_type(&self, diff: &str, name: &str) -> ApiChangeType {
        let diff_lower = diff.to_lowercase();
        let name_lower = name.to_lowercase();
        
        if diff_lower.contains(&format!("pub fn {}", name_lower)) && 
           diff_lower.contains("removed") || diff_lower.contains("-") {
            return ApiChangeType::FunctionRemoved;
        }
        if diff_lower.contains(&format!("pub fn {}", name_lower)) && 
           diff_lower.contains("added") || diff_lower.contains("+") {
            return ApiChangeType::FunctionAdded;
        }
        
        ApiChangeType::FunctionModified
    }
    
    fn infer_auth_change_type(&self, _diff: &str, _pos: usize) -> AuthChangeType {
        // Simplified inference - would need more context
        AuthChangeType::AuthenticationModified
    }
    
    fn infer_db_change_type(&self, diff: &str, _pos: usize) -> DatabaseChangeType {
        let diff_lower = diff.to_lowercase();
        
        if diff_lower.contains("create table") {
            DatabaseChangeType::TableAdded
        } else if diff_lower.contains("drop table") {
            DatabaseChangeType::TableRemoved
        } else if diff_lower.contains("alter table") {
            DatabaseChangeType::TableModified
        } else if diff_lower.contains("migration") {
            DatabaseChangeType::MigrationAdded
        } else {
            DatabaseChangeType::QueryModified
        }
    }
    
    fn infer_config_change_type(&self, diff: &str, _pos: usize) -> ConfigChangeType {
        let line = diff.lines().next().unwrap_or("");
        if line.starts_with('+') {
            ConfigChangeType::Added
        } else if line.starts_with('-') {
            ConfigChangeType::Removed
        } else {
            ConfigChangeType::Modified
        }
    }
    
    fn infer_config_environment(&self, diff: &str, file: &Path) -> ConfigEnvironment {
        let path_lower = file.to_string_lossy().to_lowercase();
        let diff_lower = diff.to_lowercase();
        
        if path_lower.contains("production") || path_lower.contains("prod") ||
           diff_lower.contains("production") {
            ConfigEnvironment::Production
        } else if path_lower.contains("development") || path_lower.contains("dev") ||
                  diff_lower.contains("development") {
            ConfigEnvironment::Development
        } else if path_lower.contains("test") || diff_lower.contains("test") {
            ConfigEnvironment::Test
        } else {
            ConfigEnvironment::Unknown
        }
    }
    
    fn infer_file_change_type(&self, diff: &str, _file: &Path) -> FileChangeType {
        if diff.starts_with("new file") || diff.starts_with("+++ /dev/null") {
            FileChangeType::Removed
        } else if diff.contains("rename from") {
            FileChangeType::Renamed
        } else if diff.contains("--- /dev/null") {
            FileChangeType::Added
        } else {
            FileChangeType::Modified
        }
    }
    
    fn categorize_file(&self, file: &Path) -> SemanticCategory {
        let path_str = file.to_string_lossy().to_lowercase();
        
        if path_str.contains("test") || path_str.contains("spec") || 
           file.file_name().map(|f| f.to_str().unwrap_or("").starts_with("test_")).unwrap_or(false) {
            SemanticCategory::Test
        } else if path_str.contains("migration") || path_str.ends_with(".sql") {
            SemanticCategory::Migration
        } else if path_str.contains("config") || self.config_patterns.iter().any(|p| p.is_match(&path_str)) {
            SemanticCategory::Configuration
        } else if path_str.contains("cargo.toml") || path_str.contains("package.json") ||
                  path_str.contains("makefile") || path_str.contains("dockerfile") {
            SemanticCategory::Build
        } else if path_str.contains(".md") || path_str.contains("readme") ||
                  path_str.contains("docs") {
            SemanticCategory::Documentation
        } else if self.secret_patterns.iter().any(|p| p.is_match(&path_str)) {
            SemanticCategory::Secret
        } else if file.extension().map(|e| matches!(e.to_str(), Some("rs") | Some("py") | Some("js") | Some("ts"))).unwrap_or(false) {
            SemanticCategory::SourceCode
        } else {
            SemanticCategory::Unknown
        }
    }
    
    fn is_breaking_change(&self, diff: &str, _name: &str) -> bool {
        let diff_lower = diff.to_lowercase();
        
        self.breaking_patterns.iter().any(|p| p.is_match(&diff_lower)) ||
        diff_lower.contains("removed") ||
        diff_lower.contains("signature")
    }
    
    fn assess_auth_risk(&self, diff: &str, _pos: usize) -> RiskLevel {
        let diff_lower = diff.to_lowercase();
        
        if self.secret_patterns.iter().any(|p| p.is_match(&diff_lower)) {
            RiskLevel::Critical
        } else if diff_lower.contains("password") || diff_lower.contains("secret") {
            RiskLevel::High
        } else if diff_lower.contains("token") || diff_lower.contains("auth") {
            RiskLevel::Medium
        } else {
            RiskLevel::Low
        }
    }
    
    fn assess_dep_risk(&self, name: &str) -> RiskLevel {
        let name_lower = name.to_lowercase();
        
        // High-risk dependency categories
        let high_risk = ["crypto", "auth", "security", "openssl", "tls", "ssl"];
        let medium_risk = ["http", "web", "server", "net"];
        
        if high_risk.iter().any(|r| name_lower.contains(r)) {
            RiskLevel::High
        } else if medium_risk.iter().any(|r| name_lower.contains(r)) {
            RiskLevel::Medium
        } else {
            RiskLevel::Low
        }
    }
    
    fn assess_risk(&self, api: &[ApiChange], auth: &[AuthChange], db: &[DatabaseChange],
                   deps: &[DependencyChange], config: &[ConfigChange]) -> RiskAssessment {
        let api_risk = if api.iter().any(|a| a.breaking) {
            RiskLevel::High
        } else if !api.is_empty() {
            RiskLevel::Medium
        } else {
            RiskLevel::Low
        };
        
        let auth_risk = auth.iter().map(|a| a.risk_level).max().unwrap_or(RiskLevel::None);
        let db_risk = if db.iter().any(|d| d.breaking) {
            RiskLevel::High
        } else if db.iter().any(|d| d.migration_required) {
            RiskLevel::Medium
        } else if !db.is_empty() {
            RiskLevel::Low
        } else {
            RiskLevel::None
        };
        
        let dep_risk = deps.iter().map(|d| d.risk_level).max().unwrap_or(RiskLevel::None);
        let config_risk = if config.iter().any(|c| c.environment == ConfigEnvironment::Production) {
            RiskLevel::High
        } else if !config.is_empty() {
            RiskLevel::Medium
        } else {
            RiskLevel::Low
        };
        
        let overall_risk = *[api_risk, auth_risk, db_risk, dep_risk, config_risk]
            .iter().max().unwrap_or(&RiskLevel::None);
        
        let requires_review = matches!(overall_risk, RiskLevel::Medium | RiskLevel::High | RiskLevel::Critical);
        let requires_approval = matches!(overall_risk, RiskLevel::High | RiskLevel::Critical);
        
        let mut reasons = vec![];
        if api_risk >= RiskLevel::Medium {
            reasons.push(format!("API changes detected with {:?} risk", api_risk));
        }
        if auth_risk >= RiskLevel::Medium {
            reasons.push(format!("Authentication/authorization changes with {:?} risk", auth_risk));
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
        
        RiskAssessment {
            overall_risk,
            api_risk,
            auth_risk,
            database_risk,
            dependency_risk,
            config_risk,
            requires_review,
            requires_approval,
            reasons,
        }
    }
    
    fn generate_summary(&self, api: &[ApiChange], auth: &[AuthChange], db: &[DatabaseChange],
                       deps: &[DependencyChange], config: &[ConfigChange], files: &[FileChange]) -> SemanticSummary {
        let total_files_changed = files.len();
        let total_lines_added: usize = files.iter().map(|f| f.lines_added).sum();
        let total_lines_removed: usize = files.iter().map(|f| f.lines_removed).sum();
        let breaking_changes = api.iter().filter(|a| a.breaking).count() +
                              db.iter().filter(|d| d.breaking).count();
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
            if line.contains(&format!("--- a/{}", file_str)) || 
               line.contains(&format!("+++ b/{}", file_str)) {
                in_file_section = true;
            } else if line.starts_with("---") || line.starts_with("+++")) {
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
    output.push_str(&format!("  Files changed: {} (+{} -{})\n", 
        diff.summary.total_files_changed,
        diff.summary.total_lines_added,
        diff.summary.total_lines_removed));
    output.push_str(&format!("  Breaking changes: {}\n", diff.summary.breaking_changes));
    output.push_str(&format!("  API surface changes: {}\n", diff.summary.api_surface_changes));
    output.push_str(&format!("  Security-relevant changes: {}\n", diff.summary.security_relevant_changes));
    output.push_str(&format!("  Infrastructure changes: {}\n\n", diff.summary.infrastructure_changes));
    
    output.push_str("Risk Assessment:\n");
    output.push_str(&format!("  Overall risk: {:?}\n", diff.risk_assessment.overall_risk));
    output.push_str(&format!("  API risk: {:?}\n", diff.risk_assessment.api_risk));
    output.push_str(&format!("  Auth risk: {:?}\n", diff.risk_assessment.auth_risk));
    output.push_str(&format!("  Database risk: {:?}\n", diff.risk_assessment.database_risk));
    output.push_str(&format!("  Dependency risk: {:?}\n", diff.risk_assessment.dependency_risk));
    output.push_str(&format!("  Config risk: {:?}\n", diff.risk_assessment.config_risk));
    output.push_str(&format!("  Requires review: {}\n", diff.risk_assessment.requires_review));
    output.push_str(&format!("  Requires approval: {}\n\n", diff.risk_assessment.requires_approval));
    
    if !diff.risk_assessment.reasons.is_empty() {
        output.push_str("Risk Reasons:\n");
        for reason in &diff.risk_assessment.reasons {
            output.push_str(&format!("  - {}\n", reason));
        }
        output.push('\n');
    }
    
    if !diff.api_changes.is_empty() {
        output.push_str("API Changes:\n");
        for change in &diff.api_changes {
            output.push_str(&format!("  {}: {:?} {} (breaking: {})\n",
                change.file.display(),
                change.change_type,
                change.signature,
                change.breaking));
        }
        output.push('\n');
    }
    
    if !diff.auth_changes.is_empty() {
        output.push_str("Auth Changes:\n");
        for change in &diff.auth_changes {
            output.push_str(&format!("  {}: {:?} ({:?})\n",
                change.file.display(),
                change.change_type,
                change.risk_level));
        }
        output.push('\n');
    }
    
    if !diff.database_changes.is_empty() {
        output.push_str("Database Changes:\n");
        for change in &diff.database_changes {
            output.push_str(&format!("  {}: {:?} (migration: {}, breaking: {})\n",
                change.file.display(),
                change.change_type,
                change.migration_required,
                change.breaking));
        }
        output.push('\n');
    }
    
    if !diff.dependency_changes.is_empty() {
        output.push_str("Dependency Changes:\n");
        for change in &diff.dependency_changes {
            output.push_str(&format!("  {}: {} {:?} ({:?})\n",
                change.file.display(),
                change.package_name,
                change.change_type,
                change.risk_level));
        }
        output.push('\n');
    }
    
    if !diff.config_changes.is_empty() {
        output.push_str("Config Changes:\n");
        for change in &diff.config_changes {
            output.push_str(&format!("  {}: {} {:?} ({:?})\n",
                change.file.display(),
                change.config_key,
                change.change_type,
                change.environment));
        }
    }
    
    output
}

pub fn has_breaking_changes(diff: &SemanticDiff) -> bool {
    diff.summary.breaking_changes > 0 || 
    diff.risk_assessment.overall_risk >= RiskLevel::High
}

pub fn requires_approval(diff: &SemanticDiff) -> bool {
    diff.risk_assessment.requires_approval
}

pub fn requires_security_review(diff: &SemanticDiff) -> bool {
    diff.risk_assessment.auth_risk >= RiskLevel::High ||
    diff.summary.security_relevant_changes > 0 ||
    diff.risk_assessment.overall_risk == RiskLevel::Critical
}
