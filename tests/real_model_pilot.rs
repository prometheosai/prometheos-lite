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
use prometheos_lite::workflow::scenario::{
    ScenarioOutcomeClass, TaskOutcome, TaskRepairOrigin, TaskRepairWriteAuthority,
};

const OFFLINE_DESCRIPTOR: &str =
    include_str!("fixtures/scenarios/rust-tokenbucket-bugfix/descriptor.json");
const LIVE_DESCRIPTOR: &str =
    include_str!("fixtures/scenarios/rust-tokenbucket-bugfix/descriptor.live.json");

// ---------------------------------------------------------------------------
// Canned (offline) responders
// ---------------------------------------------------------------------------

fn canned_chat_completion(content: &str, with_usage: bool) -> String {
    let usage = if with_usage {
        json!({ "prompt_tokens": 125, "completion_tokens": 40, "total_tokens": 165 })
    } else {
        Value::Null
    };
    json!({
        "id": "chatcmpl-offline-pilot",
        "object": "chat.completion",
        "model": "tiny-local-model",
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
        let body = responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| canned_chat_completion("I cannot help with that.", true));
        let server = common::task_execution::spawn_loopback_model(body);
        PilotEndpoint {
            endpoint: server.endpoint,
            effects: server.effects,
        }
    };
    run_model_pilot(descriptor, price_book, timeout_ms, &mut spawn)
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
        vec![canned_chat_completion(&reference_proposal(), true)],
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
                "{\"edits\":[{\"file\":\"src/bucket.rs\",\"search\":\"NO SUCH TEXT\",\"replace\":\"x\"}]}",
                true,
            ),
            canned_chat_completion(&reference_proposal(), true),
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
        vec![canned_chat_completion(&outside, true)],
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
        vec![canned_chat_completion(&outside, true)],
    );
    assert!(!run.attempts[0].applied);
    assert_eq!(run.changed_vs_baseline, Vec::<String>::new());
    assert!(run.envelope.is_none());
}

