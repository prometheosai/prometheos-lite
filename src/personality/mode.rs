//! Personality mode definitions

use serde::{Deserialize, Serialize};

/// Personality modes for the AI assistant
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PersonalityMode {
    /// Friendly, conversational companion
    Companion,
    /// Helpful guide that explains reasoning
    Navigator,
    /// Stable, reassuring presence with gentle tone
    Anchor,
    /// Direct, reflective mirror that shows things as they are
    Mirror,
}

impl PersonalityMode {
    /// Get the display name for the mode
    pub fn display_name(&self) -> &'static str {
        match self {
            PersonalityMode::Companion => "Companion",
            PersonalityMode::Navigator => "Navigator",
            PersonalityMode::Anchor => "Anchor",
            PersonalityMode::Mirror => "Mirror",
        }
    }

    /// Get the description for the mode
    pub fn description(&self) -> &'static str {
        match self {
            PersonalityMode::Companion => "Friendly, conversational companion",
            PersonalityMode::Navigator => "Helpful guide that explains reasoning",
            PersonalityMode::Anchor => "Stable, reassuring presence with gentle tone",
            PersonalityMode::Mirror => "Direct, reflective mirror that shows things as they are",
        }
    }

    /// Parse from string
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "companion" => Some(PersonalityMode::Companion),
            "navigator" => Some(PersonalityMode::Navigator),
            "anchor" => Some(PersonalityMode::Anchor),
            "mirror" => Some(PersonalityMode::Mirror),
            _ => None,
        }
    }
}

impl Default for PersonalityMode {
    fn default() -> Self {
        PersonalityMode::Companion
    }
}
