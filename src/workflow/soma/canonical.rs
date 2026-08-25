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

use sha2::{Digest as _, Sha256};

/// Maximum nesting depth, mirroring serde_json's recursion guard.
pub const MAX_DEPTH: usize = 128;

/// Canonical bytes for `value` under the decimal-v2 policy.
pub fn canonical_bytes(value: &serde_json::Value) -> Vec<u8> {
    let mut out = Vec::with_capacity(256);
    write_value(&mut out, value);
    out
}

/// SHA-256 of [`canonical_bytes`], lowercase hex.
pub fn canonical_digest(value: &serde_json::Value) -> String {
    sha256_hex(&canonical_bytes(value))
}

/// Lowercase-hex sha256 over raw bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(64);
    for b in Sha256::digest(bytes) {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn write_value(out: &mut Vec<u8>, v: &serde_json::Value) {
    match v {
        serde_json::Value::Null => out.extend_from_slice(b"null"),
        serde_json::Value::Bool(true) => out.extend_from_slice(b"true"),
        serde_json::Value::Bool(false) => out.extend_from_slice(b"false"),
        serde_json::Value::Number(n) => out.extend_from_slice(format_number(n).as_bytes()),
        serde_json::Value::String(s) => write_escaped(out, s),
        serde_json::Value::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_value(out, item);
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
                write_value(out, &map[*k]);
            }
            out.push(b'}');
        }
    }
}

fn format_number(n: &serde_json::Number) -> String {
    if let Some(i) = n.as_i64() {
        return i.to_string();
    }
    if let Some(u) = n.as_u64() {
        return u.to_string();
    }
    let f = n.as_f64().unwrap_or_default();
    if !f.is_finite() {
        // Fail closed: non-finite values have no canonical form.
        return "0".into();
    }
    if f == 0.0 {
        return "0".into();
    }
    if f.fract() == 0.0 && f.abs() < 1e18 {
        return format!("{}", f as i128);
    }
    // Shortest round-trip decimal in fixed-point notation.
    let s = format!("{f}");
    if s.contains('.') {
        let trimmed = s.trim_end_matches('0');
        let trimmed = trimmed.strip_suffix('.').unwrap_or(trimmed);
        if trimmed.is_empty() || trimmed == "-" {
            return "0".into();
        }
        return trimmed.to_string();
    }
    s
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
            Err("expected number".into())
        } else {
            Ok(())
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
                let keys = stack.last_mut().ok_or_else(|| "depth".to_string())?;
                if keys.iter().any(|k| k == &key) {
                    stack.pop();
                    self.depth -= 1;
                    return Err(format!("duplicate key {key:?}"));
                }
                keys.push(key);
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
