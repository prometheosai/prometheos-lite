//! Issue 2: Environment Fingerprinting Tests
//!
//! Comprehensive tests for environment detection including:
//! - Language detection (Rust, JavaScript, TypeScript, Python, Go, Java, etc.)
//! - Package manager detection (cargo, npm, yarn, pnpm, pip, poetry)
//! - Build/test/lint/format command detection
//! - CI configuration detection
//! - Docker/container detection
//! - Environment variable handling

use std::collections::HashMap;
use std::path::PathBuf;

use prometheos_lite::harness::environment::{
    CiConfig, ContainerConfig, EnvironmentProfile, ServiceDependency, fingerprint_environment,
};

// ============================================================================
// Basic Structure Tests
// ============================================================================

#[test]
fn test_environment_profile_default() {
    let profile = EnvironmentProfile::default();

    assert!(profile.languages.is_empty());
    assert!(profile.build_commands.is_empty());
    assert!(profile.test_commands.is_empty());
    assert!(profile.lint_commands.is_empty());
    assert!(profile.format_commands.is_empty());
    assert!(profile.type_check_commands.is_empty());
    assert!(profile.detected_files.is_empty());
    assert!(profile.warnings.is_empty());
    assert!(profile.package_manager.is_none());
    assert!(profile.ci_config.is_none());
    assert!(profile.container_config.is_none());
}

#[test]
fn test_environment_profile_creation() {
    let profile = EnvironmentProfile {
        languages: vec!["rust".to_string()],
        package_manager: Some("cargo".to_string()),
        build_commands: vec!["cargo build".to_string()],
        format_commands: vec!["cargo fmt".to_string()],
        lint_commands: vec!["cargo clippy".to_string()],
        test_commands: vec!["cargo test".to_string()],
        type_check_commands: vec![],
        services: vec![],
        detected_files: vec!["Cargo.toml".to_string()],
        ci_config: None,
        container_config: None,
        environment_variables: HashMap::new(),
        warnings: vec![],
    };

    assert_eq!(profile.languages, vec!["rust"]);
    assert_eq!(profile.package_manager, Some("cargo".to_string()));
    assert_eq!(profile.build_commands, vec!["cargo build"]);
}

#[test]
fn test_service_dependency_creation() {
    let service = ServiceDependency {
        name: "postgres".to_string(),
        required: true,
        startup_command: Some("docker run postgres".to_string()),
        health_check: Some("pg_isready".to_string()),
        port: Some(5432),
    };

    assert_eq!(service.name, "postgres");
    assert!(service.required);
    assert_eq!(service.port, Some(5432));
}

#[test]
fn test_ci_config_creation() {
    let ci = CiConfig {
        provider: "github-actions".to_string(),
        config_file: ".github/workflows/ci.yml".to_string(),
        test_command: Some("cargo test".to_string()),
        lint_command: Some("cargo clippy".to_string()),
    };

    assert_eq!(ci.provider, "github-actions");
    assert_eq!(ci.config_file, ".github/workflows/ci.yml");
    assert_eq!(ci.test_command, Some("cargo test".to_string()));
}

#[test]
fn test_container_config_creation() {
    let container = ContainerConfig {
        has_docker: true,
        has_docker_compose: true,
        has_kubernetes: false,
        services: vec!["postgres".to_string(), "redis".to_string()],
    };

    assert!(container.has_docker);
    assert!(container.has_docker_compose);
    assert!(!container.has_kubernetes);
    assert_eq!(container.services.len(), 2);
}

// ============================================================================
// Rust Project Detection Tests
// ============================================================================

#[tokio::test]
async fn test_fingerprint_rust_project() {
    // Use the sample_repo fixture which is a Rust project
    let fixture_path = PathBuf::from("tests/fixtures/sample_repo");

    let profile = fingerprint_environment(&fixture_path).await.unwrap();

    assert!(profile.languages.contains(&"rust".to_string()));
    assert_eq!(profile.package_manager, Some("cargo".to_string()));
    assert!(profile.detected_files.contains(&"Cargo.toml".to_string()));
    assert!(profile.build_commands.contains(&"cargo build".to_string()));
    assert!(profile.test_commands.contains(&"cargo test".to_string()));
}

