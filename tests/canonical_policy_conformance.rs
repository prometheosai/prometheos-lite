//! Cross-path canonicalization golden pins (#215 — Option 3).
//!
//! Three canonicalization policies exist in this repository; this file
//! pins their CURRENT bytes and digests with fixed golden constants so
//! unilateral drift in any single path fails immediately (not merely
//! `assert_ne!`-relative checks).
//!
//!   - **PATH A — `soma::canonical::{try_canonical_bytes,try_canonical_digest}`**:
//!     SOMA interop policy. Custom `format_number`: integral floats render
//!     as integers (`1.0` → `1`); shortest-round-trip fixed-point for
//!     non-integral floats (NO scientific notation); DecimalV2 limits
//!     (≤400 significant digits at text layer; |exponent| ≤ 1e10000)
//!     enforced fail-closed.
//!   - **PATH B — `portable_state::{to_canonical_json, state_digest}`**:
//!     Lite portable export. serde_json scalar rendering on the
//!     Lite-normalized state (`normalized_state` sorts set-like
//!     collections before serialization).
//!   - **PATH C — `memory_contracts::{to_canonical_json, canonical_digest}`**
//!     as applied by `ProjectCheckpoint::from_portable_work_state`:
//!     serde_json scalar rendering on the RAW `serde_json::to_value(pws)`
//!     — set-like collection ORDER is digested as-is (no normalization).
//!
//! § LOCKS: `state_digest` (B), checkpoint digest (C), and SOMA canonical
//! digest (A) are **not interchangeable identities**. Divergences that
//! currently exist (all pinned below):
//!   1. **Integral floats**: A `1.0`→`1`; B/C `1.0`→`1.0`.
//!      (Not EVERY f64 diverges: e.g. `0.7` renders `0.7` identically in
//!      all three paths.)
//!   2. **Floats outside fixed-point range**: A expands to fixed decimal
//!      digits (`1e30` → the 31-digit integer literal); serde paths
//!      (B/C) use scientific notation (`1e+30` / `1.5e-10`).
//!   3. **Set-like collection order**: B normalizes (sorted) before
//!      digests; C digests input order as-is. A operates on whatever
//!      Value it is handed.
//!
//! All pins are regression locks of behavior as of `main @ ...`, NOT
//! endorsements of divergence. Unification behind a schema-version gate
//! is a deferred product decision (see #215). `serde_json` feature
//! `arbitrary_precision` stays disabled.

use prometheos_lite::workflow::memory_contracts::{ProjectCheckpoint, canonical_digest};
use prometheos_lite::workflow::portable_state::{
    Confidence, PortableWorkState, import_portable_state, state_digest,
};
use prometheos_lite::workflow::soma::canonical::{
    try_canonical_bytes, try_canonical_digest, validate_number_lexemes,
};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// Corpus 1 — Integral float `1.0`: exact byte + digest pins, all three paths.
// ---------------------------------------------------------------------------

#[test]
fn integral_float_1_0_golden_pins() {
    let v = json!({"value": 1.0});

    // Path A bytes + digest
    let a_bytes = try_canonical_bytes(&v).unwrap();
    assert_eq!(String::from_utf8(a_bytes).unwrap(), "{\"value\":1}");
    assert_eq!(
        try_canonical_digest(&v).unwrap(),
        "48208f9428d64634bd8e28ff345bf0eab60d53c18fa2fbdb0b9bc1e84df2b5f6"
    );

    // Path C bytes + digest (raw serde_json rendering with fraction kept)
    let c_json = prometheos_lite::workflow::memory_contracts::to_canonical_json(&v);
    assert_eq!(c_json, "{\"value\":1.0}");
    assert_eq!(
        canonical_digest(&v).unwrap(),
        "3a7d647740ec6f86b72e0bf3948ab456551e07e9605e3a2785de1c66842ebb48"
    );

    // The 1.0 digests differ (divergence pinned).
    assert_ne!(
        try_canonical_digest(&v).unwrap(),
        canonical_digest(&v).unwrap()
    );
}

// ---------------------------------------------------------------------------
// Corpus 2 — Non-integral f64 where ALL three agree (0.7). Pins the
// agreement domain explicitly so the divergence claim stays bounded.
// ---------------------------------------------------------------------------

#[test]
fn non_integral_agreement_domain_0_7_pin() {
    let v = json!({"value": 0.7});
    let expected_bytes = "{\"value\":0.7}";

    assert_eq!(
        String::from_utf8(try_canonical_bytes(&v).unwrap()).unwrap(),
        expected_bytes
    );
    assert_eq!(
        prometheos_lite::workflow::memory_contracts::to_canonical_json(&v),
        expected_bytes
    );
    assert_eq!(
        try_canonical_digest(&v).unwrap(),
        canonical_digest(&v).unwrap(),
        "0.7 renders identically under A and C — agreement domain pin"
    );
}

