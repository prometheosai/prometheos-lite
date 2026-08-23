//! Lite-owned memory/retrieval/context contracts (`lite.memory.v1`).
//!
//! OWNERSHIP (binding, see #152): there is no published SOMA++ family for
//! memory/retrieval/context. Everything here is explicitly **Lite-owned**,
//! versioned `1.0.0`, and MUST NOT be presented as canonical SOMA++. Where a
//! published SOMA++ v1 shape exists we embed it verbatim and mark it:
//! [`EvidenceReferenceV1`] mirrors `spec/soma/v1/schemas/
//! EvidenceReference.schema.json` byte-for-byte in field set and JSON naming.
//! Authorization/budget fields mirror SOMA `AuthorityProfile`
//! (`readableScopes` / `writableScopes` / `budgets.token`) by name.
//!
//! Versioning is fail-closed: any document whose `schema_version` major is
//! greater than [`MEMORY_CONTRACT_MAJOR`] is rejected, never upgraded.
//!
//! Digests use SOMA-style canonicalization: recursively key-sorted compact
//! JSON, SHA-256, lowercase 64-hex.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::workflow::portable_state::PortableWorkState;

/// Major version of the Lite memory contract family implemented here.
pub const MEMORY_CONTRACT_MAJOR: u64 = 1;
/// Full version string stamped into new documents.
pub const MEMORY_CONTRACT_VERSION: &str = "1.0.0";

/// Provenance reference. Field-for-field compatible with the published
/// SOMA++ v1 `EvidenceReference` schema (camelCase wire names, SHA-256
/// lowercase hex digests). Marked canonical: identifiers/versions/digests
/// are preserved as-is across mappings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceReferenceV1 {
    pub id: String,
    /// SHA-256 (64 hex) of the originating event.
    pub event_digest: String,
    /// SHA-256 (64 hex) of the referenced artifact.
    pub artifact_digest: String,
    pub artifact_kind: String,
    pub produced_by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub produced_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Fact,
    Preference,
    Episode,
    Decision,
}

fn ensure_supported_contract_version(v: &str) -> Result<()> {
    let sv = crate::workflow::schema::SchemaVersion::parse(v)
        .with_context(|| format!("invalid lite.memory schema_version {v:?}"))?;
    let ceiling = crate::workflow::schema::SchemaVersion::new(
        MEMORY_CONTRACT_MAJOR as u32,
        u32::MAX,
        u32::MAX,
    );
    if sv > ceiling {
        bail!(
            "unsupported lite.memory contract version {v}: major above {} (fail closed)",
            MEMORY_CONTRACT_MAJOR
        );
    }
    Ok(())
}

/// Explicit retrieval request. Authorization mirrors SOMA
/// `AuthorityProfile.readableScopes`; `token_budget` mirrors
/// `AuthorityProfile.budgets.token`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryQuery {
    pub schema_version: String,
    pub query_id: String,
    pub readable_scopes: Vec<String>,
    pub text: String,
    #[serde(default)]
    pub kinds: Vec<MemoryKind>,
    #[serde(default)]
    pub token_budget: Option<u64>,
}

impl MemoryQuery {
    pub fn parse_json(json: &str) -> Result<Self> {
        let q: Self =
            serde_json::from_str(json).context("failed to parse lite.memory MemoryQuery")?;
        ensure_supported_contract_version(&q.schema_version)?;
        if q.query_id.is_empty() {
            bail!("query_id must not be empty");
        }
        Ok(q)
    }
}

/// Explicit write request. `writable_scopes` mirrors SOMA
/// `AuthorityProfile.writableScopes`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryWrite {
    pub schema_version: String,
    pub write_id: String,
    pub writable_scopes: Vec<String>,
    pub kind: MemoryKind,
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl MemoryWrite {
    pub fn parse_json(json: &str) -> Result<Self> {
        let w: Self =
            serde_json::from_str(json).context("failed to parse lite.memory MemoryWrite")?;
        ensure_supported_contract_version(&w.schema_version)?;
        if w.write_id.is_empty() || w.content.is_empty() {
            bail!("write_id and content must not be empty");
        }
        Ok(w)
    }
}

