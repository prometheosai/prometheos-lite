//! Budget guard for enforcing resource limits during flow execution

use anyhow::{Context, Result};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::{BudgetUsage, ExecutionBudget};

/// Budget guard for enforcing resource limits
pub struct BudgetGuard {
    budget: ExecutionBudget,
    usage: Arc<Mutex<BudgetUsage>>,
    start_time: Instant,
}

impl BudgetGuard {
    /// Create a new BudgetGuard with the given budget
    pub fn new(budget: ExecutionBudget) -> Self {
        Self {
            budget,
            usage: Arc::new(Mutex::new(BudgetUsage::new())),
            start_time: Instant::now(),
        }
    }

    /// Check if a step can be executed
    pub fn check_step(&self) -> Result<()> {
        let usage = self.usage.lock().unwrap();
        if usage.steps >= self.budget.max_steps {
            return Err(anyhow::anyhow!(
                "Budget exceeded: steps {}/{}",
                usage.steps,
                self.budget.max_steps
            ));
        }
        Ok(())
    }

    /// Check if an LLM call can be made
    pub fn check_llm_call(&self) -> Result<()> {
        let usage = self.usage.lock().unwrap();
        if usage.llm_calls >= self.budget.max_llm_calls {
            return Err(anyhow::anyhow!(
                "Budget exceeded: LLM calls {}/{}",
                usage.llm_calls,
                self.budget.max_llm_calls
            ));
        }
        Ok(())
    }

    /// Check if a tool call can be made
    pub fn check_tool_call(&self) -> Result<()> {
        let usage = self.usage.lock().unwrap();
        if usage.tool_calls >= self.budget.max_tool_calls {
            return Err(anyhow::anyhow!(
                "Budget exceeded: tool calls {}/{}",
                usage.tool_calls,
                self.budget.max_tool_calls
            ));
        }
        Ok(())
    }

    /// Check if runtime is within limits
    pub fn check_runtime(&self) -> Result<()> {
        let elapsed = self.start_time.elapsed().as_millis() as u64;
        if elapsed >= self.budget.max_runtime_ms {
            return Err(anyhow::anyhow!(
                "Budget exceeded: runtime {}ms/{}ms",
                elapsed,
                self.budget.max_runtime_ms
            ));
        }
        Ok(())
    }

    /// Check if a memory read can be performed
    pub fn check_memory_read(&self) -> Result<()> {
        let usage = self.usage.lock().unwrap();
        if usage.memory_reads >= self.budget.max_memory_reads {
            return Err(anyhow::anyhow!(
                "Budget exceeded: memory reads {}/{}",
                usage.memory_reads,
                self.budget.max_memory_reads
            ));
        }
        Ok(())
    }

    /// Check if a memory write can be performed
    pub fn check_memory_write(&self) -> Result<()> {
        let usage = self.usage.lock().unwrap();
        if usage.memory_writes >= self.budget.max_memory_writes {
            return Err(anyhow::anyhow!(
                "Budget exceeded: memory writes {}/{}",
                usage.memory_writes,
                self.budget.max_memory_writes
            ));
        }
        Ok(())
    }

    /// Record a step execution
    pub fn record_step(&self) -> Result<()> {
        self.check_step()?;
        let mut usage = self.usage.lock().unwrap();
        usage.increment_step();
        Ok(())
    }

    /// Record an LLM call
    pub fn record_llm_call(&self) -> Result<()> {
        self.check_llm_call()?;
        let mut usage = self.usage.lock().unwrap();
        usage.increment_llm_call();
        Ok(())
    }

    /// Record a tool call
    pub fn record_tool_call(&self) -> Result<()> {
        self.check_tool_call()?;
        let mut usage = self.usage.lock().unwrap();
        usage.increment_tool_call();
        Ok(())
    }

    /// Record a memory read
    pub fn record_memory_read(&self) -> Result<()> {
        self.check_memory_read()?;
        let mut usage = self.usage.lock().unwrap();
        usage.increment_memory_read();
        Ok(())
    }

    /// Record a memory write
    pub fn record_memory_write(&self) -> Result<()> {
        self.check_memory_write()?;
        let mut usage = self.usage.lock().unwrap();
        usage.increment_memory_write();
        Ok(())
    }

