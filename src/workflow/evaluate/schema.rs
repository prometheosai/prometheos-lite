//! Durable document schema versioning.
//!
//! Every on-disk document owned by the evaluation runtime
//! ([`ProposalRegistry`], [`ExecutionIdentity`], [`EvidenceBundle`],
//! [`EvaluationCheckpoint`], [`JournalEvent`]) carries a validated
//! [`SchemaVersion`]. The version is the single authoritative representation
//! used for read-path compatibility checks, migration dispatch, and
//! fail-closed rejection of unknown future versions.
//!
//! Existing `v1.7.0` files are unversioned. They are treated as an explicit
//! legacy version `0.0.0` — NEVER as the current version — so that every
//! current-format document is written with an explicit `schema_version` and
//! unversioned data is always migrated rather than silently accepted.

use anyhow::{Result, bail};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

/// Current supported schema version for all durable documents.
pub const CURRENT_SCHEMA_VERSION: SchemaVersion = SchemaVersion::new(1, 0, 0);

/// Version assumed for legacy unversioned documents.
///
/// Distinct from [`CURRENT_SCHEMA_VERSION`]: a document that carries no
/// `schema_version` is legacy and must be migrated, never treated as current.
pub const LEGACY_UNVERSIONED_VERSION: SchemaVersion = SchemaVersion::new(0, 0, 0);

/// A validated semantic schema version (`major.minor.patch`).
///
/// Serializes as a `"major.minor.patch"` string so `v1.7.0` evidence bundles
/// (which already carry `"schema_version": "1.0.0"`) remain byte-compatible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SchemaVersion {
    major: u32,
    minor: u32,
    patch: u32,
}

impl SchemaVersion {
    /// Construct a schema version with explicit components.
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Parse a version string, accepting `"1"`, `"1.0"`, and `"1.0.0"`.
    /// Missing minor/patch components default to `0`. Malformed input fails.
    pub fn parse(s: &str) -> Result<Self> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            bail!("empty schema version string");
        }
        let mut parts = trimmed.split('.');
        let major = parse_component(parts.next().unwrap(), "major")?;
        let minor = match parts.next() {
            Some(p) => parse_component(p, "minor")?,
            None => 0,
        };
        let patch = match parts.next() {
            Some(p) => parse_component(p, "patch")?,
            None => 0,
        };
        if parts.next().is_some() {
            bail!("malformed schema version (too many components): {s:?}");
        }
        Ok(Self {
            major,
            minor,
            patch,
        })
    }

    /// True if this version is exactly the current supported schema version.
    pub fn is_current(self) -> bool {
        self == CURRENT_SCHEMA_VERSION
    }

    /// True if this version shares the current major (read-compatible or
    /// migratable) and is not newer than the current version.
    pub fn is_supported(self) -> bool {
        self.major == CURRENT_SCHEMA_VERSION.major && self <= CURRENT_SCHEMA_VERSION
    }

    /// Format as `"major.minor.patch"`.
    pub fn to_string_owned(self) -> String {
        format!("{}.{}.{}", self.major, self.minor, self.patch)
    }
}

fn parse_component(s: &str, which: &str) -> Result<u32> {
    s.parse::<u32>()
        .map_err(|_| anyhow::anyhow!("malformed schema version {which} component: {s:?}"))
}

impl fmt::Display for SchemaVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl FromStr for SchemaVersion {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        Self::parse(s)
    }
}

impl Default for SchemaVersion {
    fn default() -> Self {
        CURRENT_SCHEMA_VERSION
    }
}

impl PartialEq<&str> for SchemaVersion {
    fn eq(&self, other: &&str) -> bool {
        &self.to_string_owned() == other
    }
}

impl Serialize for SchemaVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string_owned())
    }
}

impl<'de> Deserialize<'de> for SchemaVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}

