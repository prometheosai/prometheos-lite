//! Shared test helpers for the GovernancePermit slice (T5).
//!
//! Each integration-test crate is standalone, so a governed NodeRunner
//! needs a permit, and a permit needs an audit-clean workflow. Building
//! that workflow per test file would scatter the same contract; this
//! module holds it once.

pub mod governance_permit;
/// Shared executed-task fixture runner (Slices B and C). Each executed
/// scenario suite pins its own [`task_execution::TaskExecutionSpec`].
pub mod task_execution;

pub use governance_permit::permit_for;