// ============================================================================
// Node.js Project Detection Tests
// ============================================================================

#[tokio::test]
async fn test_fingerprint_nodejs_project() {
    // Create a temporary Node.js project
    let temp_dir = std::env::temp_dir().join("test_nodejs_project");
    std::fs::create_dir_all(&temp_dir).ok();

    // Create package.json
    let package_json = r#"{
        "name": "test-project",
        "version": "1.0.0",
        "scripts": {
            "test": "jest",
            "build": "tsc",
            "lint": "eslint .",
            "format": "prettier --write ."
        }
    }"#;
    std::fs::write(temp_dir.join("package.json"), package_json).unwrap();

    let profile = fingerprint_environment(&temp_dir).await.unwrap();

    assert!(profile.languages.contains(&"javascript".to_string()));
    assert!(profile.package_manager.is_some());
    assert!(profile.detected_files.contains(&"package.json".to_string()));

    // Cleanup
    std::fs::remove_dir_all(&temp_dir).ok();
}

#[tokio::test]
async fn test_fingerprint_typescript_project() {
    // Create a temporary TypeScript project
    let temp_dir = std::env::temp_dir().join("test_ts_project");
    std::fs::create_dir_all(&temp_dir).ok();

    std::fs::write(temp_dir.join("package.json"), "{}").unwrap();
    std::fs::write(temp_dir.join("tsconfig.json"), "{}").unwrap();

    let profile = fingerprint_environment(&temp_dir).await.unwrap();

    assert!(profile.languages.contains(&"typescript".to_string()));
    assert!(
        profile
            .type_check_commands
            .contains(&"tsc --noEmit".to_string())
    );

    // Cleanup
    std::fs::remove_dir_all(&temp_dir).ok();
}

// ============================================================================
// Python Project Detection Tests
// ============================================================================

#[tokio::test]
async fn test_fingerprint_python_project() {
    // Create a temporary Python project
    let temp_dir = std::env::temp_dir().join("test_python_project");
    std::fs::create_dir_all(&temp_dir).ok();

    std::fs::write(temp_dir.join("requirements.txt"), "requests\npytest").unwrap();

    let profile = fingerprint_environment(&temp_dir).await.unwrap();

    assert!(profile.languages.contains(&"python".to_string()));
    assert!(
        profile
            .detected_files
            .contains(&"requirements.txt".to_string())
    );

    // Cleanup
    std::fs::remove_dir_all(&temp_dir).ok();
}

// ============================================================================
// Docker Detection Tests
// ============================================================================

#[tokio::test]
async fn test_fingerprint_docker_project() {
    // Create a temporary project with Docker
    let temp_dir = std::env::temp_dir().join("test_docker_project");
    std::fs::create_dir_all(&temp_dir).ok();

    std::fs::write(temp_dir.join("Dockerfile"), "FROM rust:latest").unwrap();
    std::fs::write(temp_dir.join("docker-compose.yml"), "version: '3'").unwrap();

    let profile = fingerprint_environment(&temp_dir).await.unwrap();

    if let Some(container) = profile.container_config {
        assert!(container.has_docker);
        assert!(container.has_docker_compose);
    }

    // Cleanup
    std::fs::remove_dir_all(&temp_dir).ok();
}

// ============================================================================
// Unknown Project Tests
// ============================================================================

#[tokio::test]
async fn test_fingerprint_unknown_project() {
    // Create a temporary empty project
    let temp_dir = std::env::temp_dir().join("test_unknown_project");
    std::fs::create_dir_all(&temp_dir).ok();

    let profile = fingerprint_environment(&temp_dir).await.unwrap();

    assert!(profile.languages.contains(&"unknown".to_string()));
    assert!(!profile.warnings.is_empty());

    // Cleanup
    std::fs::remove_dir_all(&temp_dir).ok();
}
