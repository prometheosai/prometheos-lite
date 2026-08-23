//! Durable artifact integrity (checksum) primitives.
//!
//! This module defines the shared digest contract used to detect corruption or
//! tampering of durable workflow artifacts (proposal, validation, integrity,
//! evidence bundle, and persisted raw logs).
//!
//! # Threat model
//!
//! SHA-256 checksums provide corruption/tampering detection when an artifact
//! changes *without* its trusted integrity metadata (the sidecar published
//! alongside it). They are **not** a signature: a privileged attacker who can
//! rewrite both the artifact and its checksum sidecar defeats this check. We do
//! not claim cryptographic authenticity. Where a real trust boundary appears,
//! signed checksums can be layered on this same contract later.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

use super::durable;

/// Digest algorithm supported by this runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DigestAlgorithm {
    /// SHA-256 (only algorithm supported today).
    Sha256,
}

/// The durable document kind a digest describes. Used only for diagnostics and
/// the protected-artifact accounting; it does not change verification logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    /// `proposal.json`.
    Proposal,
    /// `validation.json`.
    Validation,
    /// `integrity.json`.
    Integrity,
    /// `evidence.json` (terminal evidence bundle).
    Evidence,
    /// Raw validation stdout/stderr or other persisted log.
    RawLog,
    /// Any other durable artifact protected by this contract.
    Other,
}

impl ArtifactKind {
    /// Stable lowercase string used in diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            ArtifactKind::Proposal => "proposal",
            ArtifactKind::Validation => "validation",
            ArtifactKind::Integrity => "integrity",
            ArtifactKind::Evidence => "evidence",
            ArtifactKind::RawLog => "raw_log",
            ArtifactKind::Other => "other",
        }
    }
}

/// A typed integrity record describing one durable artifact.
///
/// The `path` is repo-relative when the artifact lives inside the repository so
/// that hostile on-disk metadata cannot direct verification outside the repo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactDigest {
    /// Digest algorithm (currently always `sha256`).
    pub algorithm: DigestAlgorithm,
    /// Lowercase hex SHA-256 digest of the exact persisted bytes.
    pub sha256: String,
    /// Size in bytes of the exact persisted bytes.
    pub size_bytes: u64,
    /// Which durable document kind this digest protects.
    pub artifact_kind: ArtifactKind,
    /// Repo-relative path of the protected artifact (must be safe).
    pub path: String,
}

impl ArtifactDigest {
    /// Construct a SHA-256 digest over `bytes`, recording the repo-relative
    /// `path` (relative to `repo`) for `kind`.
    pub fn new_sha256(
        repo: &Path,
        absolute_path: &Path,
        bytes: &[u8],
        kind: ArtifactKind,
    ) -> Result<Self> {
        let sha256 = sha256_hex(bytes);
        Ok(ArtifactDigest {
            algorithm: DigestAlgorithm::Sha256,
            sha256,
            size_bytes: bytes.len() as u64,
            artifact_kind: kind,
            path: durable::repo_relative_path(repo, absolute_path),
        })
    }

    /// Fail closed if the stored hex digest is malformed.
    pub fn validate_hex(&self) -> Result<()> {
        validate_digest_hex(&self.sha256)
    }

    /// Fail closed if the stored repo-relative path is unsafe.
    pub fn validate_path(&self) -> Result<()> {
        validate_artifact_path(&self.path)
    }

    /// Verify an in-memory copy of the artifact against this digest.
    ///
    /// Recomputes the SHA-256 and size, and ensures the recorded path is safe.
    /// Returns an error (fail closed) on any mismatch.
    pub fn verify_against(&self, bytes: &[u8]) -> Result<()> {
        self.validate_hex()
            .context("integrity metadata carries a malformed digest")?;
        self.validate_path()
            .context("integrity metadata carries an unsafe path")?;
        let actual = sha256_hex(bytes);
        if actual != self.sha256 {
            bail!(
                "artifact integrity failure: sha256 mismatch for {} (expected {}, found {})",
                self.path,
                self.sha256,
                actual
            );
        }
        if bytes.len() as u64 != self.size_bytes {
            bail!(
                "artifact integrity failure: size mismatch for {} (expected {}, found {})",
                self.path,
                self.size_bytes,
                bytes.len()
            );
        }
        Ok(())
    }
}

/// Compute the lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Fail closed if `hex` is not a 64-character lowercase hex SHA-256 digest.
pub fn validate_digest_hex(hex: &str) -> Result<()> {
    if hex.len() != 64 {
        bail!("malformed sha256 digest: wrong length ({})", hex.len());
    }
    if !hex
        .bytes()
        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        bail!("malformed sha256 digest: must be lowercase hex");
    }
    Ok(())
}

