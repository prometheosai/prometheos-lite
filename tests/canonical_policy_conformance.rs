//! Cross-path canonicalization conformance pins (#215 — Option 3).
//!
//! THREE canonicalization policies exist in this repository and are now
//! pinned to their current bytes/digests so any drift fails loudly:
//!
//!   - **PATH A — `soma::canonical`** (`try_canonical_bytes` /
//!     `try_canonical_digest`): the SOMA interop policy. Custom
//!     `format_number` (integral floats render as integers: `1.0` → `1`),
//!     Python-`json.dumps(ensure_ascii=False)` escaping, byte-lexicographic
//!     key sort, and the DecimalV2 number policy enforced fail-closed
//!     (≤400 significant digits; |exponent| ≤ 1e10000; no NaN/±Inf).
//!
//!   - **PATH B — `portable_state`** (`to_canonical_json` / `state_digest`):
//!     the Lite portable-export policy. serde_json serialization after
//!     validated normalization (set-like collections sorted, keys sorted
//!     recursively, compact). serde renders integral floats WITH the
//!     fraction (`1.0` → `1.0`), so digests differ from path A for any
//!     `f64`-bearing state (e.g. `Confidence.value`).
//!
//!   - **PATH C — `memory_contracts::canonical_digest`** (used by
//!     `ProjectCheckpoint::from_portable_work_state`): an independent
//!     recursive renderer over `serde_json::to_value(pws)` — string-key
//!     sort, but **no set-like collection normalization**: capability /
//!     allowed-path array ORDER is baked into the checkpoint digest,
//!     whereas path B normalizes it away.
//!
//! § LOCK: a `state_digest` (B), a checkpoint digest (C), and a SOMA
//! canonical digest (A) are **not interchangeable identities**. They must
//! not be compared across paths, and changing any policy is a breaking
//! change to every artifact that embeds that digest.
//!
//! The pins below record each path's CURRENT (pre-unification) behavior,
//! deliberately including divergences. They are regression locks, not
//! endorsements of the divergence. Unification behind a schema-version
//! gate is a deferred product decision (#215 body); `serde_json` feature
//! `arbitrary_precision` stays disabled.

use prometheos_lite::workflow::memory_contracts::{ProjectCheckpoint, canonical_digest};
use prometheos_lite::workflow::portable_state::{
    Confidence, PortableWorkState, import_portable_state, state_digest, to_canonical_json,
};
use prometheos_lite::workflow::soma::canonical::{try_canonical_bytes, try_canonical_digest};
use serde_json::{Value, json};

/// Helper: bytes of a compact serde_json rendering for reference against
/// path A's custom writer.
fn serde_bytes(v: &Value) -> Vec<u8> {
    serde_json::to_string(v).unwrap().into_bytes()
}

// ---------------------------------------------------------------------------
// Corpus 1 — `1.0` vs `1`: path A renders integral floats as integers;
// serde-based paths B/C keep the fraction. Divergence is pinned, not fixed.
// ---------------------------------------------------------------------------

#[test]
fn integral_float_divergence_is_pinned() {
    let v = json!({"value": 1.0});

    let a_bytes = try_canonical_bytes(&v).expect("path A accepts a plain 1.0");
    assert_eq!(
        String::from_utf8(a_bytes).unwrap(),
        r#"{"value":1}"#,
        "path A renders 1.0 as integer"
    );

    let c_bytes = serde_bytes(&v);
    assert_eq!(
        String::from_utf8(c_bytes).unwrap(),
        r#"{"value":1.0}"#,
        "serde_json (path B/C backing renderer) keeps fraction"
    );

    // Divergence pinned: same logical number, different digests.
    assert_ne!(
        try_canonical_digest(&v).unwrap(),
        canonical_digest(&v).unwrap(),
        "A digest and C digest of the SAME value must differ for 1.0 (pin)"
    );
}

// ---------------------------------------------------------------------------
// Corpus 2 — non-integral values where all three policies agree. This corpus
// documents the safe overlap domain; any change here is a hard break of all
// three, so it is pinned to equality.
// ---------------------------------------------------------------------------

