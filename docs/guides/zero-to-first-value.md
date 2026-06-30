# Zero-to-First-Value: PrometheOS Lite Repo Workbench

Reach a useful result in under 5 minutes.

## Prerequisites

- [Rust toolchain](https://rustup.rs/) (latest stable)
- ~2 GB free disk space for the compiler
- No API keys required

## Clone and build

```bash
git clone https://github.com/prometheosai/prometheos-lite
cd prometheos-lite
cargo install --path .
```

Or build without installing:

```bash
cargo build
```

See [docs/guides/install.md](install.md) for full installation options.

## Run against the included fixture

The repository includes a small, intentionally-risky Rust project under `fixtures/repo-workbench/rust-risky/`.

### 1. Create a work context

```bash
cargo run -- work create \
  --repo fixtures/repo-workbench/rust-risky \
  --goal "Find risky code and suggest safe improvements" \
  --mode review
```

Expected output:

```
Created Repo Workbench WorkContext
  ID: a1b2c3d4-...
  Title: Find risky code and suggest safe improvements
  Repo: fixtures/repo-workbench/rust-risky
  Mode: review
  Project type: rust
  Candidate files: 1
  Next: prometheos work run a1b2c3d4-...
```

For machine-readable output (useful in scripts and CI), add `--json`:

```bash
cargo run -- work create \
  --repo fixtures/repo-workbench/rust-risky \
  --goal "Find risky code and suggest safe improvements" \
  --mode review \
  --json
```

### 2. Run analysis

Replace `<work_id>` with the ID from step 1:

```bash
cargo run -- work run <work_id>
```

Expected output:

```
Repo Workbench run complete
  WorkContext: a1b2c3d4-...
  Status: complete
  Files considered: 1
  Findings: 2
  Risk report: .prometheos-lite/workbench/contexts/.../artifacts/risk-report-...
  Suggested patch plan: .prometheos-lite/workbench/contexts/.../artifacts/patch-plan-...
```

### 3. List artifacts

```bash
cargo run -- work artifacts <work_id>
```

You will see two artifacts:

- `risk-report-...` — a detailed risk findings report
- `patch-plan-...` — a suggested patch plan

### 4. Inspect memory

```bash
cargo run -- work memory show <work_id>
```

Shows the persisted memory blob including the goal, repo summary, and decisions made.

### 5. Continue a context

Continue restores context from saved memory and prints the recommended next action:

```bash
cargo run -- work continue <work_id>
```

### 6. Approve an artifact (optional)

Approval records your decision in the context store. It does not modify source files.

```bash
cargo run -- work approve <artifact_id>
```

## Example end-to-end session

```bash
# Create
cargo run -- work create \
  --repo fixtures/repo-workbench/rust-risky \
  --goal "Find risky code and suggest safe improvements" \
  --mode review

# Run (paste the work_id from above)
cargo run -- work run a1b2c3d4-...

# See what was produced
cargo run -- work artifacts a1b2c3d4-...

# Read the risk report
cat .prometheos-lite/workbench/contexts/a1b2c3d4-.../artifacts/risk-report-*.md

# Read the patch plan
cat .prometheos-lite/workbench/contexts/a1b2c3d4-.../artifacts/patch-plan-*.md

# Inspect memory
cargo run -- work memory show a1b2c3d4-...

# Continue the context
cargo run -- work continue a1b2c3d4-...
```

## Legacy command surface

The same implementation is available under the `repo` subcommand:

```bash
cargo run -- repo create \
  --repo fixtures/repo-workbench/rust-risky \
  --goal "Find risky code and suggest safe improvements" \
  --mode review
```

Use `repo` instead of `work` in the commands above. The output and behavior are identical.

## Where artifacts are stored

All Repo Workbench state is written to:

```
.prometheos-lite/workbench/
  contexts/
    <work_id>/
      context.json        # WorkContext metadata and decisions
      memory.json         # Persisted memory for continuation
      artifacts/
        risk-report-<id>.md
        patch-plan-<id>.md
```

All paths are relative to the repository root passed via `--repo`.

For the current alpha scope and safety model overview, see [Alpha Notes](../release/alpha-notes.md). For a demo transcript, see [Repo Workbench Demo](../demo/repo-workbench-transcript.md).

## Safety model

- `work run` reads source files and writes artifacts under `.prometheos-lite/workbench/`.
- `work approve` records approval in the context store only.
- No repository source files are read-write accessed.
- No automatic patch application.
- No network access required.

## Cleanup

Remove all Repo Workbench state for a repository:

```bash
rm -rf fixtures/repo-workbench/rust-risky/.prometheos-lite
```

Or remove state for a specific context:

```bash
rm -rf .prometheos-lite/workbench/contexts/<work_id>
```

## You have reached first value when

- [ ] A WorkContext ID was created.
- [ ] A risk report artifact was generated.
- [ ] A suggested patch plan artifact was generated.
- [ ] Memory can be shown.
- [ ] Continue restores useful context.
- [ ] No source files were modified.

## Troubleshooting

| Symptom | Likely cause |
|---------|-------------|
| `not found: workbench context` | The work ID is invalid or the context file was deleted. Re-run `work create`. |
| `error: invalid digit found in string` | The work ID was truncated. Copy the full UUID. |
| Built but command not found | Run from the project root with `cargo run -- ...` or install: `cargo install --path .`. |
