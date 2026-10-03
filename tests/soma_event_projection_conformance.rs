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
