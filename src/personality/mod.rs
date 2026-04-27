//! Personality mode system

mod mode;
mod selector;
mod constitution;
mod prompt;

pub use mode::PersonalityMode;
pub use selector::ModeSelector;
pub use constitution::ConstitutionalFilter;
pub use prompt::PromptContext;
