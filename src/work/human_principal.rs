//! Verified human principals for human gate decisions (#131, F1).
//!
//! Law: a human gate decision names a VERIFIED human principal. It is
//! never a self-asserted label.
//!
//! - The identity comes from the OS session that owns this process — the
//!   account the operating system authenticated for it. A caller cannot
//!   choose it at the command line, and no environment variable or CLI
//!   flag can substitute for it.
//! - The identity must be listed in a durable TRUST ANCHOR for this
//!   repository (`.prometheos/human-principals.json`), provisioned by the
//!   operator. The anchor says which humans of this machine are allowed
//!   to decide on runs in this repository.
//! - When EITHER cannot be established, the caller MUST REFUSE. The
//!   runtime never promotes a self-asserted identity into a human one:
//!   an unverified session and a missing anchor are typed refusals, not
//!   silent fallbacks to a default name.
//!
//! # What this does and does not prove
//!
//! PROVEN: which OS ACCOUNT acted, that the account is one the operator
//! allowlisted for this repository, and that the decision was made by an
//! intentional `work decide` invocation in that account.
//!
//! NOT PROVEN: that a human PERSONALLY authorized the decision. An
//! automated process running under an allowlisted account inherits that
//! account's identity. OS account lookup is not human-presence
//! verification, and nothing here should be read as interactive
//! authentication. Stronger authentication (a real credential check, a
//! second factor, human-presence verification) is a separate capability,
//! not something this slice claims.
//!
//! The trust anchor is itself protected under this local trust model:
//! the decision path only ever READS it (it never provisions, rewrites,
//! or repairs it), a malformed anchor is refused, and on Unix an anchor
//! that other accounts may write is refused outright. On Windows the
//! anchor inherits the ACLs of the repository working tree it lives in;
//! the runtime does not walk Windows ACLs, so that inheritance is the
//! local trust model's protection on that platform.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Schema version of the human-principal trust anchor.
pub const PRINCIPAL_ANCHOR_SCHEMA_VERSION: &str = "1.0.0";

/// The trust anchor's location relative to a repository root.
pub const PRINCIPAL_ANCHOR_RELATIVE_PATH: &str = ".prometheos/human-principals.json";

/// The durable trust anchor: the humans authorized to record human gate
/// decisions for this repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HumanPrincipalAnchor {
    pub schema_version: String,
    /// Account identities (OS session identities) allowed to decide.
    pub principals: Vec<String>,
}

impl HumanPrincipalAnchor {
    /// Whether `identity` is one of the trusted principals.
    pub fn trusts(&self, identity: &str) -> bool {
        self.principals.iter().any(|p| p == identity)
    }
}

/// A human principal the runtime has VERIFIED for this session.
///
/// Constructing one is the only way to obtain it: the constructor is
/// private to this module's verification path, so no caller can mint a
/// verified principal from a string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedHumanPrincipal {
    identity: String,
}

impl VerifiedHumanPrincipal {
    /// The verified account identity.
    pub fn identity(&self) -> &str {
        &self.identity
    }
}

/// The OS-authenticated identity of the account running this process.
///
/// This is the account the operating system authenticated for the
/// process, not a value the caller supplies. It fails closed when the
/// platform cannot report it.
///
/// # Which identity is this?
///
/// The source is platform-specific and is stated explicitly by
/// [`IDENTITY_SOURCE`] rather than assumed:
///
/// - Windows: the CALLING THREAD's authenticated token. `GetUserNameW`
///   resolves `OpenThreadToken`, which falls back to the process token
///   only when the thread is NOT impersonating. The thread's token is
///   therefore the governing principal BY DESIGN: if a thread IS
///   impersonating another account, the IMPERSONATED identity is what is
///   reported and recorded — the process identity is never silently
///   substituted for it. This slice does not DETECT impersonation (that
///   would require reading the thread token directly); it names the
///   semantics instead, and the trust anchor bounds the impact: only an
///   identity the operator allowlisted can be reported at all.
/// - Unix: the process's EFFECTIVE uid (the identity whose permissions
///   actually apply), refused when it differs from the real uid.
pub fn session_identity() -> Result<String> {
    platform::session_identity()
}

/// WHICH identity the runtime treats as the governing principal, stated
/// explicitly so the thread token and the process token are never
/// silently conflated:
///
/// - `"thread-token"`: the calling thread's authenticated token (Windows).
///   Under impersonation the IMPERSONATED identity is what is reported —
///   the process identity is never substituted for it.
/// - `"effective-uid"`: the process's effective uid (Unix).
pub const IDENTITY_SOURCE: &str = platform::IDENTITY_SOURCE;

