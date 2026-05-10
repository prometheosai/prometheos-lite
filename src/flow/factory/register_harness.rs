use super::registry::NodeRegistry;

pub fn register_harness_nodes(registry: &mut NodeRegistry) {
    // V1.6.1 harness node names are hard-mapped to existing execution nodes.
    registry.register_alias("harness.repo_map", "code_analysis");
    registry.register_alias("harness.patch_apply", "tool");
    registry.register_alias("harness.validate", "tool");
    registry.register_alias("harness.review", "reviewer");
    registry.register_alias("harness.risk", "reviewer");
    registry.register_alias("harness.completion", "terminal");
    registry.register_alias("harness.attempt_pool", "conditional");
    registry.register_alias("harness.context_distill", "context_loader");
}
