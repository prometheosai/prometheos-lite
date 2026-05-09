use prometheos_lite::harness::ci_enforcement::{
    AntiPlaceholderCI, CIConfig, CustomPattern, Severity,
};
use std::fs;
use tempfile::TempDir;

#[test]
fn test_ci_enforcement_basic() {
    let dir = TempDir::new().unwrap();
    let file_path = dir.path().join("test.rs");

    fs::write(
        &file_path,
        r#"
fn main() {
    let placeholder_var = "test";
    TODO: implement this
    unimplemented!();
    println!("debug");
}
"#,
    )
    .unwrap();

    let ci = AntiPlaceholderCI::with_defaults().unwrap();
    let result = ci.check_repository(dir.path()).unwrap();

    assert!(!result.passed);
    assert!(result.total_violations > 0);
    assert_eq!(result.files_checked, 1);
}

#[test]
fn test_exclude_patterns() {
    let mut config = CIConfig::default();
    config
        .exclude_patterns
        .push("test_violations.rs".to_string());

    let dir = TempDir::new().unwrap();
    let test_file = dir.path().join("test_violations.rs");
    fs::write(&test_file, "TODO: this should be excluded").unwrap();

    let ci = AntiPlaceholderCI::new(config).unwrap();
    let result = ci.check_repository(dir.path()).unwrap();

    assert!(result.passed);
    assert_eq!(result.files_checked, 0);
}

#[test]
fn test_custom_patterns() {
    let mut config = CIConfig::default();
    config.custom_patterns.push(CustomPattern {
        name: "custom_test".to_string(),
        pattern: r"\bCUSTOM_PATTERN\b".to_string(),
        severity: Severity::High,
        description: "Custom pattern violation".to_string(),
        suggestion: "Fix the custom pattern".to_string(),
    });

    let dir = TempDir::new().unwrap();
    let file_path = dir.path().join("test.rs");
    fs::write(&file_path, "let x = CUSTOM_PATTERN;").unwrap();

    let ci = AntiPlaceholderCI::new(config).unwrap();
    let result = ci.check_repository(dir.path()).unwrap();

    assert!(!result.passed);
    assert_eq!(result.total_violations, 1);
    assert_eq!(result.violations[0].pattern_name, "custom_test");
}

#[test]
fn test_severity_filtering() {
    let mut config = CIConfig::default();
    config.strict_mode = false;
    config.max_todo_comments = 1;

    let dir = TempDir::new().unwrap();
    let file_path = dir.path().join("test.rs");
    fs::write(
        &file_path,
        r#"
fn main() {
    // TODO: first todo
    // TODO: second todo
}
"#,
    )
    .unwrap();

    let ci = AntiPlaceholderCI::new(config).unwrap();
    let result = ci.check_repository(dir.path()).unwrap();

    assert!(!result.passed);
}

#[test]
fn test_disable_unimplemented_check() {
    let mut config = CIConfig::default();
    config.check_unimplemented = false;
    config.check_panics = false;
    config.check_debug_prints = false;
    config.strict_mode = true;

    let dir = TempDir::new().unwrap();
    let file_path = dir.path().join("test.rs");
    fs::write(&file_path, "fn f() { unimplemented!(); }").unwrap();

    let ci = AntiPlaceholderCI::new(config).unwrap();
    let result = ci.check_repository(dir.path()).unwrap();

    assert!(result.passed);
    assert!(result
        .violations
        .iter()
        .all(|v| v.pattern_name != "unimplemented"));
}

#[test]
fn test_disable_debug_print_check() {
    let mut config = CIConfig::default();
    config.check_debug_prints = false;
    config.check_panics = false;
    config.check_unimplemented = false;
    config.strict_mode = true;

    let dir = TempDir::new().unwrap();
    let file_path = dir.path().join("test.rs");
    fs::write(&file_path, "fn f() { println!(\"debug\"); }").unwrap();

    let ci = AntiPlaceholderCI::new(config).unwrap();
    let result = ci.check_repository(dir.path()).unwrap();

    assert!(result.passed);
    assert!(result
        .violations
        .iter()
        .all(|v| v.pattern_name != "debug_print"));
}
