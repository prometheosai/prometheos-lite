# Safely fix a real bug with PrometheOS Lite

This tutorial walks through the governed patch workflow: a model/provider
generates edits, but **every change passes through dry-run, human approval, a
checkpoint, application, validation, and a recoverable evidence report** before
it touches your tree permanently.

PrometheOS does not trust provider output. A generated patch is treated as
hostile input: absolute paths, `..` traversal, Windows drive/UNC paths, plain
non-diff text, and out-of-scope files are all rejected. Provenance is recorded
without secrets.

## 1. Install

```bash
git clone https://github.com/prometheosai/prometheos-lite.git
cd prometheos-lite
cargo install --path .
prometheos --version
```

## 2. Configure a provider

The `workflow generate --provider config` path reads a small config file from
the current directory and optional environment overrides:

`prometheos.config.json`:

```json
{
  "provider": "openai",
  "model": "your-model-name",
  "base_url": "https://openrouter.ai/api/v1"
}
```

Environment overrides (optional):

```bash
export PROMETHEOS_BASE_URL="https://openrouter.ai/api/v1"
export PROMETHEOS_MODEL="your-model-name"
```

If your provider needs an API key, supply it per the provider's own
documentation. PrometheOS records only a sanitized `scheme://host[:port]` in
provenance and never persists keys, tokens, or authorization headers.

For an offline, no-model run, use `--provider mock` instead of `--provider config`.

## 3. Choose a repository

Pick a real repository you want changed. PrometheOS operates on a Git checkout
and creates a checkpoint branch before applying anything, so the working tree
must be a Git repository.

```bash
cd /path/to/your-repo
git status   # clean tree recommended
```

## 4. Set scope

Limit blast radius with `--allowed`, `--forbidden`, `--max-files`, and
`--max-lines`. Anything outside the allowed globs is rejected before any
artifact is created.

```bash
prometheos workflow generate \
  --repo /path/to/your-repo \
  --goal "Fix the off-by-one in the parser boundary check" \
  --authority assist \
  --allowed "src/**" \
  --forbidden "src/vendor/**" \
  --max-files 3 \
  --max-lines 120 \
  --validate "cargo test" \
  --provider config
```

This prints a `<WORKFLOW_ID>` and a `patch_hash`. Keep both.

## 5. Generate the proposal

The command above already generated the proposal. If you prefer to generate
separately from inspecting, re-run `generate` any time; each run produces a new
governed proposal artifact.

## 6. Inspect the report

Before approving anything, read the evidence report:

```bash
prometheos workflow report --repo /path/to/your-repo <WORKFLOW_ID>
```

Confirm:

- `provider_provenance.implementation`, `.model`, and `.route` are correct and
  contain no secrets.
- No API keys, tokens, or `Authorization` headers appear anywhere in the output.
- `changed_files` match the goal and stay inside your `--allowed` scope.
- `validation_command` is recorded (e.g. `cargo test`).
- `patch` and `patch_hash` are present and exact.
- `approved`, `dry_run_passed`, and `applied` are all `null` — your source
  files are **unchanged** until you approve and apply.

## 7. Dry-run

The proposal is applied to an isolated Git worktree and your validation command
runs there. Your real tree is untouched.

```bash
prometheos workflow dry-run --repo /path/to/your-repo <WORKFLOW_ID>
```

A failed dry-run (including a failing validation command) records the failure
and blocks apply. No changes reach your tree.

## 8. Approve

Approval is bound to the exact `patch_hash`. A different hash is rejected.

```bash
prometheos workflow approve \
  --repo /path/to/your-repo \
  <WORKFLOW_ID> \
  --patch-hash <PATCH_HASH> \
  --approver <your-name>
```

## 9. Apply

Apply creates a checkpoint branch (`prometheos/checkpoint-<WORKFLOW_ID>`),
re-checks scope, runs validation again, and only then applies the patch.

```bash
prometheos workflow apply \
  --repo /path/to/your-repo \
  <WORKFLOW_ID> \
  --patch-hash <PATCH_HASH>
```

If validation fails after apply, PrometheOS attempts an automatic rollback and
records `rollback_status` (`clean`, `rolled_back`, or `rollback_failed`) in the
report.

## 10. Rollback / recovery

Recovery is built into the artifact:

- The checkpoint branch preserves the pre-apply state.
- `report` always shows `checkpoint_ref` and `rollback_status`.
- If apply left the tree modified and rollback was disabled, use the checkpoint:

```bash
git checkout <checkpoint_ref> -- .     # restore files from checkpoint
# or restore the whole branch state from the checkpoint ref
```

## Security warning

The validation command you pass to `--validate` (and to `dry-run`/`apply`) is
executed with **your operating system permissions and is not sandboxed**. A
malicious or careless validation string can read, write, or execute anything
your user account can. Do not pass untrusted validation commands. The `sh -c`
invocation used for validation is intentionally unsandboxed; treat it like a
command you typed into your own shell.
