//! Governed remote-service harness (issue #154, Slice I) — integration tests.
//!
//! Every effectful test drives the REAL governed path:
//! `ExecutionHarness (RemoteServiceHarness) -> HarnessDispatch -> NodeRunner ->
//! governed capability (remote_service_capability)`, over a REAL TLS
//! fixture (`tests/fixtures/remote_service_fixture.py`, a separate Python
//! process serving HTTPS on loopback). No in-memory mock replay is used as
//! proof of transport. The fixture proves the adapter's HTTPS/TLS transport
//! behavior only; it is not a real remote provider and proves nothing about
//! live provider interoperability.
//!
//! Governed failures surface as `Err` with a typed `(status <code>)`
//! classification in the message — the same boundary contract the governed
//! path uses for every harness family; [`status_of`] extracts it.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Result;
use serde_json::{Value, json};

use prometheos_lite::workflow::execution_harness::{
    ExecutionHarness, HarnessDispatch, HarnessFamily, HarnessOutcome, HarnessRegistry, HarnessStep,
    canonical_harness_declarations,
};
use prometheos_lite::workflow::node_contracts::NodeManifestV1;
use prometheos_lite::workflow::node_runner::{CapabilityRegistry, NodeRunner};
use prometheos_lite::workflow::policy::LocalRestrictions;
use prometheos_lite::workflow::redaction::REDACTED_PLACEHOLDER;
use prometheos_lite::workflow::remote_service_harness::{
    REMOTE_SERVICE_CAPS_ID, REMOTE_SERVICE_HARNESS_ID, REMOTE_SERVICE_INFER_CAPABILITY,
    RemoteServiceConfig, RemoteServiceHarness, remote_service_capability,
    remote_service_declaration,
};
use prometheos_lite::workflow::soma::capability::Locality;

mod common;

const NODE_ID: &str = "remote-service-step";
const MODEL: &str = "tiny-remote-model";
const API_KEY: &str = "fixture-secret-key-12345";

fn manifest() -> NodeManifestV1 {
    NodeManifestV1::parse_json(
        &json!({
            "schemaVersion": "1.0.0",
            "nodeId": NODE_ID,
            "purpose": "governed remote service fixture",
            "inputs": [],
            "outputs": [{"name": "out", "typeRef": "string"}],
            "readableScopes": ["repo://x"],
            "writableScopes": ["work://y"],
            "retry": {"maxAttempts": 2, "retryableClasses": ["infra"]}
        })
        .to_string(),
    )
    .expect("manifest parses")
}

fn restrictions() -> LocalRestrictions {
    LocalRestrictions {
        readable_scopes: vec!["repo://x".into()],
        writable_scopes: vec!["work://y".into()],
        token_budget_ceiling: None,
        denied_providers: vec![],
        forbidden_paths: vec![],
        max_attempts: 3,
        escalation_target: "human-review".into(),
    }
}

fn runner_for(
    config: RemoteServiceConfig,
    workspace_root: std::path::PathBuf,
    permit_caps: &[&str],
) -> NodeRunner {
    let mut registry = CapabilityRegistry::new();
    registry.declare(
        REMOTE_SERVICE_INFER_CAPABILITY,
        remote_service_capability(config, workspace_root),
    );
    NodeRunner::new(registry, common::permit_for(&[NODE_ID], permit_caps))
}

fn harness() -> RemoteServiceHarness {
    RemoteServiceHarness::new()
}

fn dispatch<'a>(
    harness: &'a dyn ExecutionHarness,
    m: &'a NodeManifestV1,
    r: &'a LocalRestrictions,
    runner: &'a mut NodeRunner,
) -> HarnessDispatch<'a> {
    HarnessDispatch::new(harness, m, r, vec![], runner)
}

fn dispatch_with_secrets<'a>(
    harness: &'a dyn ExecutionHarness,
    m: &'a NodeManifestV1,
    r: &'a LocalRestrictions,
    runner: &'a mut NodeRunner,
    secrets: Vec<String>,
) -> HarnessDispatch<'a> {
    HarnessDispatch::new(harness, m, r, secrets, runner)
}

fn step(args: Value) -> HarnessStep {
    HarnessStep {
        step_id: "step-1".to_string(),
        capability: REMOTE_SERVICE_INFER_CAPABILITY.to_string(),
        args,
    }
}

fn inference_args(endpoint: &str) -> Value {
    json!({
        "model": MODEL,
        "endpoint": endpoint,
        "messages": [{"role": "user", "content": "say hello"}],
        "maxTokens": 16,
    })
}

/// The fixture CA certificate (matches the embedded CA in the Python
/// fixture). Used as an additional trust anchor; never written to the repo
/// beyond this test module.
const FIXTURE_CA_PEM: &[u8] = b"-----BEGIN CERTIFICATE-----\n\
MIIDKzCCAhOgAwIBAgIUBzrrNAfQkeFQRdobx9aOAebXloswDQYJKoZIhvcNAQEL\n\
BQAwJTEjMCEGA1UEAwwaUHJvbWV0aGVPUyBTbGljZS1JIFRlc3QgQ0EwHhcNMjYx\n\
MTEyMTUyMjE1WhcNMzYxMTA5MTUyMjE1WjAlMSMwIQYDVQQDDBpQcm9tZXRoZU9T\n\
IFNsaWNlLUkgVGVzdCBDQTCCASIwDQYJKoZIhvcNAQEBBQADggEPADCCAQoCggEB\n\
ANM9FIxh9AHOhVIG6fNR40b8+3yrr3uN9xNPckwDPZdGjsW2U+/pil9dSSss5ru7\n\
1gNwVedTSPIIellJlarKW1jOoCjIyXhKQy/TGYx74VTqIJLg8YQfMHDKQdWSn+bX\n\
4eCTDDyAZ7Wu3FBE2NzNEeLs4iohdiD8PY4vG78wKkN624g9ZlS3CwhfPbNoGZVD\n\
ZWRJLHCyRS+IWNuKkCZBtQI+JUE+etgg1xbQE7jpSEjpLECrSgpkCQwG3MqYZewV\n\
ajyr8Zw4GMWGQoWhW9q+bELRYvjKkiwGXeZ/8WEip79DVZUwR7Pz5yh+CAZvV05/\n\
YmaOdCGVF0wvCx+OhDJKfOsCAwEAAaNTMFEwHQYDVR0OBBYEFDVh5WIUAti/OILo\n\
UdH6djF2VY8UMB8GA1UdIwQYMBaAFDVh5WIUAti/OILoUdH6djF2VY8UMA8GA1Ud\n\
EwEB/wQFMAMBAf8wDQYJKoZIhvcNAQELBQADggEBAL0DL6pINa3CE7gVYCkqfVwR\n\
mWJT9hUWEHlNaGso+3dMX6oYp1WriG0xDejn8oGTZ1hKhZ4KER8oRSZU+IK+Wjy2\n\
K7WXESPowxCbtbYNOXCfjMmubAQt9FtdpUAy5YutxjqzpDVyCRbFWgEFfJh7QQKP\n\
71+0UDkf8V7TPP+mw6W4vUAf52lNbjDKvK+pn9QErMxcNql8LpNcTMS3jbaP7gZJ\n\
eBpkW7NcRGVHmoeeqZrpERhfJn5mtAKPVTzrk2UZQ7MOTPdO23jyCTY6DjD24OPn\n\
nSdKs47cVOd7jkTF/a9wDixA3r1EBb1GfmfqFuwQrBaZVUJqIAJVkbWaw0TL3GU=\n\
-----END CERTIFICATE-----\n";

