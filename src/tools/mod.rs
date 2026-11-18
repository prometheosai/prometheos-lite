//! Tool permissions and metadata system

mod coding;
mod command;
mod context;
mod interrupt;
mod metadata;
mod path_guard;
mod permissions;
mod repo;
mod trust;

pub use coding::{GetFileInfoTool, ListFilesTool, ReadFileTool, SearchCodeTool};
pub use command::{CommandTool, RunTestsTool};
pub use context::{ApprovalPolicy, ToolContext, TrustLevel};
pub use interrupt::{InterruptContext, InterruptStatus};
pub use metadata::ToolMetadata;
pub use path_guard::PathGuard;
pub use permissions::{ToolPermission, ToolPolicy};
#[doc(hidden)] // test support: exercised by tests/security_regression_slice_a.rs
pub use repo::open_write_fail_closed;
pub use repo::{
    GitDiffTool, ListTreeTool, PatchFileTool, RepoReadFileTool, RepoTool, SearchFilesTool,
    WriteFileTool,
};
pub use trust::{TrustPolicy, TrustRegistry};
