//! Library-only, read-only observation contract for PrometheOS OS integration.
//!
//! Wire contract: `prometheos.os-observation.v1`. This module defines request
//! shapes, version negotiation, refusal semantics and cursor/scope rules. It
//! deliberately contains NO transport, credential issuer, owner-registry
//! activation, execution path, protected mutation or source-trust clearance.
//! Nothing here can grant runtime authority; every caller must still pass the
//! separate owner/Principal security acceptance before any binding.

pub const CONTRACT_VERSION: &str = "prometheos.os-observation.v1";
pub const SUPPORTED_VERSIONS: [&str; 1] = [CONTRACT_VERSION];
pub const MAX_PAGE_LIMIT: u64 = 512;
pub const OBSERVE_EVENTS: &str = "observe:work-events";
pub const OBSERVE_CAPABILITIES: &str = "observe:capabilities";

/// Execution/protected-mutation scope names that an observation credential
/// must never satisfy. This is a closed deny list, never a default grant.
pub const FORBIDDEN_SCOPES: [&str; 6] = [
    "execute",
    "execute:work",
    "write",
    "patch:file",
    "approve:gate",
    "provision:owner-registry",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionDecision {
    Supported(&'static str),
    Unsupported,
    Missing,
    Malformed,
}

/// Explicit negotiation: no silent downgrade and no default version.
pub fn negotiate_version(requested: Option<&str>) -> VersionDecision {
    match requested {
        None => VersionDecision::Missing,
        Some(value) if value.is_empty() || value.chars().any(char::is_whitespace) => {
            VersionDecision::Malformed
        }
        Some(value) if SUPPORTED_VERSIONS.contains(&value) => {
            VersionDecision::Supported(CONTRACT_VERSION)
        }
        Some(_) => VersionDecision::Unsupported,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    MissingVersion,
    MalformedVersion,
    UnsupportedVersion,
    MalformedCursor,
    MalformedLimit,
    ExpiredCredential,
    RevokedCredential,
    UnauthorizedScope,
    SourceRevisionMismatch,
    SequenceGap,
}

pub fn refusal_code(refusal: Refusal) -> &'static str {
    match refusal {
        Refusal::MissingVersion => "missing_version",
        Refusal::MalformedVersion => "malformed_version",
        Refusal::UnsupportedVersion => "unsupported_version",
        Refusal::MalformedCursor => "malformed_cursor",
        Refusal::MalformedLimit => "malformed_limit",
        Refusal::ExpiredCredential => "credential_expired",
        Refusal::RevokedCredential => "credential_revoked",
        Refusal::UnauthorizedScope => "unauthorized_scope",
        Refusal::SourceRevisionMismatch => "source_revision_mismatch",
        Refusal::SequenceGap => "sequence_gap",
    }
}

/// A read-only credential VIEW. The issuer, signing and revocation store are
/// intentionally absent: this module only consumes facts supplied by an
/// already trusted, separately accepted enrollment boundary.
#[derive(Debug, Clone)]
pub struct ObservationCredential<'a> {
    pub subject: &'a str,
    pub scopes: &'a [&'a str],
    pub expires_at_unix: u64,
    pub revoked: bool,
    pub generation: u64,
}

/// Fail-closed authorization for observation scopes only. An empty subject,
/// zero generation, revocation, expiry or a non-observation scope refuses.
pub fn authorize(
    credential: &ObservationCredential<'_>,
    now_unix: u64,
    scope: &str,
) -> Result<(), Refusal> {
    if credential.revoked {
        return Err(Refusal::RevokedCredential);
    }
    if now_unix >= credential.expires_at_unix {
        return Err(Refusal::ExpiredCredential);
    }
    if credential.subject.is_empty() || credential.generation == 0 {
        return Err(Refusal::UnauthorizedScope);
    }
    if FORBIDDEN_SCOPES.contains(&scope) || !is_observation_scope(scope) {
        return Err(Refusal::UnauthorizedScope);
    }
    if !credential.scopes.contains(&scope) {
        return Err(Refusal::UnauthorizedScope);
    }
    Ok(())
}

