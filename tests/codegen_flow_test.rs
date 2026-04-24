use std::path::PathBuf;
use std::fs;

#[test]
fn test_codegen_flow_exists() {
    let flow_path = PathBuf::from("examples/codegen.flow.json");
    assert!(flow_path.exists(), "codegen.flow.json should exist in examples/");
}

#[test]
fn test_codegen_flow_valid_json() {
    let flow_path = PathBuf::from("examples/codegen.flow.json");
    let content = fs::read_to_string(&flow_path).expect("Should be able to read flow file");
    
    // Verify it's valid JSON
    let json: serde_json::Value = serde_json::from_str(&content)
        .expect("Flow file should be valid JSON");
    
    // Verify required fields exist
    assert!(json.get("version").is_some(), "Flow should have version field");
    assert!(json.get("name").is_some(), "Flow should have name field");
    assert!(json.get("nodes").is_some(), "Flow should have nodes field");
    assert!(json.get("transitions").is_some(), "Flow should have transitions field");
    assert!(json.get("start_node").is_some(), "Flow should have start_node field");
}

#[test]
fn test_codegen_flow_structure() {
    let flow_path = PathBuf::from("examples/codegen.flow.json");
    let content = fs::read_to_string(&flow_path).expect("Should be able to read flow file");
    let json: serde_json::Value = serde_json::from_str(&content).expect("Flow file should be valid JSON");
    
    // Verify the flow has the expected nodes
    let nodes = json["nodes"].as_array().expect("nodes should be an array");
    let node_ids: Vec<&str> = nodes
        .iter()
        .filter_map(|n| n["id"].as_str())
        .collect();
    
    assert!(node_ids.contains(&"planner"), "Flow should have a planner node");
    assert!(node_ids.contains(&"coder"), "Flow should have a coder node");
    assert!(node_ids.contains(&"reviewer"), "Flow should have a reviewer node");
    assert!(node_ids.contains(&"file_writer"), "Flow should have a file_writer node");
    assert!(node_ids.contains(&"memory_write"), "Flow should have a memory_write node");
}

#[test]
fn test_codegen_flow_transitions() {
    let flow_path = PathBuf::from("examples/codegen.flow.json");
    let content = fs::read_to_string(&flow_path).expect("Should be able to read flow file");
    let json: serde_json::Value = serde_json::from_str(&content).expect("Flow file should be valid JSON");
    
    // Verify the flow starts at planner
    let start_node = json["start_node"].as_str().expect("start_node should be a string");
    assert_eq!(start_node, "planner", "Flow should start at planner");

    // Verify transitions exist
    let transitions = json["transitions"].as_array().expect("transitions should be an array");
    
    // Create a map of (from, action) -> to
    let mut transition_map: std::collections::HashMap<(String, String), String> = std::collections::HashMap::new();
    for t in transitions {
        if let (Some(from), Some(action), Some(to)) = (
            t["from"].as_str(),
            t["action"].as_str(),
            t["to"].as_str()
        ) {
            transition_map.insert((from.to_string(), action.to_string()), to.to_string());
        }
    }
    
    // Verify expected transitions
    assert_eq!(
        transition_map.get(&("planner".to_string(), "continue".to_string())),
        Some(&"coder".to_string()),
        "planner should transition to coder on continue"
    );
    assert_eq!(
        transition_map.get(&("coder".to_string(), "continue".to_string())),
        Some(&"reviewer".to_string()),
        "coder should transition to reviewer on continue"
    );
    assert_eq!(
        transition_map.get(&("reviewer".to_string(), "continue".to_string())),
        Some(&"file_writer".to_string()),
        "reviewer should transition to file_writer on continue"
    );
    assert_eq!(
        transition_map.get(&("file_writer".to_string(), "continue".to_string())),
        Some(&"memory_write".to_string()),
        "file_writer should transition to memory_write on continue"
    );
}
