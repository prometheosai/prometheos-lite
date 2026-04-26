//! Time utility functions

use chrono::{DateTime, Utc, Duration};

/// Get the current timestamp as ISO 8601 string
pub fn now_iso8601() -> String {
    Utc::now().to_rfc3339()
}

/// Parse an ISO 8601 string to DateTime
pub fn parse_iso8601(s: &str) -> Result<DateTime<Utc>, chrono::ParseError> {
    DateTime::parse_from_rfc3339(s).map(|dt| dt.with_timezone(&Utc))
}

/// Format a duration in a human-readable format
pub fn format_duration(duration: Duration) -> String {
    let total_seconds = duration.num_seconds();
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;

    if hours > 0 {
        format!("{}h {}m {}s", hours, minutes, seconds)
    } else if minutes > 0 {
        format!("{}m {}s", minutes, seconds)
    } else {
        format!("{}s", seconds)
    }
}

/// Get the duration between two ISO 8601 timestamps
pub fn duration_between(start: &str, end: &str) -> Result<Duration, chrono::ParseError> {
    let start_dt = parse_iso8601(start)?;
    let end_dt = parse_iso8601(end)?;
    Ok(end_dt.signed_duration_since(start_dt).abs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_now_iso8601() {
        let s = now_iso8601();
        assert!(!s.is_empty());
        // Verify it can be parsed back
        assert!(parse_iso8601(&s).is_ok());
    }

    #[test]
    fn test_parse_iso8601() {
        let s = "2024-01-01T00:00:00Z";
        let dt = parse_iso8601(s).unwrap();
        assert_eq!(dt.year(), 2024);
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(Duration::seconds(90)), "1m 30s");
        assert_eq!(format_duration(Duration::seconds(3661)), "1h 1m 1s");
        assert_eq!(format_duration(Duration::seconds(30)), "30s");
    }

    #[test]
    fn test_duration_between() {
        let start = "2024-01-01T00:00:00Z";
        let end = "2024-01-01T01:00:00Z";
        let duration = duration_between(start, end).unwrap();
        assert_eq!(duration.num_seconds(), 3600);
    }
}
