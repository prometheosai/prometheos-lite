//! Governed native workspace-inspection adapter (issue #154, slice F).
//!
//! [`NativeHarness`] is the second real execution adapter conforming to
//! [`ExecutionHarness`]: an in-process, read-only workspace file inspector
//! that reports a deterministic, structured fact about one governed file —
//! its verified byte size and SHA-256 content digest. It is NOT a
//! declaration-only registry entry and NOT the synthetic conformance probe
//! (`harness.native.probe`).
//!
//! Effect discipline (no bypass around #117/#118):
//!
//! - The adapter never reads a file itself. Its `run` invokes the declared
//!   `harness.native.inspect` capability through [`HarnessDispatch::call`],
//!   the only sanctioned effect path. The capability handler
//!   ([`native_capability`]) is registered by the caller in the
//!   [`crate::workflow::node_runner::CapabilityRegistry`] alongside the
//!   reviewed governance permit that authorizes it.
//! - Strict argument validation: exactly one `path` argument, a string,
//!   relative, non-empty, and resource-bounded.
//! - Path confinement: the requested path must be relative (an absolute or
//!   Windows drive/UNC form fails), must contain no `..` segment on any
//!   platform, and the canonicalized file must start with the canonicalized
//!   workspace root. Symlink escapes are refused; directories, missing
//!   files, and files above the size budget are refused before any read.
//! - The handler only reads bytes. It never writes to the workspace, never
//!   follows content outside the governance boundary, and returns a digest —
//!   file CONTENT never leaves the handler, so a secret-bearing file cannot
//!   leak its contents into evidence.
//! - File-size and argument/output limits are enforced by the handler on
//!   every invocation; there is no unbounded mode. Exceeding a budget is a
//!   typed refusal and never reports success.
//!
//! Portable continuity: the adapter owns no durable work state. Its
//! `checkpoint`/`resume` bind the harness identity + revision to a digest
//! of the canonical [`PortableWorkState`](crate::workflow::portable_state::PortableWorkState),
//! which lives outside the adapter. Resume re-verifies the checkpoint and
//! refuses incompatible harness/revision/digest bindings. Any later step
//! can simply re-run against the same portable state, so no hidden
//! session/process state is required for continuation. Evidence correlation:
//! every outcome carries the journaled evidence digest.

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

use crate::workflow::artifact_integrity::sha256_hex;
use crate::workflow::execution_harness::{
    ExecutionHarness, HARNESS_SCHEMA_VERSION, HarnessCheckpointV1, HarnessContinuation,
    HarnessDeclarationV1, HarnessDispatch, HarnessEvent, HarnessFamily, HarnessOutcome,
    HarnessStep,
};
use crate::workflow::node_runner::Capability;
use crate::workflow::redaction::Redactor;
use crate::workflow::soma::capability::{
    AdapterFamily, Identity, Locality, RuntimeCapabilitySet, SUPPORTED_CAPABILITY_SCHEMA_VERSION,
};
use crate::workflow::soma::profile::{Boundaries, Boundary};
use crate::workflow::workspace::ADAPTER_REVISION;

/// Implementation-owned harness id.
pub const NATIVE_HARNESS_ID: &str = "lite.harness.native.inspect";
/// The single declared capability (registry key + declared capability).
pub const NATIVE_INSPECT_CAPABILITY: &str = "harness.native.inspect";
/// Identity name/revision of the implementation.
pub const NATIVE_IDENTITY_NAME: &str = "lite-native-inspect";
pub const NATIVE_REVISION: &str = "v1";

/// Bound for one inspection read window; hashing is incremental so memory
/// never grows with the file.
const READ_CHUNK: usize = 8192;

/// Configuration for the governed native-inspection handler.
///
/// All bounds are enforced by the handler on every invocation; there is no
/// unbounded mode.
#[derive(Clone)]
pub struct NativeInspectConfig {
    /// Files larger than this are refused before any read.
    pub max_file_bytes: u64,
    /// Bytes of serialized arguments accepted per invocation.
    pub max_args_bytes: usize,
    /// Bytes of serialized result documents accepted per invocation.
    pub max_output_bytes: usize,
    /// Bound for the inspected `path` argument value.
    pub max_path_chars: usize,
    /// Known secret values, redacted verbatim from the result document.
    pub known_secrets: Vec<String>,
}

impl Default for NativeInspectConfig {
    fn default() -> Self {
        Self {
            max_file_bytes: 8 * 1024 * 1024,
            max_args_bytes: 64 * 1024,
            max_output_bytes: 64 * 1024,
            max_path_chars: 4096,
            known_secrets: Vec::new(),
        }
    }
}

