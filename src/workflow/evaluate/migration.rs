//! Typed migration of legacy durable documents to the current schema.
//!
//! Legacy documents are unversioned and are assigned the explicit
//! [`LEGACY_UNVERSIONED_VERSION`] (`0.0.0`) — never the current version. A
//! mutable document (registry, identity, checkpoint, journal event) is
//! migrated in place exactly once: the `schema_version` field is injected and
//! the fully-typed result is validated before the atomic rewrite. Immutable
//! documents (evidence bundles) are validated in memory and never silently
//! rewritten. Unsupported future versions fail closed.

use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::path::Path;

use super::schema::{
    CURRENT_SCHEMA_VERSION, DocumentType, LEGACY_UNVERSIONED_VERSION, SchemaVersion,
    validate_version, version_diagnostic,
};

/// Read the schema version declared by a JSON document.
///
/// Legacy documents without a `schema_version` field are reported as the
/// explicit legacy version ([`LEGACY_UNVERSIONED_VERSION`]), which is distinct
/// from the current version so unversioned data is always migrated.
pub fn read_declared_version(path: &Path, doc_type: DocumentType) -> Result<SchemaVersion> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let value: Value = serde_json::from_str(&text)
        .with_context(|| format!("corrupt {} document {}", doc_type.as_str(), path.display()))?;
    read_declared_version_from_value(&value)
}

fn read_declared_version_from_value(value: &Value) -> Result<SchemaVersion> {
    match value.get("schema_version") {
        Some(v) => SchemaVersion::parse(v.as_str().context("schema_version must be a string")?),
        None => Ok(LEGACY_UNVERSIONED_VERSION),
    }
}

/// Validate the fully-migrated document against its typed representation.
///
/// A legacy document that survives this parse is complete and well-typed; a
/// document that fails is corrupt and must not be accepted as migrated.
fn typed_validate(doc_type: DocumentType, value: &Value) -> Result<()> {
    match doc_type {
        DocumentType::ProposalRegistry => {
            serde_json::from_value::<super::registry::ProposalRegistry>(value.clone()).map(|_| ())
        }
        DocumentType::ExecutionIdentity => {
            serde_json::from_value::<super::identity::ExecutionIdentity>(value.clone()).map(|_| ())
        }
        DocumentType::EvaluationCheckpoint => {
            serde_json::from_value::<super::checkpoint::EvaluationCheckpoint>(value.clone())
                .map(|_| ())
        }
        DocumentType::JournalEvent => {
            serde_json::from_value::<super::journal::JournalEvent>(value.clone()).map(|_| ())
        }
        DocumentType::EvidenceBundle => {
            serde_json::from_value::<super::evidence::EvidenceBundle>(value.clone()).map(|_| ())
        }
        DocumentType::PortableWorkState => {
            bail!("PortableWorkState documents are imported through workflow::portable_state, not the evaluation migration path")
        }
    }
    .with_context(|| format!("migrated {} document is invalid", doc_type.as_str()))
}

