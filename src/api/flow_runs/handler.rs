//! Flow run handler - entrypoint for run flow

use axum::{extract::{Path, State}, Json};
use std::sync::Arc;

use crate::api::AppState;
use crate::api::websocket::FlowEvent;
use crate::api::websocket::ConnectionManager;
use crate::config::AppConfig;
use crate::control::ControlFiles;
use crate::db::{Db, FlowRun, RunFlow};
use crate::db::repository::Repository;
use crate::flow::MemoryKind;
use crate::flow::SharedState;
use crate::flow::loader::FlowLoader;
use crate::flow::loader::YamlLoader;
use crate::flow::loader::JsonLoader;
use crate::flow::loader::FlowFile;
use crate::flow::factory::DefaultNodeFactory;
use crate::flow::NodeFactory;
use crate::flow::execution::Flow;
use crate::flow::execution::{RunDb, ContinuationEngine, FlowRun as FlowExecutionRun};
use crate::intent::{IntentClassifier, Intent, FlowSelector, DefaultFlowSelector};
use chrono::Utc;

/// Run a flow for a conversation
pub async fn run_flow(
    State(state): State<Arc<AppState>>,
    Path(conversation_id): Path<String>,
    Json(input): Json<RunFlow>,
) -> Result<Json<FlowRun>, axum::http::StatusCode> {
    let db_path = state.db_path.clone();
    let message = input.message.clone();
    let ws_manager = state.ws_manager.clone();
    let runtime = state.runtime.clone();

    // Save user message
    let db = Db::new(&db_path).map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    let _ = db.create_message(crate::db::CreateMessage {
        conversation_id: conversation_id.clone(),
        role: "user".to_string(),
        content: message.clone(),
    });

    // Create FlowRun
    let flow_run = db.create_flow_run(&conversation_id)
        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    let run_id = flow_run.id.clone();

    // Spawn async task for intent classification and routing
    tokio::spawn(async move {
        // Initialize ContinuationEngine (doesn't need to be Send)
        let checkpoint_dir = std::path::PathBuf::from(".prometheos/checkpoints");
        let continuation_engine = Arc::new(ContinuationEngine::new(checkpoint_dir.clone()));

        // Load control files
        let _control_files = match ControlFiles::load() {
            Ok(files) => files,
            Err(e) => {
                let _ = ws_manager.send_event(&run_id, FlowEvent::Error {
                    node: "system".to_string(),
                    message: format!("Failed to load control files: {}", e),
                    timestamp: Utc::now(),
                }).await;
                if let Ok(db) = Db::new(&db_path) {
                    let _ = db.update_flow_run_status(&run_id, "failed");
                }
                return;
            }
        };

        // Check for override commands
        let override_intent = Intent::from_override(&message);
        let actual_message = if override_intent.is_some() {
            message
                .split_whitespace()
                .skip_while(|word| word.starts_with('/'))
                .collect::<Vec<_>>()
                .join(" ")
                .trim()
                .to_string()
        } else {
            message.clone()
        };

        let message_to_process = if actual_message.is_empty() {
            message.clone()
        } else {
            actual_message
        };

        // Log user message as episodic memory (async, non-blocking)
        if let Some(memory_service) = runtime.memory_service.as_ref() {
            let _ = memory_service.queue_episode(
                format!("User: {}", message_to_process),
                None,
                None,
                Some(conversation_id.clone()),
                serde_json::json!({
                    "role": "user",
                    "flow_run_id": run_id,
                }),
            );
        }

        // Load relevant context from memory before LLM calls
        let relevant_context = if let Some(memory_service) = runtime.memory_service.as_ref() {
            match memory_service.semantic_search(&message_to_process, 5).await {
                Ok(memories) => {
                    let context: Vec<String> = memories.iter()
                        .filter(|m| m.kind != MemoryKind::Episodic)
                        .map(|m| m.content.clone())
                        .collect();
                    if !context.is_empty() {
                        Some(format!("Relevant Memory Context:\n{}", context.join("\n")))
                    } else {
                        None
                    }
                }
                Err(_) => None
            }
        } else {
            None
        };

        // Classify intent
        let classifier = match IntentClassifier::new() {
            Ok(c) => c,
            Err(e) => {
                let _ = ws_manager.send_event(&run_id, FlowEvent::Error {
                    node: "system".to_string(),
                    message: format!("Failed to create intent classifier: {}", e),
                    timestamp: Utc::now(),
                }).await;
                if let Ok(db) = Db::new(&db_path) {
                    let _ = db.update_flow_run_status(&run_id, "failed");
                }
                return;
            }
        };

        let classification = match classifier.classify_with_override(&message_to_process, override_intent).await {
            Ok(result) => result,
            Err(e) => {
                let _ = ws_manager.send_event(&run_id, FlowEvent::Error {
                    node: "system".to_string(),
                    message: format!("Intent classification failed: {}", e),
                    timestamp: Utc::now(),
                }).await;
                if let Ok(db) = Db::new(&db_path) {
                    let _ = db.update_flow_run_status(&run_id, "failed");
                }
                return;
            }
        };

        // Emit routing decision
        let _ = ws_manager.send_event(&run_id, FlowEvent::Output {
            node: "system".to_string(),
            data: format!("Intent: {} (confidence: {:.2})", classification.intent.display_name(), classification.confidence),
            timestamp: Utc::now(),
        }).await;

        // Use FlowSelector to select the appropriate flow
        let flow_selector = DefaultFlowSelector::with_default_dir();
        let flow_path = match flow_selector.select_flow(&classification.intent) {
            Ok(path) => path,
            Err(e) => {
                let _ = ws_manager.send_event(&run_id, FlowEvent::Error {
                    node: "system".to_string(),
                    message: format!("Flow selection failed: {}", e),
                    timestamp: Utc::now(),
                }).await;
                if let Ok(db) = Db::new(&db_path) {
                    let _ = db.update_flow_run_status(&run_id, "failed");
                }
                return;
            }
        };

        // Load the flow file
        let flow_file = if flow_path.extension().and_then(|s| s.to_str()) == Some("yaml") {
            let loader = YamlLoader::new();
            match loader.load_from_path(&flow_path) {
                Ok(file) => file,
                Err(e) => {
                    let _ = ws_manager.send_event(&run_id, FlowEvent::Error {
                        node: "system".to_string(),
                        message: format!("Failed to load YAML flow: {}", e),
                        timestamp: Utc::now(),
                    }).await;
                    if let Ok(db) = Db::new(&db_path) {
                        let _ = db.update_flow_run_status(&run_id, "failed");
                    }
                    return;
                }
            }
        } else {
            let loader = JsonLoader::new();
            match loader.load_from_path(&flow_path) {
                Ok(file) => file,
                Err(e) => {
                    let _ = ws_manager.send_event(&run_id, FlowEvent::Error {
                        node: "system".to_string(),
                        message: format!("Failed to load JSON flow: {}", e),
                        timestamp: Utc::now(),
                    }).await;
                    if let Ok(db) = Db::new(&db_path) {
                        let _ = db.update_flow_run_status(&run_id, "failed");
                    }
                    return;
                }
            }
        };

        // Create input state
        let mut input_state = SharedState::new();
        input_state.set_input("message".to_string(), serde_json::json!(message_to_process));

        // Execute the flow using the runtime
        let flow_result = {
            let flow_runtime = runtime.clone();
            let flow_path_clone = flow_path.clone();
            let run_id_clone = run_id.clone();
            let ws_manager_clone = ws_manager.clone();
            let db_path_clone = db_path.clone();
            let message_clone = message_to_process.clone();
            let continuation_engine_clone = Arc::clone(&continuation_engine);
            
            async move {
                // Initialize RunDb inside this block (not Send, but that's OK here)
                let run_db_path = std::path::PathBuf::from(".prometheos/runs.db");
                let run_db = match RunDb::new(run_db_path.clone()) {
                    Ok(db) => db,
                    Err(e) => {
                        let _ = ws_manager_clone.send_event(&run_id_clone, FlowEvent::Error {
                            node: "system".to_string(),
                            message: format!("Failed to initialize RunDb: {}", e),
                            timestamp: Utc::now(),
                        }).await;
                        if let Ok(db) = Db::new(&db_path_clone) {
                            let _ = db.update_flow_run_status(&run_id_clone, "failed");
                        }
                        return Err(e);
                    }
                };
                
                // Create FlowExecutionRun
                let flow_id = flow_path_clone.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();
                let mut flow_exec_run = FlowExecutionRun::new(flow_id);
                flow_exec_run.mark_running();
                let _ = run_db.save_run(&flow_exec_run);
                
                // Build the flow from the loaded flow file
                let factory = DefaultNodeFactory::new();
                let mut builder = Flow::builder();
                
                // Add nodes from flow file
                for node_def in &flow_file.nodes {
                    let node = factory.create(&node_def.node_type, node_def.config.clone())?;
                    builder = builder.add_node(node_def.id.clone(), node);
                }
                
                // Add transitions
                for trans in &flow_file.transitions {
                    builder = builder.add_transition(trans.from.clone(), trans.action.clone(), trans.to.clone());
                }
                
                // Set start node
                builder = builder.start(flow_file.start_node.clone());
                
                // Build the flow
                let mut flow = builder.build()?;
                
                // Create input state
                let mut state = SharedState::new();
                state.set_input("message".to_string(), serde_json::json!(message_clone));
                
                // Execute the flow
                let execution_result = flow.run(&mut state).await;
                
                match &execution_result {
                    Ok(_) => {
                        flow_exec_run.mark_completed(state.clone());
                        let _ = continuation_engine_clone.save_checkpoint(&flow_exec_run.id, &state);
                    }
                    Err(e) => {
                        flow_exec_run.mark_failed(e.to_string());
                    }
                }
                
                let _ = run_db.save_run(&flow_exec_run);
                
                match execution_result {
                    Ok(_) => {
                        // Emit the output
                        let output = state.get_all_outputs();
                        let _ = ws_manager_clone.send_event(&run_id_clone, FlowEvent::Output {
                            node: "system".to_string(),
                            data: serde_json::to_string_pretty(&output).unwrap_or_else(|_| "Failed to serialize output".to_string()),
                            timestamp: Utc::now(),
                        }).await;
                        
                        if let Ok(db) = Db::new(&db_path_clone) {
                            let _ = db.update_flow_run_status(&run_id_clone, "completed");
                        }
                        
                        Ok(())
                    }
                    Err(e) => {
                        let _ = ws_manager_clone.send_event(&run_id_clone, FlowEvent::Error {
                            node: "system".to_string(),
                            message: format!("Flow execution failed: {}", e),
                            timestamp: Utc::now(),
                        }).await;
                        if let Ok(db) = Db::new(&db_path_clone) {
                            let _ = db.update_flow_run_status(&run_id_clone, "failed");
                        }
                        Err(e)
                    }
                }
            }
        };

        if let Err(e) = flow_result.await {
            eprintln!("Flow execution failed: {}", e);
            let _ = ws_manager.send_event(&run_id, FlowEvent::Error {
                node: "system".to_string(),
                message: format!("Flow execution failed: {}", e),
                timestamp: Utc::now(),
            }).await;
            if let Ok(db) = Db::new(&db_path) {
                let _ = db.update_flow_run_status(&run_id, "failed");
            }
        }
    });

    Ok(Json(flow_run))
}
