//! Memory layer - SQLite-based storage with semantic search and embedding support

mod db;
mod embedding;
mod nodes;
mod scoring;
mod service;
mod summarizer;
mod types;
mod vector;

pub use db::MemoryDb;
pub use embedding::{EmbeddingProvider, FallbackEmbeddingProvider, LocalEmbeddingProvider};
pub use nodes::{ContextLoaderNode, MemoryExtractorNode, MemoryWriteNode};
pub use scoring::{MemoryScore, prune, prune_by_threshold, prune_combined, rank_memories};
pub use service::MemoryService;
pub use summarizer::MemorySummarizer;
pub use types::{
    ContextBundle, Memory, MemoryKind, MemoryRelationship, MemoryType, MemoryWriteTask,
};
pub use vector::{BruteForceBackend, InMemoryVectorIndex, VectorSearchBackend};
