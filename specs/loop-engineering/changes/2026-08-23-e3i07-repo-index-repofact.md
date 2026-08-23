# Change Spec: E3/I07 - Stable revision-scoped repository index + RepoFact (#167)

- **Issue:** #167 (parents #104/#107; depends on #151 #152 - both MERGED)
- **Unblocks:** #153, #155, #142
- **Branch:** feat/repo-index-repofact
- **Status:** slicing; slice 1 in progress

## Objective

Turn the experimental repository-intelligence surface
(`harness/repo_intelligence.rs`, ~3.7k lines: Cargo metadata, module graph,
public API/symbol extraction) into the stable, revision-qualified local
repository index used by `prometheos work`, replacing Repo Workbench's
first-80-files heuristic. Lite stays local-first: no Mnemosyne, no vector DB,
no model calls for indexing.

## Ownership (binding)

- Lite owns scanning, parsing, revision detection, normalized fact extraction,
  local indexing, cache invalidation, local retrieval.
- SOMA++ owns the versioned portable `RepositoryFactBatch` contract family.
  No published SOMA schema exists for it yet => Lite emits an explicitly
  Lite-owned versioned envelope (`lite.repofact.v1`) that reuses canonical
  EvidenceReference field conventions; upstream publication remains future
  work and nothing here claims canonicality (same discipline as #152).
- Mnemosyne may INGEST emitted facts (#82); it never rebuilds the scanner.

## Slices

1. **IndexedRepository core**: revision/ref/dirty-state capture; per-file
   digest + language + parser version; symbol records (file, range, kind,
   signature where available, visibility) built from existing tree-sitter
   structures; exact symbol lookup returning typed hit-or-not-found;
   fail-closed staleness gate (revision or any queried-file digest mismatch =>
   rebuild required error); deterministic serialization. Unit tests incl.
   not-found typing + stale rejection.
2. **Lexical + bounded relations lookup**: case-insensitive substring/name
   search over symbols; defines/imports/calls-where-reliable relations from
   the existing module graph; symbol-linked test association heuristic
   (name/path). Tests per relation type.
3. **RetrievalPort integration**: implement `MemoryRetrievalPort`-compatible
   local port (from #152 contracts) over the index for repo-evidence queries;
   scope checks; token budgeting reuse; typed not-found as omitted-with-reason
   evidence (never invented). Contract fixtures shared shape with the future
   Mnemosyne adapter (fixture files under tests/fixtures/repofact/).
4. **RepoFactBatch emission** (`lite.repofact.v1`): batch envelope
   (repository identity, ref, revision, dirty flag, parser version, digests)
   + normalized fact records; deterministic digest; JSON emission path +
   roundtrip tests; README/product-surface wording corrections.

## Acceptance mapping (#167)

- stable work path uses index (slice 3 wiring replaces workbench source)
- exact symbol lookup fields + typed not-found (slice 1 tests)
- stale entries rejected/rebuilt (slice 1 fail-closed gate)
- dirty-worktree explicit behavior (slice 1 captures flag; policy tested)
- local-only operation, no Mnemosyne (all slices)
- shared retrieval contract fixtures (slice 3 fixtures consumed by both ports)
- fact batches round-trip + version fail-closed (slice 4)

## Rules honored

No new dependencies (tree-sitter family already present). One bounded PR per
slice through the independent-gate loop; minimality budget respected.
