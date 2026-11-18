//! LIVE model-authored repair run (issue #136, Slice D).
//!
//! Drives the real governed path against an operator-configured
//! OpenAI-compatible HTTPS endpoint (default: OpenRouter) and seals the
//! evidence under `evidence/136-model-authored/`.
//!
//! This example does nothing and exits 0 unless explicitly enabled:
//!
//!   LITE_MODEL_REPAIR_LIVE=1 OPENROUTER_API_KEY=... cargo run --release --example model_repair_live
//!
//! Configuration (environment only; never code, never task input):
//!   LITE_MODEL_REPAIR_LIVE=1        arm the live run (required)
//!   OPENROUTER_API_KEY              provider credential (from the operator's .env)
//!   LITE_MODEL_REPAIR_MODEL         model slug (default: cohere/north-mini-code:free)
//!   LITE_MODEL_REPAIR_BASE_SHA      source repo commit to pin (default: 6feb9d4efd48f5437126dd6152843dc4b5cb1dfd)
//!   LITE_MODEL_REPAIR_EVIDENCE_DIR  output directory (default: evidence/136-model-authored/run-<pid>)
//!   LITE_MODEL_REPAIR_ENV_FILE      optional .env path for OPENROUTER_API_KEY (falls back to <manifest>/.env)
//!   LITE_MODEL_REPAIR_MAX_ATTEMPTS  attempt ceiling (default: 4)
//!   LITE_MODEL_REPAIR_TIMEOUT_SECS  per-validator budget (default: 1200)
//!   LITE_MODEL_REPAIR_REPOSITORY    source repository slug (default: prometheosai/prometheos-lite)
//!   LITE_MODEL_REPAIR_ISSUE_NUMBER  tracked issue identity (default: 136; 0 disables)
//!
//! Provenance of the task: Lite's own merged security fix e62ffaa
//! (PR #283, feat/154-slice-b-harness-selection) — a genuine HSEL-0008
//! forgery defect repaired in Lite's own history. The pinned pre-fix commit
//! is e62ffaa^ = 6feb9d4. The hidden acceptance test is the one added by
//! e62ffaa and is used ONLY by this example's post-run verification phase,
//! never by the model.
//!
//! No credential ever appears in the evidence: the redactor scrubs it from
//! rendered output before persistence; prompts contain only public source
//! and the issue statement.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde_json::json;
use sha2::Digest;

use prometheos_lite::workflow::execution_harness::{
    ExecutionHarness, HarnessDispatch, HarnessStep,
};
use prometheos_lite::workflow::governance_compiler;
use prometheos_lite::workflow::governance_permit::GovernancePermit;
use prometheos_lite::workflow::model_repair::{
    ModelDescriptor, ModelProposeError, ModelProposer, ModelRepairSpec, ModelRepairStatus,
    PromptContext, ProposedCompletion, SomaSourceBinding, ValidatorSpec, batch_of, report_digest,
    tree_digest,
};
use prometheos_lite::workflow::node_contracts::NodeManifestV1;
use prometheos_lite::workflow::node_runner::{CapabilityRegistry, NodeRunner};
use prometheos_lite::workflow::policy::LocalRestrictions;
use prometheos_lite::workflow::remote_service_harness::{
    REMOTE_SERVICE_HARNESS_ID, REMOTE_SERVICE_INFER_CAPABILITY, REMOTE_SERVICE_REVISION,
    RemoteServiceConfig, RemoteServiceHarness, remote_service_capability,
};

const TASK_ID: &str = "lite-hsel-decisionid-forgery-0001";
const REPAIR_TARGET: &str = "src/workflow/execution_harness.rs";
const BASE_SHA: &str = "6feb9d4efd48f5437126dd6152843dc4b5cb1dfd";
const HIDDEN_SHA: &str = "e62ffaa3bd585aba551a9e8e5584c6f834a21973";
const HIDDEN_TEST: &str = "semantic_verification_rejects_forged_fields";