pub fn is_observation_scope(scope: &str) -> bool {
    scope == OBSERVE_EVENTS || scope == OBSERVE_CAPABILITIES
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageRequest {
    pub after: Option<u64>,
    pub limit: u64,
}

/// Strict parsing: a malformed or empty-success cursor is refused, never
/// defaulted into an apparently successful empty page.
pub fn parse_page(after: Option<&str>, limit: Option<&str>) -> Result<PageRequest, Refusal> {
    let after = match after {
        None | Some("") => None,
        Some(value) => Some(value.parse::<u64>().map_err(|_| Refusal::MalformedCursor)?),
    };
    let limit = match limit {
        None => MAX_PAGE_LIMIT,
        Some(value) => value.parse::<u64>().map_err(|_| Refusal::MalformedLimit)?,
    };
    if !(1..=MAX_PAGE_LIMIT).contains(&limit) {
        return Err(Refusal::MalformedLimit);
    }
    Ok(PageRequest { after, limit })
}

/// Reconnection with the caller's last durable cursor: contiguous sequence is
/// accepted; a gap is an integrity refusal, never a silent skip.
pub fn check_sequence(previous: Option<u64>, next: u64) -> Result<(), Refusal> {
    match previous {
        None => Ok(()),
        Some(value) if next == value.saturating_add(1) => Ok(()),
        Some(_) => Err(Refusal::SequenceGap),
    }
}

/// A pinned client that names a different source revision gets a mismatch,
/// not a partial event stream. An unpinned client is allowed but stays
/// unverified by this library-only contract.
pub fn check_source_revision(
    client_pin: Option<&str>,
    server_revision: &str,
) -> Result<(), Refusal> {
    match client_pin {
        None => Ok(()),
        Some(pin) if pin == server_revision => Ok(()),
        Some(_) => Err(Refusal::SourceRevisionMismatch),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_credential<'a>(
        scopes: &'a [&'a str],
        revoked: bool,
        expires: u64,
    ) -> ObservationCredential<'a> {
        ObservationCredential {
            subject: "os-client-1",
            scopes,
            expires_at_unix: expires,
            revoked,
            generation: 1,
        }
    }

    #[test]
    fn supported_version_is_accepted_without_downgrade() {
        assert_eq!(
            negotiate_version(Some(CONTRACT_VERSION)),
            VersionDecision::Supported(CONTRACT_VERSION)
        );
    }

    #[test]
    fn missing_malformed_and_unsupported_versions_are_refused() {
        assert_eq!(negotiate_version(None), VersionDecision::Missing);
        assert_eq!(negotiate_version(Some("")), VersionDecision::Malformed);
        assert_eq!(
            negotiate_version(Some("prometheos.os-observation.v0")),
            VersionDecision::Unsupported
        );
        assert_eq!(
            negotiate_version(Some("prometheos.os-observation.v1 extra")),
            VersionDecision::Malformed
        );
    }

    #[test]
    fn observation_scopes_are_accepted() {
        let scopes = [OBSERVE_EVENTS, OBSERVE_CAPABILITIES];
        let credential = make_credential(&scopes, false, 200);
        assert!(authorize(&credential, 100, OBSERVE_EVENTS).is_ok());
        assert!(authorize(&credential, 100, OBSERVE_CAPABILITIES).is_ok());
    }

    #[test]
    fn expired_and_revoked_credentials_are_refused() {
        let scopes = [OBSERVE_EVENTS];
        let expired = make_credential(&scopes, false, 100);
        assert_eq!(
            authorize(&expired, 100, OBSERVE_EVENTS),
            Err(Refusal::ExpiredCredential)
        );
        let revoked = make_credential(&scopes, true, 200);
        assert_eq!(
            authorize(&revoked, 100, OBSERVE_EVENTS),
            Err(Refusal::RevokedCredential)
        );
    }

    #[test]
    fn unenrolled_unscoped_and_wrong_scope_clients_are_refused() {
        let scopes = [OBSERVE_CAPABILITIES];
        let credential = make_credential(&scopes, false, 200);
        assert_eq!(
            authorize(&credential, 100, OBSERVE_EVENTS),
            Err(Refusal::UnauthorizedScope)
        );
        let mut empty = make_credential(&scopes, false, 200);
        empty.subject = "";
        assert_eq!(
            authorize(&empty, 100, OBSERVE_CAPABILITIES),
            Err(Refusal::UnauthorizedScope)
        );
        let mut zero_generation = make_credential(&scopes, false, 200);
        zero_generation.generation = 0;
        assert_eq!(
            authorize(&zero_generation, 100, OBSERVE_CAPABILITIES),
            Err(Refusal::UnauthorizedScope)
        );
    }

    #[test]
    fn no_scope_can_escalate_observation_into_execution() {
        let scopes = [OBSERVE_EVENTS, OBSERVE_CAPABILITIES];
        let credential = make_credential(&scopes, false, 200);
        for scope in FORBIDDEN_SCOPES {
            assert_eq!(
                authorize(&credential, 100, scope),
                Err(Refusal::UnauthorizedScope),
                "observation scope must never satisfy {scope}"
            );
            assert!(!is_observation_scope(scope));
        }
    }

    #[test]
    fn malformed_cursor_and_limit_are_refused_not_defaulted() {
        assert_eq!(parse_page(Some("x"), None), Err(Refusal::MalformedCursor));
        assert_eq!(parse_page(None, Some("x")), Err(Refusal::MalformedLimit));
        assert_eq!(parse_page(None, Some("0")), Err(Refusal::MalformedLimit));
        assert_eq!(
            parse_page(None, Some((MAX_PAGE_LIMIT + 1).to_string().as_str())),
            Err(Refusal::MalformedLimit)
        );
    }

    #[test]
    fn reconnect_cursor_is_accepted_and_contiguous() {
        let page = parse_page(Some("42"), Some("10")).unwrap();
        assert_eq!(page.after, Some(42));
        assert_eq!(page.limit, 10);
        assert!(check_sequence(Some(42), 43).is_ok());
        assert_eq!(check_sequence(Some(42), 44), Err(Refusal::SequenceGap));
    }

    #[test]
    fn source_revision_mismatch_is_refused() {
        let server = "4ce9529a7b0f1a08084e0de1c7a1440e1de90e26";
        assert!(check_source_revision(Some(server), server).is_ok());
        assert!(check_source_revision(None, server).is_ok());
        assert_eq!(
            check_source_revision(Some("770e68f5bb1163fdcf61c84f98b72a394840d390"), server),
            Err(Refusal::SourceRevisionMismatch)
        );
    }

    #[test]
    fn refusal_codes_are_machine_readable_and_stable() {
        assert_eq!(
            refusal_code(Refusal::UnsupportedVersion),
            "unsupported_version"
        );
        assert_eq!(refusal_code(Refusal::SequenceGap), "sequence_gap");
    }

    #[test]
    fn contract_never_claims_clearance_or_runtime_acceptance() {
        let source = include_str!("mod.rs");
        // Compose needles so this assertion cannot match its own literals.
        let forbidden = [
            format!("SECURITY_CLEARANCE{} VERIFIED", ":"),
            format!("stage2Executed{}true", "="),
            format!("runtime_authority{}granted", "_"),
        ];
        for needle in &forbidden {
            assert!(
                !source.contains(needle.as_str()),
                "contract must not claim {needle}"
            );
        }
    }
}
