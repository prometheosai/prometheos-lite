//! Memory layer - SQLite-based storage with semantic search and embedding support

mod types;
mod db;
mod service;
mod embedding;
mod vector;
mod nodes;

pub use types::{Memory, MemoryKind, MemoryType, ContextBundle, MemoryRelationship, MemoryWriteTask};
pub use db::MemoryDb;
pub use service::MemoryService;
pub use embedding::{EmbeddingProvider, LocalEmbeddingProvider, FallbackEmbeddingProvider};
pub use vector::{VectorSearchBackend, InMemoryVectorIndex, BruteForceBackend};
pub use nodes::{MemoryExtractorNode, ContextLoaderNode, MemoryWriteNode};
