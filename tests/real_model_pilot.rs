//! #136 Slice D — genuine model-authored repair pilot.
//!
//! Two modes, both honest:
//!
//! * **Offline (default, runs in CI)** — deterministic canned responders
//!   drive the governed machinery. These tests prove the pilot path
//!   end-to-end (pre-fix failure capture, governed dispatch, proposal
//!   parsing, single-file write authority, provenance binding, sealing,
//!   reseal rejection, refusal classes and accounting honesty). The canned
//!   responder is **not** a model and is never presented as one.
//!
//! * **Live (opt-in)** — `PROMETHEOS_REAL_MODEL_PILOT=1` plus an already
//!   configured `OPENROUTER_API_KEY` runs the real thing: a Lite-governed
//!   local-model endpoint (loopback) whose operator gateway forwards to
//!   OpenRouter's free `nvidia/nemotron-3-ultra-550b-a55b:free` model
//!   (listed at prompt/completion price 0). The key is read from the
//!   environment at run time, never logged, and never committed.
//!   Evidence is written to `PROMETHEOS_PILOT_EVIDENCE_OUT` (default
//!   `target/real-model-pilot-evidence.json`). No other network egress
//!   happens. This test is skipped by default and never runs in CI.

mod common;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use common::model_pilot::{
    MAX_ATTEMPTS, ModelPilotRun, PILOT_EXAMPLE_PRICE_BOOK, PilotEndpoint, build_solver_messages,
    run_model_pilot, solver_visible_files,
};
use common::task_execution::sha256_hex;
use prometheos_lite::workflow::route_resolution::RouteProfile;
use prometheos_lite::workflow::scenario::{
    ScenarioOutcomeClass, TaskOutcome, TaskRepairOrigin, TaskRepairProvenanceKind,
    TaskRepairWriteAuthority,
};
use prometheos_lite::workflow::soma::contracts::NetworkPolicy;
use prometheos_lite::workflow::soma::types::NetworkDefault;

const OFFLINE_DESCRIPTOR: &str =
    include_str!("fixtures/scenarios/rust-tokenbucket-bugfix/descriptor.json");
const LIVE_DESCRIPTOR: &str =
    include_str!("fixtures/scenarios/rust-tokenbucket-bugfix/descriptor.live.json");

// ---------------------------------------------------------------------------
// Canned (offline) responders
// ---------------------------------------------------------------------------

fn canned_chat_completion(model: &str, content: &str, with_usage: bool) -> String {
    let usage = if with_usage {
        json!({ "prompt_tokens": 125, "completion_tokens": 40, "total_tokens": 165 })
    } else {
        Value::Null
    };
    json!({
        "id": "chatcmpl-offline-pilot",
        "object": "chat.completion",
        "model": model,
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": content },
            "finish_reason": "stop",
        }],
        "usage": usage,
    })
    .to_string()
}

/// The correct, minimal repair, as a canned model proposal. This is the
/// task-setter's reference repair used ONLY by the offline canned path to
/// emulate a successful model response; it is never shown to the live
/// model.
fn reference_proposal() -> String {
    json!({
        "edits": [{
            "file": "src/bucket.rs",
            "search": "        self.last_refill = now_millis;",
            "replace": "        self.last_refill += ticks * self.tick_millis;",
        }]
    })
    .to_string()
}

fn run_pilot_with_canned(
    descriptor: &str,
    price_book: &str,
    timeout_ms: u64,
    responses: Vec<String>,
) -> ModelPilotRun {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    let responses = RefCell::new(VecDeque::from(responses));
    let mut spawn = |_attempt: u32| {
        // Once the scripted responses are exhausted, the canned model
        // produces non-JSON junk forever (a stubborn model), never a
        // silent success. Tests that want recovery script the recovery.
        let body = responses.borrow_mut().pop_front().unwrap_or_else(|| {
            canned_chat_completion("tiny-local-model", "I cannot help with that.", true)
        });
        let server = common::task_execution::spawn_loopback_model(body);
        PilotEndpoint {
            endpoint: server.endpoint,
            effects: server.effects,
        }
    };
    run_model_pilot(
        descriptor,
        price_book,
        timeout_ms,
        RouteProfile::Standard,
        TaskRepairProvenanceKind::CannedTest,
        &mut spawn,
    )
}

fn run_canned(descriptor: &str, responses: Vec<String>) -> ModelPilotRun {
    run_pilot_with_canned(descriptor, PILOT_EXAMPLE_PRICE_BOOK, 5_000, responses)
}

// ---------------------------------------------------------------------------
// Offline: canned end-to-end runs
// ---------------------------------------------------------------------------

#[test]
fn offline_canned_model_repair_seals_repaired_envelope() {
    let run = run_canned(
        OFFLINE_DESCRIPTOR,
        vec![canned_chat_completion(
            "tiny-local-model",
            &reference_proposal(),
            true,
        )],
    );
    // Phase B: the pre-fix failure really happened (exit code non-zero).
    let pre_fix = run.pre_fix.as_ref().expect("pre-fix probe recorded");
    assert_ne!(pre_fix.exit_code, Some(0), "pre-fix validator must fail");
    assert_eq!(pre_fix.command, "cargo");

    assert_eq!(run.attempts.len(), 1);
    assert!(run.patched);
    assert!(
        run.changed_vs_baseline.iter().any(|p| p == "src/bucket.rs"),
        "changed set must include src/bucket.rs: {:?}",
        run.changed_vs_baseline
    );
    let envelope = run.envelope.as_ref().expect("envelope sealed");
    envelope.verify().expect("envelope verifies");
    assert_eq!(envelope.task_outcome, TaskOutcome::Repaired);
    assert_eq!(envelope.repair.origin, TaskRepairOrigin::ModelAuthored);
    assert_eq!(
        envelope.repair.write_authority,
        TaskRepairWriteAuthority::ModelAuthoredWorkspaceFilePolicy
    );
    let provenance = envelope
        .repair
        .model_provenance
        .as_ref()
        .expect("provenance bound");
    assert_eq!(provenance.proposal_digest, envelope.repair.patch_digest);
    assert_eq!(provenance.attempt, 1);
    assert_eq!(provenance.model_identity, "tiny-local-model");
    assert_eq!(provenance.harness_revision, "lite.harness.local_model@v1");
    assert_eq!(
        provenance.model_response_digest,
        sha256_hex(
            run.attempts[0]
                .model_response_raw
                .as_ref()
                .unwrap()
                .as_bytes()
        )
    );
    assert_eq!(
        envelope.repair.changed_files,
        vec!["src/bucket.rs".to_string()]
    );
    assert_eq!(
        run.classification,
        ScenarioOutcomeClass::GovernedProductOutcome
    );
    assert!(run.capability_effects >= 1);
}

#[test]
fn offline_canned_attempted_edits_are_preserved_across_attempts() {
    let run = run_canned(
        OFFLINE_DESCRIPTOR,
        vec![
            canned_chat_completion(
                "tiny-local-model",
                "{\"edits\":[{\"file\":\"src/bucket.rs\",\"search\":\"NO SUCH TEXT\",\"replace\":\"x\"}]}",
                true,
            ),
            canned_chat_completion("tiny-local-model", &reference_proposal(), true),
        ],
    );
    assert_eq!(run.attempts.len(), 2);
    assert_eq!(
        run.attempts[0]
            .failure
            .as_deref()
            .map(|f| f.contains("preflight")),
        Some(true)
    );
    assert!(!run.attempts[0].applied);
    assert!(run.attempts[1].applied);
    assert!(run.envelope.is_some());
}

#[test]
fn offline_canned_model_attempting_forbidden_file_is_rejected() {
    let outside = json!({
        "edits": [{ "file": "tests/rate_limit.rs", "search": "a", "replace": "b" }]
    })
    .to_string();
    let run = run_canned(
        OFFLINE_DESCRIPTOR,
        vec![canned_chat_completion("tiny-local-model", &outside, true)],
    );
    // The scripted response is rejected; the canned fallback produces
    // non-JSON junk for the remaining attempts, so nothing ever applies.
    assert_eq!(run.attempts.len(), MAX_ATTEMPTS as usize);
    assert!(!run.attempts[0].applied);
    assert!(
        run.attempts[0]
            .failure
            .as_deref()
            .map(|f| f.contains("outside the authorized editable set"))
            .unwrap_or(false),
        "attempt failure must name the editable-set refusal: {:?}",
        run.attempts[0].failure
    );
    // Zero filesystem effects: the worktree was never mutated.
    assert_eq!(
        run.changed_vs_baseline,
        Vec::<String>::new(),
        "no file may change when the proposal targets a forbidden path"
    );
    assert!(run.envelope.is_none());
    assert!(!run.patched);
    assert_eq!(run.capability_effects, MAX_ATTEMPTS as u64);
}

