//! Integration tests for API flow execution
//! Tests the Intent → FlowSelector → FlowRunner path

use prometheos_lite::flow::SharedState;
use prometheos_lite::flow::execution::Flow;
use prometheos_lite::flow::loader::{FlowLoader, YamlLoader};
use prometheos_lite::flow::{NodeFactory, factory::DefaultNodeFactory};
use prometheos_lite::intent::{DefaultFlowSelector, FlowSelector, Intent, IntentClassifier};
use std::path::PathBuf;

#[test]
fn test_flow_selector_chat_intent() {
    let selector = DefaultFlowSelector::with_default_dir();
    let intent = Intent::Conversation;

    let flow_path = selector.select_flow(&intent).unwrap();
    assert!(flow_path.ends_with("chat.flow.yaml"));
    assert!(flow_path.exists());
}

#[test]
fn test_flow_selector_planning_intent() {
    let selector = DefaultFlowSelector::with_default_dir();
    let intent = Intent::Planning;

    let flow_path = selector.select_flow(&intent).unwrap();
    assert!(flow_path.ends_with("planning.flow.yaml"));
    assert!(flow_path.exists());
}

#[test]
fn test_flow_selector_codegen_intent() {
    let selector = DefaultFlowSelector::with_default_dir();
    let intent = Intent::CodingTask;

    let flow_path = selector.select_flow(&intent).unwrap();
    assert!(flow_path.ends_with("codegen.flow.yaml"));
    assert!(flow_path.exists());
}

#[test]
fn test_flow_selector_approval_intent() {
    let selector = DefaultFlowSelector::with_default_dir();
    let intent = Intent::Approval;

    let flow_path = selector.select_flow(&intent).unwrap();
    assert!(flow_path.ends_with("approval.flow.yaml"));
    assert!(flow_path.exists());
}

#[test]
fn test_flow_selector_question_intent() {
    let selector = DefaultFlowSelector::with_default_dir();
    let intent = Intent::Question;

    let flow_path = selector.select_flow(&intent).unwrap();
    assert!(flow_path.ends_with("chat.flow.yaml"));
    assert!(flow_path.exists());
}

#[test]
fn test_load_chat_flow_yaml() {
    let loader = YamlLoader::new();
    let flow_path = PathBuf::from("flows/chat.flow.yaml");

    let flow_file = loader.load_from_path(&flow_path).unwrap();
    assert_eq!(flow_file.version, "1.0");
    assert_eq!(flow_file.name, "Direct Chat Flow");
    assert_eq!(flow_file.start_node, "llm");
    assert!(!flow_file.nodes.is_empty());
    assert!(!flow_file.transitions.is_empty());
}

#[test]
fn test_load_planning_flow_yaml() {
    let loader = YamlLoader::new();
    let flow_path = PathBuf::from("flows/planning.flow.yaml");

    let flow_file = loader.load_from_path(&flow_path).unwrap();
    assert_eq!(flow_file.version, "1.0");
    assert_eq!(flow_file.name, "Planning Flow");
    assert_eq!(flow_file.start_node, "planner");
    assert!(!flow_file.nodes.is_empty());
    assert!(!flow_file.transitions.is_empty());
}

#[test]
fn test_load_codegen_flow_yaml() {
    let loader = YamlLoader::new();
    let flow_path = PathBuf::from("flows/codegen.flow.yaml");

    let flow_file = loader.load_from_path(&flow_path).unwrap();
    assert_eq!(flow_file.version, "1.0");
    assert_eq!(flow_file.name, "Code Generation Flow");
    assert_eq!(flow_file.start_node, "planner");
    assert!(!flow_file.nodes.is_empty());
    assert!(!flow_file.transitions.is_empty());
}

#[test]
fn test_load_approval_flow_yaml() {
    let loader = YamlLoader::new();
    let flow_path = PathBuf::from("flows/approval.flow.yaml");

    let flow_file = loader.load_from_path(&flow_path).unwrap();
    assert_eq!(flow_file.version, "1.0");
    assert_eq!(flow_file.name, "Approval Flow");
    assert_eq!(flow_file.start_node, "reviewer");
    assert!(!flow_file.nodes.is_empty());
    assert!(!flow_file.transitions.is_empty());
}

#[test]
fn test_build_flow_from_yaml() {
    let loader = YamlLoader::new();
    let flow_path = PathBuf::from("flows/chat.flow.yaml");
    let flow_file = loader.load_from_path(&flow_path).unwrap();

    let factory = DefaultNodeFactory::new();
    let mut builder = Flow::builder();

    for node_def in &flow_file.nodes {
        let node = factory
            .create(&node_def.node_type, node_def.config.clone())
            .unwrap();
        builder = builder.add_node(node_def.id.clone(), node);
    }

    for trans in &flow_file.transitions {
        builder =
            builder.add_transition(trans.from.clone(), trans.action.clone(), trans.to.clone());
    }

    builder = builder.start(flow_file.start_node.clone());

    let flow = builder.build().unwrap();
    assert_eq!(flow.start_node(), "llm");
}