#[test]
fn offline_malformed_model_json_is_rejected_without_effects() {
    let run = run_canned(
        OFFLINE_DESCRIPTOR,
        vec![canned_chat_completion("I would rather not say", true); MAX_ATTEMPTS as usize],
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
        vec![canned_chat_completion(&proposal, true); MAX_ATTEMPTS as usize],
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
    let run = run_model_pilot(&stale, PILOT_EXAMPLE_PRICE_BOOK, 5_000, &mut spawn);
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
        vec![canned_chat_completion(&reference_proposal(), false)],
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
        vec![canned_chat_completion(&reference_proposal(), true)],
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
        vec![canned_chat_completion(&reference_proposal(), true)],
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
// Committed live-pilot receipt: independently re-verifiable
// ---------------------------------------------------------------------------

/// The receipt of the real, opt-in OpenRouter pilot run published in
/// `docs/evidence/real-model-pilot-tokenbucket-2026-11-16.json`. This test
/// re-derives everything from the committed bytes alone: the exact sealed
/// envelope the live pilot produced must verify, and its repair must carry
/// model-authored provenance. No network, no model, no session state.
#[test]
fn committed_live_pilot_envelope_verifies() {
    let receipt: Value = serde_json::from_str(include_str!(
        "../docs/evidence/real-model-pilot-tokenbucket-2026-11-16.json"
    ))
    .expect("committed receipt parses");
    assert_eq!(receipt["taskOutcome"], "Repaired");
    assert_eq!(receipt["patched"], true);
    let envelope: prometheos_lite::workflow::scenario::LiteTaskRunEvidence =
        serde_json::from_value(receipt["envelope"].clone()).expect("committed envelope parses");
    envelope
        .verify()
        .expect("committed live-pilot envelope verifies");
    assert_eq!(
        envelope.task_outcome,
        prometheos_lite::workflow::scenario::TaskOutcome::Repaired
    );
    assert_eq!(envelope.repair.origin, TaskRepairOrigin::ModelAuthored);
    let provenance = envelope
        .repair
        .model_provenance
        .as_ref()
        .expect("model-authored provenance bound");
    assert_eq!(provenance.proposal_digest, envelope.repair.patch_digest);
    assert_eq!(provenance.attempt, 1);
}

// ---------------------------------------------------------------------------
// Live, opt-in pilot (never runs by default)
// ---------------------------------------------------------------------------

fn spawn_openrouter_gateway(
    upstream_model: &'static str,
    harness_alias: &'static str,
) -> (String, Arc<AtomicU64>, Arc<AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("gateway binds loopback");
    let addr = listener.local_addr().expect("gateway addr");
    let url = format!("http://127.0.0.1:{}/v1", addr.port());
    let effects = Arc::new(AtomicU64::new(0));
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
                    let effects_conn = effects_thread.clone();
                    thread::spawn(move || {
                        let _ = effects_conn;
                        handle_gateway_conn(stream, upstream_model, harness_alias);
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
    });
    (url, effects, stop)
}

fn handle_gateway_conn(mut stream: std::net::TcpStream, upstream_model: &str, harness_alias: &str) {
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
    upstream_request["model"] = json!(upstream_model);
    // Operator gateway policy: this upstream is a reasoning model; with
    // reasoning enabled it spends the whole token budget on reasoning and
    // returns null content, which the Lite harness refuses as an
    // invalid response. The operator disables reasoning upstream; Lite's
    // harness itself is untouched.
    upstream_request["reasoning"] = json!({ "enabled": false });

    let api_key = match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.is_empty() => key,
        _ => return,
    };
    // Forward to OpenRouter; the upstream is asked for the free
    // `cohere/north-mini-code:free` model. The model identity is rewritten
    // to the harness-visible alias in the response so the harness's
    // model-mismatch check passes and the alias stays on the wire
    // inside Lite.
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return,
    };
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(100))
        .build()
    {
        Ok(client) => client,
        Err(_) => return,
    };
    let (status, mut response_body) = runtime.block_on(async move {
        match client
            .post("https://openrouter.ai/api/v1/chat/completions")
            .bearer_auth(&api_key)
            .header("Content-Type", "application/json")
            .header(
                "HTTP-Referer",
                "https://github.com/prometheosai/prometheos-lite",
            )
            .header("X-Title", "prometheos-lite pilot")
            .json(&upstream_request)
            .send()
            .await
        {
            Ok(response) => {
                let status = response.status().as_u16();
                (status, response.text().await.unwrap_or_default())
            }
            Err(_) => (502, String::new()),
        }
    });
    if let Ok(mut parsed) = serde_json::from_str::<Value>(&response_body) {
        parsed["model"] = json!(harness_alias);
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
            "SKIP: set PROMETHEOS_REAL_MODEL_PILOT=1 (and a configured OPENROUTER_API_KEY) to run the live pilot"
        );
        return;
    }
    assert!(
        std::env::var("OPENROUTER_API_KEY")
            .map(|k| !k.is_empty())
            .unwrap_or(false),
        "OPENROUTER_API_KEY must be configured in the environment for the live pilot"
    );
    let alias = "nemotron-ultra-550b-free";
    let (endpoint, effects, stop) =
        spawn_openrouter_gateway("nvidia/nemotron-3-ultra-550b-a55b:free", alias);
    let mut spawn = |_attempt: u32| PilotEndpoint {
        endpoint: endpoint.clone(),
        effects: effects.clone(),
    };
    let run = run_model_pilot(
        LIVE_DESCRIPTOR,
        PILOT_EXAMPLE_PRICE_BOOK,
        110_000,
        &mut spawn,
    );
    stop.store(true, Ordering::SeqCst);

    // Evidence transcript (contains no credentials; the model's public
    // task output only).
    let receipt = json!({
        "schemaVersion": "1.0.0",
        "pilot": "lite-rust-tokenbucket-bugfix-model-live-001",
        "harness": "lite.harness.local_model@v1",
        "route": "openrouter-via-loopback-gateway",
        "harnessVisibleModel": alias,
        "upstreamModel": "nvidia/nemotron-3-ultra-550b-a55b:free",
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
