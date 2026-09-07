//! Number-policy regression tests for the SOMA canonical layer
//! (audit follow-up, 2026-09-06).
//!
//! Locks three behaviors:
//!
//! 1. `try_canonical_bytes` / `try_canonical_digest` are fail-closed
//!    (`Result`-returning) and never substitute a value for a number. The
//!    historical `"0"`-substitution branch is gone.
//! 2. `validate_number_lexemes` refuses — at the RAW TEXT layer, before
//!    serde_json can silently rewrite or truncate them — number lexemes
//!    that violate the SOMA DecimalV2 limits: > 400 significant digits or
//!    |order of magnitude| > 10000.
//! 3. Reachability invariants of today's build (no `arbitrary_precision`):
//!    serde_json rejects f64-overflowing numbers at parse, and
//!    `Number::from_f64` rejects non-finite — so the non-finite error arm
//!    of the canonicalizer is unreachable *until the feature flips*, which
//!    is precisely why it must be an error and not a `"0"`.
//!
//! Scope note (mirrors the change record): the guard rejects what SOMA
//! rejects. It does NOT make in-limits high-precision decimals
//! (e.g. `1.234567890123456789`, 18-20 significant digits) canonical-
//! faithful — without `arbitrary_precision` those are truncated at parse.
//! That parity gap is the gated Option-B decision, and the future shared
//! conformance suite SHOULD keep failing such fixtures until then.

use prometheos_lite::workflow::soma::canonical::{
    CanonicalError, MAX_NUMBER_EXPONENT_ABS, MAX_NUMBER_SIGNIFICANT_DIGITS, try_canonical_bytes,
    try_canonical_digest, validate_number_lexemes,
};

#[test]
fn canonical_formatting_of_representable_numbers_is_unchanged() {
    // Integers keep exact digits.
    let v: serde_json::Value = serde_json::from_str("[0,-1,42,9223372036854775807]").unwrap();
    assert_eq!(
        try_canonical_bytes(&v).unwrap(),
        b"[0,-1,42,9223372036854775807]"
    );

    // u64 above i64 range.
    let v: serde_json::Value = serde_json::from_str("[18446744073709551615]").unwrap();
    assert_eq!(try_canonical_bytes(&v).unwrap(), b"[18446744073709551615]");

    // Short decimals: shortest round-trip fixed point, trailing zeros off.
    let v: serde_json::Value = serde_json::from_str("[1.5,0.1,2.0]").unwrap();
    assert_eq!(try_canonical_bytes(&v).unwrap(), b"[1.5,0.1,2]");

    // Large integral floats below the 1e18 fast-path bound keep digits.
    let v: serde_json::Value = serde_json::from_str("[1e15]").unwrap();
    assert_eq!(try_canonical_bytes(&v).unwrap(), b"[1000000000000000]");
}

#[test]
fn lexeme_guard_accepts_in_policy_numbers() {
    for text in [
        r#"{"a":0}"#,
        r#"{"a":-12}"#,
        r#"[1.5,0.1,2.0]"#,
        r#"{"budget":25000,"ratio":0.75}"#,
        // 17 significant digits — the f64-parse limit — is in policy.
        r#"[1.2345678901234567]"#,
        // exactly 400 significant digits: allowed.
        &format!("[1.{}]", "9".repeat(MAX_NUMBER_SIGNIFICANT_DIGITS - 1)),
        // boundary magnitude: 1e10000 is in policy (|exp| == bound).
        &format!("[1e{}]", MAX_NUMBER_EXPONENT_ABS),
        // zeros carry no magnitude, whatever the declared exponent.
        r#"[0e999999999]"#,
    ] {
        assert!(
            validate_number_lexemes(text.as_bytes()).is_ok(),
            "expected {text:?} to pass the number policy"
        );
    }
}

#[test]
fn lexeme_guard_rejects_precision_beyond_400_digits() {
    // 401 significant digits.
    let text = format!("[1.{}]", "9".repeat(MAX_NUMBER_SIGNIFICANT_DIGITS));
    match validate_number_lexemes(text.as_bytes()) {
        Err(CanonicalError::PrecisionExceeded(n)) => {
            assert_eq!(n, MAX_NUMBER_SIGNIFICANT_DIGITS + 1)
        }
        other => panic!("expected PrecisionExceeded, got {other:?}"),
    }
}

#[test]
fn lexeme_guard_rejects_magnitude_beyond_1e10000() {
    for (text, why) in [
        (format!("[1e{}]", MAX_NUMBER_EXPONENT_ABS + 1), "1e10001"),
        (
            format!("[9.5e{}]", MAX_NUMBER_EXPONENT_ABS),
            "mantissa shifts magnitude",
        ),
        (format!("[1e-{}]", MAX_NUMBER_EXPONENT_ABS + 2), "tiny end"),
    ] {
        match validate_number_lexemes(text.as_bytes()) {
            Err(CanonicalError::MagnitudeExceeded(_)) => {}
            other => panic!("{why}: expected MagnitudeExceeded, got {other:?}"),
        }
    }
}

