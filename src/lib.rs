pub mod config;
pub mod flow;
pub mod fs;
pub mod llm;
pub mod logger;

// Legacy modules - deprecated in favor of flow-centric architecture
// Use the "legacy" feature flag to enable these for backward compatibility
#[cfg(feature = "legacy")]
pub mod legacy {
    pub mod agents;
    pub mod core;
}
