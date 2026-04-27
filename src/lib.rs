pub mod api;
pub mod config;
pub mod control;
pub mod db;
pub mod flow;
pub mod fs;
pub mod intent;
pub mod llm;
pub mod logger;
pub mod personality;
pub mod tools;
pub mod utils;

// Legacy modules - deprecated in favor of flow-centric architecture
// Use the "legacy" feature flag to enable these for backward compatibility
#[cfg(feature = "legacy")]
pub mod legacy {
    pub mod agents;
    pub mod core;
}
