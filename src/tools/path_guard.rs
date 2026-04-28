//! Path guard for enforcing safe file path operations

use anyhow::{Context, Result};
use std::path::Path;

/// Path guard for validating file paths are safe
pub struct PathGuard {
    /// Allowed base directory (e.g., "prometheos-output")
    base_dir: String,
}

impl PathGuard {
    /// Create a new PathGuard with the specified base directory
    pub fn new(base_dir: String) -> Self {
        Self { base_dir }
    }

    /// Create a PathGuard with the default prometheos-output directory
    pub fn default() -> Self {
        Self::new("prometheos-output".to_string())
    }

    /// Validate a file path is safe
    ///
    /// Rules:
    /// - No absolute paths
    /// - No `..` traversal
    /// - No symlink escape
    /// - Canonical final path must remain inside base directory
    pub fn validate_path(&self, file_path: &str) -> Result<String> {
        // Reject absolute paths (Unix-style)
        if file_path.starts_with('/') {
            anyhow::bail!("Absolute paths not allowed: {}", file_path);
        }

        // Reject Windows-style absolute paths (e.g., C:\)
        if file_path.contains(':') && file_path.len() > 2 {
            let chars: Vec<char> = file_path.chars().collect();
            if chars[1] == ':' {
                anyhow::bail!("Absolute paths not allowed: {}", file_path);
            }
        }

        // Reject parent directory traversal
        if file_path.contains("..") {
            anyhow::bail!("Parent directory traversal (..) not allowed: {}", file_path);
        }

        // Ensure base directory exists
        std::fs::create_dir_all(&self.base_dir)
            .context("Failed to create base directory")?;

        // Build full path inside base directory
        let full_path = format!("{}/{}", self.base_dir, file_path);

        // Canonicalize the path to resolve any symlinks and normalize separators
        let canonical_path = Path::new(&full_path)
            .canonicalize()
            .context("Failed to canonicalize path")?;

        // Ensure canonicalized path stays inside base directory
        let base_canonical = Path::new(&self.base_dir)
            .canonicalize()
            .context("Failed to canonicalize base directory")?;

        if !canonical_path.starts_with(&base_canonical) {
            anyhow::bail!(
                "Path outside base directory not allowed: {}",
                canonical_path.display()
            );
        }

        Ok(canonical_path.display().to_string())
    }

    /// Check if a path is safe without canonicalizing (for pre-validation)
    pub fn is_safe_path(&self, file_path: &str) -> bool {
        // Quick checks without filesystem operations
        if file_path.starts_with('/') {
            return false;
        }

        if file_path.contains(':') && file_path.len() > 2 {
            let chars: Vec<char> = file_path.chars().collect();
            if chars[1] == ':' {
                return false;
            }
        }

        if file_path.contains("..") {
            return false;
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_guard_rejects_absolute_unix() {
        let guard = PathGuard::default();
        let result = guard.validate_path("/etc/passwd");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Absolute paths not allowed"));
    }

    #[test]
    fn test_path_guard_rejects_absolute_windows() {
        let guard = PathGuard::default();
        let result = guard.validate_path("C:\\Windows\\System32");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Absolute paths not allowed"));
    }

    #[test]
    fn test_path_guard_rejects_traversal() {
        let guard = PathGuard::default();
        let result = guard.validate_path("../../secret");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Parent directory traversal"));
    }

    #[test]
    fn test_path_guard_accepts_safe_path() {
        let guard = PathGuard::default();
        // Quick check doesn't require filesystem
        assert!(guard.is_safe_path("safe/output.txt"));
    }

    #[test]
    fn test_path_guard_accepts_nested_path() {
        let guard = PathGuard::default();
        // Quick check doesn't require filesystem
        assert!(guard.is_safe_path("subdir/nested/file.txt"));
    }

    #[test]
    fn test_is_safe_path_quick_check() {
        let guard = PathGuard::default();
        assert!(!guard.is_safe_path("/etc/passwd"));
        assert!(!guard.is_safe_path("C:\\Windows"));
        assert!(!guard.is_safe_path("../../secret"));
        assert!(guard.is_safe_path("safe/output.txt"));
    }
}
