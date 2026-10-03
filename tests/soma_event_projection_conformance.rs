//! #132 Slice 1B: the pure fail-closed SOMA `WorkEvent` projection over
//! the verified Slice 1A journal — conformance, goldens, and the
//! fail-closed matrix.
//!
//! Layer 1 (this file's first tests): the vendored SPEC 006 event
//! fixtures (`vendored/soma/v1.1/fixtures/`, digest-locked by the
//! bundle manifest) must audit with exactly their manifest-pinned
//! diagnostics under Lite's vendored verifier — cross-implementation
//! parity with the published soma-native verifier BEFORE the journal
//! mapping exists.
//!
//! Later sections add the journal-driven mapping tests, the byte-locked
//! goldens, and the fail-closed matrix (legacy/tampered/mixed/unmapped
//! records are refused, never fabricated).

use prometheos_lite::workflow::soma::canonical::try_canonical_digest;
use prometheos_lite::workflow::soma::event::{WorkEvent, WorkEventBatch};
use prometheos_lite::workflow::soma::supported_version;
use serde::Deserialize;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/vendored/soma/v1.1/fixtures");

#[derive(Debug, Deserialize)]
struct ManifestEntry {
    path: String,
    kind: String,
    #[serde(rename = "expected_codes")]
    expected_codes: Vec<String>,
    artifact: String,
    sha256: String,
}

/// Every SPEC 006 event fixture the v1.1 bundle pins (manifest-driven;
/// a bundle upgrade that adds fixtures automatically extends this test).
fn event_manifest_entries() -> Vec<ManifestEntry> {
    #[derive(Debug, Deserialize)]
    struct ManifestFile {
        fixtures: Vec<ManifestEntry>,
    }
    let manifest: ManifestFile = serde_json::from_str(
        &std::fs::read_to_string(format!("{FIXTURES}/manifest.json"))
            .expect("vendored fixture manifest is present"),
    )
    .expect("vendored fixture manifest shape is pinned by the bundle");
    manifest
        .fixtures
        .into_iter()
        .filter(|e| e.artifact == "WorkEvent" || e.artifact == "WorkEventBatch")
        .collect()
}

fn fixture_bytes(entry: &ManifestEntry) -> Vec<u8> {
    // Manifest paths are "fixtures/<kind>/<name>.json" relative to the
    // bundle root (vendored/soma/v1.1/).
    let rel = entry.path.strip_prefix("fixtures/").unwrap_or(&entry.path);
    std::fs::read(format!("{FIXTURES}/{rel}"))
        .unwrap_or_else(|e| panic!("{}: fixture missing ({e})", entry.path))
}

/// The known event-fixture set: if the bundle gains or loses fixtures,
/// this assert forces an explicit review of the change.
#[test]
fn vendored_event_fixture_set_is_the_pinned_fifteen() {
    let entries = event_manifest_entries();
    let mut names: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "fixtures/invalid/wev-auth-cap.json",
            "fixtures/invalid/wev-auth-level.json",
            "fixtures/invalid/wev-auth-mutation.json",
            "fixtures/invalid/wev-auth-network.json",
            "fixtures/invalid/wev-auth-scope.json",
            "fixtures/invalid/wev-cyclic.json",
            "fixtures/invalid/wev-missing-parent.json",
            "fixtures/invalid/wev-no-evidence.json",
            "fixtures/invalid/wev-replay-conflict.json",
            "fixtures/invalid/wev-semantic-loss.json",
            "fixtures/invalid/wev-unsupported-version.json",
            "fixtures/valid/wev-checkpoint-resume.json",
            "fixtures/valid/wev-handoff-native.json",
            "fixtures/valid/wev-valid-chain.json",
            "fixtures/valid/wev-valid-idempotent.json",
        ],
        "the vendored v1.1 event fixture set changed — review the bundle upgrade"
    );
}

/// Digest-lock: every event fixture's CANONICAL CONTENT digest must equal
/// the manifest sha256 — the same digest the bundle pins (computed by the
/// soma-native verifier over the canonical SOMA render of the parsed
/// fixture, NOT raw file bytes — the lock is therefore EOL-independent).
#[test]
fn vendored_event_fixtures_match_their_manifest_digests() {
    for entry in event_manifest_entries() {
        let bytes = fixture_bytes(&entry);
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .unwrap_or_else(|e| panic!("{}: not valid JSON: {e}", entry.path));
        let digest = try_canonical_digest(&value)
            .unwrap_or_else(|e| panic!("{}: canonical digest failed: {e}", entry.path));
        assert_eq!(
            digest, entry.sha256,
            "{}: canonical content digest diverges from the manifest digest",
            entry.path
        );
    }
}

