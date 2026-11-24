//! Machine-readable source-trust evidence contract for a candidate Lite source
//! revision. This target is NONPRIVILEGED, deterministic and fail-closed.
//!
//! It validates the SHAPE and INTERNAL CONSISTENCY of an evidence document and
//! cross-checks a small set of facts against the checked-out repository. It
//! does NOT create, sign, authenticate or approve evidence. A pass means only
//! "this document is internally consistent and names the reviewed candidate";
//! it never grants source-trust clearance, runtime authority, or permission to
//! merge. Those remain owner/Principal decisions.
//!
//! The negative controls below each encode one unsafe condition from the
//! source-integrity recovery dispatch and must be rejected.

use serde_json::Value;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const SCHEMA: &str = "prometheos.source-trust-evidence.v1";
const PROJECT: &str = "PROMETHEOS_LITE_ONLY";
const CANDIDATE_SHA: &str = "4ce9529a7b0f1a08084e0de1c7a1440e1de90e26";
const CANDIDATE_TREE: &str = "306a453a64a769c4fc4d156802e22de8ef3c91ea";
const BASE_MAIN_SHA: &str = "4ce9529a7b0f1a08084e0de1c7a1440e1de90e26";
const TRUSTED_POSTCSS: &str = "33ad091d26d8a9dc95ebdf616e217d985ec215b8";
const APPROVED_WORKFLOW: &str = ".github/workflows/pr341-windows-acceptance.yml";
const ZERO_DIGEST: &str = "0000000000000000000000000000000000000000000000000000000000000000";
/// Repair-time `main`, already advanced past the historical candidate. The
/// evidence package stays pinned to the historical candidate; this constant
/// only labels the revision at which the repair was made.
const CURRENT_MAIN_SHA: &str = "770e68f5bb1163fdcf61c84f98b72a394840d390";
/// Historical artifact digest, bound to the historical candidate build only.
/// It must never be re-labelled as a digest of a new build.
const HISTORICAL_ARTIFACT_SHA256: &str =
    "27a8c5b8388b0fd01be71d475a8c94665e08f8a025b08203f1f23ea0388de156";

/// Payload-shaped blobs preserved from issue #297. Never executed, only
/// compared by content hash. Only full hashes recorded in the preserved
/// evidence are listed here; other families are known only by prefix
/// (`adf4e2a8…`, `cb93f605…`) and are not invented.
const SUSPECT_BLOBS: [&str; 2] = [
    "6f62619514e903628e55e7c99c7014a35657b1fb",
    "069bcc5d5536de0fdb89316a3f7aeceefba298ad",
];
const SUSPECT_COMMITS: [&str; 3] = [
    "6d6fb679b1b383a1f34266ceada4d1cab381140c",
    "14b83781405e1dccc737da74b2203bd00593ccb4",
    "468017b4a78e5ed2aa988ed685de960d34ad24ac",
];
const ALLOWED_STATUS: [&str; 5] = ["VERIFIED", "FAILED", "BLOCKED", "NOT_RUN", "UNKNOWN"];
const ADMIN_FIELDS: [&str; 5] = [
    "AUTHENTICATED_PUSHING_ACTOR",
    "WRITER_ISOLATION",
    "CREDENTIAL_ROTATION",
    "EXPOSURE_REVIEW",
    "ADMINISTRATOR_RECEIPT",
];
const LIFECYCLE_HOOKS: [&str; 5] = [
    "preinstall",
    "install",
    "postinstall",
    "prepare",
    "prepublish",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn field<'a>(root: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = root;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current)
}

fn req_str<'a>(root: &'a Value, path: &[&str], errors: &mut Vec<String>) -> Option<&'a str> {
    match field(root, path).and_then(Value::as_str) {
        Some(value) if !value.is_empty() => Some(value),
        _ => {
            errors.push(format!("missing or empty {}", path.join(".")));
            None
        }
    }
}

fn req_bool(root: &Value, path: &[&str], errors: &mut Vec<String>) -> Option<bool> {
    match field(root, path).and_then(Value::as_bool) {
        Some(value) => Some(value),
        None => {
            errors.push(format!("missing boolean {}", path.join(".")));
            None
        }
    }
}

fn req_strings(root: &Value, path: &[&str], errors: &mut Vec<String>) -> Vec<String> {
    match field(root, path).and_then(Value::as_array) {
        Some(values) => values
            .iter()
            .map(|value| value.as_str().unwrap_or_default().to_string())
            .collect(),
        None => {
            errors.push(format!("missing string array {}", path.join(".")));
            Vec::new()
        }
    }
}

fn status<'a>(root: &'a Value, key: &str, errors: &mut Vec<String>) -> Option<&'a str> {
    match req_str(root, &[key], errors) {
        Some(value) if ALLOWED_STATUS.contains(&value) => Some(value),
        Some(value) => {
            errors.push(format!(
                "{key}: status '{value}' is outside the allowed vocabulary"
            ));
            None
        }
        None => None,
    }
}

