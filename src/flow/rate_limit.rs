//! Rate Limiting - token budgeting and execution guardrails

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::flow::{Node, NodeConfig, NodeId, SharedState};

/// Rate limit configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    pub max_tokens_per_minute: u32,
    pub max_tokens_per_hour: u32,
    pub max_requests_per_minute: u32,
    pub max_requests_per_hour: u32,
    pub max_execution_time_ms: u64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_tokens_per_minute: 10_000,
            max_tokens_per_hour: 100_000,
            max_requests_per_minute: 30,
            max_requests_per_hour: 500,
            max_execution_time_ms: 300_000, // 5 minutes
        }
    }
}

/// Token usage record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub timestamp: DateTime<Utc>,
    pub tokens: u32,
    pub node_id: Option<NodeId>,
}

/// Request record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestRecord {
    pub timestamp: DateTime<Utc>,
    pub node_id: Option<NodeId>,
}

/// Rate limiter for token budgeting and request limiting
pub struct RateLimiter {
    config: RateLimitConfig,
    token_usage: Vec<TokenUsage>,
    requests: Vec<RequestRecord>,
    current_execution_start: Option<DateTime<Utc>>,
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            config,
            token_usage: Vec::new(),
            requests: Vec::new(),
            current_execution_start: None,
        }
    }

    pub fn with_default() -> Self {
        Self::new(RateLimitConfig::default())
    }

    /// Check if a request is allowed based on rate limits
    pub fn check_request_allowed(&mut self, node_id: Option<NodeId>) -> Result<()> {
        let now = Utc::now();
        
        // Clean old records
        self.clean_old_records(&now);
        
        // Check requests per minute
        let requests_last_minute = self.requests.iter()
            .filter(|r| r.timestamp > now - chrono::Duration::minutes(1))
            .count();
        
        if requests_last_minute >= self.config.max_requests_per_minute as usize {
            anyhow::bail!("Rate limit exceeded: too many requests per minute");
        }
        
        // Check requests per hour
        let requests_last_hour = self.requests.iter()
            .filter(|r| r.timestamp > now - chrono::Duration::hours(1))
            .count();
        
        if requests_last_hour >= self.config.max_requests_per_hour as usize {
            anyhow::bail!("Rate limit exceeded: too many requests per hour");
        }
        
        // Record the request
        self.requests.push(RequestRecord {
            timestamp: now,
            node_id,
        });
        
        Ok(())
    }

    /// Check if token usage is within limits
    pub fn check_token_limit(&mut self, tokens: u32, node_id: Option<NodeId>) -> Result<()> {
        let now = Utc::now();
        
        // Clean old records
        self.clean_old_records(&now);
        
        // Calculate current token usage
        let tokens_last_minute: u32 = self.token_usage.iter()
            .filter(|t| t.timestamp > now - chrono::Duration::minutes(1))
            .map(|t| t.tokens)
            .sum();
        
        if tokens_last_minute + tokens > self.config.max_tokens_per_minute {
            anyhow::bail!("Rate limit exceeded: token budget per minute exceeded");
        }
        
        let tokens_last_hour: u32 = self.token_usage.iter()
            .filter(|t| t.timestamp > now - chrono::Duration::hours(1))
            .map(|t| t.tokens)
            .sum();
        
        if tokens_last_hour + tokens > self.config.max_tokens_per_hour {
            anyhow::bail!("Rate limit exceeded: token budget per hour exceeded");
        }
        
        // Record the token usage
        self.token_usage.push(TokenUsage {
            timestamp: now,
            tokens,
            node_id,
        });
        
        Ok(())
    }

    /// Start tracking execution time
    pub fn start_execution(&mut self) {
        self.current_execution_start = Some(Utc::now());
    }

    /// Check if execution time is within limits
    pub fn check_execution_time(&self) -> Result<()> {
        if let Some(start) = self.current_execution_start {
            let elapsed = Utc::now() - start;
            let elapsed_ms = elapsed.num_milliseconds() as u64;
            
            if elapsed_ms > self.config.max_execution_time_ms {
                anyhow::bail!("Execution time limit exceeded: {}ms > {}ms", 
                    elapsed_ms, self.config.max_execution_time_ms);
            }
        }
        Ok(())
    }

    /// Stop tracking execution time
    pub fn stop_execution(&mut self) {
        self.current_execution_start = None;
    }

    /// Clean old records outside the time windows
    fn clean_old_records(&mut self, now: &DateTime<Utc>) {
        let one_hour_ago = *now - chrono::Duration::hours(1);
        
        self.token_usage.retain(|t| t.timestamp > one_hour_ago);
        self.requests.retain(|r| r.timestamp > one_hour_ago);
    }

    /// Get current token usage statistics
    pub fn get_token_stats(&self) -> TokenStats {
        let now = Utc::now();
        
        let tokens_last_minute: u32 = self.token_usage.iter()
            .filter(|t| t.timestamp > now - chrono::Duration::minutes(1))
            .map(|t| t.tokens)
            .sum();
        
        let tokens_last_hour: u32 = self.token_usage.iter()
            .filter(|t| t.timestamp > now - chrono::Duration::hours(1))
            .map(|t| t.tokens)
            .sum();
        
        TokenStats {
            tokens_last_minute,
            tokens_last_hour,
            max_tokens_per_minute: self.config.max_tokens_per_minute,
            max_tokens_per_hour: self.config.max_tokens_per_hour,
        }
    }

    /// Get current request statistics
    pub fn get_request_stats(&self) -> RequestStats {
        let now = Utc::now();
        
        let requests_last_minute = self.requests.iter()
            .filter(|r| r.timestamp > now - chrono::Duration::minutes(1))
            .count();
        
        let requests_last_hour = self.requests.iter()
            .filter(|r| r.timestamp > now - chrono::Duration::hours(1))
            .count();
        
        RequestStats {
            requests_last_minute,
            requests_last_hour,
            max_requests_per_minute: self.config.max_requests_per_minute,
            max_requests_per_hour: self.config.max_requests_per_hour,
        }
    }

    /// Reset all rate limits
    pub fn reset(&mut self) {
        self.token_usage.clear();
        self.requests.clear();
        self.current_execution_start = None;
    }
}

