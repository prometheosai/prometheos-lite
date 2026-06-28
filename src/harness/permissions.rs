//! Permissions Ledger - Issue #31
//! Fine-grained permission control for sandbox operations

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Permission {
    Read,
    Write,
    Execute,
    Delete,
    Create,
    Network,
    Admin,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum PermissionScope {
    File,
    Directory,
    Command,
    Network,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PermissionGrant {
    pub permission: Permission,
    pub scope: PermissionScope,
    pub path: Option<PathBuf>,
    pub pattern: Option<String>,
    pub allow: bool,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct PermissionLedger {
    grants: Vec<PermissionGrant>,
    denied_operations: Vec<DeniedOperation>,
    check_count: u64,
    deny_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DeniedOperation {
    permission: Permission,
    path: Option<PathBuf>,
    timestamp: chrono::DateTime<chrono::Utc>,
    reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PermissionCheck {
    pub granted: bool,
    pub permission: Permission,
    pub matched_grant: Option<PermissionGrant>,
    pub reason: String,
}

impl Default for PermissionLedger {
    fn default() -> Self {
        Self::new()
    }
}

impl PermissionLedger {
    pub fn new() -> Self {
        Self {
            grants: Vec::new(),
            denied_operations: Vec::new(),
            check_count: 0,
            deny_count: 0,
        }
    }

    pub fn with_defaults() -> Self {
        let mut ledger = Self::new();

        // Default safe permissions
        ledger.grant(PermissionGrant {
            permission: Permission::Read,
            scope: PermissionScope::File,
            path: None,
            pattern: Some(r"^[^/]*$".to_string()), // Current directory only
            allow: true,
            reason: "Read project files".to_string(),
        });

        ledger.grant(PermissionGrant {
            permission: Permission::Write,
            scope: PermissionScope::File,
            path: None,
            pattern: Some(r"^[^/]*$".to_string()),
            allow: true,
            reason: "Write project files".to_string(),
        });

        ledger.grant(PermissionGrant {
            permission: Permission::Execute,
            scope: PermissionScope::Command,
            path: None,
            pattern: Some(r"^(cargo|rustc|git|python|node|npm|yarn|deno)$".to_string()),
            allow: true,
            reason: "Safe build tools".to_string(),
        });

        ledger
    }

    pub fn grant(&mut self, grant: PermissionGrant) {
        self.grants.push(grant);
    }

    pub fn deny(&mut self, permission: Permission, path: Option<PathBuf>, reason: String) {
        self.denied_operations.push(DeniedOperation {
            permission,
            path,
            timestamp: chrono::Utc::now(),
            reason,
        });
        self.deny_count += 1;
    }

    pub fn check(&mut self, permission: Permission, path: &Path) -> PermissionCheck {
        self.check_count += 1;

        for grant in &self.grants {
            if grant.permission == permission {
                let matches = match &grant.path {
                    Some(p) => path.starts_with(p),
                    None => match &grant.pattern {
                        Some(pattern) => {
                            let path_str = path.to_string_lossy();
                            regex::Regex::new(pattern)
                                .map(|re| re.is_match(&path_str))
                                .unwrap_or(false)
                        }
                        None => true,
                    },
                };

                if matches {
                    return PermissionCheck {
                        granted: grant.allow,
                        permission,
                        matched_grant: Some(grant.clone()),
                        reason: grant.reason.clone(),
                    };
                }
            }
        }

        // No matching grant found - deny by default
        self.deny(
            permission,
            Some(path.to_path_buf()),
            "No matching permission grant".to_string(),
        );

        PermissionCheck {
            granted: false,
            permission,
            matched_grant: None,
            reason: "Permission not granted".to_string(),
        }
    }

    pub fn check_command(&mut self, command: &str) -> PermissionCheck {
        self.check_count += 1;
        let cmd = command.split_whitespace().next().unwrap_or(command);

        for grant in &self.grants {
            if grant.permission == Permission::Execute && grant.scope == PermissionScope::Command {
                let allowed = match &grant.pattern {
                    Some(pattern) => regex::Regex::new(pattern)
                        .map(|re| re.is_match(cmd))
                        .unwrap_or(false),
                    None => false,
                };

                if allowed {
                    return PermissionCheck {
                        granted: grant.allow,
                        permission: Permission::Execute,
                        matched_grant: Some(grant.clone()),
                        reason: grant.reason.clone(),
                    };
                }
            }
        }

        self.deny(
            Permission::Execute,
            None,
            format!("Command '{}' not allowed", cmd),
        );

        PermissionCheck {
            granted: false,
            permission: Permission::Execute,
            matched_grant: None,
            reason: format!("Command '{}' not in allowed list", cmd),
        }
    }

    pub fn require(&mut self, permission: Permission, path: &Path) -> Result<()> {
        let check = self.check(permission, path);
        if check.granted {
            Ok(())
        } else {
            bail!(
                "Permission denied: {} for {}",
                format_permission(permission),
                path.display()
            )
        }
    }

    pub fn get_stats(&self) -> PermissionStats {
        PermissionStats {
            total_grants: self.grants.len(),
            total_denied: self.denied_operations.len(),
            check_count: self.check_count,
            deny_count: self.deny_count,
            allow_rate: if self.check_count > 0 {
                (self.check_count - self.deny_count) as f64 / self.check_count as f64
            } else {
                1.0
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionStats {
    pub total_grants: usize,
    pub total_denied: usize,
    pub check_count: u64,
    pub deny_count: u64,
    pub allow_rate: f64,
}

fn format_permission(p: Permission) -> &'static str {
    match p {
        Permission::Read => "read",
        Permission::Write => "write",
        Permission::Execute => "execute",
        Permission::Delete => "delete",
        Permission::Create => "create",
        Permission::Network => "network",
        Permission::Admin => "admin",
    }
}

pub fn create_restrictive_ledger() -> PermissionLedger {
    PermissionLedger::new() // No grants - all operations denied
}

pub fn create_standard_ledger() -> PermissionLedger {
    PermissionLedger::with_defaults()
}

pub fn create_permissive_ledger() -> PermissionLedger {
    let mut ledger = PermissionLedger::new();

    // Allow all file operations
    ledger.grant(PermissionGrant {
        permission: Permission::Read,
        scope: PermissionScope::File,
        path: None,
        pattern: None,
        allow: true,
        reason: "Allow all reads".to_string(),
    });

    ledger.grant(PermissionGrant {
        permission: Permission::Write,
        scope: PermissionScope::File,
        path: None,
        pattern: None,
        allow: true,
        reason: "Allow all writes".to_string(),
    });

    ledger.grant(PermissionGrant {
        permission: Permission::Execute,
        scope: PermissionScope::Command,
        path: None,
        pattern: None,
        allow: true,
        reason: "Allow all commands".to_string(),
    });

    ledger
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permission_check_granted() {
        let mut ledger = PermissionLedger::with_defaults();
        let check = ledger.check(Permission::Read, Path::new("test.rs"));
        assert!(check.granted);
    }

    #[test]
    fn test_permission_check_denied() {
        let mut ledger = PermissionLedger::new();
        let check = ledger.check(Permission::Read, Path::new("test.rs"));
        assert!(!check.granted);
    }

    #[test]
    fn test_command_check() {
        let mut ledger = PermissionLedger::with_defaults();
        let check = ledger.check_command("cargo build");
        assert!(check.granted);

        let check = ledger.check_command("rm -rf /");
        assert!(!check.granted);
    }

    #[test]
    fn test_permission_stats() {
        let mut ledger = PermissionLedger::with_defaults();
        ledger.check(Permission::Read, Path::new("test.rs"));
        ledger.check(Permission::Delete, Path::new("/etc/passwd"));

        let stats = ledger.get_stats();
        assert_eq!(stats.check_count, 2);
        assert_eq!(stats.deny_count, 1);
    }
}
