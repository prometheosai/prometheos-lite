//! Canonical JSON serialization per SOMA canonicalization v1.0.0.
//!
//! Ported from the published reference implementation (`prometheosai/soma`,
//! crates/soma-canonical) so Lite's digests are byte-identical to the
//! normative bundle pins. Rules (vendored canonicalization.json):
//! lexicographic key order by raw UTF-8 bytes, no whitespace, UTF-8,
//! canonical-decimal numbers, null only where present, arrays in declared
//! order, digest = sha256(canonical bytes).
//!
//! Divergence note: upstream keeps original number lexemes via
//! `serde_json/arbitrary_precision`. Lite does not enable that feature
//! globally, so numbers are normalized from their parsed form: integers keep
//! exact digits; non-integral floats use shortest round-trip fixed-point.
//! The pinned fixtures contain only plain integers, where both policies
//! agree byte-for-byte (verified against manifest pins in conformance tests).
//!
//! Fail-closed number policy (2026-09-06, audit follow-up): the previous
//! implementation mapped any number that was not an `i64`/`u64` or a finite
//! `f64` to the string `"0"` — a silent substitution, not a failure. With
//! `arbitrary_precision` off, that branch is unreachable (serde_json rejects
//! f64-overflow at parse and `Number::from_f64` rejects non-finite), but the
//! branch arms the moment the feature is enabled. Formatting is therefore
//! fallible: [`try_canonical_bytes`] / [`try_canonical_digest`] surface
//! [`CanonicalError`] instead of ever substituting a placeholder value.
//! (Lite-constructed-record digest helpers use those via `.expect()` per
//! the call-site contract below.)
//!
//! Raw-text number guard: [`validate_number_lexemes`] scans a JSON document
//! BEFORE serde parses it and rejects number lexemes that Lite cannot
//! represent canonically-faithfully — more than 400 significant digits or a
//! magnitude beyond the SOMA DecimalV2 limit (|exponent| > 10000). Without
//! `arbitrary_precision`, serde_json silently truncates precision past
//! ~17 significant digits (e.g. `1.234567890123456789` →
//! `1.2345678901234567`) and rewrites lexemes (`1.10` → `1.1`), so THIS guard
//! is the only point where those inputs can be refused instead of silently
//! altered. Call it at every text-ingestion boundary; `soma::validate_artifact_text`
//! already does.

//! Call-site contract (two layers):
//!
//! - **Untrusted/external data** (audit, adapter conformance, event
//!   verification, anything parsed from outside this process) MUST use
//!   [`try_canonical_bytes`] / [`try_canonical_digest`] and propagate the
//!   error. A number-policy violation on external data means the record
//!   cannot be verified — fail closed.
//! - **Lite-constructed record helpers** (`compute_digest`/`sealed`-style
//!   functions across `workflow/*`) digest values this crate built itself.
//!   Those use the `try_` API with `expect`: with `arbitrary_precision` off,
//!   `serde_json` guarantees every `Number` is a finite f64/i64/u64 (parse
//!   rejects overflow; `Number::from_f64` rejects NaN/±∞; `json!` maps
//!   non-finite to `null`), so failure is *unreachable* — and if a future
//!   change (the feature flip, or an unsafe constructor) breaks that
//!   invariant, an immediate panic is strictly better than the old behavior
//!   of silently minting `"0"`. Converting ~80 such call sites to `Result`
//!   would be noise with no reachable behavior change; the lexeme guard
//!   below is where untrusted inputs are actually refused.

use sha2::{Digest as _, Sha256};

/// Maximum nesting depth, mirroring serde_json's recursion guard.
pub const MAX_DEPTH: usize = 128;

/// SOMA DecimalV2 number limits (see `soma-canonical/src/number.rs`).
/// Magnitudes are bounded by |base-10 exponent| <= 10000 and at most 400
/// significant digits; anything beyond is rejected, never approximated.
pub const MAX_NUMBER_EXPONENT_ABS: i64 = 10_000;
pub const MAX_NUMBER_SIGNIFICANT_DIGITS: usize = 400;

