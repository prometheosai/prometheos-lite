//! Central, deterministic secret redaction and rejection.
//!
//! Persisted diagnostics (provider errors, validation stdout/stderr, validation
//! command recorded in evidence, URLs/routes, Markdown reports) may contain
//! secrets. This module provides a single facility so redactions are not
//! scattered as ad-hoc `.replace()` calls across the runtime.
//!
//! # Semantic vs diagnostic
//!
//! Redaction is for *diagnostic* text. A secret embedded in a *semantic*
//! artifact (for example a generated patch that contains the configured API
//! key) must **not** be redacted-and-published: rewriting the secret into
//! `<redacted>` changes the executable meaning of the artifact. Such artifacts
//! are rejected by [`reject_secret_bearing_patch`] before persistence.

use anyhow::{Result, bail};
use lazy_static::lazy_static;
use regex::Regex;
use std::path::Path;
use std::sync::Arc;

/// Stable placeholder substituted for any redacted secret.
pub const REDACTED_PLACEHOLDER: &str = "<redacted>";

/// Fixed fake credential used by tests to prove secrets never reach persisted
/// output. It must appear ZERO times in any JSON/Markdown/raw-log/journal
/// diagnostic produced by the runtime.
pub const SECRET_CANARY: &str = "PROMETHEOS_TEST_SECRET_DO_NOT_PERSIST_9f83c0ffee";

/// Shared, compiled redaction patterns.
struct Patterns {
    url_userinfo: Regex,
    auth_header: Regex,
    bearer: Regex,
    json_credential: Regex,
    query_credential: Regex,
}

lazy_static! {
    static ref PATTERNS: Patterns = Patterns {
        // scheme://user:secret@host -> scheme://user:<redacted>@host
        url_userinfo: Regex::new(
            r"(?P<pre>[a-zA-Z][a-zA-Z0-9+.\-]*://[^/\s:@]+:)[^/\s:@]+(?P<post>@[^/\s]+)"
        )
        .expect("valid url_userinfo regex"),
        // Authorization: <token> / Authorization= <token>, including the
        // "Authorization: Bearer <token>" / "Basic <token>" forms as one unit.
        auth_header: Regex::new(
            r"(?i)(authorization\s*[:=]\s*)(?:(?:bearer|basic)\s+[^\s,;']+|[^\s,;']+)",
        )
        .expect("valid auth_header regex"),
        // Bearer/Basic <token>
        bearer: Regex::new(r"(?i)\b(Bearer|Basic)\s+[A-Za-z0-9._~+/=-]+")
            .expect("valid bearer regex"),
        // "api_key": "value" style JSON credentials
        json_credential: Regex::new(
            r#"(?i)("(?:api[_]?key|apikey|token|secret|password|access[_]?key|client[_]?secret|private[_]?key)"\s*:\s*")[^"]*(")"#
        )
        .expect("valid json_credential regex"),
        // ?api_key=value / &token=value style query credentials
        query_credential: Regex::new(
            r"(?i)([?&](?:api[_]?key|apikey|token|secret|password|access[_]?key|client[_]?secret)=)[^&\s]+"
        )
        .expect("valid query_credential regex"),
    };
}

/// A redactor that deterministically removes known and credential-shaped
/// secrets from diagnostic text, preserving useful surrounding context.
#[derive(Clone)]
pub struct Redactor {
    known_secrets: Arc<Vec<String>>,
}

impl Default for Redactor {
    fn default() -> Self {
        Redactor {
            known_secrets: Arc::new(Vec::new()),
        }
    }
}

impl Redactor {
    /// Create a redactor with no known literal secrets (only credential-shaped
    /// patterns are matched).
    pub fn new() -> Self {
        Self::default()
    }

    /// Register explicit known secret values (configured provider credentials,
    /// test canaries). Any occurrence in diagnostic text is replaced verbatim.
    pub fn with_known_secrets(mut self, secrets: &[String]) -> Self {
        let mut merged: Vec<String> = (*self.known_secrets).clone();
        for s in secrets {
            if !s.is_empty() && !merged.contains(s) {
                merged.push(s.clone());
            }
        }
        self.known_secrets = Arc::new(merged);
        self
    }

    /// Redact diagnostic `text`, returning a copy with secrets removed.
    ///
    /// Order: known literal secrets first (exact match), then credential-shaped
    /// patterns. Replacement is always [`REDACTED_PLACEHOLDER`].
    pub fn redact(&self, text: &str) -> String {
        let mut out = text.to_string();
        for secret in self.known_secrets.iter() {
            if !secret.is_empty() {
                out = out.replace(secret, REDACTED_PLACEHOLDER);
            }
        }
        let p = &*PATTERNS;
        out = p
            .url_userinfo
            .replace_all(&out, "$pre<redacted>$post")
            .into_owned();
        out = p.auth_header.replace_all(&out, "$1<redacted>").into_owned();
        out = p.bearer.replace_all(&out, "$1 <redacted>").into_owned();
        out = p
            .json_credential
            .replace_all(&out, "$1<redacted>$2")
            .into_owned();
        out = p
            .query_credential
            .replace_all(&out, "$1<redacted>")
            .into_owned();
        out
    }