fn fixture_python() -> &'static str {
    if Command::new("python3").arg("--version").output().is_ok() {
        return "python3";
    }
    if Command::new("python").arg("--version").output().is_ok() {
        return "python";
    }
    panic!("remote_service_fixture requires python3 (or python) on PATH");
}

struct FixtureGuard {
    endpoint: String,
    base_host_port: String,
    count_file: PathBuf,
    request_log: PathBuf,
    child: Child,
    _tmp: tempfile::TempDir,
}

impl FixtureGuard {
    fn hit_count(&self) -> usize {
        for _ in 0..20 {
            if let Ok(contents) = std::fs::read_to_string(&self.count_file) {
                return contents.lines().count();
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        0
    }

    /// Parse the last logged request body (the wire payload), if any.
    fn last_request(&self) -> Option<Value> {
        let contents = std::fs::read_to_string(&self.request_log).ok()?;
        let line = contents.lines().last()?.trim();
        if line.is_empty() {
            return None;
        }
        serde_json::from_str(line).ok()
    }
}

impl Drop for FixtureGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Spawn the HTTPS fixture in its selected mode; block until it prints
/// `PORT=<n>`. `cert` selects the served certificate kind ("server" default
/// or "wronghost").
fn spawn_fixture(mode: &str, cert: Option<&str>, expect_auth: Option<&str>) -> FixtureGuard {
    let tmp = tempfile::tempdir().expect("fixture tempdir");
    let count_file = tmp.path().join("hits");
    let request_log = tmp.path().join("requests");
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/remote_service_fixture.py"
    );
    let mut cmd = Command::new(fixture_python());
    cmd.arg(script)
        .env("REMOTE_SERVICE_FIXTURE_MODE", mode)
        .env("REMOTE_SERVICE_FIXTURE_COUNT_FILE", &count_file)
        .env("REMOTE_SERVICE_FIXTURE_REQUEST_LOG", &request_log)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(cert) = cert {
        cmd.env("REMOTE_SERVICE_FIXTURE_CERT", cert);
    }
    if let Some(expect_auth) = expect_auth {
        cmd.env("REMOTE_SERVICE_FIXTURE_EXPECT_AUTH", expect_auth);
    }
    cmd.env("REMOTE_SERVICE_FIXTURE_API_KEY", API_KEY);
    let mut child = cmd.spawn().expect("HTTPS fixture spawns");
    let stdout = child.stdout.take().expect("fixture stdout piped");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .expect("fixture prints PORT line");
    let port: u16 = line
        .trim()
        .strip_prefix("PORT=")
        .and_then(|p| p.parse().ok())
        .expect("fixture PORT line is well-formed");
    let host_port = format!("localhost:{port}");
    FixtureGuard {
        endpoint: format!("https://{host_port}/v1"),
        base_host_port: host_port,
        count_file,
        request_log,
        child,
        _tmp: tmp,
    }
}

fn config_for_fixture(fixture: &FixtureGuard, model: &str) -> RemoteServiceConfig {
    RemoteServiceConfig::new(fixture.endpoint.clone(), model)
        .allowed_hosts(vec![fixture.base_host_port.clone()])
        .api_key(API_KEY)
        .allow_private_destinations(true)
        .extra_root_certs_pem(vec![FIXTURE_CA_PEM.to_vec()])
}

/// Extract the typed `(status <code>)` marker from a governed refusal error.
fn status_of(err: &anyhow::Error) -> String {
    let text = format!("{err:#}");
    let marker = "(status ";
    let start = text
        .find(marker)
        .map(|i| i + marker.len())
        .unwrap_or_else(|| panic!("error carries no typed status: {text}"));
    let end = text[start..]
        .find(')')
        .map(|i| start + i)
        .unwrap_or_else(|| panic!("typed status is not parenthesized: {text}"));
    text[start..end].to_string()
}

/// Drive one governed invocation against the fixture. The fixture's hit file
/// is independent; tests assert it directly.
async fn run_governed_inference(
    config: RemoteServiceConfig,
    workspace_root: PathBuf,
    secrets: Vec<String>,
) -> Result<HarnessOutcome> {
    let config_endpoint = config.endpoint.clone();
    let mut runner = runner_for(config, workspace_root, &[REMOTE_SERVICE_INFER_CAPABILITY]);
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch_with_secrets(&harness, &m, &r, &mut runner, secrets);
    harness
        .run(step(inference_args(&config_endpoint)), &mut d)
        .await
}

fn fixture_with_mode(mode: &str) -> (FixtureGuard, RemoteServiceConfig) {
    let fixture = spawn_fixture(mode, None, None);
    let config = config_for_fixture(&fixture, MODEL);
    (fixture, config)
}

// ---------------------------------------------------------------------------
// Declaration / registry / catalog
// ---------------------------------------------------------------------------

#[tokio::test]
async fn remote_service_declaration_is_production_grade() {
    let declaration = remote_service_declaration();
    assert_eq!(declaration.harness_id, REMOTE_SERVICE_HARNESS_ID);
    assert_eq!(declaration.family, HarnessFamily::RemoteService);
    assert_eq!(
        declaration.claimed_capabilities,
        vec![REMOTE_SERVICE_INFER_CAPABILITY]
    );
    assert_eq!(declaration.capabilities.locality, Some(Locality::Remote));
    assert_eq!(
        declaration.capabilities.effect_classes,
        vec!["network-io".to_string()]
    );
    assert_eq!(REMOTE_SERVICE_CAPS_ID, "lite.harness.remote_service.caps");

    let mut registry = HarnessRegistry::new();
    registry
        .register(Box::new(RemoteServiceHarness::new()))
        .expect("remote service declaration registers fail-closed");
    let registered = registry
        .get(REMOTE_SERVICE_HARNESS_ID)
        .expect("remote service harness is registered");
    assert_eq!(
        registered.declaration().harness_id,
        REMOTE_SERVICE_HARNESS_ID
    );
    let catalog = registry.catalog();
    let entry = catalog
        .entries
        .iter()
        .find(|e| e.harness_id == REMOTE_SERVICE_HARNESS_ID)
        .expect("remote service harness appears in the generated catalog");
    assert_eq!(entry.family, HarnessFamily::RemoteService);
    assert_eq!(entry.effect_classes, vec!["network-io".to_string()]);
    assert_eq!(entry.locality, Some(Locality::Remote));

    // The implementation-owned declaration is part of the production
    // inventory used by the catalog/drift gate.
    let inventory = canonical_harness_declarations();
    assert!(
        inventory
            .iter()
            .any(|d| d.harness_id == REMOTE_SERVICE_HARNESS_ID)
    );
}

// ---------------------------------------------------------------------------
// Positive: portable checkpoint round trip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn remote_service_checkpoint_resume_round_trip() -> Result<()> {
    let harness = RemoteServiceHarness::new();
    let outcome = HarnessOutcome {
        harness_id: REMOTE_SERVICE_HARNESS_ID.to_string(),
        output: "{\"success\":true,\"status\":\"ok\"}".to_string(),
        evidence_entry_digest: "0".repeat(64),
        events: vec![],
        content_digest: None,
    }
    .sealed();
    let portable_state_digest = "a".repeat(64);
    let checkpoint = harness.checkpoint(&outcome, &portable_state_digest).await?;
    assert_eq!(checkpoint.harness_id, REMOTE_SERVICE_HARNESS_ID);
    assert_eq!(checkpoint.harness_revision, "v1");
    let continuation = harness.resume(&checkpoint, &portable_state_digest).await?;
    assert_eq!(continuation.harness_id, REMOTE_SERVICE_HARNESS_ID);
    assert_eq!(continuation.portable_state_digest, portable_state_digest);
    let wrong = harness.resume(&checkpoint, &"b".repeat(64)).await;
    assert!(wrong.is_err());
    assert!(harness.resume(&checkpoint, &"a".repeat(64)).await.is_ok());
    Ok(())
}

