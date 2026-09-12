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
// Corpus 6 — SHARED TYPED FIXTURE golden digests for all three paths.
// The fixture has NO integral floats (0.7, 0.9 …) and its set-like arrays
// are already in sorted order — so on this fixture, all three paths agree.
// ---------------------------------------------------------------------------

fn fixture_state() -> PortableWorkState {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/portable-work-state/current-v1/portable_work_state.json"
    ))
    .expect("portable state fixture");
    import_portable_state(&text, None).expect("fixture imports")
}

/// The fixture, two ways: (a) as-is; (b) with decisions[0].confidence.value = 1.0.
fn fixture_state_with_1p0() -> PortableWorkState {
    let mut s = fixture_state();
    s.decisions[0].confidence = Some(Confidence {
        value: 1.0,
        basis: None,
    });
    s
}

/// The fixture with reversed set-like collections (required_capabilities,
/// allowed_paths).
fn fixture_state_set_reversed() -> PortableWorkState {
    let mut s = fixture_state();
    s.compatibility.required_capabilities.reverse();
    s.authority.allowed_paths.reverse();
    s
}

#[test]
fn fixture_agreement_domain_pinned_all_three_paths() {
    let state = fixture_state();

    let a = try_canonical_digest(&serde_json::to_value(&state).unwrap()).unwrap();
    let b = state_digest(&state).unwrap();
    let c = ProjectCheckpoint::from_portable_work_state(&state)
        .unwrap()
        .state_digest;

    // Golden pin (all three agree on this fixture).
    assert_eq!(
        b, "e233e4c6d2f4690754683cb3ed46b0dcf079d916523480f33e8be9206080508f",
        "path B digest of current-v1 fixture — golden pin"
    );
    assert_eq!(a, b, "path A agrees with B on this float-free fixture");
    assert_eq!(
        c, b,
        "path C agrees with B on this normalized-order fixture"
    );
}

#[test]
fn integral_float_variant_divergence_pinned_all_three_paths() {
    let s = fixture_state_with_1p0();

    let a = try_canonical_digest(&serde_json::to_value(&s).unwrap()).unwrap();
    let b = state_digest(&s).unwrap();
    let c = ProjectCheckpoint::from_portable_work_state(&s)
        .unwrap()
        .state_digest;

    // B and C still agree (both serde renderers, no normalization difference
    // on this single-value variant).
    assert_eq!(
        b, "74f0a34cb0cf96b3acf7e8c8f13f5686ccc2d4cc6c036e30fa79e4493b6da36d",
        "path B digest with confidence=1.0 — golden pin"
    );
    assert_eq!(c, b);

    // A diverges from B/C because of the 1 → 1.0 rendering difference.
    assert_eq!(
        a, "ae257cce5f13147625e0e0ef42ac55bad559a0a1123a37356918884e363c55bd",
        "path A digest with confidence=1.0 — golden pin"
    );
    assert_ne!(a, b);
}

#[test]
fn set_like_order_gap_pinned_between_b_and_c() {
    let base = fixture_state();
    let rev = fixture_state_set_reversed();

    // Path B normalizes set-like arrays: digest unchanged.
    let b_base = state_digest(&base).unwrap();
    let b_rev = state_digest(&rev).unwrap();
    assert_eq!(
        b_base, b_rev,
        "path B is order-insensitive for set-like collections"
    );

    // Path C digests input order: reversal changes the checkpoint digest.
    let c_base = ProjectCheckpoint::from_portable_work_state(&base)
        .unwrap()
        .state_digest;
    let c_rev = ProjectCheckpoint::from_portable_work_state(&rev)
        .unwrap()
        .state_digest;
    assert_eq!(
        c_base, "e233e4c6d2f4690754683cb3ed46b0dcf079d916523480f33e8be9206080508f",
        "path C order-sensitive digest (base order) — golden pin"
    );
    assert_eq!(
        c_rev, "3d4d7e0af5d2cd94ddb926d35babe819dcf155f963b9aabdb69df4354b07cba6",
        "path C order-sensitive digest (reversed order) — golden pin"
    );
    assert_ne!(c_base, c_rev);
}
