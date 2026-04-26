//! Tests for CLI runner

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flow_file_serialization() {
        let flow_file = super::types::FlowFile {
            name: "test_flow".to_string(),
            description: Some("A test flow".to_string()),
            start_node: "node1".to_string(),
            nodes: vec![super::types::NodeDefinition {
                id: "node1".to_string(),
                node_type: "placeholder".to_string(),
                config: None,
            }],
            transitions: vec![],
        };

        let json = serde_json::to_string(&flow_file).unwrap();
        let parsed: super::types::FlowFile = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.name, "test_flow");
        assert_eq!(parsed.start_node, "node1");
    }

    #[tokio::test]
    async fn test_flow_runner() {
        let node = std::sync::Arc::new(super::nodes::PassthroughNode::new(prometheos_lite::flow::NodeConfig::default()));
        let wrapped = std::sync::Arc::new(super::nodes::IdWrapper::new("test".to_string(), node));

        let flow = prometheos_lite::flow::FlowBuilder::new()
            .start("test".to_string())
            .add_node("test".to_string(), wrapped)
            .build()
            .unwrap();

        let mut runner = super::runner::FlowRunner::new(flow);

        let mut state = prometheos_lite::flow::SharedState::new();
        state.set_input("test".to_string(), serde_json::json!("value"));

        let result = runner.run(&mut state).await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_node_factory_llm() {
        let factory: Box<dyn super::factory::NodeFactory> = Box::new(super::factory::DefaultNodeFactory::new());
        let node = factory.create("llm", None).unwrap();
        assert_eq!(node.id(), "llm");
    }

    #[test]
    fn test_node_factory_tool() {
        let factory: Box<dyn super::factory::NodeFactory> = Box::new(super::factory::DefaultNodeFactory::new());
        let node = factory.create("tool", None).unwrap();
        assert_eq!(node.id(), "tool");
    }

    #[test]
    fn test_node_factory_context_loader() {
        let factory: Box<dyn super::factory::NodeFactory> = Box::new(super::factory::DefaultNodeFactory::new());
        let node = factory.create("context_loader", None).unwrap();
        assert_eq!(node.id(), "context_loader");
    }

    #[test]
    fn test_node_factory_memory_write() {
        let factory: Box<dyn super::factory::NodeFactory> = Box::new(super::factory::DefaultNodeFactory::new());
        let node = factory.create("memory_write", None).unwrap();
        assert_eq!(node.id(), "memory_write");
    }

    #[test]
    fn test_node_factory_conditional() {
        let factory: Box<dyn super::factory::NodeFactory> = Box::new(super::factory::DefaultNodeFactory::new());
        let node = factory.create("conditional", None).unwrap();
        assert_eq!(node.id(), "conditional");
    }

    #[test]
    fn test_node_factory_planner() {
        let factory: Box<dyn super::factory::NodeFactory> = Box::new(super::factory::DefaultNodeFactory::new());
        let node = factory.create("planner", None).unwrap();
        assert_eq!(node.id(), "planner");
    }

    #[test]
    fn test_node_factory_coder() {
        let factory: Box<dyn super::factory::NodeFactory> = Box::new(super::factory::DefaultNodeFactory::new());
        let node = factory.create("coder", None).unwrap();
        assert_eq!(node.id(), "coder");
    }

    #[test]
    fn test_node_factory_reviewer() {
        let factory: Box<dyn super::factory::NodeFactory> = Box::new(super::factory::DefaultNodeFactory::new());
        let node = factory.create("reviewer", None).unwrap();
        assert_eq!(node.id(), "reviewer");
    }

    #[test]
    fn test_node_factory_with_model_router() {
        let factory = super::factory::DefaultNodeFactory::new();
        assert!(factory.model_router.is_none());
    }

    #[test]
    fn test_node_factory_file_writer() {
        let factory: Box<dyn super::factory::NodeFactory> = Box::new(super::factory::DefaultNodeFactory::new());
        let node = factory.create("file_writer", None).unwrap();
        assert_eq!(node.id(), "file_writer");
    }

    #[test]
    fn test_node_factory_unknown_type() {
        let factory: Box<dyn super::factory::NodeFactory> = Box::new(super::factory::DefaultNodeFactory::new());
        let node = factory.create("unknown_type", None).unwrap();
        assert_eq!(node.id(), "passthrough");
    }

    #[test]
    fn test_node_factory_config_parsing() {
        let factory: Box<dyn super::factory::NodeFactory> = Box::new(super::factory::DefaultNodeFactory::new());
        let config = serde_json::json!({
            "retries": 5,
            "retry_delay_ms": 200,
            "timeout_ms": 60000
        });

        let node = factory.create("llm", Some(config)).unwrap();
        let node_config = node.config();
        assert_eq!(node_config.retries, 5);
        assert_eq!(node_config.retry_delay_ms, 200);
        assert_eq!(node_config.timeout_ms, Some(60000));
    }
}
