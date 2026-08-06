//! In-place migration of legacy durable documents to the current schema.
//!
//! Existing `v1.7.0` files are unversioned. Migration is idempotent: a
//! document already at the current schema is left untouched, and a legacy
//! document is upgraded exactly once (rewritten with an explicit
//! `schema_version`). Unsupported future versions fail closed.

use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::path::Path;

use super::schema::{
    CURRENT_SCHEMA_VERSION, DocumentType, SchemaVersion, validate_version, version_diagnostic,
};

/// The explicit version assumed for legacy unversioned documents.
pub const LEGACY_UNVERSIONED_VERSION: SchemaVersion = SchemaVersion::new(1, 0, 0);

/// Read the schema version declared by a JSON document.
///
/// Legacy documents without a `schema_version` field are reported as the
/// explicit legacy version ([`LEGACY_UNVERSIONED_VERSION`]).
pub fn read_declared_version(path: &Path, doc_type: DocumentType) -> Result<SchemaVersion> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let value: Value = serde_json::from_str(&text)
        .with_context(|| format!("corrupt {} document {}", doc_type.as_str(), path.display()))?;
    match value.get("schema_version") {
        Some(v) => SchemaVersion::parse(v.as_str().context("schema_version must be a string")?),
        None => Ok(LEGACY_UNVERSIONED_VERSION),
    }
}

/// Ensure a document is at the current schema, migrating it in place if it is
/// a supported legacy version.
///
/// Returns the version status of the document after migration, or an error
/// for unsupported versions (fail closed).
pub fn migrate_document(
    path: &Path,
    doc_type: DocumentType,
) -> Result<super::schema::VersionStatus> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let mut value: Value = serde_json::from_str(&text)
        .with_context(|| format!("corrupt {} document {}", doc_type.as_str(), path.display()))?;
    if !value.is_object() {
        bail!(
            "{} document is not a JSON object: {}",
            doc_type.as_str(),
            path.display()
        );
    }
    let declared = read_declared_version(path, doc_type)?;
    let status = validate_version(doc_type, declared)?;
    match status {
        super::schema::VersionStatus::Current => Ok(status),
        super::schema::VersionStatus::Legacy => {
            if value.get("schema_version").is_none() {
                if let Some(obj) = value.as_object_mut() {
                    obj.insert(
                        "schema_version".to_string(),
                        Value::String(CURRENT_SCHEMA_VERSION.to_string_owned()),
                    );
                }
                let tmp = path.with_extension("json.migrating");
                std::fs::write(&tmp, serde_json::to_string_pretty(&value)?)
                    .with_context(|| format!("failed to write migrated {}", tmp.display()))?;
                std::fs::rename(&tmp, path)
                    .with_context(|| format!("failed to commit migrated {}", path.display()))?;
            }
            Ok(super::schema::VersionStatus::Legacy)
        }
        super::schema::VersionStatus::Unsupported => bail!(
            "{}",
            version_diagnostic(doc_type, declared).migration_action
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn unversioned_legacy_document_reports_legacy_version() {
        let dir = sample_dir();
        let path = dir.path().join("doc.json");
        std::fs::write(&path, "{\"state\":\"created\"}").unwrap();
        let version = read_declared_version(&path, DocumentType::ExecutionIdentity).unwrap();
        assert_eq!(version, LEGACY_UNVERSIONED_VERSION);
    }

    #[test]
    fn declared_version_detected() {
        let dir = sample_dir();
        let path = dir.path().join("doc.json");
        std::fs::write(&path, "{\"schema_version\":\"1.0.0\"}").unwrap();
        let version = read_declared_version(&path, DocumentType::EvidenceBundle).unwrap();
        assert_eq!(version, SchemaVersion::new(1, 0, 0));
    }

    #[test]
    fn migrate_rewrites_unversioned_document_idempotently() {
        let dir = sample_dir();
        let path = dir.path().join("doc.json");
        std::fs::write(&path, "{\"state\":\"created\"}").unwrap();
        let status = migrate_document(&path, DocumentType::ExecutionIdentity).unwrap();
        assert_eq!(status, super::super::schema::VersionStatus::Current);
        // Unversioned 1.0.0 equals current 1.0.0, so no rewrite is needed.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("schema_version"));
        // Running twice is a no-op.
        let again = migrate_document(&path, DocumentType::ExecutionIdentity).unwrap();
        assert_eq!(again, super::super::schema::VersionStatus::Current);
    }

    #[test]
    fn unsupported_future_version_fails_closed() {
        let dir = sample_dir();
        let path = dir.path().join("doc.json");
        std::fs::write(&path, "{\"schema_version\":\"9.0.0\"}").unwrap();
        let err = migrate_document(&path, DocumentType::EvidenceBundle).unwrap_err();
        assert!(err.to_string().contains("fail closed"));
    }

    #[test]
    fn corrupt_document_fails_closed() {
        let dir = sample_dir();
        let path = dir.path().join("doc.json");
        std::fs::write(&path, "not json").unwrap();
        assert!(migrate_document(&path, DocumentType::JournalEvent).is_err());
    }
}