// ---------------------------------------------------------------------------
// Corpus 3 — Scientific-notation divergence: A expands to fixed-point,
// serde (B/C) keeps exponent. Pinned at both values.
// ---------------------------------------------------------------------------

#[test]
fn scientific_notation_divergence_pinned() {
    let big = json!({"value": 1e30});
    assert_eq!(
        String::from_utf8(try_canonical_bytes(&big).unwrap()).unwrap(),
        "{\"value\":1000000000000000000000000000000}"
    );
    assert_eq!(
        prometheos_lite::workflow::memory_contracts::to_canonical_json(&big),
        "{\"value\":1e+30}"
    );

    let small = json!({"value": 1.5e-10});
    assert_eq!(
        String::from_utf8(try_canonical_bytes(&small).unwrap()).unwrap(),
        "{\"value\":0.00000000015}"
    );
    assert_eq!(
        prometheos_lite::workflow::memory_contracts::to_canonical_json(&small),
        "{\"value\":1.5e-10}"
    );
}

// ---------------------------------------------------------------------------
// Corpus 4 — Key order + nesting + escaping (path A must byte-match compact
// serde_json on these — they share the same convention in this domain).
// ---------------------------------------------------------------------------

#[test]
fn key_order_nesting_and_escaping_byte_pins() {
    let v = json!({"zulu": 1, "apple": ["gamma", "alpha"], "middle": {"z": 0, "a": 1}});
    let expected = r#"{"apple":["gamma","alpha"],"middle":{"a":1,"z":0},"zulu":1}"#;
    let a = String::from_utf8(try_canonical_bytes(&v).unwrap()).unwrap();
    let c = prometheos_lite::workflow::memory_contracts::to_canonical_json(&v);
    assert_eq!(a, expected);
    assert_eq!(c, expected);

    let v2 = json!({
        "empty_obj": {},
        "empty_arr": [],
        "esc": "line\nbreak\t\"quoted\"",
        "unicode": "héllo→世界"
    });
    let expected2 = "{\"empty_arr\":[],\"empty_obj\":{},\"esc\":\"line\\nbreak\\t\\\"quoted\\\"\",\"unicode\":\"héllo→世界\"}";
    assert_eq!(
        String::from_utf8(try_canonical_bytes(&v2).unwrap()).unwrap(),
        expected2
    );
    assert_eq!(
        prometheos_lite::workflow::memory_contracts::to_canonical_json(&v2),
        expected2
    );
}

// ---------------------------------------------------------------------------
// Corpus 5 — DecimalV2 rejection boundaries (A path ONLY: B/C have no
// DecimalV2 policy). Layered behavior pinned.
// ---------------------------------------------------------------------------

#[test]
fn decimal_v2_boundaries_pinned() {
    // 400 significant digits: ok at the text guard.
    let ok_text = format!("{{\"n\": 1.{}0}}", "1".repeat(398));
    assert!(validate_number_lexemes(ok_text.as_bytes()).is_ok());

    // 401 significant digits: rejected at the text guard.
    let too_many_text = format!("{{\"n\": 1.{}1}}", "1".repeat(399));
    assert!(
        validate_number_lexemes(too_many_text.as_bytes()).is_err(),
        "text guard must reject 401 significant digits"
    );

    // By Value level serde has already truncated to f64 — pinned: the value
    // writer accepts the truncated form (guard is text-layer only).
    let v: Value = serde_json::from_str(&too_many_text).expect("serde parses 401-digit f64");
    assert!(try_canonical_bytes(&v).is_ok());

    // Magnitude: exactly 1e10000 passes the guard but exceeds f64 → parse fails.
    assert!(validate_number_lexemes(b"{\"n\": 1e10000}").is_ok());
    assert!(validate_number_lexemes(b"{\"n\": 1e10001}").is_err());
    assert!(serde_json::from_str::<Value>("{\"n\": 1e10000}").is_err());
    assert!(serde_json::from_str::<Value>("{\"n\": 1e10001}").is_err());
    assert!(serde_json::from_str::<Value>("{\"n\": 1e300}").is_ok());
}

// ---------------------------------------------------------------------------
// Corpus 6 — SHARED TYPED FIXTURE: golden byte fixtures + golden digests
// for all three paths, on three variants of the SAME portable state.
//
// The fixture has NON-integral floats (0.7, 0.9 …) and set-like arrays in
// sorted order; under those circumstances all three paths happen to agree on
// the base state. The variants break that agreement deliberately:
//   - conf1: confidence 0.7 → 1.0 makes A diverge (1 vs 1.0).
//   - rev: reversed set-like arrays make A and C diverge from B (B
//     normalizes order; A and C preserve the caller's array order).
//
// Byte pins come from golden fixture files; these were GENERATED from the
// current implementation and are now the locked policy. Any change to any
// path must update the generated fixture file in the same diff, making the
// before/after byte-level consequence visible in review.
// ---------------------------------------------------------------------------

use std::fs;
use std::path::PathBuf;

