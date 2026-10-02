//! Disclosure/redaction policy for the human plan projection. Reuses
//! `workflow::redaction` exclusively — no second redaction taxonomy.

/// Caller-supplied disclosure policy for the human plan. The canonical
/// projection never accepts one.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RedactionPolicy {
    /// Literal secrets to replace verbatim (seeded by the caller, e.g. from
    /// `workflow::redaction::collect_known_secrets(repo)`).
    pub known_secrets: Vec<String>,
    /// Rendered `Field:` line keys whose values are suppressed, e.g. "Purpose".
    pub omitted_fields: Vec<String>,
}
