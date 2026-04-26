//! Policy Hooks - pre/post validation and constitution-policy enforcement

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::flow::{Action, Input, Node, NodeConfig, Output, SharedState};

/// Policy violation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyViolation {
    pub rule_id: String,
    pub description: String,
    pub severity: PolicySeverity,
    pub node_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PolicySeverity {
    Warning,
    Error,
    Critical,
}

/// Policy rule
pub trait PolicyRule: Send + Sync {
    fn id(&self) -> String;
    fn description(&self) -> String;
    fn validate_pre(&self, state: &SharedState, input: &Input) -> Result<(), PolicyViolation>;
    fn validate_post(&self, state: &SharedState, output: &Output) -> Result<(), PolicyViolation>;
}

/// Constitution policy for enforcing safety and alignment
pub struct ConstitutionPolicy {
    rules: Vec<Arc<dyn PolicyRule>>,
    enabled: bool,
}

impl ConstitutionPolicy {
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            enabled: true,
        }
    }

    pub fn add_rule(&mut self, rule: Arc<dyn PolicyRule>) {
        self.rules.push(rule);
    }

    pub fn remove_rule(&mut self, rule_id: &str) {
        self.rules.retain(|r| r.id() != rule_id);
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Validate before node execution
    pub fn validate_pre(&self, state: &SharedState, input: &Input) -> Result<Vec<PolicyViolation>> {
        if !self.enabled {
            return Ok(Vec::new());
        }

        let mut violations = Vec::new();
        for rule in &self.rules {
            if let Err(violation) = rule.validate_pre(state, input) {
                violations.push(violation);
            }
        }
        Ok(violations)
    }

    /// Validate after node execution
    pub fn validate_post(
        &self,
        state: &SharedState,
        output: &Output,
    ) -> Result<Vec<PolicyViolation>> {
        if !self.enabled {
            return Ok(Vec::new());
        }

        let mut violations = Vec::new();
        for rule in &self.rules {
            if let Err(violation) = rule.validate_post(state, output) {
                violations.push(violation);
            }
        }
        Ok(violations)
    }
}

impl Default for ConstitutionPolicy {
    fn default() -> Self {
        Self::new()
    }
}

/// Policy-enforcing node wrapper
pub struct PolicyNode {
    inner: Arc<dyn Node>,
    policy: Arc<ConstitutionPolicy>,
    id: String,
    config: NodeConfig,
}

impl PolicyNode {
    pub fn new(inner: Arc<dyn Node>, policy: Arc<ConstitutionPolicy>) -> Self {
        let id = format!("policy_{}", inner.id());
        Self {
            inner,
            policy,
            id,
            config: NodeConfig::default(),
        }
    }
}

#[async_trait::async_trait]
impl Node for PolicyNode {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn prep(&self, state: &SharedState) -> Result<Input> {
        let input = self.inner.prep(state)?;

        // Pre-validation
        let violations = self
            .policy
            .validate_pre(state, &input)
            .context("Pre-policy validation failed")?;

        if !violations.is_empty() {
            let error_msg: String = violations
                .iter()
                .map(|v| format!("{}: {}", v.rule_id, v.description))
                .collect::<Vec<_>>()
                .join("; ");
            anyhow::bail!("Policy violations detected: {}", error_msg);
        }

        Ok(input)
    }

    async fn exec(&self, input: Input) -> Result<Output> {
        self.inner.exec(input).await
    }

    fn post(&self, state: &mut SharedState, output: Output) -> Action {
        // Post-validation
        match self.policy.validate_post(state, &output) {
            Ok(violations) => {
                if !violations.is_empty() {
                    // Log violations but continue with warning
                    eprintln!("Policy violations detected: {:?}", violations);
                }
            }
            Err(e) => {
                eprintln!("Post-policy validation error: {}", e);
            }
        }

        self.inner.post(state, output)
    }

    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}

/// Example policy: Input size limit
pub struct InputSizeLimitRule {
    max_size_bytes: usize,
}

impl InputSizeLimitRule {
    pub fn new(max_size_bytes: usize) -> Self {
        Self { max_size_bytes }
    }
}

impl PolicyRule for InputSizeLimitRule {
    fn id(&self) -> String {
        "input_size_limit".to_string()
    }

    fn description(&self) -> String {
        format!("Input must not exceed {} bytes", self.max_size_bytes)
    }

    fn validate_pre(&self, _state: &SharedState, input: &Input) -> Result<(), PolicyViolation> {
        let size = serde_json::to_vec(input).map(|v| v.len()).unwrap_or(0);

        if size > self.max_size_bytes {
            Err(PolicyViolation {
                rule_id: self.id(),
                description: format!("Input size {} exceeds limit {}", size, self.max_size_bytes),
                severity: PolicySeverity::Error,
                node_id: None,
            })
        } else {
            Ok(())
        }
    }

    fn validate_post(&self, _state: &SharedState, _output: &Output) -> Result<(), PolicyViolation> {
        Ok(())
    }
}

/// Example policy: Output content filter
pub struct ContentFilterRule {
    forbidden_words: Vec<String>,
}

impl ContentFilterRule {
    pub fn new(forbidden_words: Vec<String>) -> Self {
        Self { forbidden_words }
    }
}

impl PolicyRule for ContentFilterRule {
    fn id(&self) -> String {
        "content_filter".to_string()
    }

    fn description(&self) -> String {
        "Output must not contain forbidden words".to_string()
    }

    fn validate_pre(&self, _state: &SharedState, _input: &Input) -> Result<(), PolicyViolation> {
        Ok(())
    }

    fn validate_post(&self, _state: &SharedState, output: &Output) -> Result<(), PolicyViolation> {
        let output_str = serde_json::to_string(output).unwrap_or_default();

        for word in &self.forbidden_words {
            if output_str.to_lowercase().contains(&word.to_lowercase()) {
                return Err(PolicyViolation {
                    rule_id: self.id(),
                    description: format!("Output contains forbidden word: {}", word),
                    severity: PolicySeverity::Warning,
                    node_id: None,
                });
            }
        }

        Ok(())
    }
}

/// Example policy: State mutation check
pub struct StateMutationRule {
    allow_mutation: bool,
}

impl StateMutationRule {
    pub fn new(allow_mutation: bool) -> Self {
        Self { allow_mutation }
    }
}

impl PolicyRule for StateMutationRule {
    fn id(&self) -> String {
        "state_mutation".to_string()
    }

    fn description(&self) -> String {
        if self.allow_mutation {
            "State mutation is allowed".to_string()
        } else {
            "State mutation is not allowed".to_string()
        }
    }

    fn validate_pre(&self, _state: &SharedState, _input: &Input) -> Result<(), PolicyViolation> {
        Ok(())
    }

    fn validate_post(&self, state: &SharedState, _output: &Output) -> Result<(), PolicyViolation> {
        if !self.allow_mutation && !state.working.is_empty() {
            Err(PolicyViolation {
                rule_id: self.id(),
                description: "State mutation detected when not allowed".to_string(),
                severity: PolicySeverity::Error,
                node_id: None,
            })
        } else {
            Ok(())
        }
    }
}
