# Installing PrometheOS Lite

## Prerequisites

- [Rust toolchain](https://rustup.rs/) (latest stable)
- Cargo
- Git
- ~2 GB free disk space for the compiler
- No API keys required

## Install from local checkout

```bash
git clone https://github.com/prometheosai/prometheos-lite.git
cd prometheos-lite
cargo install --path .
```

## Verify installation

```bash
prometheos --version
prometheos --help
```

Expected output:

```
prometheos --version
prometheos-lite 1.6.1

prometheos --help
Usage: prometheos <COMMAND>
...
```

## Run first-value workflow

Once installed, run the Repo Workbench against the included fixture:

```bash
prometheos work create \
  --repo fixtures/repo-workbench/rust-risky \
  --goal "Find risky code and suggest safe improvements" \
  --mode review \
  --json
```

Then follow the remaining steps in [docs/guides/zero-to-first-value.md](zero-to-first-value.md).

## Troubleshooting

### `prometheos: command not found`

Cargo installs binaries into Cargo's bin directory. Ensure it is on your PATH:

- **Linux/macOS**: `~/.cargo/bin`
- **Windows**: `%USERPROFILE%\.cargo\bin`

Add it to your PATH if needed:

```bash
export PATH="$HOME/.cargo/bin:$PATH"    # Linux/macOS
```

Or use the full path:

```bash
~/.cargo/bin/prometheos --version
```

### `error: failed to compile`

Ensure you have the latest stable Rust toolchain:

```bash
rustup update stable
```

## Uninstall

```bash
cargo uninstall prometheos-lite
```

## Reinstall after pulling changes

```bash
git pull
cargo install --path . --force
```

## Safety model

- `work run` reads source files and writes artifacts and memory under `.prometheos-lite/workbench/`.
- `work approve` records approval in the context store only.
- No repository source files are modified during `work run`.
- No automatic patch application.
- No network access required.

## Next steps

- [Zero-to-First-Value guide](zero-to-first-value.md)
- [Repo Workbench full guide](repo-workbench-mvp.md)