#[test]
fn offline_canned_model_attempting_src_lib_rs_is_rejected() {
    let outside = json!({
        "edits": [{ "file": "src/lib.rs", "search": "//!", "replace": "//! x" }]
    })
    .to_string();
    let run = run_canned(
        OFFLINE_DESCRIPTOR,
        vec![canned_chat_completion("tiny-local-model", &outside, true)],
    );
    assert!(!run.attempts[0].applied);
    assert_eq!(run.changed_vs_baseline, Vec::<String>::new());
    assert!(run.envelope.is_none());
}

#[test]
fn offline_malformed_model_json_is_rejected_without_effects() {
    let run = run_canned(
        OFFLINE_DESCRIPTOR,
        vec![
            canned_chat_completion("tiny-local-model", "I would rather not say", true);
            MAX_ATTEMPTS as usize
        ],
    );
    assert_eq!(run.attempts.len(), MAX_ATTEMPTS as usize);
    assert!(run.attempts.iter().all(|a| !a.applied));
    assert_eq!(run.changed_vs_baseline, Vec::<String>::new());
    assert!(run.envelope.is_none());
    assert_eq!(run.capability_effects, MAX_ATTEMPTS as u64);
}

#[test]
fn offline_model_proposal_for_nonexistent_search_text_applies_nothing() {
    let proposal = json!({
        "edits": [{ "file": "src/bucket.rs", "search": "fn does_not_exist_anywhere(", "replace": "fn x(" }]
    })
    .to_string();
    let run = run_canned(
        OFFLINE_DESCRIPTOR,
        vec![canned_chat_completion("tiny-local-model", &proposal, true); MAX_ATTEMPTS as usize],
    );
    assert!(run.attempts.iter().all(|a| !a.applied));
    assert_eq!(run.changed_vs_baseline, Vec::<String>::new());
    assert!(run.envelope.is_none());
}

#[test]
fn offline_interrupted_dispatch_preserves_failure_evidence_and_applies_nothing() {
    // A loopback server that accepts, reads, and never answers; the
    // governed dispatch carries a 200 ms timeout, so the effect fails
    // with a typed timeout and no repair is applied.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let effects = Arc::new(AtomicU64::new(0));
    let effects_thread = effects.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(mut stream) => {
                    effects_thread.fetch_add(1, Ordering::SeqCst);
                    let mut buf = [0u8; 8192];
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let mut total = 0usize;
                    loop {
                        match stream.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                total += n;
                                if buf[..total].windows(4).any(|w| w == b"\r\n\r\n") {
                                    break;
                                }
                                if total > 64 * 1024 {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    // Never respond; park briefly so the client's deadline fires.
                    thread::sleep(Duration::from_secs(3));
                }
                Err(_) => break,
            }
        }
    });
    let mut spawn = |_attempt: u32| PilotEndpoint {
        endpoint: format!("http://127.0.0.1:{addr}/v1", addr = addr.port()),
        effects: effects.clone(),
    };
    let started = Instant::now();
    let run = run_model_pilot(
        OFFLINE_DESCRIPTOR,
        PILOT_EXAMPLE_PRICE_BOOK,
        200,
        RouteProfile::Standard,
        TaskRepairProvenanceKind::CannedTest,
        &mut spawn,
    );
    let _ = started.elapsed();
    assert_eq!(run.attempts.len(), 1);
    assert!(!run.attempts[0].applied);
    let failure = run.attempts[0]
        .failure
        .as_deref()
        .expect("typed failure preserved");
    assert!(
        failure.contains("governed effect failed") || failure.contains("timeout"),
        "failure must be typed: {failure}"
    );
    assert_eq!(run.changed_vs_baseline, Vec::<String>::new());
    assert_eq!(
        run.repaired_file_contents.as_deref(),
        Some(include_str!(
            "fixtures/scenarios/rust-tokenbucket-bugfix/repo/src/bucket.rs"
        )),
        "the interrupted run must leave the source file byte-identical"
    );
    assert!(run.envelope.is_none());
    assert_eq!(run.capability_effects, 1);
}

#[test]
fn offline_stale_revision_refusal_happens_before_any_effect() {
    let stale = OFFLINE_DESCRIPTOR.replace(
        "a20621ef7c5c432d4f98e6217fdc708c4426ef65",
        "0000000000000000000000000000000000000000000000000000000000000000",
    );
    let mut spawn = |_attempt: u32| PilotEndpoint {
        endpoint: "http://127.0.0.1:1/v1".to_string(),
        effects: Arc::new(AtomicU64::new(0)),
    };
    let run = run_model_pilot(
        &stale,
        PILOT_EXAMPLE_PRICE_BOOK,
        5_000,
        RouteProfile::Standard,
        TaskRepairProvenanceKind::CannedTest,
        &mut spawn,
    );
    assert_eq!(run.classification, ScenarioOutcomeClass::SetupError);
    assert!(run.stage_error.is_some() || run.acquire_error.is_some());
    assert_eq!(run.capability_effects, 0);
    assert!(run.envelope.is_none());
    assert!(run.attempts.is_empty());
}

#[test]
fn offline_duplicate_repair_application_is_refused_without_effect() {
    // Apply the reference search-replace once through the same patch
    // boundary the pilot uses, then attempt the identical edit again on
    // the repaired bytes: the dry-run must fail and the file must not
    // change a second time.
    use common::task_execution::{repair_patch_digest, stage_source_repository};
    use prometheos_lite::harness::edit_protocol::{EditOperation, SearchReplaceEdit};
    use prometheos_lite::harness::file_control::{FilePolicy, FileSet};
    use prometheos_lite::harness::patch_applier::apply_patch_with_rollback;

    let temp = tempfile::tempdir().expect("temp");
    let repo = temp.path().join("repo");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/scenarios/rust-tokenbucket-bugfix/repo");
    stage_source_repository(&fixture, &repo, "https://repo.invalid/duplicate-check.git")
        .expect("stage");

    let edits = vec![EditOperation::SearchReplace(SearchReplaceEdit {
        file: PathBuf::from("src/bucket.rs"),
        search: "        self.last_refill = now_millis;".to_string(),
        replace: "        self.last_refill += ticks * self.tick_millis;".to_string(),
        replace_all: None,
        context_lines: None,
    })];
    let mut policy = FilePolicy::strict();
    policy.repo_root = repo.clone();
    policy.allowed_write_paths = vec![PathBuf::from("src/bucket.rs")];
    let file_set = FileSet {
        editable: vec![repo.join("src/bucket.rs")],
        readonly: vec![],
        generated: vec![],
        artifacts: vec![],
        denied: vec![],
        binary: vec![],
    };
    let first = common::task_execution::dispatch_resolved_route_block_on(
        apply_patch_with_rollback(&edits, &file_set, &policy),
    )
    .expect("first apply must succeed");
    assert!(first.0.applied);
    let after_first = sha256_hex(&std::fs::read(repo.join("src/bucket.rs")).unwrap());

    let second = common::task_execution::dispatch_resolved_route_block_on(
        apply_patch_with_rollback(&edits, &file_set, &policy),
    )
    .expect("duplicate apply must not error the runtime");
    assert!(!second.0.applied, "a duplicate attempt must not reapply");
    assert!(!second.0.failures.is_empty());
    let after_second = sha256_hex(&std::fs::read(repo.join("src/bucket.rs")).unwrap());
    assert_eq!(
        after_first, after_second,
        "duplicate attempt must not mutate the file"
    );
    let _ = repair_patch_digest; // digest helper exercised elsewhere
}

#[test]
fn offline_usage_missing_from_model_response_is_unknown_never_zero() {
    let run = run_canned(
        OFFLINE_DESCRIPTOR,
        vec![canned_chat_completion(
            "tiny-local-model",
            &reference_proposal(),
            false,
        )],
    );
    let envelope = run.envelope.as_ref().expect("envelope sealed");
    let usage = serde_json::to_value(&envelope.scenario_run.attempt_accounting.usage)
        .expect("usage serializes");
    assert_eq!(
        usage["state"], "unknown",
        "usage must be Unknown, got {usage}"
    );
    let cost = serde_json::to_value(&envelope.scenario_run.attempt_accounting.cost)
        .expect("cost serializes");
    assert_eq!(cost["state"], "unknown", "cost must be Unknown, got {cost}");
}

