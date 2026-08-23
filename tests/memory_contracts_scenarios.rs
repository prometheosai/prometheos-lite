//! End-to-end scenario coverage for #152's acceptance list, exercising ONLY
//! the public `lite.memory.v1` surface (types + ports + assembly). Each test
//! is one named acceptance scenario.

use prometheos_lite::workflow::memory_contracts::{
    BackendKind, ContextBundle, EvidenceReferenceV1, MemoryBackendUnavailable, MemoryKind,
    MemoryQuery, MemoryRetrievalPort, MemoryWrite, OperationPolicy, RawCandidate,
    assemble_context_bundle, assemble_retrieval, estimate_tokens,
};

const REV: &str = "9d4f1c";

fn ev(id: &str) -> EvidenceReferenceV1 {
    EvidenceReferenceV1 {
        id: id.to_string(),
        event_digest: "1".repeat(64),
        artifact_digest: "2".repeat(64),
        artifact_kind: "memory".into(),
        produced_by: "scenario".into(),
        produced_at: Some("2026-08-23T00:00:00Z".into()),
    }
}

fn raw(id: &str, rel: f32) -> RawCandidate {
    RawCandidate {
        memory_id: id.into(),
        kind: MemoryKind::Decision,
        source_revision: REV.into(),
        evidence: ev(id),
        content: format!("content-for-{id} "),
        relevance: rel,
    }
}

fn query(budget: Option<u64>) -> MemoryQuery {
    MemoryQuery {
        schema_version: "1.0.0".into(),
        query_id: "q-scenario".into(),
        readable_scopes: vec!["project://demo".into()],
        text: "what did we decide".into(),
        kinds: vec![],
        token_budget: budget,
    }
}

fn policy(backend: BackendKind) -> OperationPolicy {
    OperationPolicy {
        backend,
        mutation: "none".into(),
        executed_at: "2026-08-23T00:00:00Z".into(),
    }
}

/// In-memory port standing in for any backend (local / mnemosyne stub /
/// cloud-allowed adapter): proves ports are interchangeable.
struct VecPort {
    backend: BackendKind,
    rows: Vec<RawCandidate>,
    unavailable: bool,
}

impl MemoryRetrievalPort for VecPort {
    fn name(&self) -> &'static str {
        "vec-port"
    }
    fn backend(&self) -> BackendKind {
        self.backend
    }
    fn retrieve(&self, _q: &MemoryQuery) -> anyhow::Result<Vec<RawCandidate>> {
        if self.unavailable {
            return Err(anyhow::Error::new(MemoryBackendUnavailable {
                backend: self.backend,
                message: "backend offline".into(),
            }));
        }
        Ok(self.rows.clone())
    }
    fn write(&self, _w: &MemoryWrite) -> anyhow::Result<String> {
        Err(anyhow::Error::new(MemoryBackendUnavailable {
            backend: self.backend,
            message: "writes disabled in scenario port".into(),
        }))
    }
}

fn run(
    port: &VecPort,
    budget: Option<u64>,
    current: Option<&str>,
) -> anyhow::Result<ContextBundle> {
    let q = query(budget);
    let raws = port.retrieve(&q)?;
    let result = assemble_retrieval(
        &q,
        port.backend(),
        "none",
        "2026-08-23T00:00:00Z".into(),
        raws,
        current,
    )?;
    let omitted = result.omitted.clone();
    assemble_context_bundle(
        &format!("bundle-{}", q.query_id),
        &result.query_id,
        result.candidates,
        omitted,
        result.policy,
    )
}

#[test]
fn scenario_local_only_happy_path() {
    let port = VecPort {
        backend: BackendKind::Local,
        rows: vec![raw("m1", 0.9), raw("m2", 0.7)],
        unavailable: false,
    };
    let b = run(&port, None, Some(REV)).unwrap();
    assert_eq!(b.blocks.len(), 2);
    assert!(b.omitted.is_empty());
    assert_eq!(
        b.blocks.iter().map(|x| x.tokens).sum::<u64>(),
        b.token_estimate
    );
    assert_eq!(b.digest.len(), 64);
}

#[test]
fn scenario_mnemosyne_backed_port_is_interchangeable() {
    // A Mnemosyne-backed adapter returns identical authorized semantics; the
    // bundle records the backend in its policy without changing contracts.
    let port = VecPort {
        backend: BackendKind::Mnemosyne,
        rows: vec![raw("mm1", 0.8)],
        unavailable: false,
    };
    let b = run(&port, None, Some(REV)).unwrap();
    assert_eq!(b.policy.backend, BackendKind::Mnemosyne);
    assert_eq!(b.blocks.len(), 1);
}