const ISSUE_STATEMENT: &str = "Independent review (HSEL-0008 class) reports that harness-selection evidence recorded by HarnessSelectionV1 can be resealed after tampering and still pass semantic verification: the record is self-consistent as bytes, but what it claims does not correspond to the trusted selection inputs it is bound to. A forged selection must fail semantic verification with an HSEL-0008-classified error.\n\nRepair src/workflow/execution_harness.rs (HarnessSelectionV1::verify_against) so semantic verification fails closed for forged decision content. Do not change the trusted-input types, do not weaken any existing verification behavior, and keep the complete execution_harness_selection suite passing.\n\nThe validator is: cargo test --offline --test execution_harness_selection.\n\nProvenance: prometheosai/prometheos-lite, merged security fix e62ffaa (PR #283, feat/154-slice-b-harness-selection). This is a real historical defect in the repository you are editing, not a constructed fixture.";

fn load_env_file(path: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return out;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let mut value = value
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        value = value.trim().to_string();
        out.push((key.trim().to_string(), value));
    }
    out
}

fn env_or_dotenv(key: &str, dotenv: &[(String, String)]) -> Option<String> {
    std::env::var(key)
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| {
            dotenv
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
        })
}

/// Operator-supplied egress grant evidence for the model-dispatch egress,
/// read only from the operator's environment. All three components must be
/// present and non-empty, or the result is `None`; a partial set is treated
/// as absent (fail closed). Nothing is synthesized and no provider
/// identity is invented.
fn operator_egress_grant(
    dotenv: &[(String, String)],
) -> Option<prometheos_lite::workflow::model_repair::EgressGrantEvidence> {
    let receipt_sha256 = env_or_dotenv("LITE_EGRESS_GRANT_RECEIPT_SHA256", dotenv)?;
    let grant_ref = env_or_dotenv("LITE_EGRESS_GRANT_REF", dotenv)?;
    let authorized_at = env_or_dotenv("LITE_EGRESS_GRANT_AUTHORIZED_AT", dotenv)?;
    Some(
        prometheos_lite::workflow::model_repair::EgressGrantEvidence {
            receipt_sha256,
            grant_ref,
            authorized_at,
        },
    )
}