impl NativeInspectConfig {
    pub fn max_file_bytes(mut self, max_file_bytes: u64) -> Self {
        self.max_file_bytes = max_file_bytes;
        self
    }

    pub fn max_args_bytes(mut self, max_args_bytes: usize) -> Self {
        self.max_args_bytes = max_args_bytes;
        self
    }

    pub fn max_output_bytes(mut self, max_output_bytes: usize) -> Self {
        self.max_output_bytes = max_output_bytes;
        self
    }

    pub fn known_secrets(mut self, known_secrets: Vec<String>) -> Self {
        self.known_secrets = known_secrets;
        self
    }
}

/// The implementation-owned, sealed declaration for the native harness.
/// Shared by [`NativeHarness`] and the canonical catalog inventory so the
/// two cannot diverge.
pub fn native_declaration() -> HarnessDeclarationV1 {
    HarnessDeclarationV1 {
        schema_version: HARNESS_SCHEMA_VERSION.to_string(),
        harness_id: NATIVE_HARNESS_ID.to_string(),
        identity: Identity {
            name: NATIVE_IDENTITY_NAME.to_string(),
            revision: NATIVE_REVISION.to_string(),
        },
        family: HarnessFamily::Native,
        declared_capabilities: vec![NATIVE_INSPECT_CAPABILITY.to_string()],
        claimed_capabilities: vec![NATIVE_INSPECT_CAPABILITY.to_string()],
        capabilities: RuntimeCapabilitySet {
            id: "lite.harness.native.inspect.caps".to_string(),
            schema_version: SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string(),
            version: SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string(),
            identity: Identity {
                name: NATIVE_IDENTITY_NAME.to_string(),
                revision: NATIVE_REVISION.to_string(),
            },
            declared_at: "2026-11-08T00:00:00Z".to_string(),
            spec_versions: vec![SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string()],
            adapter_families: vec![
                AdapterFamily {
                    family: "git-worktree".to_string(),
                    revision: ADAPTER_REVISION.to_string(),
                },
                AdapterFamily {
                    family: "existing-read-only".to_string(),
                    revision: ADAPTER_REVISION.to_string(),
                },
            ],
            effect_classes: vec!["filesystem-read".to_string()],
            workspace: Some(Boundaries {
                workspace: Some(Boundary::Read),
                filesystem: None,
                git: None,
                process: None,
                network: None,
                secret: None,
            }),
            checkpoint_import: Some(true),
            checkpoint_export: Some(true),
            resume_support: Some(true),
            // The inspection result is one terminal evidence document; nothing
            // is streamed incrementally, so streaming is not claimed.
            event_streaming: Some(false),
            read_projection: Some(true),
            locality: Some(Locality::Local),
            privacy_classes: Vec::new(),
            max_structured_input: None,
            max_structured_output: None,
            budget_ceilings: None,
            evidence_attachment: Some(true),
            validation: Some(true),
            approval_escalation: Some(false),
        },
        content_digest: None,
    }
    .sealed()
}

/// The governed native-inspection adapter.
///
/// The adapter itself never reads a file: `run` invokes the declared
/// capability through the governed dispatch. The capability handler
/// ([`native_capability`]) is registered by the caller in the
/// [`crate::workflow::node_runner::CapabilityRegistry`] alongside the
/// reviewed governance permit that authorizes it.
pub struct NativeHarness {
    declaration: HarnessDeclarationV1,
}

impl Default for NativeHarness {
    fn default() -> Self {
        Self::new()
    }
}

impl NativeHarness {
    pub fn new() -> Self {
        Self {
            declaration: native_declaration(),
        }
    }
}

#[async_trait::async_trait]
impl ExecutionHarness for NativeHarness {
    fn declaration(&self) -> &HarnessDeclarationV1 {
        &self.declaration
    }

    async fn run(
        &self,
        step: HarnessStep,
        dispatch: &mut HarnessDispatch<'_>,
    ) -> Result<HarnessOutcome> {
        // The ONLY effect path: `call` refuses capabilities outside this
        // harness's declaration before any file can be touched. The governed
        // request's known secrets ride along so the handler redacts them
        // verbatim — the redaction boundary is authoritative, not config
        // folklore.
        let mut args = step.args.clone();
        if let Some(map) = args.as_object_mut() {
            map.insert(
                "knownSecrets".to_string(),
                serde_json::json!(dispatch.known_secrets()),
            );
        }
        let run_outcome = dispatch.call(&step.capability, args, &step.step_id)?;
        let output = run_outcome.output.clone();

        // The inspection result is machine-readable JSON produced by the
        // handler; an unparseable document is not evidence of a successful
        // inspection.
        let parsed: serde_json::Value = serde_json::from_str(&output)
            .context("native capability output must be a JSON result document")?;
        if parsed.get("path").is_none() || parsed.get("sha256").is_none() {
            anyhow::bail!("native inspection result is missing required path/sha256 fields");
        }

        let events = vec![
            HarnessEvent {
                sequence: 0,
                correlation_id: step.step_id.clone(),
                kind: "started".to_string(),
                detail_digest: sha256_hex(format!("{}:started", step.step_id).as_bytes()),
            },
            HarnessEvent {
                sequence: 1,
                correlation_id: step.step_id.clone(),
                kind: "completed".to_string(),
                detail_digest: sha256_hex(format!("{}:completed", step.step_id).as_bytes()),
            },
        ];
        Ok(HarnessOutcome {
            harness_id: NATIVE_HARNESS_ID.to_string(),
            output,
            evidence_entry_digest: run_outcome.evidence_entry.entry_digest.clone(),
            events,
            content_digest: None,
        }
        .sealed())
    }