/// The trust anchor's path for a repository root.
pub fn default_anchor_path(repo_root: &Path) -> PathBuf {
    repo_root.join(PRINCIPAL_ANCHOR_RELATIVE_PATH)
}

/// Load and validate the trust anchor. READ-ONLY: this never creates,
/// repairs, or rewrites the anchor.
pub fn load_anchor(path: &Path) -> Result<HumanPrincipalAnchor> {
    ensure_anchor_is_protected(path)?;
    let text = std::fs::read_to_string(path).with_context(|| {
        format!(
            "no human-principal trust anchor at {} — a human gate decision requires a \
             verified human principal, and this repository has no anchor listing one",
            path.display()
        )
    })?;
    let anchor: HumanPrincipalAnchor = serde_json::from_str(&text).with_context(|| {
        format!(
            "human-principal trust anchor {} is not valid",
            path.display()
        )
    })?;
    if anchor.schema_version != PRINCIPAL_ANCHOR_SCHEMA_VERSION {
        anyhow::bail!(
            "unsupported human-principal trust anchor schema version {} (expected {})",
            anchor.schema_version,
            PRINCIPAL_ANCHOR_SCHEMA_VERSION
        );
    }
    if anchor.principals.is_empty() {
        anyhow::bail!(
            "human-principal trust anchor {} lists no principals",
            path.display()
        );
    }
    Ok(anchor)
}

/// VERIFY the human principal for this session.
///
/// The session identity must be established AND listed in the trust
/// anchor. Anything else is a refusal — never a fallback identity.
pub fn verify_human_principal(anchor_path: &Path) -> Result<VerifiedHumanPrincipal> {
    let anchor = load_anchor(anchor_path)?;
    let identity = session_identity().context(
        "the operating system did not report the identity of this session; a human gate \
         decision cannot be attributed to a verified human principal",
    )?;
    if !anchor.trusts(&identity) {
        anyhow::bail!(
            "session identity {identity:?} is not a trusted human principal for this \
             repository (anchor {} lists: {})",
            anchor_path.display(),
            anchor.principals.join(", ")
        );
    }
    Ok(VerifiedHumanPrincipal { identity })
}

// ---------------------------------------------------------------------------
// Trust-anchor protection
// ---------------------------------------------------------------------------

