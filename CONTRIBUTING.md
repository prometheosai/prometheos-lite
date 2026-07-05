# Contributing to PrometheOS Lite

This project favors clarity, simple module boundaries, and local-first reliability.

## Development Setup

1. Install Rust stable.
2. Clone the repository.
3. Run:

```bash
cargo build
cargo test
```

## Branch and PR Rules

- Create one focused branch per issue.
- Open one PR per issue unless explicitly grouped.
- Link the issue in the PR body (`Closes #<issue>`).
- Keep PRs small and reviewable.

## Required Checks (Local and CI)

Run these before opening/updating a PR:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

PRs should not be merged unless CI is green.

## Review Standards

- At least 1 approving review from a code owner.
- No unresolved review comments.
- No skipped required checks.
- Changes must be testable locally.

## Coding Guidelines

- Keep modules independent (`cli`, `agents`, `core`, `llm`, `fs`, `logger`, `config`).
- Avoid unnecessary abstractions and frameworks.
- Prefer readable async flows over advanced orchestration.
- Return actionable errors with clear context.
- Add tests for behavior changes.

## Commit Message Convention

Use concise conventional prefixes:

- `feat(...)`
- `fix(...)`
- `chore(...)`
- `docs(...)`
- `refactor(...)`
- `test(...)`
