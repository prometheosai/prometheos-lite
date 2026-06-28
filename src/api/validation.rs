//! API Request Validation and Hardening
//!
//! P2: Public API hardening - input validation, sanitization, and limits

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// API validation error
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    pub field: String,
    pub message: String,
    pub code: String,
}

impl ValidationError {
    pub fn new(
        field: impl Into<String>,
        message: impl Into<String>,
        code: impl Into<String>,
    ) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
            code: code.into(),
        }
    }
}

impl IntoResponse for ValidationError {
    fn into_response(self) -> Response {
        let body = Json(serde_json::json!({
            "error": "validation_error",
            "field": self.field,
            "message": self.message,
            "code": self.code,
        }));
        (StatusCode::BAD_REQUEST, body).into_response()
    }
}

/// Validate repository path
///
/// - Must be absolute path
/// - Must exist
/// - Must be a directory
/// - Must not contain dangerous patterns
pub fn validate_repo_path(path: &Path) -> Result<PathBuf, ValidationError> {
    // Check if path is absolute
    if !path.is_absolute() {
        return Err(ValidationError::new(
            "repo_root",
            "Repository path must be absolute",
            "PATH_NOT_ABSOLUTE",
        ));
    }

    // Canonicalize to resolve any symlinks and verify existence
    let canonical = path.canonicalize().map_err(|_| {
        ValidationError::new(
            "repo_root",
            "Repository path does not exist or is not accessible",
            "PATH_NOT_FOUND",
        )
    })?;

    // Verify it's a directory
    if !canonical.is_dir() {
        return Err(ValidationError::new(
            "repo_root",
            "Repository path must be a directory",
            "PATH_NOT_DIRECTORY",
        ));
    }

    // Check for dangerous patterns (path traversal)
    let path_str = canonical.to_string_lossy();
    if path_str.contains("..") || path_str.contains("~") {
        return Err(ValidationError::new(
            "repo_root",
            "Path contains invalid characters",
            "PATH_INVALID_CHARS",
        ));
    }

    Ok(canonical)
}

/// Validate work context title
///
/// - Must not be empty
/// - Must not exceed 100 characters
/// - Must not contain control characters
pub fn validate_work_context_title(title: &str) -> Result<String, ValidationError> {
    if title.trim().is_empty() {
        return Err(ValidationError::new(
            "title",
            "Title cannot be empty",
            "TITLE_EMPTY",
        ));
    }

    if title.len() > 100 {
        return Err(ValidationError::new(
            "title",
            "Title cannot exceed 100 characters",
            "TITLE_TOO_LONG",
        ));
    }

    if title.chars().any(|c| c.is_control()) {
        return Err(ValidationError::new(
            "title",
            "Title cannot contain control characters",
            "TITLE_INVALID_CHARS",
        ));
    }

    Ok(title.trim().to_string())
}

/// Validate task description
///
/// - Must not be empty
/// - Must not exceed 10000 characters
pub fn validate_task(task: &str) -> Result<String, ValidationError> {
    if task.trim().is_empty() {
        return Err(ValidationError::new(
            "task",
            "Task cannot be empty",
            "TASK_EMPTY",
        ));
    }

    if task.len() > 10000 {
        return Err(ValidationError::new(
            "task",
            "Task cannot exceed 10000 characters",
            "TASK_TOO_LONG",
        ));
    }

    Ok(task.trim().to_string())
}

/// Validate file path for edit operations
///
/// - Must be relative (no leading /)
/// - Must not contain .. for path traversal
/// - Must not contain null bytes
pub fn validate_edit_file_path(path: &str) -> Result<String, ValidationError> {
    if path.is_empty() {
        return Err(ValidationError::new(
            "file",
            "File path cannot be empty",
            "FILE_EMPTY",
        ));
    }

    // Check for absolute paths
    if path.starts_with('/') || path.starts_with("\\") || path.contains(":\\") {
        return Err(ValidationError::new(
            "file",
            "File path must be relative to repository root",
            "FILE_ABSOLUTE",
        ));
    }

    // Check for path traversal
    if path.contains("..") {
        return Err(ValidationError::new(
            "file",
            "File path cannot contain parent directory references",
            "FILE_TRAVERSAL",
        ));
    }

    // Check for null bytes
    if path.contains('\0') {
        return Err(ValidationError::new(
            "file",
            "File path cannot contain null bytes",
            "FILE_NULL_BYTES",
        ));
    }

    Ok(path.to_string())
}

/// Sanitize user input to prevent injection attacks
pub fn sanitize_input(input: &str) -> String {
    input
        .replace(['\0', '\x1b'], "") // Remove escape sequences
        .trim()
        .to_string()
}

/// Rate limiter for API endpoints
#[derive(Debug, Clone)]
pub struct RateLimiter {
    max_requests: u32,
    window_seconds: u64,
}

impl RateLimiter {
    pub fn new(max_requests: u32, window_seconds: u64) -> Self {
        Self {
            max_requests,
            window_seconds,
        }
    }

    pub fn check_rate_limit(&self, request_count: u32, window_start: std::time::Instant) -> bool {
        let elapsed = window_start.elapsed().as_secs();
        if elapsed > self.window_seconds {
            // Window has reset
            true
        } else {
            request_count < self.max_requests
        }
    }
}

/// API limits configuration
#[derive(Debug, Clone)]
pub struct ApiLimits {
    pub max_concurrent_harness_runs: usize,
    pub max_edits_per_request: usize,
    pub max_file_size_bytes: usize,
    pub max_request_size_bytes: usize,
}

impl Default for ApiLimits {
    fn default() -> Self {
        Self {
            max_concurrent_harness_runs: 5,
            max_edits_per_request: 100,
            max_file_size_bytes: 10 * 1024 * 1024,    // 10MB
            max_request_size_bytes: 50 * 1024 * 1024, // 50MB
        }
    }
}

impl ApiLimits {
    pub fn validate_edits_count(&self, count: usize) -> Result<(), ValidationError> {
        if count > self.max_edits_per_request {
            return Err(ValidationError::new(
                "edits",
                format!(
                    "Cannot exceed {} edits per request",
                    self.max_edits_per_request
                ),
                "EDITS_LIMIT_EXCEEDED",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_repo_path_empty() {
        let result = validate_repo_path(Path::new(""));
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_edit_file_path_relative() {
        let result = validate_edit_file_path("src/main.rs");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_edit_file_path_absolute() {
        let result = validate_edit_file_path("/etc/passwd");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, "FILE_ABSOLUTE");
    }

    #[test]
    fn test_validate_edit_file_path_traversal() {
        let result = validate_edit_file_path("../etc/passwd");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, "FILE_TRAVERSAL");
    }

    #[test]
    fn test_sanitize_input() {
        let input = "hello\0world\x1b[31m";
        let sanitized = sanitize_input(input);
        assert_eq!(sanitized, "helloworld[31m");
    }

    #[test]
    fn test_validate_work_context_title_empty() {
        let result = validate_work_context_title("");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_work_context_title_too_long() {
        let result = validate_work_context_title(&"a".repeat(101));
        assert!(result.is_err());
    }

    #[test]
    fn test_api_limits_validate_edits() {
        let limits = ApiLimits::default();
        assert!(limits.validate_edits_count(50).is_ok());
        assert!(limits.validate_edits_count(200).is_err());
    }
}
