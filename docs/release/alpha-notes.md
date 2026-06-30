# PrometheOS Lite Alpha Notes

PrometheOS Lite is currently in alpha.

This alpha focuses on one useful local workflow: running the Repo Workbench against a repository, generating review artifacts, recording approval decisions, and preserving memory so work can continue later.

## What is included

- Local `prometheos` CLI install path.
- Product-facing `prometheos work ...` command surface.
- Repo Workbench MVP.
- File-backed WorkContext storage.
- Repository scanning.
- Risk pattern detection.
- Risk report artifact.
- Suggested patch plan artifact.
- Approval recording.
- WorkContext memory.
- Continuation.
- Zero-to-First-Value guide.
- Demo script.
- CI golden-path coverage.
- Install guide.
- Release checklist.

## Demo workflow

```bash
prometheos work create \
  --repo fixtures/repo-workbench/rust-risky \
  --goal "Find risky code and suggest safe improvements" \
  --mode review \
  --json

prometheos work run <work_id>
prometheos work artifacts <work_id>
prometheos work memory show <work_id>
prometheos work continue <work_id>
```

## What the demo proves

- A WorkContext can be created from a local repository.
- The repository can be scanned.
- Risk-review artifacts can be generated.
- Memory can be stored and shown.
- A WorkContext can be continued.
- The workflow does not modify fixture source files.

## Safety model

- `work run` reads source files and writes artifacts/memory under `.prometheos-lite/workbench/`.
- `work approve` records approval only.
- No automatic patch application.
- No source file mutation during analysis.
- The golden-path CI verifies fixture source files are not modified.

## What is not included yet

- Automatic patch application.
- Full autonomous coding.
- Cloud control plane.
- Team workspace.
- Plugin marketplace.
- Mnemosyne integration.
- Brain integration.
- Voice/UI layer.
- SATI/trading workflows.

## Recommended first test

Use the included fixture:

```bash
prometheos work create \
  --repo fixtures/repo-workbench/rust-risky \
  --goal "Find risky code and suggest safe improvements" \
  --mode review \
  --json
```

Then follow:

- [Install guide](../guides/install.md)
- [Zero-to-First-Value guide](../guides/zero-to-first-value.md)
- [Repo Workbench guide](../guides/repo-workbench-mvp.md)

## Model compatibility direction

The alpha workflow does not depend on a specific coding model.

PrometheOS Lite is intended to remain model-agnostic: local and hosted models should eventually plug into the WorkContext workflow while preserving reviewable artifacts, approval gates, memory, and continuation.

See [Local Model Compatibility](../guides/local-model-compatibility.md).

## Next milestones

- Add demo screenshots or GIF.
- Verify install smoke test on Linux CI.
- Add alpha release tag.
- Add GitHub Release notes.
- Add optional JSON output for remaining workbench commands.
- Add explicit approval-gated patch application later.
