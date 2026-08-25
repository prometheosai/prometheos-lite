//! Shared primitives for the SOMA contract models: strict semantic
//! versions, hex digests, outcome variants, execution classes, and the
//! fail-closed numeric comparator. Ported from the published reference
//! implementation (`prometheosai/soma`, crates/soma-types + soma-validate).

use std::fmt;

// ---------------------------------------------------------------------------
// SemVer
// ---------------------------------------------------------------------------

/// Strict `MAJOR.MINOR.PATCH` semantic version.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SemVer {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemVerError(pub String);

impl fmt::Display for SemVerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid semantic version {:?}: expected MAJOR.MINOR.PATCH",
            self.0
        )
    }
}

impl SemVer {
    /// Strict parsing: no whitespace, no pre-release tags, no partial
    /// versions, no leading zeros.
    pub fn parse(input: &str) -> Result<Self, SemVerError> {
        let invalid = || SemVerError(input.to_string());
        let mut parts = input.split('.');
        let mut next = || -> Result<u64, SemVerError> {
            let raw = parts.next().ok_or_else(invalid)?;
            if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
                return Err(invalid());
            }
            if raw.len() > 1 && raw.starts_with('0') {
                return Err(invalid());
            }
            raw.parse::<u64>().map_err(|_| invalid())
        };
        let major = next()?;
        let minor = next()?;
        let patch = next()?;
        if parts.next().is_some() {
            return Err(invalid());
        }
        Ok(Self {
            major,
            minor,
            patch,
        })
    }

    /// Backward-compatibility rule: an artifact version is usable under
    /// `supported` iff it is not newer (fail closed on newer).
    pub fn is_compatible_with(&self, supported: &SemVer) -> bool {
        self <= supported
    }
}

impl fmt::Display for SemVer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

// ---------------------------------------------------------------------------
// Hex64
// ---------------------------------------------------------------------------

/// A lowercase SHA-256 hex digest (`^[0-9a-f]{64}$`), validated on parse.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Hex64(String);

impl Hex64 {
    pub fn parse(s: &str) -> Result<Self, String> {
        if s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            Ok(Self(s.to_string()))
        } else {
            Err(format!(
                "invalid sha256 digest {s:?}: expected 64 lowercase hex chars"
            ))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl serde::Serialize for Hex64 {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> serde::Deserialize<'de> for Hex64 {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Hex64::parse(&s).map_err(serde::de::Error::custom)
    }
}

// ---------------------------------------------------------------------------
// Outcome variants / execution class / mutation mode
// ---------------------------------------------------------------------------

macro_rules! string_enum {
    ($name:ident, $( $variant:ident => $text:literal ),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name { $($variant),+ }

        impl $name {
            pub const fn as_str(&self) -> &'static str {
                match self { $( $name::$variant => $text ),+ }
            }

            pub fn all() -> &'static [$name] {
                const ALL: &[$name] = &[ $( $name::$variant ),+ ];
                ALL
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
                D::Error: serde::de::Error,
            {
                let s = String::deserialize(deserializer)?;
                match $name::all().iter().find(|v| v.as_str() == s) {
                    Some(v) => Ok(*v),
                    None => Err(serde::de::Error::custom(format!(
                        concat!("unknown ", stringify!($name), " {:?}"),
                        s
                    ))),
                }
            }
        }
    };
}

string_enum!(
    OutcomeVariant,
    Produced => "Produced",
    Skipped => "Skipped",
    Blocked => "Blocked",
    Failed => "Failed",
    Cancelled => "Cancelled",
    ReviewRequired => "ReviewRequired",
);

string_enum!(
    ExecutionClass,
    Deterministic => "deterministic",
    ModelAssisted => "model-assisted",
    OpenEnded => "open-ended",
);

string_enum!(MutationMode, None_ => "none", Explicit => "explicit");

string_enum!(NetworkDefault, Allow => "allow", Deny => "deny");

string_enum!(
    Direction,
    Input => "input",
    Output => "output",
);

string_enum!(
    Cardinality,
    Single => "single",
    Multi => "multi",
);

string_enum!(
    Requiredness,
    Required => "required",
    Optional => "optional",
);

string_enum!(ConstraintKind, Static => "static", Dynamic => "dynamic");

string_enum!(WorkflowKind, Atomic => "atomic", Composite => "composite");

string_enum!(MappingStrategy, Identity => "identity", Canonical => "canonical", Adapter => "adapter");

/// Variants that semantically represent failure-like outcomes.
pub const FAILURE_VARIANTS: &[OutcomeVariant] = &[
    OutcomeVariant::Failed,
    OutcomeVariant::Blocked,
    OutcomeVariant::Cancelled,
];

/// The only success-like variant.
pub const SUCCESS_VARIANT: OutcomeVariant = OutcomeVariant::Produced;

/// SOMA type-vocabulary check (`SOMA-CMP-0006`): a type is either a
/// primitive, a CamelCase nominal identifier, or a single-level generic.
pub fn type_in_vocabulary(t: &str) -> bool {
    const PRIMITIVES: [&str; 5] = ["string", "number", "boolean", "object", "any"];
    if PRIMITIVES.contains(&t) {
        return true;
    }
    if let Some(rest) = t.strip_prefix("List<").and_then(|r| r.strip_suffix('>')) {
        // single-level generic only
        return !rest.contains('<')
            && rest.chars().next().is_some_and(|c| c.is_ascii_uppercase())
            && rest.chars().all(|c| c.is_ascii_alphanumeric());
    }
    let mut chars = t.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_uppercase())
        && chars.all(|c| c.is_ascii_alphanumeric())
}

// ---------------------------------------------------------------------------
// Exact numeric comparison (fail closed)
// ---------------------------------------------------------------------------

