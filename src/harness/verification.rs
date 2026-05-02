use crate::harness::validation::ValidationResult;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum VerificationStrength {
    None,
    StaticOnly,
    FormatOnly,
    LintOnly,
    Tests,
    Reproduction,
    Full,
}
pub fn assess_verification_strength(r: Option<&ValidationResult>) -> VerificationStrength {
    if r.is_some_and(|x| !x.command_results.is_empty()) {
        VerificationStrength::Tests
    } else {
        VerificationStrength::None
    }
}
