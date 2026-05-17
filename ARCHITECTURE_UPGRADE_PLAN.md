Recommended PrometheOS Lite architecture upgrade
User / Frontend / CLI        ↓API Layersrc/api        ↓Intent Routersrc/intent        ↓WorkRuntimeSessionsrc/work/runtime_session.rs        ↓WorkOrchestratorsrc/harness/execution_loop.rs        ↓PlaybookResolversrc/harness/work_integration.rs / src/work/playbooks        ↓Flow Enginesrc/flow        ↓Tool Registry + Model Router + Memorysrc/tools      src/llm        src/db / src/context        ↓Artifacts + Timeline + Retrieval Tracesrc/harness/artifactssrc/harness/observabilitysrc/harness/evidence

Concrete implementation plan
Phase 1: Provider registry
Add:

src/llm/provider.rssrc/llm/registry.rssrc/llm/router.rssrc/llm/providers/lmstudio.rssrc/llm/providers/openai_compatible.rssrc/llm/providers/ollama.rs

Goal:

All model calls go through ModelRouter.No direct provider-specific calls outside src/llm/providers.


Phase 2: Tool registry
Add:

src/tools/definition.rssrc/tools/registry.rssrc/tools/context.rssrc/tools/builtins/

Goal:

All tools expose name, schema, permissions, risk, and async execute().Harness and flow nodes call tools through ToolRegistry only.


Phase 3: WorkRuntimeSession
Add:

src/work/runtime_session.rssrc/work/events.rssrc/work/timeline.rs

Goal:

One shared runtime for API, CLI, WebSocket, and future desktop/editor integrations.


Phase 4: Event streaming protocol
Add:

src/api/events.rssrc/api/websocket/protocol.rs

Frontend event types:

work.startedintent.classifiedplaybook.selectedflow.node.startedmodel.stream.deltatool.startedtool.completedapproval.requiredartifact.createdmemory.retrievedmemory.writtenwork.completedwork.failed


Phase 5: Artifact store
Add DB tables and Rust types for:

artifactsartifact_linksartifact_versionsartifact_reviews

Goal:

Every important output becomes inspectable, replayable, and reusable.

Final recommendation
Use pi as a pattern library, not a dependency.

For PrometheOS Lite, the best borrow list is:



Provider registry


Unified session runtime


Tool definition registry


Append-only event timeline


Streaming lifecycle events


Artifact-first UI model


Extension/plugin hooks later


The core identity should stay yours:

PrometheOS Lite = Rust + WorkContext + Flow Engine + Harness + Memory + Bounded Local Agentic Execution