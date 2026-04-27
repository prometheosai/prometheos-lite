//! Database module for UI state persistence
//!
//! This module provides SQLite-based persistence for projects, conversations,
//! messages, flow runs, and artifacts using rusqlite.

pub mod models;
pub mod repository;

pub use models::{
    Artifact, Conversation, CreateConversation, CreateMessage, CreateProject, FlowRun, Message,
    Project, RunFlow,
};
pub use repository::Db;
pub use repository::Repository;
