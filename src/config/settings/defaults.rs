//! Default configuration values

use super::types::{MemoryBudget, StrictMode};

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

pub fn default_strict_mode() -> StrictMode {
    StrictMode::default()
}

pub fn default_strict_mode_enforce_missing_inputs() -> bool {
    false
}

pub fn default_strict_mode_enforce_missing_services() -> bool {
    false
}

pub fn default_strict_mode_enforce_empty_outputs() -> bool {
    false
}

pub fn default_strict_mode_enforce_no_unwrap() -> bool {
    false
}

pub fn default_strict_mode_enforce_no_silent_none() -> bool {
    false
}

pub fn default_strict_mode_enforce_idempotency() -> bool {
    false
}

pub fn default_repo_path() -> String {
    ".".to_string()
}
