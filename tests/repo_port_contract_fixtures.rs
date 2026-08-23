//! Shared retrieval-contract fixtures for #167/#153 (slice 3).
//!
//! Both the local repository port and any future Mnemosyne adapter must
//! produce equivalent authorized retrieval semantics over these inputs.

use prometheos_lite::workflow::memory_contracts::{
    BackendKind, MemoryBackendUnavailable, MemoryQuery, MemoryRetrievalPort, MemoryWrite,
    RawCandidate, assemble_context_bundle, assemble_retrieval,
};
use prometheos_lite::workflow::repo_index::{IndexedRepository, RepoEvidencePort};

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

fn query(text: &str) -> MemoryQuery {
    MemoryQuery {
        schema_version: "1.0.0".into(),
        query_id: "fixture-q".into(),
        readable_scopes: vec!["repo://fixture".into()],
        text: text.into(),
        kinds: vec![],
        token_budget: None,
    }
}

#[test]
fn fixture_local_backend_end_to_end() {
    let dir = init_repo("fx-local");
    let repo = dir.path().join("fx-local");
    let index = IndexedRepository::build(&repo).unwrap();
    let revision = index.identity.revision.clone();
    let port = RepoEvidencePort {
        index,
        current_revision: Some(revision.clone()),
    };

    // Contract: exact lookup intent.
    let raws = port.retrieve(&query("helper")).expect("retrieve");
    assert!(!raws.is_empty());
    for c in &raws {
        assert_eq!(c.source_revision, revision);
        assert_eq!(c.evidence.artifact_kind, "repository-symbol");
        assert_eq!(c.evidence.event_digest.len(), 64);
        assert_eq!(c.evidence.produced_by, "1.0.0");
    }
    // Contract: full pipeline to bundle with provenance + reasons + digest.
    let result = assemble_retrieval(
        &query("helper"),
        BackendKind::Local,
        "none",
        "2026-08-23T00:00:00Z".into(),
        raws,
        Some(revision.as_str()),
    )
    .unwrap();
    let bundle = assemble_context_bundle(
        "b-fixture",
        &result.query_id,
        result.candidates,
        result.omitted,
        result.policy,
    )
    .unwrap();
    assert!(!bundle.blocks.is_empty());
    assert_eq!(bundle.digest.len(), 64);
}

#[test]
fn fixture_stale_revision_is_typed_rejection() {
    let dir = init_repo("fx-stale");
    let repo = dir.path().join("fx-stale");
    let index = IndexedRepository::build(&repo).unwrap();
    let port = RepoEvidencePort {
        index,
        current_revision: Some("deadbeef".into()),
    };
    let err = port
        .retrieve(&query("helper"))
        .expect_err("stale must fail closed");
    let stale = err
        .chain()
        .filter_map(|c| c.downcast_ref::<prometheos_lite::workflow::repo_index::IndexStale>())
        .next()
        .expect("typed IndexStale in chain");
    assert!(stale.reason.contains("!= indexed revision"));
}

#[test]
fn fixture_not_found_is_empty_evidence_never_invented() {
    let dir = init_repo("fx-miss");
    let repo = dir.path().join("fx-miss");
    let index = IndexedRepository::build(&repo).unwrap();
    let port = RepoEvidencePort {
        index,
        current_revision: None,
    };
    let raws = port
        .retrieve(&query("no_such_symbol_xyz"))
        .expect("retrieve");
    assert!(raws.is_empty(), "missing symbols stay typed-absent");
}

#[test]
fn fixture_mnemosyne_shaped_port_passes_same_contract() {
    // A stub adapter proving the CONTRACT (not Mnemosyne itself) is portable:
    // same query -> same candidate shape/provenance obligations.
    struct MnemoStub;
    impl MemoryRetrievalPort for MnemoStub {
        fn name(&self) -> &'static str {
            "mnemosyne-stub"
        }
        fn backend(&self) -> BackendKind {
            BackendKind::Mnemosyne
        }
        fn retrieve(&self, q: &MemoryQuery) -> anyhow::Result<Vec<RawCandidate>> {
            if q.text == "helper" {
                return Ok(vec![RawCandidate {
                    memory_id: "mnemo:helper".into(),
                    kind: prometheos_lite::workflow::memory_contracts::MemoryKind::Fact,
                    source_revision: "9d4f1c".into(),
                    evidence: prometheos_lite::workflow::memory_contracts::EvidenceReferenceV1 {
                        id: "mnemo:helper".into(),
                        event_digest: "3".repeat(64),
                        artifact_digest: "4".repeat(64),
                        artifact_kind: "repository-symbol".into(),
                        produced_by: "stub".into(),
                        produced_at: None,
                    },
                    content: "helper".into(),
                    relevance: 0.9,
                }]);
            }
            Ok(vec![])
        }
        fn write(&self, _w: &MemoryWrite) -> anyhow::Result<String> {
            Err(anyhow::Error::new(MemoryBackendUnavailable {
                backend: BackendKind::Mnemosyne,
                message: "stub".into(),
            }))
        }
    }
    let port = MnemoStub;
    let raws = port.retrieve(&query("helper")).expect("retrieve");
    // Same obligations as local: revision-bound provenance fields present.
    assert!(raws.iter().all(|c| c.evidence.event_digest.len() == 64
        && c.evidence.artifact_digest.len() == 64
        && !c.source_revision.is_empty()));
}

#[test]
fn fixture_write_is_read_only_typed() {
    let dir = init_repo("fx-write");
    let repo = dir.path().join("fx-write");
    let index = IndexedRepository::build(&repo).unwrap();
    let port = RepoEvidencePort {
        index,
        current_revision: None,
    };
    let w = MemoryWrite {
        schema_version: "1.0.0".into(),
        write_id: "w".into(),
        writable_scopes: vec!["s".into()],
        kind: prometheos_lite::workflow::memory_contracts::MemoryKind::Fact,
        content: "x".into(),
        tags: vec![],
    };
    let err = MemoryRetrievalPort::write(&port, &w).expect_err("read-only surface");
    assert!(err.to_string().contains("read-only"));
}
