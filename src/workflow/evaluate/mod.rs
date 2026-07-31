//! Fast Governed Loop V1 — automated evaluation pipeline.
//!
//! Takes a task from definition through `REVIEW_GATE`, producing a trustworthy
//! evidence bundle. The human still makes the final correctness decision.
//!
//! ```text
//! preflight
//! → generate exactly once
//! → inspect governance
//! → isolated dry-run
//! → classify infrastructure vs patch failures
//! → verify repository integrity
//! → write structured result
//! → stop at REVIEW_GATE
//! ```
//!
//! No automatic approval, patch application, commit creation, push, or
//! pull-request creation.

mod cleanup;
mod evidence;
mod generation;
mod identity;
mod integrity;
mod orchestrator;
mod preflight;
mod registry;
mod validation;

pub use evidence::{
    CleanupRecord, EvidenceBundle, IntegrityRecord, ProposalRecord, ProviderProvenanceRecord,
    RawLogPaths, ValidationRecord,
};
pub use identity::{
    EvaluationState, ExecutionIdentity, GovernanceScopeSnapshot, TaskManifest,
    compute_identity_key, now_iso,
};
pub use integrity::verify_repo_integrity;
pub use orchestrator::{EvaluationConfig, evaluate};
pub use preflight::{DiskSpaceStatus, PreflightResult, available_disk_bytes};
pub use registry::{FenceToken, LeaseConfig, ProposalRegistry, ProposalState, RegistryEntry};
pub use validation::classify_validation_failure;