#[test]
fn offline_pricing_missing_from_price_book_is_unknown_never_zero() {
    // A price book with no entry for the fixture model and provider.
    let empty_book = r#"{
        "schemaVersion": "1.0.0",
        "source": "example-operator-price-book",
        "version": "example-2026-11",
        "entries": [
            { "provider": "someone-else", "model": "other-model", "currency": "USD",
              "promptMinorPer1k": 1, "completionMinorPer1k": 1 }
        ]
    }"#;
    let run = run_pilot_with_canned(
        OFFLINE_DESCRIPTOR,
        empty_book,
        5_000,
        vec![canned_chat_completion(
            "tiny-local-model",
            &reference_proposal(),
            true,
        )],
    );
    let envelope = run.envelope.as_ref().expect("envelope sealed");
    let cost = serde_json::to_value(&envelope.scenario_run.attempt_accounting.cost)
        .expect("cost serializes");
    assert_eq!(
        cost["state"], "unknown",
        "cost must be Unknown without a price entry, got {cost}"
    );
}

#[test]
fn offline_solver_visible_corpus_contains_no_assessor_only_material() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/scenarios/rust-tokenbucket-bugfix/repo");
    let files = solver_visible_files(&fixture);
    let descriptor =
        prometheos_lite::workflow::scenario::ScenarioDescriptor::parse_json(OFFLINE_DESCRIPTOR)
            .expect("descriptor parses");
    let failure_signal =
        "cargo test --offline failed: test refill_accumulates_sub_window_remainder FAILED";
    let messages = build_solver_messages(&descriptor, &files, failure_signal, None);
    let corpus = serde_json::to_string(&messages).expect("messages serialize");
    // Assessor-only material that must never appear solver-side:
    for forbidden in [
        "4873f68151f318d1f955ad2a78bf8572fccc44db024d7c45672e76288da67b7d",
        "1ea6e33704a57af0fe9e77400be45617ad46b2b149c681d37ad02c7421492b98",
        "5203dab9ffbbc48a56f75769fbad1db9f630c9d293d60dfed5713e76328ae97a",
        "expectedPostRunDigest",
        "expectedFileAfterDigest",
        "self.last_refill += ticks",
        "self.last_refill += self",
        "ticks * self.tick_millis",
    ] {
        assert!(
            !corpus.contains(forbidden),
            "solver-visible corpus contains assessor-only material: {forbidden}"
        );
    }
    // ... while the public failure signal and task statement are present.
    assert!(corpus.contains("refill_accumulates_sub_window_remainder"));
    assert!(corpus.contains("src/bucket.rs"));
}

#[test]
fn offline_resealed_false_success_conflicting_with_trusted_pins_is_rejected() {
    let run = run_canned(
        OFFLINE_DESCRIPTOR,
        vec![canned_chat_completion(
            "tiny-local-model",
            &reference_proposal(),
            true,
        )],
    );
    let envelope = run.envelope.expect("envelope sealed");

    // 1. Tampered validator exit code.
    let mut tampered = envelope.clone();
    tampered.validator.exit_code = Some(1);
    assert!(
        tampered.verify().is_err(),
        "tampered validator exit code must fail verify"
    );

    // 2. Tampered proposal provenance digest.
    let mut tampered = envelope.clone();
    tampered
        .repair
        .model_provenance
        .as_mut()
        .expect("provenance")
        .proposal_digest = "00".repeat(32);
    assert!(
        tampered.verify().is_err(),
        "tampered provenance binding must fail verify"
    );

    // 3. Model-authored repair relabelled as scripted (provenance kept):
    //    the scripted path may not carry model provenance.
    let mut tampered = envelope.clone();
    tampered.repair.origin = TaskRepairOrigin::ScriptedFixture;
    assert!(
        tampered.verify().is_err(),
        "relabelled origin must fail verify"
    );

    // 4. Model-authored provenance stripped.
    let mut tampered = envelope.clone();
    tampered.repair.model_provenance = None;
    assert!(
        tampered.verify().is_err(),
        "stripped provenance must fail verify"
    );

    // 5. Baseline digest conflicting with the pinned fixture.
    let mut tampered = envelope.clone();
    tampered.baseline_content_digest = "ff".repeat(32);
    assert!(
        tampered.verify().is_err(),
        "baseline conflict with trusted pin must fail verify"
    );
}

#[test]
fn offline_failed_dispatch_applies_no_repair_and_seals_nothing() {
    // The governed effect never succeeds (endpoint unreachable): typed
    // failure, zero writes, no envelope.
    let mut spawn = |_attempt: u32| PilotEndpoint {
        endpoint: "http://127.0.0.1:1/v1".to_string(),
        effects: Arc::new(AtomicU64::new(0)),
    };
    let run = run_model_pilot(
        OFFLINE_DESCRIPTOR,
        PILOT_EXAMPLE_PRICE_BOOK,
        2_000,
        RouteProfile::Standard,
        TaskRepairProvenanceKind::CannedTest,
        &mut spawn,
    );
    assert!(!run.patched);
    assert!(run.envelope.is_none());
    assert_eq!(run.changed_vs_baseline, Vec::<String>::new());
    assert_eq!(run.attempts.len(), 1);
    let failure = run.attempts[0].failure.as_deref().expect("typed failure");
    assert!(
        failure.contains("governed effect failed"),
        "typed failure: {failure}"
    );
}

// ---------------------------------------------------------------------------
// Trust-boundary / adversarial coverage (2026-11-16 repair)
// ---------------------------------------------------------------------------
//
// The live pilot forwards repository source to a cloud provider through a
// loopback operator gateway. These tests pin the authorization boundary:
// the upstream's real locality is represented, egress is explicitly
// authorized for the synthetic fixture only, and every outbound effect is
// decided before any upstream byte leaves the process.

/// Parsed and validated upstream destination. Scheme, user information,
/// fragments, host and port are all checked before this `Result` is given
/// to any caller.
struct ParsedUpstream {
    /// Lowercased host (no port, no user information).
    host: String,
}

/// Validate an upstream URL strictly, with `reqwest::Url` parsing.
///
/// Refuses malformed or ambiguous authorities, user information,
/// fragments, unexpected ports and unsupported or insecure schemes.
/// Non-loopback destinations must use HTTPS on the default port; plain
/// HTTP is tolerated only for the restricted loopback exception (127.0.0.1,
/// localhost, ::1) used by the local mock tests.
fn parse_upstream_url(url: &str) -> Result<ParsedUpstream, &'static str> {
    let parsed = reqwest::Url::parse(url).map_err(|_| "upstream URL unparseable")?;
    // Ambiguous or smuggled authority material is never acceptable.
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("upstream URL must not contain user information");
    }
    if parsed.fragment().is_some() {
        return Err("upstream URL must not contain a fragment");
    }
    let host = parsed
        .host_str()
        .map(|h| h.to_ascii_lowercase())
        .filter(|h| !h.is_empty())
        .ok_or("upstream URL has no host")?;
    let is_loopback = host == "127.0.0.1" || host == "localhost" || host == "[::1]";
    match parsed.scheme() {
        "https" => {
            if !is_loopback && parsed.port().is_some() {
                // reqwest normalizes the default port (443) away, so any
                // explicit port here is unexpected.
                return Err("unexpected port for HTTPS upstream; the default port is required");
            }
        }
        "http" => {
            if !is_loopback {
                return Err(
                    "plain HTTP egress is refused; HTTPS is required for non-loopback upstreams",
                );
            }
        }
        _ => return Err("unsupported upstream URL scheme; HTTPS is required"),
    }
    Ok(ParsedUpstream { host })
}

fn upstream_host(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(|h| h.to_ascii_lowercase()))
        .unwrap_or_default()
}

/// Mock upstream that records how many requests it received.
fn spawn_mock_upstream(response_body: String) -> (String, Arc<AtomicU64>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("mock upstream binds");
    let addr = listener.local_addr().expect("mock upstream addr");
    let url = format!("http://127.0.0.1:{}/v1/chat/completions", addr.port());
    let hits = Arc::new(AtomicU64::new(0));
    let hits_thread = hits.clone();
    let body_bytes = response_body.into_bytes();
    thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(mut stream) => {
                    hits_thread.fetch_add(1, Ordering::SeqCst);
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
                    let mut buf = Vec::with_capacity(8192);
                    let mut chunk = [0u8; 8192];
                    let mut content_length = 0usize;
                    let mut header_end: Option<usize> = None;
                    loop {
                        match stream.read(&mut chunk) {
                            Ok(0) => break,
                            Ok(n) => {
                                buf.extend_from_slice(&chunk[..n]);
                                if header_end.is_none()
                                    && let Some(pos) = find_subslice(&buf, b"\r\n\r\n")
                                {
                                    header_end = Some(pos + 4);
                                    let headers =
                                        String::from_utf8_lossy(&buf[..pos]).to_lowercase();
                                    for line in headers.lines() {
                                        if let Some(value) = line.strip_prefix("content-length:") {
                                            content_length = value.trim().parse().unwrap_or(0);
                                        }
                                    }
                                }
                                if let Some(end) = header_end
                                    && buf.len() >= end + content_length
                                {
                                    break;
                                }
                                if buf.len() > 256 * 1024 {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body_bytes.len(),
                        String::from_utf8_lossy(&body_bytes)
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.shutdown(std::net::Shutdown::Both);
                }
                Err(_) => break,
            }
        }
    });
    (url, hits)
}

