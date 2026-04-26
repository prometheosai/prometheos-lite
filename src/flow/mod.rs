//! Flow Core - The execution engine for PrometheOS Lite v1.1
//!
//! This module implements the flow-centric architecture where everything is a Flow,
//! execution equals node lifecycle, and state is explicit.

mod adapter;
pub mod debug;
pub mod execution;
pub mod intelligence;
pub mod memory;
pub mod migration;
pub mod node;
pub mod runtime;
pub mod tracing;
pub mod types;

pub use adapter::*;
pub use debug::*;
pub use execution::*;
pub use intelligence::*;
pub use memory::*;
#[cfg(feature = "legacy")]
pub use migration::*;
pub use node::*;
pub use runtime::*;
pub use tracing::*;
pub use types::*;
