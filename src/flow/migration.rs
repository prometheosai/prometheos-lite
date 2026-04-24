//! Migration utilities for transitioning from agent-based to flow-based architecture

#[cfg(feature = "legacy")]
use crate::legacy::agents::{Agent, CoderAgent, PlannerAgent, ReviewerAgent};
#[cfg(feature = "legacy")]
use crate::flow::AgentNode;
#[cfg(feature = "legacy")]
use crate::flow::Flow;
#[cfg(feature = "legacy")]
use crate::flow::SharedState;
#[cfg(feature = "legacy")]
use anyhow::Result;
#[cfg(feature = "legacy")]
use crate::llm::LlmClient;
#[cfg(feature = "legacy")]
use std::sync::Arc;

/// Create a Flow that replicates the current Planner → Coder → Reviewer sequence
#[cfg(feature = "legacy")]
pub fn create_sequential_agent_flow(llm: LlmClient) -> Flow {
    let planner = Arc::new(PlannerAgent::new(llm.clone())) as Arc<dyn Agent>;
    let coder = Arc::new(CoderAgent::new(llm.clone())) as Arc<dyn Agent>;
    let reviewer = Arc::new(ReviewerAgent::new(llm)) as Arc<dyn Agent>;

    Flow::builder()
        .start("planner".to_string())
        .add_node(
            "planner".to_string(),
            Arc::new(AgentNode::new("planner".to_string(), planner)),
        )
        .add_node(
            "coder".to_string(),
            Arc::new(AgentNode::new("coder".to_string(), coder)),
        )
        .add_node(
            "reviewer".to_string(),
            Arc::new(AgentNode::new("reviewer".to_string(), reviewer)),
        )
        .add_transition(
            "planner".to_string(),
            "continue".to_string(),
            "coder".to_string(),
        )
        .add_transition(
            "coder".to_string(),
            "continue".to_string(),
            "reviewer".to_string(),
        )
        .build()
        .expect("Failed to build sequential agent flow")
}

#[cfg(all(test, feature = "legacy"))]
mod parity_tests {
    use super::*;
    use crate::legacy::agents::Agent;
    use anyhow::Result;
    use async_trait::async_trait;

    // Mock agents for testing without actual LLM calls
    struct MockPlanner;
    struct MockCoder;
    struct MockReviewer;

    #[async_trait]
    impl Agent for MockPlanner {
        fn name(&self) -> &str {
            "planner"
        }

        async fn run(&self, input: &str) -> Result<String> {
            Ok(format!("PLAN: {}", input))
        }
    }

    #[async_trait]
    impl Agent for MockCoder {
        fn name(&self) -> &str {
            "builder"
        }

        async fn run(&self, input: &str) -> Result<String> {
            Ok(format!("CODE: {}", input))
        }
    }

    #[async_trait]
    impl Agent for MockReviewer {
        fn name(&self) -> &str {
            "reviewer"
        }

        async fn run(&self, input: &str) -> Result<String> {
            Ok(format!("REVIEW: {}", input))
        }
    }

    #[tokio::test]
    async fn test_sequential_flow_structure() {
        let planner = Arc::new(MockPlanner) as Arc<dyn Agent>;
        let coder = Arc::new(MockCoder) as Arc<dyn Agent>;
        let reviewer = Arc::new(MockReviewer) as Arc<dyn Agent>;

        let mut flow = Flow::builder()
            .start("planner".to_string())
            .add_node(
                "planner".to_string(),
                Arc::new(AgentNode::new("planner".to_string(), planner)),
            )
            .add_node(
                "coder".to_string(),
                Arc::new(AgentNode::new("coder".to_string(), coder)),
            )
            .add_node(
                "reviewer".to_string(),
                Arc::new(AgentNode::new("reviewer".to_string(), reviewer)),
            )
            .add_transition(
                "planner".to_string(),
                "continue".to_string(),
                "coder".to_string(),
            )
            .add_transition(
                "coder".to_string(),
                "continue".to_string(),
                "reviewer".to_string(),
            )
            .build()
            .expect("Failed to build flow");

        let mut state = SharedState::new();
        state.set_input("task".to_string(), serde_json::json!("test task"));

        flow.run(&mut state).await.expect("Flow execution failed");

        // Verify all agents executed
        assert!(state.get_meta("plan").is_some());
        assert!(state.get_meta("generated_output").is_some());
        assert!(state.get_meta("review").is_some());

        // Verify the sequence
        let plan = state.get_meta("plan").and_then(|v| v.as_str()).unwrap();
        let code = state
            .get_meta("generated_output")
            .and_then(|v| v.as_str())
            .unwrap();
        let review = state.get_meta("review").and_then(|v| v.as_str()).unwrap();

        assert!(plan.contains("PLAN:"));
        assert!(code.contains("CODE:"));
        assert!(review.contains("REVIEW:"));
    }

