//! Deterministic, read-only projections of the canonical SOMA++ AST.
//!
//! Slice 1 ships the versioned envelope and the canonical JSON view. The
//! human plan, verify path, and disclosure policy are added by the tasks
//! that own them.

pub mod envelope;

pub use envelope::{
    ALLOWED_PROJECTION_VERSIONS, PROJECTION_VERSION_V1, VersionedProjectionEnvelope,
};
