//! Flow Core - The execution engine for PrometheOS Lite v1.1
//!
//! This module implements the flow-centric architecture where everything is a Flow,
//! execution equals node lifecycle, and state is explicit.

mod adapter;
pub mod budget;
pub mod debug;
pub mod execution;
pub mod factory;
pub mod intelligence;
pub mod loader;
pub mod memory;
pub mod migration;
pub mod node;
pub mod output;
pub mod runtime;
pub mod testing;
pub mod tracing;
pub mod types;

pub use adapter::*;
pub use budget::*;
pub use debug::*;
pub use execution::*;
pub use factory::{NodeFactory, DefaultNodeFactory, IdWrapper, PlannerNode, CoderNode, ReviewerNode, LlmNode, ToolNode, FileWriterNode, ContextLoaderNode, MemoryWriteNode, ConditionalNode, PassthroughNode};
pub use intelligence::*;
pub use loader::*;
pub use memory::*;
#[cfg(feature = "legacy")]
pub use migration::*;
pub use node::*;
pub use output::*;
pub use runtime::*;
pub use testing::*;
pub use tracing::*;
pub use types::*;
