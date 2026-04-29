PrometheOS Lite v1.2.1 - Intent Classification Layer PRD
Overview
Implement a hybrid intent classification system that routes user messages to either direct LLM responses or the full code generation flow based on message intent, along with control markdown files for system behavior.

Issue #1: Create Intent Types Module
Type: Feature
Priority: High
Status: Pending

Description
Create the intent type definitions and related structures in src/intent/types.rs.

Acceptance Criteria
Define Intent enum with variants: Conversation, Question, CodingTask, FileEdit, ToolAction, ProjectAction, Ambiguous
Define IntentClassificationResult struct with intent, confidence score, and reasoning
Define routing decision types for handler selection
Add appropriate serde serialization support for JSON responses
Implementation Notes
Use #[derive(Debug, Clone, Serialize, Deserialize)] for types
Confidence score should be float between 0.0 and 1.0
Reasoning field should explain why classification was made
Issue #2: Implement Rule-Based Classifier
Type: Feature
Priority: High
Status: Pending

Description
Implement fast rule-based classification in src/intent/rules.rs for obvious cases.

Acceptance Criteria
Conversation patterns: "hi", "hello", "how are you", "thanks", "ok", "what can you do"
Coding task patterns: "build", "create", "implement", "generate", "make an app", "write code", "fix bug", "refactor", "add feature"
Question patterns: "what is", "how do", "explain", "why"
Character count heuristic: < 80 chars + no action verbs → likely conversation
Action verbs + software nouns → coding task
Question words → question
Return Some(Intent) for matches, None for ambiguous
Implementation Notes
Use case-insensitive matching
Pattern matching should be extensible
Consider regex for more complex patterns
Character count is a hint, not a hard rule
Issue #3: Implement Hybrid Classifier
Type: Feature
Priority: High
Status: Pending

Description
Implement hybrid classifier in src/intent/classifier.rs that combines rule-based and LLM classification.

Acceptance Criteria
First pass: rule-based classification via rules module
If rule-based returns None or low confidence (< 0.7), trigger LLM fallback
LLM classifier prompt:
Classify the user message into one intent:
CONVERSATION, QUESTION, CODING_TASK, FILE_EDIT, TOOL_ACTION, PROJECT_ACTION, AMBIGUOUS.
Return only JSON: {"intent":"...", "confidence":0.0, "reason":"..."}
Parse LLM JSON response
Return IntentClassificationResult with intent, confidence, and reasoning
Handle LLM errors gracefully (fallback to Ambiguous)
Implementation Notes
Use existing LlmClient for classification call
Add timeout for LLM classification (should be fast)
Cache common classifications if performance issues arise
Issue #4: Implement Intent Router
Type: Feature
Priority: High
Status: Pending

Description
Implement router in src/intent/router.rs that routes intents to appropriate handlers.

Acceptance Criteria
Route CONVERSATION → direct LLM with concise prompt
Route QUESTION → direct LLM with context
Route CODING_TASK → full code generation flow
Route FILE_EDIT → file editing handler (placeholder for now)
Route TOOL_ACTION → tool execution handler (placeholder for now)
Route PROJECT_ACTION → project operations handler (placeholder for now)
Route AMBIGUOUS → direct LLM with clarification request
Return routing decision with handler type
Implementation Notes
Define Handler enum: DirectLlm, CodeGenFlow, FileEdit, ToolExecution, ProjectAction
Router should be stateless
Handlers will be implemented in API layer
Issue #5: Create Control Markdown Files
Type: Feature
Priority: High
Status: Pending

Description
Create control markdown files at project root for system behavior configuration.

Acceptance Criteria
SOUL.md - Assistant identity, tone, boundaries, local-first philosophy
SKILLS.md - Available abilities mapped to intents
FLOWS.md - Available flows with intent mappings
TOOLS.md - Executable tools and permissions
MEMORY.md - Memory policy (what to remember/not remember)
PROJECT.md - Current project context
File Templates
SOUL.md:

markdown
# PrometheOS Lite - System Identity
 
## Tone
Concise, helpful, local-first AI assistant.
 
## Boundaries
- Do not generate code unless explicitly asked
- Keep simple replies under 120 words
- Respect user privacy
- Local-first philosophy
 
## Default Behavior
- Answer naturally and briefly
- Use flows for coding tasks
- Direct response for conversation
SKILLS.md:

markdown
# PrometheOS Lite - Available Skills
 
## Conversation
- Intent: CONVERSATION
- Handler: Direct LLM
- Description: Casual chat, greetings
 
## Question
- Intent: QUESTION
- Handler: Direct LLM
- Description: Information queries
 
## Code Generation
- Intent: CODING_TASK
- Handler: CodeGen Flow
- Description: Generate code, implement features
 
## File Editing
- Intent: FILE_EDIT
- Handler: File Edit
- Description: Modify existing files
 
## Tool Execution
- Intent: TOOL_ACTION
- Handler: Tool Execution
- Description: Run tools and commands
 
