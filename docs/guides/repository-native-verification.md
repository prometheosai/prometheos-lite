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

## The serial core variant

The `core` suite caps the test-thread count at four. The cap exists because the git-fixture tests create their own repositories, and a real-time antivirus scan of a freshly written object file intermittently denies that write (`unable to write file .git/objects/...: Permission denied`). Measured on a contended review host: **0 failures across 15 sequential** repository creations, **5–7 failures across 48 creations at four-way parallelism**, and a rotating two-to-ten test failures per suite run. The affected tests pass standalone, so the failures are environmental rather than code defects.

`core-serial` is the recorded alternative for a contended host: the **same checks, the same test inventory, the same assertions**, with `--test-threads 1` on the "all tests" check. It is derived from the `core` specification in code, so the two suites cannot drift apart.

```bash
python3 scripts/local_ci.py run --suite core-serial
```

The canonical `core` suite is unchanged and remains a valid way to satisfy the core requirement. The verifier accepts either; the evidence records the exact command that produced it, so which variant ran is always visible. A `core-serial` record whose "all tests" command names any other thread count is rejected, like any other command that does not match its specification exactly.

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

Teams may mirror the same command on infrastructure they own, but mirrors do not outrank the repository evidence contract. GitHub-hosted workflow files are intentionally absent so billing, quota, or service availability cannot block development. The repository policy gate (`python scripts/local_ci_policy.py`, part of the core suite) enforces this invariant: the presence of any hosted workflow file, external CI configuration, or LFS marker fails the suite.

## Repository integrity invariants

The policy gate enforces three more invariants. They are general rules, not signatures of any particular content, and each one fails closed with a message naming what it found. The inputs the gate reads are reported as violations rather than raised as errors: a missing, malformed, non-object, empty, unreadable, or directory-shaped pin file, an unreadable pinned surface, an unreadable or non-UTF-8 source file, and an unreadable vendored file are all named in the message.

| Invariant | Rule |
|---|---|
| Executable-configuration review pins | A pinned execution surface — build, bundler, toolchain, or verification-runner configuration — may only change when a human re-pins it deliberately. A missing, unreadable, or **empty** pin set is itself a violation: a control that can be switched off by emptying its own reviewed file is not a control. |
| Duplicate JSON keys | A tracked JSON document may not contain a duplicate key: two parsers reading it differently is a supply-chain hazard, not a style choice. Every tracked JSON document is scanned; only an explicit list of known-invalid fixtures is exempt, so a new invalid fixture is caught and must be exempted deliberately. |
| Vendored byte fidelity | Every file under `vendored/` must be LF-only. The bundle is upstream-locked under the `-text` gitattribute, so a checkout that rewrites line endings is a provenance violation. |

The gate enforces two further rules that predate these three: hosted-CI / LFS independence (above) and the fallback-confinement / API-LLM-boundary rule, which is documented in `docs/guides/product-surface-inventory.md` and scans every API root that exists, so neither `src/api` nor a top-level `api/` can escape it.

The gate's own behaviour is covered by `python scripts/test_local_ci_policy.py`: 26 checks that run the real gate against throwaway sandboxes — a missing, malformed, non-object, empty, unreadable, and directory-shaped pin file; a changed, deleted, and contaminated pinned surface; a duplicate key in a tracked document, in a document found by the filesystem fallback, and in a newly added fixture; CR bytes in a vendored file; an LLM-call boundary violation under each API root, alone and with both roots present; an unreadable pinned surface; a non-UTF-8 source file; a hosted workflow; and an LFS marker. The script is part of the core suite, so a change to the gate that stops rejecting any of these fails the suite.

### Pinned execution surfaces

Pins live in `scripts/reviewed_config_digests.json` and cover the files a build or the verification runner executes. Digests are computed over LF-normalized bytes, so a platform checkout does not change the verdict.

The set covers the surfaces the repository's own verification executes or reads, and the toolchain and bundler inputs it builds from — 23 surfaces in four groups:

| Group | Surfaces |
|---|---|
| Verification runner | `scripts/local_ci.py`, `scripts/local_ci_policy.py`, `scripts/review_config_digests.py`, `scripts/test_local_ci_verify.py`, `scripts/test_local_ci_policy.py`, the smoke scripts under `scripts/` and `scripts/demo/` |
| Rust toolchain inputs | `Cargo.toml`, `Cargo.lock`, `.cargo/config.toml`, `clippy.toml` |
| Frontend bundler inputs | `frontend/package.json`, `frontend/package-lock.json`, `frontend/next.config.js`, `frontend/postcss.config.js`, `frontend/tailwind.config.ts`, `frontend/tsconfig.json`, `frontend/.eslintrc.json`, `frontend/scripts/smoke.mjs` |
| Repository attributes | `.gitattributes` |

To change a pinned surface deliberately:

1. Make the change.
2. Run `python scripts/review_config_digests.py`.
3. Review the diff — the only expected change is the digest of the file you changed.
4. Commit the change and the re-pin together.

To pin a *new* surface, add its path to the `files` object in the pin file first; the set of pinned surfaces is curated, not auto-discovered.

The pin mechanism is a **change-review requirement**, not a content filter: it does not look for any particular code, so it cannot be satisfied by evading a signature. Its limit is equally explicit — a reviewer who re-pins a malicious change has approved it, so the control is the review, not the mechanism. Three consequences are worth stating plainly:

- The set is **curated and not exhaustive** of every file that could influence a build.
- The pin file is **not itself pinned**. Narrowing the set to a single surface whose digest matches makes the gate pass with the other surfaces unpinned; a reviewer sees the narrowing in the diff.
- The gate's own pin is **checked by the gate**, so replacing the gate script with a no-op makes the gate exit successfully. The signal for that is the code diff, i.e. review.

A change to a pinned surface — including a dependency change, which alters `Cargo.lock` and `frontend/package-lock.json` — surfaces in the diff as a reviewed act rather than passing silently.

