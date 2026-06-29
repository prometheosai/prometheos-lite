# Repo Workbench MVP Guide

This branch adds the first reviewable PrometheOS Lite Repo Workbench MVP path.

The MVP is intentionally small, local-first, and file-backed. It does not mutate repository files. It scans a local repo, creates a WorkContext-like record, runs a read-only risk review, stages artifacts, requires explicit approval, writes memory, and supports continuation.

## Command Surface

The Repo Workbench MVP supports two command surfaces:

### Product-facing path (preferred)

```bash
prometheos work create --repo . --goal "Find risky code and suggest safe improvements" --mode review
prometheos work run <work_id>
prometheos work artifacts <work_id>
prometheos work approve <artifact_id>
prometheos work memory show <work_id>
prometheos work continue <work_id>
```

### Legacy/direct path

```bash
prometheos repo create --repo . --goal "Find risky code and suggest safe improvements" --mode review
prometheos repo run <work_id>
prometheos repo status <work_id>
prometheos repo artifacts <work_id>
prometheos repo approve <artifact_id>
prometheos repo memory show <work_id>
prometheos repo continue <work_id>
```

`repo-workbench` is also accepted as an alias for `repo`.

```bash
prometheos repo-workbench status <work_id>
```

Both surfaces call the same implementation. The `work` path is the product-facing interface; `repo` remains available for backward compatibility.

## Golden Path

```bash
# Product-facing path
cargo run -- work create \
  --repo . \
  --goal "Find risky code and suggest safe improvements" \
  --mode review

cargo run -- work run <work_id>

cargo run -- work artifacts <work_id>

cargo run -- work approve <artifact_id>

cargo run -- work memory show <work_id>

cargo run -- work continue <work_id>
```

### Legacy path (same implementation)

```bash
cargo run -- repo create \
  --repo . \
  --goal "Find risky code and suggest safe improvements" \
  --mode review

cargo run -- repo run <work_id>

cargo run -- repo artifacts <work_id>

cargo run -- repo approve <artifact_id>

cargo run -- repo memory show <work_id>

cargo run -- repo continue <work_id>
```

## Storage

The MVP stores local state under the analyzed repo:

```text
.prometheos-lite/workbench/
├── contexts/
├── artifacts/
└── memory/
```

This keeps the first version boring, inspectable, and easy to delete.

## Safety Model

The MVP never writes changes into the target repository source files.

- `run` performs read-only analysis.
- `run` creates markdown artifacts.
- `approve` records approval only.
- A future writer must explicitly consume an approved artifact before any write can happen.

This keeps the first shipped loop safe while proving the product shape.

## Current Risk Heuristics

The first scanner checks common risky patterns, including:

- Rust panics from `unwrap`, `expect`, and `panic!`
- unfinished `TODO` / `FIXME` markers
- possible hardcoded secrets or credentials
- dynamic `eval` usage
- Python `shell=True`
- direct `innerHTML` usage

These are MVP heuristics, not a full security audit. The point is to prove the local workflow, not cosplay as a compliance department on day one.

## MVP Definition of Done

The branch satisfies the first MVP slice when:

- A user can create a local repo work context.
- The repo is scanned without modifying files.
- A risk report artifact is created.
- A suggested patch plan artifact is staged.
- Approval is required and recorded.
- Memory is written and can be shown later.
- Continue restores useful status and memory.
