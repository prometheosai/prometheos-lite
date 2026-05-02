use crate::harness::{patch_applier::PatchFailure, validation::ValidationResult};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FailureKind {
    LocalizationFailure,
    PatchParseFailure,
    PatchApplyFailure,
    CompileFailure,
    TestFailure,
    RegressionFailure,
    SemanticFailure,
    TimeoutFailure,
    PermissionFailure,
    ModelFailure,
    ToolFailure,
    SandboxFailure,
}
pub fn classify_patch_failure(_: &PatchFailure) -> FailureKind {
    FailureKind::PatchApplyFailure
}
pub fn classify_validation_failure(_: &ValidationResult) -> FailureKind {
    FailureKind::TestFailure
}
