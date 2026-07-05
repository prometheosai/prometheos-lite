# Harness Spine Architecture

## V1.2.5 Status

### Completed
- WorkExecutionService no longer forces Intent::CodingTask
- ContextLoaderNode fixed (prep emits "query", exec reads "query")
- CLI work artifacts command exists
- TemplateLoader::install_defaults() called in WorkCommand::execute()
- API routes registered for work-contexts, artifacts, continue, submit-intent, run-until-complete
- PhaseController checks approved_plan.is_none() for Planning → Execution
- PhaseController supports domain-profile flow selection
- String-based phase detection replaced with PhaseController::flow_for_phase
- Deterministic no-API flow test added (deterministic_test.flow.yaml)
- TODO removed from WorkExecutionService - domain profile now loaded and applied
- Submit semantics defined: Chat = create + AwaitingApproval, Review/Autonomous = execute immediately
- MemoryWriteNode task/content mismatch fixed in builtin_nodes.rs (prep emits "task", exec reads "task")
- GenerateResult struct added with provider/model/latency/fallback metadata
- ModelRouter::generate() now calls generate_with_metadata() internally
- ModelRouter::generate_with_metadata() and generate_stream_with_metadata() added
- LlmUtilities::call_with_metadata() and call_stream_with_metadata() added
- Autonomy semantics fixed: submit_user_intent sets autonomy level based on intent type
- Flow input contract normalized: execution_service.rs uses "task" instead of "message"
- Real integration tests added: tests/work_orchestrator_e2e.rs with full lifecycle tests
- Execution metadata tracking added: ExecutionRecord struct and execution_metadata field in WorkContext
- Database schema updated to include execution_metadata field
- From<WorkContext> for WorkContextResponse added to reduce boilerplate
- ApiError enum with IntoResponse implementation added
- Partial error handling consistency: list_work_contexts, get_work_context, create_work_context, update_work_context_status, get_work_context_artifacts now use ApiError

### Partially Implemented
- Model metadata: GenerateResult exists at router level, but metadata is not propagated to WorkContext execution_metadata. Existing nodes still call router.generate() which discards metadata.
- Error handling consistency: Handlers that need WorkOrchestrator (submit_intent, continue_work_context, run_until_complete) still use StatusCode due to Axum Handler trait compatibility issues.

### Not Implemented / Blocked
- API integration with WorkOrchestrator: submit_intent, continue_work_context, and run_until_complete still use WorkContextService directly. Attempted to wire to WorkOrchestrator using ApiError, but these handlers fail Axum's Handler trait bound. This is a real type compatibility issue requiring investigation into handler return type constraints.
- run-until-complete loop implementation: blocked by API integration issue above
- Deterministic tests proving API → WorkOrchestrator → WorkExecutionService: blocked by API integration issue
- Structured repo-aware coding tools (list_tree, read_file, search_files, patch_file, git_diff, run_tests): explicitly deferred to V1.4

### Production Readiness
- V1.2.5 direction: correct
- Implementation: improved but incomplete
- CLI path: 8/10
- Core orchestrator: 8/10
- API path: 2/10 (bypasses WorkOrchestrator - Handler trait compatibility issue, not framework limitation)
- Error handling consistency: 4/10 (split between ApiError and StatusCode)
- Playbook integration: 4/10
- Execution completeness standard: 6/10
- Model metadata: 5.5/10 (router-level only, not propagated to WorkContext)
- Overall production readiness: ~5.8/10
- Claim of "fully complete": no

## Overview

The Harness Spine is the central execution layer for persistent work in PrometheOS Lite. It provides the foundational infrastructure for autonomous, persistent work execution through three core components:

- **WorkOrchestrator**: Central execution loop managing persistent work contexts with hard stop contracts
- **PlaybookResolver**: Scores and selects playbooks based on domain, usage, and confidence
- **WorkContext**: Default execution path for all work operations

## Components

### WorkOrchestrator

The `WorkOrchestrator` is the central service that owns the high-level execution loop for persistent work contexts. It provides:

- **Intent Submission**: `submit_user_intent()` - Classifies user intent and routes to appropriate context
- **Context Continuation**: `continue_context()` - Resumes blocked contexts
- **Bounded Execution**: `run_until_blocked_or_complete()` - Executes with hard stop contracts
- **Context Routing**: `route_to_context()` - Routes requests to appropriate work contexts

#### Execution Limits (Hard Stop Contracts)

The `ExecutionLimits` struct defines hard stop contracts for autonomous execution:

