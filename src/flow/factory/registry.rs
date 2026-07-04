use std::collections::HashMap;

/// NodeRegistry maps external node names to canonical node types.
#[derive(Debug, Clone, Default)]
pub struct NodeRegistry {
    aliases: HashMap<String, String>,
}

impl NodeRegistry {
    pub fn new() -> Self {
        Self {
            aliases: HashMap::new(),
        }
    }

    pub fn register(&mut self, node_type: &str) {
        self.aliases
            .insert(node_type.to_string(), node_type.to_string());
    }

    pub fn register_alias(&mut self, alias: &str, canonical: &str) {
        self.aliases
            .insert(alias.to_string(), canonical.to_string());
    }

    pub fn resolve(&self, node_type: &str) -> Option<&str> {
        self.aliases.get(node_type).map(|s| s.as_str())
    }
}
