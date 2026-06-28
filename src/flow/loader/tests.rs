//! Tests for flow file loaders

#[cfg(test)]
mod loader_tests {
    use crate::flow::{
        FlowFile, FlowInputs, FlowLoader, FlowOutputs, JsonLoader, NodeDefinition, YamlLoader,
        validate_flow_file,
    };
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_yaml_loader() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let yaml_path = temp_dir.path().join("test_flow.yaml");

        let yaml_content = r#"
version: "1.0"
name: "Test Flow"
description: "A test flow for YAML loading"
start_node: "node1"
inputs:
  required:
    - "task"
outputs:
  primary: "generated"
  include:
    - "review"
nodes:
  - id: "node1"
    node_type: "passthrough"
    config: null
transitions: []
"#;

        fs::write(&yaml_path, yaml_content).expect("Failed to write YAML file");

        let loader = YamlLoader::new();
        let flow_file = loader
            .load_from_path(&yaml_path)
            .expect("Failed to load YAML");

        assert_eq!(flow_file.version, "1.0");
        assert_eq!(flow_file.name, "Test Flow");
        assert_eq!(flow_file.start_node, "node1");
        assert_eq!(flow_file.nodes.len(), 1);
        assert_eq!(flow_file.nodes[0].id, "node1");
        assert_eq!(flow_file.nodes[0].node_type, "passthrough");
    }

    #[test]
    fn test_json_loader() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let json_path = temp_dir.path().join("test_flow.json");

        let json_content = r#"
{
  "version": "1.0",
  "name": "Test Flow",
  "description": "A test flow for JSON loading",
  "start_node": "node1",
  "inputs": {
    "required": ["task"]
  },
  "outputs": {
    "primary": "generated",
    "include": ["review"]
  },
  "nodes": [
    {
      "id": "node1",
      "node_type": "passthrough",
      "config": null
    }
  ],
  "transitions": []
}
"#;

        fs::write(&json_path, json_content).expect("Failed to write JSON file");

        let loader = JsonLoader::new();
        let flow_file = loader
            .load_from_path(&json_path)
            .expect("Failed to load JSON");

        assert_eq!(flow_file.version, "1.0");
        assert_eq!(flow_file.name, "Test Flow");
        assert_eq!(flow_file.start_node, "node1");
        assert_eq!(flow_file.nodes.len(), 1);
        assert_eq!(flow_file.nodes[0].id, "node1");
        assert_eq!(flow_file.nodes[0].node_type, "passthrough");
    }

    #[test]
    fn test_validate_flow_file() {
        let flow_file = FlowFile {
            version: "1.0".to_string(),
            name: "Test Flow".to_string(),
            description: Some("A test flow".to_string()),
            start_node: "node1".to_string(),
            inputs: Some(FlowInputs {
                required: vec!["task".to_string()],
            }),
            outputs: Some(FlowOutputs {
                primary: "generated".to_string(),
                include: vec!["review".to_string()],
            }),
            nodes: vec![NodeDefinition {
                id: "node1".to_string(),
                node_type: "passthrough".to_string(),
                config: None,
            }],
            transitions: vec![],
        };

        let result = validate_flow_file(&flow_file);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_flow_file_missing_version() {
        let flow_file = FlowFile {
            version: "".to_string(),
            name: "Test Flow".to_string(),
            description: Some("A test flow".to_string()),
            start_node: "node1".to_string(),
            inputs: None,
            outputs: None,
            nodes: vec![NodeDefinition {
                id: "node1".to_string(),
                node_type: "passthrough".to_string(),
                config: None,
            }],
            transitions: vec![],
        };

        let result = validate_flow_file(&flow_file);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("version"));
    }

    #[test]
    fn test_validate_flow_file_missing_start_node() {
        let flow_file = FlowFile {
            version: "1.0".to_string(),
            name: "Test Flow".to_string(),
            description: Some("A test flow".to_string()),
            start_node: "nonexistent".to_string(),
            inputs: None,
            outputs: None,
            nodes: vec![NodeDefinition {
                id: "node1".to_string(),
                node_type: "passthrough".to_string(),
                config: None,
            }],
            transitions: vec![],
        };

        let result = validate_flow_file(&flow_file);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Start node"));
    }
}