/// Token usage statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenStats {
    pub tokens_last_minute: u32,
    pub tokens_last_hour: u32,
    pub max_tokens_per_minute: u32,
    pub max_tokens_per_hour: u32,
}

/// Request statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestStats {
    pub requests_last_minute: usize,
    pub requests_last_hour: usize,
    pub max_requests_per_minute: u32,
    pub max_requests_per_hour: u32,
}

/// Shared rate limiter for use across the application
pub type SharedRateLimiter = Arc<Mutex<RateLimiter>>;

/// Create a new shared rate limiter
pub fn create_rate_limiter(config: RateLimitConfig) -> SharedRateLimiter {
    Arc::new(Mutex::new(RateLimiter::new(config)))
}

/// Create a shared rate limiter with default configuration
pub fn create_default_rate_limiter() -> SharedRateLimiter {
    Arc::new(Mutex::new(RateLimiter::with_default()))
}

/// Rate-limiting node wrapper
pub struct RateLimitedNode {
    inner: Arc<dyn Node>,
    rate_limiter: SharedRateLimiter,
    id: String,
    estimated_tokens: u32,
}

impl RateLimitedNode {
    pub fn new(inner: Arc<dyn Node>, rate_limiter: SharedRateLimiter, estimated_tokens: u32) -> Self {
        let id = format!("rate_limited_{}", inner.id());
        Self {
            inner,
            rate_limiter,
            id,
            estimated_tokens,
        }
    }
}

#[async_trait::async_trait]
impl Node for RateLimitedNode {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        // Check rate limits before execution
        if let Ok(mut limiter) = self.rate_limiter.lock() {
            limiter.check_request_allowed(Some(self.inner.id()))?;
            limiter.check_token_limit(self.estimated_tokens, Some(self.inner.id()))?;
            limiter.start_execution();
        }
        