/// One retrieved candidate with mandatory provenance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetrievalCandidate {
    pub memory_id: String,
    pub kind: MemoryKind,
    /// Repository/source revision the memory was produced against.
    pub source_revision: String,
    /// Canonical SOMA++ v1 EvidenceReference field-set.
    pub evidence: EvidenceReferenceV1,
    pub content: String,
    /// Relevance score in `[0,1]`; backend-defined scale normalized by the port.
    pub relevance: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OmittedEntry {
    pub memory_id: String,
    pub reason: String,
}

/// Backend/port descriptor kept explicitly Lite-owned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    Local,
    Mnemosyne,
    CloudAllowed,
}

/// Operation policy attached to results and bundles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationPolicy {
    pub backend: BackendKind,
    /// Mirrors SOMA AuthorityProfile.mutation for memory operations.
    pub mutation: String,
    pub executed_at: String,
}

/// Provenance-rich retrieval result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetrievalResult {
    pub schema_version: String,
    pub query_id: String,
    pub candidates: Vec<RetrievalCandidate>,
    /// Candidates considered but excluded, always with a reason.
    #[serde(default)]
    pub omitted: Vec<OmittedEntry>,
    pub token_estimate: u64,
    pub policy: OperationPolicy,
}

impl RetrievalResult {
    pub fn parse_json(json: &str) -> Result<Self> {
        let r: Self =
            serde_json::from_str(json).context("failed to parse lite.memory RetrievalResult")?;
        ensure_supported_contract_version(&r.schema_version)?;
        Ok(r)
    }
}

/// Final ordered context delivery.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContextBundle {
    pub schema_version: String,
    pub bundle_id: String,
    pub query_id: String,
    pub blocks: Vec<ContextBlock>,
    #[serde(default)]
    pub omitted: Vec<OmittedEntry>,
    pub token_estimate: u64,
    /// SHA-256 over the canonical form of this bundle (sorted-key compact).
    pub digest: String,
    pub policy: OperationPolicy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContextBlock {
    pub memory_id: String,
    pub content: String,
    pub tokens: u64,
    pub selected_because: String,
}

/// Recursively key-sorted compact canonical JSON (SOMA convention).
pub fn to_canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let inner: Vec<String> = keys
                .iter()
                .map(|k| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap_or_default(),
                        to_canonical_json(&map[*k])
                    )
                })
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        Value::Array(items) => {
            let inner: Vec<String> = items.iter().map(to_canonical_json).collect();
            format!("[{}]", inner.join(","))
        }
        other => other.to_string(),
    }
}

/// SHA-256 lowercase hex over [`to_canonical_json`].
pub fn canonical_digest(value: &Value) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(to_canonical_json(value).as_bytes());
    Ok(format!("{:x}", hasher.finalize()))
}

/// A checkpoint projected from a `PortableWorkState`, preserving its
/// canonical identifiers/versions/digests (SOMA CheckpointEnvelope-style
/// semver + digest chain, fail-closed on unsupported versions).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectCheckpoint {
    pub schema_version: String,
    pub checkpoint_id: String,
    pub work_id: String,
    pub task_id: String,
    /// Repository revision the underlying state was produced against.
    pub revision: String,
    /// Canonical digest of the source portable work state.
    pub state_digest: String,
}

impl ProjectCheckpoint {
    /// Map FROM a `PortableWorkState`. Fails closed when the source carries an
    /// unsupported major version.
    pub fn from_portable_work_state(pws: &PortableWorkState) -> Result<Self> {
        let ceiling = crate::workflow::schema::SchemaVersion::new(1, u32::MAX, u32::MAX);
        if pws.schema_version > ceiling {
            bail!("unsupported portable work state version (fail closed)");
        }
        let value = serde_json::to_value(pws).context("serialize pws for digest")?;
        Ok(Self {
            schema_version: MEMORY_CONTRACT_VERSION.to_string(),
            checkpoint_id: format!("pcpt-{}", pws.work.work_id),
            work_id: pws.work.work_id.clone(),
            task_id: pws.work.task_id.clone(),
            revision: pws.repository.revision.clone(),
            state_digest: canonical_digest(&value)?,
        })
    }
}

// ---------------------------------------------------------------------------
// Slice 2: provider-neutral retrieval pipeline (scopes, staleness, budget)
// ---------------------------------------------------------------------------

