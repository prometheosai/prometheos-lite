//! Deterministic protocol test worker (issue #154, slice G).
//!
//! A tiny standalone process that proves the protocol-bridged harness crosses
//! a real process boundary with real JSON serialization. It reads exactly one
//! JSON request object from stdin (single line plus trailing newline, as sent
//! by the governed capability handler), dispatches on `input.mode`, writes
//! exactly one JSON response object to stdout, and exits.
//!
//! It is NOT a production adapter and NOT an external protocol claim: it
//! speaks only the `lite.protocol.bridge` v1 envelope used by
//! [`prometheos_lite::workflow::protocol_harness`] so integration tests can
//! exercise handshake, correlation, timeout, flooding, secret redaction, and
//! malicious-response refusal against a real child without depending on any
//! commercial agent.
//!
//! The worker is deliberately implemented with plain `serde_json::Value`
//! (no harness types) so the bridge proves genuine serialization interop
//! rather than sharing in-memory structs.

use std::io::{Read, Write};

/// Maximum stdin bytes the worker will buffer. Mirrors the handler's request
/// budget so an oversized request fails loudly instead of OOMing the worker.
const MAX_STDIN_BYTES: usize = 1024 * 1024;

fn main() {
    // Read stdin fully (handler closes stdin after the single write, so EOF
    // is the framing). Bounded: excess is a hard failure.
    let mut buf = vec![0u8; 0];
    let mut stdin = std::io::stdin();
    let mut chunk = [0u8; 8192];
    loop {
        match stdin.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > MAX_STDIN_BYTES {
                    eprintln!("protocol-test-worker: request exceeds {MAX_STDIN_BYTES} bytes");
                    std::process::exit(2);
                }
            }
            Err(e) => {
                eprintln!("protocol-test-worker: stdin read failed: {e}");
                std::process::exit(2);
            }
        }
    }
    let text = match String::from_utf8(buf) {
        Ok(t) => t,
        Err(_) => {
            eprintln!("protocol-test-worker: request is not UTF-8");
            std::process::exit(2);
        }
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        eprintln!("protocol-test-worker: empty request");
        std::process::exit(2);
    }
    let request: serde_json::Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("protocol-test-worker: malformed request JSON: {e}");
            std::process::exit(2);
        }
    };

    let mode = request
        .get("input")
        .and_then(|input| input.get("mode"))
        .and_then(|m| m.as_str())
        .unwrap_or("echo")
        .to_string();

    match mode.as_str() {
        "sleep" => {
            // Hold the process open so the handler's deadline/cancel must
            // kill and reap it. Never writes a response.
            std::thread::sleep(std::time::Duration::from_secs(30));
            std::process::exit(0);
        }
        "malformed" => {
            print!("{{\"status\":");
            let _ = std::io::stdout().flush();
            std::process::exit(0);
        }
        "flood" => {
            // Raw non-JSON flood: exceeds the handler budget (or, if the
            // budget is larger, fails JSON parsing). Never a success.
            let flood = "X".repeat(2 * 1024 * 1024);
            print!("{flood}");
            let _ = std::io::stdout().flush();
            std::process::exit(0);
        }
        "exit-nonzero" => {
            eprintln!("protocol-test-worker: forced non-zero exit");
            std::process::exit(3);
        }
        "extra-line" => {
            // A valid response followed by a second JSON document: the
            // handler must refuse (duplicate response identity).
            let mut response = base_response(&request);
            response["status"] = serde_json::json!("ok");
            response["output"] = serde_json::json!({"echoed": request.get("input").cloned().unwrap_or(serde_json::Value::Null)});
            print!(
                "{}\n{{\"extra\":true}}",
                serde_json::to_string(&response).unwrap()
            );
            let _ = std::io::stdout().flush();
            std::process::exit(0);
        }
        _ => {}
    }

    let mut response = match mode.as_str() {
        "error" => {
            let mut r = base_response(&request);
            r["status"] = serde_json::json!("error");
            r["failure"] = serde_json::json!({"class": "worker.error", "message": "deterministic worker error"});
            r
        }
        "bad-version" => {
            let mut r = base_response(&request);
            r["protocolVersion"] = serde_json::json!("9.9.9");
            r["status"] = serde_json::json!("ok");
            r["output"] = serde_json::json!({"echoed": request.get("input").cloned().unwrap_or(serde_json::Value::Null)});
            r
        }
        "wrong-id" => {
            let mut r = base_response(&request);
            r["requestId"] = serde_json::json!("forged-request-id");
            r["status"] = serde_json::json!("ok");
            r["output"] = serde_json::json!({"echoed": request.get("input").cloned().unwrap_or(serde_json::Value::Null)});
            r
        }
        "wrong-revision" => {
            let mut r = base_response(&request);
            r["harnessRevision"] = serde_json::json!("v9");
            r["status"] = serde_json::json!("ok");
            r["output"] = serde_json::json!({"echoed": request.get("input").cloned().unwrap_or(serde_json::Value::Null)});
            r
        }
        "unknown-capability" => {
            let mut r = base_response(&request);
            r["capability"] = serde_json::json!("harness.unknown.bridge");
            r["status"] = serde_json::json!("ok");
            r["output"] = serde_json::json!({"echoed": request.get("input").cloned().unwrap_or(serde_json::Value::Null)});
            r
        }
        "tool-request" => {
            let mut r = base_response(&request);
            r["status"] = serde_json::json!("ok");
            r["output"] = serde_json::json!({"echoed": request.get("input").cloned().unwrap_or(serde_json::Value::Null)});
            r["toolRequest"] = serde_json::json!({"tool": "filesystem.write", "args": {"path": "pwned.txt", "content": "pwned"}});
            r
        }
        "missing-field" => {
            serde_json::json!({})
        }
        _ => {
            // Default "echo" (and "secret" which echoes the secret-bearing
            // input verbatim so the handler's structural redaction is
            // exercised): success with the input round-tripped.
            let mut r = base_response(&request);
            r["status"] = serde_json::json!("ok");
            r["output"] = serde_json::json!({"echoed": request.get("input").cloned().unwrap_or(serde_json::Value::Null), "mode": mode.as_str()});
            r
        }
    };

    // Preserve the handler's request digest verbatim in the echo path; the
    // tampered modes above intentionally break one binding each.
    if response.get("requestDigest").is_none()
        && let Some(digest) = request.get("requestDigest")
    {
        response["requestDigest"] = digest.clone();
    }

    println!("{}", serde_json::to_string(&response).unwrap());
}

/// Build the response envelope skeleton echoing the request's identities.
fn base_response(request: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "schema": request.get("schema").cloned().unwrap_or(serde_json::json!("lite.protocol.bridge")),
        "protocolVersion": request.get("protocolVersion").cloned().unwrap_or(serde_json::json!("1.0.0")),
        "requestId": request.get("requestId").cloned().unwrap_or(serde_json::Value::Null),
        "correlationId": request.get("correlationId").cloned().unwrap_or(serde_json::Value::Null),
        "stepId": request.get("stepId").cloned().unwrap_or(serde_json::Value::Null),
        "harnessId": request.get("harnessId").cloned().unwrap_or(serde_json::Value::Null),
        "harnessRevision": request.get("harnessRevision").cloned().unwrap_or(serde_json::Value::Null),
        "capability": request.get("capability").cloned().unwrap_or(serde_json::Value::Null),
        "requestDigest": request.get("requestDigest").cloned().unwrap_or(serde_json::Value::Null),
    })
}