// ---------------------------------------------------------------------------
// Positive: real TLS wire round trip against the HTTPS fixture
// ---------------------------------------------------------------------------

#[tokio::test]
async fn remote_service_https_round_trip() -> Result<()> {
    let (fixture, config) = fixture_with_mode("ok");
    let root = tempfile::tempdir().expect("workspace tempdir");
    let outcome = run_governed_inference(config, root.path().to_path_buf(), vec![]).await?;
    let output: Value = serde_json::from_str(&outcome.output)?;
    assert_eq!(output["success"], json!(true));
    assert_eq!(output["output"]["content"]["model"], json!(MODEL));
    assert_eq!(
        output["output"]["content"]["content"],
        json!("fixture response")
    );
    assert_eq!(
        output["output"]["content"]["usage"]["promptTokens"],
        json!(11),
        "usage tokens must be the provider-supplied values"
    );
    assert_eq!(
        output["output"]["content"]["usage"]["totalTokens"],
        json!(18)
    );
    assert!(output["output"]["latencyMs"].as_u64().is_some());
    assert_eq!(fixture.hit_count(), 1, "fixture must be hit exactly once");
    Ok(())
}

#[tokio::test]
async fn remote_service_usage_absent_is_reported_unknown() -> Result<()> {
    let (fixture, config) = fixture_with_mode("ok-no-usage");
    let root = tempfile::tempdir().expect("workspace tempdir");
    let outcome = run_governed_inference(config, root.path().to_path_buf(), vec![]).await?;
    let output: Value = serde_json::from_str(&outcome.output)?;
    assert_eq!(
        output["output"]["content"]["usage"]["promptTokens"],
        Value::Null,
        "absent usage must be unknown (null), never invented"
    );
    assert_eq!(
        output["output"]["content"]["usage"]["totalTokens"],
        Value::Null
    );
    assert_eq!(fixture.hit_count(), 1);
    Ok(())
}

#[tokio::test]
async fn remote_service_bearer_credential_is_transmitted() -> Result<()> {
    let fixture = spawn_fixture("require-auth", None, Some(&format!("Bearer {API_KEY}")));
    let config = config_for_fixture(&fixture, MODEL).api_key(API_KEY);
    let root = tempfile::tempdir().expect("workspace tempdir");
    let outcome = run_governed_inference(config, root.path().to_path_buf(), vec![]).await?;
    let output: Value = serde_json::from_str(&outcome.output)?;
    assert_eq!(output["success"], json!(true));
    Ok(())
}

#[tokio::test]
async fn remote_service_wrong_credential_is_rejected_by_provider() -> Result<()> {
    let fixture = spawn_fixture("require-auth", None, Some("Bearer the-right-key"));
    let config = config_for_fixture(&fixture, MODEL).api_key("the-wrong-key");
    let root = tempfile::tempdir().expect("workspace tempdir");
    let err = run_governed_inference(config, root.path().to_path_buf(), vec![])
        .await
        .expect_err("wrong provider credential must be rejected");
    assert_eq!(status_of(&err), "provider-error");
    Ok(())
}

