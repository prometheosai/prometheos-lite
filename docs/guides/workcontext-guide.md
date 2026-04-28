# WorkContext Guide

## Overview

WorkContext is the persistent operational layer of PrometheOS Lite that manages real-world work across time. It provides a structured way to track work items, their lifecycle, artifacts, and execution context.

## Core Concepts

### WorkContext

A WorkContext represents a unit of work with:
- **Title**: Human-readable name for the work
- **Domain**: Category of work (Software, Business, Marketing, Personal, Creative, Research, Operations, General)
- **Goal**: The primary objective of the work
- **Status**: Current state (Draft, InProgress, AwaitingApproval, Completed, Blocked)
- **Phase**: Lifecycle stage (Intake, Planning, Execution, Review, Finalization)
- **Priority**: Urgency level (Low, Medium, High, Urgent)
- **Autonomy Level**: Execution mode (Autonomous, Review, Chat)
- **Approval Policy**: When approval is required (Auto, RequireForTools, RequireForSideEffects, RequireForUntrusted, ManualAll)
- **Artifacts**: Outputs produced during work execution
- **Completion Criteria**: Conditions that must be met for work completion

### Lifecycle Phases

1. **Intake**: Initial work definition and requirements gathering
2. **Planning**: Creating execution plans and resource allocation
3. **Execution**: Performing the actual work
4. **Review**: Evaluating outputs and quality
5. **Finalization**: Completing and archiving the work

### Autonomy Levels

- **Autonomous**: AI executes without human intervention
- **Review**: AI executes but requires human review before finalizing
- **Chat**: AI requires human confirmation for all actions

### Approval Policies

- **Auto**: No approval required
- **RequireForTools**: Approval required for tool usage
- **RequireForSideEffects**: Approval required for operations with side effects
- **RequireForUntrusted**: Approval required for untrusted operations
- **ManualAll**: All actions require manual approval

## Usage

### CLI Commands

#### Create a WorkContext

```bash
prometheos-lite work create --title "Build API" --domain software --goal "Create a REST API"
```

#### List WorkContexts

```bash
prometheos-lite work list
```

#### Show WorkContext Details

```bash
prometheos-lite work show <context-id>
```

#### Continue a WorkContext

```bash
prometheos-lite work continue <context-id>
```

#### Update WorkContext Status

```bash
prometheos-lite work set-status <context-id> --status in_progress
```

### API Endpoints

#### List WorkContexts

```http
GET /work-contexts
```

#### Get WorkContext

```http
GET /work-contexts/:id
```

#### Create WorkContext

```http
POST /work-contexts
Content-Type: application/json

{
  "title": "Build API",
  "domain": "software",
  "goal": "Create a REST API",
  "user_id": "user-123"
}
```

#### Update WorkContext Status

```http
POST /work-contexts/:id/status
Content-Type: application/json

{
  "status": "in_progress"
}
```

### Templates

WorkContext provides pre-configured templates for common work patterns:

#### Software Development Template

```rust
use prometheos_lite::work::software_development_template;

let context = software_development_template(
    "Build API".to_string(),
    "Create a REST API".to_string(),
);
```

#### Research Template

```rust
use prometheos_lite::work::research_template;

let context = research_template(
    "Research AI".to_string(),
    "Investigate AI techniques".to_string(),
);
```

#### Planning Template

```rust
use prometheos_lite::work::planning_template;

let context = planning_template(
    "Project Plan".to_string(),
    "Create project roadmap".to_string(),
);
```

#### Bug Fix Template

```rust
use prometheos_lite::work::bug_fix_template;

let context = bug_fix_template(
    "Fix bug".to_string(),
    "Fix critical issue".to_string(),
);
```

## Programmatic Usage

### Creating a WorkContext

```rust
use prometheos_lite::db::Db;
use prometheos_lite::work::{WorkContextService, types::{WorkDomain, WorkStatus, WorkPhase}};
use std::sync::Arc;

let db = Arc::new(Db::new("prometheos.db")?);
let work_context_service = WorkContextService::new(db);

let context = work_context_service.create_context(
    "user-123".to_string(),
    "Build API".to_string(),
    WorkDomain::Software,
    "Create a REST API".to_string(),
)?;
```

### Updating WorkContext

```rust
let mut context = work_context_service.get_context(&context_id)?.unwrap();

// Update phase
work_context_service.update_phase(&mut context, WorkPhase::Planning)?;

// Update status
work_context_service.update_status(&mut context, WorkStatus::InProgress)?;
```

### Adding Artifacts

```rust
use prometheos_lite::work::{Artifact, ArtifactKind, ArtifactStorage};

let artifact = Artifact::new(
    "artifact-1".to_string(),
    context.id.clone(),
    ArtifactKind::Code,
    "API implementation".to_string(),
    serde_json::json!({"content": "code here"}),
    "system".to_string(),
);

work_context_service.add_artifact(&mut context, artifact)?;
```

### Executing Flows in Context

```rust
use prometheos_lite::work::WorkExecutionService;
use prometheos_lite::flow::{RuntimeContext, execution_service::FlowExecutionService};

let runtime = Arc::new(RuntimeContext::default());
let flow_execution_service = Arc::new(FlowExecutionService::new(runtime)?);
let work_execution_service = WorkExecutionService::new(
    work_context_service.clone(),
    flow_execution_service,
);

let mut context = work_context_service.get_context(&context_id)?.unwrap();
let artifact = work_execution_service.execute_flow_in_context(&mut context, "planning").await?;
```

## Phase Controller

The PhaseController manages lifecycle transitions:

```rust
use prometheos_lite::work::PhaseController;

// Get next phase
let next_phase = PhaseController::next_phase(&context);

// Check if transition is valid
let can_transition = PhaseController::can_transition(
    context.current_phase,
    WorkPhase::Planning,
);

// Check if approval is required
let requires_approval = PhaseController::requires_approval(&context, next_phase.unwrap());
```

## Best Practices

1. **Use Templates**: Start with pre-configured templates for common work patterns
2. **Set Appropriate Autonomy**: Choose autonomy levels based on task complexity and trust requirements
3. **Define Completion Criteria**: Clearly specify what constitutes completion for each work item
4. **Track Artifacts**: Use artifacts to capture all outputs and intermediate results
5. **Update Status Regularly**: Keep status in sync with actual work progress
6. **Use Approval Policies**: Configure approval policies based on risk tolerance
7. **Phase Transitions**: Follow the natural lifecycle phases for structured work execution

## Database Schema

WorkContext uses SQLite for persistence with the following tables:

- `work_contexts`: Main work context records
- `artifacts`: Generated artifacts and outputs
- `work_context_events`: Event log for audit trail
- `conversation_work_contexts`: Link between conversations and work contexts

## Testing

Integration tests are available in `tests/work_context_integration_test.rs`:

```bash
cargo test --test work_context_integration_test
```

## Troubleshooting

### Schema Mismatch with In-Memory Databases

In-memory databases may have schema inconsistencies. For artifact persistence tests, use file-based databases:

```rust
let db = Db::new("file:test.db?mode=memory&cache=shared")?;
```

### Approval Required Errors

If you encounter "Approval required" errors, check:
- The context's approval policy
- The current phase and next phase
- The autonomy level setting

Adjust these settings or provide the required approval to proceed.