/// Number-policy violation discovered while canonicalizing.
///
/// Fail closed: every variant means "this number has no canonical form Lite
/// can faithfully produce", so the caller must refuse the input rather than
/// compute with a substituted value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonicalError {
    /// The number is not finite (NaN/±Infinity, or — once
    /// `serde_json/arbitrary_precision` is enabled — any lexeme too large to
    /// represent). Reachable in today's build only through programmatically
    /// constructed `Number`s; armed guard for the feature flip.
    NonFiniteNumber,
    /// |exponent| beyond 10^10000 (SOMA DecimalV2 magnitude limit).
    MagnitudeExceeded(String),
    /// More than 400 significant digits (SOMA DecimalV2 precision limit).
    PrecisionExceeded(usize),
    /// The input is not a valid JSON number lexeme (or the document is
    /// malformed in a way the number scan cannot attribute). Not a policy
    /// violation: refuse the input as malformed.
    MalformedLexeme(String),
}

impl std::fmt::Display for CanonicalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NonFiniteNumber => {
                write!(f, "number has no finite canonical form (NaN/±Inf/overflow)")
            }
            Self::MagnitudeExceeded(lexeme) => write!(
                f,
                "number magnitude beyond ±1e{MAX_NUMBER_EXPONENT_ABS}: {lexeme:?}"
            ),
            Self::PrecisionExceeded(digits) => write!(
                f,
                "number has {digits} significant digits; maximum is {MAX_NUMBER_SIGNIFICANT_DIGITS}"
            ),
            Self::MalformedLexeme(what) => write!(f, "not a valid JSON number lexeme: {what:?}"),
        }
    }
}

impl std::error::Error for CanonicalError {}

/// Canonical bytes for `value` under the decimal-v2 policy. Fail closed:
/// any number that violates the policy is an error, never a substituted
/// value.
pub fn try_canonical_bytes(value: &serde_json::Value) -> Result<Vec<u8>, CanonicalError> {
    let mut out = Vec::with_capacity(256);
    write_value(&mut out, value)?;
    Ok(out)
}

/// SHA-256 of [`try_canonical_bytes`], lowercase hex. Fail closed on any
/// number-policy violation.
pub fn try_canonical_digest(value: &serde_json::Value) -> Result<String, CanonicalError> {
    Ok(sha256_hex(&try_canonical_bytes(value)?))
}

/// Lowercase-hex sha256 over raw bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(64);
    for b in Sha256::digest(bytes) {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn write_value(out: &mut Vec<u8>, v: &serde_json::Value) -> Result<(), CanonicalError> {
    match v {
        serde_json::Value::Null => out.extend_from_slice(b"null"),
        serde_json::Value::Bool(true) => out.extend_from_slice(b"true"),
        serde_json::Value::Bool(false) => out.extend_from_slice(b"false"),
        serde_json::Value::Number(n) => out.extend_from_slice(format_number(n)?.as_bytes()),
        serde_json::Value::String(s) => write_escaped(out, s),
        serde_json::Value::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_value(out, item)?;
            }
            out.push(b']');
        }
        serde_json::Value::Object(map) => {
            // serde_json's default Map is BTreeMap: iteration order already
            // equals byte-lexicographic order. Sort defensively anyway so the
            // contract holds even if the feature set changes.
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_unstable_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
            out.push(b'{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_escaped(out, k);
                out.push(b':');
                write_value(out, &map[*k])?;
            }
            out.push(b'}');
        }
    }
    Ok(())
}