fn git(repo: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("git runs")?;
    if !out.status.success() {
        bail!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn stage_clone(src: &Path, dst: &Path, base_sha: &str) -> Result<()> {
    Command::new("git")
        .args([
            "-c",
            "core.autocrlf=false",
            "clone",
            "--no-local",
            "--no-hardlinks",
            src.to_str().unwrap(),
            dst.to_str().unwrap(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("git clone runs")?;
    git(dst, &["checkout", "-q", "--detach", base_sha])?;
    Ok(())
}

/// Hidden-acceptance evaluation, run only after the model is finished and
/// the report is sealed. Overlays the e62ffaa acceptance test on two fresh
/// clones: the pristine pinned one (must FAIL) and the model-repaired one
/// (must PASS). Never visible to the model.
fn hidden_acceptance(
    src_repo: &Path,
    base_sha: &str,
    hidden_sha: &str,
    hidden_test: &str,
    repair_diff: Option<&str>,
    work: &Path,
) -> Result<serde_json::Value> {
    let run = |label: &str, diff: Option<&str>| -> Result<serde_json::Value> {
        let dir = work.join(label);
        stage_clone(src_repo, &dir, base_sha)?;
        if let Some(diff) = diff {
            let mut child = Command::new("git")
                .args(["apply", "--whitespace=nowarn", "-"])
                .current_dir(&dir)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .context("git apply spawns")?;
            use std::io::Write;
            child
                .stdin
                .as_mut()
                .expect("stdin")
                .write_all(diff.as_bytes())?;
            let out = child.wait_with_output()?;
            if !out.status.success() {
                bail!("git apply failed: {}", String::from_utf8_lossy(&out.stderr));
            }
        }
        // Overlay the hidden acceptance test (verbatim from the fix commit).
        let hidden = git(
            src_repo,
            &[
                "show",
                &format!("{hidden_sha}:tests/execution_harness_selection.rs"),
            ],
        )?;
        let dest = dir.join("tests").join("execution_harness_selection.rs");
        std::fs::write(&dest, hidden).context("hidden test overlay writes")?;
        let out = Command::new("cargo")
            .args([
                "test",
                "--offline",
                "--test",
                "execution_harness_selection",
                hidden_test,
                "--",
                "--exact",
            ])
            .current_dir(&dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .context("cargo runs")?;
        Ok(json!({
            "overlay": label,
            "exitCode": out.status.code(),
            "stdoutSha256": format!("{:x}", sha2::Sha256::digest(&out.stdout)),
            "stderrSha256": format!("{:x}", sha2::Sha256::digest(&out.stderr)),
            "stdoutBytes": out.stdout.len(),
            "stderrBytes": out.stderr.len(),
            "passed": out.status.code() == Some(0),
        }))
    };
    let baseline = run("baseline", None)?;
    let repaired = run("repaired", repair_diff)?;
    Ok(json!({
        "hiddenTest": hidden_test,
        "hiddenSourceCommit": hidden_sha,
        "baselineOverlay": baseline,
        "repairedOverlay": repaired,
    }))
}

struct LiveProposer {
    runner: NodeRunner,
    manifest: NodeManifestV1,
    restrictions: LocalRestrictions,
    harness: RemoteServiceHarness,
    descriptor: ModelDescriptor,
    max_tokens: u64,
}

impl LiveProposer {
    fn build(
        config: RemoteServiceConfig,
        api_key: String,
        worktree: &Path,
        model: &str,
    ) -> Result<Self> {
        let mut registry = CapabilityRegistry::new();
        registry.declare(
            REMOTE_SERVICE_INFER_CAPABILITY,
            remote_service_capability(config, worktree.to_path_buf()),
        );
        // Reviewed permit for exactly this node and capability.
        let caps = [REMOTE_SERVICE_INFER_CAPABILITY];
        let mut authority_entries: Vec<String> = vec![
            "\"readable:repo://task\"".to_string(),
            "\"writable:work://y\"".to_string(),
        ];
        authority_entries.extend(caps.iter().map(|c| format!("{c:?}")));
        let authority_json = authority_entries.join(", ");
        let tools_json = caps
            .iter()
            .map(|c| format!("{c:?}: []"))
            .collect::<Vec<_>>()
            .join(", ");
        let unit = format!(
            r#"{{ "schemaVersion": "1.1.0", "version": "1.1.0", "id": "model-repair-live",
  "executionClass": "deterministic",
  "inputs": [{{"name": "goal", "type": "string", "acceptedOutcomes": ["Produced"]}}],
  "outputs": [{{"name": "result", "type": "string", "emits": ["Produced"]}}],
  "authority": [{authority_json}],
  "effects": [], "uses": [], "secrets": [], "context": [] }}"#
        );
        let workflow = format!(
            r#"{{ "schemaVersion": "1.1.0", "version": "1.1.0", "id": "lite-model-repair-live",
  "name": "LiveModelRepair", "kind": "atomic",
  "inputPorts": [{{"name": "goal", "direction": "input", "type": "string", "cardinality": "single", "requiredness": "required"}}],
  "outputPorts": [{{"name": "result", "direction": "output", "type": "string", "cardinality": "single", "requiredness": "required"}}],
  "body": [{unit}],
  "authority": {{
    "executionClass": "model-assisted", "mutation": "none", "tools": {{{tools_json}}},
    "readableScopes": ["repo://task"],
    "writableScopes": ["work://y"],
    "networkPolicy": {{"default": "deny"}}, "providerPolicy": {{"allowlist": []}},
    "secrets": []
  }} }}"#
        );
        let identity = governance_compiler::compile_workflow_text(&workflow)
            .map_err(|diags| anyhow::anyhow!("workflow compiles: {diags:?}"))?
            .canonicalization
            .sha256;
        let permit = GovernancePermit::issue(&workflow, &identity)
            .map_err(|diags| anyhow::anyhow!("permit issues: {diags:?}"))?;
        let runner = NodeRunner::new(registry, permit);
        let manifest = NodeManifestV1::parse_json(
            &json!({
                "schemaVersion": "1.0.0",
                "nodeId": "model-repair-live",
                "purpose": "governed live model-authored repair proposal",
                "inputs": [],
                "outputs": [{ "name": "out", "typeRef": "string" }],
                "readableScopes": ["repo://task"],
                "writableScopes": [],
                "retry": { "maxAttempts": 1, "retryableClasses": [] }
            })
            .to_string(),
        )
        .context("manifest parses")?;
        let restrictions = LocalRestrictions {
            readable_scopes: vec!["repo://task".into()],
            writable_scopes: vec![],
            token_budget_ceiling: None,
            denied_providers: vec![],
            forbidden_paths: vec![],
            max_attempts: 1,
            escalation_target: "human-review".into(),
        };
        let _ = api_key;
        Ok(Self {
            runner,
            manifest,
            restrictions,
            harness: RemoteServiceHarness::new(),
            descriptor: ModelDescriptor {
                provider: "openrouter".to_string(),
                model: model.to_string(),
                harness_id: REMOTE_SERVICE_HARNESS_ID.to_string(),
                harness_revision: REMOTE_SERVICE_REVISION.to_string(),
            },
            max_tokens: 4096,
        })
    }
}

impl ModelProposer for LiveProposer {
    fn descriptor(&self) -> ModelDescriptor {
        self.descriptor.clone()
    }

    fn propose(&mut self, prompt: &str) -> Result<ProposedCompletion, ModelProposeError> {
        let model = self.descriptor.model.clone();
        let step = HarnessStep {
            step_id: "model-repair-live-step".to_string(),
            capability: REMOTE_SERVICE_INFER_CAPABILITY.to_string(),
            args: json!({
                "model": model,
                "messages": [
                    { "role": "system", "content": "You are a precise coding agent. Output exactly one fenced diff block and nothing else." },
                    { "role": "user", "content": prompt }
                ],
                "maxTokens": self.max_tokens,
            }),
        };
        // The ModelProposer trait is synchronous; this example's main runs
        // on a tokio runtime. A block_on on this thread would panic with
        // "Cannot start a runtime from within a runtime", so drive the
        // governed dispatch on a dedicated OS thread with its own
        // current-thread runtime.
        let harness = &self.harness;
        let manifest = &self.manifest;
        let restrictions = &self.restrictions;
        let runner = &mut self.runner;
        let joined = std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|e| {
                            ModelProposeError::Unavailable(format!("tokio runtime: {e}"))
                        })?;
                    let mut dispatch =
                        HarnessDispatch::new(harness, manifest, restrictions, vec![], runner);
                    runtime
                        .block_on(harness.run(step, &mut dispatch))
                        .map_err(|e| {
                            ModelProposeError::Unavailable(format!(
                                "governed dispatch failed: {e:#}"
                            ))
                        })
                })
                .join()
        });
        let outcome = joined.map_err(|_| {
            ModelProposeError::Unavailable("dispatch thread panicked".to_string())
        })??;
        let output = outcome.output;
        let parsed: serde_json::Value = serde_json::from_str(&output).map_err(|e| {
            ModelProposeError::Unavailable(format!(
                "harness output is not the expected envelope: {e}"
            ))
        })?;
        let success = parsed.get("success").and_then(serde_json::Value::as_bool) == Some(true);
        if !success {
            let status = parsed
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let message = parsed
                .get("output")
                .and_then(|o| o.get("error"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unclassified provider failure")
                .to_string();
            return Err(match status.as_str() {
                s if s.contains("budget") => ModelProposeError::Budget(message),
                "refused" | "refusal" => ModelProposeError::Refused(message),
                _ => ModelProposeError::Unavailable(format!("{status}: {message}")),
            });
        }
        let inner = parsed
            .get("output")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let content = inner
            .get("content")
            .map(|c| match c {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .ok_or_else(|| {
                ModelProposeError::Unavailable("harness output lacks content".to_string())
            })?;
        // Responder-issued response id (e.g. OpenAI-compatible `id`), captured
        // at the response boundary by the harness. Absent when the responder
        // was silent — never synthesized, never substituted by a hash.
        let provider_response_id = inner
            .get("responseId")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.chars().take(512).collect::<String>());
        let provider_response_id_source = provider_response_id
            .as_ref()
            .map(|_| "response-body:id via lite.harness.remote_service".to_string());
        let local_request_id = inner
            .get("requestId")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        // The remote-service harness does not surface response headers, so a
        // gateway trace id cannot be captured here; recorded absent rather
        // than inferred.
        Ok(ProposedCompletion {
            content,
            provider_response_id,
            provider_response_id_source,
            gateway_trace_id: None,
            local_request_id,
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let live = std::env::var("LITE_MODEL_REPAIR_LIVE").ok().as_deref() == Some("1");
    if !live {
        println!(
            "SKIP: set LITE_MODEL_REPAIR_LIVE=1 to run the live pilot (uses the operator-configured OpenRouter endpoint)."
        );
        return Ok(());
    }
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let env_file = std::env::var("LITE_MODEL_REPAIR_ENV_FILE")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(manifest_dir).join(".env"));
    let dotenv = load_env_file(&env_file);
    let api_key = env_or_dotenv("OPENROUTER_API_KEY", &dotenv)
        .context("OPENROUTER_API_KEY not set in environment or .env")?;
    let model = std::env::var("LITE_MODEL_REPAIR_MODEL")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "cohere/north-mini-code:free".to_string());
    let base_sha = std::env::var("LITE_MODEL_REPAIR_BASE_SHA")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| BASE_SHA.to_string());
    let evidence_dir = std::env::var("LITE_MODEL_REPAIR_EVIDENCE_DIR")
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(manifest_dir).join("evidence").join(TASK_ID));
    let max_attempts: u32 = std::env::var("LITE_MODEL_REPAIR_MAX_ATTEMPTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4);
    let validator_timeout = std::time::Duration::from_secs(
        std::env::var("LITE_MODEL_REPAIR_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1200),
    );
    // SOMA source binding. The staged tree is a clone of this repository at
    // the pinned commit, so the GitHub slug is recorded — configured by the
    // operator, never derived from the model. The default issue number is
    // the tracked milestone for this producer path (#136); operators may
    // override or set 0 to record an explicitly empty binding.
    let repository = std::env::var("LITE_MODEL_REPAIR_REPOSITORY")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "prometheosai/prometheos-lite".to_string());
    let issue_number = match std::env::var("LITE_MODEL_REPAIR_ISSUE_NUMBER") {
        Ok(v) => match v.parse::<u64>() {
            Ok(0) => None,
            Ok(n) => Some(n),
            Err(_) => {
                eprintln!(
                    "warning: LITE_MODEL_REPAIR_ISSUE_NUMBER={v:?} is not a number; recording no issue binding"
                );
                None
            }
        },
        Err(_) => Some(136),
    };

    let work = tempfile::tempdir().context("tempdir")?;
    let worktree = work.path().join("pinned-src");
    println!("staging {TASK_ID} at {base_sha} ...");
    stage_clone(Path::new(manifest_dir), &worktree, &base_sha)?;
    let head = git(&worktree, &["rev-parse", "HEAD"])?;
    anyhow::ensure!(
        head == base_sha,
        "staged head {head} != pinned base {base_sha}"
    );
    let target_path = worktree.join(REPAIR_TARGET);
    let target_bytes = std::fs::read(&target_path).context("target readable")?;
    let target_sha = format!("{:x}", sha2::Sha256::digest(&target_bytes));
    let fingerprint = tree_digest(&worktree).map_err(|e| anyhow::anyhow!("tree digests: {e}"))?;
    // The pinned file exceeds the provider wire budget; extract the bounded
    // region around the defect (HarnessSelectionV1 and verify_against) from
    // the staged tree itself — never an embedded copy.
    let full_text = String::from_utf8(target_bytes).context("target is utf-8")?;
    let all_lines: Vec<&str> = full_text.lines().collect();
    let excerpt_start: usize = 615; // 1-based line 616
    let excerpt_end: usize = 851; // 1-based line 851 (end of impl HarnessSelectionV1)
    anyhow::ensure!(
        excerpt_end <= all_lines.len(),
        "excerpt range exceeds file length"
    );
    let excerpt = all_lines[excerpt_start..excerpt_end].join("\n");

    let config = RemoteServiceConfig::new("https://openrouter.ai/api/v1/chat/completions", &model)
        .allowed_hosts(vec!["openrouter.ai".to_string()])
        .api_key(api_key.clone())
        .timeout_ms(120_000)
        .max_output_tokens(4096)
        .max_output_bytes(256 * 1024)
        .known_secrets(vec![api_key]);
    let proposer = LiveProposer::build(
        config,
        env_or_dotenv("OPENROUTER_API_KEY", &dotenv).unwrap_or_default(),
        &worktree,
        &model,
    )?;

    let spec = ModelRepairSpec {
        task_id: TASK_ID.to_string(),
        repair_target: REPAIR_TARGET.to_string(),
        worktree: worktree.clone(),
        repair_target_sha256: target_sha,
        workspace_fingerprint: fingerprint,
        base_revision: base_sha.clone(),
        issue_statement: ISSUE_STATEMENT.to_string(),
        validator: ValidatorSpec {
            command: "cargo".to_string(),
            args: vec![
                "test".to_string(),
                "--offline".to_string(),
                "--test".to_string(),
                "execution_harness_selection".to_string(),
            ],
            expected_exit_code: 0,
            assertion: "execution_harness_selection suite passes at the pinned base tree"
                .to_string(),
        },
        validator_timeout,
        max_attempts,
        validator_target_dir: Some(worktree.join("target")),
        prompt_context: Some(PromptContext {
            text: excerpt,
            start_line: (excerpt_start + 1) as u64,
            note: format!(
                "excerpt of lines {}-{} of src/workflow/execution_harness.rs",
                excerpt_start + 1,
                excerpt_end
            ),
        }),
        source: Some(SomaSourceBinding {
            repository,
            repository_revision: base_sha.clone(),
            issue_number,
        }),
        // Operator-supplied egress grant evidence, read only from the
        // operator's environment. Any absent component leaves this `None`;
        // the strict path then reports the grant as unverified and the run
        // is never strictly eligible. Nothing here is synthesized, and an
        // environment variable alone never implies authorization.
        egress_grant: operator_egress_grant(&dotenv),
    };

    println!("dispatching governed model ({model}) ...");
    let report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    println!("status: {:?}", report.status);
    println!("attempts: {}", report.attempts.len());

    std::fs::create_dir_all(&evidence_dir)
        .with_context(|| format!("mkdir {}", evidence_dir.display()))?;
    std::fs::write(
        evidence_dir.join("report.json"),
        serde_json::to_vec_pretty(&report).context("report serializes")?,
    )?;
    let batch = batch_of(&report);
    std::fs::write(
        evidence_dir.join("work-events.json"),
        serde_json::to_vec_pretty(&batch).context("batch serializes")?,
    )?;
    if let Some(diff) = &report.final_diff_excerpt {
        std::fs::write(evidence_dir.join("final.diff"), diff)?;
    }

    // Hidden acceptance: only now, never shown to the model.
    let hidden = hidden_acceptance(
        Path::new(manifest_dir),
        &base_sha,
        HIDDEN_SHA,
        HIDDEN_TEST,
        if report.status == ModelRepairStatus::Repaired {
            report.final_diff_excerpt.clone()
        } else {
            None
        }
        .as_deref(),
        &work.path().join("overlay-check"),
    )?;
    std::fs::write(
        evidence_dir.join("acceptance.json"),
        serde_json::to_vec_pretty(&hidden).context("acceptance serializes")?,
    )?;

    let summary = json!({
        "taskId": TASK_ID,
        "baseRevision": base_sha,
        "hiddenCommit": HIDDEN_SHA,
        "reportDigest": report_digest(&report),
        "status": format!("{:?}", report.status),
        "attempts": report.attempts.len(),
        "eventsSha256": report.events_sha256,
        "promptBytesTotal": report.prompt_bytes_total,
        "wallMs": report.wall_ms,
    });
    std::fs::write(
        evidence_dir.join("summary.json"),
        serde_json::to_vec_pretty(&summary).context("summary serializes")?,
    )?;
    println!("evidence written to {}", evidence_dir.display());
    println!("acceptance: {}", hidden);
    Ok(())
}