/// Mock upstream that replies with a canned 3xx redirect and records
/// how many requests it received. Used to prove that redirects from an
/// allowed upstream are never followed to another host.
fn spawn_redirect_mock_upstream(status_line: &str, location: String) -> (String, Arc<AtomicU64>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("redirect mock binds");
    let addr = listener.local_addr().expect("redirect mock addr");
    let url = format!("http://127.0.0.1:{}/v1/chat/completions", addr.port());
    let hits = Arc::new(AtomicU64::new(0));
    let hits_thread = hits.clone();
    let status_line = status_line.to_string();
    thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(mut stream) => {
                    hits_thread.fetch_add(1, Ordering::SeqCst);
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
                    let mut buf = Vec::with_capacity(8192);
                    let mut chunk = [0u8; 8192];
                    let mut content_length = 0usize;
                    let mut header_end: Option<usize> = None;
                    loop {
                        match stream.read(&mut chunk) {
                            Ok(0) => break,
                            Ok(n) => {
                                buf.extend_from_slice(&chunk[..n]);
                                if header_end.is_none()
                                    && let Some(pos) = find_subslice(&buf, b"\r\n\r\n")
                                {
                                    header_end = Some(pos + 4);
                                    let headers =
                                        String::from_utf8_lossy(&buf[..pos]).to_lowercase();
                                    for line in headers.lines() {
                                        if let Some(value) = line.strip_prefix("content-length:") {
                                            content_length = value.trim().parse().unwrap_or(0);
                                        }
                                    }
                                }
                                if let Some(end) = header_end
                                    && buf.len() >= end + content_length
                                {
                                    break;
                                }
                                if buf.len() > 256 * 1024 {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    let body = "{}";
                    let response = format!(
                        "HTTP/1.1 {status_line}\r\nLocation: {location}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.shutdown(std::net::Shutdown::Both);
                }
                Err(_) => break,
            }
        }
    });
    (url, hits)
}

/// Per-run grant that AUTHORIZES this synthetic fixture egress to the
/// named host only (deny + allowlist containing exactly that host).
fn allow_remote_grant(
    upstream_url: String,
    host: String,
    known_secrets: Vec<String>,
) -> GatewayEgressGrant {
    GatewayEgressGrant {
        upstream_url,
        effective_network_policy: NetworkPolicy {
            default: NetworkDefault::Deny,
            allowlist: Some(vec![host]),
        },
        permit_synthetic_source_egress: true,
        provider_identity: "openrouter-via-loopback-gateway".to_string(),
        upstream_model: "nvidia/nemotron-3-ultra-550b-a55b:free".to_string(),
        harness_visible_model: "nemotron-ultra-550b-free".to_string(),
        known_secrets,
        would_dial: Arc::new(AtomicU64::new(0)),
        refused: Arc::new(AtomicU64::new(0)),
    }
}

/// Grant that authorizes NOTHING: effective policy denies with no
/// allowlist, and no synthetic-source egress is authorized.
fn deny_remote_grant(upstream_url: String, _host: String) -> GatewayEgressGrant {
    GatewayEgressGrant {
        upstream_url,
        effective_network_policy: NetworkPolicy {
            default: NetworkDefault::Deny,
            allowlist: None,
        },
        permit_synthetic_source_egress: false,
        provider_identity: "openrouter-via-loopback-gateway".to_string(),
        upstream_model: "nvidia/nemotron-3-ultra-550b-a55b:free".to_string(),
        harness_visible_model: "nemotron-ultra-550b-free".to_string(),
        known_secrets: vec![],
        would_dial: Arc::new(AtomicU64::new(0)),
        refused: Arc::new(AtomicU64::new(0)),
    }
}

#[test]
fn remote_provider_behind_localhost_cannot_satisfy_local_only_policy() {
    // Route resolution with a conservative (local-only) profile must
    // refuse the remote-backed route before any effect; the loopback URL
    // does not launder the provider's cloud locality.
    let mut spawn = |_attempt: u32| -> PilotEndpoint {
        panic!("dispatch must not be reached when route resolution refuses")
    };
    let run = run_model_pilot(
        LIVE_DESCRIPTOR,
        PILOT_EXAMPLE_PRICE_BOOK,
        5_000,
        RouteProfile::Conservative,
        TaskRepairProvenanceKind::OperatorObservedLive,
        &mut spawn,
    );
    assert_eq!(run.classification, ScenarioOutcomeClass::SetupError);
    assert_eq!(run.capability_effects, 0);
    assert!(run.attempts.is_empty());
    assert_eq!(run.changed_vs_baseline, Vec::<String>::new());
}

#[test]
fn network_deny_policy_refuses_remote_egress_before_any_upstream_bytes() {
    // The upstream records hits: the denial must happen BEFORE dialing.
    let (upstream_url, hits) = spawn_mock_upstream(canned_chat_completion(
        "nemotron-ultra-550b-free",
        &reference_proposal(),
        true,
    ));
    let host = upstream_host(&upstream_url);
    let grant = deny_remote_grant(upstream_url, host);
    let (endpoint, effects, refused, would_dial, stop) = spawn_openrouter_gateway(grant);
    let mut spawn = |_attempt: u32| PilotEndpoint {
        endpoint: endpoint.clone(),
        effects: effects.clone(),
    };
    let run = run_model_pilot(
        LIVE_DESCRIPTOR,
        PILOT_EXAMPLE_PRICE_BOOK,
        5_000,
        RouteProfile::Standard,
        TaskRepairProvenanceKind::OperatorObservedLive,
        &mut spawn,
    );
    stop.store(true, Ordering::SeqCst);
    assert_eq!(run.attempts.len(), 1);
    let failure = run.attempts[0].failure.as_deref().expect("typed failure");
    assert!(
        failure.contains("governed effect failed"),
        "typed failure: {failure}"
    );
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "upstream must receive zero requests"
    );
    assert_eq!(
        would_dial.load(Ordering::SeqCst),
        0,
        "egress refused before dialing"
    );
    assert_eq!(
        refused.load(Ordering::SeqCst),
        1,
        "gateway must record the refusal"
    );
    assert_eq!(run.changed_vs_baseline, Vec::<String>::new());
    assert!(!run.patched);
    assert!(run.envelope.is_none());
}

#[test]
fn authorized_cloud_backed_synthetic_task_can_dispatch() {
    // The same remote route with an explicit per-run authorization for
    // the synthetic fixture source dispatches, applies and seals.
    let (upstream_url, hits) = spawn_mock_upstream(canned_chat_completion(
        "nemotron-ultra-550b-free",
        &reference_proposal(),
        true,
    ));
    let host = upstream_host(&upstream_url);
    let grant = allow_remote_grant(upstream_url, host, vec![]);
    let (endpoint, effects, refused, would_dial, stop) = spawn_openrouter_gateway(grant);
    let mut spawn = |_attempt: u32| PilotEndpoint {
        endpoint: endpoint.clone(),
        effects: effects.clone(),
    };
    let run = run_model_pilot(
        LIVE_DESCRIPTOR,
        PILOT_EXAMPLE_PRICE_BOOK,
        10_000,
        RouteProfile::Standard,
        TaskRepairProvenanceKind::OperatorObservedLive,
        &mut spawn,
    );
    stop.store(true, Ordering::SeqCst);
    assert_eq!(run.attempts.len(), 1);
    assert!(
        run.patched,
        "authorized cloud-backed synthetic task must dispatch"
    );
    assert_eq!(would_dial.load(Ordering::SeqCst), 1);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert_eq!(refused.load(Ordering::SeqCst), 0);
    let envelope = run.envelope.as_ref().expect("envelope sealed");
    let provenance = envelope
        .repair
        .model_provenance
        .as_ref()
        .expect("provenance bound");
    assert_eq!(
        provenance.kind,
        Some(TaskRepairProvenanceKind::OperatorObservedLive)
    );
    assert_eq!(provenance.model_identity, "nemotron-ultra-550b-free");
    envelope
        .verify()
        .expect("live-path envelope verifies end-to-end");
}