/// Format one parsed number under the decimal-v2 policy. Fail closed:
/// returns `Err(CanonicalError)` for any number Lite cannot represent
/// exactly, never a substituted value.
///
/// Reachability note: with `arbitrary_precision` off (today's build), serde
/// rejects f64-overflowing text at parse and `Number::from_f64` rejects
/// non-finite values, so the `Err` paths here cannot fire. They exist
/// because the feature flip (see module docs) would otherwise arm a silent
/// `"0"`-substitution landmine — the previous behavior this replaces.
fn format_number(n: &serde_json::Number) -> Result<String, CanonicalError> {
    if let Some(i) = n.as_i64() {
        return check_formatted(i.to_string());
    }
    if let Some(u) = n.as_u64() {
        return check_formatted(u.to_string());
    }
    let f = n.as_f64().ok_or(CanonicalError::NonFiniteNumber)?;
    if !f.is_finite() {
        return Err(CanonicalError::NonFiniteNumber);
    }
    if f == 0.0 {
        return Ok("0".into());
    }
    if f.fract() == 0.0 && f.abs() < 1e18 {
        return check_formatted(format!("{}", f as i128));
    }
    // Shortest round-trip decimal in fixed-point notation.
    let s = format!("{f}");
    if s.contains('.') {
        let trimmed = s.trim_end_matches('0');
        let trimmed = trimmed.strip_suffix('.').unwrap_or(trimmed);
        if trimmed.is_empty() || trimmed == "-" {
            return Ok("0".into());
        }
        return check_formatted(trimmed.to_string());
    }
    check_formatted(s)
}

/// Post-check the formatted lexeme against the SOMA DecimalV2 limits, so
/// every exit from [`format_number`] — present or future — passes the same
/// bounds as the raw-text guard below. (With `arbitrary_precision` off these
/// limits are unreachable from real f64-derived output: f64 yields at most 17
/// significant digits and |exponent| ≤ 308. They cost one scan and arm the
/// invariant for any future number path.)
fn check_formatted(s: String) -> Result<String, CanonicalError> {
    check_number_lexeme(&s)?;
    Ok(s)
}

/// JSON string escaping identical to Python `json.dumps(ensure_ascii=False)`
/// (the normative escaping table).
pub fn write_escaped(out: &mut Vec<u8>, s: &str) {
    out.push(b'"');
    for ch in s.chars() {
        match ch {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            '\u{8}' => out.extend_from_slice(b"\\b"),
            '\t' => out.extend_from_slice(b"\\t"),
            '\n' => out.extend_from_slice(b"\\n"),
            '\u{c}' => out.extend_from_slice(b"\\f"),
            '\r' => out.extend_from_slice(b"\\r"),
            c if (c as u32) < 0x20 => {
                const HEX: &[u8; 16] = b"0123456789abcdef";
                let n = c as u32;
                out.extend_from_slice(b"\\u00");
                out.push(HEX[((n >> 4) & 0xF) as usize]);
                out.push(HEX[(n & 0xF) as usize]);
            }
            c => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    out.push(b'"');
}

// ---------------------------------------------------------------------------
// Number lexeme validation (SOMA DecimalV2 limits)
// ---------------------------------------------------------------------------

/// Validate one JSON number lexeme against the SOMA DecimalV2 limits:
/// at most [`MAX_NUMBER_SIGNIFICANT_DIGITS`] significant digits and an
/// order of magnitude within ±[`MAX_NUMBER_EXPONENT_ABS`].
///
/// Significant-digit counting: digits of the concatenated integer and
/// fraction parts with leading zeros stripped; trailing zeros are counted
/// (strictest reading — a policy limit must never under-count). The value
/// zero carries no magnitude and is always accepted.
fn check_number_lexeme(lexeme: &str) -> Result<(), CanonicalError> {
    let b = lexeme.as_bytes();
    let mut i = 0usize;
    if b.first() == Some(&b'-') {
        i += 1;
    }
    let int_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let int_part = &lexeme[int_start..i];
    let mut frac_part = "";
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let fs = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        frac_part = &lexeme[fs..i];
    }
    if int_part.len() + frac_part.len() == 0 {
        return Err(CanonicalError::MalformedLexeme(lexeme.to_string()));
    }
    // A JSON number MUST have an integer part; `".5"` is not valid JSON.
    if int_part.is_empty() {
        return Err(CanonicalError::MalformedLexeme(lexeme.to_string()));
    }
    let mut exp: i64 = 0;
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        let exp_sign: i64 = if i < b.len() && b[i] == b'-' {
            i += 1;
            -1
        } else {
            if i < b.len() && b[i] == b'+' {
                i += 1;
            }
            1
        };
        let es = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if es == i {
            return Err(CanonicalError::MalformedLexeme(lexeme.to_string()));
        }
        // Saturating accumulate: anything beyond the bound plus headroom
        // is indistinguishable for the check.
        for &d in &b[es..i] {
            exp = exp.saturating_mul(10).saturating_add((d - b'0') as i64);
            if exp > MAX_NUMBER_EXPONENT_ABS * 2 {
                exp = MAX_NUMBER_EXPONENT_ABS * 2;
                break;
            }
        }
        exp *= exp_sign;
    }
    if i != b.len() {
        return Err(CanonicalError::MalformedLexeme(lexeme.to_string()));
    }

    let digits: String = int_part.chars().chain(frac_part.chars()).collect();
    let sig = digits.trim_start_matches('0');
    if sig.is_empty() {
        // Value is zero: no magnitude to bound.
        return Ok(());
    }
    if sig.len() > MAX_NUMBER_SIGNIFICANT_DIGITS {
        return Err(CanonicalError::PrecisionExceeded(sig.len()));
    }
    // Order of magnitude of the leading significant digit.
    let int_sig = int_part.trim_start_matches('0');
    let lead_e10: i64 = if !int_sig.is_empty() {
        exp.saturating_add(int_sig.len() as i64 - 1)
    } else {
        let frac_leading_zeros = (frac_part.len() - frac_part.trim_start_matches('0').len()) as i64;
        exp.saturating_sub(frac_leading_zeros + 1)
    };
    // SOMA DecimalV2: the VALUE must stay within (10^-10000, 10^10000).
    // lead_e10 > bound is definite overflow; lead_e10 == bound is only the
    // exact value 1e10000 (significand "1"); anything larger at that
    // exponent (e.g. 9.5e10000) exceeds the bound. Symmetric on the tiny
    // end.
    if !(-MAX_NUMBER_EXPONENT_ABS..=MAX_NUMBER_EXPONENT_ABS).contains(&lead_e10)
        || (lead_e10 == MAX_NUMBER_EXPONENT_ABS && sig != "1")
    {
        return Err(CanonicalError::MagnitudeExceeded(lexeme.to_string()));
    }
    Ok(())
}

