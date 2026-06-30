# Provider Architecture Audit

## Summary

PrometheOS Lite has a complete multi-provider LLM abstraction layer that is not used by the current Repo Workbench golden path. The golden path is fully deterministic and does not invoke any model provider. The provider architecture supports OpenRouter, OpenAI-compatible endpoints, Ollama, LM Studio, and Anthropic through a common `LlmClient` HTTP client, but all local/local-model features are unused by the golden path today.

## Current provider-related code

| File | Purpose | Used by golden path? | Status |
|------|---------|---------------------|--------|
| `src/repo_workbench.rs` | Deterministic risk scanner and artifact generator | Yes (all of it) | Active, no model calls |
| `src/llm/client/llm_client.rs` | OpenAI-compatible HTTP client for chat completions | No | Active |
| `src/llm/client/types.rs` | Request/response types for chat completions | No | Active |
| `src/flow/intelligence/provider.rs` | `LlmProvider` trait + OpenRouter, OpenAi, Anthropic, Ollama, LmStudio, GenericOpenAiCompatible providers | No | Active |
| `src/flow/intelligence/router.rs` | `ModelRouter` - mode-aware provider selection with failover and cooldown | No | Active |
| `src/flow/intelligence/mod.rs` | Public re-exports for intelligence module | No | Active |
| `src/config/settings/types.rs` | `AppConfig`, `LlmProviderConfig`, `LlmRoutingConfig`, `ModeChains` | Indirect (not model-specific) | Active |
| `src/config/settings/defaults.rs` | Default provider entries (all OpenRouter-first), default model, base URL | Indirect | Active |
| `src/config/settings/loader.rs` | Config loading from env/file | Indirect | Active |
| `src/cli/runtime_builder.rs` | Builds `ModelRouter` with configured providers + embedding provider selection | No | Active |
| `src/cli/commands/repo_workbench.rs` | CLI wrapper for repo_workbench functions | Yes | Active |
| `src/cli/commands/work.rs` | `prometheos work ...` command surface | Yes (delegates to repo_workbench) | Active |
| `src/harness/patch_provider.rs` | LLM-backed patch provider for harness flows | No | Active |
| `src/flow/memory/embedding.rs` | Embedding providers (OpenRouter, Jina, local) | No | Active |

## Current Repo Workbench model usage

- **Does `prometheos work create` invoke a model?** No. It scans the repository and creates a `WorkbenchContext` with metadata. All deterministic.
- **Does `prometheos work run` invoke a model?** No. It calls `analyze_risks()` (pattern-based), `render_risk_report()`, and `render_patch_suggestions()` — all pure Rust functions operating on local file content only.
- **Are risk reports deterministic/local today?** Yes. `analyze_risks()` at `src/repo_workbench.rs:541` performs pattern matching and heuristics against source files. No LLM call.
- **Are suggested patch plans generated without a model today?** Yes. `render_patch_suggestions()` at `src/repo_workbench.rs:672` generates template-based suggestions from findings. No LLM call.

## Current provider support

### OpenRouter
- **Classification: Supported**
- Default provider with 4 mode entries (fast, balanced, deep, coding).
- Configured via `OPENROUTER_API_KEY` env var.
- `OpenRouterProvider` wraps `OpenAiProvider` with name `"openrouter"`.
- Default base URL: `https://openrouter.ai/api`.
- Default model per mode: varies (all free-tier OpenRouter models).

### OpenAI-compatible endpoints
- **Classification: Supported**
- `OpenAiProvider` at `src/flow/intelligence/provider.rs:106`.
- `LlmClient` connects to `{base_url}/v1/chat/completions`.
- Configurable via `provider_type: "openai"` in provider config.
- API key via env var specified in `api_key_env` field.

### LM Studio
- **Classification: Supported**
- `LmStudioProvider` wraps `OpenAiProvider` with name `"lmstudio"`.
- `local: true` in metadata.
- Configurable via `provider_type: "lmstudio"` in provider config.
- Also used in `patch_provider.rs` error messages as example configuration.
- LM Studio serves an OpenAI-compatible endpoint at `http://localhost:1234/v1`.

### Ollama
- **Classification: Supported**
- `OllamaProvider` wraps `OpenAiProvider` with name `"ollama"`.
- `local: true` in metadata.
- Configurable via `provider_type: "ollama"` in provider config.
- Ollama serves an OpenAI-compatible endpoint at `http://localhost:11434/v1`.
- A default embedding URL of `http://localhost:11434` exists in config defaults.

