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

## Required Repository-Native Verification

Hosted CI is not a merge authority. After committing the candidate revision, run:

```bash
python3 scripts/local_ci.py run --suite core
python3 scripts/local_ci.py run --suite platform
python3 scripts/local_ci.py run --suite smoke
```

Windows uses `py -3` and Git Bash for the smoke suite. Ordinary changes run all three suites on one owned machine. Platform-sensitive changes and releases require explicitly selected owned-platform evidence; unavailable relevant coverage must be disclosed for human review. Frontend changes also run `--suite frontend`. See [Repository-Native Verification](docs/guides/repository-native-verification.md).

PRs should not be merged unless exact-commit evidence is complete and independently reviewed.

## Review Standards

- At least 1 approving review from a code owner.
- No unresolved review comments.
- No missing or invalid required evidence.
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
