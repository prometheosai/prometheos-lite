# Tracing and Replay

## Overview

PrometheOS Lite provides comprehensive tracing and replay capabilities for flow execution. All runs are tracked with detailed event logs and can be replayed observationally.

## Tracing

### Tracer

The `Tracer` captures execution events throughout the flow lifecycle.

```rust
use prometheos_lite::flow::Tracer;

let tracer = Tracer::new();
tracer.log(LogLevel::Info, "Starting flow execution");
tracer.add_timeline_event(TimelineEvent::NodeStarted { node_id: "node1".to_string() });
```

### Event Types

Events are categorized into two layers:

#### Run-Level Events
- `RunStarted`: Flow execution begins
- `RunCompleted`: Flow execution completes successfully
- `RunFailed`: Flow execution fails with error

#### Flow-Level Events
- `FlowLoaded`: Flow file loaded
- `FlowValidationFailed`: Flow validation error
- `NodeStarted`: Node execution begins
- `NodeCompleted`: Node execution completes
- `NodeFailed`: Node execution fails
- `TransitionTaken`: Transition between nodes
- `BudgetChecked`: Budget limit checked
- `ToolRequested`: Tool execution requested
- `ToolCompleted`: Tool execution completes
- `MemoryRead`: Memory read operation
- `MemoryWrite`: Memory write operation
- `EvaluationCompleted`: Evaluation completed
- `OutputGenerated`: Output generated

### Timeline Export

Export the full timeline as JSON:

```rust
let timeline_json = tracer.export_timeline()?;
```

## Persistence

### RunDb

`RunDb` provides SQLite-backed persistence for flow runs and events.

```rust
use prometheos_lite::flow::execution::RunDb;

let run_db = RunDb::new(".prometheos/runs.db")?;
run_db.save_run(&flow_run)?;
let runs = run_db.list_runs()?;
```

### FlowRun

`FlowRun` tracks metadata for each flow execution:

```rust
use prometheos_lite::flow::execution::FlowRun;

let mut flow_run = FlowRun::new("flow_id".to_string());
flow_run.mark_running();
flow_run.mark_completed(state.clone());
run_db.save_run(&flow_run)?;
```

## Checkpointing

### ContinuationEngine

`ContinuationEngine` manages state snapshots for replay and resume.

```rust
use prometheos_lite::flow::execution::ContinuationEngine;

let continuation_engine = ContinuationEngine::new(".prometheos/checkpoints");
continuation_engine.save_checkpoint("run_id", &state)?;
let loaded_state = continuation_engine.load_checkpoint("run_id")?;
```

### Checkpoint Lifecycle

Checkpoints are saved at key points:
- On flow completion
- On flow failure
- On pause (when implemented)
- On budget exceeded (when implemented)

## Replay

### Observational Replay

Replay a previous run without re-executing:

```bash
prometheos flow replay <run_id>
```

This displays:
- Run metadata (run_id, status, timestamps)
- Event timeline
- Node execution order
- Outputs
- Evaluation metrics

### CLI Replay Command

```rust
use prometheos_lite::cli::commands::flow::ReplayCommand;

let cmd = ReplayCommand {
    run_id: "run_123".to_string(),
    verbose: true,
};
cmd.execute().await?;
```

## Event Storage

Events are stored in SQLite with the following schema:

```sql
CREATE TABLE events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL,
    node_id TEXT,
    event_type TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    data TEXT,
    FOREIGN KEY (run_id) REFERENCES runs(id)
);
```

## Replay Use Cases

1. **Debugging**: Inspect failed runs to understand what went wrong
2. **Analysis**: Analyze execution patterns and performance
3. **Audit**: Review what actions were taken during execution
4. **Learning**: Understand how flows behave in different scenarios

## Best Practices

- Always enable tracing in production for observability
- Save checkpoints on completion for replay capability
- Use run IDs to track and reference specific executions
- Review event timelines to optimize flow performance
