# Cross-implementation plan goldens — provenance

Oracle: **soma-core `022142b`** ("R6: atomic, collision-free,
integrity-verified run store"), the parent of `cb2b403` that introduced the
v1.3 plan shape. Its `spec/soma/v1.1/schemas/ExecutionPlan.schema.json` was
verified **byte-identical** to the schema vendored at
`vendored/soma/v1.1/schemas/ExecutionPlan.schema.json`, so its compiler is a
schema-matched oracle for the v1.1 plan shape Lite seals (review correction 1).

- Worktree: `E:\Projects\soma-core-golden` (detached at `022142b`)
- Toolchain: `rustc 1.98.0 (88d9e12ae 2026-08-18)`, `cargo 1.98.0`
- Build (succeeded fully offline, no network):
  `cargo build -p soma-cli --offline`
- Generation (2026-09-27), one command per case:

  ```
  <worktree>\target\debug\soma.exe compile <workflow.json> --out <golden>.plan.canonical.json
  ```

  The command prints `OK plan digest <sha256> -> <path>`; that value is the
  SHA-256 over the plan's **canonical bytes including the embedded seal**
  (`soma_compiler::plan_digest`), not the seal itself. Each
  `<golden>.plan.sha256` records the golden's embedded
  `canonicalization.sha256` — the seal rule Lite compares against (seal
  input removes only `canonicalization.sha256`).

| case | workflow input | seal (`canonicalization.sha256`, in `.plan.sha256`) | CLI whole-plan digest |
|---|---|---|---|
| `wf-valid-base` | `vendored/soma/v1.1/fixtures/valid/wf-valid-base.json` (published spec fixture) | `e48e2b0c378c0d0f1fa870abb9f97346c40641ea0837ca9c0854024f64eedb3f` | `bb9e5512150199fb425f236dcac5b9ac212379ec40ff5d5264a004ab0c9824d3` |
| `wf-valid-composite` | `vendored/soma/v1.1/fixtures/valid/wf-valid-composite.json` (published spec fixture) | `ba7a73e69751703ad1b04a859181ee3ddba75fede9dae69d12ddc5e6bb647c57` | `4aaa795f1340666dc3f14a38465472ce34b9b67d0a9084d3d4cfad7dc65b0ad4` |
| `wf-valid-gov` | `vendored/soma/v1.1/fixtures/valid/wf-valid-gov.json` (published spec fixture) | `e775bb5b5b2fb7bfe41be8f7a5f6a5fb04c0577465118bc79b5e6656f660e38f` | `91dec53860442ca7eeea8d0ab31e430fd21d0b722b0166100e4e6703e7f665cb` |
| `lite-reorder` | `tests/fixtures/soma-golden/lite-reorder.json` (Lite-authored input with a downstream unit declared before its producer; accepted by the oracle unchanged) | `810f35b972f79ddf9b28a9da1614c45ab58c507197e4a118ad565f51fcb2d2ea` | `f8ca085acc32f1481485e3809195a0589f0f364c1b98eaf4a41ac79148fbbfb0` |

Honest notes:

- The oracle accepts **lenient** workflow input (`from_json_str_lenient`);
  the goldens are its compiled, canonical plan bytes — not its inputs.
- `lite-reorder.json` is Lite-authored (not a published spec fixture); it
  exists to pin topological reordering (`s{pos:04}` keyed by position, not
  declaration order). The oracle compiled it with exit 0, unchanged.
- No fallback was needed: the oracle built offline and accepted all four
  inputs, so these are genuine oracle outputs, never Lite-fabricated
  "oracle" bytes.
- `tests/soma_golden_crossimpl.rs` proves (a) Lite plan bytes are
  byte-identical to these files, (b) Lite's seal digest equals the recorded
  oracle digest, (c) the golden bytes re-canonicalize to themselves, and
  (d) each golden's embedded `canonicalization.sha256` equals its record.
