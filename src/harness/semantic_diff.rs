use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SemanticDiff {
    pub api_changes: bool,
    pub auth_changes: bool,
    pub database_changes: bool,
    pub dependency_changes: bool,
    pub config_changes: bool,
    pub changed_files: Vec<String>,
}
pub fn analyze_semantic_diff(diff: &str) -> SemanticDiff {
    let l = diff.to_lowercase();
    SemanticDiff {
        api_changes: l.contains("api"),
        auth_changes: l.contains("auth") || l.contains("token"),
        database_changes: l.contains("schema"),
        dependency_changes: l.contains("cargo.toml") || l.contains("package.json"),
        config_changes: l.contains("config"),
        changed_files: vec![],
    }
}
