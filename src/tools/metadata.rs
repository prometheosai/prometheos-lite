//! Tool metadata system

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Metadata for a tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolMetadata {
    /// Unique identifier for the tool
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Description of what the tool does
    pub description: String,
    /// Schema hash for validation
    pub schema_hash: String,
    /// Version of the tool
    pub version: String,
    /// Additional metadata
    pub metadata: HashMap<String, serde_json::Value>,
}

impl ToolMetadata {
    /// Create new tool metadata
    pub fn new(id: String, name: String, description: String) -> Self {
        Self {
            id,
            name,
            description,
            schema_hash: String::new(),
            version: "1.0.0".to_string(),
            metadata: HashMap::new(),
        }
    }

    /// Set the schema hash
    pub fn with_schema_hash(mut self, hash: String) -> Self {
        self.schema_hash = hash;
        self
    }

    /// Set the version
    pub fn with_version(mut self, version: String) -> Self {
        self.version = version;
        self
    }

    /// Add metadata entry
    pub fn with_metadata(mut self, key: String, value: serde_json::Value) -> Self {
        self.metadata.insert(key, value);
        self
    }

    /// Generate schema hash from a JSON schema
    pub fn generate_schema_hash(schema: &serde_json::Value) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let schema_str = serde_json::to_string(schema).unwrap_or_default();
        let mut hasher = DefaultHasher::new();
        schema_str.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }
}