/// Fail closed if a repo-relative artifact path is absolute or traverses
/// outside the repository (`..`). Absolute detection is platform-aware so a
/// Linux CI runner still rejects Windows-shaped hostile paths.
pub fn validate_artifact_path(path: &str) -> Result<()> {
    if path.is_empty() {
        bail!("empty artifact path is not allowed");
    }
    let p = Path::new(path);
    let looks_absolute = p.is_absolute()
        || path.starts_with('/')
        || path.starts_with("\\\\")
        || path.starts_with("//");
    if looks_absolute {
        bail!("artifact path is absolute (must be repo-relative): {path}");
    }
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        bail!("artifact path is a Windows drive path (must be repo-relative): {path}");
    }
    if path.split('/').any(|c| c == "..") {
        bail!("artifact path escapes the repository: {path}");
    }
    Ok(())
}

/// Return the checksum sidecar path paired with `artifact_path`.
///
/// The sidecar lives next to the artifact as `<name>.integrity.json` so it is
/// removed by retention only when its artifact is also removed (see
/// [`crate::workflow::retention`]).
pub fn sidecar_for(artifact_path: &Path) -> PathBuf {
    let file_name = artifact_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "artifact".to_string());
    match artifact_path.parent() {
        Some(parent) => parent.join(format!("{file_name}.integrity.json")),
        None => PathBuf::from(format!("{file_name}.integrity.json")),
    }
}

/// Durably publish `bytes` to `absolute_path` and then publish its integrity
/// sidecar in the correct order: artifact bytes first, then the checksum.
///
/// `kind` classifies the artifact for diagnostics. The repo-relative path is
/// recorded from `repo` so the digest cannot escape the repository.
///
/// Returns the computed [`ArtifactDigest`].
pub fn publish_with_integrity(
    repo: &Path,
    absolute_path: &Path,
    bytes: &[u8],
    kind: ArtifactKind,
) -> Result<ArtifactDigest> {
    durable::atomic_write_bytes(absolute_path, bytes)
        .with_context(|| format!("failed to publish artifact {}", absolute_path.display()))?;
    let digest = ArtifactDigest::new_sha256(repo, absolute_path, bytes, kind)?;
    let sidecar = sidecar_for(absolute_path);
    durable::atomic_write_json(&sidecar, &digest)
        .with_context(|| format!("failed to publish integrity sidecar {}", sidecar.display()))?;
    Ok(digest)
}

/// Read `absolute_path` bytes, load the paired integrity sidecar, and verify
/// the bytes before returning them. Fail closed on any mismatch, missing
/// sidecar (for #115-format artifacts), corruption, or unsafe metadata.
///
/// `expected_kind` is used to surface a clear error when the persisted sidecar
/// describes a different kind.
pub fn read_verified(
    repo: &Path,
    absolute_path: &Path,
    expected_kind: ArtifactKind,
) -> Result<Vec<u8>> {
    let bytes = std::fs::read(absolute_path)
        .with_context(|| format!("failed to read artifact {}", absolute_path.display()))?;
    let sidecar = sidecar_for(absolute_path);
    verify_with_sidecar(repo, absolute_path, expected_kind, &bytes, &sidecar)
}

/// Read `absolute_path` and verify its integrity sidecar, but tolerate a
/// *missing* sidecar as a pre-#115 legacy artifact (loaded unverified).
///
/// Newly written #115-format artifacts always carry a sidecar, so this only
/// admits genuinely legacy files; a present-but-corrupt sidecar still fails
/// closed.
pub fn read_verified_or_legacy(
    repo: &Path,
    absolute_path: &Path,
    expected_kind: ArtifactKind,
) -> Result<Vec<u8>> {
    let sidecar = sidecar_for(absolute_path);
    if sidecar.exists() {
        read_verified(repo, absolute_path, expected_kind)
    } else {
        std::fs::read(absolute_path)
            .with_context(|| format!("failed to read legacy artifact {}", absolute_path.display()))
    }
}

