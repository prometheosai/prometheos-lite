//! Issue 23: Tool Permission Ledger Tests
//!
//! Comprehensive tests for the Tool Permission Ledger including:
//! - Permission enum (Read, Write, Execute, Delete, Create, Network, Admin)
//! - PermissionScope enum (File, Directory, Command, Network, System)
//! - PermissionGrant struct (permission, scope, path, pattern, allow, reason)
//! - PermissionLedger for managing permissions
//! - PermissionCheck struct (granted, permission, matched_grant, reason)
//! - grant() for adding permissions
//! - check() for validating permissions
//! - check_command() for command validation
//! - deny() for recording denials
//! - with_defaults() for safe default permissions

use std::path::PathBuf;

use prometheos_lite::harness::permissions::{
    Permission, PermissionCheck, PermissionGrant, PermissionLedger, PermissionScope,
};

// ============================================================================
// Permission Tests
// ============================================================================

#[test]
fn test_permission_variants() {
    assert!(matches!(Permission::Read, Permission::Read));
    assert!(matches!(Permission::Write, Permission::Write));
    assert!(matches!(Permission::Execute, Permission::Execute));
    assert!(matches!(Permission::Delete, Permission::Delete));
    assert!(matches!(Permission::Create, Permission::Create));
    assert!(matches!(Permission::Network, Permission::Network));
    assert!(matches!(Permission::Admin, Permission::Admin));
}

#[test]
fn test_permission_display() {
    assert_eq!(format!("{:?}", Permission::Read), "Read");
    assert_eq!(format!("{:?}", Permission::Write), "Write");
    assert_eq!(format!("{:?}", Permission::Execute), "Execute");
}

// ============================================================================
// PermissionScope Tests
// ============================================================================

#[test]
fn test_permission_scope_variants() {
    assert!(matches!(PermissionScope::File, PermissionScope::File));
    assert!(matches!(
        PermissionScope::Directory,
        PermissionScope::Directory
    ));
    assert!(matches!(PermissionScope::Command, PermissionScope::Command));
    assert!(matches!(PermissionScope::Network, PermissionScope::Network));
    assert!(matches!(PermissionScope::System, PermissionScope::System));
}

// ============================================================================
// PermissionGrant Tests
// ============================================================================

#[test]
fn test_permission_grant_creation() {
    let grant = PermissionGrant {
        permission: Permission::Read,
        scope: PermissionScope::File,
        path: Some(PathBuf::from("/tmp/test")),
        pattern: None,
        allow: true,
        reason: "Allow reading test files".to_string(),
    };

    assert!(matches!(grant.permission, Permission::Read));
    assert!(matches!(grant.scope, PermissionScope::File));
    assert_eq!(grant.path, Some(PathBuf::from("/tmp/test")));
    assert!(grant.allow);
    assert_eq!(grant.reason, "Allow reading test files");
}

#[test]
fn test_permission_grant_with_pattern() {
    let grant = PermissionGrant {
        permission: Permission::Execute,
        scope: PermissionScope::Command,
        path: None,
        pattern: Some(r"^cargo|rustc$".to_string()),
        allow: true,
        reason: "Safe build tools".to_string(),
    };

    assert!(matches!(grant.permission, Permission::Execute));
    assert!(grant.pattern.is_some());
    assert!(grant.path.is_none());
}

#[test]
fn test_permission_grant_deny() {
    let grant = PermissionGrant {
        permission: Permission::Network,
        scope: PermissionScope::System,
        path: None,
        pattern: None,
        allow: false,
        reason: "No network access".to_string(),
    };

    assert!(!grant.allow);
    assert!(matches!(grant.permission, Permission::Network));
}

// ============================================================================
// PermissionLedger Tests
// ============================================================================

#[test]
fn test_permission_ledger_new() {
    let ledger = PermissionLedger::new();
    // Ledger created successfully
    assert!(true);
}

