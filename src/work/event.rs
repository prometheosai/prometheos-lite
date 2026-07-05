//! WorkContext event log for continuation history

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// WorkContextEvent - an event in the WorkContext lifecycle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkContextEvent {
    pub id: String,
    pub work_context_id: String,
    pub event_type: String,
    pub data: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

impl WorkContextEvent {
    /// Create a new work context event
    pub fn new(
        id: String,
        work_context_id: String,
        event_type: String,
        data: serde_json::Value,
    ) -> Self {
        Self {
            id,
            work_context_id,
            event_type,
            data,
            created_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_creation() {
        let event = WorkContextEvent::new(
            "evt-1".to_string(),
            "ctx-1".to_string(),
            "phase_transition".to_string(),
            serde_json::json!({"from": "Intake", "to": "Planning"}),
        );

        assert_eq!(event.id, "evt-1");
        assert_eq!(event.event_type, "phase_transition");
    }
}
