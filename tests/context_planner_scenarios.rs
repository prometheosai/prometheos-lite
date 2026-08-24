//! End-to-end scenario wiring RepoEvidencePort into the ContextPlanner.

use prometheos_lite::workflow::memory_contracts::{
    BackendKind, MemoryQuery, MemoryRetrievalPort, assemble_retrieval,
};
use prometheos_lite::workflow::repo_index::{IndexedRepository, RepoEvidencePort};

#[test]
fn local_repo_port_end_to_end_through_planner() {
    let dir = tempfile::tempdir_in(std::env::temp_dir()).unwrap();
    let repo = dir.path().join("cp-repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    let g = |a: &[&str]| {
        let o = std::process::Command::new("git")
            .args(a)
            .current_dir(&repo)
            .output()
            .unwrap();
        assert!(o.status.success());
    };
    g(&["init", "-q"]);
    g(&["config", "user.email", "t@t"]);
    g(&["config", "user.name", "T"]);
    std::fs::write(
        repo.join("src/lib.rs"),
        "pub fn compute(x: u32) -> u32 { x * 2 }\n",
    )
    .unwrap();
    g(&["add", "."]);
    g(&["commit", "-q", "-m", "base"]);

    let index = IndexedRepository::build(&repo).unwrap();
    let revision = index.identity.revision.clone();
    let port = RepoEvidencePort {
        index,
        current_revision: Some(revision.clone()),
    };

    let query = MemoryQuery {
        schema_version: "1.0.0".into(),
        query_id: "q-cp".into(),
        readable_scopes: vec!["repo://fixture".into()],
        text: "compute".into(),
        kinds: vec![],
        token_budget: None,
    };
    let raws = MemoryRetrievalPort::retrieve(&port, &query).unwrap();
    assert!(!raws.is_empty(), "port must find 'compute'");
    let result = assemble_retrieval(
        &query,
        BackendKind::Local,
        "none",
        "2026-08-24T00:00:00Z".into(),
        raws,
        Some(revision.as_str()),
    )
    .unwrap();
    assert!(!result.candidates.is_empty());
    for c in &result.candidates {
        assert_eq!(c.source_revision, revision);
        assert_eq!(c.evidence.artifact_kind, "repository-symbol");
    }
}
