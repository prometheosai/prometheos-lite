//! Interrupt context for human approval and resumable execution

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Interrupt context for pausing execution and requiring human approval
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterruptContext {
    /// Unique interrupt identifier
    pub interrupt_id: String,
    /// Run identifier
    pub run_id: String,
    /// Trace identifier
    pub trace_id: String,
    /// Node identifier
    pub node_id: String,
    /// Reason for the interrupt
    pub reason: String,
    /// Expected schema for the decision
    pub expected_schema: serde_json::Value,
    /// Optional expiration time
    pub expires_at: Option<DateTime<Utc>>,
    /// Current status of the interrupt
    pub status: InterruptStatus,
    /// Decision data (if approved/denied)
    pub decision: Option<serde_json::Value>,
    /// Timestamp when interrupt was created
    pub created_at: DateTime<Utc>,
}

/// Status of an interrupt
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InterruptStatus {
    /// Interrupt is pending approval
    Pending,
    /// Interrupt was approved
    Approved,
    /// Interrupt was denied
    Denied,
    /// Interrupt expired
    Expired,
}

impl InterruptContext {
    /// Create a new interrupt context
    pub fn new(
        run_id: String,
        trace_id: String,
        node_id: String,
        reason: String,
        expected_schema: serde_json::Value,
    ) -> Self {
        Self {
            interrupt_id: Uuid::new_v4().to_string(),
            run_id,
            trace_id,
            node_id,
            reason,
            expected_schema,
            expires_at: None,
            status: InterruptStatus::Pending,
            decision: None,
            created_at: Utc::now(),
        }
    }

    /// Set the expiration time
    pub fn with_expiration(mut self, expires_at: DateTime<Utc>) -> Self {
        self.expires_at = Some(expires_at);
        self
    }

    /// Approve the interrupt with a decision
    pub fn approve(&mut self, decision: serde_json::Value) -> Result<(), String> {
        if self.status != InterruptStatus::Pending {
            return Err(format!("Cannot approve interrupt in status: {:?}", self.status));
        }

        // Validate decision against expected schema
        if !self.validate_decision(&decision) {
            return Err("Decision does not match expected schema".to_string());
        }

        self.status = InterruptStatus::Approved;
        self.decision = Some(decision);
        Ok(())
    }

    /// Deny the interrupt
    pub fn deny(&mut self) -> Result<(), String> {
        if self.status != InterruptStatus::Pending {
            return Err(format!("Cannot deny interrupt in status: {:?}", self.status));
        }

        self.status = InterruptStatus::Denied;
        Ok(())
    }

    /// Check if the interrupt has expired
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            Utc::now() > expires_at
        } else {
            false
        }
    }

    /// Validate a decision against the expected schema
    fn validate_decision(&self, decision: &serde_json::Value) -> bool {
        // Simple validation: check if decision has the same structure as expected schema
        // In a real implementation, this would use a proper JSON schema validator
        match (&self.expected_schema, decision) {
            (serde_json::Value::Object(expected), serde_json::Value::Object(decision)) => {
                expected.keys().all(|k| decision.get(k).is_some())
            }
            (serde_json::Value::Array(_), serde_json::Value::Array(_)) => true,
            (serde_json::Value::String(_), serde_json::Value::String(_)) => true,
            (serde_json::Value::Number(_), serde_json::Value::Number(_)) => true,
            (serde_json::Value::Bool(_), serde_json::Value::Bool(_)) => true,
            (serde_json::Value::Null, serde_json::Value::Null) => true,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interrupt_context_creation() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "approved": {"type": "boolean"}
            }
        });

        let context = InterruptContext::new(
            "run1".to_string(),
            "trace1".to_string(),
            "node1".to_string(),
            "Tool execution requires approval".to_string(),
            schema,
        );

        assert_eq!(context.run_id, "run1");
        assert_eq!(context.status, InterruptStatus::Pending);
        assert!(context.decision.is_none());
    }

    #[test]
    fn test_interrupt_approve() {
        let schema = serde_json::json!({
            "approved": true
        });

        let mut context = InterruptContext::new(
            "run1".to_string(),
            "trace1".to_string(),
            "node1".to_string(),
            "Tool execution requires approval".to_string(),
            schema,
        );

        let decision = serde_json::json!({"approved": true});
        let result = context.approve(decision);

        assert!(result.is_ok());
        assert_eq!(context.status, InterruptStatus::Approved);
        assert!(context.decision.is_some());
    }

    #[test]
    fn test_interrupt_deny() {
        let schema = serde_json::json!({});

        let mut context = InterruptContext::new(
            "run1".to_string(),
            "trace1".to_string(),
            "node1".to_string(),
            "Tool execution requires approval".to_string(),
            schema,
        );

        let result = context.deny();

        assert!(result.is_ok());
        assert_eq!(context.status, InterruptStatus::Denied);
    }

    #[test]
    fn test_interrupt_expiration() {
        let schema = serde_json::json!({});

        let mut context = InterruptContext::new(
            "run1".to_string(),
            "trace1".to_string(),
            "node1".to_string(),
            "Tool execution requires approval".to_string(),
            schema,
        );

        // No expiration by default
        assert!(!context.is_expired());

        // Set expiration in the past
        let past = Utc::now() - chrono::Duration::hours(1);
        context.expires_at = Some(past);
        assert!(context.is_expired());

        // Set expiration in the future
        let future = Utc::now() + chrono::Duration::hours(1);
        context.expires_at = Some(future);
        assert!(!context.is_expired());
    }
}
