# Local Model Compatibility

PrometheOS Lite is designed to be a local-first workbench for safe autonomous software workflows.

The model is not the product boundary. The workflow is.

PrometheOS Lite should be able to wrap coding models through:

- repository context
- WorkContexts
- artifacts
- approval gates
- memory
- continuation
- trace collection

## Why local coding models matter

Local coding models are improving quickly. They make it possible to run more developer workflows locally, reduce dependence on remote providers, and experiment with repo-aware automation in a safer environment.

## Ollama and Ornith

Ollama provides a local model runtime.

Ornith is an example of a local/open coding-model family that can be served through Ollama.

Example Ollama command:

```bash
ollama run ornith
```

PrometheOS Lite does not currently invoke Ornith directly. The Repo Workbench MVP is deterministic and does not require a model to reach first value.

## Current PrometheOS Lite state

The current Repo Workbench MVP does not require a model to reach first value. It scans repositories, detects risk patterns, generates artifacts, records approval decisions, and writes memory locally.

PrometheOS Lite already supports OpenAI-compatible endpoints through provider configuration, including local providers such as LM Studio and Ollama. These are available for flows that invoke a model, but the golden path does not depend on them.

## Target compatibility path

Future local model compatibility should look like:

```text
prometheos work
  -> WorkContext
  -> repo scan
  -> model provider
  -> artifact generation
  -> approval gate
  -> memory
  -> continuation
```

## Future provider goals

PrometheOS Lite should eventually support:

- local deterministic analysis
- Ollama-compatible local models
- OpenAI-compatible local endpoints
- hosted model providers
- explicit model selection per WorkContext
- provider metadata in artifacts and memory

## Safety model

Local model support must preserve the current safety model:

- no automatic source modification during analysis
- no automatic patch application
- approval records decisions only
- artifacts are reviewable before any future apply step
- source mutation must remain behind an explicit approval-gated command

## Non-goals for alpha

The alpha does not yet promise:

- automatic coding
- autonomous patch application
- benchmark-level performance claims
- Brain learning integration
- Mnemosyne memory backend
- cloud orchestration
- model training
