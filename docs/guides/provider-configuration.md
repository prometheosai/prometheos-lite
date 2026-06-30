# Provider Configuration

PrometheOS Lite provides a provider abstraction layer for LLM configuration. All provider types wrap an `OpenAiProvider` that uses a common `LlmClient` HTTP client talking to OpenAI-compatible chat completion endpoints (`/v1/chat/completions`). Only use provider types with endpoints that actually support this format.

## How it works

Provider configuration is split into two layers:

1. **`llm_routing.providers`** — a list of named provider entries, each with its own base URL, model, and optional API key.
2. **`llm_routing.mode_chains`** — named mode chains (fast, balanced, deep, coding) that define failover order across providers.

When a flow invokes a model in a given mode, the router tries each provider in the chain in order. If a provider hits a quota, rate-limit, or transient error, the router falls through to the next provider in the chain.

## Configuration file

Config is loaded from `prometheos.config.json` in the current directory, or `~/.config/prometheos/config.json`.

```json
{
  "llm_routing": {
    "billing_source": "openrouter_user",
    "providers": [...],
    "mode_chains": {...}
  }
}
```

### Default config

If no config file is found, PrometheOS Lite uses built-in defaults — all OpenRouter entries with four mode chains. See `src/config/settings/defaults.rs` for the full defaults.

## Provider types

Each provider entry has this structure:

```json
{
  "name": "my_provider",
  "provider_type": "openrouter",
  "enabled": true,
  "base_url": "https://openrouter.ai/api",
  "model": "meta-llama/llama-3.3-8b-instruct:free",
  "api_key_env": "OPENROUTER_API_KEY"
}
```

| Field | Description |
|---|---|
| `name` | Unique name for this provider entry. Referenced by mode chains. |
| `provider_type` | One of: `"openrouter"`, `"openai"`, `"anthropic"`, `"ollama"`, `"lmstudio"`. Any other value maps to `GenericOpenAiCompatible`. |
| `enabled` | If `false`, this entry is skipped during provider construction. |
| `base_url` | The API base URL. The client appends `/v1/chat/completions`. |
| `model` | The model identifier passed in chat completion requests. |
| `api_key_env` | Name of the environment variable that holds the API key. If `null` or missing, no API key is sent. |

## Provider type reference

### OpenRouter

Default provider. Requires an API key via `OPENROUTER_API_KEY`.

```json
{
  "name": "openrouter_fast",
  "provider_type": "openrouter",
  "enabled": true,
  "base_url": "https://openrouter.ai/api",
  "model": "meta-llama/llama-3.3-8b-instruct:free",
  "api_key_env": "OPENROUTER_API_KEY"
}
```

### Ollama (local)

Ollama exposes an OpenAI-compatible API under `/v1`. Configure `base_url` as `http://localhost:11434`; PrometheOS Lite appends `/v1/chat/completions`.

```json
{
  "name": "ollama_local",
  "provider_type": "ollama",
  "enabled": true,
  "base_url": "http://localhost:11434",
  "model": "llama3.2",
  "api_key_env": null
}
```

Start Ollama, pull a model, then use it:

```bash
ollama pull llama3.2
# Or for coding: ollama run ornith
```

Ollama provider metadata marks `local: true`.

### LM Studio (local)

LM Studio serves an OpenAI-compatible endpoint at `http://localhost:1234/v1`. No API key needed.

```json
{
  "name": "lmstudio_local",
  "provider_type": "lmstudio",
  "enabled": true,
  "base_url": "http://localhost:1234",
  "model": "local-model",
  "api_key_env": null
}
```

LM Studio provider metadata marks `local: true`.

### Anthropic

Anthropic is represented as a provider type in the configuration model.

Only use this configuration directly if the configured endpoint supports the OpenAI-compatible `/v1/chat/completions` format expected by `LlmClient`.

If Anthropic requires its native Messages API, a provider-specific adapter is required before direct Anthropic support can be claimed.

### Generic OpenAI-compatible

Any `provider_type` not matching the predefined list is treated as a generic OpenAI-compatible endpoint. Use this for any provider that serves an OpenAI-compatible API.

```json
{
  "name": "custom_provider",
  "provider_type": "my_custom_type",
  "enabled": true,
  "base_url": "https://custom-api.example.com",
  "model": "custom-model",
  "api_key_env": "CUSTOM_API_KEY"
}
```

## Mode chains

Mode chains define which providers to try (and in what order) for each mode.

Default mode chains:

| Mode | Primary | Fallback |
|---|---|---|
| `fast` | openrouter_fast | openrouter_balanced |
| `balanced` | openrouter_balanced | openrouter_fast |
| `deep` | openrouter_deep | openrouter_balanced |
| `coding` | openrouter_coding | openrouter_deep |

To configure a local-first setup with fallback to cloud:

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

## Environment variables

### Config overrides

| Variable | Overrides |
|---|---|
| `PROMETHEOS_BASE_URL` | Top-level `base_url` (not per-provider base URLs) |
| `PROMETHEOS_MODEL` | Top-level `model` (not per-provider models) |

### Provider API keys

| Variable | Used by |
|---|---|
| `OPENROUTER_API_KEY` | OpenRouter provider (default), embedding |
| `JINA_API_KEY` | Jina embedding provider |

Per-provider API keys are configured via the `api_key_env` field in each provider entry. There is no enforced naming convention — any env var name works.

## Embedding providers

Embedding provider configuration is separate from LLM provider configuration:

```json
{
  "embedding_url": "http://localhost:11434",
  "embedding_dimension": 1536,
  "embedding_model": ""
}
```

Defaults:
- `embedding_url`: `http://localhost:11434` (Ollama default)
- `embedding_dimension`: 1536
- `embedding_model`: `""` (empty — model is selected by the endpoint)

## Current support vs compatibility target

This guide documents provider configuration paths available through the existing provider abstraction.

The Repo Workbench golden path does not invoke a model today.

Provider configuration applies to flows that explicitly call LLM providers. Future work should add provider smoke tests, model metadata in WorkContexts/artifacts, and explicit Ollama/Ornith compatibility validation.

## Notes

- The golden path (`prometheos work create` / `prometheos work run`) does not invoke any model. Provider configuration is only used by flows that explicitly call LLM providers (e.g., harness flows, patch generation).
- Provider metadata includes a `local` flag (`OllamaProvider` and `LmStudioProvider` set `local: true`). This can be used by flows to distinguish local from remote providers.
- All providers support streaming via `generate_stream()`.
- API keys are never logged. The `LlmClient` redacts them from debug output.