/// A candidate as returned by a raw backend before Lite enforcement.
#[derive(Debug, Clone)]
pub struct RawCandidate {
    pub memory_id: String,
    pub kind: MemoryKind,
    pub source_revision: String,
    pub evidence: EvidenceReferenceV1,
    pub content: String,
    /// Backend relevance in `[0,1]`.
    pub relevance: f32,
}

/// Typed backend failure so "backend unavailable" is never conflated with an
/// ordinary retrieval error (acceptance: backend-unavailable case).
#[derive(Debug)]
pub struct MemoryBackendUnavailable {
    pub backend: BackendKind,
    pub message: String,
}

impl std::fmt::Display for MemoryBackendUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "memory backend '{}' unavailable: {}",
            match self.backend {
                BackendKind::Local => "local",
                BackendKind::Mnemosyne => "mnemosyne",
                BackendKind::CloudAllowed => "cloud-allowed",
            },
            self.message
        )
    }
}

impl std::error::Error for MemoryBackendUnavailable {}

/// Provider-neutral port. Implementations: local store, optional Mnemosyne
/// adapter, cloud-allowed adapter. Ports MUST enforce scope authorization
/// server-side of this trait; Lite re-verifies non-emptiness here.
pub trait MemoryRetrievalPort: Send + Sync {
    fn name(&self) -> &'static str;
    fn backend(&self) -> BackendKind;
    fn retrieve(&self, query: &MemoryQuery) -> Result<Vec<RawCandidate>>;
    fn write(&self, write: &MemoryWrite) -> Result<String>;
}

/// Cheap deterministic token estimate (~4 chars/token); budgets must never
/// depend on a tokenizer being installed.
pub fn estimate_tokens(text: &str) -> u64 {
    (text.chars().count() as u64).div_ceil(4)
}