/// Scan a raw JSON document and reject any number lexeme Lite cannot
/// represent canonically-faithfully (SOMA DecimalV2 limits). This is the
/// ONLY layer that sees the original number lexemes: with
/// `serde_json/arbitrary_precision` off, parsing silently rewrites lexemes
/// (`1.10` → `1.1`) and truncates beyond ~17 significant digits, so a guard
/// that runs post-parse cannot observe the violation.
///
/// Call this on every text-ingestion path whose numbers may reach a
/// canonical digest. Returns `Err(CanonicalError::MalformedLexeme)` for
/// documents whose numbers are not valid JSON number grammar at all; those
/// inputs are malformed documents, not policy violations — callers
/// typically report them identically (refuse the input).
pub fn validate_number_lexemes(bytes: &[u8]) -> Result<(), CanonicalError> {
    let mut p = Parser {
        b: bytes,
        i: 0,
        depth: 0,
        detect_dups: false,
    };
    match p.value_top(&mut Vec::new()) {
        Ok(()) => {
            p.ws();
            if p.i != p.b.len() {
                return Err(CanonicalError::MalformedLexeme(
                    String::from_utf8_lossy(&bytes[p.i..]).into_owned(),
                ));
            }
            Ok(())
        }
        // The number walker encodes violations as tagged strings; decode.
        Err(e) if e.starts_with("number-malformed") => Err(CanonicalError::MalformedLexeme(e)),
        Err(e) if e.starts_with("number-magnitude: ") => Err(CanonicalError::MagnitudeExceeded(
            e.trim_start_matches("number-magnitude: ").to_string(),
        )),
        Err(e) if e.starts_with("number-precision: ") => {
            let n: usize = e
                .trim_start_matches("number-precision: ")
                .parse()
                .unwrap_or(MAX_NUMBER_SIGNIFICANT_DIGITS + 1);
            Err(CanonicalError::PrecisionExceeded(n))
        }
        Err(e) => Err(CanonicalError::MalformedLexeme(e)),
    }
}
// ---------------------------------------------------------------------------
// Duplicate-key scan (SOMA-CMP-0007)
// ---------------------------------------------------------------------------