    async fn checkpoint(
        &self,
        _outcome: &HarnessOutcome,
        portable_state_digest: &str,
    ) -> Result<HarnessCheckpointV1> {
        // Bind this implementation to the digest of the portable work state
        // that lives in the store — process memory carries no durable truth.
        Ok(HarnessCheckpointV1 {
            schema_version: HARNESS_SCHEMA_VERSION.to_string(),
            harness_id: NATIVE_HARNESS_ID.to_string(),
            harness_revision: NATIVE_REVISION.to_string(),
            portable_state_digest: portable_state_digest.to_string(),
            resume_step: None,
            content_digest: None,
        }
        .sealed())
    }

    async fn resume(
        &self,
        checkpoint: &HarnessCheckpointV1,
        portable_state_digest: &str,
    ) -> Result<HarnessContinuation> {
        // Re-verify the sealed binding, then require exact identity,
        // revision, and portable-state agreement.
        checkpoint.verify()?;
        if checkpoint.harness_id != NATIVE_HARNESS_ID {
            anyhow::bail!(
                "harness checkpoint was exported by {:?}, not by {NATIVE_HARNESS_ID:?}",
                checkpoint.harness_id
            );
        }
        if checkpoint.harness_revision != NATIVE_REVISION {
            anyhow::bail!(
                "harness checkpoint revision {:?} is incompatible with this implementation ({NATIVE_REVISION:?})",
                checkpoint.harness_revision
            );
        }
        if checkpoint.portable_state_digest != portable_state_digest {
            anyhow::bail!(
                "harness checkpoint portable state digest does not match the supplied state"
            );
        }
        Ok(HarnessContinuation {
            harness_id: NATIVE_HARNESS_ID.to_string(),
            harness_revision: NATIVE_REVISION.to_string(),
            portable_state_digest: portable_state_digest.to_string(),
            resume_step: checkpoint.resume_step.clone(),
        })
    }
}

/// Build the governed `harness.native.inspect` capability for a runner's
/// registry, bound to an acquired workspace root.
///
/// The handler is `Fn` (reusable across invocations); each invocation is
/// independently validated and confined to `workspace_root`. Refusals never
/// mutate the workspace and never surface file content.
pub fn native_capability(config: NativeInspectConfig, workspace_root: PathBuf) -> Capability {
    Capability::deterministic(&["path"], move |args| {
        run_native_inspect_guarded(&config, &workspace_root, args)
    })
}

// ---------------------------------------------------------------------------
// Governed inspection
// ---------------------------------------------------------------------------

/// Decoded, strictly validated invocation arguments.
struct InspectArgs {
    path: String,
    known_secrets: Vec<String>,
}

fn decode_args(args: &serde_json::Value, max_args_bytes: usize) -> Result<InspectArgs> {
    let raw = serde_json::to_string(args).context("arguments serialize")?;
    if raw.len() > max_args_bytes {
        bail!(
            "native inspection arguments exceed budget ({} > {max_args_bytes} bytes)",
            raw.len()
        );
    }
    let map = args
        .as_object()
        .context("native inspection arguments must be a JSON object")?;
    for key in map.keys() {
        if key != "path" && key != "knownSecrets" {
            bail!("native inspection rejects unsupported argument {key:?}");
        }
    }
    let path = map
        .get("path")
        .and_then(serde_json::Value::as_str)
        .context("native inspection requires a string 'path' argument")?;
    if path.is_empty() {
        bail!("native inspection rejects an empty path");
    }
    let known_secrets = match map.get("knownSecrets") {
        None => Vec::new(),
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(str::to_string)
            .collect(),
        Some(_) => bail!("native inspection 'knownSecrets' must be an array of strings"),
    };
    Ok(InspectArgs {
        path: path.to_string(),
        known_secrets,
    })
}