/// Enforce Lite-side invariants and assemble the provenance-rich result:
///
/// - `query.readable_scopes` MUST be non-empty (empty scope set = nothing
///   authorized => hard error, not an empty success);
/// - candidates whose `source_revision` differs from `current_revision`
///   (when known) are OMITTED with reason `"stale revision"` â€” stale data is
///   never delivered;
/// - candidates exceeding the query's token budget (greedy by relevance,
///   stable order) are omitted with `"token budget exceeded"`;
/// - every delivered candidate carries full SOMA EvidenceReference fields.
pub fn assemble_retrieval(
    query: &MemoryQuery,
    backend: BackendKind,
    mutation: &str,
    executed_at: String,
    raw: Vec<RawCandidate>,
    current_revision: Option<&str>,
) -> Result<RetrievalResult> {
    ensure_supported_contract_version(&query.schema_version)?;
    if query.readable_scopes.is_empty() {
        bail!("memory query carries no readable scopes: nothing is authorized");
    }

    let mut selected: Vec<RetrievalCandidate> = Vec::new();
    let mut omitted: Vec<OmittedEntry> = Vec::new();
    let mut used_tokens: u64 = 0;
    let budget = query.token_budget.unwrap_or(u64::MAX);

    // Stable greedy order: relevance desc, then memory_id for determinism.
    let mut ordered = raw;
    ordered.sort_by(|a, b| {
        b.relevance
            .partial_cmp(&a.relevance)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.memory_id.cmp(&b.memory_id))
    });

    for c in ordered {
        if let Some(cur) = current_revision
            && c.source_revision != cur
        {
            omitted.push(OmittedEntry {
                memory_id: c.memory_id.clone(),
                reason: format!(
                    "stale revision: produced against {}, current {}",
                    c.source_revision, cur
                ),
            });
            continue;
        }
        let tokens = estimate_tokens(&c.content);
        if used_tokens.saturating_add(tokens) > budget {
            omitted.push(OmittedEntry {
                memory_id: c.memory_id.clone(),
                reason: "token budget exceeded".to_string(),
            });
            continue;
        }
        used_tokens = used_tokens.saturating_add(tokens);
        selected.push(RetrievalCandidate {
            memory_id: c.memory_id,
            kind: c.kind,
            source_revision: c.source_revision,
            evidence: c.evidence,
            content: c.content,
            relevance: c.relevance,
        });
    }

    Ok(RetrievalResult {
        schema_version: MEMORY_CONTRACT_VERSION.to_string(),
        query_id: query.query_id.clone(),
        token_estimate: used_tokens,
        candidates: selected,
        omitted,
        policy: OperationPolicy {
            backend,
            mutation: mutation.to_string(),
            executed_at,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::portable_state::{RepositorySnapshot, WorkIdentity};
    use crate::workflow::schema::CURRENT_SCHEMA_VERSION;

    #[test]
    fn future_major_version_fails_closed() {
        let json = r#"{"schemaVersion":"2.0.0","queryId":"q","readableScopes":[],"text":"t"}"#;
        // serde uses our snake/camel rename: schemaVersion matches camelCase.
        let err = MemoryQuery::parse_json(json).unwrap_err();
        assert!(
            err.to_string().contains("fail closed"),
            "unexpected: {err:#}"
        );
    }

    #[test]
    fn query_roundtrip_and_current_version_accepted() {
        let q = MemoryQuery {
            schema_version: MEMORY_CONTRACT_VERSION.to_string(),
            query_id: "q-1".into(),
            readable_scopes: vec!["project".into()],
            text: "find decisions".into(),
            kinds: vec![MemoryKind::Decision],
            token_budget: Some(512),
        };
        let parsed = MemoryQuery::parse_json(&serde_json::to_string(&q).unwrap()).unwrap();
        assert_eq!(parsed, q);
    }

    #[test]
    fn digest_is_stable_hex_and_content_sensitive() {
        let a = serde_json::json!({"b":1,"a":[2,1]});
        let d1 = canonical_digest(&a).unwrap();
        let d2 = canonical_digest(&a).unwrap();
        assert_eq!(d1, d2);
        assert_eq!(d1.len(), 64);
        assert!(
            d1.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        let b = serde_json::json!({"a":[2,1],"b":1});
        assert_eq!(
            canonical_digest(&b).unwrap(),
            d1,
            "key order must not matter"
        );
        let c = serde_json::json!({"b":2,"a":[2,1]});
        assert_ne!(canonical_digest(&c).unwrap(), d1);
    }

    // ---- slice 2: pipeline enforcement ----

    fn ev(id: &str) -> EvidenceReferenceV1 {
        EvidenceReferenceV1 {
            id: id.to_string(),
            event_digest: "a".repeat(64),
            artifact_digest: "b".repeat(64),
            artifact_kind: "memory".into(),
            produced_by: "test".into(),
            produced_at: None,
        }
    }

    fn raw(id: &str, rev: &str, rel: f32, chars: usize) -> RawCandidate {
        RawCandidate {
            memory_id: id.into(),
            kind: MemoryKind::Fact,
            source_revision: rev.into(),
            evidence: ev(id),
            content: "x".repeat(chars),
            relevance: rel,
        }
    }

    fn q(budget: Option<u64>) -> MemoryQuery {
        MemoryQuery {
            schema_version: MEMORY_CONTRACT_VERSION.into(),
            query_id: "q".into(),
            readable_scopes: vec!["p".into()],
            text: "t".into(),
            kinds: vec![],
            token_budget: budget,
        }
    }

    #[test]
    fn empty_scopes_are_rejected() {
        let mut query = q(None);
        query.readable_scopes.clear();
        let err = assemble_retrieval(&query, BackendKind::Local, "none", now(), vec![], None)
            .unwrap_err();
        assert!(err.to_string().contains("no readable scopes"), "{err}");
    }

    #[test]
    fn stale_revision_is_omitted_never_delivered() {
        let r = assemble_retrieval(
            &q(None),
            BackendKind::Local,
            "none",
            now(),
            vec![raw("fresh", "rev1", 0.9, 40), raw("old", "rev0", 0.8, 40)],
            Some("rev1"),
        )
        .unwrap();
        assert_eq!(r.candidates.len(), 1);
        assert_eq!(r.candidates[0].memory_id, "fresh");
        assert_eq!(r.omitted.len(), 1);
        assert!(r.omitted[0].reason.starts_with("stale revision"));
    }

    #[test]
    fn token_budget_trims_by_relevance_with_reason() {
        let mut query = q(Some(20)); // ~80 chars total
        query.readable_scopes = vec!["p".into()];
        let r = assemble_retrieval(
            &query,
            BackendKind::Local,
            "none",
            now(),
            vec![raw("hi", "r", 0.9, 160), raw("lo", "r", 0.5, 40)],
            None,
        )
        .unwrap();
        assert_eq!(r.candidates.len(), 1);
        assert_eq!(r.candidates[0].memory_id, "lo");
        assert_eq!(r.omitted.len(), 1);
        assert_eq!(r.omitted[0].reason, "token budget exceeded");
        assert_eq!(r.token_estimate, estimate_tokens(&"x".repeat(40)));
    }

    #[test]
    fn backend_unavailable_is_a_distinct_typed_error() {
        let e = MemoryBackendUnavailable {
            backend: BackendKind::Mnemosyne,
            message: "connection refused".into(),
        };
        assert!(e.to_string().contains("mnemosyne"));
        assert!(e.to_string().contains("unavailable"));
    }

    struct UnavailablePort;
    impl MemoryRetrievalPort for UnavailablePort {
        fn name(&self) -> &'static str {
            "unavailable"
        }
        fn backend(&self) -> BackendKind {
            BackendKind::Local
        }
        fn retrieve(&self, _q: &MemoryQuery) -> Result<Vec<RawCandidate>> {
            Err(anyhow::Error::new(MemoryBackendUnavailable {
                backend: BackendKind::Local,
                message: "db locked".into(),
            }))
        }
        fn write(&self, _w: &MemoryWrite) -> Result<String> {
            Err(anyhow::Error::new(MemoryBackendUnavailable {
                backend: BackendKind::Local,
                message: "db locked".into(),
            }))
        }
    }

    #[tokio::test]
    async fn port_unavailable_surfaces_typed_error() {
        use anyhow::Context as _;
        let port = UnavailablePort;
        let err = port.retrieve(&q(None)).context("retrieve");
        let err = err.expect_err("must fail");
        let typed = err
            .chain()
            .filter_map(|c| c.downcast_ref::<MemoryBackendUnavailable>())
            .next()
            .expect("typed backend-unavailable must be in the chain");
        assert!(matches!(typed.backend, BackendKind::Local));
    }

    fn now() -> String {
        "2026-08-23T00:00:00Z".into()
    }
    fn sample_pws() -> PortableWorkState {
        PortableWorkState {
            schema_version: CURRENT_SCHEMA_VERSION,
            work: WorkIdentity {
                work_id: "w-1".into(),
                task_id: "t-1".into(),
                objective: "obj".into(),
                created_at: "2026-01-01T00:00:00Z".into(),
                updated_at: "2026-01-01T00:00:00Z".into(),
            },
            repository: RepositorySnapshot {
                identity: "origin".into(),
                branch: "main".into(),
                revision: "abc123".into(),
                local_path: None,
            },
            plan: None,
            steps: vec![],
            decisions: vec![],
            context_refs: vec![],
            artifact_refs: vec![],
            proposal_ref: None,
            diff_ref: None,
            validation_results: vec![],
            review_results: vec![],
            failures: vec![],
            authority: crate::workflow::portable_state::AuthoritySnapshot {
                authority: crate::workflow::AuthorityLevel::Propose,
                allowed_paths: vec![],
                forbidden_paths: vec![],
                allow_dependency_changes: false,
                max_files_changed: None,
                max_lines_changed: None,
                policy_digest: None,
            },
            execution_history: vec![],
            compatibility: crate::workflow::portable_state::CompatibilityMetadata {
                state_schema_version: crate::workflow::schema::PORTABLE_WORK_STATE_SCHEMA_VERSION,
                required_capabilities: vec![],
                optional_capabilities: vec![],
                resume_blockers: vec![],
            },
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn checkpoint_mapping_roundtrips_identifiers() {
        let pws = sample_pws();
        let cp = ProjectCheckpoint::from_portable_work_state(&pws).unwrap();
        assert_eq!(cp.work_id, "w-1");
        assert_eq!(cp.task_id, "t-1");
        assert_eq!(cp.revision, "abc123");
        assert_eq!(cp.state_digest.len(), 64);
        // Deterministic: same state => same digest.
        let again = ProjectCheckpoint::from_portable_work_state(&pws).unwrap();
        assert_eq!(cp, again);
    }
}
