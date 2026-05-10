//! Strict mode enforcement for flow execution
//! - No silent Option::None propagation
//! - Tool idempotency checks
//! - No silent failures

use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::config::StrictMode as StrictModeConfig;

/// Strict mode enforcement context
#[derive(Debug, Clone)]
pub struct StrictModeEnforcer {
    config: StrictModeConfig,
    /// Track tool call results for idempotency checks
    tool_call_cache: Arc<Mutex<HashMap<String, ToolCallResult>>>,
    /// Track tool outbox to prevent duplicate executions
    tool_outbox: Arc<Mutex<HashMap<String, bool>>>,
}

#[derive(Debug, Clone)]
struct ToolCallResult {
    result_hash: String,
    _timestamp: chrono::DateTime<chrono::Utc>,
}

impl StrictModeEnforcer {
    /// Create a new strict mode enforcer
    pub fn new(config: StrictModeConfig) -> Self {
        Self {
            config,
            tool_call_cache: Arc::new(Mutex::new(HashMap::new())),
            tool_outbox: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Check if strict mode is enabled
    pub fn is_enabled(&self) -> bool {
        self.config.enforce_missing_inputs
            || self.config.enforce_missing_services
            || self.config.enforce_empty_outputs
            || self.config.enforce_no_silent_none
            || self.config.enforce_idempotency
    }

    /// Validate input is present and non-empty
    pub fn validate_input(&self, input: &Value, field_name: &str) -> Result<()> {
        if !self.config.enforce_missing_inputs {
            return Ok(());
        }

        if input.is_null() {
            bail!(
                "Strict mode violation: Input '{}' is null. Missing inputs are not allowed.",
                field_name
            );
        }

        if let Some(obj) = input.as_object()
            && obj.is_empty()
        {
            bail!(
                "Strict mode violation: Input '{}' is empty object. Missing inputs are not allowed.",
                field_name
            );
        }

        if let Some(arr) = input.as_array()
            && arr.is_empty()
        {
            bail!(
                "Strict mode violation: Input '{}' is empty array. Missing inputs are not allowed.",
                field_name
            );
        }

        if let Some(s) = input.as_str()
            && s.trim().is_empty()
        {
            bail!(
                "Strict mode violation: Input '{}' is empty string. Missing inputs are not allowed.",
                field_name
            );
        }

        Ok(())
    }

    /// Validate service is available
    pub fn validate_service<T>(&self, service: Option<&T>, service_name: &str) -> Result<()> {
        if !self.config.enforce_missing_services {
            return Ok(());
        }

        if service.is_none() {
            bail!(
                "Strict mode violation: Service '{}' is not available. Missing services are not allowed.",
                service_name
            );
        }

        Ok(())
    }

    /// Validate output is non-empty
    pub fn validate_output(&self, output: &Value, output_name: &str) -> Result<()> {
        if !self.config.enforce_empty_outputs {
            return Ok(());
        }

        if output.is_null() {
            bail!(
                "Strict mode violation: Output '{}' is null. Empty outputs are not allowed.",
                output_name
            );
        }

        if let Some(obj) = output.as_object()
            && obj.is_empty()
        {
            bail!(
                "Strict mode violation: Output '{}' is empty object. Empty outputs are not allowed.",
                output_name
            );
        }

        if let Some(arr) = output.as_array()
            && arr.is_empty()
        {
            bail!(
                "Strict mode violation: Output '{}' is empty array. Empty outputs are not allowed.",
                output_name
            );
        }

        if let Some(s) = output.as_str()
            && s.trim().is_empty()
        {
            bail!(
                "Strict mode violation: Output '{}' is empty string. Empty outputs are not allowed.",
                output_name
            );
        }

        Ok(())
    }

    /// Validate Option is not None (no silent None propagation)
    pub fn validate_option<T: Clone>(&self, value: Option<&T>, value_name: &str) -> Result<T> {
        if !self.config.enforce_no_silent_none {
            return value
                .cloned()
                .context(format!("Value '{}' is None", value_name));
        }

        value.cloned().context(format!(
            "Strict mode violation: Value '{}' is None. Silent None propagation is not allowed.",
            value_name
        ))
    }

    /// Check tool idempotency - same args should produce same result
    pub fn check_tool_idempotency(
        &self,
        tool_name: &str,
        args: &Value,
        result: &Value,
    ) -> Result<()> {
        if !self.config.enforce_idempotency {
            return Ok(());
        }

        let args_hash = self.compute_hash(args);
        let result_hash = self.compute_hash(result);
        let cache_key = format!("{}:{}", tool_name, args_hash);

        let mut cache = self.tool_call_cache.lock().unwrap();

        if let Some(cached) = cache.get(&cache_key) {
            // Check if result is consistent with previous call
            if cached.result_hash != result_hash {
                bail!(
                    "Strict mode violation: Tool '{}' produced different result for same arguments. Idempotency check failed. Previous hash: {}, Current hash: {}",
                    tool_name,
                    cached.result_hash,
                    result_hash
                );
            }
        } else {
            // Cache this result
            cache.insert(
                cache_key,
                ToolCallResult {
                    result_hash,
                    _timestamp: chrono::Utc::now(),
                },
            );
        }

        Ok(())
    }

    /// Check if tool has already been executed (prevent duplicate execution)
    pub fn check_tool_outbox(&self, tool_name: &str, args: &Value) -> Result<bool> {
        if !self.config.enforce_idempotency {
            return Ok(false);
        }

        let args_hash = self.compute_hash(args);
        let outbox_key = format!("{}:{}", tool_name, args_hash);

        let mut outbox = self.tool_outbox.lock().unwrap();

        if outbox.contains_key(&outbox_key) {
            return Ok(true); // Already executed
        }

        // Mark as executed
        outbox.insert(outbox_key, true);
        Ok(false)
    }

    /// Clear tool call cache (useful for testing or when idempotency is not required)
    pub fn clear_tool_cache(&self) {
        let mut cache = self.tool_call_cache.lock().unwrap();
        cache.clear();
    }

    /// Clear tool outbox
    pub fn clear_tool_outbox(&self) {
        let mut outbox = self.tool_outbox.lock().unwrap();
        outbox.clear();
    }

    /// Compute a simple hash for JSON values
    fn compute_hash(&self, value: &Value) -> String {
        format!("{:x}", md5::compute(value.to_string().as_bytes()))
    }

    /// V1.4: Check if tool failure should stop execution
    pub fn should_stop_on_tool_failure(&self) -> bool {
        self.config.enforce_empty_outputs // Reuse existing flag for tool failure
    }

    /// V1.4: Check if invalid patch should stop execution
    pub fn should_stop_on_invalid_patch(&self) -> bool {
        self.config.enforce_empty_outputs // Reuse existing flag for invalid patch
    }

    /// V1.4: Check if test failure should trigger retry
    pub fn should_retry_on_test_failure(&self) -> bool {
        !self.config.enforce_empty_outputs // Invert for retry behavior
    }

    /// V1.4: Validate patch result in strict mode
    pub fn validate_patch_result(&self, patch_result: &Value) -> Result<()> {
        if self.should_stop_on_invalid_patch() {
            let success = patch_result
                .get("success")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let validation = patch_result
                .get("validation")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            if !success || validation == "failed" {
                bail!(
                    "Strict mode violation: Patch validation failed. Result: {:?}",
                    patch_result
                );
            }
        }

        Ok(())
    }

    /// V1.4: Validate test result in strict mode
    pub fn validate_test_result(&self, test_result: &Value) -> Result<bool> {
        let success = test_result
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if !success && self.should_stop_on_tool_failure() {
            bail!("Strict mode violation: Test execution failed");
        }

        Ok(success)
    }

    /// V1.5: Enforce no silent failures - any error should be propagated
    pub fn enforce_no_silent_failure(&self, result: Result<()>, operation: &str) -> Result<()> {
        if self.config.enforce_empty_outputs {
            result
                .with_context(|| format!("Strict mode violation: Operation '{}' failed", operation))
        } else {
            result
        }
    }

    /// V1.5: Ban unsafe unwrap - provide safe alternative
    pub fn safe_unwrap<T: Clone>(&self, value: Option<&T>, context: &str) -> Result<T> {
        self.validate_option(value, context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn create_strict_enforcer() -> StrictModeEnforcer {
        StrictModeEnforcer::new(StrictModeConfig {
            enforce_missing_inputs: true,
            enforce_missing_services: true,
            enforce_empty_outputs: true,
            enforce_no_unwrap: false,
            enforce_no_silent_none: true,
            enforce_idempotency: true,
        })
    }

    fn create_lenient_enforcer() -> StrictModeEnforcer {
        StrictModeEnforcer::new(StrictModeConfig {
            enforce_missing_inputs: false,
            enforce_missing_services: false,
            enforce_empty_outputs: false,
            enforce_no_unwrap: false,
            enforce_no_silent_none: false,
            enforce_idempotency: false,
        })
    }

    #[test]
    fn test_validate_input_strict() {
        let enforcer = create_strict_enforcer();

        // Valid input
        assert!(enforcer.validate_input(&json!("test"), "field").is_ok());
        assert!(
            enforcer
                .validate_input(&json!({"key": "value"}), "field")
                .is_ok()
        );
        assert!(enforcer.validate_input(&json!(["item"]), "field").is_ok());

        // Invalid input
        assert!(enforcer.validate_input(&json!(null), "field").is_err());
        assert!(enforcer.validate_input(&json!(""), "field").is_err());
        assert!(enforcer.validate_input(&json!({}), "field").is_err());
        assert!(enforcer.validate_input(&json!([]), "field").is_err());
    }

    #[test]
    fn test_validate_input_lenient() {
        let enforcer = create_lenient_enforcer();

        // All inputs should pass in lenient mode
        assert!(enforcer.validate_input(&json!(null), "field").is_ok());
        assert!(enforcer.validate_input(&json!(""), "field").is_ok());
        assert!(enforcer.validate_input(&json!({}), "field").is_ok());
    }

    #[test]
    fn test_validate_service_strict() {
        let enforcer = create_strict_enforcer();

        let service = "test_service";
        assert!(enforcer.validate_service(Some(&service), "service").is_ok());
        assert!(
            enforcer
                .validate_service::<String>(None, "service")
                .is_err()
        );
    }

    #[test]
    fn test_validate_service_lenient() {
        let enforcer = create_lenient_enforcer();

        assert!(enforcer.validate_service::<String>(None, "service").is_ok());
    }

    #[test]
    fn test_validate_output_strict() {
        let enforcer = create_strict_enforcer();

        assert!(enforcer.validate_output(&json!("result"), "output").is_ok());
        assert!(enforcer.validate_output(&json!(null), "output").is_err());
        assert!(enforcer.validate_output(&json!(""), "output").is_err());
    }

    #[test]
    fn test_validate_option_strict() {
        let enforcer = create_strict_enforcer();

        let value = "test";
        assert!(enforcer.validate_option(Some(&value), "value").is_ok());
        assert!(enforcer.validate_option::<String>(None, "value").is_err());
    }

    #[test]
    fn test_tool_idempotency() {
        let enforcer = create_strict_enforcer();

        let args = json!({"input": "test"});
        let result1 = json!({"output": "result1"});

        // First call should succeed
        assert!(
            enforcer
                .check_tool_idempotency("test_tool", &args, &result1)
                .is_ok()
        );

        // Same args, same result should succeed
        assert!(
            enforcer
                .check_tool_idempotency("test_tool", &args, &result1)
                .is_ok()
        );

        // Same args, different result should fail
        let result2 = json!({"output": "result2"});
        assert!(
            enforcer
                .check_tool_idempotency("test_tool", &args, &result2)
                .is_err()
        );

        // Clear cache and retry
        enforcer.clear_tool_cache();
        assert!(
            enforcer
                .check_tool_idempotency("test_tool", &args, &result2)
                .is_ok()
        );
    }
}
