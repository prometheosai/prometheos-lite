//! Database module for UI state persistence
//!
//! This module provides SQLite-based persistence for projects, conversations,
//! messages, flow runs, and artifacts using rusqlite.

pub mod models;
pub mod repository;

pub use models::{Project, Conversation, Message, FlowRun, Artifact, CreateProject, CreateConversation, CreateMessage, RunFlow};
pub use repository::{Db, Repository};
