//! Control file loading and management

use anyhow::Result;
use std::path::PathBuf;

/// Control file loader
pub struct ControlFiles {
    pub soul: String,
    pub skills: String,
    pub flows: String,
    pub tools: String,
    pub memory: String,
}

impl ControlFiles {
    /// Load control files from .prometheos directory
    pub fn load() -> Result<Self> {
        let base_dir = PathBuf::from(".prometheos");

        let soul =
            Self::read_file(&base_dir.join("SOUL.md")).unwrap_or_else(|_| Self::default_soul());

        let skills =
            Self::read_file(&base_dir.join("SKILLS.md")).unwrap_or_else(|_| Self::default_skills());

        let flows =
            Self::read_file(&base_dir.join("FLOWS.md")).unwrap_or_else(|_| Self::default_flows());

        let tools =
            Self::read_file(&base_dir.join("TOOLS.md")).unwrap_or_else(|_| Self::default_tools());

        let memory =
            Self::read_file(&base_dir.join("MEMORY.md")).unwrap_or_else(|_| Self::default_memory());

        Ok(Self {
            soul,
            skills,
            flows,
            tools,
            memory,
        })
    }

    fn read_file(path: &PathBuf) -> Result<String> {
        std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Failed to read control file {:?}: {}", path, e))
    }

    fn default_soul() -> String {
        "# PrometheOS Lite - System Identity\n\n## Tone\nConcise, helpful, local-first AI assistant.\n\n## Boundaries\n- Do not generate code unless explicitly asked\n- Keep simple replies under 120 words\n- Respect user privacy\n- Local-first philosophy\n".to_string()
    }

    fn default_skills() -> String {
        "# PrometheOS Lite - Available Skills\n\n## Conversation\n- Intent: CONVERSATION\n- Handler: Direct LLM\n\n## Question\n- Intent: QUESTION\n- Handler: Direct LLM\n\n## Code Generation\n- Intent: CODING_TASK\n- Handler: CodeGen Flow\n".to_string()
    }

    fn default_flows() -> String {
        "# PrometheOS Lite - Available Flows\n\n## Code Generation Flow\n- File: flows/code-generation.json\n- Intent: CODING_TASK\n".to_string()
    }

    fn default_tools() -> String {
        "# PrometheOS Lite - Available Tools\n\n## cargo_check\n- Command: cargo check\n"
            .to_string()
    }

    fn default_memory() -> String {
        "# PrometheOS Lite - Memory Policy\n\n## Remember\n- Project goals\n- User preferences\n\n## Do Not Remember\n- Secrets\n- API keys\n".to_string()
    }

    /// Build conversation prompt from control files
    pub fn build_conversation_prompt(&self, user_message: &str) -> String {
        format!(
            "{}\n\n{}\n\nYou are PrometheOS Lite, a concise local AI assistant.\n\
            Answer naturally and briefly.\n\
            Do not generate code unless explicitly asked.\n\
            Keep simple replies under 120 words.\n\n\
            User message: {}",
            self.soul, self.skills, user_message
        )
    }

    /// Build coding task prompt from control files
    pub fn build_coding_prompt(&self, user_message: &str) -> String {
        format!(
            "{}\n\n{}\n\nYou are a planning AI. Create a detailed plan for the following task:\n\n{}\n\nProvide a step-by-step plan with clear objectives.",
            self.soul, self.flows, user_message
        )
    }
}

impl Default for ControlFiles {
    fn default() -> Self {
        Self {
            soul: Self::default_soul(),
            skills: Self::default_skills(),
            flows: Self::default_flows(),
            tools: Self::default_tools(),
            memory: Self::default_memory(),
        }
    }
}