```rust
pub struct ExecutionLimits {
    pub max_iterations: u32,           // Default: 10
    pub max_runtime_ms: u64,            // Default: 300,000 (5 minutes)
    pub max_tool_calls: u32,            // Default: 50
    pub max_cost: f64,                  // Default: $1.00
    pub approval_required_for_side_effects: bool,  // Default: true
    pub completion_criteria: Vec<String>,
    pub failure_threshold: f32,         // Default: 0.3
}
```

### PlaybookResolver

The `PlaybookResolver` selects appropriate playbooks for a given `WorkContext` based on:

- **Domain Matching**: Bonus for matching domain profiles
- **Usage History**: Diminishing returns boost based on usage count
- **Confidence Score**: Base confidence from playbook metadata

#### Scoring Algorithm

```
score = base_confidence + domain_bonus + usage_boost

where:
- base_confidence: 0.0 to 1.0
- domain_bonus: +0.3 if domain matches
- usage_boost: 0.1 * ln(usage_count + 1)
```

### WorkContext Execution Path

The `WorkContext` is now the default execution path for all work operations. Key changes:

- **Intent Routing**: `CODING_TASK` and `APPROVAL` intents route to `WorkOrchestrator`
- **Execution Options**: `FlowExecutionService` accepts optional `work_context_id`
- **State Tracking**: Context state is updated on each execution step

## Data Flow

### Intent Submission Flow

```
User Message
    ↓
IntentClassifier
    ↓
WorkOrchestrator::submit_user_intent()
    ↓
Route to Context (create or attach)
    ↓
PlaybookResolver::resolve_playbook()
    ↓
Apply playbook settings
    ↓
FlowExecutionService::execute_message()
    ↓
Update WorkContext state
```

### Bounded Execution Flow

```
WorkContext ID + ExecutionLimits
    ↓
WorkOrchestrator::run_until_blocked_or_complete()
    ↓
Loop:
  - Check limits (iterations, runtime, tool calls, cost)
  - Check completion criteria
  - Check blocked status
  - Execute next step
  - Update context state
    ↓
Return final context state
```

## Database Schema

### New Table: playbook_usage_log

Tracks playbook usage for analytics and confidence adjustment:

```sql
CREATE TABLE playbook_usage_log (
    id TEXT PRIMARY KEY,
    playbook_id TEXT NOT NULL,
    work_context_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    domain TEXT NOT NULL,
    outcome TEXT NOT NULL,
    used_at TEXT NOT NULL,
    FOREIGN KEY (playbook_id) REFERENCES work_context_playbooks(id) ON DELETE CASCADE,
    FOREIGN KEY (work_context_id) REFERENCES work_contexts(id) ON DELETE CASCADE
);
```

### Enhanced Playbook Operations

- `increment_usage_count()`: Increments playbook usage counter
- `update_confidence()`: Updates playbook confidence score

## CLI Integration

### New Commands

```bash
# Submit a user intent
prometheos work submit "Build a REST API" --conversation-id <id>

# Continue a blocked context
prometheos work continue <context-id>

# Run context until blocked or complete
prometheos work run <context-id> --max-iterations 20 --max-runtime-ms 600000
```

## API Integration

### New Endpoints

- `POST /work-contexts/submit-intent` - Submit user intent
- `POST /work-contexts/:id/continue` - Continue blocked context
- `POST /work-contexts/:id/run-until-complete` - Run with limits
- `GET /playbooks` - List playbooks
- `POST /playbooks` - Create playbook
- `PUT /playbooks/:id` - Update playbook

## Critical Bug Fix

### ContextLoaderNode Input Mismatch

**Bug**: `ContextLoaderNode::prep()` emitted `{"task": task}` but `exec()` expected `input["query"]`

**Fix**: Changed `prep()` to emit `{"query": task}` to match `exec()` expectations

**Location**: `src/flow/factory/builtin_nodes.rs`

## Testing

### Unit Tests

- `test_execution_limits_default()` - Validates default limits
- `test_execution_limits_builder()` - Validates builder pattern
- `test_playbook_resolver_resolve_playbook()` - Tests playbook resolution
- `test_work_orchestrator_route_to_context()` - Tests context routing
- `test_work_context_service_create_and_get()` - Tests context CRUD
- `test_work_context_service_update_status()` - Tests status updates
- `test_playbook_repository_increment_usage()` - Tests usage tracking
- `test_playbook_repository_update_confidence()` - Tests confidence updates

### Test Coverage

All new code paths have unit tests with >80% coverage target.

## Future Enhancements (Not in MVP)

- SkillKernel for skill extraction and management
- EvolutionEngine for playbook evolution
- Coding harness nodes for specialized operations
- Job queue for async execution
- Metadata tracking on all LLM/tool outputs
- Control panel endpoints
- WebSocket events for real-time updates
