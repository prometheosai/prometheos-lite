//! Stable, revision-qualified local repository index (#167 slice 1).
//!
//! OWNERSHIP: Lite owns scanning, revision detection, digests, indexing and
//! local retrieval. This module wraps the existing extraction engine
//! (`harness::repo_intelligence`) in a stable envelope that records
//! repository identity, ref, commit revision, dirty-worktree state, parser
//! version, per-file SHA-256 digests, symbols and relations - and FAILS
//! CLOSED when the repository has moved underneath it.
//!
//! Determinism: files and symbols are stored sorted; the same repository
//! content at the same revision always produces the same envelope bytes.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::harness::repo_intelligence::{
    CodeSymbol, SymbolEdge, extract_symbols_and_relationships,
};

/// Parser/extraction engine version. Bump whenever symbol extraction changes
/// so persisted envelopes can detect stale engines (fail closed).
pub const INDEX_PARSER_VERSION: &str = "1.0.0";

/// Repository identity + state captured at build time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepositoryIdentity {
    /// Canonical filesystem path of the repository root.
    pub root: String,
    /// Short branch/ref name (`--abbrev-ref HEAD`); "HEAD" when detached.
    pub ref_name: String,
    /// Full commit sha the index is qualified against.
    pub revision: String,
    /// True when `git status --porcelain` reported any entry at build time.
    pub dirty: bool,
}

/// Per-file provenance recorded by the index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileDigestEntry {
    /// SHA-256 lowercase hex over the file bytes at build time.
    pub sha256: String,
    /// Language tag from the existing detector ("rust", "ts", ...).
    pub language: String,
}

/// Typed lookup outcome: a hit carries full provenance; misses are explicit
/// evidence, never fabricated context (#167 acceptance).
#[derive(Debug, Clone, PartialEq)]
pub enum SymbolLookup<'a> {
    Hit {
        name: &'a str,
        file: &'a Path,
        line_start: usize,
        line_end: usize,
        kind: crate::harness::repo_intelligence::SymbolKind,
        signature: Option<&'a str>,
        visibility: &'a crate::harness::repo_intelligence::Visibility,
        revision: &'a str,
        file_sha256: Option<&'a str>,
    },
    NotFound {
        name: String,
    },
}

/// The stable, revision-qualified index envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IndexedRepository {
    pub schema_version: String,
    pub parser_version: String,
    pub identity: RepositoryIdentity,
    /// Relative-path -> digest/language for every indexed code file (sorted).
    pub files: BTreeMap<String, FileDigestEntry>,
    /// Symbols extracted from those files, sorted by (file, line_start, name).
    pub symbols: Vec<CodeSymbol>,
    /// Relations extracted alongside the symbols, sorted deterministically.
    pub relations: Vec<SymbolEdge>,
    /// RFC3339 build timestamp.
    pub built_at: String,
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .context("failed to run git")?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

impl IndexedRepository {
    /// Build the index over `root` using the existing ignore-aware walker and
    /// symbol extractor. Captures repository identity FIRST so the envelope
    /// is qualified even if files change mid-build (staleness gate catches it).
    pub fn build(root: &Path) -> Result<Self> {
        let root_canon = root
            .canonicalize()
            .with_context(|| format!("canonicalize {}", root.display()))?;
        let revision = git(&root_canon, &["rev-parse", "HEAD"])?;
        let ref_name = git(&root_canon, &["rev-parse", "--abbrev-ref", "HEAD"])
            .unwrap_or_else(|_| "HEAD".to_string());
        let dirty = !git(&root_canon, &["status", "--porcelain"])?.is_empty();

        // Reuse the experimental engine's ignore-aware collection.
        let mut files_abs: Vec<PathBuf> = Vec::new();
        collect_files_public(&root_canon, &mut files_abs)?;

        let mut files: BTreeMap<String, FileDigestEntry> = BTreeMap::new();
        let mut symbols: Vec<CodeSymbol> = Vec::new();
        let mut relations: Vec<SymbolEdge> = Vec::new();

        for abs in files_abs {
            let rel = abs
                .strip_prefix(&root_canon)
                .unwrap_or(&abs)
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = std::fs::read(&abs).with_context(|| format!("read {}", abs.display()))?;
            let language = detect_language_public(&abs);
            files.insert(
                rel.clone(),
                FileDigestEntry {
                    sha256: sha256_hex(&bytes),
                    language: language.clone(),
                },
            );
            if let Ok(content) = String::from_utf8(bytes)
                && let Some((syms, rels)) =
                    extract_symbols_and_relationships(&abs, &content, &language)
            {
                symbols.extend(syms);
                relations.extend(rels);
            }
        }

        // Deterministic ordering.
        symbols.sort_by(|a, b| {
            a.file
                .cmp(&b.file)
                .then(a.line_start.cmp(&b.line_start))
                .then(a.name.cmp(&b.name))
        });
        relations.sort_by(|a, b| {
            a.file
                .cmp(&b.file)
                .then(a.line.cmp(&b.line))
                .then(a.from.cmp(&b.from))
                .then(a.to.cmp(&b.to))
        });

        Ok(Self {
            schema_version: "1.0.0".to_string(),
            parser_version: INDEX_PARSER_VERSION.to_string(),
            identity: RepositoryIdentity {
                root: root_canon.display().to_string(),
                ref_name,
                revision,
                dirty,
            },
            files,
            symbols,
            relations,
            built_at: super::evaluate::now_iso(),
        })
    }