/// The durable document kinds versioned by this runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DocumentType {
    /// `<repo>/.prometheos/workflow/proposal_registry.json`
    ProposalRegistry,
    /// `<evidence_dir>/execution_identity.json`
    ExecutionIdentity,
    /// `<evidence_dir>/evidence.json`
    EvidenceBundle,
    /// `<repo>/.prometheos/workflow/checkpoint/<identity_key>.json`
    EvaluationCheckpoint,
    /// Journal event file under the identity journal directory.
    JournalEvent,
}

impl DocumentType {
    /// Stable display name used in diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            DocumentType::ProposalRegistry => "ProposalRegistry",
            DocumentType::ExecutionIdentity => "ExecutionIdentity",
            DocumentType::EvidenceBundle => "EvidenceBundle",
            DocumentType::EvaluationCheckpoint => "EvaluationCheckpoint",
            DocumentType::JournalEvent => "JournalEvent",
        }
    }

    /// Inclusive supported version range for this document type.
    pub fn supported_range(self) -> (SchemaVersion, SchemaVersion) {
        (SchemaVersion::new(1, 0, 0), CURRENT_SCHEMA_VERSION)
    }
}

/// Classification of a discovered document version relative to this runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionStatus {
    /// Matches the current schema exactly.
    Current,
    /// Older than current but within the supported major; migratable.
    Legacy,
    /// Newer than any supported version; must fail closed.
    Unsupported,
}

/// Actionable diagnostic for a version mismatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionDiagnostic {
    /// Which durable document was inspected.
    pub document_type: DocumentType,
    /// The version discovered on disk.
    pub discovered: SchemaVersion,
    /// The inclusive supported range `(min, max)`.
    pub supported: (SchemaVersion, SchemaVersion),
    /// Human-readable migration action, when available.
    pub migration_action: String,
}

/// Validate a discovered document version against the supported range.
///
/// Fails only on malformed versions. Returns the classification
/// ([`VersionStatus`]) so the caller can decide whether to read, migrate,
/// or fail closed.
pub fn validate_version(
    document_type: DocumentType,
    discovered: SchemaVersion,
) -> Result<VersionStatus> {
    let (min, max) = document_type.supported_range();
    if discovered == LEGACY_UNVERSIONED_VERSION {
        return Ok(VersionStatus::Legacy);
    }
    if discovered.is_current() {
        return Ok(VersionStatus::Current);
    }
    if discovered.is_supported() && discovered >= min && discovered <= max {
        return Ok(VersionStatus::Legacy);
    }
    Ok(VersionStatus::Unsupported)
}

