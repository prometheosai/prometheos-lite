//! String utility functions

/// Truncate a string to a maximum length, adding ellipsis if truncated
pub fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

/// Check if a string is empty or only whitespace
pub fn is_blank(s: &str) -> bool {
    s.trim().is_empty()
}

/// Convert a string to snake_case
pub fn to_snake_case(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_uppercase() {
                format!("_{}", c.to_lowercase().collect::<String>())
            } else if c == ' ' || c == '-' {
                "_".to_string()
            } else {
                c.to_string()
            }
        })
        .collect::<String>()
        .trim_start_matches('_')
        .to_string()
}

/// Convert a string to camelCase
pub fn to_camel_case(s: &str) -> String {
    let parts: Vec<&str> = s.split('_').collect();
    parts
        .iter()
        .enumerate()
        .map(|(i, part)| {
            if i == 0 {
                part.to_lowercase()
            } else {
                let mut chars = part.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 8), "hello...");
        assert_eq!(truncate("", 5), "");
    }

    #[test]
    fn test_is_blank() {
        assert!(is_blank(""));
        assert!(is_blank("   "));
        assert!(is_blank("\t\n"));
        assert!(!is_blank("hello"));
        assert!(!is_blank("  hello  "));
    }

    #[test]
    fn test_to_snake_case() {
        assert_eq!(to_snake_case("HelloWorld"), "hello_world");
        assert_eq!(to_snake_case("hello-world"), "hello_world");
        assert_eq!(to_snake_case("hello world"), "hello_world");
        assert_eq!(to_snake_case("hello"), "hello");
    }

    #[test]
    fn test_to_camel_case() {
        assert_eq!(to_camel_case("hello_world"), "helloWorld");
        assert_eq!(to_camel_case("hello"), "hello");
        assert_eq!(to_camel_case("hello_world_test"), "helloWorldTest");
    }
}