#[test]
fn scenario_cloud_allowed_policy_is_recorded() {
    let port = VecPort {
        backend: BackendKind::CloudAllowed,
        rows: vec![raw("c1", 0.6)],
        unavailable: false,
    };
    let b = run(&port, None, Some(REV)).unwrap();
    assert_eq!(b.policy.backend, BackendKind::CloudAllowed);
    assert_eq!(b.blocks[0].memory_id, "c1");
}

#[test]
fn scenario_stale_revision_never_delivered() {
    let mut stale = raw("old", 0.95);
    stale.source_revision = "older-rev".into();
    let port = VecPort {
        backend: BackendKind::Local,
        rows: vec![stale, raw("fresh", 0.5)],
        unavailable: false,
    };
    let b = run(&port, None, Some(REV)).unwrap();
    assert_eq!(b.blocks.len(), 1);
    assert_eq!(b.blocks[0].memory_id, "fresh");
    assert!(
        b.omitted
            .iter()
            .any(|o| o.reason.starts_with("stale revision"))
    );
}

#[test]
fn scenario_conflicting_duplicates_resolved_with_reason() {
    let mut dup_a = raw("dup", 0.5);
    dup_a.evidence.event_digest = "a".repeat(64);
    let mut dup_b = raw("dup", 0.8);
    dup_b.evidence.event_digest = "b".repeat(64);
    let port = VecPort {
        backend: BackendKind::Local,
        rows: vec![dup_a, dup_b],
        unavailable: false,
    };
    let b = run(&port, None, Some(REV)).unwrap();
    assert_eq!(b.blocks.len(), 1);
    assert!(
        b.omitted
            .iter()
            .any(|o| o.reason.starts_with("conflicting duplicate"))
    );
}

#[test]
fn scenario_token_budget_selects_and_reports_omissions() {
    let port = VecPort {
        backend: BackendKind::Local,
        rows: vec![raw("big", 0.99), raw("small", 0.4)],
        unavailable: false,
    };
    // ~4 chars/token: "big" (~16 chars content => 4 tokens x?) sized to overflow.
    let big_chars = 400;
    let mut rows = port.rows.clone();
    rows[0].content = "z".repeat(big_chars);
    let b = run(&VecPort { rows, ..port }, Some(50), Some(REV)).unwrap();
    assert_eq!(b.blocks.len(), 1);
    assert_eq!(b.blocks[0].memory_id, "small");
    assert!(
        b.omitted
            .iter()
            .any(|o| o.reason == "token budget exceeded")
    );
}

#[test]
fn scenario_backend_unavailable_is_typed_not_generic() {
    let port = VecPort {
        backend: BackendKind::Mnemosyne,
        rows: vec![],
        unavailable: true,
    };
    let err = run(&port, None, Some(REV)).unwrap_err();
    let typed = err
        .chain()
        .filter_map(|c| c.downcast_ref::<MemoryBackendUnavailable>())
        .next()
        .expect("typed error must survive propagation");
    assert_eq!(typed.backend, BackendKind::Mnemosyne);
}

#[test]
fn scenario_deletion_expiry_write_is_scope_fail_closed() {
    // Deletion/expiry surface today: a write with NO writable scopes is
    // rejected at parse — nothing can be persisted without authorization.
    let w = MemoryWrite {
        schema_version: "1.0.0".into(),
        write_id: "w-expire".into(),
        writable_scopes: vec![], // unauthorized: no scope grants this write
        kind: MemoryKind::Fact,
        content: "expired fact".into(),
        tags: vec!["ttl:0".into()],
    };
    let err = MemoryWrite::parse_json(&serde_json::to_string(&w).unwrap()).unwrap_err();
    assert!(err.to_string().contains("no writable scopes"));
    // And the port would refuse anyway (writes disabled in scenario port).
    let port = VecPort {
        backend: BackendKind::Local,
        rows: vec![],
        unavailable: false,
    };
    let parsed = MemoryWrite {
        writable_scopes: vec!["project://demo".into()],
        ..w
    };
    let err2 = port.write(&parsed).unwrap_err();
    assert!(err2.to_string().contains("unavailable"));
}

#[test]
fn estimate_matches_bundle_accounting() {
    let c = vec![raw("t1", 0.5)];
    let pol = policy(BackendKind::Local);
    let r = assemble_retrieval(
        &query(None),
        BackendKind::Local,
        "none",
        "2026-08-23T00:00:00Z".into(),
        c,
        None,
    )
    .unwrap();
    let bundle = assemble_context_bundle("b", &r.query_id, r.candidates, r.omitted, pol).unwrap();
    let expected: u64 = bundle
        .blocks
        .iter()
        .map(|b| estimate_tokens(&b.content))
        .sum();
    assert_eq!(expected, bundle.token_estimate);
}