#[test]
fn non_integral_agreement_domain_is_pinned() {
    let v = json!({
        "rate": 0.75,
        "epsilon": 0.1,
        "count": 42,
        "label": "portable",
        "flags": [true, false],
        "nested": {"b": 2, "a": [3, null]}
    });

    let a_bytes = try_canonical_bytes(&v).unwrap();
    let c_json = serde_json::to_string(&v).unwrap();
    assert_eq!(
        String::from_utf8(a_bytes).unwrap(),
        c_json,
        "on non-integral corpus, custom A writer must be byte-identical to serde compact output"
    );
    assert_eq!(
        try_canonical_digest(&v).unwrap(),
        canonical_digest(&v).unwrap(),
        "on non-integral corpus, A and C digests must agree"
    );
}

// ---------------------------------------------------------------------------
// Corpus 3 — key ordering: byte-lexicographic sort, keys out of order on
// input produce canonical order. Pinned on both custom writer A and C path.
// ---------------------------------------------------------------------------

#[test]
fn key_ordering_is_pinned() {
    let v = json!({"zulu": 1, "apple": ["gamma", "alpha"], "middle": {"z": 0, "a": 1}});

    let a = String::from_utf8(try_canonical_bytes(&v).unwrap()).unwrap();
    let c = serde_json::to_string(&v).unwrap();
    let expected = r#"{"apple":["gamma","alpha"],"middle":{"a":1,"z":0},"zulu":1}"#;
    assert_eq!(a, expected, "path A key order");
    assert_eq!(c, expected, "path C key order");
    assert_eq!(a, c);
}

// ---------------------------------------------------------------------------
// Corpus 4 — nested empty objects/arrays and string escaping.
// ---------------------------------------------------------------------------

#[test]
fn nested_and_escaping_pinned() {
    let v = json!({
        "empty_obj": {},
        "empty_arr": [],
        "esc": "line\nbreak\t\"quoted\"",
        "unicode": "héllo→世界"
    });

    let a = String::from_utf8(try_canonical_bytes(&v).unwrap()).unwrap();
    let c = serde_json::to_string(&v).unwrap();
    let expected =
        r#"{"empty_arr":[],"empty_obj":{},"esc":"line\nbreak\t\"quoted\"","unicode":"héllo→世界"}"#;
    assert_eq!(a, expected, "path A nested/escaping");
    assert_eq!(c, expected, "path C nested/escaping");
}

// ---------------------------------------------------------------------------
// Corpus 5 — DecimalV2 rejection boundaries. Path A enforces the limits at
// TWO layers with deliberately different reach (see #216):
//   - `validate_number_lexemes` (text level): sees the original lexemes,
//     rejects >400 significant digits — the ONLY layer that can.
//   - `try_canonical_bytes` (Value level): by then serde has already
//     truncated to f64 (<=17 sig digits), so the limit can never fire; this
//     asymmetry is pinned, not a bug.
// Paths B/C carry NO DecimalV2 policy at all: the corpus parses and digests
// serde-side regardless of the text-level count.
// ---------------------------------------------------------------------------

use prometheos_lite::workflow::soma::canonical::validate_number_lexemes;

#[test]
fn decimal_v2_boundaries_pinned() {
    // 400 significant digits: ok at the text layer (A policy).
    let ok_text = format!("{{\"n\": 1.{}0}}", "1".repeat(398));
    assert!(validate_number_lexemes(ok_text.as_bytes()).is_ok());

    // 401 significant digits: rejected at the text layer.
    let too_many_text = format!("{{\"n\": 1.{}1}}", "1".repeat(399));
    assert!(
        validate_number_lexemes(too_many_text.as_bytes()).is_err(),
        "text guard must reject 401 significant digits"
    );

    // After serde parses the same text, the Value is f64-truncated, so the
    // value-level writer CANNOT observe the original 401 digits — pinned: it
    // accepts what parse produced.
    let v: Value = serde_json::from_str(&too_many_text).expect("serde parses 401-digit f64");
    assert!(
        try_canonical_bytes(&v).is_ok(),
        "value-level A writer accepts f64-truncated form: the precision guard is text-only (pinned)"
    );

    // Magnitude: exactly 1e10000 is in policy at the TEXT guard but exceeds
    // f64 range, so serde parse fails while the text guard accepts.
    assert!(validate_number_lexemes(b"{\"n\": 1e10000}").is_ok());
    assert!(validate_number_lexemes(b"{\"n\": 1e10001}").is_err());
    assert!(serde_json::from_str::<Value>("{\"n\": 1e10001}").is_err());
    assert!(serde_json::from_str::<Value>("{\"n\": 1e10000}").is_err());
    assert!(serde_json::from_str::<Value>("{\"n\": 1e300}").is_ok());
}

