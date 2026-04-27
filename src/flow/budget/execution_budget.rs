//! Execution budget for enforcing per-run resource limits

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Execution budget configuration for a single flow run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionBudget {
    /// Maximum number of node execution steps
    pub max_steps: u32,
    /// Maximum number of LLM API calls
    pub max_llm_calls: u32,
    /// Maximum number of tool calls
    pub max_tool_calls: u32,
    /// Maximum runtime in milliseconds
    pub max_runtime_ms: u64,
    /// Maximum number of memory read operations
    pub max_memory_reads: u32,
    /// Maximum number of memory write operations
    pub max_memory_writes: u32,
}

impl Default for ExecutionBudget {
    fn default() -> Self {
        Self {
            max_steps: 100,
            max_llm_calls: 50,
            max_tool_calls: 20,
            max_runtime_ms: 300_000, // 5 minutes
            max_memory_reads: 100,
            max_memory_writes: 50,
        }
    }
}

impl ExecutionBudget {
    /// Create a new ExecutionBudget with custom limits
    pub fn new(
        max_steps: u32,
        max_llm_calls: u32,
        max_tool_calls: u32,
        max_runtime_ms: u64,
        max_memory_reads: u32,
        max_memory_writes: u32,
    ) -> Self {
        Self {
            max_steps,
            max_llm_calls,
            max_tool_calls,
            max_runtime_ms,
            max_memory_reads,
            max_memory_writes,
        }
    }

    /// Create a budget with only step limit
    pub fn with_steps(max_steps: u32) -> Self {
        Self {
            max_steps,
            ..Default::default()
        }
    }

    /// Create a budget with only runtime limit
    pub fn with_runtime(max_runtime_ms: u64) -> Self {
        Self {
            max_runtime_ms,
            ..Default::default()
        }
    }

    /// Create a budget with only LLM call limit
    pub fn with_llm_calls(max_llm_calls: u32) -> Self {
        Self {
            max_llm_calls,
            ..Default::default()
        }
    }

    /// Check if budget is unlimited for a specific resource
    pub fn is_unlimited(&self) -> bool {
        self.max_steps == u32::MAX
            && self.max_llm_calls == u32::MAX
            && self.max_tool_calls == u32::MAX
            && self.max_runtime_ms == u64::MAX
            && self.max_memory_reads == u32::MAX
            && self.max_memory_writes == u32::MAX
    }
}

/// Budget usage tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetUsage {
    /// Number of steps executed
    pub steps: u32,
    /// Number of LLM calls made
    pub llm_calls: u32,
    /// Number of tool calls made
    pub tool_calls: u32,
    /// Runtime in milliseconds
    pub runtime_ms: u64,
    /// Number of memory reads
    pub memory_reads: u32,
    /// Number of memory writes
    pub memory_writes: u32,
}

impl Default for BudgetUsage {
    fn default() -> Self {
        Self {
            steps: 0,
            llm_calls: 0,
            tool_calls: 0,
            runtime_ms: 0,
            memory_reads: 0,
            memory_writes: 0,
        }
    }
}

impl BudgetUsage {
    /// Create a new BudgetUsage
    pub fn new() -> Self {
        Self::default()
    }

    /// Increment step count
    pub fn increment_step(&mut self) {
        self.steps += 1;
    }

    /// Increment LLM call count
    pub fn increment_llm_call(&mut self) {
        self.llm_calls += 1;
    }

    /// Increment tool call count
    pub fn increment_tool_call(&mut self) {
        self.tool_calls += 1;
    }

    /// Add runtime
    pub fn add_runtime(&mut self, duration_ms: u64) {
        self.runtime_ms += duration_ms;
    }

    /// Increment memory read count
    pub fn increment_memory_read(&mut self) {
        self.memory_reads += 1;
    }

    /// Increment memory write count
    pub fn increment_memory_write(&mut self) {
        self.memory_writes += 1;
    }

