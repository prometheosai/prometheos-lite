use anyhow::Result;
use async_trait::async_trait;

use crate::legacy::agents::{Agent, AgentRole};
use crate::llm::LlmClient;

#[derive(Debug, Clone)]
pub struct PlannerAgent {
    llm: LlmClient,
}

impl PlannerAgent {
    pub fn new(llm: LlmClient) -> Self {
        Self { llm }
    }

    fn prompt(&self, input: &str) -> String {
        format!(
            r#"You are the Planner lane in PrometheOS Lite.

Convert the user's goal into a concise implementation plan.

Return structured markdown with exactly these sections:
## Task Breakdown
- Logical steps in execution order.

## File Targets
- Likely files or modules to create or edit.

## Acceptance Criteria
- Concrete checks that prove the work is complete.

Keep it practical, local-first, and scoped to a lightweight autonomous dev loop.

User goal:
{input}"#
        )
    }
}

#[async_trait]
impl Agent for PlannerAgent {
    fn name(&self) -> &str {
        AgentRole::Planner.as_str()
    }

    async fn run(&self, input: &str) -> Result<String> {
        self.llm.generate(&self.prompt(input)).await
    }
}