// ---------------------------------------------------------------------------
// Corpus 6 — Three-path identity pins on a REAL fixture state. Digests are
// per-path sha256 hex constants; changing path semantics must touch this
// test deliberately.
// ---------------------------------------------------------------------------

fn fixture_state() -> PortableWorkState {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/portable-work-state/current-v1/portable_work_state.json"
    ))
    .expect("portable state fixture");
    import_portable_state(&text, None).expect("fixture imports")
}

/// Mutate the fixture: set one decision's confidence to exactly 1.0.
fn fixture_state_with_1p0() -> PortableWorkState {
    let mut s = fixture_state();
    s.decisions[0].confidence = Some(Confidence {
        value: 1.0,
        basis: None,
    });
    s
}

#[test]
fn three_path_digest_identities_are_pinned_on_fixture() {
    let state = fixture_state();

    // Path B digest of the fixture: sha256 of `to_canonical_json(state)`.
    let b = state_digest(&state).expect("path B digest");
    assert_eq!(
        b, "e233e4c6d2f4690754683cb3ed46b0dcf079d916523480f33e8be9206080508f",
        "path B digest of current-v1 fixture is pinned; any change here is a deliberate policy revision"
    );

    // Path A digest over the same state; the fixture carries only
    // non-integral floats (0.7, 0.9), so A and B agree here.
    let a = try_canonical_digest(&serde_json::to_value(&state).unwrap())
        .expect("path A digest of same state");
    assert_eq!(
        a, b,
        "fixture has no integral floats: A and B agree (pin confirms the overlap domain)"
    );

    // With an integral float (confidence 1.0), the paths diverge.
    let state_1p0 = fixture_state_with_1p0();
    let a1 = try_canonical_digest(&serde_json::to_value(&state_1p0).unwrap()).unwrap();
    let b1 = state_digest(&state_1p0).unwrap();
    assert_ne!(
        a1, b1,
        "with an integral float (confidence=1.0), A digest 1.0 as integer, B keeps 1.0 — pinned divergence"
    );

    // Path C checkpoint digest of the ORIGINAL state (right now: no set normalization).
    let cp = ProjectCheckpoint::from_portable_work_state(&state).expect("checkpoint");
    assert_eq!(
        cp.state_digest, b,
        "path C (canonical_digest over serde value) on the fixture matches path B: float-free state falls in the shared domain"
    );
}

#[test]
fn integral_float_divergence_reaches_state_digest() {
    let s1p0 = fixture_state_with_1p0();
    let s0 = fixture_state();

    let b0 = state_digest(&s0).unwrap();
    let b1 = state_digest(&s1p0).unwrap();

    // Sanity: forcing 1.0 changes the state's computed digest (f64 present).
    assert_ne!(b0, b1, "confidence 0.7 vs 1.0 → different state digest");

    // And the raw canonical bytes under A vs B diverge on 1.0:
    let value = serde_json::to_value(&s1p0).expect("state to value");
    let a_bytes = String::from_utf8(try_canonical_bytes(&value).unwrap()).unwrap();
    let b_bytes = to_canonical_json(&s1p0)
        .expect("path B canonical")
        .to_string();
    assert_ne!(
        a_bytes, b_bytes,
        "1.0 renders as 1 under A and 1.0 under B — divergence pinned at the state level"
    );
}

#[test]
fn set_like_normalization_gap_between_b_and_c_is_pinned() {
    // Two states differing ONLY in set-like collection order.
    let s1 = fixture_state();
    let mut s2 = fixture_state();
    s2.compatibility.required_capabilities.reverse();
    s2.authority.allowed_paths.reverse();

    // Path B normalizes set-like collections before digests.
    assert_eq!(
        state_digest(&s1).unwrap(),
        state_digest(&s2).unwrap(),
        "path B: set-like order must not affect the digest"
    );

    // Path C (checkpoint) does NOT normalize: same state identity,
    // different checkpoint digest depending on input order. Pinned gap.
    let c1 = ProjectCheckpoint::from_portable_work_state(&s1).unwrap();
    let c2 = ProjectCheckpoint::from_portable_work_state(&s2).unwrap();
    assert_ne!(
        c1.state_digest, c2.state_digest,
        "path C checkpoint digest is order-sensitive to set-like collections — pinned gap"
    );
}