/// Refuse a trust anchor other accounts may write.
///
/// A trust anchor another account can modify is not a trust anchor: it
/// would let that account add itself to the allowlist. On Unix the file
/// mode answers this directly. On Windows the anchor inherits the ACLs
/// of the working tree it lives in — the runtime does not walk Windows
/// ACLs, so that inheritance is the local trust model's protection.
#[cfg(unix)]
fn ensure_anchor_is_protected(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let Ok(meta) = std::fs::metadata(path) else {
        // No anchor yet: the refusal is raised by the read below.
        return Ok(());
    };
    let mode = meta.permissions().mode();
    if mode & 0o022 != 0 {
        anyhow::bail!(
            "human-principal trust anchor {} is writable by other accounts (mode {:o}); \
             a trust anchor others can modify is not a trust anchor",
            path.display(),
            mode & 0o777
        );
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_anchor_is_protected(_path: &Path) -> Result<()> {
    Ok(())
}

// ---------------------------------------------------------------------------
// Platform session identity
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod platform {
    use anyhow::Result;

    /// The governing identity source on Windows.
    ///
    /// `GetUserNameW` resolves the CALLING THREAD's security context
    /// (`OpenThreadToken`), falling back to the process token only when
    /// the thread is not impersonating. The thread's authenticated token
    /// is therefore the governing principal, stated here so the thread
    /// token and the process token are never silently conflated.
    pub const IDENTITY_SOURCE: &str = "thread-token";

    /// The account the operating system authenticated for this thread.
    ///
    /// `GetUserNameW` reads the thread's security context, NOT an
    /// environment variable: poisoning `USERNAME` or `USERDOMAIN` does
    /// not change the answer. If the thread is impersonating another
    /// account, THAT identity is reported — the process identity is never
    /// substituted for it. The Windows local trust model relies on the
    /// anchor's inherited ACLs rather than on a mode check, because
    /// Windows has no POSIX mode bits.
    pub fn session_identity() -> Result<String> {
        use winapi::shared::minwindef::DWORD;
        use winapi::um::winbase::GetUserNameW;
        use winapi::um::winnt::WCHAR;

        // UNITS_TO_ALLOCATE is the documented starting buffer size for
        // GetUserNameW; the API reports the required size when it fails.
        const UNITS_TO_ALLOCATE: DWORD = 256;

        let mut size: DWORD = UNITS_TO_ALLOCATE;
        let mut buffer: Vec<WCHAR> = vec![0; UNITS_TO_ALLOCATE as usize];
        let ok = unsafe { GetUserNameW(buffer.as_mut_ptr(), &mut size) };
        if ok == 0 {
            anyhow::bail!(
                "GetUserNameW failed: the operating system did not report this session's identity"
            );
        }
        // The returned size INCLUDES the terminating null.
        let len = (size as usize).saturating_sub(1).min(buffer.len());
        let identity = String::from_utf16_lossy(&buffer[..len]);
        if identity.trim().is_empty() {
            anyhow::bail!("the operating system reported an empty session identity");
        }
        Ok(identity)
    }
}

#[cfg(unix)]
mod platform {
    use anyhow::Result;

    /// The governing identity source on Unix.
    ///
    /// The EFFECTIVE uid — the identity whose permissions actually
    /// apply — not the real uid, and not an environment variable.
    pub const IDENTITY_SOURCE: &str = "effective-uid";

    /// The account the operating system authenticated for this process:
    /// the EFFECTIVE uid's passwd entry.
    ///
    /// The effective uid is the identity whose permissions actually
    /// apply, so it is the one the trust anchor must be checked against.
    /// When it differs from the real uid the process runs with
    /// privileges the allowlisted human does not hold (a privilege
    /// escalation); that is REFUSED rather than reported as the human.
    ///
    /// Platform facts (#264): the process's uids are read from the POSIX
    /// source via libc (`geteuid`/`getuid`) — sysinfo 0.30 carries no
    /// user lookup on `System` at all. The passwd database is exposed
    /// only through a POPULATED `Users` collection (`new()` builds an
    /// empty one), so an unrefreshed list or a missing passwd entry is a
    /// typed refusal, never a fallback.
    pub fn session_identity() -> Result<String> {
        // SAFETY: POSIX uid getters are thread-safe syscall wrappers with no
        // preconditions; they cannot be substituted by caller-supplied data.
        let effective_raw = unsafe { libc::geteuid() };
        let real_raw = unsafe { libc::getuid() };
        if real_raw != effective_raw {
            anyhow::bail!(
                "this process runs with an effective uid that differs from its real uid \
                 (privilege escalation); refusing to attribute a human decision to it"
            );
        }
        // sysinfo 0.30: `Users::new()` is an empty collection and
        // `get_user_by_id` is a scan of it, so the list must be POPULATED
        // before it can be searched (#264).
        let users = sysinfo::Users::new_with_refreshed_list();
        let effective = sysinfo::Uid::try_from(effective_raw as usize).map_err(|_| {
            anyhow::anyhow!("the operating system did not report this session's effective uid")
        })?;
        let user = users.get_user_by_id(&effective).ok_or_else(|| {
            anyhow::anyhow!(
                "the operating system did not report a passwd entry for this session's effective uid"
            )
        })?;
        let identity = user.name().to_string();
        if identity.trim().is_empty() {
            anyhow::bail!("the operating system reported an empty session identity");
        }
        Ok(identity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_anchor(dir: &Path, principals: &[&str]) -> PathBuf {
        let path = default_anchor_path(dir);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("anchor dir");
        let anchor = HumanPrincipalAnchor {
            schema_version: PRINCIPAL_ANCHOR_SCHEMA_VERSION.into(),
            principals: principals.iter().map(|p| p.to_string()).collect(),
        };
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&anchor).expect("anchor json"),
        )
        .expect("write anchor");
        path
    }

    #[test]
    fn a_missing_anchor_is_a_typed_refusal_not_a_fallback() {
        let dir = tempfile::tempdir().expect("temp dir");
        let err = verify_human_principal(&default_anchor_path(dir.path()))
            .expect_err("a missing anchor must refuse");
        let msg = format!("{err:#}");
        assert!(msg.contains("trust anchor"), "{msg}");
    }

    #[test]
    fn an_anchor_listing_another_identity_refuses() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = write_anchor(dir.path(), &["someone-else"]);
        let err = verify_human_principal(&path).expect_err("an untrusted session must refuse");
        let msg = format!("{err:#}");
        assert!(msg.contains("not a trusted human principal"), "{msg}");
    }

    #[test]
    fn an_anchor_listing_this_session_verifies_the_principal() {
        let dir = tempfile::tempdir().expect("temp dir");
        let identity = session_identity().expect("session identity");
        let path = write_anchor(dir.path(), &[identity.as_str()]);
        let principal = verify_human_principal(&path).expect("verified");
        assert_eq!(principal.identity(), identity);
    }

    #[test]
    fn an_empty_or_unsupported_anchor_refuses() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = write_anchor(dir.path(), &[]);
        let err = load_anchor(&path).expect_err("an empty anchor must refuse");
        assert!(err.to_string().contains("lists no principals"), "{err}");

        let path = default_anchor_path(dir.path());
        std::fs::write(&path, r#"{"schemaVersion":"9.9.9","principals":["x"]}"#).expect("write");
        let err = load_anchor(&path).expect_err("an unsupported version must refuse");
        assert!(err.to_string().contains("unsupported"), "{err}");

        std::fs::write(&path, "{not json").expect("write");
        let err = load_anchor(&path).expect_err("a malformed anchor must refuse");
        assert!(err.to_string().contains("not valid"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn an_anchor_writable_by_other_accounts_is_refused() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("temp dir");
        let path = write_anchor(dir.path(), &["someone"]);
        let mut perms = std::fs::metadata(&path).expect("meta").permissions();
        perms.set_mode(0o666);
        std::fs::set_permissions(&path, perms).expect("chmod");
        let err = load_anchor(&path).expect_err("an others-writable anchor must refuse");
        assert!(
            err.to_string().contains("writable by other accounts"),
            "{err}"
        );
    }

    #[test]
    fn the_session_identity_is_not_caller_supplied() {
        // The identity comes from the OS, so it is never empty and never
        // a value this process chose.
        let identity = session_identity().expect("session identity");
        assert!(!identity.trim().is_empty());
        assert_ne!(identity, "cli-user");
    }

    #[test]
    fn the_governing_identity_source_is_declared_and_resolved_per_thread() {
        // The identity source is DECLARED, not assumed: on Windows it is
        // the calling thread's authenticated token (`GetUserNameW` ->
        // OpenThreadToken), on Unix the process's effective uid. The
        // thread token and the process token must never be silently
        // conflated, so the declared source is asserted here and the
        // resolution is exercised from a worker thread as well as the
        // main one.
        #[cfg(windows)]
        assert_eq!(IDENTITY_SOURCE, "thread-token");
        #[cfg(unix)]
        assert_eq!(IDENTITY_SOURCE, "effective-uid");

        let main_identity = session_identity().expect("main thread identity");
        let worker = std::thread::spawn(|| session_identity().expect("worker thread identity"));
        let worker_identity = worker.join().expect("join worker");
        // With no impersonation active the thread token falls back to the
        // process token, so both threads report the same account.
        assert_eq!(
            main_identity, worker_identity,
            "the thread-scoped resolution must report the same account when no impersonation is active"
        );
    }

    /// Probe: only meaningful in the child process spawned by
    /// `session_identity_comes_from_the_process_not_the_environment`.
    #[test]
    fn identity_probe() {
        if std::env::var("PROMETHEOS_IDENTITY_PROBE").is_err() {
            return;
        }
        let identity = session_identity().expect("session identity");
        println!("PROMETHEOS_PROBE_IDENTITY={identity}");
    }

    #[test]
    fn session_identity_comes_from_the_process_not_the_environment() {
        // The identity must come from the process security context, not
        // from the environment. Prove it: run a probe in a child process
        // with every identity-bearing environment variable poisoned, and
        // assert the reported identity is unchanged.
        let exe = std::env::current_exe().expect("current exe");
        let output = std::process::Command::new(exe)
            .args([
                "--exact",
                "work::human_principal::tests::identity_probe",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("PROMETHEOS_IDENTITY_PROBE", "1")
            .env("USERNAME", "spoofed-user")
            .env("USER", "spoofed-user")
            .env("LOGNAME", "spoofed-user")
            .env("USERDOMAIN", "spoofed-domain")
            .output()
            .expect("run probe");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "probe failed: {stdout}{}",
            String::from_utf8_lossy(&output.stderr)
        );
        // The test harness prints its "test <name> ... " prefix without a
        // trailing newline, so the marker can share a line with it: split
        // on the marker rather than matching a line prefix.
        let probed = stdout
            .split("PROMETHEOS_PROBE_IDENTITY=")
            .nth(1)
            .map(|rest| rest.lines().next().unwrap_or("").trim().to_string())
            .filter(|value| !value.is_empty())
            .expect("probe identity");
        let mine = session_identity().expect("session identity");
        assert_eq!(
            probed, mine,
            "the session identity must not come from the environment"
        );
        assert_ne!(probed, "spoofed-user");
    }
}