/// Verify already-read `bytes` against the sidecar at `sidecar_path`.
pub fn verify_with_sidecar(
    repo: &Path,
    absolute_path: &Path,
    expected_kind: ArtifactKind,
    bytes: &[u8],
    sidecar_path: &Path,
) -> Result<Vec<u8>> {
    let text = std::fs::read_to_string(sidecar_path).with_context(|| {
        format!(
            "missing or unreadable integrity sidecar for {} (required for #115-format artifacts)",
            absolute_path.display()
        )
    })?;
    let digest: ArtifactDigest = serde_json::from_str(&text)
        .with_context(|| format!("corrupt integrity sidecar {}", sidecar_path.display()))?;
    if digest.artifact_kind != expected_kind {
        bail!(
            "integrity sidecar kind mismatch for {}: expected {}, found {}",
            absolute_path.display(),
            expected_kind.as_str(),
            digest.artifact_kind.as_str()
        );
    }
    // Bind the integrity metadata to the artifact path: the sidecar must describe
    // exactly this artifact and must never verify a relocated or swapped file.
    let expected_path = durable::repo_relative_path(repo, absolute_path);
    if digest.path != expected_path {
        bail!(
            "integrity sidecar path mismatch for {}: sidecar describes '{}' but artifact is '{}'",
            absolute_path.display(),
            digest.path,
            expected_path
        );
    }
    digest.verify_against(bytes).with_context(|| {
        format!(
            "integrity verification failed for {}",
            absolute_path.display()
        )
    })?;
    Ok(bytes.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn digest_hex_validation() {
        let good = sha256_hex(b"hello");
        assert!(validate_digest_hex(&good).is_ok());
        assert!(validate_digest_hex("ABCDEF").is_err());
        assert!(validate_digest_hex(&"a".repeat(63)).is_err());
        // A 64-character lowercase hex string is well-formed (valid digest).
        assert!(validate_digest_hex(&"a".repeat(64)).is_ok());
        assert!(validate_digest_hex(&format!("{good}X")).is_err());
    }

    #[test]
    fn artifact_path_validation() {
        assert!(validate_artifact_path("workflow/abc/proposal.json").is_ok());
        assert!(validate_artifact_path("/etc/passwd").is_err());
        assert!(validate_artifact_path("a/../../etc").is_err());
        assert!(validate_artifact_path("//server/share").is_err());
        assert!(validate_artifact_path("../escape").is_err());
        assert!(validate_artifact_path("").is_err());
    }

    #[test]
    fn publish_then_read_verified_round_trips() {
        let dir = tmp();
        let repo = dir.path();
        let path = repo.join("sub").join("artifact.bin");
        let bytes = b"trusted content".to_vec();
        let digest = publish_with_integrity(repo, &path, &bytes, ArtifactKind::Other).unwrap();
        assert_eq!(digest.size_bytes, bytes.len() as u64);

        let read = read_verified(repo, &path, ArtifactKind::Other).unwrap();
        assert_eq!(read, bytes);

        // Corrupt the artifact; verification must fail closed.
        std::fs::write(&path, b"tampered").unwrap();
        assert!(read_verified(repo, &path, ArtifactKind::Other).is_err());
    }

    #[test]
    fn missing_sidecar_fails_closed() {
        let dir = tmp();
        let repo = dir.path();
        let path = repo.join("no_sidecar.bin");
        std::fs::write(&path, b"data").unwrap();
        assert!(read_verified(repo, &path, ArtifactKind::Other).is_err());
    }

    #[test]
    fn sidecar_kind_mismatch_fails_closed() {
        let dir = tmp();
        let repo = dir.path();
        let path = repo.join("kind.bin");
        let bytes = b"data".to_vec();
        publish_with_integrity(repo, &path, &bytes, ArtifactKind::Proposal).unwrap();
        // Expecting a different kind must fail.
        assert!(read_verified(repo, &path, ArtifactKind::Validation).is_err());
    }

    #[test]
    fn corrupt_sidecar_fails_closed() {
        let dir = tmp();
        let repo = dir.path();
        let path = repo.join("c.bin");
        let bytes = b"data".to_vec();
        publish_with_integrity(repo, &path, &bytes, ArtifactKind::Other).unwrap();
        let sidecar = sidecar_for(&path);
        std::fs::write(&sidecar, "not json").unwrap();
        assert!(read_verified(repo, &path, ArtifactKind::Other).is_err());
    }

    #[test]
    fn integrity_metadata_is_bound_to_artifact_path() {
        let dir = tmp();
        let repo = dir.path();
        let path = repo.join("a.json");
        let bytes = b"data".to_vec();
        publish_with_integrity(repo, &path, &bytes, ArtifactKind::Other).unwrap();
        let sidecar = sidecar_for(&path);
        // Verification with the correct artifact path succeeds.
        assert!(verify_with_sidecar(repo, &path, ArtifactKind::Other, &bytes, &sidecar).is_ok());
        // The same bytes verified against a *different* artifact path must fail:
        // the sidecar is bound to the original path and cannot validate a
        // relocated or swapped file.
        let other = repo.join("b.json");
        assert!(verify_with_sidecar(repo, &other, ArtifactKind::Other, &bytes, &sidecar).is_err());
    }
}