    /// Fail closed if a *semantic* artifact (such as a generated patch) contains
    /// a known secret. Such artifacts must be rejected, never redacted-and-
    /// published, because redaction would silently change their meaning.
    pub fn reject_secret_bearing_patch(&self, patch: &str) -> Result<()> {
        for secret in self.known_secrets.iter() {
            if !secret.is_empty() && patch.contains(secret) {
                bail!(
                    "rejected semantic artifact: it contains a known secret and must not be persisted"
                );
            }
        }
        Ok(())
    }
}

/// Convenience: redact `text` using a redactor seeded with the given known
/// secrets (and the standard credential-shaped patterns).
pub fn redact_diagnostics(text: &str, known_secrets: &[String]) -> String {
    Redactor::new()
        .with_known_secrets(known_secrets)
        .redact(text)
}

/// Collect the known secrets that must be redacted from persisted diagnostics.
///
/// Sources, in order:
/// 1. `PROMETHEOS_KNOWN_SECRETS` — comma/newline/semicolon separated values.
/// 2. A repo-local `.prometheos/known_secrets` file (one value per line, `#`
///    comments and blank lines ignored). This file is operator-maintained and
///    must be git-ignored by the user; it is never persisted by this tool.
///
/// This is the production wiring for [`Redactor::with_known_secrets`]: the
/// orchestrator passes the result into isolated validation so that any secret the
/// operator has declared never reaches persisted evidence or raw logs.
pub fn collect_known_secrets(repo: &Path) -> Vec<String> {
    let mut secrets: Vec<String> = Vec::new();
    let mut push = |s: &str| {
        let s = s.trim();
        if !s.is_empty() && !secrets.contains(&s.to_string()) {
            secrets.push(s.to_string());
        }
    };
    if let Ok(v) = std::env::var("PROMETHEOS_KNOWN_SECRETS") {
        for part in v.split(|c| [',', '\n', ';'].contains(&c)) {
            push(part);
        }
    }
    let file = repo.join(".prometheos").join("known_secrets");
    if let Ok(content) = std::fs::read_to_string(&file) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            push(line);
        }
    }
    secrets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_known_secret_verbatim() {
        let r = Redactor::new().with_known_secrets(&[SECRET_CANARY.to_string()]);
        let out = r.redact(&format!("error: {SECRET_CANARY} leaked"));
        assert!(!out.contains(SECRET_CANARY));
        assert!(out.contains(REDACTED_PLACEHOLDER));
        assert!(out.contains("error:"));
    }

    #[test]
    fn redacts_url_userinfo() {
        let r = Redactor::new();
        let out = r.redact("fetch https://alice:s3cr3t@example.com/api");
        assert!(!out.contains("s3cr3t"));
        assert!(out.contains("alice:<redacted>@example.com"));
    }

    #[test]
    fn redacts_auth_header_and_bearer() {
        let r = Redactor::new();
        let out = r.redact("Authorization: Bearer tok-12345 then Basic abc");
        assert!(!out.contains("tok-12345"));
        assert!(!out.contains("abc"));
        // The whole Authorization header (including the Bearer token) is one
        // redaction unit; the trailing Basic credential is caught separately.
        assert!(out.contains("Authorization: <redacted>"));
        assert!(out.contains("Basic <redacted>"));
    }

    #[test]
    fn redacts_json_and_query_credentials() {
        let r = Redactor::new();
        let in_json = r#"{"api_key":"k-123","password":"p-456"}"#;
        let out = r.redact(in_json);
        assert!(!out.contains("k-123"));
        assert!(!out.contains("p-456"));
        assert!(out.contains("\"api_key\":\"<redacted>\""));
        let out2 = r.redact("https://x.com/path?token=ttt&safe=1");
        assert!(!out2.contains("ttt"));
        assert!(out2.contains("safe=1"));
    }

    #[test]
    fn canary_is_fully_redacted() {
        let r = Redactor::new().with_known_secrets(&[SECRET_CANARY.to_string()]);
        let diag = format!(
            "provider error with {canary} and also url https://u:{canary}@host and Bearer {canary}",
            canary = SECRET_CANARY
        );
        let out = r.redact(&diag);
        assert_eq!(out.matches(SECRET_CANARY).count(), 0);
        assert!(out.contains("provider error"));
        assert!(out.contains("url"));
    }

    #[test]
    fn rejects_secret_bearing_patch() {
        let r = Redactor::new().with_known_secrets(&[SECRET_CANARY.to_string()]);
        let patch = format!("--- a/x\n+++ b/x\n+secret={}\n", SECRET_CANARY);
        assert!(r.reject_secret_bearing_patch(&patch).is_err());
        assert!(
            r.reject_secret_bearing_patch("--- a/x\n+++ b/x\n+ok\n")
                .is_ok()
        );
    }
}
