//! UUID utility functions

use uuid::Uuid;

/// Generate a new UUID v4
pub fn generate_uuid() -> String {
    Uuid::new_v4().to_string()
}

/// Generate a new UUID v4 as a Uuid object
pub fn generate_uuid_v4() -> Uuid {
    Uuid::new_v4()
}

/// Parse a UUID string, returning None if invalid
pub fn parse_uuid(s: &str) -> Option<Uuid> {
    Uuid::parse_str(s).ok()
}

/// Validate a UUID string
pub fn is_valid_uuid(s: &str) -> bool {
    Uuid::parse_str(s).is_ok()
}

/// Generate a short UUID (first 8 characters)
pub fn generate_short_uuid() -> String {
    let uuid = Uuid::new_v4().to_string();
    uuid[..8].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_uuid() {
        let uuid = generate_uuid();
        assert_eq!(uuid.len(), 36);
        assert!(is_valid_uuid(&uuid));
    }

    #[test]
    fn test_parse_uuid() {
        let uuid = Uuid::new_v4().to_string();
        assert!(parse_uuid(&uuid).is_some());
        assert!(parse_uuid("invalid").is_none());
    }

    #[test]
    fn test_is_valid_uuid() {
        let uuid = Uuid::new_v4().to_string();
        assert!(is_valid_uuid(&uuid));
        assert!(!is_valid_uuid("not-a-uuid"));
    }

    #[test]
    fn test_generate_short_uuid() {
        let short = generate_short_uuid();
        assert_eq!(short.len(), 8);
    }
}