/// Valid SPEC 006 fixtures must parse into their pinned artifact type
/// and audit CLEAN under the vendored verifier.
#[test]
fn vendored_valid_event_fixtures_audit_clean() {
    let supported = supported_version();
    for entry in event_manifest_entries()
        .into_iter()
        .filter(|e| e.kind == "valid")
    {
        let bytes = fixture_bytes(&entry);
        match entry.artifact.as_str() {
            "WorkEvent" => {
                let event: WorkEvent = serde_json::from_slice(&bytes)
                    .unwrap_or_else(|e| panic!("{}: parse failed: {e}", entry.path));
                assert!(
                    event.audit(&supported).is_empty(),
                    "{}: valid fixture audits dirty: {:?}",
                    entry.path,
                    event.audit(&supported)
                );
            }
            "WorkEventBatch" => {
                let batch: WorkEventBatch = serde_json::from_slice(&bytes)
                    .unwrap_or_else(|e| panic!("{}: parse failed: {e}", entry.path));
                assert!(
                    batch.audit(&supported).is_empty(),
                    "{}: valid fixture audits dirty: {:?}",
                    entry.path,
                    batch.audit(&supported)
                );
            }
            other => panic!("{}: unknown artifact {other}", entry.path),
        }
    }
}

/// Invalid SPEC 006 fixtures must audit with EXACTLY their
/// manifest-pinned `expected_codes` (set equality — the same ground-truth
/// rule the published soma-native verifier enforces).
#[test]
fn vendored_invalid_event_fixtures_produce_manifest_pinned_codes() {
    let supported = supported_version();
    for entry in event_manifest_entries()
        .into_iter()
        .filter(|e| e.kind == "invalid")
    {
        let bytes = fixture_bytes(&entry);
        let codes: Vec<String> = match entry.artifact.as_str() {
            "WorkEvent" => {
                let event: WorkEvent = serde_json::from_slice(&bytes)
                    .unwrap_or_else(|e| panic!("{}: parse failed: {e}", entry.path));
                event
                    .audit(&supported)
                    .into_iter()
                    .map(|d| d.code)
                    .collect()
            }
            "WorkEventBatch" => {
                let batch: WorkEventBatch = serde_json::from_slice(&bytes)
                    .unwrap_or_else(|e| panic!("{}: parse failed: {e}", entry.path));
                batch
                    .audit(&supported)
                    .into_iter()
                    .map(|d| d.code)
                    .collect()
            }
            other => panic!("{}: unknown artifact {other}", entry.path),
        };
        let mut want = entry.expected_codes.clone();
        want.sort_unstable();
        let mut got = codes;
        got.sort_unstable();
        got.dedup();
        assert_eq!(
            got, want,
            "{}: audit codes diverge from the manifest-pinned expected_codes",
            entry.path
        );
    }
}

// ---------------------------------------------------------------------------
// Slice 1B journal projection: stream pages (correction 1 - a page is
// NOT a WorkEventBatch), complete-record source digest (correction 2),
// cursor semantics, and determinism.
// ---------------------------------------------------------------------------

use prometheos_lite::db::Db;
use prometheos_lite::work::JournalContext;
use prometheos_lite::work::WorkContextService;
use prometheos_lite::work::soma_projection::{ProjectionPage, WorkEventStreamPage, project_page};
use prometheos_lite::work::types::{AutonomyLevel, WorkDomain};
use prometheos_lite::workflow::projection::VersionedProjectionEnvelope;
use std::sync::Arc;

fn test_journal() -> JournalContext {
    JournalContext::internal_system(
        format!("test-{}", uuid::Uuid::new_v4()),
        JournalContext::work_authority(AutonomyLevel::Review, Default::default()),
    )
}

fn setup() -> (Arc<Db>, Arc<WorkContextService>) {
    let db = Arc::new(Db::in_memory().unwrap());
    let wcs = Arc::new(WorkContextService::new(db.clone()));
    (db, wcs)
}

fn create_context(wcs: &WorkContextService) -> prometheos_lite::work::types::WorkContext {
    let journal = test_journal();
    wcs.create_context(
        "user-1".to_string(),
        "Projection test".to_string(),
        WorkDomain::General,
        "goal".to_string(),
        &journal,
    )
    .unwrap()
}