/// Failure mode of the strict structural scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanError {
    /// The document is malformed; nothing about duplicate-freeness is known.
    Malformed,
}

/// Returns the first duplicated object key, `Ok(None)` when clean, or
/// `Err(Malformed)` when the document cannot be trusted. Fail closed:
/// malformed input is never treated as duplicate-free.
///
/// Ported from the published verifier (`soma_canonical::find_duplicate_key`):
/// escape-aware recursive walker over raw bytes with a depth guard.
pub fn find_duplicate_key(bytes: &[u8]) -> Result<Option<String>, ScanError> {
    let mut p = Parser {
        b: bytes,
        i: 0,
        depth: 0,
        detect_dups: true,
    };
    match p.value_top(&mut Vec::new()) {
        Ok(()) => {}
        Err(e) if e.starts_with("duplicate key") => return Ok(Some(e)),
        Err(_) => return Err(ScanError::Malformed),
    }
    p.ws();
    if p.i != p.b.len() {
        return Err(ScanError::Malformed);
    }
    Ok(None)
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
    depth: usize,
    /// When false, duplicate keys are not tracked (the number-policy scan
    /// reuses this walker; duplicate reports belong to `find_duplicate_key`
    /// alone so each guard owns exactly one diagnostic family).
    detect_dups: bool,
}

