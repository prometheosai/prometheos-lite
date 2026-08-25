# Change: E3/I04 — governed node conformance kit

**Issue:** #119 · **Depends on:** #116 ✅ #117 ✅ #118 ✅ (+ #171 seam incorporated).

## Scope

`tests/node_conformance_kit.rs` — a categorized conformance battery driving candidate node implementations through the real Lite machinery (NodeRunner gates, lite.policy.v1 authorization, lite.node.v1 contracts, redaction, journal durability, #171 workspace seam):

| Category | Proven by |
|---|---|
| schema/adapter violations | malformed lite.node.v1 manifests fail closed |
| authority/policy enforcement | scope-intersection rejection before effects |
| runtime/idempotency/retry | same-identity runs execute once; unbounded retry refused |
| cancellation | preflight refuses consumed capabilities (no post-cancel dispatch) |
| evidence/durability | terminal results bind journaled digests; secrets redacted before retention |
| governed-path bypass | one-shot capability re-resolution, undeclared capabilities, missing args |
| workspace violations (#171) | read-only writes denied; stale revision rejected; adapter mismatch rejected |

- Reference compliant node passes every category.
- Each defective variant fails EXACTLY its category (no generic "node invalid").
- SOMA normative fixtures remain owned by the vendored bundle suite (`soma_ast_conformance`); this kit reuses the published diagnostic CODES via the shared runner/soma modules rather than copying divergent meanings.
- Reusable by E5: candidates plug in as NodeRunner registries + manifests using only public APIs.

## Disclosed deferrals

- SOMA #83 capability-negotiation fixtures: upstream not published; kit asserts today's rule (capability ≠ authority; no silent substitution) via bypass tests; will extend when published.
- Foundry owns independent cross-runtime certification; a green kit makes NO such claim.

## Verification

cargo test --test node_conformance_kit; fmt/clippy/full lib green. No dependency changes.
