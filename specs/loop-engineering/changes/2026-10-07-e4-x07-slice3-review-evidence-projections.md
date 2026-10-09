# Change: E4/X07 Slice 3 — review-report + evidence-timeline projections (repair round 2)

**Issue:** #243 (parent #164); repair of REVIEW FAILED at `b64cabce156cf93e0b32214a8b4edd85f7618d0f` (previous repair `d12180f`).
**Governing spec:** `docs/architecture/e4-x07-slice-3-review-evidence-projections.md` (rev updated in this round to remove fabricated semantics for composite-boundary attribution, to add the timeline event-count contract, and to remove `evidence_bundles` from the review input contract).
**Base:** `main@80f201d791d21c58477b6d895ad9475fd329ee28` (`#240` SPEC 007).

## Repair (5 concrete defects resolved at `b64cabce156cf93e0b32214a8b4edd85f7618d0f`)

1. **Gate basis digest binding (item 1)** — `HumanDecisionRecordV1.basisEvidenceDigest` was documented in `graph_gates.rs:100` as the *reviewed artifact digest* (`artifactDigest`), but the code (`validate_gate_basis`, `supported_by_gate_basis` in `review.rs`) resolved it against `EvidenceReference.eventDigest`. Fixed to resolve against `artifactDigest`; added positive (`artifactDigest==basis`) and negative (`eventDigest==basis` but `artifactDigest` missing → `SOMA-CMP-0004`) controls.
2. **Fabricated private-boundary attribution (item 2)** — `generalize_file` (`review.rs:822`) inferred containment from `ReviewIssue.file` path prefixes (`starts_with`). `ReviewIssue.file` is not a workflow composite/node identity; the authoritative issue→boundary binding (required by spec §6.3) does not exist in Slice 3. Removed `authorizedBoundaries` from `ReviewDisclosurePolicy`, removed `validate_boundaries`, `composite_boundaries`, and `generalize_file`, and revised spec §4.5/§6.3/§10-row-12. Filenames are never interpreted as composite ids; findings render verbatim when `files` is authorized. Replaced test with `filenames_cannot_masquerade_as_composite_identities`.
3. **Fabricated zero digest (item 3)** — `TimelineEventView.semantic_digest` emitted a 64-zero `Hex64` (`withheld_digest()` at `timeline.rs:319`) when `referenceDigest` was withheld. Changed to `Option<Hex64>` (`None` when withheld); deleted `withheld_digest()`. Added `withheld_reference_digest_is_null_not_zero_digest`.
4. **No-op timeline count authorization (item 4)** — `TIMELINE_COUNT_TARGETS = ["events"]` (`timeline.rs:38`) was validated but never read. Implemented independently gated `TimelineProjectionPayload.event_count` (`Some(events.len() as u64)` when `count_authorization` includes `"events"`, else `None` + withheld omission). Added positive/withheld/unknown-target tests and updated spec §6.2/§5.1.
5. **Semantically ignored authoritative input (item 5)** — `ReviewFacts.evidence_bundles` (`review.rs`) was never read, rendered, or bound. The approved contract (§4.0) named it authoritative input, but no legitimate projection contribution exists (`EvidenceBundle` has no `EvidenceReference` mapping, and `final_state` is evaluate/recovery data, not projection output). Removed the field from `ReviewFacts` and the `NO_BUNDLES` test helper; revised spec §4.0/§4.2. Added mutation regression `every_accepted_review_fact_changes_render_or_fails_closed`.

## Files changed (`6` files; budget exceeded with explicit approval — single bounded repair)

- `src/workflow/projection/review.rs` (items 1, 2, 5)
- `src/workflow/projection/timeline.rs` (items 3, 4)
- `tests/review_timeline_projection_conformance_tests.rs` (tests + golden writer)
- `tests/fixtures/slice3/valid/review-report-basic.json` (regenerated)
- `tests/fixtures/slice3/valid/timeline-basic.json` (regenerated)
- `docs/architecture/e4-x07-slice-3-review-evidence-projections.md` (spec revisions)

`vendored/soma/**`: unchanged (verified via `git diff --stat -- vendored/` — empty).

## Verification (all at new head `b64cabce156cf93e0b32214a8b4edd85f7618d0f`)

- `cargo fmt --check`: PASS
- `cargo clippy --all-targets --all-features -- -D warnings`: PASS
- `tests/review_timeline_projection_conformance_tests`: **56 passed, 1 ignored** (authoring helper `#[ignore]`d; 56 includes the new row-mapped tests)
- `tests/emitted_diagnostics_conformance`: **13 passed**
- `tests/projection_conformance_tests`: **64 passed**
- `tests/soma_capability_conformance`: **27 passed**
- Evidence (single clean passes; #214 retries documented):
  - core: `fe4a9ab58137f1048a0ab065f40e7ae0df093f8836d60c5398aa2f3c0828ffb5`
  - platform: `5d31ba2923f28abe79008e24f7f12154ede1ec7d06d866dd97659859ebe8523a`
  - smoke: `f6c0de7eebb54bde8c5294157ceb987efe5a251bdc1a879ac0b0be3d736c910b`
- `python scripts/local_ci.py verify --commit b64cabce156cf93e0b32214a8b4edd85f7618d0f --require-platform windows`: **PASS: 3 evidence files verify**

### Retries / #214 disclosure (honest, not hidden)

Severe Norton AV interference (`.git/objects` permission-denied) this session:
- core: first full attempt at this head passed (no retries needed).
- platform: passed after ~11 full-suite attempts (victims rotated across `cancellation_tests` and lib resource-enforcement).
- smoke: passed after ~18 attempts (`approval-controlled patch smoke` victim `src/calc.rs` object hash `7a/97037f...` repeatedly locked; other attempts hit `provider governance` or the same victim).
Published evidence is a single clean pass at `b64cabce156cf93e0b32214a8b4edd85f7618d0f`; no evidence carried from earlier heads; no steps skipped.

## Design revisions recorded (not hidden in code only)

- `ReviewDisclosurePolicy.authorized_boundaries` removed (no authoritative binding); `generalize_file`, `validate_boundaries`, `composite_boundaries` removed.
- `ReviewFacts.evidence_bundles` removed; evidence references come exclusively from `evidence_references`.
- `TimelineEventView.semantic_digest: Option<Hex64>`; `TimelineProjectionPayload.event_count: Option<u64>`.
- Spec §4.0/4.2/4.5/5.1/5.2/6.2/6.3/§10 updated; new rows 25–28 added.

## Boundaries honored

No #217, no #132 Slice 3 promotion, no autonomous-execution promotion, no compiled-plan work (#163), no spec redesign. PR #245 left open; no merge; no rebase; no force-push.