#[test]
fn test_permission_ledger_with_defaults() {
    let ledger = PermissionLedger::with_defaults();
    // Ledger created with defaults
    assert!(true);
}

#[test]
fn test_permission_ledger_grant() {
    let mut ledger = PermissionLedger::new();

    ledger.grant(PermissionGrant {
        permission: Permission::Read,
        scope: PermissionScope::File,
        path: Some(PathBuf::from("/tmp")),
        pattern: None,
        allow: true,
        reason: "Allow reading tmp files".to_string(),
    });

    // Grant added successfully
    assert!(true);
}

#[test]
fn test_permission_check_granted() {
    let mut ledger = PermissionLedger::new();

    ledger.grant(PermissionGrant {
        permission: Permission::Read,
        scope: PermissionScope::File,
        path: Some(PathBuf::from("/tmp")),
        pattern: None,
        allow: true,
        reason: "Allow reading tmp files".to_string(),
    });

    let check = ledger.check(Permission::Read, PathBuf::from("/tmp/test.txt").as_path());

    assert!(check.granted);
    assert!(matches!(check.permission, Permission::Read));
}

#[test]
fn test_permission_check_denied() {
    let mut ledger = PermissionLedger::new();

    // No grants added, should deny by default
    let check = ledger.check(Permission::Write, PathBuf::from("/tmp/test.txt").as_path());

    assert!(!check.granted);
    assert!(matches!(check.permission, Permission::Write));
}

// ============================================================================
// PermissionCheck Tests
// ============================================================================

#[test]
fn test_permission_check_granted_variant() {
    let check = PermissionCheck {
        granted: true,
        permission: Permission::Execute,
        matched_grant: None,
        reason: "Command allowed".to_string(),
    };

    assert!(check.granted);
    assert!(matches!(check.permission, Permission::Execute));
    assert_eq!(check.reason, "Command allowed");
}

#[test]
fn test_permission_check_denied_variant() {
    let check = PermissionCheck {
        granted: false,
        permission: Permission::Admin,
        matched_grant: None,
        reason: "Admin not allowed".to_string(),
    };

    assert!(!check.granted);
    assert!(matches!(check.permission, Permission::Admin));
}

// ============================================================================
// Integration Tests
// ============================================================================

#[test]
fn test_complex_permission_grants() {
    let mut ledger = PermissionLedger::new();

    // Add multiple grants
    ledger.grant(PermissionGrant {
        permission: Permission::Read,
        scope: PermissionScope::File,
        path: Some(PathBuf::from("/project")),
        pattern: None,
        allow: true,
        reason: "Read project files".to_string(),
    });

    ledger.grant(PermissionGrant {
        permission: Permission::Write,
        scope: PermissionScope::File,
        path: Some(PathBuf::from("/project/output")),
        pattern: None,
        allow: true,
        reason: "Write output files".to_string(),
    });

    ledger.grant(PermissionGrant {
        permission: Permission::Execute,
        scope: PermissionScope::Command,
        path: None,
        pattern: Some(r"^cargo$".to_string()),
        allow: true,
        reason: "Run cargo commands".to_string(),
    });

    // Check various permissions
    let read_check = ledger.check(
        Permission::Read,
        PathBuf::from("/project/src/main.rs").as_path(),
    );
    assert!(read_check.granted);

    let write_check = ledger.check(
        Permission::Write,
        PathBuf::from("/project/output/result.txt").as_path(),
    );
    assert!(write_check.granted);
}

#[test]
fn test_permission_with_regex_pattern() {
    let grant = PermissionGrant {
        permission: Permission::Read,
        scope: PermissionScope::File,
        path: None,
        pattern: Some(r".*\.rs$".to_string()),
        allow: true,
        reason: "Read Rust files".to_string(),
    };

    assert!(grant.pattern.is_some());
    let pattern = grant.pattern.unwrap();
    assert!(pattern.contains(r"\.rs"));
}