    #[tokio::test]
    async fn test_flow_parity_with_sequential_execution() {
        // This test verifies that the Flow produces equivalent output to the old SequentialOrchestrator
        // In a real scenario, this would compare actual outputs from the same LLM

        let planner = Arc::new(MockPlanner) as Arc<dyn Agent>;
        let coder = Arc::new(MockCoder) as Arc<dyn Agent>;
        let reviewer = Arc::new(MockReviewer) as Arc<dyn Agent>;

        // Execute via Flow
        let mut flow = Flow::builder()
            .start("planner".to_string())
            .add_node(
                "planner".to_string(),
                Arc::new(AgentNode::new("planner".to_string(), planner.clone())),
            )
            .add_node(
                "coder".to_string(),
                Arc::new(AgentNode::new("coder".to_string(), coder.clone())),
            )
            .add_node(
                "reviewer".to_string(),
                Arc::new(AgentNode::new("reviewer".to_string(), reviewer.clone())),
            )
            .add_transition(
                "planner".to_string(),
                "continue".to_string(),
                "coder".to_string(),
            )
            .add_transition(
                "coder".to_string(),
                "continue".to_string(),
                "reviewer".to_string(),
            )
            .build()
            .expect("Failed to build flow");

        let mut flow_state = SharedState::new();
        flow_state.set_input("task".to_string(), serde_json::json!("test task"));

        flow.run(&mut flow_state)
            .await
            .expect("Flow execution failed");

        // Execute sequentially (old way)
        let task = "test task";
        let plan = planner.run(task).await.unwrap();
        let code = coder.run(&plan).await.unwrap();
        let review = reviewer.run(&code).await.unwrap();

        // Compare outputs
        let flow_plan = flow_state
            .get_meta("plan")
            .and_then(|v| v.as_str())
            .unwrap();
        let flow_code = flow_state
            .get_meta("generated_output")
            .and_then(|v| v.as_str())
            .unwrap();
        let flow_review = flow_state
            .get_meta("review")
            .and_then(|v| v.as_str())
            .unwrap();

        assert_eq!(flow_plan, plan);
        assert_eq!(flow_code, code);
        assert_eq!(flow_review, review);
    }

    #[test]
    fn test_flow_validation() {
        let planner = Arc::new(MockPlanner) as Arc<dyn Agent>;
        let coder = Arc::new(MockCoder) as Arc<dyn Agent>;
        let reviewer = Arc::new(MockReviewer) as Arc<dyn Agent>;

        // Valid flow
        let valid_flow = Flow::builder()
            .start("planner".to_string())
            .add_node(
                "planner".to_string(),
                Arc::new(AgentNode::new("planner".to_string(), planner.clone())),
            )
            .add_node(
                "coder".to_string(),
                Arc::new(AgentNode::new("coder".to_string(), coder.clone())),
            )
            .add_node(
                "reviewer".to_string(),
                Arc::new(AgentNode::new("reviewer".to_string(), reviewer.clone())),
            )
            .add_transition(
                "planner".to_string(),
                "continue".to_string(),
                "coder".to_string(),
            )
            .add_transition(
                "coder".to_string(),
                "continue".to_string(),
                "reviewer".to_string(),
            )
            .build();

        assert!(valid_flow.is_ok());

        // Invalid flow - unreachable node
        let invalid_flow = Flow::builder()
            .start("planner".to_string())
            .add_node(
                "planner".to_string(),
                Arc::new(AgentNode::new("planner".to_string(), planner)),
            )
            .add_node(
                "coder".to_string(),
                Arc::new(AgentNode::new("coder".to_string(), coder)),
            )
            .add_node(
                "reviewer".to_string(),
                Arc::new(AgentNode::new("reviewer".to_string(), reviewer)),
            )
            .add_transition(
                "planner".to_string(),
                "continue".to_string(),
                "coder".to_string(),
            )
            .build();

        assert!(invalid_flow.is_err());
    }
}
