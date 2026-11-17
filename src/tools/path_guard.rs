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

    /// Validate a file path is safe
    ///
    /// Rules:
    /// - No absolute paths
    /// - No `..` traversal
    /// - No symlink escape (including the final component and every ancestor)
    /// - Canonical resolved path must remain inside the base directory
    ///
    /// Non-existent targets are supported: the deepest existing ancestor is
    /// canonicalized and checked instead, so nested fresh-directory writes
    /// are not rejected merely because the parent does not exist yet.
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

        // Reject NUL and non-printable control characters
        if file_path.chars().any(|c| c.is_control()) {
            anyhow::bail!("Control characters not allowed in paths: {:?}", file_path);
        }

        // Ensure base directory exists
        std::fs::create_dir_all(&self.base_dir).context("Failed to create base directory")?;

        // Build full path inside base directory
        let base_canonical = Path::new(&self.base_dir)
            .canonicalize()
            .context("Failed to canonicalize base directory")?;
        let full_path = std::path::PathBuf::from(&self.base_dir).join(file_path);

        // Canonicalize the deepest existing ancestor (which may be the target
        // itself), and verify it stays inside the canonical base directory.
        let mut ancestor = full_path.as_path();
        loop {
            match ancestor.symlink_metadata() {
                Ok(_) => break,
                Err(_) => match ancestor.parent() {
                    Some(parent) => ancestor = parent,
                    None => break,
                },
            }
        }
        let canonical_ancestor = ancestor
            .canonicalize()
            .context("Failed to canonicalize ancestor directory")?;
        if !canonical_ancestor.starts_with(&base_canonical) {
            anyhow::bail!(
                "Path outside base directory not allowed: {}",
                canonical_ancestor.display()
            );
        }

        // If the final component itself exists (file or symlink), resolve it
        // too and require it to stay inside the canonical base directory.
        if let Ok(_) = full_path.symlink_metadata()
            && let Ok(canonical_full) = full_path.canonicalize()
            && !canonical_full.starts_with(&base_canonical)
        {
            anyhow::bail!(
                "Path escapes base directory through a symlink: {}",
                canonical_full.display()
            );
        }

        // Return the full path (not canonicalized since file may not exist yet)
        Ok(full_path.to_string_lossy().into_owned())
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

impl Default for PathGuard {
    fn default() -> Self {
        Self::new("prometheos-output".to_string())
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
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Absolute paths not allowed")
        );
    }

    #[test]
    fn test_path_guard_rejects_absolute_windows() {
        let guard = PathGuard::default();
        let result = guard.validate_path("C:\\Windows\\System32");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Absolute paths not allowed")
        );
    }

    #[test]
    fn test_path_guard_rejects_traversal() {
        let guard = PathGuard::default();
        let result = guard.validate_path("../../secret");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Parent directory traversal")
        );
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
