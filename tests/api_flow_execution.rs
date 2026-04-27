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