    /// Get usage percentage for a specific resource
    pub fn usage_percentage(&self, budget: &ExecutionBudget) -> HashMap<String, f64> {
        let mut usage = HashMap::new();

        usage.insert(
            "steps".to_string(),
            if budget.max_steps == 0 {
                0.0
            } else {
                (self.steps as f64 / budget.max_steps as f64) * 100.0
            },
        );
        usage.insert(
            "llm_calls".to_string(),
            if budget.max_llm_calls == 0 {
                0.0
            } else {
                (self.llm_calls as f64 / budget.max_llm_calls as f64) * 100.0
            },
        );
        usage.insert(
            "tool_calls".to_string(),
            if budget.max_tool_calls == 0 {
                0.0
            } else {
                (self.tool_calls as f64 / budget.max_tool_calls as f64) * 100.0
            },
        );
        usage.insert(
            "runtime".to_string(),
            if budget.max_runtime_ms == 0 {
                0.0
            } else {
                (self.runtime_ms as f64 / budget.max_runtime_ms as f64) * 100.0
            },
        );
        usage.insert(
            "memory_reads".to_string(),
            if budget.max_memory_reads == 0 {
                0.0
            } else {
                (self.memory_reads as f64 / budget.max_memory_reads as f64) * 100.0
            },
        );
        usage.insert(
            "memory_writes".to_string(),
            if budget.max_memory_writes == 0 {
                0.0
            } else {
                (self.memory_writes as f64 / budget.max_memory_writes as f64) * 100.0
            },
        );

        usage
    }

    /// Check if any limit has been exceeded
    pub fn is_exceeded(&self, budget: &ExecutionBudget) -> bool {
        self.steps > budget.max_steps
            || self.llm_calls > budget.max_llm_calls
            || self.tool_calls > budget.max_tool_calls
            || self.runtime_ms > budget.max_runtime_ms
            || self.memory_reads > budget.max_memory_reads
            || self.memory_writes > budget.max_memory_writes
    }

    /// Get the first exceeded limit
    pub fn get_exceeded_limit(&self, budget: &ExecutionBudget) -> Option<String> {
        if self.steps > budget.max_steps {
            Some(format!("steps: {}/{}", self.steps, budget.max_steps))
        } else if self.llm_calls > budget.max_llm_calls {
            Some(format!(
                "llm_calls: {}/{}",
                self.llm_calls, budget.max_llm_calls
            ))
        } else if self.tool_calls > budget.max_tool_calls {
            Some(format!(
                "tool_calls: {}/{}",
                self.tool_calls, budget.max_tool_calls
            ))
        } else if self.runtime_ms > budget.max_runtime_ms {
            Some(format!(
                "runtime: {}ms/{}ms",
                self.runtime_ms, budget.max_runtime_ms
            ))
        } else if self.memory_reads > budget.max_memory_reads {
            Some(format!(
                "memory_reads: {}/{}",
                self.memory_reads, budget.max_memory_reads
            ))
        } else if self.memory_writes > budget.max_memory_writes {
            Some(format!(
                "memory_writes: {}/{}",
                self.memory_writes, budget.max_memory_writes
            ))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_budget_default() {
        let budget = ExecutionBudget::default();
        assert_eq!(budget.max_steps, 100);
        assert_eq!(budget.max_llm_calls, 50);
        assert_eq!(budget.max_tool_calls, 20);
        assert_eq!(budget.max_runtime_ms, 300_000);
    }

    #[test]
    fn test_execution_budget_custom() {
        let budget = ExecutionBudget::new(10, 5, 3, 60_000, 20, 10);
        assert_eq!(budget.max_steps, 10);
        assert_eq!(budget.max_llm_calls, 5);
        assert_eq!(budget.max_tool_calls, 3);
    }

    #[test]
    fn test_budget_usage_increment() {
        let mut usage = BudgetUsage::new();
        usage.increment_step();
        usage.increment_llm_call();
        usage.increment_tool_call();
        usage.add_runtime(100);
        usage.increment_memory_read();
        usage.increment_memory_write();

        assert_eq!(usage.steps, 1);
        assert_eq!(usage.llm_calls, 1);
        assert_eq!(usage.tool_calls, 1);
        assert_eq!(usage.runtime_ms, 100);
        assert_eq!(usage.memory_reads, 1);
        assert_eq!(usage.memory_writes, 1);
    }

    #[test]
    fn test_budget_usage_exceeded() {
        let budget = ExecutionBudget::new(5, 3, 2, 10_000, 10, 5);
        let mut usage = BudgetUsage::new();

        assert!(!usage.is_exceeded(&budget));

        usage.steps = 6;
        assert!(usage.is_exceeded(&budget));
        assert_eq!(
            usage.get_exceeded_limit(&budget),
            Some("steps: 6/5".to_string())
        );
    }

    #[test]
    fn test_usage_percentage() {
        let budget = ExecutionBudget::new(100, 50, 20, 1000, 100, 50);
        let mut usage = BudgetUsage::new();

        usage.steps = 50;
        usage.llm_calls = 25;

        let percentages = usage.usage_percentage(&budget);
        assert_eq!(percentages.get("steps"), Some(&50.0));
        assert_eq!(percentages.get("llm_calls"), Some(&50.0));
    }
}
