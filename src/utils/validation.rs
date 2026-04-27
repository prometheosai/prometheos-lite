//! Shared validation and guard functions

use anyhow::{Result, bail};

/// Validate that a string is not empty or only whitespace
pub fn validate_non_blank(value: &str, field_name: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{} cannot be empty or blank", field_name);
    }
    Ok(())
}

/// Validate that a string has a minimum length
pub fn validate_min_length(value: &str, min_len: usize, field_name: &str) -> Result<()> {
    if value.len() < min_len {
        bail!("{} must be at least {} characters", field_name, min_len);
    }
    Ok(())
}

/// Validate that a string has a maximum length
pub fn validate_max_length(value: &str, max_len: usize, field_name: &str) -> Result<()> {
    if value.len() > max_len {
        bail!("{} must be at most {} characters", field_name, max_len);
    }
    Ok(())
}

/// Validate that a string is within a length range
pub fn validate_length_range(value: &str, min: usize, max: usize, field_name: &str) -> Result<()> {
    validate_min_length(value, min, field_name)?;
    validate_max_length(value, max, field_name)?;
    Ok(())
}

/// Validate that a number is within a range
pub fn validate_range<T: PartialOrd + std::fmt::Debug>(
    value: T,
    min: T,
    max: T,
    field_name: &str,
) -> Result<()> {
    if value < min || value > max {
        bail!("{} must be between {:?} and {:?}", field_name, min, max);
    }
    Ok(())
}

/// Validate that a number is positive
pub fn validate_positive(value: i64, field_name: &str) -> Result<()> {
    if value <= 0 {
        bail!("{} must be positive, got {}", field_name, value);
    }
    Ok(())
}

/// Validate that a number is non-negative
pub fn validate_non_negative(value: i64, field_name: &str) -> Result<()> {
    if value < 0 {
        bail!("{} must be non-negative, got {}", field_name, value);
    }
    Ok(())
}

/// Validate that a URL string looks like a valid URL
pub fn validate_url(value: &str, field_name: &str) -> Result<()> {
    if !value.starts_with("http://") && !value.starts_with("https://") {
        bail!(
            "{} must be a valid URL starting with http:// or https://",
            field_name
        );
    }
    Ok(())
}

/// Validate that an email string looks like a valid email
pub fn validate_email(value: &str, field_name: &str) -> Result<()> {
    if !value.contains('@') || !value.contains('.') {
        bail!("{} must be a valid email address", field_name);
    }
    Ok(())
}

/// Validate that a UUID string is valid
pub fn validate_uuid(value: &str, field_name: &str) -> Result<()> {
    if !super::ids::is_valid_uuid(value) {
        bail!("{} must be a valid UUID", field_name);
    }
    Ok(())
}

/// Guard: return early if value is None
pub fn guard_some<T>(value: Option<T>, error_msg: String) -> Result<T> {
    value.ok_or_else(|| anyhow::anyhow!(error_msg))
}

/// Guard: return early if condition is false
pub fn guard_true(condition: bool, error_msg: String) -> Result<()> {
    if !condition {
        bail!("{}", error_msg);
    }
    Ok(())
}

/// Guard: return early if condition is true
pub fn guard_false(condition: bool, error_msg: String) -> Result<()> {
    if condition {
        bail!("{}", error_msg);
    }
    Ok(())
}

/// Validate that a collection is not empty
pub fn validate_not_empty<T>(collection: &[T], field_name: &str) -> Result<()> {
    if collection.is_empty() {
        bail!("{} cannot be empty", field_name);
    }
    Ok(())
}

/// Validate that a collection has a maximum size
pub fn validate_max_size<T>(collection: &[T], max_size: usize, field_name: &str) -> Result<()> {
    if collection.len() > max_size {
        bail!("{} cannot have more than {} items", field_name, max_size);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_non_blank() {
        assert!(validate_non_blank("hello", "test").is_ok());
        assert!(validate_non_blank("   ", "test").is_err());
        assert!(validate_non_blank("", "test").is_err());
    }

    #[test]
    fn test_validate_min_length() {
        assert!(validate_min_length("hello", 3, "test").is_ok());
        assert!(validate_min_length("hi", 3, "test").is_err());
    }

    #[test]
    fn test_validate_max_length() {
        assert!(validate_max_length("hi", 10, "test").is_ok());
        assert!(validate_max_length("hello world", 5, "test").is_err());
    }

    #[test]
    fn test_validate_range() {
        assert!(validate_range(5, 1, 10, "test").is_ok());
        assert!(validate_range(0, 1, 10, "test").is_err());
        assert!(validate_range(11, 1, 10, "test").is_err());
    }

    #[test]
    fn test_validate_url() {
        assert!(validate_url("http://example.com", "test").is_ok());
        assert!(validate_url("https://example.com", "test").is_ok());
        assert!(validate_url("ftp://example.com", "test").is_err());
        assert!(validate_url("example.com", "test").is_err());
    }

    #[test]
    fn test_validate_email() {
        assert!(validate_email("test@example.com", "test").is_ok());
        assert!(validate_email("invalid", "test").is_err());
        assert!(validate_email("test@", "test").is_err());
    }

    #[test]
    fn test_guard_some() {
        assert!(guard_some(Some(42), "error".to_string()).is_ok());
        assert!(guard_some::<i32>(None, "error".to_string()).is_err());
    }

    #[test]
    fn test_guard_true() {
        assert!(guard_true(true, "error".to_string()).is_ok());
        assert!(guard_true(false, "error".to_string()).is_err());
    }

    #[test]
    fn test_validate_not_empty() {
        assert!(validate_not_empty(&[1, 2, 3], "test").is_ok());
        assert!(validate_not_empty::<i32>(&[], "test").is_err());
    }
}
