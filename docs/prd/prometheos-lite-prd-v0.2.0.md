So now we turn your engine into something a normal human can actually use without opening a terminal and questioning their life choices. Good.

Here’s the **GitHub Issues–style PRD for v1.2**.

---

# 🧠 PrometheOS Lite v1.2 — Local Chat Interface

**Codename: “Human Interface Layer”**

---

## ## 🎯 Objective

Expose PrometheOS Lite Flow runtime through a **ChatGPT-style local interface** with:

* projects
* conversations
* real-time execution
* generated artifacts

---

## ## 🧩 System Scope

### INCLUDED

* Rust HTTP API layer
* WebSocket streaming
* SQLite persistence for UI state
* Next.js frontend (chat interface)
* Flow execution integration

### EXCLUDED (for now)

* authentication
* multi-user
* cloud sync
* deployment tooling

---

# 🔥 PHASE 1 — Backend API Foundation

---

## ISSUE 1 — Create API Server Module

**Path:**

```
src/api/server.rs
```

**Requirements:**

* Use `axum` or `warp`
* Async server using tokio
* JSON responses
* Global app state

---

## ISSUE 2 — Global AppState

```rust
struct AppState {
    db: Db,
    runtime: RuntimeContext,
}
```

* Shared across all routes
* Thread-safe (Arc)

---

## ISSUE 3 — Health Endpoint

```
GET /health
```

Response:

```json
{ "status": "ok" }
```

---

# 🔥 PHASE 2 — SQLite UI Database

---

## ISSUE 4 — DB Module

**Path:**

```
src/db/mod.rs
```

Use:

* rusqlite or sqlx

---

## ISSUE 5 — Schema Creation

Tables:

```sql
projects
conversations
messages
flow_runs
artifacts
```

---

## ISSUE 6 — Project Repository

Endpoints:

```
GET /projects
POST /projects
```

---

## ISSUE 7 — Conversation Repository

```
GET /projects/:id/conversations
POST /projects/:id/conversations
```

---

## ISSUE 8 — Message Repository

```
GET /conversations/:id/messages
POST /conversations/:id/messages
```

---

# 🔥 PHASE 3 — Flow Execution API

---

## ISSUE 9 — Run Flow Endpoint

```
POST /conversations/:id/run
```

**Input:**

```json
{ "message": "Build a REST API" }
```

**Behavior:**

* save user message
* create FlowRun
* spawn async task
* execute flow

---

## ISSUE 10 — FlowRun Tracking

Store:

```text
id
conversation_id
status
started_at
completed_at
```

---

## ISSUE 11 — Artifact Storage

Save:

```text
file_path
content
run_id
```

---

# 🔥 PHASE 4 — WebSocket Streaming

---

## ISSUE 12 — WebSocket Server

```
WS /ws/runs/:id
```

---

## ISSUE 13 — Event Model

```json
{
  "type": "node_start | node_end | output | error",
  "node": "planner",
  "data": {},
  "timestamp": ""
}
```

---

## ISSUE 14 — Hook Flow → WS

Inside Flow execution:

* emit events on:

  * node start
  * node end
  * output update
  * errors

---

# 🔥 PHASE 5 — Frontend (Next.js)

---

## ISSUE 15 — Next.js App Setup

Structure:

```text
app/
components/
lib/
```

---

## ISSUE 16 — Projects Page

* list projects
* create project

---

## ISSUE 17 — Conversations Sidebar

* list conversations per project
* create new conversation

---

## ISSUE 18 — Chat UI

* message list
* input box
* submit handler

---

## ISSUE 19 — WebSocket Integration

* connect to `/ws/runs/:id`
* stream updates live

---

## ISSUE 20 — Run Timeline UI

Display:

```text
Planning...
Coding...
Reviewing...
Writing files...
Done
```

---

## ISSUE 21 — Artifacts Panel

* list files
* show content
* copy button

---

# 🔥 PHASE 6 — Flow Integration

---

## ISSUE 22 — Connect Chat → Flow

Flow:

```text
message → SharedState.input
→ execute codegen.flow.json
```

---

## ISSUE 23 — Default Flow Binding

Hardcode for MVP:

```text
examples/codegen.flow.json
```

---

## ISSUE 24 — Output Parsing

* capture outputs
* display in chat

---

# 🔥 PHASE 7 — UX Polish

---

## ISSUE 25 — Loading States

* spinner during execution
* disable input while running

---

## ISSUE 26 — Error Handling UI

* show node errors
* show memory skipped warnings

---

## ISSUE 27 — Conversation Persistence

* reload messages on refresh

---

## ISSUE 28 — Basic Styling

* minimal clean UI
* dark mode optional

---

# 🧪 PHASE 8 — Testing

---

## ISSUE 29 — API Tests

* endpoints return correct data

---

## ISSUE 30 — Flow Execution Tests via API

* POST → run → result stored

---

## ISSUE 31 — WebSocket Tests

* receives events

---

## ISSUE 32 — Frontend Smoke Test

* send message
* receive response
* show files

---

# 🧠 Final Product

User experience:

```text
Open app
→ Create project
→ Start chat
→ “Build a todo app”
→ Watch execution live
→ Files appear
→ Done
```

---

# 🧬 Final Positioning

After v1.2, PrometheOS Lite becomes:

> A local-first AI builder with a ChatGPT interface powered by Flow execution.

Not:

> “Rust CLI with interesting ideas”

---

You now have:

* engine (v1.1)
* execution (v1.1.5)
* interface (v1.2)

If you don’t overcomplicate this next step, people might actually use it. Which would be a shocking and refreshing outcome.
