//! Interrupt context for human approval and resumable execution

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// InterruptContext - represents a point where human approval is needed
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
    /// WorkContext identifier (V1.2: for WorkContext integration)
    pub work_context_id: Option<String>,
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
            work_context_id: None,
            created_at: Utc::now(),
        }
    }

    /// Create a new interrupt context with WorkContext association
    pub fn new_with_work_context(
        run_id: String,
        trace_id: String,
        node_id: String,
        reason: String,
        expected_schema: serde_json::Value,
        work_context_id: String,
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
            work_context_id: Some(work_context_id),
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
            return Err(format!(
                "Cannot approve interrupt in status: {:?}",
                self.status
            ));
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
            return Err(format!(
                "Cannot deny interrupt in status: {:?}",
                self.status
            ));
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
        // Schema validation is currently enforced through required field checks.
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

    /// Persist this interrupt to the database
    pub fn persist(&self) -> Result<(), String> {
        let db_path = ".prometheos/runs.db";
        if !std::path::Path::new(db_path).exists() {
            return Err("Database not found".to_string());
        }

        let db = crate::db::repository::Db::new(db_path)
            .map_err(|e| format!("Failed to open database: {}", e))?;

        use crate::db::repository::InterruptOperations;
        let _ = InterruptOperations::create_interrupt(
            &db,
            &self.run_id,
            &self.trace_id,
            &self.node_id,
            &self.reason,
            &self.expected_schema.to_string(),
            self.work_context_id.as_deref(),
        )
        .map_err(|e| format!("Failed to persist interrupt: {}", e))?;

        Ok(())
    }

    /// Load an interrupt from the database by ID
    pub fn load(interrupt_id: &str) -> Result<Option<Self>, String> {
        let db_path = ".prometheos/runs.db";
        if !std::path::Path::new(db_path).exists() {
            return Err("Database not found".to_string());
        }

        let db = crate::db::repository::Db::new(db_path)
            .map_err(|e| format!("Failed to open database: {}", e))?;

        use crate::db::repository::InterruptOperations;
        let entry = InterruptOperations::get_interrupt(&db, interrupt_id)
            .map_err(|e| format!("Failed to load interrupt: {}", e))?;

        entry
            .map(|e| {
                let expected_schema: serde_json::Value =
                    serde_json::from_str(&e.expected_schema).unwrap_or(serde_json::Value::Null);
                let decision: Option<serde_json::Value> =
                    e.decision.and_then(|d| serde_json::from_str(&d).ok());

                Ok(Self {
                    interrupt_id: e.id,
                    run_id: e.run_id,
                    trace_id: e.trace_id,
                    node_id: e.node_id,
                    reason: e.reason,
                    expected_schema,
                    expires_at: e.expires_at,
                    status: match e.status.as_str() {
                        "pending" => InterruptStatus::Pending,
                        "approved" => InterruptStatus::Approved,
                        "denied" => InterruptStatus::Denied,
                        "expired" => InterruptStatus::Expired,
                        _ => InterruptStatus::Pending,
                    },
                    decision,
                    work_context_id: e.work_context_id,
                    created_at: e.created_at,
                })
            })
            .transpose()
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
