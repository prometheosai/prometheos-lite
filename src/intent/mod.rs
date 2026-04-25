//! Intent Classification Module
//!
//! This module provides intent classification for routing user messages
//! to appropriate handlers (direct LLM or code generation flow).

pub mod types;
pub mod rules;
pub mod classifier;
pub mod router;

pub use types::{Intent, IntentClassificationResult, Handler};
pub use classifier::IntentClassifier;
pub use router::IntentRouter;
