//! Dispatch 398 distinct-process worker for production provisioning.
//! Emits a strict machine-readable outcome file and exits with a distinct
//! status for accepted, refused and execution-failure outcomes. With no
//! environment inputs it performs no side effects.
#![cfg(windows)]

use prometheos_lite::work::owner_registry::windows::provision_registry;
use prometheos_lite::work::owner_registry::{
    OWNER_REGISTRY_SCHEMA, OwnerBinding, OwnerRegistry, canonical_json,
};
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use winapi::shared::winerror::WAIT_TIMEOUT;
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
use winapi::um::synchapi::{OpenEventW, SetEvent, WaitForSingleObject};
use winapi::um::winbase::WAIT_OBJECT_0;
use winapi::um::winnt::{EVENT_MODIFY_STATE, HANDLE, SYNCHRONIZE};

const SID_A: &str = "S-1-5-21-1111111111-2222222222-3333333333-1001";
const EXIT_ACCEPTED: i32 = 0;
const EXIT_REFUSED: i32 = 10;
const EXIT_EXECUTION_FAILURE: i32 = 20;

struct EventHandle(HANDLE);
impl Drop for EventHandle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

fn open_event(name: &str) -> Result<EventHandle, String> {
    let handle = unsafe {
        OpenEventW(
            SYNCHRONIZE | EVENT_MODIFY_STATE,
            0,
            wide(OsStr::new(name)).as_ptr(),
        )
    };
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return Err(format!("OpenEventW {name} failed: {}", unsafe {
            GetLastError()
        }));
    }
    Ok(EventHandle(handle))
}

fn emit(path: &str, body: serde_json::Value) {
    if std::fs::write(path, body.to_string()).is_err() {
        eprintln!("PR341_WORKER_OUTCOME_WRITE_FAILED path={path}");
    }
}

fn finish(path: &str, body: serde_json::Value, code: i32) -> ! {
    emit(path, body);
    std::process::exit(code);
}

#[test]
fn provision_one_generation_from_worker_process() {
    let outcome_path = match std::env::var("PR341_WORKER_OUTCOME") {
        Ok(value) => value,
        Err(_) => {
            println!("PR341_WORKER_IDLE no outcome path requested");
            return;
        }
    };
    let registry_path = match std::env::var("PR341_WORKER_REGISTRY") {
        Ok(value) => value,
        Err(_) => finish(
            &outcome_path,
            serde_json::json!({"kind": "execution_failure", "detail": "missing registry path"}),
            EXIT_EXECUTION_FAILURE,
        ),
    };
    let generation: u64 = match std::env::var("PR341_WORKER_GENERATION")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
    {
        Some(value) => value,
        None => finish(
            &outcome_path,
            serde_json::json!({"kind": "execution_failure", "detail": "invalid generation"}),
            EXIT_EXECUTION_FAILURE,
        ),
    };
    let owner = std::env::var("PR341_WORKER_OWNER").unwrap_or_else(|_| "pr341-worker".to_string());
    let ready_name = match std::env::var("PR341_WORKER_READY_EVENT") {
        Ok(value) => value,
        Err(_) => finish(
            &outcome_path,
            serde_json::json!({"kind": "execution_failure", "detail": "missing ready event"}),
            EXIT_EXECUTION_FAILURE,
        ),
    };
    let go_name = match std::env::var("PR341_WORKER_GO_EVENT") {
        Ok(value) => value,
        Err(_) => finish(
            &outcome_path,
            serde_json::json!({"kind": "execution_failure", "detail": "missing go event"}),
            EXIT_EXECUTION_FAILURE,
        ),
    };

    let ready = match open_event(&ready_name) {
        Ok(event) => event,
        Err(error) => finish(
            &outcome_path,
            serde_json::json!({"kind": "execution_failure", "detail": error}),
            EXIT_EXECUTION_FAILURE,
        ),
    };
    if unsafe { SetEvent(ready.0) } == 0 {
        finish(
            &outcome_path,
            serde_json::json!({"kind": "execution_failure", "detail": "SetEvent ready failed"}),
            EXIT_EXECUTION_FAILURE,
        );
    }
    drop(ready);

    let go = match open_event(&go_name) {
        Ok(event) => event,
        Err(error) => finish(
            &outcome_path,
            serde_json::json!({"kind": "execution_failure", "detail": error}),
            EXIT_EXECUTION_FAILURE,
        ),
    };
    let wait = unsafe { WaitForSingleObject(go.0, 10_000) };
    drop(go);
    if wait == WAIT_TIMEOUT || wait != WAIT_OBJECT_0 {
        finish(
            &outcome_path,
            serde_json::json!({"kind": "execution_failure", "detail": "go event timeout"}),
            EXIT_EXECUTION_FAILURE,
        );
    }

    let proposed = OwnerRegistry {
        schema_version: OWNER_REGISTRY_SCHEMA.to_string(),
        generation,
        bindings: vec![OwnerBinding {
            sid: SID_A.to_string(),
            owner: owner.clone(),
            revoked: false,
            revision: 1,
        }],
    };
    let digest = canonical_json(&proposed)
        .ok()
        .map(|bytes| hex(Sha256::digest(&bytes).as_slice()))
        .unwrap_or_default();

    match provision_registry(Path::new(&registry_path), &proposed) {
        Ok(()) => finish(
            &outcome_path,
            serde_json::json!({
                "kind": "accepted", "generation": generation, "owner": owner,
                "registry_sha256": digest
            }),
            EXIT_ACCEPTED,
        ),
        Err(refusal) => finish(
            &outcome_path,
            serde_json::json!({
                "kind": "refused", "generation": generation, "owner": owner,
                "refusal": format!("{refusal:?}")
            }),
            EXIT_REFUSED,
        ),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
