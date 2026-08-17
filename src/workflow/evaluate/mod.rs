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

mod cancellation;
mod checkpoint;
mod cleanup;
mod durable;
mod evidence;
mod generation;
mod heartbeat;
mod identity;
mod integrity;
mod journal;
mod lock;
mod migration;
mod orchestrator;
mod preflight;
mod recovery;
mod registry;
mod resource;
mod schema;
mod transition;
mod validation;

pub use cancellation::CancellationToken;
pub use checkpoint::{EvaluationCheckpoint, read_checkpoint, write_checkpoint};
pub use evidence::{
    CleanupRecord, EvidenceBundle, IntegrityRecord, ProposalRecord, ProviderProvenanceRecord,
    RawLogPaths, ValidationRecord,
};
pub use identity::{
    EvaluationState, ExecutionIdentity, GovernanceScopeSnapshot, TaskManifest,
    compute_identity_key, now_iso,
};
pub use integrity::verify_repo_integrity;
pub use journal::{JournalEvent, read_journal};
pub use lock::WorkflowFileLock;
pub use orchestrator::{EvaluationConfig, evaluate, evaluate_with_cancellation};
pub use preflight::{DiskSpaceStatus, PreflightResult, available_disk_bytes};
pub use recovery::{
    RecoveredEvaluation, RecoveryDisposition, determine_recovery_disposition, ensure_resumable,
    recover_evaluation,
};
pub use registry::{
    FenceToken, LeaseConfig, OwnershipObservation, ProposalRegistry, ProposalState, RegistryEntry,
    TakeoverResult, is_entry_stale_at, try_take_ownership, try_take_ownership_cas,
};
pub use resource::{ResourceLimitKind, ResourceLimits, classification_for_resource};
pub use transition::validate_transition;
pub use validation::classify_validation_failure;
