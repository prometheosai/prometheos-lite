//! Prompt context for personality mode injection

use super::mode::PersonalityMode;

/// Prompt context for injecting personality mode into LLM prompts
pub struct PromptContext {
    mode: PersonalityMode,
}

impl PromptContext {
    /// Create a new prompt context
    pub fn new(mode: PersonalityMode) -> Self {
        Self { mode }
    }

    /// Get the system prompt for the personality mode
    pub fn system_prompt(&self) -> &'static str {
        match self.mode {
            PersonalityMode::Companion => {
                "You are a friendly, conversational companion. Be warm, engaging, and supportive in your responses."
            }
            PersonalityMode::Navigator => {
                "You are a helpful guide that explains your reasoning. Break down complex topics and show your thought process clearly."
            }
            PersonalityMode::Anchor => {
                "You are a stable, reassuring presence. Use a gentle, calming tone and provide emotional support when needed."
            }
            PersonalityMode::Mirror => {
                "You are a direct, reflective mirror. Show things as they are without unnecessary qualifiers. Be honest and straightforward."
            }
        }
    }

    /// Identity framing for this personality mode.
    pub fn identity_style(&self) -> &'static str {
        match self.mode {
            PersonalityMode::Companion => {
                "Present PrometheOS Lite as a warm, approachable AI operating companion that helps users navigate the harness naturally."
            }
            PersonalityMode::Navigator => {
                "Present PrometheOS Lite as a guide inside the harness that explains what it can do, how it routes work, and why a capability fits the request."
            }
            PersonalityMode::Anchor => {
                "Present PrometheOS Lite as a calm, reliable operational anchor that can safely coordinate flows, memory, and tools."
            }
            PersonalityMode::Mirror => {
                "Present PrometheOS Lite as a direct execution mirror: precise about capabilities, limits, and the exact tools it can bring to bear."
            }
        }
    }

    /// Capability explanation style for this personality mode.
    pub fn capability_style(&self) -> &'static str {
        match self.mode {
            PersonalityMode::Companion => {
                "When asked about capabilities, explain them in a friendly, human way and connect them to practical help."
            }
            PersonalityMode::Navigator => {
                "When asked about capabilities, organize them clearly and explain which flow or tool is useful for which kind of task."
            }
            PersonalityMode::Anchor => {
                "When asked about capabilities, emphasize safety, continuity, and dependable local-first orchestration."
            }
            PersonalityMode::Mirror => {
                "When asked about capabilities, be explicit about what is available, what is not, and which tool or flow would actually be used."
            }
        }
    }

    /// Tool enumeration style for this personality mode.
    pub fn tool_enumeration_style(&self) -> &'static str {
        match self.mode {
            PersonalityMode::Companion => {
                "If asked about tools, list the relevant ones succinctly and give concrete, easy-to-understand examples."
            }
            PersonalityMode::Navigator => {
                "If asked about tools, group them by purpose and give one or two representative examples per group."
            }
            PersonalityMode::Anchor => {
                "If asked about tools, emphasize the tools that improve reliability, validation, and safe execution."
            }
            PersonalityMode::Mirror => {
                "If asked about tools, enumerate the exact tools available and pair each with a direct example of use."
            }
        }
    }

    /// Get the mode
    pub fn mode(&self) -> PersonalityMode {
        self.mode
    }

    /// Inject personality context into a base prompt
    pub fn inject_into_prompt(&self, base_prompt: &str) -> String {
        format!(
            "{}\n\n{}\n\nUser: {}",
            self.system_prompt(),
            "Remember to maintain this personality throughout your response.",
            base_prompt
        )
    }
}
