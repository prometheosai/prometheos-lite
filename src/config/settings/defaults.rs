//! Default configuration values

use super::types::MemoryBudget;

pub fn default_embedding_url() -> String {
    "http://localhost:11434".to_string()
}

pub fn default_embedding_dimension() -> usize {
    1536
}

pub fn default_memory_db_path() -> String {
    "prometheos_memory.db".to_string()
}

pub fn default_context_window_size() -> usize {
    4096
}

pub fn default_memory_budget() -> MemoryBudget {
    MemoryBudget {
        project_facts: 0.4,
        user_preferences: 0.25,
        recent_episodes: 0.2,
        decisions_constraints: 0.15,
    }
}
