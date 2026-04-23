use anyhow::Result;
use async_trait::async_trait;

use crate::{
    agents::{Agent, AgentRole},
    llm::LlmClient,
};

#[derive(Debug, Clone)]
pub struct ReviewerAgent {
    llm: LlmClient,
}

impl ReviewerAgent {
    pub fn new(llm: LlmClient) -> Self {
        Self { llm }
    }

    fn prompt(&self, input: &str) -> String {
        format!(
            r#"You are the Reviewer lane in PrometheOS Lite.

Analyze the Builder output for correctness, missing pieces, unsafe assumptions, and structure.
Refine the output when needed.

Return structured markdown with exactly these sections:
## Review Summary
- Short assessment of the generated output.

## Refined Files
If changes are needed, provide complete corrected file blocks:
### path/to/file.ext
```language
file contents
```
If no file changes are needed, write:
- No changes required.

## Validation
- Commands or checks the user should run.

## Confidence
- High, Medium, or Low with one short reason.

Rules:
- Prefer small corrections over rewrites.
- Preserve the Builder's intended file structure unless it is wrong.
- Keep the output concise and actionable.

Builder output:
{input}"#
        )
    }
}

#[async_trait]
impl Agent for ReviewerAgent {
    fn name(&self) -> &str {
        AgentRole::Reviewer.as_str()
    }

    async fn run(&self, input: &str) -> Result<String> {
        self.llm.generate(&self.prompt(input)).await
    }
}
