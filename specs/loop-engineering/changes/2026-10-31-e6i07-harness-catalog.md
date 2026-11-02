# Change: E6/I07 Slice C — Generated Capability Registry and CI Drift Protection

**Issue:** #154
**Depends on:** Slice A (`b710e9da66`), Slice B (PR #283, merged at `be890036637613f1e4676f5349643314862fe88b`), SPEC 007 / SOMA `#83` negotiation core, #117/#118 governed gates.

## Why this slice

Slice A shipped the seam, the declaration, the catalogue, and the conformance kit. Slice B bound harness selection to SPEC 007 negotiation. Both left the capability catalog as a runtime-generated artifact with no committed canonical form and no drift detection: a declaration could change without updating the published catalog, and there was no fail-closed validation of declaration consistency.

Slice C closes that gap: the catalog is now generated deterministically from implementation-owned declarations, validated for consistency, and protected against drift by a committed fixture and regression tests.

## Scope

- **Enhanced `HarnessCatalogEntry`** — expanded to include effect classes, spec versions, adapter families, locality, checkpoint support, event streaming, read projection, privacy classes, evidence attachment, validation, approval escalation, unclaimed capabilities, and over-claimed flag.
- **`HarnessDeclarationV1::validate()`** — fail-closed validation returning a list of findings: schema version, empty identity, empty capabilities, duplicate capabilities, over-claimed capabilities, identity divergence, stale spec versions.
- **`HarnessCatalog::validate()`** — fail-closed validation: schema/conformance versions, duplicate harness IDs, invalid declaration digests, over-claimed entries, unsorted entries.
- **Deterministic generation** — `HarnessRegistry::catalog()` produces byte-identical output regardless of registration order (entries sorted by `harnessId`).
- **Committed fixture** — `tests/fixtures/harness_catalog_canonical.json` is the canonical generated catalog.
- **Drift detection test** — regenerates the catalog from known declarations and compares byte-for-byte to the committed fixture.
- **Stale artifact detection** — modifying a declaration produces a different catalog digest.
- **Capability/authority separation** — the catalog contains declared capabilities and limitations, never granted authority.

## Invariants enforced (fail closed)

- Capability does not imply authority: the catalog contains no authority/grant/privilege fields.
- Harness identity is separate from workspace identity.
- No adapter may bypass #117/#118 governance (unchanged from Slice A/B).
- No duplicated source of truth: the catalog is generated from declarations, not hand-maintained.
- No silent substitution or privilege expansion: over-claimed capabilities are detected.
- No hardcoded catalog that can silently diverge: drift detection test catches changes.
- Generated artifacts reproduce byte-for-byte: deterministic JSON with sorted entries.

## Refusal codes

| Code | Meaning |
| --- | --- |
| (validation findings) | `HarnessDeclarationV1::validate()` returns findings for malformed declarations |
| (validation findings) | `HarnessCatalog::validate()` returns findings for inconsistent catalogs |

## Non-goals honored

No concrete external adapter, no transport, no scheduler, no autonomous-loop graduation, no dependency change, no `prometheos work` change. The nine gates, permit enforcement, and `HarnessDispatch` are untouched.

## Verification evidence

- `cargo fmt --check` → exit 0
- `cargo clippy --all-targets --all-features -- -D warnings` → exit 0
- `cargo test --test harness_catalog_generation` → 8/8 PASS
- `cargo test --test execution_harness_conformance` → 5/5 PASS
- `cargo test --test execution_harness_selection` → 10/10 PASS
- Drift detection: committed fixture matches generated output byte-for-byte
- Stale artifact detection: modified declaration produces different catalog

## Budget note

Exceeds the default 200-net-line preference (enhanced catalog + validation + 8 tests + fixture + change record). The escalation is explicit: the drift protection is meaningless without the validation and tests that prove it works.