# 🧠 PrometheOS Lite — Execution PRD (GitHub Issues & PRs)

**Stack:** Rust (CLI-first, local execution)
**Goal:** Ship a fast, reliable, local-first multi-agent CLI that developers can use and extend

---

# 🎯 CORE OBJECTIVE

Build a CLI tool that:

* Runs multiple AI agents locally
* Streams execution in real-time
* Generates real files on disk
* Works with local LLMs (LM Studio-compatible APIs)
* Is simple enough for contributors to extend quickly

---

# 🧱 PHASE 0 — PROJECT FOUNDATION

---

## 📌 ISSUE #1 — Initialize Rust Workspace & CLI

**Goal:** Create a working CLI entrypoint

**Tasks:**

* Initialize Rust project (`cargo new prometheos-lite`)
* Add CLI parsing with `clap`
* Create base command:

```bash
prometheos run "<task>"
```

**PR:**

* `feat(cli): initialize Rust CLI with run command`

---

## 📌 ISSUE #2 — Project Structure

**Goal:** Define modular structure for contributors

```text
/src
  /cli
  /agents
  /core
  /llm
  /fs
  /logger
  /config
```

**PR:**

* `chore: scaffold project modules`

---

## 📌 ISSUE #3 — Async Runtime Setup

**Goal:** Enable async execution across system

**Tasks:**

* Add `tokio`
* Setup async main

**PR:**

* `chore(runtime): add tokio async runtime`

---

# ⚡ PHASE 1 — LLM INTEGRATION

---

## 📌 ISSUE #4 — LLM Client (Local-first)

**Goal:** Connect to LM Studio-compatible endpoint

**Tasks:**

* HTTP client with `reqwest`
* POST `/v1/chat/completions`
* Configurable:

  * base_url
  * model

**Interface:**

```rust
pub async fn generate(prompt: &str) -> Result<String>
```

**PR:**

* `feat(llm): implement local-first LLM client`

---

## 📌 ISSUE #5 — Config Loader

**Goal:** Load runtime configuration

**File:**

```text
prometheos.config.json
```

**Fields:**

* provider
* base_url
* model

**PR:**

* `feat(config): add config loader`

---

# 🧠 PHASE 2 — AGENT SYSTEM

---

## 📌 ISSUE #6 — Agent Trait

**Goal:** Standard interface for all agents

```rust
pub trait Agent {
    fn name(&self) -> &str;
    async fn run(&self, input: &str) -> Result<String>;
}
```

**PR:**

* `feat(agents): define agent trait`

---

## 📌 ISSUE #7 — Planner Agent

**Goal:** Structure tasks into steps

**Behavior:**

* Break input into logical steps
* Return structured plan text

**PR:**

* `feat(agents): implement planner agent`

---

## 📌 ISSUE #8 — Coder Agent

**Goal:** Generate files and code

**Tasks:**

* Call LLM with task prompt
* Return structured output (files + content)

**PR:**

* `feat(agents): implement coder agent`

---

## 📌 ISSUE #9 — Reviewer Agent

**Goal:** Improve generated output

**Tasks:**

* Analyze coder output
* Refine structure and correctness

**PR:**

* `feat(agents): implement reviewer agent`

---

# ⚙️ PHASE 3 — ORCHESTRATION

---

## 📌 ISSUE #10 — Sequential Orchestrator

**Goal:** Coordinate agent execution

**Flow:**

```text
Planner → Coder → Reviewer
```

**Tasks:**

* Pass outputs between agents
* Maintain execution context

**PR:**

* `feat(core): add sequential orchestrator`

---

# 🎥 PHASE 4 — REAL-TIME EXPERIENCE

---

## 📌 ISSUE #11 — Structured Logger

**Goal:** Clear agent-based logs

**Format:**

```text
[Planner] → ...
[Coder] → ...
[Reviewer] → ...
```

**Tasks:**

* Create logger module
* Support streaming logs

**PR:**

* `feat(logger): implement structured agent logger`

---

## 📌 ISSUE #12 — Streaming Renderer

**Goal:** Display output progressively

**Tasks:**

* Render text as it arrives
* Handle chunked responses

**PR:**

* `feat(logger): add streaming output renderer`

---

## 📌 ISSUE #13 — Execution Timeline

**Goal:** Improve readability of flow

**Tasks:**

* Show step transitions
* Emit clear lifecycle events

**PR:**

* `feat(core): add execution timeline events`

---

# 📁 PHASE 5 — FILE SYSTEM

---

## 📌 ISSUE #14 — File Parser

**Goal:** Extract files from LLM output

**Tasks:**

* Detect file blocks
* Extract filenames + content

**PR:**

* `feat(fs): implement file parser`

---

## 📌 ISSUE #15 — File Writer

**Goal:** Persist generated files

**Tasks:**

* Create `/prometheos-output`
* Write files safely
* Handle basic conflicts

**PR:**

* `feat(fs): implement file writer`

---

# ⚙️ PHASE 6 — CLI EXPERIENCE

---

## 📌 ISSUE #16 — CLI Output Improvements

**Goal:** Improve usability

**Tasks:**

* Add loading states
* Print output directory
* Clear success/failure messages

**PR:**

* `feat(cli): improve CLI UX`

---

## 📌 ISSUE #17 — Error Handling

**Goal:** Ensure stability

**Tasks:**

* Handle LLM failures
* Retry basic requests
* Graceful error messages

**PR:**

* `fix: add error handling and retry logic`

---

# 🚀 PHASE 7 — RELEASE READINESS

---

## 📌 ISSUE #18 — Demo Optimization

**Goal:** Ensure consistent high-quality outputs

**Tasks:**

* Tune prompts
* Validate common use cases

**PR:**

* `perf: optimize default prompts`

---

## 📌 ISSUE #19 — Documentation

**Tasks:**

* Final README
* Example commands
* Output samples

**PR:**

* `docs: finalize documentation`

---

# 🧩 OPTIONAL POST-LAUNCH

---

## 📌 ISSUE #20 — Plugin Interface

* Allow custom agents

---

## 📌 ISSUE #21 — Basic Web Viewer

* Optional UI for logs

---

# 🧠 CONTRIBUTOR GUIDELINES (IMPORTANT)

* Keep modules independent
* Avoid unnecessary abstractions
* Prioritize clarity over complexity
* Prefer simple async flows over advanced orchestration
* All features must be testable locally
