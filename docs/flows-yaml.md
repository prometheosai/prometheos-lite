# Flow YAML Format

## Overview

Flows are defined in YAML files that describe the execution graph. Each flow has nodes, transitions, and a starting point.

## Flow File Structure

```yaml
version: "1.0"
name: "Flow Name"
description: "Flow description"

nodes:
  - id: "node_id"
    node_type: "node_type"
    config:
      # Node-specific configuration

transitions:
  - from: "node_id"
    action: "action_name"
    to: "target_node_id"

start_node: "start_node_id"
```

## Node Types

### LLM Node
```yaml
- id: "llm"
  node_type: "llm"
  config:
    model: "gpt-4"
    prompt: "Your prompt here"
    temperature: 0.7
```

### Planner Node
```yaml
- id: "planner"
  node_type: "planner"
  config:
    model: "gpt-4"
    max_steps: 5
```

### Coder Node
```yaml
- id: "coder"
  node_type: "coder"
  config:
    model: "gpt-4"
    max_iterations: 3
```

### Tool Node
```yaml
- id: "tool"
  node_type: "tool"
  config:
    tool_name: "file_read"
    tool_args:
      path: "file.txt"
```

### Memory Write Node
```yaml
- id: "memory_write"
  node_type: "memory_write"
  config:
    memory_type: "episodic"
    content_key: "result"
```

## Transitions

Transitions define how execution moves between nodes based on actions returned by nodes.

```yaml
transitions:
  - from: "planner"
    action: "complete"
    to: "coder"
  - from: "coder"
    action: "needs_revision"
    to: "reviewer"
  - from: "coder"
    action: "complete"
    to: "end"
```

## Example Flow

```yaml
version: "1.0"
name: "Code Generation"
description: "Generate code from a task description"

nodes:
  - id: "planner"
    node_type: "planner"
    config:
      model: "gpt-4"
      max_steps: 5

  - id: "coder"
    node_type: "coder"
    config:
      model: "gpt-4"
      max_iterations: 3

  - id: "reviewer"
    node_type: "reviewer"
    config:
      model: "gpt-4"

transitions:
  - from: "planner"
    action: "complete"
    to: "coder"
  - from: "coder"
    action: "needs_revision"
    to: "coder"
  - from: "coder"
    action: "complete"
    to: "reviewer"
  - from: "reviewer"
    action: "approved"
    to: "end"
  - from: "reviewer"
    action: "needs_revision"
    to: "coder"

start_node: "planner"
```

## Loading Flows

Flows can be loaded using the `FlowLoader` trait:

```rust
use prometheos_lite::flow::loader::{FlowLoader, YamlLoader};

let loader = YamlLoader::new();
let flow_file = loader.load_from_path("path/to/flow.yaml")?;
```