/// Produce an actionable [`VersionDiagnostic`] describing the discovered
/// version, the supported range, and the required action.
pub fn version_diagnostic(
    document_type: DocumentType,
    discovered: SchemaVersion,
) -> VersionDiagnostic {
    let (min, max) = document_type.supported_range();
    let migration_action = match validate_version(document_type, discovered) {
        Ok(VersionStatus::Current) => "none: version is current".to_string(),
        Ok(VersionStatus::Legacy) => format!(
            "migrate {} version {} to current {}",
            document_type.as_str(),
            discovered,
            max
        ),
        Ok(VersionStatus::Unsupported) | Err(_) => format!(
            "fail closed: {} version {} is outside the supported range {}-{}",
            document_type.as_str(),
            discovered,
            min,
            max
        ),
    };
    VersionDiagnostic {
        document_type,
        discovered,
        supported: (min, max),
        migration_action,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_short_forms() {
        assert_eq!(
            SchemaVersion::parse("1").unwrap(),
            SchemaVersion::new(1, 0, 0)
        );
        assert_eq!(
            SchemaVersion::parse("1.0").unwrap(),
            SchemaVersion::new(1, 0, 0)
        );
        assert_eq!(
            SchemaVersion::parse("1.0.0").unwrap(),
            SchemaVersion::new(1, 0, 0)
        );
        assert_eq!(
            SchemaVersion::parse("2.3.4").unwrap(),
            SchemaVersion::new(2, 3, 4)
        );
    }

    #[test]
    fn parse_rejects_malformed() {
        assert!(SchemaVersion::parse("").is_err());
        assert!(SchemaVersion::parse("abc").is_err());
        assert!(SchemaVersion::parse("1.2.3.4").is_err());
        assert!(SchemaVersion::parse("1..3").is_err());
        assert!(SchemaVersion::parse("1.x").is_err());
    }

    #[test]
    fn ordering_is_deterministic() {
        assert!(SchemaVersion::new(1, 0, 0) < SchemaVersion::new(1, 1, 0));
        assert!(SchemaVersion::new(1, 1, 0) < SchemaVersion::new(2, 0, 0));
        assert!(SchemaVersion::new(1, 0, 0) == SchemaVersion::new(1, 0, 0));
        assert!(SchemaVersion::new(2, 0, 0) > SchemaVersion::new(1, 9, 9));
    }

    #[test]
    fn current_is_supported_but_future_is_not() {
        assert!(SchemaVersion::new(1, 0, 0).is_current());
        assert!(SchemaVersion::new(1, 0, 0).is_supported());
        // Any version beyond the current 1.0.0 is a future version.
        assert!(!SchemaVersion::new(1, 2, 0).is_current());
        assert!(!SchemaVersion::new(1, 2, 0).is_supported());
        assert!(!SchemaVersion::new(2, 0, 0).is_supported());
        assert!(!SchemaVersion::new(99, 0, 0).is_supported());
    }

    #[test]
    fn validation_classifies_legacy_and_unsupported() {
        let doc = DocumentType::ExecutionIdentity;
        assert_eq!(
            validate_version(doc, SchemaVersion::new(1, 0, 0)).unwrap(),
            VersionStatus::Current
        );
        // Unversioned legacy (0.0.0) is Legacy, never Current.
        assert_eq!(
            validate_version(doc, LEGACY_UNVERSIONED_VERSION).unwrap(),
            VersionStatus::Legacy
        );
        assert!(!LEGACY_UNVERSIONED_VERSION.is_current());
        // Future versions within the current major and above it fail closed.
        assert_eq!(
            validate_version(doc, SchemaVersion::new(1, 1, 0)).unwrap(),
            VersionStatus::Unsupported
        );
        assert_eq!(
            validate_version(doc, SchemaVersion::new(2, 0, 0)).unwrap(),
            VersionStatus::Unsupported
        );
    }

    #[test]
    fn diagnostic_contains_actionable_fields() {
        let diag = version_diagnostic(DocumentType::EvidenceBundle, SchemaVersion::new(9, 0, 0));
        assert_eq!(diag.document_type, DocumentType::EvidenceBundle);
        assert_eq!(diag.discovered, SchemaVersion::new(9, 0, 0));
        assert_eq!(diag.supported.1, CURRENT_SCHEMA_VERSION);
        assert!(diag.migration_action.contains("fail closed"));
        assert!(diag.migration_action.contains("EvidenceBundle"));

        // A current version needs no migration.
        let current =
            version_diagnostic(DocumentType::ProposalRegistry, SchemaVersion::new(1, 0, 0));
        assert_eq!(current.discovered, SchemaVersion::new(1, 0, 0));
        assert!(current.migration_action.contains("none"));

        // An unsupported future version must fail closed.
        let future =
            version_diagnostic(DocumentType::ProposalRegistry, SchemaVersion::new(3, 0, 0));
        assert!(future.migration_action.contains("fail closed"));
    }

    #[test]
    fn serializes_as_string_and_round_trips() {
        let json = serde_json::to_string(&SchemaVersion::new(1, 0, 0)).unwrap();
        assert_eq!(json, "\"1.0.0\"");
        let parsed: SchemaVersion = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, SchemaVersion::new(1, 0, 0));
    }

    #[test]
    fn compares_with_str_literal() {
        assert!(SchemaVersion::new(1, 0, 0) == "1.0.0");
        assert!(SchemaVersion::new(1, 2, 0) != "1.0.0");
    }
}