#[test]
fn resealed_provenance_identity_swaps_are_rejected() {
    let run = run_canned(
        OFFLINE_DESCRIPTOR,
        vec![canned_chat_completion(
            "tiny-local-model",
            &reference_proposal(),
            true,
        )],
    );
    let envelope = run.envelope.expect("envelope sealed");

    let mut tampered = envelope.clone();
    tampered
        .repair
        .model_provenance
        .as_mut()
        .expect("provenance")
        .model_identity = "wrong-provider/model".to_string();
    assert!(
        tampered.verify().is_err(),
        "wrong provider/model identity must fail verify"
    );

    let mut tampered = envelope.clone();
    tampered
        .repair
        .model_provenance
        .as_mut()
        .expect("provenance")
        .harness_revision = "lite.harness.local_model@v2".to_string();
    assert!(
        tampered.verify().is_err(),
        "wrong harness revision must fail verify"
    );

    let mut tampered = envelope.clone();
    tampered
        .repair
        .model_provenance
        .as_mut()
        .expect("provenance")
        .attempt_idempotency_key = Some("not-the-dispatched-attempt".to_string());
    assert!(
        tampered.verify().is_err(),
        "wrong winning-attempt identity must fail verify"
    );

    // Fabricated raw model-response digest (valid hex, wrong bytes).
    let mut tampered = envelope.clone();
    tampered
        .repair
        .model_provenance
        .as_mut()
        .expect("provenance")
        .raw_response = Some("different bytes entirely".to_string());
    assert!(
        tampered.verify().is_err(),
        "raw response bytes not hashing to the recorded digest must fail verify"
    );

    // Stripped and swapped canned/live kinds.
    let mut tampered = envelope.clone();
    tampered
        .repair
        .model_provenance
        .as_mut()
        .expect("provenance")
        .kind = None;
    assert!(
        tampered.verify().is_err(),
        "absent provenance kind must fail verify"
    );
    let mut tampered = envelope;
    tampered
        .repair
        .model_provenance
        .as_mut()
        .expect("provenance")
        .kind = Some(TaskRepairProvenanceKind::OperatorObservedLive);
    assert!(
        tampered.verify().is_err(),
        "a canned envelope re-labelled as live must fail verify"
    );
}

#[test]
fn offline_canned_envelope_is_never_operator_observed_live() {
    let run = run_canned(
        OFFLINE_DESCRIPTOR,
        vec![canned_chat_completion(
            "tiny-local-model",
            &reference_proposal(),
            true,
        )],
    );
    let envelope = run.envelope.expect("envelope sealed");
    assert_eq!(
        envelope.repair.model_provenance.as_ref().unwrap().kind,
        Some(TaskRepairProvenanceKind::CannedTest),
        "offline canned responder must never seal as operator-observed-live"
    );
}

#[test]
fn egress_preflight_refuses_secret_bearing_outbound_messages() {
    let upstream = "http://127.0.0.1:1/v1/chat/completions".to_string();
    let grant = allow_remote_grant(
        upstream,
        "127.0.0.1".to_string(),
        vec!["CANARY-SECRET-9f8a7b6c".to_string()],
    );
    let with_secret = br#"{"messages":[{"role":"user","content":"leak CANARY-SECRET-9f8a7b6c"}]}"#;
    assert!(egress_preflight(&grant, with_secret).is_err());
    let clean = br#"{"messages":[{"role":"user","content":"public fixture source only"}]}"#;
    assert!(egress_preflight(&grant, clean).is_ok());
}