    /// Fail-closed freshness check: current HEAD must equal the captured
    /// revision and every indexed file's digest must still match. Any drift
    /// returns [`IndexStale`] - callers must rebuild, never read stale.
    pub fn verify_fresh(&self, root: &Path) -> Result<()> {
        let current = git(root, &["rev-parse", "HEAD"])?;
        if current != self.identity.revision {
            return Err(IndexStale {
                reason: format!(
                    "repository moved: indexed revision {}, current {current}",
                    self.identity.revision
                ),
            }
            .into());
        }
        for (rel, entry) in &self.files {
            let abs = root.join(rel);
            let Ok(bytes) = std::fs::read(&abs) else {
                return Err(IndexStale {
                    reason: format!("indexed file disappeared: {rel}"),
                }
                .into());
            };
            if sha256_hex(&bytes) != entry.sha256 {
                return Err(IndexStale {
                    reason: format!("indexed file changed: {rel}"),
                }
                .into());
            }
        }
        Ok(())
    }

    /// Exact symbol lookup by name. Multiple definitions (overloads across
    /// impls) all surface as hits; zero hits is a TYPED not-found.
    pub fn exact_symbol<'a>(&'a self, name: &str) -> SymbolLookup<'a> {
        let matches: Vec<&CodeSymbol> = self.symbols.iter().filter(|s| s.name == name).collect();
        if matches.is_empty() {
            return SymbolLookup::NotFound {
                name: name.to_string(),
            };
        }
        // Single canonical hit path used by tests/callers; multi-hit callers
        // can iterate `symbols` themselves. Return the first in index order.
        let s = matches[0];
        // Symbol paths are absolute (extractor convention); normalize to the
        // index's repo-relative key for digest provenance.
        let abs_norm = s.file.to_string_lossy().replace('\\', "/");
        let root_norm = self
            .identity
            .root
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_string();
        let rel_key = abs_norm
            .strip_prefix(root_norm.as_str())
            .map(|r| r.trim_start_matches('/').to_string())
            .unwrap_or_else(|| abs_norm.clone());
        let file_sha256 = self.files.get(&rel_key).map(|e| e.sha256.as_str());
        SymbolLookup::Hit {
            name: &s.name,
            file: &s.file,
            line_start: s.line_start,
            line_end: s.line_end,
            kind: s.kind,
            signature: s.signature.as_deref(),
            visibility: &s.visibility,
            revision: &self.identity.revision,
            file_sha256,
        }
    }

    /// Count of distinct symbols named `name` (diagnostic helper).
    pub fn symbol_occurrences(&self, name: &str) -> usize {
        self.symbols.iter().filter(|s| s.name == name).count()
    }

    // -----------------------------------------------------------------
    // Slice 2: lexical + bounded relations lookup
    // -----------------------------------------------------------------

    /// Case-insensitive substring search over symbol names. Deterministic
    /// (index order). Empty `needle` yields no results (avoid full-table scan).
    pub fn lexical_symbol_search<'a>(&'a self, needle: &str) -> Vec<SymbolLookup<'a>> {
        if needle.is_empty() {
            return Vec::new();
        }
        let n = needle.to_ascii_lowercase();
        self.symbols
            .iter()
            .filter(|s| s.name.to_ascii_lowercase().contains(&n))
            .map(move |s| {
                let rel_key = {
                    let p = s.file.to_string_lossy().replace('\\', "/");
                    let root_norm = self
                        .identity
                        .root
                        .replace('\\', "/")
                        .trim_end_matches('/')
                        .to_string();
                    match p.strip_prefix(root_norm.as_str()) {
                        Some(r) => r.trim_start_matches('/').to_string(),
                        None => p.clone(),
                    }
                };
                let file_sha256 = self.files.get(&rel_key).map(|e| e.sha256.as_str());
                SymbolLookup::Hit {
                    name: &s.name,
                    file: &s.file,
                    line_start: s.line_start,
                    line_end: s.line_end,
                    kind: s.kind,
                    signature: s.signature.as_deref(),
                    visibility: &s.visibility,
                    revision: &self.identity.revision,
                    file_sha256,
                }
            })
            .collect()
    }

    /// Outgoing edges whose `from` equals this value. Extractor convention:
    /// for Import/Call kinds `from` is the SOURCE FILE NAME, not a symbol.
    pub fn relations_from<'a>(
        &'a self,
        from: &str,
        kinds: &'a [crate::harness::repo_intelligence::EdgeKind],
    ) -> Vec<&'a SymbolEdge> {
        self.relations
            .iter()
            .filter(|e| e.from == from && kinds.contains(&e.kind))
            .collect()
    }

    /// Incoming edges whose `to` equals this value (for Import/Call kinds
    /// `to` is the referenced target name).
    pub fn relations_to<'a>(
        &'a self,
        to: &str,
        kinds: &'a [crate::harness::repo_intelligence::EdgeKind],
    ) -> Vec<&'a SymbolEdge> {
        self.relations
            .iter()
            .filter(|e| e.to == to && kinds.contains(&e.kind))
            .collect()
    }

    /// Heuristic symbol-linked test association. A symbol links to
    /// `symbol_name` when ANY of:
    /// - its own name contains "test" and the target name, or
    /// - it lives in a test/spec file whose path references the target name,
    /// - it is a Test-kind symbol whose path references the FILE STEM of a
    ///   definition site of the target (module-under-test convention,
    ///   e.g. `util.rs` <-> `tests/util_test.rs`).
    pub fn linked_tests<'a>(&'a self, symbol_name: &str) -> Vec<&'a CodeSymbol> {
        let lower = symbol_name.to_ascii_lowercase();
        // File stems of the target's definition sites.
        let mut stems: Vec<String> = Vec::new();
        for s in self.symbols.iter().filter(|s| s.name == symbol_name) {
            if let Some(stem) = s.file.file_stem() {
                let st = stem.to_string_lossy().to_ascii_lowercase().to_string();
                if !st.is_empty() && !stems.contains(&st) {
                    stems.push(st);
                }
            }
        }
        self.symbols
            .iter()
            .filter(|s| {
                let n = s.name.to_ascii_lowercase();
                let path = s.file.to_string_lossy().to_ascii_lowercase();
                let name_links = n.contains("test") && n.contains(&lower);
                let path_links =
                    (path.contains("test") || path.contains("spec")) && path.contains(&lower);
                let module_links = stems.iter().any(|st| {
                    path.contains(&format!("{st}_test"))
                        || path.contains(&format!("{st}_spec"))
                        || path.ends_with(&format!("test_{st}.rs"))
                });
                name_links || path_links || module_links
            })
            .collect()
    }
}