type Dup = String;

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn expect(&mut self, c: u8) -> Result<(), Dup> {
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err("malformed".into())
        }
    }

    fn literal(&mut self, lit: &[u8]) -> Result<(), Dup> {
        if self.b.len() >= self.i + lit.len() && &self.b[self.i..self.i + lit.len()] == lit {
            self.i += lit.len();
            Ok(())
        } else {
            Err("malformed".into())
        }
    }

    fn string(&mut self) -> Result<String, Dup> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let c = self.peek().ok_or_else(|| "unterminated".to_string())?;
            self.i += 1;
            match c {
                b'"' => return Ok(out),
                b'\\' => {
                    let e = self.peek().ok_or_else(|| "bad escape".to_string())?;
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let v = self.hex4()?;
                            if (0xD800..=0xDBFF).contains(&v)
                                && self.b.get(self.i) == Some(&b'\\')
                                && self.b.get(self.i + 1) == Some(&b'u')
                            {
                                let save = self.i;
                                self.i += 2;
                                match self.hex4() {
                                    Ok(lo) if (0xDC00..=0xDFFF).contains(&lo) => {
                                        let c = 0x10000
                                            + ((v as u32 - 0xD800) << 10)
                                            + (lo as u32 - 0xDC00);
                                        out.push(char::from_u32(c).unwrap_or('\u{fffd}'));
                                    }
                                    _ => {
                                        self.i = save;
                                        out.push('\u{fffd}');
                                    }
                                }
                            } else if (0xD800..=0xDFFF).contains(&v) {
                                out.push('\u{fffd}');
                            } else {
                                out.push(char::from_u32(v as u32).unwrap_or('\u{fffd}'));
                            }
                        }
                        _ => return Err("bad escape".into()),
                    }
                }
                _ => {
                    let start = self.i - 1;
                    let len = utf8_len(c);
                    let end = start + len;
                    if end > self.b.len() {
                        return Err("bad utf8".into());
                    }
                    let s = std::str::from_utf8(&self.b[start..end])
                        .map_err(|_| "bad utf8".to_string())?;
                    out.push_str(s);
                    self.i = end;
                }
            }
        }
    }

    fn hex4(&mut self) -> Result<u16, Dup> {
        let h = self
            .b
            .get(self.i..self.i + 4)
            .ok_or_else(|| "bad u".to_string())?;
        self.i += 4;
        std::str::from_utf8(h)
            .ok()
            .and_then(|s| u16::from_str_radix(s, 16).ok())
            .ok_or_else(|| "bad hex".to_string())
    }

    fn number(&mut self) -> Result<(), Dup> {
        let start = self.i;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.' | b'e' | b'E') {
                self.i += 1;
            } else {
                break;
            }
        }
        if start == self.i {
            return Err("expected number".into());
        }
        if self.detect_dups {
            // Duplicate-key scan only: policy checks belong to the number
            // guard (`validate_number_lexemes`); keeping this branch loose
            // preserves find_duplicate_key's pre-existing charter
            // byte-for-byte (its blanket Malformed mapping must not start
            // swallowing number-policy signals).
            return Ok(());
        }
        // Number-policy check (SOMA DecimalV2): the lexeme still exists at
        // this layer, so this is the only place >17-digit truncation or
        // magnitude overflow can be refused instead of silently altered.
        // Encoded as tagged strings so `find_duplicate_key` can keep its
        // blanket "not a duplicate-key error => malformed" mapping while
        // [`validate_number_lexemes`] decodes the precise violation.
        let lexeme =
            std::str::from_utf8(&self.b[start..self.i]).map_err(|_| "bad utf8".to_string())?;
        match check_number_lexeme(lexeme) {
            Ok(()) => Ok(()),
            Err(CanonicalError::MalformedLexeme(_)) => Err("number-malformed".to_string()),
            Err(CanonicalError::MagnitudeExceeded(_)) => Err(format!("number-magnitude: {lexeme}")),
            Err(CanonicalError::PrecisionExceeded(n)) => Err(format!("number-precision: {n}")),
            Err(CanonicalError::NonFiniteNumber) => {
                // No lexeme form can be non-finite; unreachable, fail closed.
                Err("number-malformed".to_string())
            }
        }
    }

    fn value_top(&mut self, stack: &mut Vec<Vec<String>>) -> Result<(), Dup> {
        self.ws();
        match self.peek().ok_or_else(|| "empty".to_string())? {
            b'{' => self.object(stack),
            b'[' => self.array(stack),
            b'"' => self.string().map(|_| ()),
            b't' => self.literal(b"true"),
            b'f' => self.literal(b"false"),
            b'n' => self.literal(b"null"),
            _ => self.number(),
        }
    }

    fn array(&mut self, stack: &mut Vec<Vec<String>>) -> Result<(), Dup> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            self.depth -= 1;
            return Err("max depth".into());
        }
        self.expect(b'[')?;
        self.ws();
        if self.peek() == Some(b']') {
            self.i += 1;
            self.depth -= 1;
            return Ok(());
        }
        loop {
            self.value_top(stack)?;
            self.ws();
            match self
                .peek()
                .ok_or_else(|| "unterminated array".to_string())?
            {
                b',' => self.i += 1,
                b']' => {
                    self.i += 1;
                    self.depth -= 1;
                    return Ok(());
                }
                _ => {
                    self.depth -= 1;
                    return Err("array sep".into());
                }
            }
        }
    }

    fn object(&mut self, stack: &mut Vec<Vec<String>>) -> Result<(), Dup> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            self.depth -= 1;
            return Err("max depth".into());
        }
        self.expect(b'{')?;
        stack.push(Vec::new());
        self.ws();
        if self.peek() == Some(b'}') {
            self.i += 1;
            stack.pop();
            self.depth -= 1;
            return Ok(());
        }
        loop {
            self.ws();
            let key = self.string()?;
            self.ws();
            self.expect(b':')?;
            {
                if self.detect_dups {
                    let keys = stack.last_mut().ok_or_else(|| "depth".to_string())?;
                    if keys.iter().any(|k| k == &key) {
                        stack.pop();
                        self.depth -= 1;
                        return Err(format!("duplicate key {key:?}"));
                    }
                    keys.push(key);
                }
            }
            self.value_top(stack)?;
            self.ws();
            match self
                .peek()
                .ok_or_else(|| "unterminated object".to_string())?
            {
                b',' => self.i += 1,
                b'}' => {
                    self.i += 1;
                    stack.pop();
                    self.depth -= 1;
                    return Ok(());
                }
                _ => {
                    stack.pop();
                    self.depth -= 1;
                    return Err("object sep".into());
                }
            }
        }
    }
}

fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}
