# CHANGE-0001 (proposal) — Promote SPEC-0001 (Unified Authority Resolution) and Project-Trust ADR as normative planning records

**Kind:** documentation-only promotion candidate (gate G-A of ROADMAP-0001 v0.2).
**Authorization status:** NOT authorized — requires explicit operator approval before
the PR is opened. No implementation change rides on this PR.

## Scope of change (2 files + this record)

1. `specs/SPEC-0001-unified-authority-resolution-v0.3.md` — NEW: the control-gate-repaired,
   independently reviewed, and independently amendment-verified normative specification
   for unified authority resolution and the project-trust boundary (canonical decision
   contract, vocabulary adapters, execution-boundary invariant, TOCTOU/revocation rules,
   migration plan without broad compatibility grants, evidence-ledger requirements,
   17-target acceptance matrix + accepted 18th target `durable_trust_single_source_of_truth`
   recorded separately, mechanical guards MG1–MG3, approval-gate ladder).
2. `specs/ADR-0002-project-trust-persistence-v0.1.1.md` — NEW: the project-trust
   persistence ADR (identity tuple incl. `validity`, non-transfer semantics, attribution
   + revocation-without-deletion, storage requirements, open points).

## What changes and what cannot change

- Normative text of both promotion candidates is **byte-identical** to the reviewed +
  independently verified successors (SPEC-0001 v0.3, ADR v0.1.1) except exactly the two
  declared private-track directives, which do not belong in the product repository:
  (a) the SPEC's two-line private-track/lineage note removed; (b) the ADR **Scope:**
  line reworded from "private research track…" to "repository normative record" —
  nothing else. The review-ritual step re-verifies this derivation before opening the
  PR (diff original↔candidate, scan for foreign references, re-hash).
- **Not included:** any behavioral code change; any roadmap/workstream file (optional,
  operator decision); any dependency change; any frontend/API change.
- Preservation laws carried verbatim by the spec text itself: `GovernancePermit`
  exclusivity (never widened), human-gate law, no new authority vocabulary, golden-path
  compatibility, "no implementation authorized" status (§12 hold persists until the
  implementation gates).

## Verification

- Docs-only: no executable code changed (`cargo fmt --check` / `cargo check` /
  `cargo test` / `cargo clippy` results unaffected — run once on the branch for
  record-keeping, no suite additions); no frontend files touched (frontend checks not
  triggered).
- Identity: SHA-256 recorded for both promotion candidates and this record; comparison
  against the reviewed originals is the independent reviewer's first step.

## Provenance + review chain (chain of records, without cross-references to private-track naming)

Accepted planning baseline: ROADMAP-0001 v0.2 (approved planning-only). Review chain:
control-gate repair (v0.1→v0.2) → independent seven-finding review (all SATISFIED) →
four amendment verifications (EXACTLY-THE-FOUR-AMENDMENTS) → trust-reconciliation
design note → 18-target roadmap (approved planning baseline). All correspondence
and digests recorded in the open correspondence issue.

## Explicit non-goals

No implementation authorization (spec §12 remains in force); no dependency changes;
no new vocabulary types in this PR; no edits to existing issues/review states; no merge
without operator approval under the Loop Engineering Protocol.
