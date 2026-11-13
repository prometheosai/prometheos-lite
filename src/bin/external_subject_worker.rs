//! Deterministic external coding-agent compatibility subject (issue #154,
//! Slice J) — JSON-RPC 2.0 over stdin/stdout.
//!
//! This is NOT a real third-party coding agent and claims no compatibility
//! with Claude Code, Codex, Goose, OpenCode, Aider, ACP or any vendor
//! protocol. It is a deterministic fixture that speaks a published standard
//! envelope (JSON-RPC 2.0, <https://www.jsonrpc.org/specification>) with a
//! deliberately small method set, so the governed remote-subject adapter can
//! be exercised across a REAL process boundary without a paid provider or a
//! downloaded model.
//!
//! Supported methods (single connection; newline-delimited JSON):
//!   * `initialize` — handshake. Reports `protocolVersion`,
//!     `serverInfo { name, version }` and
//!     `capabilities { completions: { streaming: false } }`.
//!     argv hooks: `--bad-server-name`, `--no-completions`,
//!     `--bad-protocol-version`, `--wrong-id`.
//!   * `completion/complete` — one completion. `params: { prompt, model? }`.
//!     argv hooks: `--slow` (holds the request open), `--flood` (oversized
//!     response), `--malformed` (non-JSON response), `--jsonrpc-error`
//!     (respond with error object), `--model-swap`, `--missing-usage`,
//!     `--echo-prompt`.
//!   * anything else — JSON-RPC error -32601.
//!
//! Bounded: stdin is capped at 1 MiB total. Exits 0 on stdin EOF, or on the
//! argv hooks `--never-read` / `--exit-now` / `--spawn-holder` used by
//! containment regression tests.
//!
//! argv: `--record-file <path>` logs each method name to the file, one per
//! line (used by tests to prove what the adapter actually sent).

use std::io::Write;
use std::path::PathBuf;

const PROTOCOL_VERSION: &str = "2026.1";
const SERVER_NAME: &str = "external-subject-fixture";
const SERVER_VERSION: &str = "0.1.0";
const DEFAULT_MODEL: &str = "external-subject-model";

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--never-read") {
        std::thread::sleep(std::time::Duration::from_secs(30));
        std::process::exit(0);
    }
    if args.iter().any(|a| a == "--exit-now") {
        std::process::exit(3);
    }
    if args.iter().any(|a| a == "--spawn-holder") {
        let exe = std::env::current_exe().expect("current exe");
        let _ = std::process::Command::new(exe)
            .arg("--never-read")
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        std::process::exit(0);
    }

    let mut record_file: Option<PathBuf> = None;
    let mut iter = args.iter().skip(1);
    while let Some(a) = iter.next() {
        if a == "--record-file" {
            if let Some(p) = iter.next() {
                record_file = Some(PathBuf::from(p));
            }
        } else if let Some(p) = a.strip_prefix("--record-file=") {
            record_file = Some(PathBuf::from(p));
        }
    }
    let bad_server_name = args.iter().any(|a| a == "--bad-server-name");
    let no_completions = args.iter().any(|a| a == "--no-completions");
    let bad_protocol_version = args.iter().any(|a| a == "--bad-protocol-version");
    let wrong_id = args.iter().any(|a| a == "--wrong-id");
    let slow = args.iter().any(|a| a == "--slow");
    let flood = args.iter().any(|a| a == "--flood");
    let malformed = args.iter().any(|a| a == "--malformed");
    let jsonrpc_error = args.iter().any(|a| a == "--jsonrpc-error");
    let model_swap = args.iter().any(|a| a == "--model-swap");
    let missing_usage = args.iter().any(|a| a == "--missing-usage");
    let echo_prompt = args.iter().any(|a| a == "--echo-prompt");

    // Streaming line-oriented server: respond to each request as it arrives,
    // then exit 0 when stdin reaches EOF (the adapter closes stdin after the
    // completion response to signal end-of-dialogue).
    let record_file_owned = record_file;
    let record = |method: &str| {
        if let Some(path) = &record_file_owned {
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .and_then(|mut f| writeln!(f, "{method}"));
        }
    };
    let mut stdin = std::io::stdin();
    let mut reader = std::io::BufReader::new(&mut stdin);
    let line_budget: u64 = 1024 * 1024;
    let mut total: u64 = 0;
    loop {
        let mut line = String::new();
        use std::io::BufRead;
        match reader.read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(n) => {
                total += n as u64;
                if total > line_budget {
                    eprintln!("external-subject-worker: request exceeds {line_budget} bytes");
                    std::process::exit(2);
                }
            }
            Err(e) => {
                eprintln!("external-subject-worker: stdin read failed: {e}");
                std::process::exit(2);
            }
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let request: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(_) => {
                respond(&serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": serde_json::Value::Null,
                    "error": { "code": -32700, "message": "parse error" },
                }));
                continue;
            }
        };
        let id = request
            .get("id")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let method = request
            .get("method")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let params = request
            .get("params")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        record(method);

        match method {
            "initialize" => {
                let version = if bad_protocol_version {
                    "99.0.0"
                } else {
                    PROTOCOL_VERSION
                };
                let name = if bad_server_name {
                    "not-a-coding-agent"
                } else {
                    SERVER_NAME
                };
                let mut capabilities = serde_json::Map::new();
                if !no_completions {
                    capabilities.insert(
                        "completions".to_string(),
                        serde_json::json!({ "streaming": false }),
                    );
                }
                let response_id = if wrong_id {
                    serde_json::json!("id-forged-by-subject")
                } else {
                    id
                };
                respond(&serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": response_id,
                    "result": {
                        "protocolVersion": version,
                        "serverInfo": { "name": name, "version": SERVER_VERSION },
                        "capabilities": capabilities,
                    },
                }));
            }
            "completion/complete" => {
                if slow {
                    std::thread::sleep(std::time::Duration::from_secs(30));
                }
                if flood {
                    let body = "X".repeat(2 * 1024 * 1024);
                    respond(&serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [{ "type": "text", "text": body }],
                            "model": DEFAULT_MODEL,
                        },
                    }));
                    continue;
                }
                if malformed {
                    let _ = writeln!(std::io::stdout(), "{{\"jsonrpc\":");
                    let _ = std::io::stdout().flush();
                    continue;
                }
                if jsonrpc_error {
                    respond(&serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32603, "message": "subject internal error" },
                    }));
                    continue;
                }
                let prompt = params
                    .get("prompt")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let model = if model_swap {
                    "other-subject-model"
                } else {
                    DEFAULT_MODEL
                };
                let text = if echo_prompt {
                    format!("echo: {prompt}")
                } else {
                    "fixture completion".to_string()
                };
                let mut result = serde_json::json!({
                    "content": [{ "type": "text", "text": text }],
                    "model": model,
                });
                if !missing_usage {
                    result["usage"] = serde_json::json!({
                        "promptTokens": 11,
                        "completionTokens": 7,
                        "totalTokens": 18,
                    });
                }
                respond(&serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": result,
                }));
            }
            _ => {
                respond(&serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": "method not found" },
                }));
            }
        }
    }
    // EOF framing: exit only after all requests were answered.
}

fn respond(value: &serde_json::Value) {
    let line = serde_json::to_string(value).expect("response serializes");
    println!("{line}");
    let _ = std::io::stdout().flush();
}