fn split_fixed(s: &str) -> (String, String) {
    match s.split_once('.') {
        Some((i, f)) => (i.to_string(), f.to_string()),
        None => (s.to_string(), String::new()),
    }
}

/// Exact magnitude comparison of two NON-NEGATIVE fixed-point strings.
fn mag_gt(a: &str, b: &str) -> bool {
    let (ia, fa) = split_fixed(a);
    let (ib, fb) = split_fixed(b);
    let int_cmp = cmp_zero_stripped(&ia, &ib);
    if int_cmp != std::cmp::Ordering::Equal {
        return int_cmp == std::cmp::Ordering::Greater;
    }
    let width = fa.len().max(fb.len());
    let fa_p = format!("{:0<width$}", fa, width = width);
    let fb_p = format!("{:0<width$}", fb, width = width);
    fa_p > fb_p
}

fn cmp_zero_stripped(a: &str, b: &str) -> std::cmp::Ordering {
    let az = a.trim_start_matches('0');
    let bz = b.trim_start_matches('0');
    match (az.is_empty(), bz.is_empty()) {
        (true, true) => std::cmp::Ordering::Equal,
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        (false, false) => az.len().cmp(&bz.len()).then_with(|| az.cmp(bz)),
    }
}

/// Outcome of an exact numeric comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmp {
    Greater,
    NotGreater,
    /// One of the lexemes violates the number policy. Callers MUST fail
    /// closed (treat as a violation), never silently skip the check.
    Incomparable,
}

/// Normalize a JSON number to canonical fixed-point text (see
/// [`crate::workflow::soma::canonical`] for the policy note).
fn fixed_lexeme(n: &serde_json::Number) -> Option<String> {
    let rendered = if let Some(i) = n.as_i64() {
        i.to_string()
    } else if let Some(u) = n.as_u64() {
        u.to_string()
    } else {
        let f = n.as_f64()?;
        if !f.is_finite() {
            return None;
        }
        if f == 0.0 {
            "0".to_string()
        } else if f.fract() == 0.0 && f.abs() < 1e18 {
            format!("{}", f as i128)
        } else {
            format!("{f}")
        }
    };
    // Fold exponent-free forms through the decimal normalizer invariants:
    // strip trailing fractional zeros and a bare ".0".
    let s = rendered;
    if s.contains('.') {
        let trimmed = s.trim_end_matches('0');
        let trimmed = trimmed.strip_suffix('.').unwrap_or(trimmed);
        if trimmed.is_empty() || trimmed == "-" {
            return Some("0".into());
        }
        return Some(trimmed.to_string());
    }
    Some(s)
}

/// Exact strict greater-than over two JSON numbers.
pub fn number_cmp(a: &serde_json::Number, b: &serde_json::Number) -> Cmp {
    let (fa, fb) = match (fixed_lexeme(a), fixed_lexeme(b)) {
        (Some(fa), Some(fb)) => (fa, fb),
        _ => return Cmp::Incomparable,
    };
    match (fa.starts_with('-'), fb.starts_with('-')) {
        (false, true) => Cmp::Greater,
        (true, false) => Cmp::NotGreater,
        (false, false) => {
            if mag_gt(&fa, &fb) {
                Cmp::Greater
            } else {
                Cmp::NotGreater
            }
        }
        (true, true) => {
            if mag_gt(fb.trim_start_matches('-'), fa.trim_start_matches('-')) {
                Cmp::Greater
            } else {
                Cmp::NotGreater
            }
        }
    }
}

/// Sum JSON numbers exactly where possible; `None` = uncountable (overflow /
/// out-of-policy lexeme). Callers fail closed on `None`.
pub fn sum_numbers(vals: Vec<&serde_json::Number>) -> Option<serde_json::Number> {
    use std::str::FromStr as _;
    let all_int = vals.iter().all(|v| v.is_i64() || v.is_u64());
    if all_int {
        let mut acc: i128 = 0;
        for v in vals {
            let x = if let Some(i) = v.as_i64() {
                i as i128
            } else {
                v.as_u64()? as i128
            };
            acc = acc.checked_add(x)?;
        }
        return serde_json::Number::from_str(&acc.to_string()).ok();
    }
    // Decimal-exact via scaled fixed-point string arithmetic.
    let fixed: Vec<String> = vals
        .iter()
        .map(|v| fixed_lexeme(v))
        .collect::<Option<_>>()?;
    let max_frac = fixed
        .iter()
        .map(|s| s.split('.').nth(1).map(str::len).unwrap_or(0))
        .max()?;
    let mut acc: i128 = 0;
    for s in &fixed {
        let neg = s.starts_with('-');
        let body = s.trim_start_matches('-');
        let (int_part, frac_part) = match body.split_once('.') {
            Some((i, f)) => (i.to_string(), f.to_string()),
            None => (body.to_string(), "0".repeat(max_frac)),
        };
        let int_padded = format!(
            "{:0>width$}",
            int_part,
            width = max_frac.saturating_sub(int_part.len()) + 1
        );
        let frac_padded = format!("{:0<width$}", frac_part, width = max_frac);
        let scaled = format!("{int_padded}{frac_padded}");
        let magnitude = i128::from_str(&scaled).ok()?;
        let signed = if neg { -magnitude } else { magnitude };
        acc = acc.checked_add(signed)?;
    }
    let sign = if acc < 0 { "-" } else { "" };
    let digits = acc.unsigned_abs().to_string();
    let num = if digits.len() <= max_frac {
        let frac = format!("{:0>width$}", digits, width = max_frac);
        format!("0.{frac}")
    } else {
        let (i, f) = digits.split_at(digits.len() - max_frac);
        format!("{sign}{i}.{f}")
    };
    serde_json::Number::from_str(&num).ok()
}