fn is_hex64(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// S3: a missing Git fact is never an empty, clean history. Every Git query
/// used by the validator goes through this fail-closed wrapper.
fn git_required(root: &Path, args: &[&str]) -> Result<String, String> {
    git(root, args).ok_or_else(|| format!("git query failed: {}", args.join(" ")))
}

/// S1: an on-disk fixture whose creation and removal are ownership-bound.
/// The root is created with an exclusive `create_dir`; nested directories and
/// files are created exclusively and tracked. Cleanup removes ONLY those
/// tracked paths, deepest first, with non-recursive syscalls. A collision or
/// substituted symlink is refused rather than adopted or deleted.
struct OwnedFixture {
    root: PathBuf,
    dirs: Vec<PathBuf>,
    files: Vec<PathBuf>,
}

/// Exclusive directory creation: an existing path (including a planted
/// symlink) is a collision, never a fixture to adopt.
fn exclusive_create_dir(path: &Path) -> Result<(), String> {
    match std::fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Err("collision: target path already exists".into())
        }
        Err(error) => Err(format!("exclusive creation failed: {error}")),
    }
}

impl OwnedFixture {
    fn allocate(label: &str) -> Result<Self, String> {
        let base = std::env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("clock unavailable: {error}"))?
            .as_nanos();
        for sequence in 0..32_u32 {
            let name = format!(
                "prometheos-{label}-{}-{nanos:x}-{sequence:x}",
                std::process::id()
            );
            let root = base.join(name);
            match exclusive_create_dir(&root) {
                Ok(()) => {
                    let metadata = std::fs::symlink_metadata(&root)
                        .map_err(|error| format!("fixture root unverifiable: {error}"))?;
                    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
                        return Err("fixture root is not an owned real directory".into());
                    }
                    return Ok(Self {
                        root,
                        dirs: Vec::new(),
                        files: Vec::new(),
                    });
                }
                Err(error) if error.contains("collision") => continue,
                Err(error) => return Err(format!("fixture allocation failed: {error}")),
            }
        }
        Err("exclusive fixture allocation exhausted (collision)".into())
    }

    fn add_file(&mut self, relative: &str, contents: &str) -> Result<PathBuf, String> {
        let relative_path = Path::new(relative);
        if relative_path.is_absolute()
            || relative_path
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err("fixture path must be relative and component-clean".into());
        }
        let mut current = self.root.clone();
        if let Some(parent) = relative_path.parent() {
            for component in parent.components() {
                current.push(component);
                if !current.exists() {
                    exclusive_create_dir(&current)
                        .map_err(|error| format!("fixture directory creation failed: {error}"))?;
                    self.dirs.push(current.clone());
                } else {
                    let metadata = std::fs::symlink_metadata(&current)
                        .map_err(|error| format!("fixture path unverifiable: {error}"))?;
                    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
                        return Err("fixture path substituted by a non-directory or symlink".into());
                    }
                }
            }
        }
        let file_path = self.root.join(relative_path);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&file_path)
            .map_err(|error| format!("exclusive fixture file creation failed: {error}"))?;
        file.write_all(contents.as_bytes())
            .map_err(|error| format!("fixture write failed: {error}"))?;
        self.files.push(file_path.clone());
        Ok(file_path)
    }

    fn cleanup(&self) -> Result<(), String> {
        let is_under_root = |path: &Path| {
            path.starts_with(&self.root)
                && std::fs::symlink_metadata(path)
                    .is_ok_and(|metadata| !metadata.file_type().is_symlink())
        };
        for file in self.files.iter().rev() {
            if !is_under_root(file) {
                return Err("refusing to remove a fixture path that is no longer owned".into());
            }
            std::fs::remove_file(file)
                .map_err(|error| format!("owned fixture file cleanup failed: {error}"))?;
        }
        for dir in self.dirs.iter().rev() {
            if !is_under_root(dir) {
                return Err(
                    "refusing to remove a fixture directory that is no longer owned".into(),
                );
            }
            std::fs::remove_dir(dir)
                .map_err(|error| format!("owned fixture directory cleanup failed: {error}"))?;
        }
        if !is_under_root(&self.root) {
            return Err("fixture root no longer owned".into());
        }
        std::fs::remove_dir(&self.root)
            .map_err(|error| format!("owned fixture root cleanup failed: {error}"))
    }
}

/// Read the one approved workflow from disk and derive its permission block.
/// This is independent of the document's own claims: a document cannot assert
/// `contents: read` while the executable workflow on disk grants more.
fn actual_workflow_permissions(root: &Path) -> Result<(Vec<String>, bool), String> {
    let path = root.join(APPROVED_WORKFLOW);
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("approved workflow unreadable: {error}"))?;
    let mut block = Vec::new();
    let mut capturing = false;
    for line in text.lines() {
        if line.starts_with("permissions:") {
            capturing = true;
            continue;
        }
        if capturing {
            if line.starts_with(char::is_whitespace) {
                let trimmed = line.trim();
                if !trimmed.is_empty() && !trimmed.starts_with('#') {
                    block.push(trimmed.to_string());
                }
            } else {
                break;
            }
        }
    }
    let uses_secrets = text.contains("secrets.") || text.contains("github.token");
    Ok((block, uses_secrets))
}

