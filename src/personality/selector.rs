//! Personality mode selector

use super::mode::PersonalityMode;

/// Mode selector for choosing personality based on context
pub struct ModeSelector {
    default_mode: PersonalityMode,
}

impl ModeSelector {
    /// Create a new mode selector with default mode
    pub fn new(default_mode: PersonalityMode) -> Self {
        Self { default_mode }
    }

    /// Select mode based on text input
    pub fn select_from_text(&self, text: &str) -> PersonalityMode {
        let text_lower = text.to_lowercase();

        // Simple heuristic-based selection
        if text_lower.contains("help")
            || text_lower.contains("explain")
            || text_lower.contains("guide")
        {
            PersonalityMode::Navigator
        } else if text_lower.contains("calm")
            || text_lower.contains("reassure")
            || text_lower.contains("gentle")
        {
            PersonalityMode::Anchor
        } else if text_lower.contains("direct")
            || text_lower.contains("honest")
            || text_lower.contains("reflect")
        {
            PersonalityMode::Mirror
        } else {
            self.default_mode
        }
    }

    /// Select mode explicitly by name
    pub fn select_by_name(&self, name: &str) -> Option<PersonalityMode> {
        PersonalityMode::parse(name)
    }

    /// Get the default mode
    pub fn default(&self) -> PersonalityMode {
        self.default_mode
    }
}

impl Default for ModeSelector {
    fn default() -> Self {
        Self::new(PersonalityMode::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mode_selector_creation() {
        let selector = ModeSelector::new(PersonalityMode::Companion);
        assert_eq!(selector.default(), PersonalityMode::Companion);
    }

    #[test]
    fn test_mode_selector_default() {
        let selector = ModeSelector::new(PersonalityMode::default());
        assert_eq!(selector.default(), PersonalityMode::Companion);
    }

    #[test]
    fn test_select_from_text_navigator() {
        let selector = ModeSelector::new(PersonalityMode::Companion);

        assert_eq!(
            selector.select_from_text("help me"),
            PersonalityMode::Navigator
        );
        assert_eq!(
            selector.select_from_text("explain this"),
            PersonalityMode::Navigator
        );
        assert_eq!(
            selector.select_from_text("guide me"),
            PersonalityMode::Navigator
        );
    }

    #[test]
    fn test_select_from_text_anchor() {
        let selector = ModeSelector::new(PersonalityMode::Companion);

        assert_eq!(
            selector.select_from_text("calm down"),
            PersonalityMode::Anchor
        );
        assert_eq!(
            selector.select_from_text("reassure me"),
            PersonalityMode::Anchor
        );
        assert_eq!(
            selector.select_from_text("be gentle"),
            PersonalityMode::Anchor
        );
    }

    #[test]
    fn test_select_from_text_mirror() {
        let selector = ModeSelector::new(PersonalityMode::Companion);

        assert_eq!(
            selector.select_from_text("be direct"),
            PersonalityMode::Mirror
        );
        assert_eq!(
            selector.select_from_text("be honest"),
            PersonalityMode::Mirror
        );
        assert_eq!(
            selector.select_from_text("reflect on this"),
            PersonalityMode::Mirror
        );
    }

    #[test]
    fn test_select_from_text_default() {
        let selector = ModeSelector::new(PersonalityMode::Companion);

        assert_eq!(
            selector.select_from_text("random text"),
            PersonalityMode::Companion
        );
    }

    #[test]
    fn test_select_by_name() {
        let selector = ModeSelector::new(PersonalityMode::Companion);

        assert_eq!(
            selector.select_by_name("companion"),
            Some(PersonalityMode::Companion)
        );
        assert_eq!(
            selector.select_by_name("navigator"),
            Some(PersonalityMode::Navigator)
        );
        assert_eq!(
            selector.select_by_name("anchor"),
            Some(PersonalityMode::Anchor)
        );
        assert_eq!(
            selector.select_by_name("mirror"),
            Some(PersonalityMode::Mirror)
        );
        assert_eq!(selector.select_by_name("invalid"), None);
    }
}
