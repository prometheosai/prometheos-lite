//! Constitutional filter for post-generation processing

use super::mode::PersonalityMode;

/// Constitutional filter for applying personality constraints
pub struct ConstitutionalFilter {
    mode: PersonalityMode,
}

impl ConstitutionalFilter {
    /// Create a new constitutional filter for a mode
    pub fn new(mode: PersonalityMode) -> Self {
        Self { mode }
    }

    /// Apply constitutional filter to generated text
    pub fn filter(&self, text: &str) -> String {
        let mut filtered = text.to_string();

        // Apply mode-specific filters
        match self.mode {
            PersonalityMode::Anchor => {
                filtered = self.apply_anchor_filter(&filtered);
            }
            PersonalityMode::Mirror => {
                filtered = self.apply_mirror_filter(&filtered);
            }
            PersonalityMode::Companion | PersonalityMode::Navigator => {
                // No specific filters for these modes
            }
        }

        // Apply universal filters
        filtered = self.shorten_excessive_output(&filtered);
        filtered = self.remove_false_certainty(&filtered);

        filtered
    }

    /// Anchor mode: ensure gentle tone
    fn apply_anchor_filter(&self, text: &str) -> String {
        let text = text.replace("must", "should");
        let text = text.replace("have to", "might want to");
        let text = text.replace("need to", "could consider");
        text
    }

    /// Mirror mode: ensure directness
    fn apply_mirror_filter(&self, text: &str) -> String {
        let text = text.replace("I think", "");
        let text = text.replace("I believe", "");
        let text = text.replace("It seems like", "");
        text
    }

    /// Universal: shorten excessive output
    fn shorten_excessive_output(&self, text: &str) -> String {
        const MAX_LENGTH: usize = 2000;
        if text.len() > MAX_LENGTH {
            format!("{}...", &text[..MAX_LENGTH])
        } else {
            text.to_string()
        }
    }

    /// Universal: remove false certainty
    fn remove_false_certainty(&self, text: &str) -> String {
        let text = text.replace("definitely", "likely");
        let text = text.replace("certainly", "probably");
        let text = text.replace("without a doubt", "most likely");
        text
    }
}