## Project Actions
- Intent: PROJECT_ACTION
- Handler: Project Operations
- Description: Project-level operations
FLOWS.md:

markdown
# PrometheOS Lite - Available Flows
 
## Code Generation Flow
- File: flows/code-generation.json
- Intent: CODING_TASK
- Input: task description
- Output: generated files
- Nodes: planner → coder → reviewer → file_writer → memory_write
TOOLS.md:

markdown
# PrometheOS Lite - Available Tools
 
## cargo_check
- Command: cargo check
- Permissions: read_project, execute_cargo
- Description: Check Rust project for errors
 
## cargo_build
- Command: cargo build
- Permissions: read_project, execute_cargo
- Description: Build Rust project
MEMORY.md:

markdown
# PrometheOS Lite - Memory Policy
 
## Remember
- Project goals
- User preferences
- Generated architecture decisions
- Important context
 
## Do Not Remember
- Secrets
- API keys
- Private credentials
- Temporary test data
PROJECT.md:

markdown
# PrometheOS Lite - Project Context
 
## Project Name
PrometheOS Lite
 
## Stack
- Backend: Rust (Axum)
- Frontend: Next.js
- LLM: LM Studio (local)
 
## Goals
- Local-first AI assistant
- Flow-based code generation
- Intent-aware routing
 
## Current Phase
v1.2.1 - Intent Classification Layer
 
## Important Paths
- Backend: src/
- Frontend: frontend/
- Flows: flows/
- Config: prometheos.config.json
Issue #6: Integrate Intent Classifier into API
Type: Feature
Priority: High
Status: Pending

Description
Modify e:\Projects\PrometheOS-Lite/src/api/server.rs run_flow endpoint to classify intent before routing.

Acceptance Criteria
Load intent classifier at startup
Load control markdown files (SOUL.md, SKILLS.md) for system prompt
Classify intent before spawning flow execution
Route to direct LLM for CONVERSATION/QUESTION intents
Route to full code generation flow for CODING_TASK intent
Use concise conversation prompt for non-coding intents:
You are PrometheOS Lite, a concise local AI assistant.
Answer naturally and briefly.
Do not generate code unless explicitly asked.
Keep simple replies under 120 words.
Emit appropriate WebSocket events for routing decision
Update flow run status based on handler type
Implementation Notes
Add intent field to FlowRun model
Create separate handler function for direct LLM responses
Keep existing flow execution for CODING_TASK
Update AppState to include intent classifier
Issue #7: Update Module Exports
Type: Feature
Priority: High
Status: Pending

Description
Add intent module to e:\Projects\PrometheOS-Lite/src/lib.rs exports.

Acceptance Criteria
Add pub mod intent; to src/lib.rs
Re-export key types: Intent, IntentClassificationResult
Ensure module compiles without errors
Issue #8: Test Intent Detection
Type: Testing
Priority: Medium
Status: Pending

Description
Test intent classification with various message types.

Acceptance Criteria
Test obvious conversation: "hi", "hello", "how are you"
Test obvious coding tasks: "create a REST API", "fix this bug"
Test questions: "what is Rust?", "how do I use axum?"
Test ambiguous cases that trigger LLM fallback
Verify routing to correct handler for each intent type
Verify concise responses for conversational intents
Verify full flow execution for coding tasks
Add unit tests for rule-based classifier
Add integration tests for LLM fallback
Test Cases
rust
#[test]
fn test_conversation_classification() {
    assert_eq!(classify("hi"), Some(Intent::Conversation));
    assert_eq!(classify("hello"), Some(Intent::Conversation));
}
 
#[test]
fn test_coding_task_classification() {
    assert_eq!(classify("create a REST API"), Some(Intent::CodingTask));
    assert_eq!(classify("fix this bug"), Some(Intent::CodingTask));
}
 
#[test]
fn test_question_classification() {
    assert_eq!(classify("what is Rust?"), Some(Intent::Question));
    assert_eq!(classify("how do I use axum?"), Some(Intent::Question));
}
Dependencies
Required
Existing LlmClient for LLM fallback
Existing serde for JSON serialization
Existing tokio for async operations
New Dependencies
None (uses existing dependencies)
Timeline
Sprint 1: Issues #1-#4 (Intent module implementation)
Sprint 2: Issues #5-#6 (Control files and API integration)
Sprint 3: Issues #7-#8 (Exports and testing)
Success Metrics
Conversational messages get direct LLM responses (< 1s)
Coding tasks trigger full flow execution
Intent classification accuracy > 90%
No regression in existing flow execution
User feedback on response quality
Open Questions
Should we cache LLM classification results for common messages?
What confidence threshold should trigger LLM fallback? (proposed: 0.7)
Should we add telemetry for intent classification accuracy?

2. Add execution tracing for intent

Log:

intent_detected
confidence
routing_decision

This will save you later when debugging weird behavior.

3. Add “intent override”

Let user force:

/run flow

or:

/ask

Humans hate when AI guesses wrong.

4. Cache common intents
hi → conversation
thanks → conversation

Avoid unnecessary LLM calls.