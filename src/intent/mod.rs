//! Intent Classification Module
//!
//! This module provides intent classification for routing user messages
//! to appropriate handlers (direct LLM or code generation flow).

pub mod classifier;
pub mod flow_selector;
pub mod router;
pub mod rules;
pub mod types;

pub use classifier::IntentClassifier;
pub use flow_selector::{DefaultFlowSelector, FlowPath, FlowSelector};
pub use router::IntentRouter;
pub use types::{Handler, Intent, IntentClassificationResult};
