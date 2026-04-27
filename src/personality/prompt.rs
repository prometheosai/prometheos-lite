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
