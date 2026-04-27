//! Path utility functions for DB, config, and file paths

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Get the default database path
pub fn default_db_path() -> PathBuf {
    PathBuf::from("prometheos.db")
}

/// Get the default memory database path
pub fn default_memory_db_path() -> PathBuf {
    PathBuf::from("prometheos_memory.db")
}

/// Get the default config file path
pub fn default_config_path() -> PathBuf {
    PathBuf::from("prometheos.config.json")
}

/// Ensure a directory exists, creating it if necessary
pub fn ensure_dir_exists(path: &Path) -> Result<()> {
    if !path.exists() {
        std::fs::create_dir_all(path)
            .with_context(|| format!("Failed to create directory: {}", path.display()))?;
    }
    Ok(())
}

/// Ensure a parent directory exists for a file path
pub fn ensure_parent_dir_exists(file_path: &Path) -> Result<()> {
    if let Some(parent) = file_path.parent() {
        ensure_dir_exists(parent)?;
    }
    Ok(())
}

/// Join paths safely, handling absolute paths correctly
pub fn join_paths(base: &Path, relative: &Path) -> PathBuf {
    if relative.is_absolute() {
        relative.to_path_buf()
    } else {
        base.join(relative)
    }
}

/// Normalize a path to use forward slashes (useful for cross-platform)
pub fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Get the file extension without the dot
pub fn get_extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|s| s.to_string())
}

/// Check if a path has a specific extension (case-insensitive)
pub fn has_extension(path: &Path, ext: &str) -> bool {
    get_extension(path)
        .map(|e| e.eq_ignore_ascii_case(ext))
        .unwrap_or(false)
}

/// Create a temporary file path with a given prefix
pub fn temp_file_path(prefix: &str) -> PathBuf {
    let temp_dir = std::env::temp_dir();
    let uuid = super::ids::generate_short_uuid();
    temp_dir.join(format!("{}_{}", prefix, uuid))
}

/// Get the current working directory as a PathBuf
pub fn current_dir() -> Result<PathBuf> {
    std::env::current_dir().context("Failed to get current directory")
}

/// Resolve a path relative to the current working directory
pub fn resolve_path(path: &Path) -> Result<PathBuf> {
    let current = current_dir()?;
    Ok(join_paths(&current, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_paths() {
        assert_eq!(default_db_path(), PathBuf::from("prometheos.db"));
        assert_eq!(
            default_memory_db_path(),
            PathBuf::from("prometheos_memory.db")
        );
        assert_eq!(
            default_config_path(),
            PathBuf::from("prometheos.config.json")
        );
    }

    #[test]
    fn test_join_paths() {
        let base = PathBuf::from("/base");
        let relative = PathBuf::from("subdir/file.txt");
        let joined = join_paths(&base, &relative);
        assert_eq!(joined, PathBuf::from("/base/subdir/file.txt"));

        let absolute = PathBuf::from("/absolute/path");
        let joined_abs = join_paths(&base, &absolute);
        assert_eq!(joined_abs, PathBuf::from("/absolute/path"));
    }

    #[test]
    fn test_get_extension() {
        let path = PathBuf::from("file.txt");
        assert_eq!(get_extension(&path), Some("txt".to_string()));

        let no_ext = PathBuf::from("file");
        assert_eq!(get_extension(&no_ext), None);
    }

    #[test]
    fn test_has_extension() {
        let path = PathBuf::from("file.JSON");
        assert!(has_extension(&path, "json"));
        assert!(!has_extension(&path, "txt"));
    }

    #[test]
    fn test_normalize_path() {
        let path = PathBuf::from("subdir\\file.txt");
        let normalized = normalize_path(&path);
        assert_eq!(normalized, "subdir/file.txt");
    }
}