#[tokio::test]
async fn test_intent_classification_conversation() {
    let classifier = IntentClassifier::new().unwrap();
    let result = classifier.classify("Hello, how are you?").await.unwrap();
    assert_eq!(result.intent, Intent::Conversation);
}

#[tokio::test]
async fn test_intent_classification_question() {
    let classifier = IntentClassifier::new().unwrap();
    let result = classifier
        .classify("What is the capital of France?")
        .await
        .unwrap();
    assert_eq!(result.intent, Intent::Question);
}

#[tokio::test]
async fn test_intent_classification_coding() {
    let classifier = IntentClassifier::new().unwrap();
    let result = classifier
        .classify("Write a function to sort an array")
        .await
        .unwrap();
    assert_eq!(result.intent, Intent::CodingTask);
}

#[tokio::test]
async fn test_intent_classification_planning() {
    let classifier = IntentClassifier::new().unwrap();
    let result = classifier
        .classify("Plan a project to build a web app")
        .await
        .unwrap();
    assert_eq!(result.intent, Intent::Planning);
}

#[test]
fn test_intent_override_direct_chat() {
    let override_intent = Intent::from_override("/chat hello");
    assert_eq!(override_intent, Some(Intent::Conversation));
}

#[test]
fn test_intent_override_planning() {
    let override_intent = Intent::from_override("/plan create a REST API");
    assert_eq!(override_intent, Some(Intent::Planning));
}

#[test]
fn test_intent_override_codegen() {
    let override_intent = Intent::from_override("/code write a sorting function");
    assert_eq!(override_intent, Some(Intent::CodingTask));
}

#[test]
fn test_shared_state_with_personality_mode() {
    let mut state = SharedState::new();
    state.set_input("message".to_string(), serde_json::json!("test message"));
    state.set_personality_mode("companion");

    assert_eq!(
        state.get_input("message"),
        Some(&serde_json::json!("test message"))
    );
    assert_eq!(state.get_personality_mode(), Some("companion".to_string()));
}

#[test]
fn test_flow_selector_default_flow() {
    let selector = DefaultFlowSelector::with_default_dir();
    let default = selector.default_flow();
    assert!(default.ends_with("chat.flow.yaml"));
}

// ---------------------------------------------------------------------------
// E6/I01 Slice C (R5): workflow templates for bug fix / feature /
// refactor / test / docs / review workflows. Each new flow is a
// minimal valid FlowFile with a real start_node, real nodes, and
// real transitions. Acceptance: the templates cover all six
// workflow kinds (issue #130).
// ---------------------------------------------------------------------------

const E6I01_WORKFLOW_TEMPLATES: &[(&str, &str, &str)] = &[
    ("flows/bug_fix.flow.yaml", "Bug Fix Flow", "localize"),
    ("flows/feature.flow.yaml", "Feature Flow", "plan"),
    ("flows/refactor.flow.yaml", "Refactor Flow", "baseline"),
    ("flows/test.flow.yaml", "Test Flow", "read_target"),
    ("flows/docs.flow.yaml", "Documentation Flow", "inspect"),
    ("flows/review.flow.yaml", "Review Flow", "security_review"),
];

#[test]
fn e6i01_six_workflow_templates_load_and_have_valid_shape() {
    let loader = YamlLoader::new();
    for (path, expected_name, expected_start) in E6I01_WORKFLOW_TEMPLATES {
        let flow_path = PathBuf::from(path);
        let flow = loader
            .load_from_path(&flow_path)
            .unwrap_or_else(|e| panic!("failed to load {path}: {e}"));
        assert_eq!(flow.version, "1.0", "{path}: version must be 1.0");
        assert_eq!(flow.name, *expected_name, "{path}: name mismatch");
        assert_eq!(
            flow.start_node, *expected_start,
            "{path}: start_node mismatch"
        );
        assert!(
            !flow.nodes.is_empty(),
            "{path}: must declare at least one node"
        );
        assert!(
            !flow.transitions.is_empty(),
            "{path}: must declare at least one transition"
        );
        // Every transition's from/to must appear in the node set.
        for t in &flow.transitions {
            assert!(
                flow.nodes.iter().any(|n| n.id == t.from),
                "{path}: transition from={} has no node",
                t.from
            );
            assert!(
                flow.nodes.iter().any(|n| n.id == t.to),
                "{path}: transition to={} has no node",
                t.to
            );
        }
    }
}

#[test]
fn software_template_review_flow_reference_is_satisfied() {
    // The `templates/software.yaml` file lists `review.flow.yaml` in
    // its `lifecycle_template.phases[*].required_flows` list. This
    // test asserts the file exists, fixing a latent broken reference.
    let path = PathBuf::from("flows/review.flow.yaml");
    assert!(path.exists(), "flows/review.flow.yaml must exist (referenced by templates/software.yaml)");
    let loader = YamlLoader::new();
    let flow = loader.load_from_path(&path).unwrap();
    assert_eq!(flow.name, "Review Flow");
}