/// Drive a fixed, fully-mappable event set through the REAL writers.
fn drive_events(wcs: &WorkContextService) -> prometheos_lite::work::types::WorkContext {
    let journal = test_journal();
    let mut context = create_context(wcs); // context_created
    wcs.update_status(
        &mut context,
        prometheos_lite::work::types::WorkStatus::InProgress,
        &journal,
    )
    .unwrap(); // status_changed
    wcs.update_phase(
        &mut context,
        prometheos_lite::work::types::WorkPhase::Planning,
        &journal,
    )
    .unwrap(); // phase_transition
    context
}

#[test]
fn page_payload_is_not_a_batch_on_the_wire() {
    let (db, wcs) = setup();
    let context = drive_events(&wcs);
    let page = project_page(&db, &context.id, 0, 500).unwrap();
    let event = page.envelope.payload.events[0].clone();

    // A page serializes with ONLY `events`; deserializing it as a
    // WorkEventBatch must fail (missing required batch fields).
    let page_bytes = serde_json::to_vec(&WorkEventStreamPage {
        events: vec![event.clone()],
    })
    .unwrap();
    assert!(
        serde_json::from_slice::<prometheos_lite::workflow::soma::event::WorkEventBatch>(
            &page_bytes
        )
        .is_err(),
        "a stream page must not be consumable as a WorkEventBatch"
    );

    // A batch must not deserialize as a page (deny_unknown_fields).
    let batch = prometheos_lite::workflow::soma::event::WorkEventBatch {
        schema_version: "1.1.0".to_string(),
        version: "1.1.0".to_string(),
        run_id: "run-1".to_string(),
        events: vec![event],
        compatibility: None,
    };
    let batch_bytes = serde_json::to_vec(&batch).unwrap();
    assert!(
        serde_json::from_slice::<WorkEventStreamPage>(&batch_bytes).is_err(),
        "a WorkEventBatch must not be consumable as a stream page"
    );
}

#[test]
fn pages_cover_the_journal_without_gaps_or_duplication() {
    let (db, wcs) = setup();
    let context = drive_events(&wcs); // exactly 3 events

    // Page through with a small limit; the union must be every event,
    // seq-ordered, exactly once.
    let mut seen: Vec<(u64, String)> = Vec::new();
    let mut after = 0i64;
    let mut pages = 0;
    loop {
        let page: ProjectionPage = project_page(&db, &context.id, after, 2).unwrap();
        for event in &page.envelope.payload.events {
            seen.push((event.sequence, event.id.clone()));
        }
        pages += 1;
        match page.next_after {
            Some(next) => after = next,
            None => break,
        }
        assert!(pages < 10, "paging must terminate");
    }
    assert_eq!(pages, 2, "3 events with limit 2 -> exactly 2 pages");
    assert_eq!(seen.len(), 3);
    let mut seqs: Vec<u64> = seen.iter().map(|(s, _)| *s).collect();
    let mut sorted = seqs.clone();
    sorted.sort_unstable();
    assert_eq!(seqs, sorted, "pages preserve durable seq order");
    seqs.dedup();
    assert_eq!(seqs.len(), 3, "no duplicates across pages");

    // Resuming from the last DELIVERED seq (the exhausted cursor) yields
    // an empty page: no gaps, no duplication on reconnect.
    let last_delivered = seen.last().unwrap().0 as i64;
    let done = project_page(&db, &context.id, last_delivered, 2).unwrap();
    assert!(done.envelope.payload.events.is_empty());
    assert_eq!(done.next_after, None);
}

#[test]
fn page_envelope_carries_verified_digests_and_is_deterministic() {
    let (db, wcs) = setup();
    let context = drive_events(&wcs);

    let first = project_page(&db, &context.id, 0, 500).unwrap();
    let second = project_page(&db, &context.id, 0, 500).unwrap();

    // Pure projection: identical input -> byte-identical canonical output.
    assert_eq!(
        first.envelope.canonical_bytes().unwrap(),
        second.envelope.canonical_bytes().unwrap(),
        "the projection is deterministic"
    );

    let env: &VersionedProjectionEnvelope<WorkEventStreamPage> = &first.envelope;
    assert_eq!(env.projection_version, "projection.v1");
    assert_eq!(env.schema_version, "1.1.0");
    assert_ne!(
        env.source_digest, env.projection_digest,
        "journal binding and payload digest are distinct roles"
    );
    for digest in [&env.source_digest, &env.projection_digest] {
        assert_eq!(digest.len(), 64);
        assert!(
            digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
    }
}

#[test]
fn empty_context_projects_an_empty_page() {
    let (db, _wcs) = setup();
    // A context id with no journal at all: an honest empty stream page.
    let page = project_page(&db, &uuid::Uuid::new_v4().to_string(), 0, 10).unwrap();
    assert!(page.envelope.payload.events.is_empty());
    assert_eq!(page.next_after, None);
}