// ---------------------------------------------------------------------------
// TLS verification
// ---------------------------------------------------------------------------

#[tokio::test]
async fn remote_service_untrusted_ca_is_refused() {
    let fixture = spawn_fixture("ok", None, None);
    // The client does NOT trust the fixture CA here (no extra trust anchor):
    // TLS verification must fail against the system roots.
    let config = RemoteServiceConfig::new(fixture.endpoint.clone(), MODEL)
        .allowed_hosts(vec![fixture.base_host_port.clone()])
        .api_key(API_KEY)
        .allow_private_destinations(true);
    let root = tempfile::tempdir().expect("workspace tempdir");
    let err = run_governed_inference(config, root.path().to_path_buf(), vec![])
        .await
        .expect_err("untrusted CA must be refused");
    assert_eq!(status_of(&err), "transport-error");
}

#[tokio::test]
async fn remote_service_wrong_cert_name_is_refused() {
    let fixture = spawn_fixture("ok", Some("wronghost"), None);
    let config = config_for_fixture(&fixture, MODEL);
    let root = tempfile::tempdir().expect("workspace tempdir");
    let err = run_governed_inference(config, root.path().to_path_buf(), vec![])
        .await
        .expect_err("certificate hostname mismatch must be refused");
    assert_eq!(status_of(&err), "transport-error");
}

// ---------------------------------------------------------------------------
// Redirects / proxies
// ---------------------------------------------------------------------------

#[tokio::test]
async fn remote_service_redirect_is_never_followed() -> Result<()> {
    let (fixture, config) = fixture_with_mode("redirect");
    let root = tempfile::tempdir().expect("workspace tempdir");
    let err = run_governed_inference(config, root.path().to_path_buf(), vec![])
        .await
        .expect_err("redirects must be refused, never followed");
    assert_eq!(status_of(&err), "provider-error");
    assert_eq!(fixture.hit_count(), 1, "redirect target must never be hit");
    Ok(())
}

#[tokio::test]
async fn remote_service_proxy_environment_is_not_inherited() -> Result<()> {
    let (fixture, config) = fixture_with_mode("ok");
    // Point every proxy variable at an unreachable address; the adapter
    // builds its client with no_proxy(), so the fixture must still be hit.
    // SAFETY: environment mutation is process-global; this test sets and
    // removes the variables itself, and the adapter ignores proxies anyway.
    unsafe {
        std::env::set_var("HTTPS_PROXY", "http://127.0.0.1:9");
        std::env::set_var("HTTP_PROXY", "http://127.0.0.1:9");
        std::env::set_var("ALL_PROXY", "http://127.0.0.1:9");
    }
    let root = tempfile::tempdir().expect("workspace tempdir");
    let result = run_governed_inference(config, root.path().to_path_buf(), vec![]).await;
    unsafe {
        std::env::remove_var("HTTPS_PROXY");
        std::env::remove_var("HTTP_PROXY");
        std::env::remove_var("ALL_PROXY");
    }
    let outcome = result?;
    let output: Value = serde_json::from_str(&outcome.output)?;
    assert_eq!(
        output["success"],
        json!(true),
        "env proxies must not divert the request"
    );
    assert_eq!(fixture.hit_count(), 1);
    Ok(())
}

// ---------------------------------------------------------------------------
// Destination policy (SSRF) — refusal before any network effect
// ---------------------------------------------------------------------------

#[tokio::test]
async fn remote_service_refuses_private_destination_by_default() {
    let fixture = spawn_fixture("ok", None, None);
    let config = RemoteServiceConfig::new(fixture.endpoint.clone(), MODEL)
        .allowed_hosts(vec![fixture.base_host_port.clone()])
        .api_key(API_KEY); // allow_private_destinations defaults to false
    let root = tempfile::tempdir().expect("workspace tempdir");
    let err = run_governed_inference(config, root.path().to_path_buf(), vec![])
        .await
        .expect_err("private destination must be refused by the default policy");
    assert_eq!(status_of(&err), "destination-refused");
    assert_eq!(fixture.hit_count(), 0, "no network effect may occur");
}

#[tokio::test]
async fn remote_service_refuses_metadata_destination() {
    let config = RemoteServiceConfig::new("https://169.254.169.254/latest/meta-data", MODEL)
        .allowed_hosts(vec!["169.254.169.254".to_string()])
        .api_key(API_KEY);
    let root = tempfile::tempdir().expect("workspace tempdir");
    let err = run_governed_inference(config, root.path().to_path_buf(), vec![])
        .await
        .expect_err("metadata destination must be refused");
    assert_eq!(status_of(&err), "destination-refused");
}

#[tokio::test]
async fn remote_service_refuses_metadata_destination_even_with_private_policy() {
    let config = RemoteServiceConfig::new("https://169.254.169.254/latest/meta-data", MODEL)
        .allowed_hosts(vec!["169.254.169.254".to_string()])
        .api_key(API_KEY)
        .allow_private_destinations(true);
    let root = tempfile::tempdir().expect("workspace tempdir");
    let err = run_governed_inference(config, root.path().to_path_buf(), vec![])
        .await
        .expect_err(
            "metadata destination must be refused even under the private-destination policy",
        );
    assert_eq!(status_of(&err), "destination-refused");
}

#[tokio::test]
async fn remote_service_refuses_ipv6_metadata_destination() {
    let config = RemoteServiceConfig::new("https://[fd00:ec2::254]/latest/meta-data", MODEL)
        .allowed_hosts(vec!["[fd00:ec2::254]".to_string()])
        .api_key(API_KEY)
        .allow_private_destinations(true);
    let root = tempfile::tempdir().expect("workspace tempdir");
    let err = run_governed_inference(config, root.path().to_path_buf(), vec![])
        .await
        .expect_err("IPv6 metadata destination must be refused");
    assert_eq!(status_of(&err), "destination-refused");
}

#[tokio::test]
async fn remote_service_refuses_unsupported_scheme() {
    let fixture = spawn_fixture("ok", None, None);
    let config = RemoteServiceConfig::new(fixture.endpoint.replace("https://", "http://"), MODEL)
        .allowed_hosts(vec![fixture.base_host_port.clone()])
        .api_key(API_KEY)
        .allow_private_destinations(true);
    let root = tempfile::tempdir().expect("workspace tempdir");
    let err = run_governed_inference(config, root.path().to_path_buf(), vec![])
        .await
        .expect_err("plain http must be refused");
    assert_eq!(status_of(&err), "unsupported-scheme");
    assert_eq!(fixture.hit_count(), 0);
}

