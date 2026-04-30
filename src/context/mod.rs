//! Context management for LLM prompt construction and token budgeting
//!
//! This module provides:
//! - ContextBudgeter: Token budget management and context trimming
//! - ContextBuilder: Unified context construction across all nodes
//! - Token estimation utilities

pub mod budgeter;
pub mod builder;

pub use budgeter::*;
pub use builder::*;