### Anthropic
- **Classification: Supported**
- `AnthropicProvider` wraps `OpenAiProvider` with name `"anthropic"`.
- Uses OpenAI-compatible chat completions format (Anthropic's API supports this).

### Generic OpenAI-compatible
- **Classification: Supported**
- `GenericOpenAiCompatibleProvider` wraps `OpenAiProvider` with a custom name.
- Catch-all for any provider type not explicitly matched.

### Environment-variable configuration
- **Classification: Partially supported**
- `PROMETHEOS_BASE_URL` - overrides base URL from config loader.
- `PROMETHEOS_PROVIDER`, `PROMETHEOS_MODEL` - used in harness/patch_provider.rs.
- `OPENROUTER_API_KEY` - used by OpenRouter provider and embedding.
- `JINA_API_KEY` - used by Jina embedding provider.
- Per-provider `api_key_env` field in config supports arbitrary env var names.
- No consistent env-var convention across all providers.

### Config-file configuration
- **Classification: Supported**
- `prometheos.config.json` or `~/.config/prometheos/config.json`.
- Full `AppConfig` with `llm_routing` section supporting multiple providers and mode chains.
- Config loader supports env-var overrides for individual fields.

## Gaps for Ollama/Ornith compatibility

1. **No golden-path integration**: The golden path does not invoke any model. Adding Ornith would require either a separate flow or extending the golden path.
2. **No per-WorkContext model selection**: Model/provider selection is configured globally, not per-WorkContext.
3. **No model metadata in artifacts**: Artifacts do not record which provider/model generated them.
4. **No explicit Ornith model name**: Ornith is not listed in any config defaults, provider examples, or documentation.
5. **No provider-agnostic test path**: Tests for provider/LLM code require live API keys or endpoints.
6. **No consistent provider discovery**: Provider selection is by configured name, not by capability negotiation.
7. **No streaming in golden path**: Even if a model were added, the golden path output format does not use streaming.
8. **Ollama provider exists but is not default**: Users must explicitly configure `ollama` as their provider type; it is not listed in the default provider entries.
9. **Embedding provider selection is separate from LLM provider selection**: Different config fields control embedding vs. LLM endpoints.

## Smallest safe integration path

1. **Provider config audit cleanup**: Normalize env-var conventions across providers. Document existing provider support in a concise reference doc.
2. **Explicit provider config docs**: Create a provider configuration guide showing how to configure Ollama, LM Studio, and OpenRouter with examples.
3. **Model metadata in WorkContext/artifacts**: Add optional `provider` and `model` fields to WorkContext and artifact metadata when a model is invoked (unchanged for deterministic path).
4. **OpenAI-compatible local endpoint adapter**: No new code needed — `GenericOpenAiCompatibleProvider` and `LlmClient` already support this. Only documentation is missing.
5. **Ollama compatibility**: Already works through the existing `ollama` provider type and OpenAI-compatible endpoint. Needs configuration docs.
6. **Ornith compatibility**: Would work through Ollama (`ollama run ornith`) using the existing `ollama` provider type. No code changes needed. Needs documentation only.
7. **Optional model invocation in golden path**: Extend `work run` with an optional `--model` flag or mode that invokes a configured model for deeper analysis. This should remain optional — the deterministic path should always work without a model.

## Risks

- **Overclaiming support**: Updating docs to say "prometheos supports Ollama" when no golden-path flow uses Ollama could mislead. Current README already claims "Local providers such as LM Studio/Ollama through provider entries" which is technically correct (they are configured providers) but no golden-path flow exercises them.
- **Coupling golden path to one model**: If model invocation is added to the golden path, the deterministic fallback must remain available for users without a model running.
- **Breaking deterministic first-value path**: Adding model requirements to `work run` would break the zero-to-first-value promise. Model invocation must remain optional.
- **API key leakage**: The fixture repo at `fixtures/repo-workbench/rust-risky/src/main.rs` contains `let secret = format!("api_key = {}", "sk-1234567890");` as a test pattern, and `api_key` is listed in risk-detection patterns. Provider config code must not log API keys.
- **Provider complexity before alpha release**: Adding more provider options before the alpha release could distract from stabilizing the golden path and release process.

## Recommendation

The next PR should be a **provider configuration guide** (`docs/guides/provider-configuration.md`) that documents:

- How to configure OpenRouter (current default)
- How to configure Ollama (including Ornith)
- How to configure LM Studio
- How to configure OpenAI-compatible endpoints
- Environment variables reference
- Config file reference
- How the provider selection and failover work

This requires no code changes. It closes the documentation gap for local model support while the alpha golden path remains deterministic, model-agnostic, and safe.

After the docs PR, a follow-up could add optional `--model` / `--provider` flags to `prometheos work run` for model-augmented analysis, but only after the alpha release is stable and the deterministic path is proven.
