use super::registry::NodeRegistry;

pub fn register_harness_nodes(registry: &mut NodeRegistry) {
    registry.register("harness.repo_map");
    registry.register("harness.patch_apply");
    registry.register("harness.validate");
    registry.register("harness.review");
    registry.register("harness.risk");
    registry.register("harness.completion");
    registry.register("harness.attempt_pool");
    registry.register("harness.context_distill");
}