/// S2: independently enumerate the real workflow directory. Symlinks,
/// non-file entries, an unreadable directory and unexpected files are all
/// refused; the caller-supplied JSON array is never trusted on its own.
fn actual_workflow_inventory(root: &Path) -> Result<Vec<String>, String> {
    let directory = root.join(".github").join("workflows");
    let metadata = std::fs::symlink_metadata(&directory)
        .map_err(|error| format!("workflow inventory unreadable: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        return Err("workflow inventory path is a symlink or not a directory".into());
    }
    let mut names = Vec::new();
    let entries = std::fs::read_dir(&directory)
        .map_err(|error| format!("workflow inventory enumeration failed: {error}"))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("workflow entry unreadable: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("workflow entry type unreadable: {error}"))?;
        if file_type.is_symlink() || !file_type.is_file() {
            return Err(format!(
                "unsafe workflow entry (symlink or non-file): {}",
                entry.file_name().to_string_lossy()
            ));
        }
        names.push(format!(
            ".github/workflows/{}",
            entry.file_name().to_string_lossy()
        ));
    }
    names.sort();
    Ok(names)
}

/// S4: read the tracked frontend manifest and report only what it actually
/// contains. This is a narrow cross-check, not SBOM/signature verification.
fn actual_manifest_hooks(root: &Path) -> Result<(Vec<String>, Vec<String>), String> {
    let path = root.join("frontend").join("package.json");
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("frontend manifest unreadable: {error}"))?;
    let manifest: Value = serde_json::from_str(&text)
        .map_err(|error| format!("frontend manifest malformed: {error}"))?;
    let scripts = manifest
        .get("scripts")
        .and_then(Value::as_object)
        .ok_or("frontend manifest has no scripts object")?;
    let mut all = scripts.keys().cloned().collect::<Vec<_>>();
    all.sort();
    let mut hooks = all
        .iter()
        .filter(|name| LIFECYCLE_HOOKS.contains(&name.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    hooks.sort();
    Ok((all, hooks))
}

/// Fail-closed validation. Returns findings; an empty vector is the only pass.
fn validate(doc: &Value, root: &Path) -> Vec<String> {
    let mut errors = Vec::new();

    if req_str(doc, &["schemaVersion"], &mut errors) != Some(SCHEMA) {
        errors.push("unknown schemaVersion".into());
    }
    if req_str(doc, &["PROJECT"], &mut errors) != Some(PROJECT) {
        errors.push("PROJECT binding mismatch".into());
    }

    // Required fields, present and status-valued.
    for key in [
        "CANDIDATE_SOURCE_SHA",
        "CANDIDATE_TREE_SHA",
        "BASE_MAIN_SHA",
        "BUILD_COMMAND",
        "BUILD_ARTIFACT_SHA256",
        "BUILD_ENVIRONMENT",
    ] {
        req_str(doc, &[key], &mut errors);
    }
    if req_strings(doc, &["BUILD_INPUTS"], &mut errors).is_empty() {
        errors.push("BUILD_INPUTS must not be empty".into());
    }
    if field(doc, &["TEST_RESULTS"])
        .and_then(Value::as_object)
        .is_none_or(|results| results.is_empty())
    {
        errors.push("TEST_RESULTS must be a non-empty object".into());
    }
    for key in [
        "SOURCE_ANCESTRY_VERDICT",
        "KNOWN_SUSPECT_CONTENT_IN_CANDIDATE",
        "KNOWN_SUSPECT_ANCESTRY",
        "DEPENDENCY_AND_BUILD_INPUT_REVIEW",
        "AUTHENTICATED_PUSHING_ACTOR",
        "WRITER_ISOLATION",
        "CREDENTIAL_ROTATION",
        "EXPOSURE_REVIEW",
        "ADMINISTRATOR_RECEIPT",
        "BUILD_REPRODUCIBILITY",
        "SOURCE_INTEGRITY_STATUS",
        "SECURITY_CLEARANCE",
        "RUNTIME_AUTHORITY",
    ] {
        status(doc, key, &mut errors);
    }
    if req_bool(doc, &["PRINCIPAL_REVIEW_REQUIRED"], &mut errors) != Some(true) {
        errors.push("PRINCIPAL_REVIEW_REQUIRED must be true".into());
    }

    let candidate = req_str(doc, &["CANDIDATE_SOURCE_SHA"], &mut errors).unwrap_or_default();
    let tree = req_str(doc, &["CANDIDATE_TREE_SHA"], &mut errors).unwrap_or_default();
    let base = req_str(doc, &["BASE_MAIN_SHA"], &mut errors).unwrap_or_default();

    // Quarantined revisions must never be nominated as candidate or base.
    for suspect in SUSPECT_COMMITS {
        if candidate == suspect || base == suspect {
            errors.push(format!(
                "quarantined revision used as source SHA: {suspect}"
            ));
        }
    }
    if candidate != CANDIDATE_SHA || base != BASE_MAIN_SHA || tree != CANDIDATE_TREE {
        errors.push("candidate revision is not the reviewed candidate".into());
    }

    // Observations: known suspect content and ancestry.
    let observed_blob =
        req_str(doc, &["observations", "postcss_blob"], &mut errors).unwrap_or_default();
    for suspect in SUSPECT_BLOBS {
        if observed_blob == suspect {
            errors.push("known suspect artifact named as trusted content".into());
        }
    }
    if observed_blob != TRUSTED_POSTCSS {
        errors.push("postcss blob is not the independently trusted blob".into());
    }
    let actual_blob = match git_required(root, &["rev-parse", "HEAD:frontend/postcss.config.js"]) {
        Ok(blob) => blob,
        Err(error) => {
            errors.push(format!("S3 FAIL-CLOSED: {error}"));
            String::new()
        }
    };
    if !actual_blob.is_empty() && actual_blob != observed_blob {
        errors.push("declared postcss blob does not match the checked-out tree".into());
    }
    if req_bool(
        doc,
        &["observations", "suspect_commits_in_ancestry_empty"],
        &mut errors,
    ) != Some(true)
    {
        errors.push("suspect ancestry observation is not asserted empty".into());
    }
    let ancestry = match git_required(root, &["rev-list", candidate]) {
        Ok(value) => value,
        Err(error) => {
            errors.push(format!("S3 FAIL-CLOSED: {error}"));
            String::new()
        }
    };
    for suspect in SUSPECT_COMMITS {
        if ancestry.lines().any(|line| line == suspect) {
            errors.push(format!(
                "suspect commit {suspect} is reachable from the candidate"
            ));
        }
    }
    if field(doc, &["observations", "candidate_tree_sha"]).and_then(Value::as_str) != Some(tree) {
        errors.push("declared tree SHA does not match CANDIDATE_TREE_SHA".into());
    }
    let actual_tree = match git_required(root, &["rev-parse", &format!("{candidate}^{{tree}}")]) {
        Ok(value) => value,
        Err(error) => {
            errors.push(format!("S3 FAIL-CLOSED: {error}"));
            String::new()
        }
    };
    if !actual_tree.is_empty() && actual_tree != tree {
        errors.push("CANDIDATE_TREE_SHA does not match the candidate commit".into());
    }

    // Build hooks / workflows: the single reviewed workflow only.
    let workflows = req_strings(doc, &["observations", "workflow_files"], &mut errors);
    if workflows.len() != 1 || workflows.first().map(String::as_str) != Some(APPROVED_WORKFLOW) {
        errors.push("unapproved or additional executable workflow is present".into());
    }
    let permissions = req_strings(doc, &["observations", "workflow_permissions"], &mut errors);
    if permissions.len() != 1 || permissions.first().map(String::as_str) != Some("contents: read") {
        errors.push("workflow permission expansion is not permitted".into());
    }
    if req_bool(doc, &["observations", "workflow_uses_secrets"], &mut errors) != Some(false) {
        errors.push("reviewed workflow must not reference secrets".into());
    }
    match actual_workflow_permissions(root) {
        Ok((disk_permissions, disk_uses_secrets)) => {
            if disk_permissions != permissions {
                errors.push("declared workflow permissions differ from the file on disk".into());
            }
            if disk_permissions.len() != 1
                || disk_permissions.first().map(String::as_str) != Some("contents: read")
            {
                errors.push("executable workflow on disk has over-broad permissions".into());
            }
            if disk_uses_secrets {
                errors.push("executable workflow on disk references secrets or tokens".into());
            }
        }
        Err(error) => errors.push(error),
    }
    // S2: the documented list is a claim; the on-disk directory is the fact.
    match actual_workflow_inventory(root) {
        Ok(mut disk) => {
            disk.dedup();
            let mut declared = workflows.clone();
            declared.sort();
            declared.dedup();
            if disk != declared {
                errors.push(
                    "S2 FAIL-CLOSED: on-disk workflow inventory differs from documented list"
                        .into(),
                );
            }
            if disk.len() != 1 || disk.first().map(String::as_str) != Some(APPROVED_WORKFLOW) {
                errors.push("S2 FAIL-CLOSED: unexpected workflow file on disk".into());
            }
        }
        Err(error) => errors.push(format!("S2 FAIL-CLOSED: {error}")),
    }
    if req_bool(
        doc,
        &["observations", "unreviewed_executable_inputs_empty"],
        &mut errors,
    ) != Some(true)
    {
        errors.push("unreviewed executable build input is declared".into());
    }

    // Dependency lifecycle execution: none may be introduced.
    let hooks = req_strings(
        doc,
        &["observations", "dependency_lifecycle_scripts"],
        &mut errors,
    );
    for hook in &hooks {
        if LIFECYCLE_HOOKS.contains(&hook.as_str()) {
            errors.push(format!(
                "unexpected executable dependency lifecycle script: {hook}"
            ));
        }
    }
    // S4: cross-check the documentary assertion against the tracked manifest.
    // This is deliberately narrow: one manifest, no SBOM, no signature.
    match actual_manifest_hooks(root) {
        Ok((all_scripts, manifest_hooks)) => {
            if field(doc, &["observations", "manifest_crosscheck", "source"])
                .and_then(Value::as_str)
                != Some("frontend/package.json")
            {
                errors.push("S4 FAIL-CLOSED: manifest cross-check source is not identified".into());
            }
            if req_bool(
                doc,
                &[
                    "observations",
                    "manifest_crosscheck",
                    "lifecycle_hooks_present",
                ],
                &mut errors,
            ) != Some(!manifest_hooks.is_empty())
            {
                errors.push(
                    "S4 FAIL-CLOSED: manifest lifecycle assertion differs from the manifest".into(),
                );
            }
            let documented = req_strings(
                doc,
                &["observations", "manifest_crosscheck", "scripts"],
                &mut errors,
            );
            if documented != all_scripts {
                errors.push("S4 FAIL-CLOSED: documented scripts differ from the manifest".into());
            }
            for hook in manifest_hooks {
                errors.push(format!(
                    "S4 FAIL-CLOSED: actual manifest lifecycle hook: {hook}"
                ));
            }
        }
        Err(error) => errors.push(format!("S4 FAIL-CLOSED: {error}")),
    }
    if req_str(doc, &["notes", "dependency_assertion_scope"], &mut errors)
        .is_none_or(|note| !note.contains("not full supply-chain proof"))
    {
        errors.push("dependency assertions must be labelled as not full supply-chain proof".into());
    }

    // Build evidence binding and digest.
    if field(doc, &["build", "evidence_candidate_sha"]).and_then(Value::as_str) != Some(candidate) {
        errors.push("build evidence is bound to a different source revision".into());
    }
    let digest = req_str(doc, &["BUILD_ARTIFACT_SHA256"], &mut errors).unwrap_or_default();
    if !is_hex64(digest) || digest == ZERO_DIGEST {
        errors.push("BUILD_ARTIFACT_SHA256 is not a real SHA-256 digest".into());
    }
    if field(doc, &["build", "artifact_sha256"]).and_then(Value::as_str) != Some(digest) {
        errors.push("duplicate artifact digest fields disagree".into());
    }

    // Clearance cannot be derived from a documentary check, and possible
    // runtime acceptance must never be claimed from a non-existent test.
    let runtime = [
        "stage2_executed",
        "runtime_test_executed",
        "runtime_acceptance_claimed",
    ];
    for key in runtime {
        if req_bool(doc, &["runtime", key], &mut errors) != Some(false) {
            errors.push(format!("runtime.{key} must remain false"));
        }
    }
    if req_bool(
        doc,
        &["runtime", "observation_boundary_active"],
        &mut errors,
    ) != Some(false)
    {
        errors.push("observation boundary must not be activated by evidence".into());
    }

    // Conflicting clearance and exclusion rules.
    let required = req_strings(doc, &["clearance", "required_revisions"], &mut errors);
    let excluded = req_strings(doc, &["clearance", "excluded_revisions"], &mut errors);
    if !required.iter().any(|value| value == candidate) {
        errors.push("candidate revision is not in the required revision set".into());
    }
    for suspect in SUSPECT_COMMITS {
        if !excluded.iter().any(|value| value == suspect) {
            errors.push(format!("quarantined revision {suspect} is not excluded"));
        }
    }
    if excluded.iter().any(|value| value == candidate)
        || required.iter().any(|value| excluded.contains(value))
    {
        errors.push("clearance required/excluded revision rules conflict".into());
    }

    // Unattributed clearance: admin gaps gate every clearance-looking verdict.
    let admin_blocked = ADMIN_FIELDS
        .iter()
        .any(|key| status(doc, key, &mut errors) != Some("VERIFIED"));
    if admin_blocked {
        for key in [
            "SOURCE_INTEGRITY_STATUS",
            "SECURITY_CLEARANCE",
            "RUNTIME_AUTHORITY",
        ] {
            if status(doc, key, &mut errors) == Some("VERIFIED") {
                errors.push(format!(
                    "{key} cannot be VERIFIED with unresolved admin fields"
                ));
            }
        }
    }
    if req_str(doc, &["notes", "security_clearance"], &mut errors)
        .is_none_or(|note| !note.contains("NOT_SELF_GRANTED"))
    {
        errors.push("clearance note must record NOT_SELF_GRANTED".into());
    }
    // S5: the package is immutable historical evidence; it must not masquerade
    // as a clearance of the current main, nor re-use the historical digest.
    if req_str(
        doc,
        &["notes", "historical_vs_current", "historical_candidate_sha"],
        &mut errors,
    ) != Some(CANDIDATE_SHA)
    {
        errors.push("S5 FAIL-CLOSED: historical candidate SHA not identified".into());
    }
    if req_str(
        doc,
        &["notes", "historical_vs_current", "current_main_at_repair"],
        &mut errors,
    ) != Some(CURRENT_MAIN_SHA)
    {
        errors.push("S5 FAIL-CLOSED: current main at repair time must be distinguished".into());
    }
    if req_bool(
        doc,
        &[
            "notes",
            "historical_vs_current",
            "evidence_applies_to_current_main",
        ],
        &mut errors,
    ) != Some(false)
    {
        errors
            .push("S5 FAIL-CLOSED: historical evidence must not be applied to current main".into());
    }
    if req_str(doc, &["build", "artifact_sha256"], &mut errors) == Some(HISTORICAL_ARTIFACT_SHA256)
        && req_str(
            doc,
            &[
                "notes",
                "historical_vs_current",
                "artifact_digest_is_historical",
            ],
            &mut errors,
        ) != Some("YES")
    {
        errors.push(
            "S5 FAIL-CLOSED: historical artifact digest must be labelled as historical".into(),
        );
    }

    // False-positive READY states: no aggregate ready/cleared flag is allowed.
    for key in ["READY", "ready", "overallStatus", "cleared", "CLEARED"] {
        if let Some(value) = doc.get(key) {
            let text = value.as_str().unwrap_or_default();
            if value.as_bool() == Some(true) || text == "READY" || text == "VERIFIED" {
                errors.push(format!("false-positive READY state via '{key}'"));
            }
        }
    }
    errors
}

fn evidence_document() -> Value {
    let path = repo_root().join("docs/evidence/source-trust/candidate-4ce9529.json");
    let text = std::fs::read_to_string(path).expect("committed evidence package is readable");
    serde_json::from_str(&text).expect("committed evidence package is valid JSON")
}

fn expect_rejected(label: &str, mutate: impl FnOnce(&mut Value)) {
    let mut doc = evidence_document();
    mutate(&mut doc);
    let errors = validate(&doc, &repo_root());
    assert!(!errors.is_empty(), "{label}: unsafe document was accepted");
}

fn set(doc: &mut Value, path: &[&str], value: Value) {
    let mut current = doc;
    for key in &path[..path.len() - 1] {
        current = current
            .get_mut(*key)
            .unwrap_or_else(|| panic!("missing path {} in fixture", path.join(".")));
    }
    current
        .as_object_mut()
        .unwrap_or_else(|| panic!("path {} is not an object", path.join(".")))
        .insert(path[path.len() - 1].to_string(), value);
}

#[test]
fn committed_evidence_package_is_internally_consistent() {
    let errors = validate(&evidence_document(), &repo_root());
    assert!(errors.is_empty(), "valid evidence rejected: {errors:#?}");
}

#[test]
fn known_malicious_artifact_is_rejected() {
    expect_rejected("suspect postcss blob", |doc| {
        set(
            doc,
            &["observations", "postcss_blob"],
            Value::String(SUSPECT_BLOBS[0].into()),
        );
    });
}

#[test]
fn mismatched_source_revision_and_build_evidence_is_rejected() {
    expect_rejected("build bound to other SHA", |doc| {
        set(
            doc,
            &["build", "evidence_candidate_sha"],
            Value::String(SUSPECT_COMMITS[0].into()),
        );
    });
}

#[test]
fn unauthorized_workflow_permission_expansion_is_rejected() {
    expect_rejected("workflow write permission", |doc| {
        set(
            doc,
            &["observations", "workflow_permissions"],
            serde_json::json!(["contents: write"]),
        );
    });
}

#[test]
fn on_disk_workflow_permission_expansion_is_detected_independently() {
    // The document is honest; the executable file on disk is not. The fixture
    // is exclusively allocated and only its own tracked paths are removed.
    let mut fixture = OwnedFixture::allocate("workflow-perm").expect("exclusive fixture");
    let original = std::fs::read_to_string(repo_root().join(APPROVED_WORKFLOW)).unwrap();
    let tampered = original.replace("contents: read", "contents: write");
    assert_ne!(
        tampered, original,
        "fixture must actually change the permission"
    );
    fixture.add_file(APPROVED_WORKFLOW, &tampered).unwrap();
    let errors = validate(&evidence_document(), &fixture.root);
    let cleanup = fixture.cleanup();
    assert!(
        errors
            .iter()
            .any(|error| error.contains("on disk has over-broad permissions")),
        "on-disk permission expansion was not detected: {errors:#?}"
    );
    assert!(cleanup.is_ok(), "owned fixture cleanup failed: {cleanup:?}");
}

#[test]
fn owned_fixture_refuses_collision_and_preserves_unowned_content() {
    let parent = OwnedFixture::allocate("collision-parent").expect("exclusive parent");
    let target = parent.root.join("occupied");
    exclusive_create_dir(&target).expect("first allocation succeeds");
    std::fs::write(target.join("sentinel.txt"), "preexisting").unwrap();
    let collision = exclusive_create_dir(&target);
    assert!(
        collision.is_err(),
        "exclusive allocator must refuse a collision"
    );
    assert_eq!(
        std::fs::read_to_string(target.join("sentinel.txt")).unwrap(),
        "preexisting",
        "a refused allocation must not alter unowned content"
    );
    // Cleanup only removes paths this test created; the unoccupied subtree
    // remains, so root removal fails closed rather than recursively deleting.
    assert!(parent.cleanup().is_err());
    let _ = std::fs::remove_file(target.join("sentinel.txt"));
    let _ = std::fs::remove_dir(&target);
    let _ = parent.cleanup();
}

#[test]
fn owned_fixture_refuses_symlink_substitution() {
    #[cfg(windows)]
    {
        let parent = OwnedFixture::allocate("substitution-parent").expect("exclusive parent");
        let victim = parent.root.join("victim");
        exclusive_create_dir(&victim).unwrap();
        std::fs::write(victim.join("sentinel.txt"), "untouched").unwrap();
        let link = parent.root.join("substituted");
        if std::os::windows::fs::symlink_dir(&victim, &link).is_err() {
            eprintln!("SKIP: symlink creation not permitted on this host");
            let _ = parent.cleanup();
            return;
        }
        assert!(
            exclusive_create_dir(&link).is_err(),
            "a planted symlink must be a refused collision"
        );
        assert_eq!(
            std::fs::read_to_string(victim.join("sentinel.txt")).unwrap(),
            "untouched"
        );
        // Windows directory symlinks are removed with remove_dir, not
        // remove_file; this only removes the link this fixture created.
        std::fs::remove_dir(&link).unwrap();
        std::fs::remove_file(victim.join("sentinel.txt")).unwrap();
        std::fs::remove_dir(&victim).unwrap();
        assert!(parent.cleanup().is_ok());
    }
    #[cfg(not(windows))]
    {
        eprintln!("SKIP: Windows symlink substitution fixture");
    }
}

#[test]
fn suspicious_build_hook_path_is_rejected() {
    expect_rejected("extra workflow", |doc| {
        set(
            doc,
            &["observations", "workflow_files"],
            serde_json::json!([APPROVED_WORKFLOW, ".github/workflows/evil.yml"]),
        );
    });
}

#[test]
fn unexpected_dependency_lifecycle_script_is_rejected() {
    expect_rejected("postinstall hook", |doc| {
        set(
            doc,
            &["observations", "dependency_lifecycle_scripts"],
            serde_json::json!(["postinstall"]),
        );
    });
    expect_rejected("undeclared executable input", |doc| {
        set(
            doc,
            &["observations", "unreviewed_executable_inputs_empty"],
            Value::Bool(false),
        );
    });
}

#[test]
fn missing_provenance_field_is_rejected() {
    expect_rejected("missing actor field", |doc| {
        doc.as_object_mut()
            .unwrap()
            .remove("AUTHENTICATED_PUSHING_ACTOR");
    });
}

#[test]
fn incorrect_artifact_digest_is_rejected() {
    expect_rejected("zero digest", |doc| {
        set(
            doc,
            &["BUILD_ARTIFACT_SHA256"],
            Value::String(ZERO_DIGEST.into()),
        );
    });
    expect_rejected("digest field disagreement", |doc| {
        set(
            doc,
            &["build", "artifact_sha256"],
            Value::String("a".repeat(64)),
        );
    });
}

#[test]
fn unattributed_source_clearance_is_rejected() {
    expect_rejected("self-granted integrity verdict", |doc| {
        set(
            doc,
            &["SOURCE_INTEGRITY_STATUS"],
            Value::String("VERIFIED".into()),
        );
    });
    expect_rejected("self-granted security clearance", |doc| {
        set(
            doc,
            &["SECURITY_CLEARANCE"],
            Value::String("VERIFIED".into()),
        );
    });
    expect_rejected("erased review requirement", |doc| {
        set(doc, &["PRINCIPAL_REVIEW_REQUIRED"], Value::Bool(false));
    });
}

#[test]
fn quarantined_source_revision_is_rejected() {
    expect_rejected("suspect candidate", |doc| {
        set(
            doc,
            &["CANDIDATE_SOURCE_SHA"],
            Value::String(SUSPECT_COMMITS[1].into()),
        );
    });
    expect_rejected("suspect base", |doc| {
        set(
            doc,
            &["BASE_MAIN_SHA"],
            Value::String(SUSPECT_COMMITS[2].into()),
        );
    });
}

#[test]
fn conflicting_clearance_rules_are_rejected() {
    expect_rejected("candidate excluded", |doc| {
        set(
            doc,
            &["clearance", "excluded_revisions"],
            serde_json::json!([CANDIDATE_SHA]),
        );
    });
    expect_rejected("quarantine omitted", |doc| {
        set(
            doc,
            &["clearance", "excluded_revisions"],
            serde_json::json!([]),
        );
    });
}

#[test]
fn false_ready_state_is_rejected() {
    expect_rejected("explicit READY", |doc| {
        set(doc, &["READY"], Value::Bool(true));
    });
    expect_rejected("aggregate ready status", |doc| {
        set(doc, &["overallStatus"], Value::String("READY".into()));
    });
}

#[test]
fn runtime_acceptance_claim_without_runtime_test_is_rejected() {
    expect_rejected("claimed Stage 2 acceptance", |doc| {
        set(doc, &["runtime", "stage2_executed"], Value::Bool(true));
    });
    expect_rejected("claimed runtime acceptance", |doc| {
        set(
            doc,
            &["runtime", "runtime_acceptance_claimed"],
            Value::Bool(true),
        );
    });
    expect_rejected("activated observation boundary", |doc| {
        set(
            doc,
            &["runtime", "observation_boundary_active"],
            Value::Bool(true),
        );
    });
}

#[test]
fn tree_digest_mismatch_is_rejected() {
    expect_rejected("tree mismatch", |doc| {
        set(doc, &["CANDIDATE_TREE_SHA"], Value::String("ab".repeat(20)));
    });
}

#[test]
fn status_outside_allowed_vocabulary_is_rejected() {
    expect_rejected("invented status", |doc| {
        set(
            doc,
            &["SOURCE_ANCESTRY_VERDICT"],
            Value::String("PROBABLY_FINE".into()),
        );
    });
}

#[test]
fn suspect_ancestry_observation_cannot_be_hidden() {
    expect_rejected("hidden suspect ancestry", |doc| {
        set(
            doc,
            &["observations", "suspect_commits_in_ancestry_empty"],
            Value::Bool(false),
        );
    });
}

#[test]
fn on_disk_extra_workflow_is_rejected() {
    // S2: the JSON still lists only the approved workflow, but disk has two.
    let mut fixture = OwnedFixture::allocate("extra-workflow").expect("exclusive fixture");
    let original = std::fs::read_to_string(repo_root().join(APPROVED_WORKFLOW)).unwrap();
    fixture.add_file(APPROVED_WORKFLOW, &original).unwrap();
    fixture
        .add_file(".github/workflows/evil.yml", "name: evil\non: push\n")
        .unwrap();
    let errors = validate(&evidence_document(), &fixture.root);
    let cleanup = fixture.cleanup();
    assert!(
        errors
            .iter()
            .any(|error| error.contains("unexpected workflow file on disk")),
        "extra on-disk workflow was not detected: {errors:#?}"
    );
    assert!(cleanup.is_ok(), "owned fixture cleanup failed: {cleanup:?}");
}

#[test]
fn unsafe_workflow_inventory_is_refused() {
    // S2: a regular file where the workflow directory should be is not an
    // empty inventory; it is an unverifiable inventory and must fail closed.
    let mut fixture = OwnedFixture::allocate("bad-inventory").expect("exclusive fixture");
    fixture
        .add_file(".github/workflows", "not a directory\n")
        .unwrap();
    let errors = validate(&evidence_document(), &fixture.root);
    let cleanup = fixture.cleanup();
    assert!(
        errors
            .iter()
            .any(|error| error.contains("workflow inventory path is a symlink or not a directory")),
        "unreadable workflow inventory was accepted: {errors:#?}"
    );
    assert!(cleanup.is_ok(), "owned fixture cleanup failed: {cleanup:?}");
}

#[test]
fn missing_git_evidence_fails_closed() {
    // S3: a fixture that is not a Git worktree yields failed queries. Those
    // failures must be explicit findings, never an empty clean ancestry.
    let fixture = OwnedFixture::allocate("git-failure").expect("exclusive fixture");
    let errors = validate(&evidence_document(), &fixture.root);
    let cleanup = fixture.cleanup();
    assert!(
        errors
            .iter()
            .any(|error| error.contains("S3 FAIL-CLOSED: git query failed: rev-list")),
        "failed ancestry query was not refused: {errors:#?}"
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("S3 FAIL-CLOSED: git query failed: rev-parse")),
        "failed tree/blob query was not refused: {errors:#?}"
    );
    assert!(cleanup.is_ok(), "owned fixture cleanup failed: {cleanup:?}");
}

#[test]
fn unresolvable_candidate_sha_fails_closed() {
    let mut doc = evidence_document();
    set(
        &mut doc,
        &["CANDIDATE_SOURCE_SHA"],
        Value::String("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef".into()),
    );
    let errors = validate(&doc, &repo_root());
    assert!(
        errors
            .iter()
            .any(|error| error.contains("S3 FAIL-CLOSED: git query failed: rev-list")),
        "unresolvable candidate was treated as clean ancestry: {errors:#?}"
    );
}

#[test]
fn actual_manifest_lifecycle_hook_is_rejected() {
    // S4: a real hook in the tracked manifest is refused even when the
    // documentary assertion still claims none.
    let mut fixture = OwnedFixture::allocate("manifest-hook").expect("exclusive fixture");
    let original =
        std::fs::read_to_string(repo_root().join("frontend").join("package.json")).unwrap();
    let tampered = original.replacen(
        "\"dev\":",
        "\"postinstall\": \"echo not executed\",\n    \"dev\":",
        1,
    );
    assert_ne!(tampered, original, "fixture must actually add a hook");
    fixture
        .add_file("frontend/package.json", &tampered)
        .unwrap();
    let errors = validate(&evidence_document(), &fixture.root);
    let cleanup = fixture.cleanup();
    assert!(
        errors
            .iter()
            .any(|error| error.contains("actual manifest lifecycle hook: postinstall")),
        "real manifest hook was not detected: {errors:#?}"
    );
    assert!(cleanup.is_ok(), "owned fixture cleanup failed: {cleanup:?}");
}

#[test]
fn documentary_assertion_must_not_claim_supply_chain_proof() {
    expect_rejected("fabricated supply-chain assurance", |doc| {
        set(
            doc,
            &["notes", "dependency_assertion_scope"],
            Value::String("full supply-chain proof".into()),
        );
    });
    expect_rejected("manifest mismatch", |doc| {
        set(
            doc,
            &["observations", "manifest_crosscheck", "scripts"],
            serde_json::json!(["dev"]),
        );
    });
}

#[test]
fn historical_evidence_cannot_be_relabelled_as_current() {
    expect_rejected("applied to current main", |doc| {
        set(
            doc,
            &[
                "notes",
                "historical_vs_current",
                "evidence_applies_to_current_main",
            ],
            Value::Bool(true),
        );
    });
    expect_rejected("historical digest relabelled", |doc| {
        set(
            doc,
            &[
                "notes",
                "historical_vs_current",
                "artifact_digest_is_historical",
            ],
            Value::String("NO".into()),
        );
    });
    expect_rejected("current main mislabelled as candidate", |doc| {
        set(
            doc,
            &["notes", "historical_vs_current", "current_main_at_repair"],
            Value::String(CANDIDATE_SHA.into()),
        );
    });
}
