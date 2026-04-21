# 🧠 PrometheOS Lite

**Run a team of AI agents locally.**

PrometheOS Lite is a Rust-based, local-first multi-agent CLI that plans, generates, and refines code in real time using AI agents.

---

## ⚡ Features

- Multi-agent execution:
  - Planner → structures the task
  - Coder → generates files and code
  - Reviewer → improves and fixes output
- Real-time streaming logs in terminal
- Local-first (LM Studio-compatible APIs)
- Generates real project files on disk
- Simple, extensible architecture for contributors

---

## 🚀 Quick Start

### 1. Clone the repository

```bash
git clone https://github.com/prometheosai/prometheos-lite
cd prometheos-lite
````

---

### 2. Install dependencies

```bash
cargo build
```

---

### 3. Configure

Create a `prometheos.config.json` file:

```json
{
  "provider": "lmstudio",
  "base_url": "http://localhost:1234/v1",
  "model": "your-model-name"
}
```

---

### 4. Run

```bash
cargo run -- run "build a simple SaaS landing page"
```

---

### 5. Output

* Live agent execution in terminal
* Generated files saved in:

```bash
/prometheos-output/
```

---

## 🧠 How It Works

```text
User Input
   ↓
Planner Agent
   ↓
Coder Agent
   ↓
Reviewer Agent
   ↓
File Output
```

* Sequential orchestration
* Stateless agents
* Local-first execution

---

## 📦 Project Structure

```text
/src
  /cli        → CLI entrypoint
  /agents     → planner, coder, reviewer
  /core       → orchestration logic
  /llm        → model client (LM Studio-compatible)
  /fs         → file parsing & writing
  /logger     → streaming logs
  /config     → configuration loader
```

---

## 🔌 Model Support

Compatible with:

* Local models via LM Studio (OpenAI-compatible API)
* Any OpenAI-compatible endpoint

---

## 🧑‍💻 Contributing

We welcome contributions.

### Setup

```bash
cargo build
cargo run -- run "test task"
```

### Guidelines

* Keep modules simple and focused
* Prefer clarity over abstraction
* Avoid unnecessary complexity
* Ensure features run locally without external dependencies

---

## 📌 Roadmap

* [ ] Improved file generation
* [ ] Better prompt tuning
* [ ] Plugin system for custom agents
* [ ] Optional web-based log viewer

---

## ⚠️ Disclaimer

This tool generates code automatically.
Always review outputs before using in production.

---

## 🔗 PrometheOS

PrometheOS Lite is part of the PrometheOS ecosystem.

GitHub: [https://github.com/orgs/prometheosai](https://github.com/orgs/prometheosai)
