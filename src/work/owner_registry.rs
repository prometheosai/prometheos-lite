// Copyright (c) 2026 Memento Mori Labs LLC. All rights reserved.
// Proprietary. Trusted Windows SID-to-Lite-owner provisioning (issue #340).
//
// This is STAGE 1 only: a versioned, machine-local, administrator/SYSTEM
// controlled owner binding registry. It deliberately provides NO authenticated
// transport, NO client enrollment and NO execution authority. Every operation
// fails closed unless a real elevated Windows authority is proven and the
// registry storage and all of its ancestors pass strict ACL/reparse checks.
//
// A simulated SID fixture can validate DATA FORMATS only. It is never proof of
// a Windows principal. Elevated provisioning acceptance is a separate gate.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

/// Wire/schema identity of the owner registry.
pub const OWNER_REGISTRY_SCHEMA: &str = "prometheos-owner-registry-v1";

/// Location below the machine-local ProgramData root.
pub const PROGRAMDATA_RELATIVE_PATH: &str = "PrometheOS/Lite/owner-registry/registry.json";

/// Well-known Windows SIDs used by the privilege separation model.
pub const SID_SYSTEM: &str = "S-1-5-18";
pub const SID_ADMINISTRATORS: &str = "S-1-5-32-544";
pub const SID_EVERYONE: &str = "S-1-1-0";
pub const SID_AUTHENTICATED_USERS: &str = "S-1-5-11";
pub const SID_USERS: &str = "S-1-5-32-545";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OwnerBinding {
    /// Exact Windows SID string of the trusted principal.
    pub sid: String,
    /// Lite work-context owner (`WorkContext.user_id`) this principal may observe.
    pub owner: String,
    /// Durable, monotonic revocation marker. Once true, never cleared.
    pub revoked: bool,
    /// Monotonic registry revision for replacement/race detection.
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OwnerRegistry {
    pub schema_version: String,
    /// Monotonic generation. Every provisioning write must increase it.
    pub generation: u64,
    pub bindings: Vec<OwnerBinding>,
}

impl OwnerRegistry {
    pub fn empty(generation: u64) -> Self {
        Self {
            schema_version: OWNER_REGISTRY_SCHEMA.to_owned(),
            generation,
            bindings: Vec::new(),
        }
    }
}

/// Validate the SID string as DATA (not as proof of a Windows principal).
/// Accepts the canonical `S-1-<authority>-<subauthority>...` decimal form.
pub fn is_well_formed_sid(value: &str) -> bool {
    let parts: Vec<&str> = value.split('-').collect();
    if parts.len() < 4 || parts[0] != "S" || parts[1] != "1" {
        return false;
    }
    parts[2..]
        .iter()
        .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Validate the complete registry document before any trust decision.
pub fn validate_registry(registry: &OwnerRegistry) -> Result<()> {
    if registry.schema_version != OWNER_REGISTRY_SCHEMA {
        bail!(
            "unsupported owner registry schema {} (expected {})",
            registry.schema_version,
            OWNER_REGISTRY_SCHEMA
        );
    }
    if registry.generation == 0 {
        bail!("owner registry generation must be greater than zero");
    }
    let mut seen = std::collections::BTreeSet::new();
    for (index, binding) in registry.bindings.iter().enumerate() {
        if !is_well_formed_sid(&binding.sid) {
            bail!("binding {index} has a malformed SID");
        }
        if binding.owner.trim().is_empty() {
            bail!("binding {index} has an empty owner");
        }
        if binding.revision == 0 {
            bail!("binding {index} has a zero revision");
        }
        if !seen.insert(binding.sid.as_str()) {
            bail!("duplicate SID binding at index {index}");
        }
    }
    Ok(())
}

/// Resolve a SID to its owner. Revoked bindings are a permanent miss; this
/// function is called on every request and intentionally has no cache.
pub fn lookup_owner(registry: &OwnerRegistry, sid: &str) -> Result<Option<String>> {
    validate_registry(registry)?;
    if !is_well_formed_sid(sid) {
        bail!("cannot resolve a malformed SID");
    }
    Ok(registry
        .bindings
        .iter()
        .find(|binding| binding.sid == sid && !binding.revoked)
        .map(|binding| binding.owner.clone()))
}

/// Serialize deterministically so provisioning and verification agree byte-for-byte.
pub fn canonical_json(registry: &OwnerRegistry) -> Result<Vec<u8>> {
    validate_registry(registry)?;
    serde_json::to_vec(registry).context("serialize owner registry")
}

/// Machine-local absolute path for the owner registry.
#[cfg(windows)]
pub fn programdata_root() -> Result<PathBuf> {
    std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .context("ProgramData is unavailable or not absolute; refuse to provision")
}

#[cfg(windows)]
pub fn default_registry_path() -> Result<PathBuf> {
    Ok(programdata_root()?.join(PROGRAMDATA_RELATIVE_PATH.replace('/', "\\")))
}

#[cfg(not(windows))]
pub fn default_registry_path() -> Result<PathBuf> {
    // No trusted Windows provisioning boundary exists off Windows. Fail closed.
    bail!("trusted owner registry provisioning is Windows-only")
}

/// Reason a provisioning attempt was refused. Kept as data so callers cannot
/// accidentally treat a refusal as a successful write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    ElevatedAuthorityRequired,
    UntrustedLocation,
    UnverifiableAcl,
    UnsafePath,
    InvalidRegistry,
    IoFailure,
}

impl Refusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ElevatedAuthorityRequired => {
                "elevated Windows administrator/SYSTEM authority required"
            }
            Self::UntrustedLocation => "registry location is not the protected machine-local path",
            Self::UnverifiableAcl => "owner or DACL could not be verified",
            Self::UnsafePath => "unsafe reparse/junction/replacement path",
            Self::InvalidRegistry => "invalid owner registry document",
            Self::IoFailure => "registry I/O failure",
        }
    }
}

