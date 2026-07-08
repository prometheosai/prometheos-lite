# PrometheOS Lite Examples

This directory contains example flow definitions that demonstrate the capabilities of PrometheOS Lite's flow runtime.

## Quick Start

Run the code generation flow:

```bash
prometheos flow examples/codegen.flow.json --input '{"task":"Build a Rust CLI todo app"}'
```

## Available Flows

### codegen.flow.json

A complete code generation pipeline that demonstrates the full flow execution path:

```
task → planner → coder → reviewer → file_writer → memory_write
```

**Nodes:**
- **planner**: Creates a structured plan for the given task
- **coder**: Generates code based on the plan
- **reviewer**: Reviews and refines the generated code
- **file_writer**: Writes the generated files to `prometheos-output/`
- **memory_write**: Logs the execution to memory (gracefully skips if embedding server is unavailable)

**Usage:**
```bash
prometheos flow examples/codegen.flow.json --input '{"task":"Build a Rust CLI todo app"}'
```

**Output:**
- Generated files written to `prometheos-output/`
- Memory logged (if embedding server is available)
- Flow execution summary printed to terminal

## Flow File Format

Flow files are JSON documents with the following structure:

```json
{
  "version": "1",
  "name": "Flow Name",
  "description": "Flow description",
  "start_node": "node_id",
  "nodes": [
    {
      "id": "node_id",
      "node_type": "node_type",
      "config": {
        "retries": 3,
        "retry_delay_ms": 1000
      }
    }
  ],
  "transitions": [
    {
      "from": "from_node",
      "action": "continue",
      "to": "to_node"
    }
  ]
}
```

### Available Node Types

- **planner**: Creates structured plans using LLM
- **coder**: Generates code using LLM
- **reviewer**: Reviews and refines code using LLM
- **llm**: Generic LLM node for custom prompts
- **tool**: Executes external tools and commands
- **file_writer**: Writes files to disk
- **context_loader**: Loads context from memory
- **memory_write**: Writes data to memory
- **conditional**: Conditional branching based on state
- **passthrough**: No-op node (default for unknown types)

### Node Configuration

Each node can have optional configuration:

- **retries**: Number of retry attempts (default: 3)
- **retry_delay_ms**: Delay between retries in milliseconds (default: 100)
- **timeout_ms**: Optional timeout for node execution

### Transitions

Transitions define the flow execution path:

- **from**: Source node ID
- **action**: Action string that triggers the transition (typically "continue")
- **to**: Target node ID

## Creating Custom Flows

1. Copy an existing flow file as a template
2. Modify the nodes and transitions to match your workflow
3. Test your flow with the `flow` command
4. Add documentation to this README

## Requirements

- **LLM Server**: An OpenAI-compatible LLM server (e.g., LM Studio) must be running
- **Config**: `prometheos.config.json` must be configured with your LLM endpoint
- **Embedding Server**: Optional - if unavailable, memory_write nodes will gracefully skip

## Troubleshooting

**Flow fails to load:**
- Verify JSON syntax is valid
- Check that all referenced node IDs exist
- Ensure transitions reference valid node IDs

**LLM nodes fail:**
- Verify LM Studio or compatible server is running
- Check `prometheos.config.json` for correct base_url and model
- Ensure network connectivity to LLM endpoint

**File writer fails:**
- Check write permissions in the project directory
- Ensure `prometheos-output/` directory can be created

**Memory write fails:**
- This is expected if embedding server is unavailable
- The flow will continue and log a warning
- To enable memory, configure `embedding_url` in `prometheos.config.json`