/// Fail-closed staleness error (revision or file digest drift).
#[derive(Debug)]
pub struct IndexStale {
    pub reason: String,
}
impl std::fmt::Display for IndexStale {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "repository index is stale: {}", self.reason)
    }
}
impl std::error::Error for IndexStale {}

// Bridges to the (now-public) experimental engine helpers.
fn collect_files_public(p: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    crate::harness::repo_intelligence::collect_files(p, out)
}
fn detect_language_public(p: &Path) -> String {
    crate::harness::repo_intelligence::detect_language(p)
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Slice 4: versioned normalized fact-batch emission (lite.repofact.v1)
// ---------------------------------------------------------------------------

/// One normalized repository fact. Lite-owned `lite.repofact.v1`
/// (no published SOMA++ family exists; upstream publication is future work).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepositoryFactV1 {
    /// Stable id: `<revision>:<rel_path>#<name>` (matches retrieval ids).
    pub fact_id: String,
    pub kind: String,
    pub name: String,
    pub rel_path: String,
    pub line_start: u32,
    pub line_end: u32,
    pub visibility: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    pub file_sha256: String,
}

/// Versioned batch envelope binding facts to a repository revision with a
/// canonical digest. Fail-closed on unknown major versions when parsed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepoFactBatchV1 {
    pub schema_version: String,
    pub parser_version: String,
    pub identity: RepositoryIdentity,
    pub facts: Vec<RepositoryFactV1>,
    /// Canonical digest over every other field (recomputable).
    pub batch_digest: String,
}