/// Ensure a document is at the current schema, migrating it in place if it is
/// a supported legacy version.
///
/// - Current versions are left untouched.
/// - Unversioned legacy (`0.0.0`) documents are migrated: the version field is
///   injected and the full typed document is validated before the atomic
///   rewrite. Running migration twice is a no-op (idempotent).
/// - Evidence bundles are immutable: a legacy bundle is validated in memory
///   but never rewritten.
/// - Unsupported future versions fail closed.
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
        super::schema::VersionStatus::Current => {
            // A schema marker alone is not validation. Every current document
            // is still type-validated; a malformed current document (missing
            // required fields) fails closed rather than being silently trusted.
            typed_validate(doc_type, &value)?;
            Ok(status)
        }
        super::schema::VersionStatus::Legacy => {
            if doc_type == DocumentType::EvidenceBundle {
                // Immutable evidence: validate the migrated form in memory,
                // never silently rewrite the completed artifact. The in-memory
                // version is upgraded so downstream readers see a current doc.
                let mut migrated = value.clone();
                if let Some(obj) = migrated.as_object_mut() {
                    obj.insert(
                        "schema_version".to_string(),
                        Value::String(CURRENT_SCHEMA_VERSION.to_string_owned()),
                    );
                }
                typed_validate(doc_type, &migrated)?;
                return Ok(status);
            }
            // Mutable documents: always migrate the schema version to CURRENT,
            // including documents that already carry an explicit older version
            // (e.g. "0.0.0"). The fully-typed form is validated before the
            // atomic rewrite, which happens only when the version actually
            // changed (so re-running migration is idempotent).
            let current = CURRENT_SCHEMA_VERSION.to_string_owned();
            let changed =
                value.get("schema_version").and_then(|v| v.as_str()) != Some(current.as_str());
            if let Some(obj) = value.as_object_mut() {
                obj.insert("schema_version".to_string(), Value::String(current.clone()));
            }
            typed_validate(doc_type, &value)?;
            if changed {
                super::durable::atomic_write_json(path, &value)
                    .with_context(|| format!("failed to commit migrated {}", path.display()))?;
            }
            Ok(super::schema::VersionStatus::Current)
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
        assert_eq!(version, SchemaVersion::new(0, 0, 0));
        assert!(!version.is_current());
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
        // A legacy unversioned ProposalRegistry.
        std::fs::write(&path, "{\"entries\":{}}").unwrap();
        let status = migrate_document(&path, DocumentType::ProposalRegistry).unwrap();
        assert_eq!(status, super::super::schema::VersionStatus::Current);
        // The version field must now be present and current.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("schema_version"));
        assert!(text.contains("\"1.0.0\""));
        // Running twice is a no-op.
        let again = migrate_document(&path, DocumentType::ProposalRegistry).unwrap();
        assert_eq!(again, super::super::schema::VersionStatus::Current);
        let text2 = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, text2);
    }

    #[test]
    fn migrate_current_document_is_noop() {
        let dir = sample_dir();
        let path = dir.path().join("doc.json");
        std::fs::write(&path, "{\"schema_version\":\"1.0.0\",\"entries\":{}}").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let status = migrate_document(&path, DocumentType::ProposalRegistry).unwrap();
        assert_eq!(status, super::super::schema::VersionStatus::Current);
        // Current document left byte-for-byte untouched.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }

    #[test]
    fn legacy_execution_identity_migrates_with_full_typed_validation() {
        let dir = sample_dir();
        let path = dir.path().join("doc.json");
        // Minimal unversioned identity: unversioned legacy must NOT parse as a
        // complete ExecutionIdentity (missing fields), so migration fails
        // closed instead of fabricating a valid document.
        std::fs::write(&path, "{\"state\":\"created\"}").unwrap();
        let err = migrate_document(&path, DocumentType::ExecutionIdentity).unwrap_err();
        assert!(err.to_string().contains("invalid"));
    }

    #[test]
    fn evidence_bundle_legacy_is_validated_but_never_rewritten() {
        let dir = sample_dir();
        let path = dir.path().join("doc.json");
        // A legacy evidence bundle without a version field but otherwise
        // complete enough to parse as an EvidenceBundle after in-memory
        // injection. The on-disk file must remain byte-identical.
        let bundle = serde_json::json!({
            "run_id": "run-1",
            "task_id": "task-1",
            "repo": "/tmp/repo",
            "repo_pin_before": "abc",
            "repo_pin_after": "abc",
            "provider_provenance": {
                "implementation": "mock",
                "model": null,
                "route": null,
                "generated_at": null,
                "input_digest": null,
                "patch_hash": null
            },
            "effective_governance": {
                "allowed_paths": [],
                "forbidden_paths": [],
                "allow_dependency_changes": false,
                "max_files_changed": null,
                "max_lines_changed": null,
                "authority": "propose",
                "validation_command": null
            },
            "proposal": null,
            "validation": null,
            "failure_classification": null,
            "integrity": null,
            "cleanup": null,
            "raw_logs": {"stdout": null, "stderr": null, "validation_output": null},
            "final_state": "in_progress",
            "completed_at": ""
        });
        let original = serde_json::to_string_pretty(&bundle).unwrap();
        std::fs::write(&path, &original).unwrap();
        let status = migrate_document(&path, DocumentType::EvidenceBundle).unwrap();
        assert_eq!(status, super::super::schema::VersionStatus::Legacy);
        // Immutable evidence: the file was NOT rewritten.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
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

    #[test]
    fn malformed_current_registry_fails() {
        let dir = sample_dir();
        let path = dir.path().join("doc.json");
        // A current-version marker is not validation: required fields are absent.
        std::fs::write(&path, "{\"schema_version\":\"1.0.0\"}").unwrap();
        let err = migrate_document(&path, DocumentType::ProposalRegistry).unwrap_err();
        assert!(err.to_string().contains("invalid"));
    }

    #[test]
    fn malformed_current_identity_fails() {
        let dir = sample_dir();
        let path = dir.path().join("doc.json");
        std::fs::write(&path, "{\"schema_version\":\"1.0.0\"}").unwrap();
        let err = migrate_document(&path, DocumentType::ExecutionIdentity).unwrap_err();
        assert!(err.to_string().contains("invalid"));
    }

    #[test]
    fn malformed_current_evidence_fails() {
        let dir = sample_dir();
        let path = dir.path().join("doc.json");
        std::fs::write(&path, "{\"schema_version\":\"1.0.0\"}").unwrap();
        let err = migrate_document(&path, DocumentType::EvidenceBundle).unwrap_err();
        assert!(err.to_string().contains("invalid"));
    }

    #[test]
    fn explicit_legacy_version_upgrades_to_current() {
        let dir = sample_dir();
        let path = dir.path().join("doc.json");
        // An explicit older version (0.0.0) with a complete body must be
        // upgraded to the current schema version, not left as-is.
        std::fs::write(&path, "{\"schema_version\":\"0.0.0\",\"entries\":{}}").unwrap();
        let status = migrate_document(&path, DocumentType::ProposalRegistry).unwrap();
        assert_eq!(status, super::super::schema::VersionStatus::Current);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("\"1.0.0\""),
            "explicit legacy version must upgrade to current: {text}"
        );
    }

    #[test]
    fn migration_is_idempotent() {
        let dir = sample_dir();
        let path = dir.path().join("doc.json");
        std::fs::write(&path, "{\"schema_version\":\"0.0.0\",\"entries\":{}}").unwrap();
        let first = migrate_document(&path, DocumentType::ProposalRegistry).unwrap();
        assert_eq!(first, super::super::schema::VersionStatus::Current);
        let text1 = std::fs::read_to_string(&path).unwrap();
        let second = migrate_document(&path, DocumentType::ProposalRegistry).unwrap();
        assert_eq!(second, super::super::schema::VersionStatus::Current);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            text1,
            "migrating an already-current document must be a no-op"
        );
    }
}