#[tokio::test]
async fn remote_service_refuses_credentials_in_url() {
    let fixture = spawn_fixture("ok", None, None);
    let config = RemoteServiceConfig::new(
        format!("https://user:pass@{}/v1", fixture.base_host_port),
        MODEL,
    )
    .allowed_hosts(vec![fixture.base_host_port.clone()])
    .api_key(API_KEY)
    .allow_private_destinations(true);
    let root = tempfile::tempdir().expect("workspace tempdir");
    let err = run_governed_inference(config, root.path().to_path_buf(), vec![])
        .await
        .expect_err("credentials in URL must be refused");
    assert_eq!(status_of(&err), "invalid-endpoint");
    assert_eq!(fixture.hit_count(), 0);
}

#[tokio::test]
async fn remote_service_refuses_endpoint_not_in_allowlist() {
    let fixture = spawn_fixture("ok", None, None);
    let config = RemoteServiceConfig::new(fixture.endpoint.clone(), MODEL)
        .allowed_hosts(vec!["otherhost.internal:443".to_string()])
        .api_key(API_KEY)
        .allow_private_destinations(true);
    let root = tempfile::tempdir().expect("workspace tempdir");
    let err = run_governed_inference(config, root.path().to_path_buf(), vec![])
        .await
        .expect_err("endpoint outside the allowlist must be refused");
    assert_eq!(status_of(&err), "allowlist-refused");
    assert_eq!(fixture.hit_count(), 0);
}

#[tokio::test]
async fn remote_service_refuses_empty_allowlist() {
    let fixture = spawn_fixture("ok", None, None);
    let config = RemoteServiceConfig::new(fixture.endpoint.clone(), MODEL)
        .allowed_hosts(vec![])
        .api_key(API_KEY)
        .allow_private_destinations(true);
    let root = tempfile::tempdir().expect("workspace tempdir");
    let err = run_governed_inference(config, root.path().to_path_buf(), vec![])
        .await
        .expect_err("an empty destination allowlist must be refused");
    assert_eq!(status_of(&err), "allowlist-refused");
    assert_eq!(fixture.hit_count(), 0);
}