impl RepoFactBatchV1 {
    /// Build from an index. Facts mirror indexed symbols with full
    /// provenance (file digest + revision via the envelope).
    pub fn from_index(index: &IndexedRepository) -> Result<Self> {
        let root_norm = index
            .identity
            .root
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_string();
        let mut facts = Vec::new();
        for s in &index.symbols {
            let abs_norm = s.file.to_string_lossy().replace('\\', "/");
            let rel = abs_norm
                .strip_prefix(root_norm.as_str())
                .map(|r| r.trim_start_matches('/').to_string())
                .unwrap_or_else(|| abs_norm.clone());
            let file_sha256 = index
                .files
                .get(&rel)
                .map(|e| e.sha256.clone())
                .unwrap_or_else(|| "0".repeat(64));
            facts.push(RepositoryFactV1 {
                fact_id: format!("{}:{}#{}", index.identity.revision, rel, s.name),
                kind: format!("{:?}", s.kind),
                name: s.name.clone(),
                rel_path: rel,
                line_start: s.line_start as u32,
                line_end: s.line_end as u32,
                visibility: format!("{:?}", s.visibility),
                signature: s.signature.clone(),
                file_sha256,
            });
        }
        let pre = serde_json::json!({
            "facts": facts,
            "identity": index.identity,
            "parserVersion": index.parser_version,
            "schemaVersion": "1.0.0",
        });
        Ok(Self {
            schema_version: "1.0.0".to_string(),
            parser_version: index.parser_version.clone(),
            identity: index.identity.clone(),
            batch_digest: crate::workflow::memory_contracts::canonical_digest(&pre)?,
            facts,
        })
    }

    /// Fail-closed parse: rejects major versions above 1.
    pub fn parse_json(json: &str) -> Result<Self> {
        let b: Self = serde_json::from_str(json).context("failed to parse lite.repofact batch")?;
        let ceiling = crate::workflow::schema::SchemaVersion::new(1, u32::MAX, u32::MAX);
        let sv = crate::workflow::schema::SchemaVersion::parse(&b.schema_version)
            .context("invalid lite.repofact schema_version")?;
        if sv > ceiling {
            bail!(
                "unsupported lite.repofact version {} (fail closed)",
                b.schema_version
            );
        }
        Ok(b)
    }
}

// Slice 3: local RetrievalPort integration (#153 contract family)
// ---------------------------------------------------------------------------

/// Read-only [`crate::workflow::memory_contracts::MemoryRetrievalPort`]
/// backed by an [`IndexedRepository`]. Query text is matched deterministically:
/// exact symbol name first, then case-insensitive lexical search.
///
/// Staleness is fail-closed: when a `current_revision` is supplied and it
/// differs from the indexed revision, retrieval errors with
/// [`IndexStale`] in the chain (never silently serves stale context).
///
/// The repository index is a read-only evidence surface: `write` is always a
/// typed unavailable error.
pub struct RepoEvidencePort {
    pub index: IndexedRepository,
    pub current_revision: Option<String>,
}

