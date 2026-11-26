//! Dispatch 397 distinct-process worker for production provisioning.
//! Invoked only by concurrent_provisioning_serializes_production_updates with
//! explicit environment inputs. With no inputs it performs no side effects.
#![cfg(windows)]

use prometheos_lite::work::owner_registry::windows::provision_registry;
use prometheos_lite::work::owner_registry::{OWNER_REGISTRY_SCHEMA, OwnerBinding, OwnerRegistry};
use std::path::Path;

const SID_A: &str = "S-1-5-21-1111111111-2222222222-3333333333-1001";

#[test]
fn provision_one_generation_from_worker_process() {
    let path = match std::env::var("PR341_WORKER_REGISTRY") {
        Ok(value) => value,
        Err(_) => {
            println!("PR341_WORKER_IDLE no registry requested");
            return;
        }
    };
    let generation: u64 = std::env::var("PR341_WORKER_GENERATION")
        .expect("worker generation must be supplied")
        .parse()
        .expect("worker generation must be numeric");
    let proposed = OwnerRegistry {
        schema_version: OWNER_REGISTRY_SCHEMA.to_string(),
        generation,
        bindings: vec![OwnerBinding {
            sid: SID_A.to_string(),
            owner: "pr341-worker".to_string(),
            revoked: false,
            revision: 1,
        }],
    };
    match provision_registry(Path::new(&path), &proposed) {
        Ok(()) => println!("PROVISION_OK generation={generation}"),
        Err(error) => println!("PROVISION_REFUSED generation={generation} error={error:?}"),
    }
}