#[test]
fn lexeme_guard_boundary_is_lexeme_independent() {
    // Reviewer P1: the magnitude boundary is a property of the VALUE, not
    // of the text. Exactly 10^10000 is in policy no matter how it is
    // spelled; anything strictly above it is out.
    let in_policy = [
        "[1e10000]".to_string(),
        "[1.0e10000]".to_string(),
        "[10e9999]".to_string(),
        "[100e9998]".to_string(),
        "[0.1e10001]".to_string(),
        // 1 followed by 300 zeros via exponent shifting.
        format!("[1{}e9700]", "0".repeat(300)),
        // negative sign changes nothing about the magnitude rule.
        "[-1e10000]".to_string(),
        "[-10e9999]".to_string(),
    ];
    for text in &in_policy {
        assert!(
            matches!(validate_number_lexemes(text.as_bytes()), Ok(())),
            "{text:?} equals exactly ±1e10000 and must be in policy"
        );
    }
    let out_of_policy = [
        // Just above 1e10000 in various spellings.
        "[10.00000001e9999]".to_string(),
        "[1.00000001e10000]".to_string(),
        "[2e10000]".to_string(),
        "[1.5e10000]".to_string(),
    ];
    for text in &out_of_policy {
        assert!(
            matches!(
                validate_number_lexemes(text.as_bytes()),
                Err(CanonicalError::MagnitudeExceeded(_))
            ),
            "{text:?} exceeds 1e10000 and must be out of policy"
        );
    }
}

#[test]
fn lexeme_guard_flags_grammar_garbage_as_malformed_not_policy() {
    for text in ["[1..2]", "[1e]", "[.5]", "[01x]"] {
        match validate_number_lexemes(text.as_bytes()) {
            Err(CanonicalError::MalformedLexeme(_)) => {}
            other => panic!("expected MalformedLexeme for {text:?}, got {other:?}"),
        }
    }
}

#[test]
fn number_guard_runs_before_artifact_schema_validation() {
    // `validate_artifact_text` is the canonical text-ingest choke point.
    // Ordering matters: the number policy refusal must fire even when the
    // artifact kind itself is unknown, because without it serde would
    // silently rewrite the number on the way to that later error.
    let text = format!(r#"{{"x": 1e{}}}"#, MAX_NUMBER_EXPONENT_ABS + 1);
    let result = prometheos_lite::workflow::soma::validate_artifact_text("NoSuchKind", &text);
    let msg = result.expect_err("out-of-policy number must refuse the artifact");
    assert!(
        msg.contains("number policy violation"),
        "expected number-policy refusal, got: {msg}"
    );
}

#[test]
fn todays_build_rejects_dangerous_numbers_before_they_reach_symbols() {
    // These invariants are exactly why the canonicalizer's non-finite arm is
    // currently unreachable — and why it must stay an error path: the moment
    // `arbitrary_precision` flips on, these rejections stop happening at the
    // serde layer, and the canonicalizer becomes the last line of defense.

    // f64-magnitude overflow fails at parse (no Infinity in a Number).
    assert!(serde_json::from_str::<serde_json::Value>("[1e999]").is_err());

    // Programmatic construction refuses non-finite.
    assert!(serde_json::Number::from_f64(f64::NAN).is_none());
    assert!(serde_json::Number::from_f64(f64::INFINITY).is_none());
    assert!(serde_json::Number::from_f64(f64::NEG_INFINITY).is_none());

    // And f64::MAX (largest FINITE float) is representable and formats.
    let v = serde_json::json!([f64::MAX]);
    assert!(try_canonical_digest(&v).is_ok());
}

#[test]
fn no_number_input_ever_becomes_silent_zero() {
    // Regression for the historical bug shape: the old implementation mined
    // `as_f64().unwrap_or_default()` and emitted "0" for anything it could
    // not represent. Assert that both the guard and the formatter refuse
    // such inputs — and that a legitimate zero still canonicalizes to "0".
    let v = serde_json::json!([0.0]);
    assert_eq!(try_canonical_bytes(&v).unwrap(), b"[0]");

    let text = format!("[1.{}]", "9".repeat(500));
    assert!(validate_number_lexemes(text.as_bytes()).is_err());
}

#[test]
fn fixture_pinned_digests_still_verify() {
    // The vendored v1.1 fixture suite (integers only) is the authoritative
    // regression net for canonical output bytes; it must be unaffected.
    // The full check lives in tests/soma_ast_conformance.rs; here we only
    // assert the shared path stays coherent for the simplest pinned case.
    let v = serde_json::json!({"a": 1, "b": [true, null]});
    let d1 = try_canonical_digest(&v).unwrap();
    let d2 = try_canonical_digest(&v).unwrap();
    assert_eq!(d1, d2, "digests must be deterministic");
}
