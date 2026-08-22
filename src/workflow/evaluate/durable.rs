//! Compatibility shim.
//!
//! The crash-safe atomic JSON publication machinery now lives at
//! [`crate::workflow::durable`]. This module re-exports it so existing
//! evaluation code and tests are unchanged.

pub use crate::workflow::durable::{
    atomic_write_json, confined_workflow_dir, repo_relative_path, resolve_repo_relative,
    versioned_write_json,
};
