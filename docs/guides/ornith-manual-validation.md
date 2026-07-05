# Ornith Manual Validation Guide

This guide explains how to manually validate Ornith with PrometheOS Lite through an OpenAI-compatible local endpoint.

Ornith support is not automatic. This is a manual compatibility path.

PrometheOS Lite remains model-agnostic.

The stable alpha workflow remains `prometheos work`.

## Status

Status: manual / experimental

This guide verifies local endpoint compatibility only.

It does not claim:

- benchmark performance
- production support
- automatic Ornith integration
- full harness value
- PrometheOS Lite improves Ornith results

## Prerequisites

- Rust toolchain
- PrometheOS Lite repo
- Ollama or another OpenAI-compatible local runtime
- Ornith model available locally
- enough local hardware for the selected model

## Option A: Ollama

Start Ollama and pull the model:

```bash
ollama pull ornith
ollama run ornith
```

If the model name in your local registry differs (e.g. `ornith:9b`, `ornith:35b`), use that exact name.

Ollama exposes an OpenAI-compatible API. Configure the base URL as:

```text
http://localhost:11434
```

PrometheOS Lite appends `/v1/chat/completions` through the client path. Do not include `/v1` in the base URL.

## Option B: Generic OpenAI-compatible endpoint

Any local runtime that serves an OpenAI-compatible chat completions endpoint can be used — LM Studio, vLLM, SGLang, or similar.

Configure the base URL and model according to the runtime's documentation. No API key is required unless the runtime itself requires one.

## Run the manual endpoint test

Use the existing ignored integration test with the exact environment variables from the repo:

```bash
PROMETHEOS_TEST_LOCAL_BASE_URL=http://localhost:11434 \
PROMETHEOS_TEST_LOCAL_MODEL=ornith \
cargo test local_openai_compatible_endpoint -- --ignored
```

If you are using a different model name or base URL, set the env vars accordingly.

## Expected result

A passing test means:

* PrometheOS Lite can send a chat-completion request to the local endpoint.
* The local endpoint can respond.
* Request/response parsing works.
* No external hosted provider is required.
* No API key is required for the local endpoint, unless the runtime itself requires one.

A passing test does not mean:

* benchmark validation
* Repo Workbench model integration
* full harness validation
* frontend support
* stable alpha promotion

## Troubleshooting

| Symptom | Likely cause |
|---|---|
| `connection refused` | local runtime not running |
| `404 not found` | wrong base URL (should be `http://localhost:11434` without `/v1`) |
| `model not found` | wrong model name or model not pulled |
| `out of memory` | insufficient memory/GPU for the selected model |
| `response parse error` | endpoint not fully OpenAI-compatible or response shape mismatch |
| timeout | model too large for hardware or endpoint overloaded |

## Relationship to PrometheOS Lite

* PrometheOS Lite is not the model.
* Ornith is a model provider candidate.
* PrometheOS Lite is the workbench that records context, provenance, approvals, artifacts, and verification paths.
* This guide validates compatibility, not product superiority.

## Boundary: autonomous execution loop is not part of this validation

This guide validates manual Ornith endpoint compatibility only.

It does not promote the autonomous execution loop.

PrometheOS Lite currently distinguishes between:

- the stable alpha path: `prometheos work`, which is read-only and produces review artifacts
- experimental harness execution paths, which may evaluate, dry-run, checkpoint, rollback, and apply patches depending on mode and policy

The autonomous execution loop must not be promoted until a separate graduation decision exists.

At minimum, that future decision should require:

- the model validation plan in `docs/research/model-layer-positioning.md` has been run
- manual Ornith/local endpoint validation has produced real results
- harness value has been compared against direct model usage
- a small internal benchmark slice exists
- minimality enforcement is wired into the real execution path and tested
- escalation behavior is documented and tested
- plan-locking / scope-locking exists and is tested
- rollback/checkpoint behavior is covered by tests
- docs clearly distinguish read-only review, assisted patching, and autonomous patching

Until then, the autonomous loop remains experimental.

## Next steps after manual validation

If the manual validation passes, future PRs can add:

* repeatable local model comparison harness
* fixture-based tasks
* artifact-level model comparison reports
* optional model-augmented Repo Workbench mode

If it fails, future PRs should document:

* endpoint incompatibility
* required adapter changes
* model/runtime-specific quirks
