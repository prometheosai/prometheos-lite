# Change Spec: E3/I07 - Stable revision-scoped repository index + RepoFact (#167)

- **Issue:** #167 (parents #104/#107; depends on #151 #152 - both MERGED)
- **Unblocks:** #153, #155, #142
- **Branch:** feat/repo-index-repofact
- **Status:** slices 1-2 MERGED via #177; slices 3-4 under review in #178

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

## Slices 3-4 completion notes (PR #178)

- RepoEvidencePort: local MemoryRetrievalPort over the index; exact-then-
  lexical policy; SOMA EvidenceReference provenance per hit (artifact digest
  = file sha256, event digest = sha256(revision+rel)); fail-closed staleness
  via typed IndexStale through the error chain.
- lite.repofact.v1: RepositoryFactV1/RepoFactBatchV1 with canonical batch
  digest excluding itself; parse_json rejects major>1. Lite-owned family;
  upstream SOMA publication remains future work.
- Workbench source swap: scan_repo sources candidate files from the index
  (HEAD + per-file digests) for git contexts - the first-80-files cap is
  gone. Bounded legacy walk retained ONLY for non-git directories (the
  index itself stays fail-closed for evaluation runs). README updated.
- Known follow-ups: from_index currently falls back to a zero digest when a
  rel-path mapping misses (should hard-error); reason-taxonomy refinement
  for stale-vs-conflict from #152 applies here too once shared.
- Hygiene: repo_workbench.rs encoding fully repaired this PR (BOM + mojibake
  removed); recommend adding a byte-hygiene guard to reviewonly CI.