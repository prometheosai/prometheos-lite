use super::registry::NodeRegistry;

pub fn register_builtin_nodes(registry: &mut NodeRegistry) {
    for node_type in [
        "planner",
        "coder",
        "reviewer",
        "terminal",
        "llm",
        "tool",
        "file_writer",
        "context_loader",
        "memory_write",
        "conditional",
        "passthrough",
        "code_analysis",
        "symbol_resolution",
        "dependency_analysis",
    ] {
        registry.register(node_type);
    }
}