fn fixture_state() -> PortableWorkState {
    let text = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/portable-work-state/current-v1/portable_work_state.json"
    ))
    .expect("portable state fixture");
    import_portable_state(&text, None).expect("fixture imports")
}

/// The fixture with decisions[0].confidence = 1.0.
fn fixture_state_with_1p0() -> PortableWorkState {
    let mut s = fixture_state();
    s.decisions[0].confidence = Some(Confidence {
        value: 1.0,
        basis: None,
    });
    s
}

/// The fixture with set-like collections reversed (required_capabilities,
/// allowed_paths).
fn fixture_state_set_reversed() -> PortableWorkState {
    let mut s = fixture_state();
    s.compatibility.required_capabilities.reverse();
    s.authority.allowed_paths.reverse();
    s
}

fn golden(stem: &str, path_letter: &str) -> Vec<u8> {
    fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/canonical-policy/current-v1")
            .join(format!("{stem}.{path_letter}.json")),
    )
    .unwrap_or_else(|e| {
        panic!("golden fixture {stem}.{path_letter}.json missing: {e}")
    })
}

fn digest_of(bytes: &[u8]) -> String {
    prometheos_lite::workflow::soma::canonical::sha256_hex(bytes)
}

/// Golden digests keyed by (variant, path).
#[track_caller]
fn pin_variant(stem: &str, state: &PortableWorkState, digests: [&str; 3]) {
    let v = serde_json::to_value(state).expect("state to value");

    // Bytes — through the public per-path entry points.
    let a_bytes = try_canonical_bytes(&v).expect("path A bytes");
    let b_bytes = prometheos_lite::workflow::portable_state::to_canonical_json(state)
        .expect("path B bytes")
        .into_bytes();
    let c_bytes = prometheos_lite::workflow::memory_contracts::to_canonical_json(&v).into_bytes();

    assert_eq!(
        a_bytes,
        golden(stem, "a"),
        "path A bytes on {stem} must match the golden fixture"
    );
    assert_eq!(
        b_bytes,
        golden(stem, "b"),
        "path B bytes on {stem} must match the golden fixture"
    );
    assert_eq!(
        c_bytes,
        golden(stem, "c"),
        "path C bytes on {stem} must match the golden fixture"
    );

    // Digests — recomputed from the same bytes AND through the public
    // digest functions (they must agree by construction).
    assert_eq!(
        try_canonical_digest(&v).unwrap(),
        digests[0],
        "path A digest on {stem} — golden pin"
    );
    assert_eq!(digest_of(&a_bytes), digests[0]);
    assert_eq!(
        state_digest(state).unwrap(),
        digests[1],
        "path B digest on {stem} — golden pin"
    );
    assert_eq!(digest_of(&b_bytes), digests[1]);
    assert_eq!(
        canonical_digest(&v).unwrap(),
        digests[2],
        "path C digest on {stem} — golden pin"
    );
    assert_eq!(digest_of(&c_bytes), digests[2]);

    // Public checkpoint entrypoint agrees with canonical_digest(v).
    assert_eq!(
        ProjectCheckpoint::from_portable_work_state(state)
            .unwrap()
            .state_digest,
        digests[2],
        "ProjectCheckpoint digest must equal canonical_digest(to_value(state)) on {stem}"
    );
}

const BASE_DIGEST: &str = "e233e4c6d2f4690754683cb3ed46b0dcf079d916523480f33e8be9206080508f";
const CONF1_A_DIGEST: &str = "ae257cce5f13147625e0e0ef42ac55bad559a0a1123a37356918884e363c55bd";
const CONF1_BC_DIGEST: &str = "74f0a34cb0cf96b3acf7e8c8f13f5686ccc2d4cc6c036e30fa79e4493b6da36d";
const REV_AC_DIGEST: &str = "3d4d7e0af5d2cd94ddb926d35babe819dcf155f963b9aabdb69df4354b07cba6";

#[test]
fn golden_fixture_base_all_paths_agree() {
    pin_variant("base", &fixture_state(), [BASE_DIGEST, BASE_DIGEST, BASE_DIGEST]);
}

#[test]
fn golden_fixture_integral_float_splits_path_a() {
    let state = fixture_state_with_1p0();
    pin_variant("conf1", &state, [CONF1_A_DIGEST, CONF1_BC_DIGEST, CONF1_BC_DIGEST]);

    // The one byte difference must be `1.0` vs `1` for confidence.value.
    let a = golden("conf1", "a");
    let b = golden("conf1", "b");
    assert!(String::from_utf8(b).unwrap().contains(r#""value":1.0"#));
    assert!(String::from_utf8(a).unwrap().contains(r#""value":1"#));
}

#[test]
fn golden_fixture_set_order_reversal_splits_path_b() {
    let state = fixture_state_set_reversed();
    // A and C preserve array order → both change identically.
    // B normalizes set-like collections → unchanged.
    pin_variant(
        "rev",
        &state,
        [REV_AC_DIGEST, BASE_DIGEST, REV_AC_DIGEST],
    );
}