/// Validate a caller-supplied path as hostile input before any filesystem
/// access. Absolute and `..` forms are refused on every platform; a Windows
/// drive/UNC form is refused even when running on Unix.
fn validate_relative_path(raw: &str, max_path_chars: usize) -> Result<()> {
    if raw.chars().count() > max_path_chars {
        bail!("native inspection path exceeds {max_path_chars} characters");
    }
    if raw.contains('\0') {
        bail!("native inspection path contains a NUL byte");
    }
    if Path::new(raw).is_absolute() || raw.starts_with('/') || raw.starts_with('\\') {
        bail!("native inspection path must be relative: {raw:?}");
    }
    let bytes = raw.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        bail!("native inspection path must be relative (drive form): {raw:?}");
    }
    for segment in raw.split(['/', '\\']) {
        if segment == ".." {
            bail!("native inspection path escapes the workspace: {raw:?}");
        }
    }
    Ok(())
}

/// Resolve the inspected file and prove it lies inside the workspace. The
/// canonicalized target must start with the canonicalized root, so a
/// symlink inside the workspace that points outside is refused. Missing
/// files and directories are refused.
fn confined_file(workspace_root: &Path, raw: &str) -> Result<(PathBuf, u64)> {
    let root = std::fs::canonicalize(workspace_root).with_context(|| {
        format!(
            "workspace root {} is not accessible",
            workspace_root.display()
        )
    })?;
    let candidate = root.join(raw);
    let canonical = match std::fs::canonicalize(&candidate) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            bail!("native inspection target does not exist: {raw:?}")
        }
        Err(e) => return Err(e).context("native inspection could not resolve the target path"),
    };
    if !canonical.starts_with(&root) {
        bail!("native inspection path escapes the workspace: {raw:?}");
    }
    let metadata = std::fs::metadata(&canonical)
        .with_context(|| format!("native inspection could not stat {raw:?}"))?;
    if !metadata.is_file() {
        bail!("native inspection target is not a regular file: {raw:?}");
    }
    Ok((canonical, metadata.len()))
}

/// Read and digest one confined file in bounded memory. The size budget is
/// checked against the stat size first, then re-verified against the bytes
/// actually read (a file growing mid-read cannot smuggle unbounded input).
/// Only the digest and size leave this function — content never does.
fn digest_confined_file(path: &Path, max_file_bytes: u64) -> Result<(String, u64)> {
    use sha2::Digest as _;
    let mut file = std::fs::File::open(path).context("native inspection could not open target")?;
    let mut hasher = sha2::Sha256::new();
    let mut total: u64 = 0;
    let mut buf = [0u8; READ_CHUNK];
    loop {
        let n = file
            .read(&mut buf)
            .context("native inspection read failed")?;
        if n == 0 {
            break;
        }
        total = total.saturating_add(n as u64);
        if total > max_file_bytes {
            bail!("native inspection file exceeds size budget ({max_file_bytes} bytes)");
        }
        hasher.update(&buf[..n]);
    }
    Ok((format!("{:x}", hasher.finalize()), total))
}

fn run_native_inspect_guarded(
    config: &NativeInspectConfig,
    workspace_root: &Path,
    args: &serde_json::Value,
) -> Result<String> {
    let decoded = decode_args(args, config.max_args_bytes)?;
    validate_relative_path(&decoded.path, config.max_path_chars)?;
    let (canonical, stat_size) = confined_file(workspace_root, &decoded.path)?;
    if stat_size > config.max_file_bytes {
        bail!(
            "native inspection file exceeds size budget ({} > {} bytes)",
            stat_size,
            config.max_file_bytes
        );
    }
    let (digest, read_bytes) = digest_confined_file(&canonical, config.max_file_bytes)?;
    if read_bytes != stat_size {
        bail!("native inspection file changed during read; refusing to report");
    }
    let result = serde_json::json!({
        "path": decoded.path,
        "sizeBytes": stat_size,
        "sha256": digest,
        "contentReturned": false,
    });
    let text = serde_json::to_string(&result).context("inspection result serializes")?;
    if text.len() > config.max_output_bytes {
        bail!(
            "native inspection result exceeds output budget ({} > {} bytes)",
            text.len(),
            config.max_output_bytes
        );
    }
    // Request-scoped secret handling, mirroring the governed boundary: the
    // result document is redacted with the request's known secrets before
    // it can reach evidence. File bytes never enter this document.
    let mut secrets = config.known_secrets.clone();
    secrets.extend(decoded.known_secrets);
    let redactor = Redactor::new().with_known_secrets(&secrets);
    let redacted = redactor.redact(&text);
    if redacted.len() > config.max_output_bytes {
        bail!("native inspection redacted result exceeds output budget");
    }
    Ok(redacted)
}