    /// Update runtime and check if within limits
    pub fn update_runtime(&self) -> Result<()> {
        let elapsed = self.start_time.elapsed().as_millis() as u64;
        let mut usage = self.usage.lock().unwrap();
        usage.runtime_ms = elapsed;
        drop(usage);
        self.check_runtime()
    }

    /// Get current usage
    pub fn get_usage(&self) -> BudgetUsage {
        let mut usage = self.usage.lock().unwrap();
        usage.runtime_ms = self.start_time.elapsed().as_millis() as u64;
        usage.clone()
    }

    /// Get budget configuration
    pub fn get_budget(&self) -> &ExecutionBudget {
        &self.budget
    }

    /// Check if budget is exceeded
    pub fn is_exceeded(&self) -> bool {
        let usage = self.get_usage();
        usage.is_exceeded(&self.budget)
    }

    /// Get usage report as JSON
    pub fn get_report(&self) -> serde_json::Value {
        let usage = self.get_usage();
        let percentages = usage.usage_percentage(&self.budget);

        serde_json::json!({
            "budget": {
                "max_steps": self.budget.max_steps,
                "max_llm_calls": self.budget.max_llm_calls,
                "max_tool_calls": self.budget.max_tool_calls,
                "max_runtime_ms": self.budget.max_runtime_ms,
                "max_memory_reads": self.budget.max_memory_reads,
                "max_memory_writes": self.budget.max_memory_writes,
            },
            "usage": {
                "steps": usage.steps,
                "llm_calls": usage.llm_calls,
                "tool_calls": usage.tool_calls,
                "runtime_ms": usage.runtime_ms,
                "memory_reads": usage.memory_reads,
                "memory_writes": usage.memory_writes,
            },
            "usage_percentage": percentages,
            "exceeded": usage.is_exceeded(&self.budget),
            "exceeded_limit": usage.get_exceeded_limit(&self.budget),
        })
    }
}

impl Clone for BudgetGuard {
    fn clone(&self) -> Self {
        Self {
            budget: self.budget.clone(),
            usage: Arc::clone(&self.usage),
            start_time: self.start_time,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_budget_guard_step_check() {
        let budget = ExecutionBudget::with_steps(2);
        let guard = BudgetGuard::new(budget);

        assert!(guard.record_step().is_ok());
        assert!(guard.record_step().is_ok());
        assert!(guard.record_step().is_err());
    }

    #[test]
    fn test_budget_guard_llm_call_check() {
        let budget = ExecutionBudget::with_llm_calls(1);
        let guard = BudgetGuard::new(budget);

        assert!(guard.record_llm_call().is_ok());
        assert!(guard.record_llm_call().is_err());
    }

    #[test]
    fn test_budget_guard_tool_call_check() {
        let budget = ExecutionBudget::new(100, 100, 1, 100_000, 100, 100);
        let guard = BudgetGuard::new(budget);

        assert!(guard.record_tool_call().is_ok());
        assert!(guard.record_tool_call().is_err());
    }

    #[test]
    fn test_budget_guard_memory_check() {
        let budget = ExecutionBudget::new(100, 100, 100, 100_000, 1, 1);
        let guard = BudgetGuard::new(budget);

        assert!(guard.record_memory_read().is_ok());
        assert!(guard.record_memory_read().is_err());

        assert!(guard.record_memory_write().is_ok());
        assert!(guard.record_memory_write().is_err());
    }

    #[test]
    fn test_budget_guard_usage_report() {
        let budget = ExecutionBudget::new(10, 5, 3, 60_000, 10, 5);
        let guard = BudgetGuard::new(budget);

        guard.record_step().unwrap();
        guard.record_llm_call().unwrap();

        let report = guard.get_report();
        assert_eq!(report["usage"]["steps"], 1);
        assert_eq!(report["usage"]["llm_calls"], 1);
        assert_eq!(report["usage_percentage"]["steps"], 10.0);
        assert_eq!(report["usage_percentage"]["llm_calls"], 20.0);
    }

    #[test]
    fn test_budget_guard_clone() {
        let budget = ExecutionBudget::with_steps(5);
        let guard1 = BudgetGuard::new(budget);
        let guard2 = guard1.clone();

        guard1.record_step().unwrap();
        guard2.record_step().unwrap();

        // Both guards share the same usage
        let usage = guard1.get_usage();
        assert_eq!(usage.steps, 2);
    }
}
