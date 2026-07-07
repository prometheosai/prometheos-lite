# PrometheOS Lite — Agent Instructions

This is the first file every coding agent should read before working in this repository.

PrometheOS Lite follows the Loop Engineering Protocol.

## Required reading

Before modifying the repository, read:

- [Loop Engineering Protocol](docs/LOOP_ENGINEERING.md)
- [Agent Protocol](specs/loop-engineering/AGENT_PROTOCOL.md)
- [Safety Gates](specs/loop-engineering/SAFETY_GATES.md)
- [Product Surface Inventory](docs/guides/product-surface-inventory.md)
- [Model-layer positioning](docs/research/model-layer-positioning.md)
- [Autonomous loop graduation criteria](docs/research/autonomous-loop-graduation-criteria.md)

## Product boundaries

```
Stable alpha:
- prometheos work

Experimental:
- prometheos serve
- frontend
- API server
- local model / Ornith validation
- harness execution loop
- autonomous execution loop

Future / not alpha:
- Brain
- Mnemosyne integration
- cloud/team control plane
- plugin marketplace
- benchmark claims
- autonomous coding claims
```

## Rules

- No unattended merges.
- No CI weakening.
- No dependency changes without explicit approval.
- No benchmark claims without completed validation evidence.
- No automatic Ornith/local model support claims unless validated.
- No frontend promotion.
- No API server promotion.
- No autonomous execution promotion.
- No Brain/Mnemosyne/cloud/plugin claims.
- No unrelated refactors.
- One concern per PR.
- Prefer small PRs.
- Record exact verification evidence.
- Do not claim unrun checks passed.
- Update progress/handoff files when working under an active queue.

## Minimality budget

Default PR budget:
- Prefer 5 files or fewer.
- Prefer 200 net changed lines or fewer.
- Prefer one bounded task per PR.
- Escalate before exceeding the budget.

These are defaults, not hard limits. Some docs/planning PRs may exceed them with explicit scope.

## Verification expectations

Rust baseline:
```bash
cargo fmt --check
cargo check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Frontend baseline:
```bash
cd frontend
npm ci
npm run lint
npm run build
```

Frontend checks are required when frontend files, frontend docs, frontend CI, or frontend-related queue files are touched.
