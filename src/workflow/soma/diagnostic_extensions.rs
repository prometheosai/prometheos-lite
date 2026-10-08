//! Repository-owned diagnostic-extension registry.
//!
//! The vendored SOMA catalogues (`vendored/soma/v1.1`,
//! `vendored/soma/v1.2`) are upstream-provenance-locked: `.gitattributes`
//! keeps them `-text` (byte-stable), and any change to them requires an
//! explicit reviewed bundle upgrade (see `vendored/soma/v1.2/PROVENANCE.md`).
//! Lite therefore never edits them. When Lite must emit a diagnostic code
//! that upstream has not published yet, the code is pinned HERE instead —
//! a clearly repository-owned extension.
//!
//! Resolution law (see `category_for` in the parent module):
//!
//! 1. consult the vendored upstream v1.1 catalogue first (normative);
//! 2. then this extension registry;
//! 3. a code present in BOTH registries is corruption and fails closed
//!    at first resolution (`validate_registry` runs at init);
//! 4. codes absent from both fall back fail-safely to `"general"`.

/// One pinned repository-owned diagnostic extension: a Lite-emitted code
/// that is NOT published in the upstream-vendored catalogues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiagnosticExtension {
    /// The emitted diagnostic code (SOMA-* namespace, Lite-pinned).
    pub code: &'static str,
    /// The category the code resolves to (pinned, never inferred).
    pub category: &'static str,
}

/// The complete Lite diagnostic-extension registry. Keep this registry and
/// the `EXTENSION` mirror in `tests/emitted_diagnostics_conformance.rs` in
/// sync — the conformance suite fails closed on drift.
pub const DIAGNOSTIC_EXTENSIONS: &[DiagnosticExtension] = &[DiagnosticExtension {
    // E4/X07 Slice 3: duplicate review-issue, gate-record, or event
    // identity (the same `(sequence, semanticDigest)` inside one run).
    // Pinned here, not in the vendored catalogues — #240 locked the
    // upstream trees with an additivity and provenance contract.
    code: "SOMA-CMP-0011",
    category: "duplicate_identity",
}];

/// Fail-closed registry validation: an extension registry is valid only
/// when it contains no internal duplicate code and no code the upstream
/// catalogue has already published — the vendored catalogues stay
/// normative, and an overlapping extension could silently re-categorize
/// an upstream code.
pub fn validate_registry(
    registry: &[DiagnosticExtension],
    upstream_codes: &[&str],
) -> Result<(), String> {
    for (index, entry) in registry.iter().enumerate() {
        if upstream_codes.contains(&entry.code) {
            return Err(format!(
                "extension code {} is already published in the upstream catalogue; \
                 the vendored catalogues are immutable — remove the extension",
                entry.code
            ));
        }
        if registry[..index]
            .iter()
            .any(|other| other.code == entry.code)
        {
            return Err(format!(
                "extension code {} is registered twice in the extension registry",
                entry.code
            ));
        }
    }
    Ok(())
}

/// Resolve a code against the extension registry only (the upstream
/// catalogue is consulted first by the caller). `None` when the code is
/// not a pinned extension.
pub fn extension_category(code: &str) -> Option<&'static str> {
    DIAGNOSTIC_EXTENSIONS
        .iter()
        .find(|entry| entry.code == code)
        .map(|entry| entry.category)
}