#[tokio::test]
async fn remote_service_refuses_endpoint_mismatch_from_task_input() {
    let (fixture, config) = fixture_with_mode("ok");
    let root = tempfile::tempdir().expect("workspace tempdir");
    let args = json!({
        "model": MODEL,
        "endpoint": "https://unrelated.internal/v1",
        "messages": [{"role": "user", "content": "say hello"}],
        "maxTokens": 16,
    });
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[REMOTE_SERVICE_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch(&harness, &m, &r, &mut runner);
    let outcome = harness.run(step(args), &mut d).await;
    let err = outcome.expect_err("task-input endpoint override must be refused");
    assert_eq!(status_of(&err), "endpoint-mismatch");
    assert_eq!(fixture.hit_count(), 0);
}

#[tokio::test]
async fn remote_service_refuses_unauthorized_capability() {
    let fixture = spawn_fixture("ok", None, None);
    let config = config_for_fixture(&fixture, MODEL);
    let config_endpoint = config.endpoint.clone();
    let root = tempfile::tempdir().expect("workspace tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &["harness.local_model.infer"],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch(&harness, &m, &r, &mut runner);
    let wrong = HarnessStep {
        step_id: "step-1".to_string(),
        capability: "harness.local_model.infer".to_string(),
        args: inference_args(&config_endpoint),
    };
    let err = harness
        .run(wrong, &mut d)
        .await
        .expect_err("undeclared capability must refuse");
    assert!(!format!("{err:#}").is_empty());
    assert_eq!(fixture.hit_count(), 0, "no network effect may occur");
}

// ---------------------------------------------------------------------------
// Response-shape refusals (fixture answers with wrong content)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn remote_service_refuses_model_mismatch_in_response() -> Result<()> {
    let (_fixture, config) = fixture_with_mode("model-mismatch");
    let root = tempfile::tempdir().expect("workspace tempdir");
    let err = run_governed_inference(config, root.path().to_path_buf(), vec![])
        .await
        .expect_err("a different responding model must be refused");
    assert_eq!(status_of(&err), "model-mismatch");
    Ok(())
}

#[tokio::test]
async fn remote_service_refuses_missing_model_in_response() -> Result<()> {
    let (_fixture, config) = fixture_with_mode("missing-model");
    let root = tempfile::tempdir().expect("workspace tempdir");
    let err = run_governed_inference(config, root.path().to_path_buf(), vec![])
        .await
        .expect_err("a response without a model attestation must be refused");
    assert_eq!(status_of(&err), "model-mismatch");
    Ok(())
}

#[tokio::test]
async fn remote_service_refuses_malformed_response() -> Result<()> {
    let (_fixture, config) = fixture_with_mode("malformed");
    let root = tempfile::tempdir().expect("workspace tempdir");
    let err = run_governed_inference(config, root.path().to_path_buf(), vec![])
        .await
        .expect_err("malformed response must be refused");
    assert_eq!(status_of(&err), "invalid-json");
    Ok(())
}

#[tokio::test]
async fn remote_service_refuses_oversized_response() -> Result<()> {
    let (_fixture, config) = fixture_with_mode("oversize");
    let config = config.max_output_bytes(4096);
    let root = tempfile::tempdir().expect("workspace tempdir");
    let err = run_governed_inference(config, root.path().to_path_buf(), vec![])
        .await
        .expect_err("oversized response must be refused");
    assert_eq!(status_of(&err), "oversized-response");
    Ok(())
}

#[tokio::test]
async fn remote_service_provider_error_body_is_not_echoed() -> Result<()> {
    let (_fixture, config) = fixture_with_mode("status-500");
    let root = tempfile::tempdir().expect("workspace tempdir");
    let err = run_governed_inference(config, root.path().to_path_buf(), vec![])
        .await
        .expect_err("provider failure must be refused");
    assert_eq!(status_of(&err), "provider-error");
    let text = format!("{err:#}");
    assert!(
        !text.contains("provider exploded"),
        "provider error body must not be reflected anywhere"
    );
    assert!(
        !text.contains("fake-secret-key-12345"),
        "provider body secrets must not be reflected"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Deadline and cancellation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn remote_service_slow_fixture_times_out() -> Result<()> {
    let (_fixture, config) = fixture_with_mode("slow");
    let config = config.timeout_ms(300);
    let root = tempfile::tempdir().expect("workspace tempdir");
    let err = run_governed_inference(config, root.path().to_path_buf(), vec![])
        .await
        .expect_err("slow fixture must hit the deadline");
    assert_eq!(status_of(&err), "timeout");
    Ok(())
}

// The governed capability handler is synchronous and joins its transport
// worker thread; on a current-thread test runtime that would block the whole
// runtime for the invocation's duration. Multi-threaded flavor lets this
// test set the cancel flag while the invocation is in flight.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_service_cancellation_aborts_invocation() -> Result<()> {
    let (_fixture, config) = fixture_with_mode("slow");
    let cancel = Arc::new(AtomicBool::new(false));
    let config = config.timeout_ms(30_000).cancel(cancel.clone());
    let root = tempfile::tempdir().expect("workspace tempdir");
    let root_buf = root.path().to_path_buf();
    let handle =
        tokio::spawn(async move { run_governed_inference(config, root_buf, vec![]).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    cancel.store(true, Ordering::SeqCst);
    let err = handle.await.expect("task joins");
    let err = err.expect_err("cancelled invocation must be refused");
    assert_eq!(status_of(&err), "cancelled");
    Ok(())
}

// ---------------------------------------------------------------------------
// Secret handling
// ---------------------------------------------------------------------------

#[tokio::test]
async fn remote_service_secret_in_response_is_redacted() -> Result<()> {
    let (_fixture, config) = fixture_with_mode("secret-echo");
    let root = tempfile::tempdir().expect("workspace tempdir");
    let outcome =
        run_governed_inference(config, root.path().to_path_buf(), vec![API_KEY.to_string()])
            .await?;
    assert!(
        !outcome.output.contains(API_KEY),
        "known secret must not appear anywhere in the persisted outcome: {}",
        outcome.output
    );
    assert!(outcome.output.contains(REDACTED_PLACEHOLDER));
    Ok(())
}

#[tokio::test]
async fn remote_service_secret_is_not_reflected_on_failure() -> Result<()> {
    let (_fixture, config) = fixture_with_mode("status-500");
    let root = tempfile::tempdir().expect("workspace tempdir");
    let err = run_governed_inference(config, root.path().to_path_buf(), vec![API_KEY.to_string()])
        .await
        .expect_err("provider failure must be refused");
    let text = format!("{err:#}");
    assert!(!text.contains(API_KEY));
    assert!(!text.contains("fake-secret-key-12345"));
    Ok(())
}

// ---------------------------------------------------------------------------
// Strict task timeout policy (reject before ANY DNS/network effect)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn remote_service_task_timeout_zero_is_refused() {
    let (_fixture, config) = fixture_with_mode("ok");
    let root = tempfile::tempdir().expect("workspace tempdir");
    let args = json!({
        "model": MODEL,
        "endpoint": config.endpoint.clone(),
        "messages": [{"role": "user", "content": "say hello"}],
        "maxTokens": 16,
        "timeoutMs": 0,
    });
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[REMOTE_SERVICE_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch(&harness, &m, &r, &mut runner);
    let err = harness
        .run(step(args), &mut d)
        .await
        .expect_err("timeoutMs=0 must be refused");
    assert_eq!(status_of(&err), "invalid-timeout");
    // The fixture is not spawned by this test; remote wire coverage above
    // proves the governed path runs end-to-end when validation passes.
}

#[tokio::test]
async fn remote_service_task_timeout_over_ceiling_is_refused() {
    let (fixture, config) = fixture_with_mode("ok");
    let config = config.timeout_ms(1000); // operator ceiling: 1s
    let root = tempfile::tempdir().expect("workspace tempdir");
    let args = json!({
        "model": MODEL,
        "endpoint": config.endpoint.clone(),
        "messages": [{"role": "user", "content": "say hello"}],
        "maxTokens": 16,
        "timeoutMs": 10_000, // over the ceiling
    });
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[REMOTE_SERVICE_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch(&harness, &m, &r, &mut runner);
    let err = harness
        .run(step(args), &mut d)
        .await
        .expect_err("over-ceiling timeoutMs must be refused");
    assert_eq!(status_of(&err), "invalid-timeout");
    assert_eq!(fixture.hit_count(), 0, "no DNS/network effect may occur");
}

#[tokio::test]
async fn remote_service_task_timeout_non_integer_forms_are_refused() {
    for bad in [
        json!(1.5),
        json!("300"),
        json!(true),
        json!(null),
        json!(-1),
    ] {
        let (_fixture, config) = fixture_with_mode("ok");
        let root = tempfile::tempdir().expect("workspace tempdir");
        let args = json!({
            "model": MODEL,
            "endpoint": config.endpoint.clone(),
            "messages": [{"role": "user", "content": "say hello"}],
            "maxTokens": 16,
            "timeoutMs": bad,
        });
        let mut runner = runner_for(
            config,
            root.path().to_path_buf(),
            &[REMOTE_SERVICE_INFER_CAPABILITY],
        );
        let harness = harness();
        let m = manifest();
        let r = restrictions();
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        let err = harness
            .run(step(args), &mut d)
            .await
            .expect_err("non-integer timeoutMs must be refused");
        assert_eq!(
            status_of(&err),
            "invalid-timeout",
            "input {bad} must refuse"
        );
    }
}

#[tokio::test]
async fn remote_service_task_timeout_shorter_than_ceiling_is_allowed() {
    let (fixture, config) = fixture_with_mode("ok");
    let config = config.timeout_ms(30_000);
    let root = tempfile::tempdir().expect("workspace tempdir");
    let args = json!({
        "model": MODEL,
        "endpoint": config.endpoint.clone(),
        "messages": [{"role": "user", "content": "say hello"}],
        "maxTokens": 16,
        "timeoutMs": 1000, // shorter than the ceiling: allowed
    });
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[REMOTE_SERVICE_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch(&harness, &m, &r, &mut runner);
    let outcome = harness
        .run(step(args), &mut d)
        .await
        .expect("shorter timeout within ceiling must succeed");
    let output: Value = serde_json::from_str(&outcome.output).unwrap();
    assert_eq!(output["success"], json!(true));
    assert_eq!(fixture.hit_count(), 1);
}

#[tokio::test]
async fn remote_service_operator_timeout_zero_config_refused() {
    let (fixture, config) = fixture_with_mode("ok");
    let config = config.timeout_ms(0); // invalid operator configuration
    let root = tempfile::tempdir().expect("workspace tempdir");
    let err = run_governed_inference(config, root.path().to_path_buf(), vec![])
        .await
        .expect_err("zero operator timeout ceiling must fail closed");
    assert_eq!(status_of(&err), "invalid-config");
    assert_eq!(fixture.hit_count(), 0, "no network effect may occur");
}

#[tokio::test]
async fn remote_service_operator_token_zero_config_refused() {
    let (fixture, config) = fixture_with_mode("ok");
    let config = config.max_output_tokens(0); // invalid operator configuration
    let root = tempfile::tempdir().expect("workspace tempdir");
    let err = run_governed_inference(config, root.path().to_path_buf(), vec![])
        .await
        .expect_err("zero operator token ceiling must fail closed");
    assert_eq!(status_of(&err), "invalid-config");
    assert_eq!(fixture.hit_count(), 0, "no network effect may occur");
}

// ---------------------------------------------------------------------------
// Governed token budget (operator ceiling; never exceeded; never invented)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn remote_service_max_tokens_over_ceiling_is_refused() {
    let (fixture, config) = fixture_with_mode("ok");
    let config = config.max_output_tokens(64); // operator ceiling: 64
    let root = tempfile::tempdir().expect("workspace tempdir");
    let args = json!({
        "model": MODEL,
        "endpoint": config.endpoint.clone(),
        "messages": [{"role": "user", "content": "say hello"}],
        "maxTokens": 65, // over the ceiling
    });
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[REMOTE_SERVICE_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch(&harness, &m, &r, &mut runner);
    let err = harness
        .run(step(args), &mut d)
        .await
        .expect_err("over-ceiling maxTokens must be refused");
    assert_eq!(status_of(&err), "invalid-token-budget");
    assert_eq!(fixture.hit_count(), 0, "no network effect may occur");
}

#[tokio::test]
async fn remote_service_max_tokens_invalid_forms_are_refused() {
    for bad in [json!(0), json!(1.5), json!("100"), json!(true), json!(null)] {
        let (_fixture, config) = fixture_with_mode("ok");
        let root = tempfile::tempdir().expect("workspace tempdir");
        let args = json!({
            "model": MODEL,
            "endpoint": config.endpoint.clone(),
            "messages": [{"role": "user", "content": "say hello"}],
            "maxTokens": bad,
        });
        let mut runner = runner_for(
            config,
            root.path().to_path_buf(),
            &[REMOTE_SERVICE_INFER_CAPABILITY],
        );
        let harness = harness();
        let m = manifest();
        let r = restrictions();
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        let err = harness
            .run(step(args), &mut d)
            .await
            .expect_err("invalid maxTokens must be refused");
        assert_eq!(
            status_of(&err),
            "invalid-token-budget",
            "input {bad} must refuse"
        );
    }
}

#[tokio::test]
async fn remote_service_max_tokens_at_ceiling_is_allowed() {
    let (fixture, config) = fixture_with_mode("ok");
    let config = config.max_output_tokens(64);
    let root = tempfile::tempdir().expect("workspace tempdir");
    let args = json!({
        "model": MODEL,
        "endpoint": config.endpoint.clone(),
        "messages": [{"role": "user", "content": "say hello"}],
        "maxTokens": 64,
    });
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[REMOTE_SERVICE_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch(&harness, &m, &r, &mut runner);
    let outcome = harness
        .run(step(args), &mut d)
        .await
        .expect("maxTokens at the ceiling must succeed");
    let output: Value = serde_json::from_str(&outcome.output).unwrap();
    assert_eq!(output["success"], json!(true));
    assert_eq!(fixture.hit_count(), 1);
}

// ---------------------------------------------------------------------------
// responseFormat and identifier validation (fail closed before any effect)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn remote_service_response_format_garbage_is_refused() {
    for bad in [
        json!("json"),
        json!("text"),
        json!(true),
        json!(123),
        json!({"type": "json_object"}),
    ] {
        let (_fixture, config) = fixture_with_mode("ok");
        let root = tempfile::tempdir().expect("workspace tempdir");
        let args = json!({
            "model": MODEL,
            "endpoint": config.endpoint.clone(),
            "messages": [{"role": "user", "content": "say hello"}],
            "maxTokens": 16,
            "responseFormat": bad,
        });
        let mut runner = runner_for(
            config,
            root.path().to_path_buf(),
            &[REMOTE_SERVICE_INFER_CAPABILITY],
        );
        let harness = harness();
        let m = manifest();
        let r = restrictions();
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        let err = harness
            .run(step(args), &mut d)
            .await
            .expect_err("unsupported responseFormat must be refused");
        assert_eq!(
            status_of(&err),
            "invalid-response-format",
            "input {bad} must refuse"
        );
    }
}

#[tokio::test]
async fn remote_service_response_format_json_object_is_accepted() {
    let (fixture, config) = fixture_with_mode("ok-json");
    let root = tempfile::tempdir().expect("workspace tempdir");
    let args = json!({
        "model": MODEL,
        "endpoint": config.endpoint.clone(),
        "messages": [{"role": "user", "content": "say hello"}],
        "maxTokens": 16,
        "responseFormat": "json_object",
    });
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[REMOTE_SERVICE_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch(&harness, &m, &r, &mut runner);
    let outcome = harness
        .run(step(args), &mut d)
        .await
        .expect("json_object responseFormat must be accepted");
    let output: Value = serde_json::from_str(&outcome.output).unwrap();
    assert_eq!(output["success"], json!(true));
    assert_eq!(fixture.hit_count(), 1);
}

#[tokio::test]
async fn remote_service_step_identifier_too_long_is_refused() {
    let (_fixture, config) = fixture_with_mode("ok");
    let root = tempfile::tempdir().expect("workspace tempdir");
    let config_endpoint = config.endpoint.clone();
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[REMOTE_SERVICE_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch(&harness, &m, &r, &mut runner);
    let oversized = HarnessStep {
        step_id: "s".repeat(513),
        capability: REMOTE_SERVICE_INFER_CAPABILITY.to_string(),
        args: inference_args(&config_endpoint),
    };
    let err = harness
        .run(oversized, &mut d)
        .await
        .expect_err("over-long step identifier must be refused before dispatch");
    assert!(
        format!("{err:#}").contains("step identifier exceeds"),
        "unexpected error: {err:#}"
    );
}

#[tokio::test]
async fn remote_service_connection_budget_includes_dns() {
    // Fixture takes 10s; a 250ms task deadline must fire from within the
    // resolution+request lifecycle, not after it.
    let (fixture, config) = fixture_with_mode("slow");
    let config = config.timeout_ms(30_000);
    let root = tempfile::tempdir().expect("workspace tempdir");
    let args = json!({
        "model": MODEL,
        "endpoint": config.endpoint.clone(),
        "messages": [{"role": "user", "content": "say hello"}],
        "maxTokens": 16,
        "timeoutMs": 1500,
    });
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[REMOTE_SERVICE_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let started = std::time::Instant::now();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(args), &mut d)
            .await
            .expect_err("slow fixture must hit the 250ms deadline")
    };
    let elapsed = started.elapsed();
    assert_eq!(status_of(&err), "timeout");
    assert!(
        elapsed < Duration::from_secs(5),
        "deadline must fire inside the lifecycle, not after it ({elapsed:?})"
    );
    assert_eq!(fixture.hit_count(), 1);
}

// ---------------------------------------------------------------------------
// Wire verification: effective max_tokens on the serialized request
// ---------------------------------------------------------------------------

fn inference_args_without_max_tokens(endpoint: &str) -> Value {
    json!({
        "model": MODEL,
        "endpoint": endpoint,
        "messages": [{"role": "user", "content": "say hello"}],
    })
}

async fn run_governed_args(
    config: RemoteServiceConfig,
    workspace_root: PathBuf,
    args: Value,
) -> Result<HarnessOutcome> {
    let mut runner = runner_for(config, workspace_root, &[REMOTE_SERVICE_INFER_CAPABILITY]);
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch_with_secrets(&harness, &m, &r, &mut runner, vec![]);
    harness.run(step(args), &mut d).await
}

#[tokio::test]
async fn remote_service_omitted_max_tokens_uses_operator_ceiling() -> Result<()> {
    let (fixture, config) = fixture_with_mode("ok");
    let config = config.max_output_tokens(128);
    let endpoint = config.endpoint.clone();
    let root = tempfile::tempdir().expect("workspace tempdir");
    let args = inference_args_without_max_tokens(&endpoint);
    let outcome = run_governed_args(config, root.path().to_path_buf(), args).await?;
    let output: Value = serde_json::from_str(&outcome.output)?;
    assert_eq!(output["success"], json!(true));
    let wire = fixture
        .last_request()
        .expect("the fixture logged the wire request");
    assert_eq!(
        wire["max_tokens"],
        json!(128),
        "an omitted task maxTokens must yield the operator ceiling on the wire"
    );
    Ok(())
}

#[tokio::test]
async fn remote_service_omitted_max_tokens_uses_default_operator_ceiling() -> Result<()> {
    let (fixture, config) = fixture_with_mode("ok"); // default ceiling: 4096
    let endpoint = config.endpoint.clone();
    let root = tempfile::tempdir().expect("workspace tempdir");
    let args = inference_args_without_max_tokens(&endpoint);
    let outcome = run_governed_args(config, root.path().to_path_buf(), args).await?;
    let output: Value = serde_json::from_str(&outcome.output)?;
    assert_eq!(output["success"], json!(true));
    let wire = fixture
        .last_request()
        .expect("the fixture logged the wire request");
    assert_eq!(
        wire["max_tokens"],
        json!(4096),
        "the default operator ceiling must be present on the wire"
    );
    Ok(())
}

#[tokio::test]
async fn remote_service_explicit_shorter_max_tokens_is_used_on_wire() -> Result<()> {
    let (fixture, config) = fixture_with_mode("ok");
    let config = config.max_output_tokens(4096);
    let endpoint = config.endpoint.clone();
    let root = tempfile::tempdir().expect("workspace tempdir");
    let args = json!({
        "model": MODEL,
        "endpoint": endpoint,
        "messages": [{"role": "user", "content": "say hello"}],
        "maxTokens": 100,
    });
    let outcome = run_governed_args(config, root.path().to_path_buf(), args).await?;
    let output: Value = serde_json::from_str(&outcome.output)?;
    assert_eq!(output["success"], json!(true));
    let wire = fixture
        .last_request()
        .expect("the fixture logged the wire request");
    assert_eq!(
        wire["max_tokens"],
        json!(100),
        "a shorter task budget is sent as-is"
    );
    Ok(())
}

#[tokio::test]
async fn remote_service_over_ceiling_max_tokens_refused_before_network() -> Result<()> {
    let (fixture, config) = fixture_with_mode("ok");
    let config = config.max_output_tokens(64);
    let endpoint = config.endpoint.clone();
    let root = tempfile::tempdir().expect("workspace tempdir");
    let args = json!({
        "model": MODEL,
        "endpoint": endpoint,
        "messages": [{"role": "user", "content": "say hello"}],
        "maxTokens": 65,
    });
    let err = run_governed_args(config, root.path().to_path_buf(), args)
        .await
        .expect_err("over-ceiling maxTokens must refuse");
    assert_eq!(status_of(&err), "invalid-token-budget");
    assert_eq!(fixture.hit_count(), 0, "no network effect may occur");
    assert!(
        fixture.last_request().is_none(),
        "the fixture must never see an out-of-budget request"
    );
    Ok(())
}
