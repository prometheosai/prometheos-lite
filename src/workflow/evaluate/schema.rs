//! Compatibility shim.
//!
//! The durable document schema versioning machinery now lives at
//! [`crate::workflow::schema`] so it can be shared by the evaluation pipeline
//! and the portable work state contract. This module re-exports it so existing
//! evaluation code and tests are unchanged.

pub use crate::workflow::schema::{
    CURRENT_SCHEMA_VERSION, DocumentType, LEGACY_UNVERSIONED_VERSION, SchemaVersion, VersionStatus,
    validate_version, version_diagnostic,
};