impl crate::workflow::memory_contracts::MemoryRetrievalPort for RepoEvidencePort {
    fn name(&self) -> &'static str {
        "repo-evidence"
    }
    fn backend(&self) -> crate::workflow::memory_contracts::BackendKind {
        crate::workflow::memory_contracts::BackendKind::Local
    }

    fn retrieve(
        &self,
        query: &crate::workflow::memory_contracts::MemoryQuery,
    ) -> Result<Vec<crate::workflow::memory_contracts::RawCandidate>> {
        use crate::workflow::memory_contracts::{EvidenceReferenceV1, RawCandidate};
        if let Some(cur) = &self.current_revision
            && cur.as_str() != self.index.identity.revision
        {
            return Err(IndexStale {
                reason: format!(
                    "query revision {cur} != indexed revision {}",
                    self.index.identity.revision
                ),
            }
            .into());
        }
        let term = query.text.trim();
        if term.is_empty() {
            return Ok(Vec::new());
        }
        // Deterministic: exact hit(s) preferred; else lexical matches.
        let mut out: Vec<RawCandidate> = Vec::new();
        let push = |s: &CodeSymbol, out: &mut Vec<RawCandidate>| {
            let abs_norm = s.file.to_string_lossy().replace('\\', "/");
            let root_norm = self
                .index
                .identity
                .root
                .replace('\\', "/")
                .trim_end_matches('/')
                .to_string();
            let rel = abs_norm
                .strip_prefix(root_norm.as_str())
                .map(|r| r.trim_start_matches('/').to_string())
                .unwrap_or_else(|| abs_norm.clone());
            let artifact_digest = self
                .index
                .files
                .get(&rel)
                .map(|e| e.sha256.clone())
                .unwrap_or_else(|| "0".repeat(64));
            let memory_id = format!("repo:{rel}#{}", s.name);
            let event_digest = {
                let mut h = Sha256::new();
                h.update(self.index.identity.revision.as_bytes());
                h.update(b"\n");
                h.update(rel.as_bytes());
                format!("{:x}", h.finalize())
            };
            out.push(RawCandidate {
                memory_id,
                kind: crate::workflow::memory_contracts::MemoryKind::Fact,
                source_revision: self.index.identity.revision.clone(),
                evidence: EvidenceReferenceV1 {
                    id: String::new(), // filled below (id == memory id)
                    event_digest,
                    artifact_digest,
                    artifact_kind: "repository-symbol".into(),
                    produced_by: self.index.parser_version.clone(),
                    produced_at: Some(self.index.built_at.clone()),
                },
                content: match &s.signature {
                    Some(sig) => format!("{sig}\n// {}:{}..{}", rel, s.line_start, s.line_end),
                    None => format!("{}\n// {}:{}..{}", s.name, rel, s.line_start, s.line_end),
                },
                relevance: 1.0,
            });
            let last = out.last_mut().expect("just pushed");
            last.evidence.id = last.memory_id.clone();
        };
        match self.index.exact_symbol(term) {
            SymbolLookup::Hit { .. } => {
                if let Some(s) = self.index.symbols.iter().find(|s| s.name == term) {
                    push(s, &mut out);
                }
            }
            SymbolLookup::NotFound { .. } => {
                for s in self.index.symbols.iter().filter(|s| {
                    s.name
                        .to_ascii_lowercase()
                        .contains(&term.to_ascii_lowercase())
                }) {
                    push(s, &mut out);
                }
            }
        }
        Ok(out)
    }

    fn write(&self, _write: &crate::workflow::memory_contracts::MemoryWrite) -> Result<String> {
        Err(anyhow::Error::new(
            crate::workflow::memory_contracts::MemoryBackendUnavailable {
                backend: crate::workflow::memory_contracts::BackendKind::Local,
                message: "repository evidence index is read-only".into(),
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_repo(tag: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir_in(std::env::temp_dir()).expect("tempdir");
        let repo = dir.path().join(tag);
        std::fs::create_dir_all(repo.join("src")).unwrap();
        let g = |a: &[&str]| {
            let o = std::process::Command::new("git")
                .args(a)
                .current_dir(&repo)
                .output()
                .unwrap();
            assert!(
                o.status.success(),
                "git {a:?}: {}",
                String::from_utf8_lossy(&o.stderr)
            );
        };
        g(&["init", "-q"]);
        g(&["config", "user.email", "t@t"]);
        g(&["config", "user.name", "T"]);
        std::fs::write(
            repo.join("src/lib.rs"),
            "pub mod util;\nuse crate::util::helper;\npub fn top() -> u32 { helper() }\n",
        )
        .unwrap();
        std::fs::write(repo.join("src/util.rs"), "pub fn helper() -> u32 { 1 }\n").unwrap();
        g(&["add", "."]);
        g(&["commit", "-q", "-m", "base"]);
        dir
    }

    #[test]
    fn build_captures_identity_files_and_symbols() {
        let dir = init_repo("idx-build");
        let repo = dir.path().join("idx-build");
        let idx = IndexedRepository::build(&repo).unwrap();
        assert!(!idx.identity.revision.is_empty());
        assert!(!idx.identity.dirty);
        assert_eq!(idx.parser_version, INDEX_PARSER_VERSION);
        assert!(idx.files.contains_key("src/lib.rs"));
        assert!(idx.files.contains_key("src/util.rs"));
        assert!(
            idx.symbols.iter().any(|s| s.name == "top"),
            "top() must be indexed"
        );
        assert!(
            idx.symbols.iter().any(|s| s.name == "helper"),
            "helper() must be indexed"
        );
    }

    #[test]
    fn exact_lookup_hit_carries_provenance() {
        let dir = init_repo("idx-hit");
        let repo = dir.path().join("idx-hit");
        let idx = IndexedRepository::build(&repo).unwrap();
        match idx.exact_symbol("helper") {
            SymbolLookup::Hit {
                revision,
                file_sha256,
                ..
            } => {
                assert_eq!(revision, &idx.identity.revision);
                assert!(file_sha256.is_some(), "hit must carry file digest");
            }
            other => panic!("expected hit, got {other:?}"),
        }
    }

    #[test]
    fn missing_symbol_is_typed_not_found() {
        let dir = init_repo("idx-miss");
        let repo = dir.path().join("idx-miss");
        let idx = IndexedRepository::build(&repo).unwrap();
        match idx.exact_symbol("definitely_not_there") {
            SymbolLookup::NotFound { name } => assert_eq!(name, "definitely_not_there"),
            other => panic!("expected not-found, got {other:?}"),
        }
    }

    #[test]
    fn content_change_makes_index_stale_fail_closed() {
        let dir = init_repo("idx-stale");
        let repo = dir.path().join("idx-stale");
        let idx = IndexedRepository::build(&repo).unwrap();
        idx.verify_fresh(&repo).expect("fresh right after build");
        std::fs::write(repo.join("src/util.rs"), "pub fn helper() -> u32 { 2 }\n").unwrap();
        let err = idx.verify_fresh(&repo).expect_err("must go stale");
        assert!(err.to_string().contains("changed"), "{err}");
    }

    #[test]
    fn new_commit_changes_revision_staleness() {
        let dir = init_repo("idx-rev");
        let repo = dir.path().join("idx-rev");
        let idx = IndexedRepository::build(&repo).unwrap();
        std::fs::write(repo.join("CHANGELOG.md"), "x\n").unwrap();
        let g = |a: &[&str]| {
            std::process::Command::new("git")
                .args(a)
                .current_dir(&repo)
                .output()
                .unwrap()
        };
        g(&["add", "."]);
        g(&["commit", "-q", "-m", "second"]);
        let err = idx.verify_fresh(&repo).expect_err("revision moved");
        assert!(err.to_string().contains("moved"), "{err}");
    }

    #[test]
    fn dirty_worktree_flag_is_explicit() {
        let dir = init_repo("idx-dirty");
        let repo = dir.path().join("idx-dirty");
        // Uncommitted modification BEFORE build => dirty=true, still indexes.
        std::fs::write(
            repo.join("src/util.rs"),
            "// touched\npub fn helper() -> u32 { 3 }\n",
        )
        .unwrap();
        let idx = IndexedRepository::build(&repo).unwrap();
        assert!(idx.identity.dirty, "uncommitted change must set dirty flag");
    }
    // ---- slice 2: lexical + relations ----

    #[test]
    fn lexical_search_is_case_insensitive_and_deterministic() {
        let dir = init_repo("idx-lex");
        let repo = dir.path().join("idx-lex");
        let idx = IndexedRepository::build(&repo).unwrap();
        let hits = idx.lexical_symbol_search("HELP");
        assert!(
            hits.iter()
                .any(|h| matches!(h, SymbolLookup::Hit { name, .. } if *name == "helper"))
        );
        assert!(
            idx.lexical_symbol_search("").is_empty(),
            "empty needle = no scan"
        );
    }

    #[test]
    fn bounded_relations_query_is_typed_and_directional() {
        use crate::harness::repo_intelligence::EdgeKind;
        let dir = init_repo("idx-rel");
        let repo = dir.path().join("idx-rel");
        let idx = IndexedRepository::build(&repo).unwrap();
        let kinds = [EdgeKind::Import, EdgeKind::Call, EdgeKind::Reference];
        // Engine convention: Import/Call edges carry the SOURCE FILE NAME in
        // `from` and the referenced target name in `to`.
        let out_to = idx.relations_to("helper", &kinds);
        assert!(
            !out_to.is_empty(),
            "fixture must produce import/call edges into helper"
        );
        let dbg: Vec<String> = out_to
            .iter()
            .map(|e| format!("{}->{} {:?}", e.from, e.to, e.kind))
            .collect();
        assert!(!dbg.is_empty(), "fixture must produce edges");
        for e in &out_to {
            assert!(kinds.contains(&e.kind));
        }
        let helper_edge = out_to
            .iter()
            .find(|e| e.to.trim_end_matches(';') == "helper")
            .expect("no edge targeting helper");
        let src_file = helper_edge.from.as_str();
        let out_from = idx.relations_from(src_file, &[EdgeKind::Import]);
        assert!(
            !out_from.is_empty(),
            "directional from-query must find edges; src_file={src_file:?} all={dbg:?}"
        );
    }

    #[test]
    fn linked_tests_uses_name_and_path_heuristic() {
        let dir = init_repo("idx-tests");
        let repo = dir.path().join("idx-tests");
        std::fs::create_dir_all(repo.join("tests")).unwrap();
        std::fs::write(
            repo.join("tests").join("util_test.rs"),
            "#[test]\nfn helper_works() { assert_eq!(1,1); }\n",
        )
        .unwrap();
        let g = |a: &[&str]| {
            std::process::Command::new("git")
                .args(a)
                .current_dir(&repo)
                .output()
                .unwrap()
        };
        g(&["add", "."]);
        g(&["commit", "-q", "-m", "tests"]);
        let idx = IndexedRepository::build(&repo).unwrap();
        let linked = idx.linked_tests("helper");
        assert!(
            linked.iter().any(|s| s.name.contains("helper")),
            "test symbol linking failed"
        );
    }
    // ---- slice 4: repofact batch ----

    #[test]
    fn repofact_batch_is_deterministic_and_roundtrips() {
        let dir = init_repo("rf1");
        let repo = dir.path().join("rf1");
        let idx = IndexedRepository::build(&repo).unwrap();
        let b1 = RepoFactBatchV1::from_index(&idx).unwrap();
        let b2 = RepoFactBatchV1::from_index(&idx).unwrap();
        assert_eq!(b1.batch_digest, b2.batch_digest);
        assert_eq!(b1.batch_digest.len(), 64);
        assert!(b1.facts.iter().any(|f| f.name == "top"));
        assert!(b1.facts.iter().all(|f| f.file_sha256.len() == 64));
        let json = serde_json::to_string(&b1).unwrap();
        let parsed = RepoFactBatchV1::parse_json(&json).unwrap();
        assert_eq!(parsed, b1);
    }

    #[test]
    fn repofact_batch_rejects_future_major() {
        let dir = init_repo("rf2");
        let repo = dir.path().join("rf2");
        let idx = IndexedRepository::build(&repo).unwrap();
        let mut b = RepoFactBatchV1::from_index(&idx).unwrap();
        b.schema_version = "7.0.0".into();
        let err = RepoFactBatchV1::parse_json(&serde_json::to_string(&b).unwrap()).unwrap_err();
        assert!(err.to_string().contains("fail closed"), "{err}");
    }
}
