use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct EnvironmentProfile {
    pub languages: Vec<String>,
    pub package_manager: Option<String>,
    pub build_commands: Vec<String>,
    pub format_commands: Vec<String>,
    pub lint_commands: Vec<String>,
    pub test_commands: Vec<String>,
    pub services: Vec<ServiceDependency>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceDependency {
    pub name: String,
    pub required: bool,
    pub startup_command: Option<String>,
}
pub async fn fingerprint_environment(root: &Path) -> Result<EnvironmentProfile> {
    let mut p = EnvironmentProfile::default();
    if root.join("Cargo.toml").exists() {
        p.languages.push("rust".into());
        p.package_manager = Some("cargo".into());
        p.build_commands.push("cargo build".into());
        p.format_commands.push("cargo fmt --check".into());
        p.test_commands.push("cargo test".into())
    }
    if root.join("package.json").exists() {
        p.languages.push("javascript".into());
        p.package_manager.get_or_insert("npm".into());
        p.test_commands.push("npm test".into())
    }
    Ok(p)
}