#[test]
fn egress_preflight_refuses_without_authorization_and_without_allowlist() {
    let upstream = "http://127.0.0.1:1/v1/chat/completions".to_string();
    // Explicit permit, allowlisted host: allowed.
    let allowed = allow_remote_grant(upstream.clone(), "127.0.0.1".to_string(), vec![]);
    assert!(egress_preflight(&allowed, br#"{"messages":[]}"#).is_ok());
    // Not authorized for this synthetic source: refused even on allowlist.
    let mut denied = allow_remote_grant(upstream.clone(), "127.0.0.1".to_string(), vec![]);
    denied.permit_synthetic_source_egress = false;
    assert!(egress_preflight(&denied, br#"{"messages":[]}"#).is_err());
    // No allowlist in the effective policy: refused.
    let denied = deny_remote_grant(upstream, "127.0.0.1".to_string());
    assert!(egress_preflight(&denied, br#"{"messages":[]}"#).is_err());
}

#[test]
fn egress_preflight_refuses_plain_http_to_non_loopback_upstream() {
    // http is refused for non-loopback hosts even when authorized+allowlisted.
    let upstream = "http://example.com/v1/chat/completions".to_string();
    let grant = allow_remote_grant(upstream, "example.com".to_string(), vec![]);
    assert!(egress_preflight(&grant, br#"{"messages":[]}"#).is_err());
    // And an allowlist missing the scheme-matched host refuses the dial.
    let upstream = "https://example.com/v1/chat/completions".to_string();
    let grant = allow_remote_grant(upstream, "other.example.com".to_string(), vec![]);
    assert!(egress_preflight(&grant, br#"{"messages":[]}"#).is_err());
}

#[test]
fn egress_preflight_refuses_userinfo_hostname_confusion() {
    // `https://openrouter.ai:443@attacker.invalid/path`: a naive
    // string-split authority says "openrouter.ai", but the real authority
    // host is attacker.invalid with user information "openrouter.ai:443".
    // Both the user-information rule and the allowlist must refuse it.
    let spoofed = "https://openrouter.ai:443@attacker.invalid/path".to_string();
    let grant = allow_remote_grant(spoofed, "openrouter.ai".to_string(), vec![]);
    assert!(egress_preflight(&grant, br#"{"messages":[]}"#).is_err());
    // Even when the true host is allowlisted, user information is refused.
    let spoofed = "https://openrouter.ai@attacker.invalid/path".to_string();
    let grant = allow_remote_grant(spoofed, "attacker.invalid".to_string(), vec![]);
    assert!(egress_preflight(&grant, br#"{"messages":[]}"#).is_err());
    // Password-bearing user information is refused too.
    let spoofed = "https://user:pass@openrouter.ai/api/v1/chat/completions".to_string();
    let grant = allow_remote_grant(spoofed, "openrouter.ai".to_string(), vec![]);
    assert!(egress_preflight(&grant, br#"{"messages":[]}"#).is_err());
    // Fragment smuggling behind an authorized-looking host is refused.
    let spoofed = "https://openrouter.ai/api/v1/chat/completions#@attacker.invalid".to_string();
    let grant = allow_remote_grant(spoofed, "openrouter.ai".to_string(), vec![]);
    assert!(egress_preflight(&grant, br#"{"messages":[]}"#).is_err());
    // Malformed authorities are refused, not guessed at.
    let grant = allow_remote_grant("https://".to_string(), "openrouter.ai".to_string(), vec![]);
    assert!(egress_preflight(&grant, br#"{"messages":[]}"#).is_err());
    let grant = allow_remote_grant("not-a-url".to_string(), "openrouter.ai".to_string(), vec![]);
    assert!(egress_preflight(&grant, br#"{"messages":[]}"#).is_err());
}

#[test]
fn egress_preflight_refuses_unexpected_port_and_destination() {
    // Non-default explicit port on an HTTPS destination: refused even
    // when the host is allowlisted and authorized.
    let upstream = "https://openrouter.ai:8443/api/v1/chat/completions".to_string();
    let grant = allow_remote_grant(upstream, "openrouter.ai".to_string(), vec![]);
    assert!(egress_preflight(&grant, br#"{"messages":[]}"#).is_err());
    // The explicit default HTTPS port is equivalent to none and allowed.
    let upstream = "https://openrouter.ai:443/api/v1/chat/completions".to_string();
    let grant = allow_remote_grant(upstream, "openrouter.ai".to_string(), vec![]);
    assert!(egress_preflight(&grant, br#"{"messages":[]}"#).is_ok());
    // An allowlist entry that does not match the real authority is refused.
    let upstream = "https://other.example.com/api/v1/chat/completions".to_string();
    let grant = allow_remote_grant(upstream, "openrouter.ai".to_string(), vec![]);
    assert!(egress_preflight(&grant, br#"{"messages":[]}"#).is_err());
    // The explicitly authorized live HTTPS destination is accepted.
    let upstream = "https://openrouter.ai/api/v1/chat/completions".to_string();
    let grant = allow_remote_grant(upstream, "openrouter.ai".to_string(), vec![]);
    assert!(egress_preflight(&grant, br#"{"messages":[]}"#).is_ok());
}

#[test]
fn egress_preflight_refuses_unsupported_and_insecure_schemes() {
    for upstream in [
        "ftp://openrouter.ai/api/v1/chat/completions",
        "file:///etc/passwd",
        "wss://openrouter.ai/api/v1/chat/completions",
        "gopher://openrouter.ai/",
        "http://openrouter.ai/api/v1/chat/completions",
    ] {
        let grant = allow_remote_grant(upstream.to_string(), "openrouter.ai".to_string(), vec![]);
        assert!(
            egress_preflight(&grant, br#"{"messages":[]}"#).is_err(),
            "scheme must be refused: {upstream}"
        );
    }
    // The restricted loopback exception still allows local mock tests.
    let upstream = "http://127.0.0.1:9/v1/chat/completions".to_string();
    let grant = allow_remote_grant(upstream, "127.0.0.1".to_string(), vec![]);
    assert!(egress_preflight(&grant, br#"{"messages":[]}"#).is_ok());
    let upstream = "http://localhost:9/v1/chat/completions".to_string();
    let grant = allow_remote_grant(upstream, "localhost".to_string(), vec![]);
    assert!(egress_preflight(&grant, br#"{"messages":[]}"#).is_ok());
}

#[test]
fn gateway_refuses_secret_bearing_outbound_payload_before_any_dial() {
    // The check runs against the serialized payload that will actually be
    // transmitted (inbound messages + upstream model/reasoning rewrite),
    // not merely against the inbound gateway body.
    let secret = "CANARY-SECRET-9f8a7b6c";
    let (upstream_url, hits) = spawn_mock_upstream(canned_chat_completion(
        "nemotron-ultra-550b-free",
        &reference_proposal(),
        true,
    ));
    let host = upstream_host(&upstream_url);
    let grant = allow_remote_grant(upstream_url, host, vec![secret.to_string()]);
    let (endpoint, _effects, refused, would_dial, stop) = spawn_openrouter_gateway(grant);
    let gateway_addr = endpoint
        .strip_prefix("http://")
        .and_then(|rest| rest.split('/').next())
        .expect("gateway host:port")
        .to_string();
    let body = json!({
        "messages": [{"role": "user", "content": format!("leak {secret}")}]
    })
    .to_string();
    let request = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: {gateway_addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = std::net::TcpStream::connect(&gateway_addr).expect("connect gateway");
    stream.write_all(request.as_bytes()).expect("write request");
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    stop.store(true, Ordering::SeqCst);
    assert!(
        response.starts_with("HTTP/1.1 403"),
        "gateway must refuse with 403 egress-refused, got: {response}"
    );
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "upstream must receive zero requests"
    );
    assert_eq!(
        would_dial.load(Ordering::SeqCst),
        0,
        "egress refused before dialing"
    );
    assert_eq!(refused.load(Ordering::SeqCst), 1);
}

struct RedirectPilotResult {
    run: ModelPilotRun,
    redirect_target_hits: Arc<AtomicU64>,
    authorized_upstream_hits: Arc<AtomicU64>,
    refused: Arc<AtomicU64>,
    would_dial: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
}

fn run_redirect_pilot(status_line: &str) -> RedirectPilotResult {
    // Redirect target on a different host name (localhost vs 127.0.0.1):
    // a second, unauthorized destination the gateway must never dial.
    let (target_url, target_hits) = spawn_mock_upstream(canned_chat_completion(
        "nemotron-ultra-550b-free",
        &reference_proposal(),
        true,
    ));
    let cross_host_location = target_url.replacen("127.0.0.1", "localhost", 1);
    let (redirect_url, upstream_hits) =
        spawn_redirect_mock_upstream(status_line, cross_host_location);
    let host = upstream_host(&redirect_url);
    let grant = allow_remote_grant(redirect_url, host, vec![]);
    let (endpoint, effects, refused, would_dial, stop) = spawn_openrouter_gateway(grant);
    let mut spawn = |_attempt: u32| PilotEndpoint {
        endpoint: endpoint.clone(),
        effects: effects.clone(),
    };
    let run = run_model_pilot(
        LIVE_DESCRIPTOR,
        PILOT_EXAMPLE_PRICE_BOOK,
        10_000,
        RouteProfile::Standard,
        TaskRepairProvenanceKind::OperatorObservedLive,
        &mut spawn,
    );
    RedirectPilotResult {
        run,
        redirect_target_hits: target_hits,
        authorized_upstream_hits: upstream_hits,
        refused,
        would_dial,
        stop,
    }
}

#[test]
fn cross_host_307_redirect_is_never_followed() {
    let outcome = run_redirect_pilot("307 Temporary Redirect");
    outcome.stop.store(true, Ordering::SeqCst);
    let run = &outcome.run;
    let target_hits = &outcome.redirect_target_hits;
    let upstream_hits = &outcome.authorized_upstream_hits;
    let refused = &outcome.refused;
    let would_dial = &outcome.would_dial;
    assert_eq!(
        refused.load(Ordering::SeqCst),
        0,
        "the authorized upstream itself is not refused"
    );
    assert_eq!(
        target_hits.load(Ordering::SeqCst),
        0,
        "the redirect target must receive zero requests"
    );
    assert_eq!(
        upstream_hits.load(Ordering::SeqCst),
        1,
        "only the authorized upstream is dialed"
    );
    assert_eq!(would_dial.load(Ordering::SeqCst), 1);
    assert!(!run.patched);
    assert!(run.envelope.is_none());
    let failure = run.attempts[0].failure.as_deref().expect("typed failure");
    assert!(
        failure.contains("governed effect failed"),
        "redirect must surface as a non-successful provider response: {failure}"
    );
}

#[test]
fn cross_host_308_redirect_is_never_followed() {
    let outcome = run_redirect_pilot("308 Permanent Redirect");
    outcome.stop.store(true, Ordering::SeqCst);
    let run = &outcome.run;
    let target_hits = &outcome.redirect_target_hits;
    let upstream_hits = &outcome.authorized_upstream_hits;
    let refused = &outcome.refused;
    let would_dial = &outcome.would_dial;
    assert_eq!(
        refused.load(Ordering::SeqCst),
        0,
        "the authorized upstream itself is not refused"
    );
    assert_eq!(
        target_hits.load(Ordering::SeqCst),
        0,
        "the redirect target must receive zero requests"
    );
    assert_eq!(
        upstream_hits.load(Ordering::SeqCst),
        1,
        "only the authorized upstream is dialed"
    );
    assert_eq!(would_dial.load(Ordering::SeqCst), 1);
    assert!(!run.patched);
    assert!(run.envelope.is_none());
    let failure = run.attempts[0].failure.as_deref().expect("typed failure");
    assert!(
        failure.contains("governed effect failed"),
        "redirect must surface as a non-successful provider response: {failure}"
    );
}

// ---------------------------------------------------------------------------
// Committed live-pilot receipt: preserved historical evidence, accurately
// classified; every link recomputable from the committed bytes
// ---------------------------------------------------------------------------

/// The receipt of the real, opt-in OpenRouter pilot run published in
/// `docs/evidence/real-model-pilot-tokenbucket-2026-11-16.json`.
///
/// That run crossed the cloud boundary before the trust-boundary repair
/// added egress authorization to the gateway and to the sealed provenance
/// binding. It is preserved **unchanged** as historical experimental
/// evidence and is explicitly NOT presented as a policy-compliant governed
/// run (see `authorityAssessment` inside the receipt and the guide). The
/// envelope therefore can never pass the strengthened verification again —
/// that is the correct outcome, not a regression.
///
/// What this test proves from the committed bytes alone, with no network,
/// no model and no session state:
///   1. the raw proposal bytes re-hash to the recorded response digest;
///   2. the parsed proposal re-derives the applied patch digest;
///   3. the recorded raw proposal applied to the pinned fixture produces
///      the recorded post-run tree digest and passes the pinned validator
///      (an independent assessor replay, separate from the envelope's
///      internal consistency check);
///   4. the preserved envelope fails the repaired verification, with the
///      provenance binding named.
#[test]
fn historical_live_pilot_receipt_is_consistent_but_not_policy_compliant() {
    let receipt: Value = serde_json::from_str(include_str!(
        "../docs/evidence/real-model-pilot-tokenbucket-2026-11-16.json"
    ))
    .expect("committed receipt parses");
    assert_eq!(receipt["taskOutcome"], "Repaired");
    assert_eq!(receipt["patched"], true);
    assert_eq!(
        receipt["authorityAssessment"],
        "historical-ungoverned-egress"
    );

    // --- Link 1: raw proposal bytes -> response digest ---
    let raw_proposal = receipt["attempts"][0]["proposal"]
        .as_str()
        .expect("raw proposal string recorded");
    let response_digest = sha256_hex(raw_proposal.as_bytes());
    assert_eq!(
        receipt["attempts"][0]["modelResponseDigest"]
            .as_str()
            .unwrap(),
        response_digest
    );

    // --- Link 2: parsed proposal -> patch digest ---
    let edits =
        common::model_pilot::parse_model_proposal(raw_proposal).expect("recorded proposal parses");
    let digest = common::model_pilot::proposal_digest(&edits);
    assert_eq!(
        receipt["attempts"][0]["proposalDigest"].as_str().unwrap(),
        digest
    );

    let envelope: prometheos_lite::workflow::scenario::LiteTaskRunEvidence =
        serde_json::from_value(receipt["envelope"].clone()).expect("committed envelope parses");
    let provenance = envelope
        .repair
        .model_provenance
        .as_ref()
        .expect("model-authored provenance bound");
    assert_eq!(provenance.model_response_digest, response_digest);
    assert_eq!(provenance.proposal_digest, digest);
    assert_eq!(provenance.attempt, 1);
    assert_eq!(envelope.repair.patch_digest, digest);
    assert_eq!(
        envelope.repair.changed_files,
        vec!["src/bucket.rs".to_string()]
    );

    // --- Link 3: independent assessor replay in a fresh workspace ---------
    // Apply the recorded proposal to the pinned fixture bytes and require
    // the pinned validator to pass and the tree digest to equal the
    // recorded post-run digest. This does not rely on the envelope at all.
    let temp = tempfile::tempdir().expect("assessor workspace");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/scenarios/rust-tokenbucket-bugfix/repo");
    common::task_execution::stage_source_repository(
        &fixture,
        &temp.path().join("source"),
        "https://repo.invalid/assessor-replay.git",
    )
    .expect("stage pinned fixture");
    let verifier_workdir = &temp.path().join("source");
    let (policy, file_set) = {
        use prometheos_lite::harness::file_control::{FilePolicy, FileSet};
        let mut policy = FilePolicy::strict();
        policy.repo_root = verifier_workdir.clone();
        policy.allowed_write_paths = vec![PathBuf::from("src/bucket.rs")];
        let file_set = FileSet {
            editable: vec![verifier_workdir.join("src/bucket.rs")],
            readonly: vec![],
            generated: vec![],
            artifacts: vec![],
            denied: vec![],
            binary: vec![],
        };
        (policy, file_set)
    };
    let applied = common::task_execution::dispatch_resolved_route_block_on(
        prometheos_lite::harness::patch_applier::apply_patch_with_rollback(
            &edits, &file_set, &policy,
        ),
    )
    .expect("recorded proposal applies");
    assert!(applied.0.applied, "recorded proposal applies cleanly");
    let pins = envelope
        .scenario_run
        .descriptor
        .validation
        .clone()
        .expect("pins");
    let target_dir = temp.path().join("validator-target");
    let record = common::task_execution::run_validator(&pins, verifier_workdir, &target_dir);
    assert_eq!(
        record.exit_code,
        Some(0),
        "recorded proposal passes the pinned validator"
    );
    let tree_digest = prometheos_lite::workflow::scenario::task_run_tree_digest(verifier_workdir)
        .expect("tree digest");
    assert_eq!(
        tree_digest, envelope.post_run_content_digest,
        "recorded proposal reproduces the recorded post-run tree digest"
    );

    // --- Link 4: the preserved envelope fails repaired verification ------
    assert!(
        envelope.verify().is_err(),
        "the preserved historical envelope (pre-repair provenance) must not verify"
    );
}

// ---------------------------------------------------------------------------
// Live, opt-in pilot (never runs by default)
// ---------------------------------------------------------------------------

/// Operator-granted authorization for ONE synthetic-source egress run.
/// The gateway refuses to open an upstream connection unless this grant
/// names the exact upstream and explicitly authorizes the synthetic
/// fixture source — the authorization is checked at the outbound effect,
/// before any bytes leave the process.
pub struct GatewayEgressGrant {
    /// Full upstream completions URL the gateway may dial.
    pub upstream_url: String,
    /// Effective network policy from the authority of the run being
    /// gated; re-checked at the outbound boundary.
    pub effective_network_policy: prometheos_lite::workflow::soma::contracts::NetworkPolicy,
    /// Explicit per-run operator authorization for this synthetic
    /// fixture source to be transmitted. False denies everything.
    pub permit_synthetic_source_egress: bool,
    /// Provider identity pinned for this run (must equal the descriptor's).
    pub provider_identity: String,
    /// Upstream model the gateway may request.
    pub upstream_model: String,
    /// Model identity the harness and the rest of Lite see (alias).
    pub harness_visible_model: String,
    /// Known secret values that must never leave this gateway.
    pub known_secrets: Vec<String>,
    /// Connections opened to the upstream. Incremented only after the
    /// egress preflight passes.
    pub would_dial: Arc<AtomicU64>,
    /// Connections refused by the egress preflight.
    pub refused: Arc<AtomicU64>,
}

/// Outbound effect boundary. Runs BEFORE the gateway opens any upstream
/// connection; on any refusal it returns `Err(reason)` and the caller
/// must NOT dial out. `body` must be the exact serialized payload that
/// will be transmitted on the wire.
fn egress_preflight(grant: &GatewayEgressGrant, body: &[u8]) -> Result<(), &'static str> {
    // Scheme pinning, authority disambiguation and port pinning via
    // proper URL parsing (never manual string splitting).
    let parsed = parse_upstream_url(&grant.upstream_url)?;
    let host = parsed.host;
    // The effective network policy decides, not the URL shape.
    let policy_permits = match grant.effective_network_policy.default {
        prometheos_lite::workflow::soma::types::NetworkDefault::Allow => true,
        prometheos_lite::workflow::soma::types::NetworkDefault::Deny => grant
            .effective_network_policy
            .allowlist
            .as_ref()
            .map(|list| list.iter().any(|entry| entry.eq_ignore_ascii_case(&host)))
            .unwrap_or(false),
    };
    if !policy_permits {
        return Err("effective network policy denies the upstream host");
    }
    // Explicit authorization for this synthetic source, separately.
    if !grant.permit_synthetic_source_egress {
        return Err("synthetic-source egress is not authorized for this run");
    }
    // No known secret may cross this boundary.
    for secret in &grant.known_secrets {
        if !secret.is_empty() && body.windows(secret.len()).any(|w| w == secret.as_bytes()) {
            return Err("outbound message contains a known secret; refused");
        }
    }
    Ok(())
}

fn spawn_openrouter_gateway(
    grant: GatewayEgressGrant,
) -> (
    String,
    Arc<AtomicU64>,
    Arc<AtomicU64>,
    Arc<AtomicU64>,
    Arc<AtomicBool>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("gateway binds loopback");
    let addr = listener.local_addr().expect("gateway addr");
    let url = format!("http://127.0.0.1:{}/v1", addr.port());
    let effects = Arc::new(AtomicU64::new(0));
    let refused = grant.refused.clone();
    let would_dial = grant.would_dial.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let effects_thread = effects.clone();
    let stop_thread = stop.clone();
    thread::spawn(move || {
        listener.set_nonblocking(true).expect("gateway nonblocking");
        loop {
            if stop_thread.load(Ordering::SeqCst) {
                break;
            }
            match listener.accept() {
                Ok((stream, _)) => {
                    effects_thread.fetch_add(1, Ordering::SeqCst);
                    let connection_grant = GatewayEgressGrant {
                        upstream_url: grant.upstream_url.clone(),
                        effective_network_policy: grant.effective_network_policy.clone(),
                        permit_synthetic_source_egress: grant.permit_synthetic_source_egress,
                        provider_identity: grant.provider_identity.clone(),
                        upstream_model: grant.upstream_model.clone(),
                        harness_visible_model: grant.harness_visible_model.clone(),
                        known_secrets: grant.known_secrets.clone(),
                        would_dial: grant.would_dial.clone(),
                        refused: grant.refused.clone(),
                    };
                    thread::spawn(move || handle_gateway_conn(stream, connection_grant));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
    });
    (url, effects, refused, would_dial, stop)
}

fn handle_gateway_conn(mut stream: std::net::TcpStream, grant: GatewayEgressGrant) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let mut buf = Vec::with_capacity(8192);
    let mut chunk = [0u8; 8192];
    let mut content_length = 0usize;
    let mut header_end = None;
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if header_end.is_none()
                    && let Some(pos) = find_subslice(&buf, b"\r\n\r\n")
                {
                    header_end = Some(pos + 4);
                    let headers = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
                    for line in headers.lines() {
                        if let Some(value) = line.strip_prefix("content-length:") {
                            content_length = value.trim().parse().unwrap_or(0);
                        }
                    }
                }
                if let Some(end) = header_end
                    && buf.len() >= end + content_length
                {
                    break;
                }
                if buf.len() > 256 * 1024 {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let end = match header_end {
        Some(end) => end,
        None => return,
    };
    let body = &buf[end..end + content_length.min(buf.len().saturating_sub(end))];
    let request: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return,
    };
    if !request.get("messages").and_then(Value::as_array).is_some() {
        return;
    }
    let mut upstream_request = request.clone();
    upstream_request["model"] = json!(grant.upstream_model);
    // Operator gateway policy: this upstream is a reasoning model; with
    // reasoning enabled it spends the whole token budget on reasoning and
    // returns null content, which the Lite harness refuses as an
    // invalid response. The operator disables reasoning upstream; Lite's
    // harness itself is untouched.
    upstream_request["reasoning"] = json!({ "enabled": false });

    // ---- Build the actual outbound payload FIRST ----------------------
    // The authorization and known-secret checks below run against these
    // exact bytes, which are the bytes transmitted on the wire.
    let payload = match serde_json::to_vec(&upstream_request) {
        Ok(payload) => payload,
        Err(_) => return,
    };

    // ---- EGRESS PREFLIGHT: decide before any upstream effect -----------
    if let Err(reason) = egress_preflight(&grant, &payload) {
        grant.refused.fetch_add(1, Ordering::SeqCst);
        let refusal_body = json!({ "error": "egress-refused", "reason": reason }).to_string();
        let refusal = format!(
            "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            refusal_body.len(),
            refusal_body
        );
        let _ = stream.write_all(refusal.as_bytes());
        let _ = stream.shutdown(std::net::Shutdown::Both);
        return;
    }
    // The authorization holds; count the outbound effect.
    grant.would_dial.fetch_add(1, Ordering::SeqCst);

    // The key is attached when present. The offline mock-upstream tests run
    // without it by design; the live path asserts its presence before ever
    // reaching the gateway.
    let api_key = std::env::var("OPENROUTER_API_KEY")
        .ok()
        .filter(|k| !k.is_empty());
    // Forward to the pinned upstream; the model identity is rewritten to
    // the harness-visible alias in the response so the harness's
    // model-mismatch check passes and only the alias appears on the
    // Lite-facing wire.
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return,
    };
    // Redirects are NEVER followed: a 301/302/303/307/308 from the
    // upstream must not become permission to contact another host. Any
    // redirect response is handled as a non-successful provider response
    // below.
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(100))
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(client) => client,
        Err(_) => return,
    };
    let upstream_url = grant.upstream_url.clone();
    let (status, mut response_body) = runtime.block_on(async move {
        let builder = client
            .post(&upstream_url)
            .header("Content-Type", "application/json")
            .header(
                "HTTP-Referer",
                "https://github.com/prometheosai/prometheos-lite",
            )
            .header("X-Title", "prometheos-lite pilot")
            .body(payload);
        let builder = match &api_key {
            Some(key) => builder.bearer_auth(key),
            None => builder,
        };
        match builder.send().await {
            Ok(response) => {
                let status = response.status().as_u16();
                (status, response.text().await.unwrap_or_default())
            }
            Err(_) => (502, String::new()),
        }
    });
    // A redirect response is a non-successful provider response. It is
    // never followed (Policy::none above) and never becomes a fresh
    // authorized dial; the harness sees an explicit non-2xx result.
    let mut redirect_refused = false;
    let status = if (300..400).contains(&status) {
        redirect_refused = true;
        502
    } else {
        status
    };
    if redirect_refused {
        response_body = json!({
            "error": "redirect-refused",
            "reason": "upstream returned a redirect; redirects are not followed and the destination is not authorized",
        })
        .to_string();
    }
    if !redirect_refused && let Ok(mut parsed) = serde_json::from_str::<Value>(&response_body) {
        parsed["model"] = json!(grant.harness_visible_model);
        response_body = parsed.to_string();
    }
    let response_text = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response_body.len(),
        response_body
    );
    let _ = stream.write_all(response_text.as_bytes());
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[test]
fn live_openrouter_run_is_opt_in() {
    if std::env::var("PROMETHEOS_REAL_MODEL_PILOT").as_deref() != Ok("1") {
        eprintln!(
            "SKIP: set PROMETHEOS_REAL_MODEL_PILOT=1 and PROMETHEOS_PILOT_SYNTHETIC_EGRESS_AUTHORIZATION=1 to run the live pilot"
        );
        return;
    }
    assert!(
        std::env::var("OPENROUTER_API_KEY")
            .map(|k| !k.is_empty())
            .unwrap_or(false),
        "OPENROUTER_API_KEY must be configured in the environment for the live pilot"
    );
    assert_eq!(
        std::env::var("PROMETHEOS_PILOT_SYNTHETIC_EGRESS_AUTHORIZATION").as_deref(),
        Ok("1"),
        "the live pilot requires the explicit synthetic-source egress authorization"
    );
    let alias = "nemotron-ultra-550b-free";
    let policy = NetworkPolicy {
        default: NetworkDefault::Deny,
        allowlist: Some(vec![common::model_pilot::LIVE_EGRESS_HOST.to_string()]),
    };
    let grant = GatewayEgressGrant {
        upstream_url: format!(
            "https://{}/api/v1/chat/completions",
            common::model_pilot::LIVE_EGRESS_HOST
        ),
        effective_network_policy: policy,
        permit_synthetic_source_egress: true,
        provider_identity: "openrouter-via-loopback-gateway".to_string(),
        upstream_model: "nvidia/nemotron-3-ultra-550b-a55b:free".to_string(),
        harness_visible_model: alias.to_string(),
        known_secrets: vec![prometheos_lite::workflow::redaction::SECRET_CANARY.to_string()],
        would_dial: Arc::new(AtomicU64::new(0)),
        refused: Arc::new(AtomicU64::new(0)),
    };
    let (endpoint, effects, refused, would_dial, stop) = spawn_openrouter_gateway(grant);
    let mut spawn = |_attempt: u32| PilotEndpoint {
        endpoint: endpoint.clone(),
        effects: effects.clone(),
    };
    let run = run_model_pilot(
        LIVE_DESCRIPTOR,
        PILOT_EXAMPLE_PRICE_BOOK,
        110_000,
        RouteProfile::Standard,
        TaskRepairProvenanceKind::OperatorObservedLive,
        &mut spawn,
    );
    stop.store(true, Ordering::SeqCst);
    let _ = refused.load(Ordering::SeqCst);
    let _ = would_dial.load(Ordering::SeqCst);

    // Evidence transcript (contains no credentials; the model's public
    // task output only).
    let receipt = json!({
        "schemaVersion": "1.0.0",
        "pilot": "lite-rust-tokenbucket-bugfix-model-live-001",
        "harness": "lite.harness.local_model@v1",
        "route": "openrouter-via-loopback-gateway",
        "harnessVisibleModel": alias,
        "upstreamModel": "nvidia/nemotron-3-ultra-550b-a55b:free",
        "egressAuthorization": {
            "permitSyntheticSourceEgress": true,
            "upstreamHost": common::model_pilot::LIVE_EGRESS_HOST,
            "providerIdentity": "openrouter-via-loopback-gateway",
            "effectiveNetworkPolicy": "deny + allowlist [openrouter.ai]",
        },
        "capabilityEffects": run.capability_effects,
        "attempts": run.attempts.iter().map(|a| json!({
            "attempt": a.attempt,
            "modelResponseDigest": a.model_response_digest,
            "proposalDigest": a.proposal_digest,
            "editCount": a.edit_count,
            "applied": a.applied,
            "validatorExitCode": a.validator_exit_code,
            "failure": a.failure,
            "proposal": a.model_response_raw,
        })).collect::<Vec<_>>(),
        "preFixProbe": run.pre_fix.as_ref().map(|p| json!({
            "command": p.command,
            "args": p.args,
            "exitCode": p.exit_code,
            "outputDigest": p.output_digest,
            "outputBytes": p.output_bytes,
            "timedOut": p.timed_out,
            "outputTail": p.output_tail,
        })),
        "classification": format!("{:?}", run.classification),
        "sealError": run.seal_error,
        "patched": run.patched,
        "changedVsBaseline": run.changed_vs_baseline,
        "baselineDigest": run.baseline_digest,
        "postRunDigest": run.post_run_digest,
        "envelopeSealed": run.envelope.is_some(),
        "taskOutcome": run.envelope.as_ref().map(|e| format!("{:?}", e.task_outcome)),
        "envelopeDigest": run.envelope.as_ref().and_then(|e| e.content_digest.clone()),
        // The full sealed envelope, so an independent reviewer can call
        // verify() on the exact document the pilot sealed.
        "envelope": run.envelope,
        "accounting": run.accounting.as_ref().map(|a| json!({
            "usage": a.usage,
            "cost": a.cost,
        })),
        "validatorExitCode": run.attempts.last().and_then(|a| a.validator_exit_code),
    });
    let out = std::env::var("PROMETHEOS_PILOT_EVIDENCE_OUT")
        .unwrap_or_else(|_| "target/real-model-pilot-evidence.json".to_string());
    if let Some(parent) = Path::new(&out).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(
        &out,
        serde_json::to_string_pretty(&receipt).expect("receipt serializes"),
    )
    .expect("receipt written");
    eprintln!("pilot evidence written to {out}");
    eprintln!(
        "classification: {:?}; patched: {}; envelope: {}",
        run.classification,
        run.patched,
        run.envelope.is_some()
    );
    // The pilot is opt-in infrastructure; reaching this point with a sealed
    // or honestly refused envelope is the pass condition. Whether the model
    // succeeded is recorded in the receipt, not asserted.
    assert!(run.pre_fix.is_some());
}
