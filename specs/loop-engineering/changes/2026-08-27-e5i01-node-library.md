---
title: "E5/I01 — Governed intake, repository discovery, and planning nodes"
issue: 125
date: 2026-08-27
owner: loop-engineering (Lite-owned)
depends_on: [119, 123, 124]
state: implemented
verification:
  fmt: "cargo fmt --check — clean"
  clippy: "cargo clippy --all-targets --all-features -- -D warnings — exit 0"
  lib_tests: "cargo test --lib — 919 passed (916 prior + 3 new node_library unit tests)"
  conformance: "cargo test --test node_library_conformance — 7 passed"
---

# E5/I01 — Governed intake, repository discovery, and planning nodes

## Summary

Implements the first three nodes of the E5 node library as **Lite-owned
`lite.node` capability** handlers driven unchanged by the generic nine-gate
`NodeRunner` (lite.node.v1 contracts, lite.policy.v1 authorization, redaction,
journal durability — the same machinery proven by issue #119's node
conformance kit). No new graph contracts, no protocol changes, no dependency
changes.

- `intake` — validates a user objective, emits a typed `IntakeTaskManifestV1`,
  and **fails closed** on ambiguous or out-of-repository (unauthorized) scope.
- `repo-discovery` — builds a revision-qualified index via the existing
  `IndexedRepository` engine, emitting `DiscoveryResultV1` (files, languages,
  tests, constraints) plus the canonical `lite.repofact` batch digest. Read-only.
- `planning` — emits a typed, scoped `ScopedPlanV1` **linked to discovery
  evidence** (revision + fact-batch digest) and referencing discovered files.

Every node is **read-only** with respect to the target repository: manifests
declare `writableScopes: []` and `readableScopes: ["repo://fixture"]`.

## Acceptance — proven

1. Intake rejects ambiguous / unauthorized scope safely (handler returns `Err`
   → runner propagates; no journal entry, no repo mutation).
   (`intake_rejects_ambiguous_objective_safely`,
   `intake_rejects_unauthorized_scope_safely`)
2. Discovery records files / languages / tests / constraints as evidence and
   emits a `node-output` evidence ref.
   (`discovery_records_files_languages_tests_constraints`)
3. Planning output is typed and linked to discovery evidence (cites discovery
   revision + fact-batch digest; every step references the discovery digest).
   (`planning_is_typed_and_linked_to_discovery`)
4. Nodes pass the conformance categories: schema/arg validation, policy
   (write outside granted writable scope denied), governed-path bypass
   (undeclared capability cannot run), evidence durability (journal + redaction).
   (`policy_denies_write_outside_granted_writable_scope`,
   `governed_path_bypass_is_blocked_for_undeclared_capability`, plus positive
   runs asserting `evidence_refs` populated)

## Files

- `src/workflow/node_library.rs` (new) — three `Capability` handlers, typed
  outputs, `intake_discovery_planning_registry()`, `node_manifest()` helper,
  lib unit tests.
- `src/workflow/mod.rs` — `pub mod node_library;`
- `tests/node_library_conformance.rs` (new) — drives the three nodes through the
  nine-gate `NodeRunner`; reuses only public `NodeRunner` / `CapabilityRegistry`
  / `NodeManifestV1` / `IndexedRepository` APIs (per the #119 kit's stated
  "reusable by E5" contract).
- (this file) — change log.

## Reused building blocks (no reimplementation)

- `crate::workflow::repo_index::{IndexedRepository, RepoFactBatchV1}` for
  discovery and the digest-bound fact batch.
- `crate::workflow::node_runner::{NodeRunner, Capability, CapabilityRegistry}`
  for the governed execution harness.
- `crate::workflow::node_contracts::NodeManifestV1` for typed manifests.
- `crate::workflow::memory_contracts::canonical_digest` for stable task/plan ids.

## Open guards / notes

- Intake scope authorization is conservative: it rejects objectives that
  reference an absolute path, parent traversal (`..`), UNC, or a Windows drive
  letter. Broader "authorized repository scope" enforcement is delegated to the
  runner's lite.policy.v1 gate (proven by the policy-denial test).
- The planning node emits a typed structure; the *execution* of plan steps is
  out of scope for I01 (later E5 issues own the executor nodes).
