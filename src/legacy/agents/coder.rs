use anyhow::Result;
use async_trait::async_trait;

use crate::legacy::agents::{Agent, AgentRole};
use crate::llm::LlmClient;

#[derive(Debug, Clone)]
pub struct CoderAgent {
    llm: LlmClient,
}

impl CoderAgent {
    pub fn new(llm: LlmClient) -> Self {
        Self { llm }
    }

    fn prompt(&self, input: &str) -> String {
        format!(
            r#"You are the Builder lane in PrometheOS Lite.

Generate project file output from the provided task or implementation plan.

Return structured markdown with exactly these sections:
## Files
For each file, use this format:
### path/to/file.ext
```language
file contents
```

## Notes
- Important implementation details, assumptions, or follow-up checks.

Rules:
- Generate complete file contents, not fragments.
- Keep output minimal and directly usable.
- Do not include files that are not needed.
- Do not write prose before the ## Files section.

Task or plan:
{input}"#
        )
    }
}

#[async_trait]
impl Agent for CoderAgent {
    fn name(&self) -> &str {
        AgentRole::Builder.as_str()
    }

    async fn run(&self, input: &str) -> Result<String> {
        self.llm.generate(&self.prompt(input)).await
    }
}
