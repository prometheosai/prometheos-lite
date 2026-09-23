# Repository-Native Verification

PrometheOS Lite does not require GitHub Actions or another hosted CI/CD service for merge or release authority.

The authoritative gate is `scripts/local_ci.py`. It runs repository-owned commands and emits SHA-256-protected JSON evidence bound to the exact Git commit, operating system, architecture, and clean-working-tree state.

## Run the gates

Run after committing the candidate revision:

```bash
python3 scripts/local_ci.py run --suite core
python3 scripts/local_ci.py run --suite platform
python3 scripts/local_ci.py run --suite smoke
```

On Windows, use `py -3` instead of `python3` and run the smoke suite from Git Bash. For an ordinary change, run `core`, `platform`, and `smoke` on one owned machine. Frontend changes additionally require:

```bash
python3 scripts/local_ci.py run --suite frontend
```

Evidence is written under `.local-ci/evidence/<commit>/` and ignored by Git. Attach it or paste its digest into the PR/review record. Evidence produced with `--allow-dirty` is explicitly non-mergeable.

## Verify collected evidence

```bash
python3 scripts/local_ci.py verify --commit <full-sha> <evidence-files...>
```

The verifier requires `core`, `platform`, and `smoke` evidence. Every record must match the exact commit, report a clean tree, contain only passing checks, and retain its evidence digest. Frontend evidence is additionally required when frontend paths change.

For platform-sensitive changes and releases, require each relevant owned platform explicitly:

```bash
python3 scripts/local_ci.py verify --commit <full-sha> \
  --require-platform linux \
  --require-platform darwin \
  --require-platform windows \
  <evidence-files...>
```

An unavailable platform does not silently pass. Record the missing coverage in the review, and the human reviewer decides whether it is relevant to the change. Ordinary development is not blocked merely because three owned machines are unavailable.

A human reviewer verifies the relevant evidence before merge. No hosted status API is authoritative.

## Deployment and releases

Releases remain deliberate local operations. Run the evidence gate, complete `docs/release/release-checklist.md`, obtain explicit human approval, then create and push the tag manually. This repository does not automatically deploy, publish, sign, or release from a hosted event.

Teams may mirror the same command on infrastructure they own, but mirrors do not outrank the repository evidence contract. GitHub-hosted workflow files are intentionally absent so billing, quota, or service availability cannot block development.