pub type RegistryResult<T> = std::result::Result<T, Refusal>;

#[cfg(windows)]
pub mod windows {
    use super::*;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;
    use winapi::ctypes::c_void;
    use winapi::shared::minwindef::{BOOL, DWORD, FALSE, LPVOID, TRUE};
    use winapi::shared::sddl::{ConvertSidToStringSidW, ConvertStringSidToSidW};
    use winapi::shared::winerror::ERROR_SUCCESS;
    use winapi::um::accctrl::SE_FILE_OBJECT;
    use winapi::um::aclapi::GetNamedSecurityInfoW;
    use winapi::um::fileapi::GetFileAttributesW;
    use winapi::um::handleapi::CloseHandle;
    use winapi::um::processthreadsapi::{GetCurrentProcess, OpenProcessToken};
    use winapi::um::securitybaseapi::{
        CheckTokenMembership, GetAce, GetAclInformation, GetTokenInformation, IsValidAcl,
    };
    use winapi::um::winbase::LocalFree;
    use winapi::um::winnt::{
        ACCESS_ALLOWED_ACE, ACCESS_ALLOWED_ACE_TYPE, ACE_HEADER, ACL_SIZE_INFORMATION,
        AclSizeInformation, DACL_SECURITY_INFORMATION, FILE_ATTRIBUTE_DIRECTORY,
        FILE_ATTRIBUTE_REPARSE_POINT, GENERIC_WRITE, HANDLE, OWNER_SECURITY_INFORMATION, PACL, SID,
        TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation, WRITE_DAC, WRITE_OWNER,
    };

    const INVALID_FILE_ATTRIBUTES: DWORD = 0xFFFF_FFFF;

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    fn sid_to_string(sid: *mut SID) -> RegistryResult<String> {
        if sid.is_null() {
            return Err(Refusal::UnverifiableAcl);
        }
        let mut text: *mut u16 = ptr::null_mut();
        let ok = unsafe { ConvertSidToStringSidW(sid as *mut c_void, &mut text) };
        if ok != TRUE || text.is_null() {
            return Err(Refusal::UnverifiableAcl);
        }
        let result = unsafe {
            let len = (0..).take_while(|&i| *text.offset(i) != 0).count();
            String::from_utf16_lossy(std::slice::from_raw_parts(text, len))
        };
        unsafe { LocalFree(text as LPVOID) };
        Ok(result)
    }

