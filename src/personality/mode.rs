//! Personality mode definitions

use serde::{Deserialize, Serialize};

/// Personality modes for the AI assistant
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum PersonalityMode {
    /// Friendly, conversational companion
    #[default]
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
    pub fn parse(s: &str) -> Option<Self> {
        <Self as std::str::FromStr>::from_str(s).ok()
    }
}

impl std::str::FromStr for PersonalityMode {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "companion" => Ok(PersonalityMode::Companion),
            "navigator" => Ok(PersonalityMode::Navigator),
            "anchor" => Ok(PersonalityMode::Anchor),
            "mirror" => Ok(PersonalityMode::Mirror),
            _ => Err(()),
        }
    }
}
