# PrometheOS Lite v1.6.1-alpha.1

> ⚠️ **Alpha release.** This is a manual alpha. Review the safety model before use.

PrometheOS Lite is a **local-first AI workbench** for safe autonomous software workflows. It scans repositories, creates WorkContexts, generates review artifacts, records approval decisions, and preserves memory so work can continue later.

## ✨ What's in this alpha

### Repo Workbench golden path
- Create work contexts against local repositories
- Deterministic static analysis (tree-sitter powered)
- Reviewable risk reports and patch plan artifacts
- Approval recording (no automatic patch application)
- Memory persistence and work continuation

### Provider architecture
- OpenAI-compatible provider abstraction (OpenRouter, Ollama, LM Studio, BYOK)
- Provider configuration with mode-chain routing
- Mock HTTP provider smoke tests (no external deps)
- Local model compatibility documentation

### Provenance
- Artifacts record whether a model was invoked
- Deterministic Repo Workbench artifacts record "no model invoked"
- Reviewable evidence trail

### CI
- Linux install smoke test
- Repo Workbench golden path test
- Source file mutation guard

## 📦 Install

```bash
git clone https://github.com/prometheosai/prometheos-lite.git
cd prometheos-lite
cargo install --path . --force
prometheos --version
```

## 🚀 First value in 5 minutes

```bash
prometheos work create \
  --repo fixtures/repo-workbench/rust-risky \
  --goal "Find risky code and suggest safe improvements" \
  --mode review \
  --json

prometheos work run <work_id>
prometheos work artifacts <work_id>
prometheos work memory show <work_id>
```

## 🛡️ Safety model

| Rule | Detail |
|---|---|
| No source modification | `work run` analyzes, does not mutate |
| Approval only | `work approve` records decisions, applies nothing |
| Reviewable artifacts | All generated artifacts are human-readable |
| Provenance tracked | Every artifact records model/provider info |
| CI-verified | Golden path CI verifies fixture files unchanged |

## 🔮 Not included (yet)

- Automatic patch application
- Full autonomous coding
- Cloud/team control plane
- Brain learning / Mnemosyne memory
- Plugin marketplace
- UI/voice layer
- Benchmark claims

## 📚 Docs

- [Install guide](docs/guides/install.md)
- [Zero-to-first-value guide](docs/guides/zero-to-first-value.md)
- [Repo Workbench MVP guide](docs/guides/repo-workbench-mvp.md)
- [Provider configuration guide](docs/guides/provider-configuration.md)
- [Ollama and Ornith compatibility guide](docs/guides/ollama-ornith-compatibility.md)
- [Local model compatibility guide](docs/guides/local-model-compatibility.md)

## 🏷️ Suggested tag commands

Do not run these unless explicitly authorized:

```bash
git tag -a v1.6.1-alpha.1 -m "PrometheOS Lite v1.6.1-alpha.1"
git push origin v1.6.1-alpha.1
```
