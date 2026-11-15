# Governed-workflow evaluation fixtures (Lite-owned, experimental)

Issue #136 Slice A delivers the first repeatable, governed-workflow evaluation
fixtures for PrometheOS Lite. This page documents what Lite owns, what it
deliberately does not own, and the exact status of the Foundry boundary.

**No benchmark score, pass rate, comparison, certification or public claim is
produced by these fixtures. There is no Foundry integration claim.**

## Ownership boundary

| Concern | Owner |
|---|---|
| Executable product scenarios (pinned repositories, tasks, expected outcomes) | **Lite** |
| Fixture content pinning (content digests, fixture revisions) | **Lite** |
| Deterministic local fixture runner around the governed dispatch boundary | **Lite** |
| Raw evidence records (route evidence, snapshot, attempt, journal entry, accounting, graph) | **Lite** |
| Scenario descriptor schema (`lite.fixtures.scenario`) and run-envelope namespace (`lite.evidence.scenario-run`) | **Lite** (internal) |
| External benchmark runner, matrix aggregation, metrics, comparisons, certification, public-claim policy | **Foundry** |
| The Foundry external evaluation format (unpublished) | **Foundry** |

Lite produces tasks and raw evidence. Foundry owns everything downstream of
that. Nothing in this repository may present Lite-internal envelopes as a
Foundry contract, and no code path treats a Lite-internal record as a
benchmark result.

## What Slice A contains

- **Scenario descriptor** — versioned (`schemaVersion: "1.0.0"`, namespace
  `lite.fixtures.scenario`), validated in
  `src/workflow/scenario.rs` with `deny_unknown_fields`, failing closed on
  malformed documents, unsupported versions/languages, path leakage and
  missing or ambiguous pins.
- **Pinned fixtures** (`tests/fixtures/scenarios/<id>/`):
  - `rust-bugfix-arithmetic` — a deliberately broken Rust crate (content-digest
    pinned; `-text` in `.gitattributes` so the digest is byte-stable);
  - `python-bugfix-strings` — a deliberately broken Python module (same
    pinning rules).
  - Each directory pins: inputs (task statement), repository content digest
    (sha256 over the sorted `path hash` listing — not machine paths, not git
    history), expected observable outcome, evidence artifacts, policy profile,
    authority, workspace configuration (`existing-read-only`, adapter revision
    `lite.workspace.adapter.v1`, read-only), harness identity + revision
    (`lite.harness.local_model` / `v1`), route identity (route set, candidate
    pin, provider, model, injected evaluation/dispatch/trust instants), and the
    controlled/variable dimension split (Slice A fixtures are fully
    controlled).
- **Deterministic local runner** — `tests/scenario_fixtures.rs` executes each
  scenario through the real governed boundary (`resolve_route` →
  `dispatch_resolved_route` → `account_routed_outcome`) with a loopback model
  fixture. No network beyond `127.0.0.1`, no paid provider, no fixture
  toolchain execution, no unattended authority.
- **Lite-internal run envelope** — `LiteScenarioRunEvidence` (namespace
  `lite.evidence.scenario-run`, `schemaVersion: "1.0.0"`) carries: scenario id
  and digest, repository/route/workspace pins, the full digest chain over the
  native records (route evidence, snapshot, decision, requirements, candidate
  set, attempt, journal entry, accounting record), the raw sealed accounting
  record and graph aggregate, the classification, and an explicitly typed
  `externalEvaluationFormat: { status: "unsupported" }` stub.

## Failure classification

Setups and tasks are classified, never silently counted as product/model
failure:

| Class | Meaning |
|---|---|
| `setup-error` | Deterministic setup defect: malformed descriptor, unsupported version/language, fixture digest drift, path leakage, missing/ambiguous pins, pre-effect route/workspace/authority refusal. |
| `invalid-task` | The task itself is broken or self-contradictory under the pinned authority/workspace (empty/too-short statement; read-only workspace pinned with writable scopes; route/policy profile disagreement; contradictory authority). |
| `infrastructure-error` | The local harness/runtime pipeline failed after a governed dispatch (e.g., accounting failed to seal). |
| `governed-product-outcome` | The governed boundary ran. A typed, sealed failure inside the effect is **still a product outcome** and is accounted as such; it is not a setup error and says nothing about model capability beyond this scenario. |

## Repeatability

- All evaluation instants are injected; the trusted dispatch clock is fixed
  per fixture (`FixedTrustedTimeSource`).
- The resolution side of the boundary pins every instant, so route evidence,
  snapshot and decision digests reproduce byte-for-byte across reruns.
- The effect side is sealed after the governed effect settles and embeds the
  executing process's wall-clock instants (`startedAt`/`completedAt`,
  measured dispatch latency). Byte equality is therefore asserted only on
  wall-clock-free canonical parts; effect-side records are re-verified on
  every rerun. Any claim of fully byte-identical effect chains would be
  false and is not made.
- Fixture content drift fails closed (`fixture_content_digest`); fixtures are
  `-text` in `.gitattributes` so CRLF conversion cannot change the pinned
  digest.

## Foundry contract status

The Foundry external evaluation format **does not exist yet** (no published
schema, no consumed-by contract). Slice A therefore ships a
`LiteScenarioRunEvidence` envelope that is Lite-internal, carries no Foundry
namespace as a claim of consumption, and types its external-format stub as
`unsupported` — fail-closed at re-verification
(`external_evaluation_format_is_typed_unsupported`).

When Foundry publishes the format, the "Foundry import" work belongs in a
separate, explicitly reviewed change that pins Foundry's schema revision.
Until then:

- Lite does **not** claim Foundry consumes these envelopes;
- Lite does **not** emit benchmark metrics, scores or comparisons;
- Foundry can reproduce latency/token/cost/digest measurements from the raw
  evidence only after Foundry defines its own import contract — that
  definition is out of scope here.