    fn well_known_sid(text: &str) -> RegistryResult<*mut c_void> {
        let mut wide_text: Vec<u16> = OsStr::new(text)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut sid: *mut c_void = ptr::null_mut();
        let ok = unsafe { ConvertStringSidToSidW(wide_text.as_mut_ptr(), &mut sid) };
        if ok != TRUE || sid.is_null() {
            return Err(Refusal::UnverifiableAcl);
        }
        Ok(sid)
    }

    /// Prove that the CURRENT PROCESS holds real elevated administrator or
    /// SYSTEM authority. This is checked before any write or replacement.
    pub fn require_provisioning_authority() -> RegistryResult<String> {
        let process = unsafe { GetCurrentProcess() };
        let mut token: HANDLE = ptr::null_mut();
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } != TRUE {
            return Err(Refusal::ElevatedAuthorityRequired);
        }
        struct TokenGuard(HANDLE);
        impl Drop for TokenGuard {
            fn drop(&mut self) {
                unsafe { CloseHandle(self.0) };
            }
        }
        let _guard = TokenGuard(token);

        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut returned: DWORD = 0;
        let ok = unsafe {
            GetTokenInformation(
                token,
                TokenElevation,
                &mut elevation as *mut _ as LPVOID,
                std::mem::size_of::<TOKEN_ELEVATION>() as DWORD,
                &mut returned,
            )
        };
        if ok != TRUE || returned as usize != std::mem::size_of::<TOKEN_ELEVATION>() {
            return Err(Refusal::ElevatedAuthorityRequired);
        }
        if elevation.TokenIsElevated != 1 {
            return Err(Refusal::ElevatedAuthorityRequired);
        }

