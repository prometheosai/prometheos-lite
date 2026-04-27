//! Intent Classification Module
//!
//! This module provides intent classification for routing user messages
//! to appropriate handlers (direct LLM or code generation flow).

pub mod types;
pub mod rules;
pub mod classifier;
pub mod router;
pub mod flow_selector;

pub use types::{Intent, IntentClassificationResult, Handler};
pub use classifier::IntentClassifier;
pub use router::IntentRouter;
pub use flow_selector::{FlowSelector, DefaultFlowSelector, FlowPath};
