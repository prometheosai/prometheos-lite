# Ollama and Ornith Compatibility

PrometheOS Lite is model-agnostic by design.

The current Repo Workbench golden path does not require a model. It performs deterministic static analysis and writes reviewable artifacts locally.

Provider-backed flows can use the existing OpenAI-compatible provider path where supported.

## What is supported today

- Provider configuration can describe Ollama, LM Studio, OpenRouter, OpenAI-compatible endpoints, and generic providers.
- Provider configuration parsing is covered by tests.
- OpenAI-compatible request/response behavior is covered by mock provider tests.
- Repo Workbench artifacts include provenance metadata.
- The deterministic Repo Workbench path explicitly records that no model was invoked.

## What is not claimed yet

- No automatic Ornith integration is claimed.
- No benchmark claims are made.
- No model download is performed by PrometheOS Lite.
- No normal CI test requires Ollama or Ornith.
- No source patching is performed automatically.
- No provider-backed Repo Workbench generation is claimed unless implemented separately.

## Ollama setup

Install Ollama using its official instructions.

Then run:

```bash
ollama pull llama3.2
ollama run llama3.2
```

For Ornith, use the model name available in the local Ollama registry. Example:

```bash
ollama run ornith
```

Only use the exact model name available locally.

## PrometheOS provider config example

Use a provider entry like:

```json
{
  "name": "ollama_local",
  "provider_type": "ollama",
  "enabled": true,
  "base_url": "http://localhost:11434",
  "model": "ornith",
  "api_key_env": null
}
```

Note:

PrometheOS Lite appends `/v1/chat/completions` through the OpenAI-compatible client path. Configure `base_url` as `http://localhost:11434`, not `http://localhost:11434/v1/chat/completions`.

## Local-first mode chain

```json
{
  "mode_chains": {
    "fast": ["ollama_local"],
    "balanced": ["ollama_local", "openrouter_balanced"],
    "deep": ["ollama_local", "openrouter_deep"],
    "coding": ["ollama_local", "openrouter_coding"]
  }
}
```

## Provenance

When model-backed flows are used in the future, artifacts should record provider and model metadata.

Current deterministic Repo Workbench artifacts record:

- model invoked: no
- provider: none
- model: none

## Manual local endpoint smoke test

This test is ignored by default and never runs in normal CI.

Example:

```bash
PROMETHEOS_TEST_LOCAL_BASE_URL=http://localhost:11434 \
PROMETHEOS_TEST_LOCAL_MODEL=ornith \
cargo test local_openai_compatible_endpoint -- --ignored
```

## Manual validation checklist

- [ ] Ollama installed locally
- [ ] desired model pulled locally
- [ ] local endpoint reachable
- [ ] provider config points to `http://localhost:11434`
- [ ] flow using provider path succeeds
- [ ] generated artifact provenance is accurate, if artifacts are generated