        // Elevation alone is not proof of the intended authority: require
        // explicit Administrators membership or the SYSTEM SID.
        let admins = well_known_sid(SID_ADMINISTRATORS)?;
        let mut is_admin: BOOL = FALSE;
        let membership =
            unsafe { CheckTokenMembership(ptr::null_mut(), admins as *mut c_void, &mut is_admin) };
        unsafe { LocalFree(admins as LPVOID) };
        if membership != TRUE {
            return Err(Refusal::ElevatedAuthorityRequired);
        }
        if is_admin == TRUE {
            return Ok(SID_ADMINISTRATORS.to_owned());
        }
        let system = well_known_sid(SID_SYSTEM)?;
        let mut is_system: BOOL = FALSE;
        let membership =
            unsafe { CheckTokenMembership(ptr::null_mut(), system as *mut c_void, &mut is_system) };
        unsafe { LocalFree(system as LPVOID) };
        if membership == TRUE && is_system == TRUE {
            return Ok(SID_SYSTEM.to_owned());
        }
        Err(Refusal::ElevatedAuthorityRequired)
    }

    /// Reject reparse points on the target and every existing ancestor. A
    /// junction or symlink can silently redirect a validated path elsewhere.
    /// This is path-component inspection only; trust boundaries are enforced
    /// separately by `validate_registry_location`.
    pub fn reject_reparse(path: &Path) -> RegistryResult<()> {
        let mut current: &Path = path;
        loop {
            let attrs = unsafe { GetFileAttributesW(wide(current.as_os_str()).as_ptr()) };
            if attrs != INVALID_FILE_ATTRIBUTES && attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return Err(Refusal::UnsafePath);
            }
            match current.parent() {
                Some(parent) if !parent.as_os_str().is_empty() => current = parent,
                _ => break,
            }
        }
        Ok(())
    }

    fn is_trusted_ace_sid(sid: &str) -> bool {
        sid == SID_ADMINISTRATORS || sid == SID_SYSTEM
    }

    /// Validate the file's OWNER and effective DACL. A NULL/empty DACL, a
    /// non-administrator owner, or any allow ACE for an untrusted SID is fatal.
    pub fn validate_file_security(path: &Path) -> RegistryResult<()> {
        if !path.is_absolute() {
            return Err(Refusal::UntrustedLocation);
        }
        let wpath = wide(path.as_os_str());
        let mut owner: *mut c_void = ptr::null_mut();
        let mut group: *mut c_void = ptr::null_mut();
        let mut dacl: PACL = ptr::null_mut();
        let mut sacl: PACL = ptr::null_mut();
        let mut descriptor: *mut c_void = ptr::null_mut();
        let status = unsafe {
            GetNamedSecurityInfoW(
                wpath.as_ptr(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                &mut group,
                &mut dacl,
                &mut sacl,
                &mut descriptor,
            )
        };
        if status != ERROR_SUCCESS || descriptor.is_null() {
            return Err(Refusal::UnverifiableAcl);
        }
        struct DescriptorGuard(*mut c_void);
        impl Drop for DescriptorGuard {
            fn drop(&mut self) {
                unsafe { LocalFree(self.0) };
            }
        }
        let _guard = DescriptorGuard(descriptor);

        let owner_sid = sid_to_string(owner as *mut SID)?;
        if !is_trusted_ace_sid(&owner_sid) {
            return Err(Refusal::UnverifiableAcl);
        }
        if dacl.is_null() || unsafe { IsValidAcl(dacl) } != TRUE {
            return Err(Refusal::UnverifiableAcl);
        }
        let mut info: ACL_SIZE_INFORMATION = unsafe { std::mem::zeroed() };
        let ok = unsafe {
            GetAclInformation(
                dacl,
                &mut info as *mut _ as LPVOID,
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as DWORD,
                AclSizeInformation,
            )
        };
        if ok != TRUE {
            return Err(Refusal::UnverifiableAcl);
        }
        if info.AceCount == 0 {
            return Err(Refusal::UnverifiableAcl);
        }
        for index in 0..info.AceCount {
            let mut ace: LPVOID = ptr::null_mut();
            if unsafe { GetAce(dacl, index, &mut ace) } != TRUE || ace.is_null() {
                return Err(Refusal::UnverifiableAcl);
            }
            let header = unsafe { &*(ace as *const ACE_HEADER) };
            if header.AceType != ACCESS_ALLOWED_ACE_TYPE {
                // Deny ACEs and audit/callback ACEs are not part of the
                // reviewed minimal policy; refuse rather than reason about them.
                return Err(Refusal::UnverifiableAcl);
            }
            let allowed = unsafe { &*(ace as *const ACCESS_ALLOWED_ACE) };
            if allowed.Mask & (GENERIC_WRITE | WRITE_DAC | WRITE_OWNER) != 0 {
                let ace_sid = sid_to_string((&allowed.SidStart as *const DWORD) as *mut SID)?;
                if !is_trusted_ace_sid(&ace_sid) {
                    return Err(Refusal::UnverifiableAcl);
                }
            }
            // Read-only grants to the restricted future service are permitted
            // only when they carry no write/DACL/OWNER bits.
        }
        Ok(())
    }

    /// Full location policy: exact ProgramData path, absolute, no reparse on
    /// any component, and strict ACLs on the file and every existing ancestor.
    pub fn validate_registry_location(path: &Path) -> RegistryResult<()> {
        let expected = default_registry_path().map_err(|_| Refusal::UntrustedLocation)?;
        if !path.is_absolute() || path != expected.as_path() {
            return Err(Refusal::UntrustedLocation);
        }
        for component in path.components() {
            if matches!(
                component,
                Component::Prefix(_) | Component::RootDir | Component::Normal(_)
            ) {
                continue;
            }
            return Err(Refusal::UntrustedLocation); // ParentDir / CurDir components
        }
        let root = programdata_root().map_err(|_| Refusal::UntrustedLocation)?;
        reject_reparse(path)?;
        let mut current: &Path = path;
        loop {
            if current.exists() {
                let attrs = unsafe { GetFileAttributesW(wide(current.as_os_str()).as_ptr()) };
                if attrs == INVALID_FILE_ATTRIBUTES {
                    return Err(Refusal::UnverifiableAcl);
                }
                if current.is_dir() && attrs & FILE_ATTRIBUTE_DIRECTORY == 0 {
                    return Err(Refusal::UnsafePath);
                }
                // ProgramData itself is OS-managed; the protected subtree
                // beneath it must verify at every level.
                if current != root.as_path() {
                    validate_file_security(current)?;
                }
            }
            if current == root.as_path() {
                break;
            }
            match current.parent() {
                Some(parent) if !parent.as_os_str().is_empty() => current = parent,
                _ => return Err(Refusal::UntrustedLocation),
            }
        }
        Ok(())
    }

    /// Administrator-only provisioning. Performs no write unless real elevated
    /// authority and the protected location both verify first.
    pub fn provision_registry(path: &Path, registry: &OwnerRegistry) -> RegistryResult<()> {
        require_provisioning_authority()?;
        validate_registry_location(path)?;
        let bytes = canonical_json(registry).map_err(|_| Refusal::InvalidRegistry)?;
        let parent = path.parent().ok_or(Refusal::UntrustedLocation)?;
        let temp = parent.join("registry.json.tmp");
        std::fs::write(&temp, &bytes).map_err(|_| Refusal::IoFailure)?;
        // Re-validate the temporary file before it can replace the binding.
        validate_file_security(&temp)?;
        std::fs::rename(&temp, path).map_err(|_| Refusal::IoFailure)?;
        // Post-condition: the installed registry must still pass full policy.
        validate_registry_location(path)
    }

    /// Trusted read. No cache: revocation is observed on every call.
    pub fn load_registry(path: &Path) -> RegistryResult<OwnerRegistry> {
        validate_registry_location(path)?;
        let bytes = std::fs::read(path).map_err(|_| Refusal::IoFailure)?;
        let registry: OwnerRegistry =
            serde_json::from_slice(&bytes).map_err(|_| Refusal::InvalidRegistry)?;
        validate_registry(&registry).map_err(|_| Refusal::InvalidRegistry)?;
        Ok(registry)
    }

    pub fn resolve_owner(path: &Path, sid: &str) -> RegistryResult<Option<String>> {
        let registry = load_registry(path)?;
        lookup_owner(&registry, sid).map_err(|_| Refusal::InvalidRegistry)
    }

    /// Fail-closed helper for tests and callers on non-Windows targets.
    pub fn refuse_off_windows() -> Refusal {
        Refusal::ElevatedAuthorityRequired
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(sid: &str, owner: &str, revoked: bool) -> OwnerBinding {
        OwnerBinding {
            sid: sid.to_owned(),
            owner: owner.to_owned(),
            revoked,
            revision: 1,
        }
    }

    #[test]
    fn well_formed_sid_format_only() {
        assert!(is_well_formed_sid(SID_SYSTEM));
        assert!(is_well_formed_sid(SID_ADMINISTRATORS));
        assert!(!is_well_formed_sid("S-1-5-admin"));
        assert!(!is_well_formed_sid("S-2-5-18"));
        assert!(!is_well_formed_sid("diegorhoger"));
    }

    #[test]
    fn valid_registry_and_owner_lookup() {
        let registry = OwnerRegistry {
            schema_version: OWNER_REGISTRY_SCHEMA.to_owned(),
            generation: 1,
            bindings: vec![binding(SID_ADMINISTRATORS, "owner-a", false)],
        };
        assert!(validate_registry(&registry).is_ok());
        assert_eq!(
            lookup_owner(&registry, SID_ADMINISTRATORS)
                .unwrap()
                .as_deref(),
            Some("owner-a")
        );
        assert_eq!(lookup_owner(&registry, SID_SYSTEM).unwrap(), None);
    }

    #[test]
    fn revoked_binding_is_a_permanent_miss() {
        let registry = OwnerRegistry {
            schema_version: OWNER_REGISTRY_SCHEMA.to_owned(),
            generation: 2,
            bindings: vec![binding(SID_ADMINISTRATORS, "owner-a", true)],
        };
        assert_eq!(lookup_owner(&registry, SID_ADMINISTRATORS).unwrap(), None);
    }

    #[test]
    fn bad_schema_duplicate_sid_and_empty_owner_are_refused() {
        let mut registry = OwnerRegistry::empty(1);
        registry.schema_version = "other-v1".to_owned();
        assert!(validate_registry(&registry).is_err());

        let mut registry = OwnerRegistry::empty(1);
        registry.bindings = vec![
            binding(SID_SYSTEM, "a", false),
            binding(SID_SYSTEM, "b", false),
        ];
        assert!(validate_registry(&registry).is_err());

        let mut registry = OwnerRegistry::empty(1);
        registry.bindings = vec![binding(SID_SYSTEM, "   ", false)];
        assert!(validate_registry(&registry).is_err());

        assert!(validate_registry(&OwnerRegistry::empty(0)).is_err());
    }

    #[test]
    fn malformed_sid_is_never_resolved() {
        let registry = OwnerRegistry::empty(1);
        assert!(lookup_owner(&registry, "not-a-sid").is_err());
    }

    #[test]
    fn canonical_json_is_deterministic() {
        let registry = OwnerRegistry {
            schema_version: OWNER_REGISTRY_SCHEMA.to_owned(),
            generation: 7,
            bindings: vec![binding(SID_SYSTEM, "a", false)],
        };
        assert_eq!(
            canonical_json(&registry).unwrap(),
            canonical_json(&registry).unwrap()
        );
    }

    #[test]
    fn off_windows_has_no_trusted_provisioner() {
        #[cfg(not(windows))]
        assert!(default_registry_path().is_err());
        assert_eq!(
            windows::refuse_off_windows(),
            Refusal::ElevatedAuthorityRequired
        );
    }

    #[cfg(windows)]
    #[test]
    fn relative_location_is_refused() {
        assert_eq!(
            windows::validate_registry_location(Path::new("registry.json")),
            Err(Refusal::UntrustedLocation)
        );
    }

    #[cfg(windows)]
    #[test]
    fn missing_file_security_cannot_be_verified() {
        let missing = std::env::temp_dir().join(format!(
            "prometheos-owner-registry-missing-{}.json",
            std::process::id()
        ));
        assert_eq!(
            windows::validate_file_security(&missing),
            Err(Refusal::UnverifiableAcl)
        );
    }

    #[cfg(windows)]
    #[test]
    fn ordinary_user_file_is_not_a_trusted_registry() {
        // A user-owned temp file has neither an administrator owner nor the
        // required restricted DACL. Real Windows ACL inspection must refuse it.
        let path = std::env::temp_dir().join(format!(
            "prometheos-owner-registry-ordinary-{}.json",
            std::process::id()
        ));
        std::fs::write(&path, b"{}").expect("create temp file");
        let result = windows::validate_file_security(&path);
        let _ = std::fs::remove_file(&path);
        assert_eq!(result, Err(Refusal::UnverifiableAcl));
    }

    #[cfg(windows)]
    #[test]
    fn reparse_point_or_junction_is_detected() {
        let dir = std::env::temp_dir().join(format!(
            "prometheos-owner-registry-reparse-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let target = dir.join("target");
        std::fs::create_dir_all(&target).expect("create target dir");
        let link = dir.join("link");
        let created = std::os::windows::fs::symlink_dir(&target, &link);
        if created.is_err() {
            eprintln!("SKIP: junction creation unavailable; reparse acceptance NOT RUN");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        assert_eq!(
            windows::reject_reparse(&link.join("registry.json")),
            Err(Refusal::UnsafePath)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn world_writable_directory_is_refused() {
        let dir = std::env::temp_dir().join(format!(
            "prometheos-owner-registry-world-{}-{}",
            std::process::id(),
            OWNER_REGISTRY_SCHEMA.len()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        // Real Windows ACL mutation via icacls, not a simulated permission flag.
        let status = std::process::Command::new("icacls")
            .arg(&dir)
            .arg("/grant")
            .arg("Everyone:(OI)(CI)F")
            .output();
        let Ok(output) = status else {
            eprintln!("SKIP: icacls unavailable; Windows ACL negative test NOT RUN");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        };
        if !output.status.success() {
            eprintln!("SKIP: icacls could not modify test ACL; Windows ACL negative test NOT RUN");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let result = windows::validate_file_security(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(result, Err(Refusal::UnverifiableAcl));
    }

    #[cfg(windows)]
    #[test]
    fn unprivileged_provisioning_is_denied() {
        // This test environment is not elevated. If it ever is, skip rather
        // than write to a real protected location.
        if windows::require_provisioning_authority().is_ok() {
            eprintln!("SKIP: elevated test host; privileged provisioning acceptance NOT RUN");
            return;
        }
        let registry = OwnerRegistry::empty(1);
        let result =
            windows::provision_registry(Path::new("C:\\nonexistent\\registry.json"), &registry);
        assert_eq!(result, Err(Refusal::ElevatedAuthorityRequired));
    }
}