        self.inner.prep(state)
    }

    async fn exec(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        // Check execution time
        if let Ok(limiter) = self.rate_limiter.lock() {
            limiter.check_execution_time()?;
        }
        
        self.inner.exec(input).await
    }

    fn post(&self, state: &mut SharedState, output: serde_json::Value) -> String {
        // Stop execution tracking
        if let Ok(mut limiter) = self.rate_limiter.lock() {
            limiter.stop_execution();
        }
        
        self.inner.post(state, output)
    }

    fn config(&self) -> crate::flow::NodeConfig {
        self.inner.config()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::{Node, NodeConfig};
    use async_trait::async_trait;
    use std::sync::Arc;

    struct TestNode {
        id: String,
    }

    impl TestNode {
        fn new(id: String) -> Self {
            Self { id }
        }
    }

    #[async_trait]
    impl Node for TestNode {
        fn id(&self) -> String {
            self.id.clone()
        }

        fn prep(&self, _state: &SharedState) -> Result<serde_json::Value> {
            Ok(serde_json::json!({}))
        }

        async fn exec(&self, _input: serde_json::Value) -> Result<serde_json::Value> {
            Ok(serde_json::json!({ "result": "success" }))
        }

        fn post(&self, _state: &mut SharedState, _output: serde_json::Value) -> String {
            "continue".to_string()
        }

        fn config(&self) -> NodeConfig {
            NodeConfig::default()
        }
    }

    #[test]
    fn test_rate_limiter_request_limit() {
        let config = RateLimitConfig {
            max_requests_per_minute: 2,
            ..Default::default()
        };
        
        let mut limiter = RateLimiter::new(config);
        
        // First request should succeed
        assert!(limiter.check_request_allowed(Some("node1".to_string())).is_ok());
        
        // Second request should succeed
        assert!(limiter.check_request_allowed(Some("node1".to_string())).is_ok());
        
        // Third request should fail
        assert!(limiter.check_request_allowed(Some("node1".to_string())).is_err());
    }

    #[test]
    fn test_rate_limiter_token_limit() {
        let config = RateLimitConfig {
            max_tokens_per_minute: 100,
            ..Default::default()
        };
        
        let mut limiter = RateLimiter::new(config);
        
        // First 50 tokens should succeed
        assert!(limiter.check_token_limit(50, Some("node1".to_string())).is_ok());
        
        // Next 40 tokens should succeed
        assert!(limiter.check_token_limit(40, Some("node1".to_string())).is_ok());
        
        // Next 20 tokens should fail (50 + 40 + 20 = 110 > 100)
        assert!(limiter.check_token_limit(20, Some("node1".to_string())).is_err());
    }

    #[test]
    fn test_rate_limiter_execution_time() {
        let config = RateLimitConfig {
            max_execution_time_ms: 100,
            ..Default::default()
        };
        
        let mut limiter = RateLimiter::new(config);
        
        limiter.start_execution();
        
        // Should be within limit
        assert!(limiter.check_execution_time().is_ok());
        
        // Simulate time passing
        std::thread::sleep(Duration::from_millis(150));
        
        // Should exceed limit
        assert!(limiter.check_execution_time().is_err());
    }

    #[test]
    fn test_rate_limiter_stats() {
        let config = RateLimitConfig::default();
        let mut limiter = RateLimiter::new(config);
        
        limiter.check_token_limit(100, Some("node1".to_string())).unwrap();
        limiter.check_request_allowed(Some("node1".to_string())).unwrap();
        
        let token_stats = limiter.get_token_stats();
        assert_eq!(token_stats.tokens_last_minute, 100);
        
        let request_stats = limiter.get_request_stats();
        assert_eq!(request_stats.requests_last_minute, 1);
    }

    #[test]
    fn test_rate_limiter_reset() {
        let config = RateLimitConfig {
            max_requests_per_minute: 1,
            ..Default::default()
        };
        
        let mut limiter = RateLimiter::new(config);
        
        limiter.check_request_allowed(Some("node1".to_string())).unwrap();
        assert!(limiter.check_request_allowed(Some("node1".to_string())).is_err());
        
        limiter.reset();
        assert!(limiter.check_request_allowed(Some("node1".to_string())).is_ok());
    }

    #[tokio::test]
    async fn test_rate_limited_node() {
        let inner = Arc::new(TestNode::new("test".to_string()));
        let rate_limiter = create_default_rate_limiter();
        
        let rate_limited_node = RateLimitedNode::new(inner, rate_limiter, 100);
        
        let mut state = SharedState::new();
        let input = rate_limited_node.prep(&state).unwrap();
        let output = rate_limited_node.exec(input).await.unwrap();
        
        let action = rate_limited_node.post(&mut state, output);
        assert_eq!(action, "continue");
    }
}
