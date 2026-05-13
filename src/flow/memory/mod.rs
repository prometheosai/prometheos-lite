//! Memory layer - SQLite-based storage with semantic search and embedding support

pub mod db;
pub mod embedding;
pub mod nodes;
pub mod scoring;
pub mod service;
pub mod summarizer;
pub mod types;
pub mod vector;

pub use db::MemoryDb;
pub use embedding::{EmbeddingProvider, FallbackEmbeddingProvider, JinaEmbeddingProvider, LocalEmbeddingProvider};
pub use nodes::{ContextLoaderNode, MemoryExtractorNode, MemoryWriteNode};
pub use scoring::{MemoryScore, prune, prune_by_threshold, prune_combined, rank_memories};
pub use service::MemoryService;
pub use summarizer::MemorySummarizer;
pub use types::{
    ContextBundle, Memory, MemoryKind, MemoryRelationship, MemoryType, MemoryWriteTask,
};
pub use vector::{BruteForceBackend, InMemoryVectorIndex, VectorSearchBackend};
