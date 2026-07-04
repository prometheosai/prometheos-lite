//! Flow Core - The execution engine for PrometheOS Lite v1.1
//!
//! This module implements the flow-centric architecture where everything is a Flow,
//! execution equals node lifecycle, and state is explicit.

#[cfg(feature = "legacy")]
pub mod adapter;
pub mod budget;
pub mod debug;
pub mod execution;
pub mod execution_service;
pub mod factory;
pub mod idempotency;
pub mod intelligence;
pub mod loader;
pub mod loop_detection;
pub mod memory;
pub mod migration;
pub mod node;
pub mod opentelemetry;
pub mod output;
pub mod runtime;
pub mod snapshot;
pub mod strict_mode;
pub mod testing;
pub mod tracing;
pub mod types;

pub use budget::*;
pub use debug::*;
pub use execution::*;
pub use execution_service::*;
pub use factory::{
    CoderNode, ConditionalNode, ContextLoaderNode, DefaultNodeFactory, FileWriterNode, IdWrapper,
    LlmNode, MemoryWriteNode, NodeFactory, PassthroughNode, PlannerNode, ReviewerNode,
    TerminalNode, ToolNode,
};
pub use idempotency::*;
pub use intelligence::{
    ModelRouter, Tool, ToolInput, ToolOutput, ToolRegistry, ToolRuntime, ToolSandboxProfile,
};
pub use loader::*;
pub use loop_detection::*;
pub use memory::*;
#[cfg(feature = "legacy")]
pub use migration::*;
pub use node::*;
pub use opentelemetry::*;
pub use output::*;
pub use runtime::*;
pub use snapshot::*;
pub use strict_mode::*;
pub use testing::*;
pub use tracing::*;
pub use types::*;
