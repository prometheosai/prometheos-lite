// Copyright (c) 2026 Memento Mori Labs LLC. All rights reserved.
// Proprietary. Trusted Windows SID-to-Lite-owner provisioning (issue #340).
//
// STAGE 1 ONLY: a versioned, machine-local, administrator/SYSTEM-controlled
// owner binding registry. No transport, client enrollment, credential issuer
// or execution authority is added here.
//
// Security invariants (repair dispatch 055):
//  - all Windows checks are bound to ONE explicitly opened process token;
//  - ACL evaluation maps generic and specific rights, including directory
//    replacement rights; unknown ACE forms fail closed;
//  - reads and writes use verified OS handles, not re-resolved paths;
//  - updates are monotonic and conflict-aware (no rollback, no revocation
//    reversal, no owner change, no concurrent overwrite);
//  - the ProgramData root comes from the Known Folder API, never the process
//    environment alone.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

pub const OWNER_REGISTRY_SCHEMA: &str = "prometheos-owner-registry-v1";
pub const PROGRAMDATA_RELATIVE_PATH: &str = "PrometheOS/Lite/owner-registry/registry.json";

pub const SID_SYSTEM: &str = "S-1-5-18";
pub const SID_ADMINISTRATORS: &str = "S-1-5-32-544";
pub const SID_EVERYONE: &str = "S-1-1-0";
pub const SID_AUTHENTICATED_USERS: &str = "S-1-5-11";
pub const SID_USERS: &str = "S-1-5-32-545";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OwnerBinding {
    pub sid: String,
    pub owner: String,
    pub revoked: bool,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OwnerRegistry {
    pub schema_version: String,
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

/// Data-format check only. Windows structural validation is `windows::validate_sid`.
pub fn is_well_formed_sid(value: &str) -> bool {
    let parts: Vec<&str> = value.split('-').collect();
    if parts.len() < 4 || parts[0] != "S" || parts[1] != "1" {
        return false;
    }
    parts[2..]
        .iter()
        .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

pub fn validate_registry(registry: &OwnerRegistry) -> Result<()> {
    if registry.schema_version != OWNER_REGISTRY_SCHEMA {
        bail!(
            "unsupported owner registry schema {}",
            registry.schema_version
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

/// Atomic, monotonic update rule. Any violation is a hard refusal.
pub fn enforce_monotonic_update(
    previous: Option<&OwnerRegistry>,
    proposed: &OwnerRegistry,
) -> Result<()> {
    validate_registry(proposed)?;
    let Some(previous) = previous else {
        return Ok(());
    };
    validate_registry(previous)?;
    if proposed.generation <= previous.generation {
        bail!("registry generation must strictly increase");
    }
    let old: BTreeMap<&str, &OwnerBinding> = previous
        .bindings
        .iter()
        .map(|b| (b.sid.as_str(), b))
        .collect();
    for binding in &proposed.bindings {
        if let Some(before) = old.get(binding.sid.as_str()) {
            if before.revoked && !binding.revoked {
                bail!("a revoked binding cannot be reactivated");
            }
            if before.owner != binding.owner {
                bail!("an existing SID's owner cannot change");
            }
            if binding.revision < before.revision {
                bail!("binding revision cannot decrease");
            }
            if binding.revoked != before.revoked && binding.revision == before.revision {
                bail!("a revocation change requires a strictly greater revision");
            }
        }
    }
    Ok(())
}

pub fn lookup_owner(registry: &OwnerRegistry, sid: &str) -> Result<Option<String>> {
    validate_registry(registry)?;
    if !is_well_formed_sid(sid) {
        bail!("cannot resolve a malformed SID");
    }
    Ok(registry
        .bindings
        .iter()
        .find(|b| b.sid == sid && !b.revoked)
        .map(|b| b.owner.clone()))
}

pub fn canonical_json(registry: &OwnerRegistry) -> Result<Vec<u8>> {
    validate_registry(registry)?;
    serde_json::to_vec(registry).context("serialize owner registry")
}

#[cfg(windows)]
pub fn programdata_root() -> Result<PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use winapi::shared::winerror::S_OK;
    use winapi::um::combaseapi::CoTaskMemFree;
    use winapi::um::knownfolders::FOLDERID_ProgramData;
    use winapi::um::shlobj::SHGetKnownFolderPath;

    let mut raw: *mut u16 = std::ptr::null_mut();
    let hr =
        unsafe { SHGetKnownFolderPath(&FOLDERID_ProgramData, 0, std::ptr::null_mut(), &mut raw) };
    if hr != S_OK || raw.is_null() {
        bail!("ProgramData Known Folder could not be resolved");
    }
    let path = unsafe {
        let len = (0..).take_while(|&i| *raw.offset(i) != 0).count();
        PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(raw, len)))
    };
    unsafe { CoTaskMemFree(raw as *mut winapi::ctypes::c_void) };
    if !path.is_absolute() {
        bail!("ProgramData Known Folder is not absolute");
    }
    let env_root = std::env::var_os("ProgramData").map(PathBuf::from);
    if !known_folder_matches_environment(&path, env_root.as_deref()) {
        bail!("poisoned ProgramData environment does not match the OS Known Folder");
    }
    Ok(path)
}

/// Pure comparison rule: a caller-controlled environment value is allowed only
/// when it agrees exactly (case-insensitively) with the OS Known Folder root.
pub fn known_folder_matches_environment(os_root: &Path, env_root: Option<&Path>) -> bool {
    match env_root {
        Some(env) => env
            .to_string_lossy()
            .eq_ignore_ascii_case(&os_root.to_string_lossy()),
        None => true,
    }
}

#[cfg(windows)]
pub fn default_registry_path() -> Result<PathBuf> {
    Ok(programdata_root()?.join(PROGRAMDATA_RELATIVE_PATH.replace('/', "\\")))
}

#[cfg(not(windows))]
pub fn default_registry_path() -> Result<PathBuf> {
    bail!("trusted owner registry provisioning is Windows-only")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    ElevatedAuthorityRequired,
    UntrustedLocation,
    UnverifiableAcl,
    UnsafePath,
    InvalidRegistry,
    IoFailure,
    NonMonotonicUpdate,
    ConcurrentUpdate,
    ImpersonationDetected,
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
            Self::NonMonotonicUpdate => "non-monotonic or revocation-reversing update",
            Self::ConcurrentUpdate => "concurrent provisioning conflict",
            Self::ImpersonationDetected => "thread impersonation is not a provisioning identity",
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
    use winapi::um::aclapi::GetSecurityInfo;
    use winapi::um::fileapi::{
        BY_HANDLE_FILE_INFORMATION, CREATE_NEW, CreateFileW, GetFileInformationByHandle,
        GetFileType, OPEN_EXISTING, ReadFile, WriteFile,
    };
    use winapi::um::handleapi::CloseHandle;
    use winapi::um::processthreadsapi::{
        GetCurrentProcess, GetCurrentThread, OpenProcessToken, OpenThreadToken,
    };
    use winapi::um::securitybaseapi::{
        CheckTokenMembership, DuplicateToken, GetAce, GetAclInformation, GetLengthSid,
        GetSidSubAuthorityCount, GetTokenInformation, IsValidAcl, IsValidSid,
    };
    use winapi::um::winbase::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_TYPE_DISK, LocalFree,
    };
    use winapi::um::winnt::{
        ACCESS_ALLOWED_ACE, ACCESS_ALLOWED_ACE_TYPE, ACE_HEADER, ACL_SIZE_INFORMATION,
        AclSizeInformation, DACL_SECURITY_INFORMATION, DELETE, FILE_ALL_ACCESS, FILE_APPEND_DATA,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_DELETE_CHILD, FILE_SHARE_READ, FILE_WRITE_ATTRIBUTES,
        FILE_WRITE_DATA, FILE_WRITE_EA, GENERIC_ALL, GENERIC_READ, GENERIC_WRITE, HANDLE,
        OWNER_SECURITY_INFORMATION, PACL, READ_CONTROL, SID, SecurityImpersonation,
        TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_USER, TokenElevation, TokenUser, WRITE_DAC,
        WRITE_OWNER,
    };

    const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;

    struct HandleGuard(HANDLE);
    impl Drop for HandleGuard {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    fn sid_to_string(sid: *mut SID) -> RegistryResult<String> {
        if sid.is_null() || unsafe { IsValidSid(sid as *mut c_void) } != TRUE {
            return Err(Refusal::UnverifiableAcl);
        }
        let mut text: *mut u16 = ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(sid as *mut c_void, &mut text) } != TRUE
            || text.is_null()
        {
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
        if unsafe { ConvertStringSidToSidW(wide_text.as_mut_ptr(), &mut sid) } != TRUE
            || sid.is_null()
        {
            return Err(Refusal::UnverifiableAcl);
        }
        Ok(sid)
    }

    /// Structural SID validation using Windows SID APIs, not decimal grammar.
    pub fn validate_sid(text: &str) -> RegistryResult<()> {
        let sid = well_known_sid(text)?;
        struct SidGuard(*mut c_void);
        impl Drop for SidGuard {
            fn drop(&mut self) {
                unsafe { LocalFree(self.0) };
            }
        }
        let _guard = SidGuard(sid);
        if unsafe { IsValidSid(sid) } != TRUE {
            return Err(Refusal::InvalidRegistry);
        }
        let typed = sid as *mut SID;
        let revision = unsafe { (*typed).Revision };
        let count = unsafe { GetSidSubAuthorityCount(sid) };
        if revision != 1 || count.is_null() || unsafe { *count } == 0 || unsafe { *count } > 15 {
            return Err(Refusal::InvalidRegistry);
        }
        if unsafe { GetLengthSid(sid) } < 12 {
            return Err(Refusal::InvalidRegistry);
        }
        let canonical = sid_to_string(sid as *mut SID)?;
        if !canonical.eq_ignore_ascii_case(text) {
            return Err(Refusal::InvalidRegistry);
        }
        Ok(())
    }

    /// Full Windows access-right evaluation: generic rights are expanded,
    /// specific write/delete/take-ownership/directory-replacement rights are
    /// rejected for untrusted SIDs. Unknown ACE forms fail closed.
    fn ace_is_dangerous(mask: DWORD) -> bool {
        let generic_all = mask & GENERIC_ALL != 0 || mask & FILE_ALL_ACCESS == FILE_ALL_ACCESS;
        let generic_write = mask & GENERIC_WRITE != 0;
        let specific = mask
            & (DELETE
                | WRITE_DAC
                | WRITE_OWNER
                | FILE_DELETE_CHILD
                | FILE_WRITE_DATA
                | FILE_APPEND_DATA
                | FILE_WRITE_ATTRIBUTES
                | FILE_WRITE_EA);
        generic_all || generic_write || specific != 0
    }

    fn trusted_sid(sid: &str) -> bool {
        sid.eq_ignore_ascii_case(SID_ADMINISTRATORS) || sid.eq_ignore_ascii_case(SID_SYSTEM)
    }

    fn validate_acl(acl: PACL) -> RegistryResult<()> {
        if acl.is_null() || unsafe { IsValidAcl(acl) } != TRUE {
            return Err(Refusal::UnverifiableAcl);
        }
        let mut info: ACL_SIZE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe {
            GetAclInformation(
                acl,
                &mut info as *mut _ as LPVOID,
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as DWORD,
                AclSizeInformation,
            )
        } != TRUE
            || info.AceCount == 0
        {
            return Err(Refusal::UnverifiableAcl);
        }
        for index in 0..info.AceCount {
            let mut ace: LPVOID = ptr::null_mut();
            if unsafe { GetAce(acl, index, &mut ace) } != TRUE || ace.is_null() {
                return Err(Refusal::UnverifiableAcl);
            }
            let header = unsafe { &*(ace as *const ACE_HEADER) };
            if header.AceType != ACCESS_ALLOWED_ACE_TYPE {
                return Err(Refusal::UnverifiableAcl);
            }
            let allowed = unsafe { &*(ace as *const ACCESS_ALLOWED_ACE) };
            let sid = sid_to_string((&allowed.SidStart as *const DWORD) as *mut SID)?;
            if ace_is_dangerous(allowed.Mask) && !trusted_sid(&sid) {
                return Err(Refusal::UnverifiableAcl);
            }
        }
        Ok(())
    }

    fn validate_handle_security(handle: HANDLE) -> RegistryResult<()> {
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(handle, &mut info) } != TRUE {
            return Err(Refusal::UnverifiableAcl);
        }
        if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(Refusal::UnsafePath);
        }
        if unsafe { GetFileType(handle) } != FILE_TYPE_DISK {
            return Err(Refusal::UnsafePath);
        }
        let mut owner: *mut c_void = ptr::null_mut();
        let mut group: *mut c_void = ptr::null_mut();
        let mut dacl: PACL = ptr::null_mut();
        let mut sacl: PACL = ptr::null_mut();
        let mut descriptor: *mut c_void = ptr::null_mut();
        let status = unsafe {
            GetSecurityInfo(
                handle,
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
        if !trusted_sid(&sid_to_string(owner as *mut SID)?) {
            return Err(Refusal::UnverifiableAcl);
        }
        validate_acl(dacl)
    }

    fn open_verified(path: &Path, write: bool, create_new: bool) -> RegistryResult<HANDLE> {
        let desired = GENERIC_READ | READ_CONTROL | if write { GENERIC_WRITE } else { 0 };
        let share = if write { 0 } else { FILE_SHARE_READ };
        let creation = if create_new {
            CREATE_NEW
        } else {
            OPEN_EXISTING
        };
        let handle = unsafe {
            CreateFileW(
                wide(path.as_os_str()).as_ptr(),
                desired,
                share,
                ptr::null_mut(),
                creation,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE || handle.is_null() {
            return Err(Refusal::IoFailure);
        }
        if let Err(refusal) = validate_handle_security(handle) {
            unsafe { CloseHandle(handle) };
            return Err(refusal);
        }
        Ok(handle)
    }

    /// One token, one identity: a process token is opened explicitly and every
    /// check (elevation, membership, SYSTEM) is bound to that same token.
    /// Thread impersonation is refused as a provisioning identity.
    pub fn require_provisioning_authority() -> RegistryResult<String> {
        let mut thread_token: HANDLE = ptr::null_mut();
        let impersonating =
            unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, TRUE, &mut thread_token) }
                == TRUE;
        if impersonating {
            if !thread_token.is_null() {
                unsafe { CloseHandle(thread_token) };
            }
            return Err(Refusal::ImpersonationDetected);
        }
        let mut token: HANDLE = ptr::null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } != TRUE {
            return Err(Refusal::ElevatedAuthorityRequired);
        }
        let _guard = HandleGuard(token);
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut returned: DWORD = 0;
        if unsafe {
            GetTokenInformation(
                token,
                TokenElevation,
                &mut elevation as *mut _ as LPVOID,
                std::mem::size_of::<TOKEN_ELEVATION>() as DWORD,
                &mut returned,
            )
        } != TRUE
            || elevation.TokenIsElevated != 1
        {
            return Err(Refusal::ElevatedAuthorityRequired);
        }
        let mut user_buf = [0u8; std::mem::size_of::<TOKEN_USER>() + 128];
        if unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                user_buf.as_mut_ptr() as LPVOID,
                user_buf.len() as DWORD,
                &mut returned,
            )
        } != TRUE
        {
            return Err(Refusal::ElevatedAuthorityRequired);
        }
        let user = unsafe { &*(user_buf.as_ptr() as *const TOKEN_USER) };
        let process_sid = sid_to_string(user.User.Sid as *mut SID)?;
        // CheckTokenMembership needs an impersonation token. Duplicate the
        // SAME process token so elevation, user SID and membership all come
        // from one verified identity, never a different thread token.
        let mut impersonation: HANDLE = ptr::null_mut();
        if unsafe { DuplicateToken(token, SecurityImpersonation, &mut impersonation) } != TRUE {
            return Err(Refusal::ElevatedAuthorityRequired);
        }
        let _impersonation_guard = HandleGuard(impersonation);
        for candidate in [SID_ADMINISTRATORS, SID_SYSTEM] {
            let sid = well_known_sid(candidate)?;
            let mut member: BOOL = FALSE;
            let ok = unsafe { CheckTokenMembership(impersonation, sid, &mut member) };
            unsafe { LocalFree(sid) };
            if ok == TRUE && member == TRUE {
                return Ok(format!("{candidate} ({process_sid})"));
            }
        }
        Err(Refusal::ElevatedAuthorityRequired)
    }

    /// Verify the exact location and every ancestor through stable handles.
    pub fn validate_registry_location(path: &Path) -> RegistryResult<()> {
        let expected = default_registry_path().map_err(|_| Refusal::UntrustedLocation)?;
        if !path.is_absolute() || path != expected.as_path() {
            return Err(Refusal::UntrustedLocation);
        }
        for component in path.components() {
            if !matches!(
                component,
                Component::Prefix(_) | Component::RootDir | Component::Normal(_)
            ) {
                return Err(Refusal::UntrustedLocation);
            }
        }
        let root = programdata_root().map_err(|_| Refusal::UntrustedLocation)?;
        let handle = open_verified(path, false, false)?;
        let _guard = HandleGuard(handle);
        let mut current: &Path = path;
        loop {
            let dir_handle = open_verified(current, false, false)?;
            let _dir_guard = HandleGuard(dir_handle);
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

    /// Unique, exclusive lock file. Collision is an explicit conflict refusal.
    struct LockGuard {
        path: PathBuf,
        handle: HANDLE,
    }
    impl Drop for LockGuard {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.handle) };
            let _ = std::fs::remove_file(&self.path);
        }
    }
    fn acquire_lock(parent: &Path) -> RegistryResult<LockGuard> {
        let path = parent.join(format!(".registry-{}-{}.lock", std::process::id(), {
            use std::time::{SystemTime, UNIX_EPOCH};
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        }));
        let handle = unsafe {
            CreateFileW(
                wide(path.as_os_str()).as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                ptr::null_mut(),
                CREATE_NEW,
                FILE_FLAG_BACKUP_SEMANTICS,
                ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE || handle.is_null() {
            return Err(Refusal::ConcurrentUpdate);
        }
        Ok(LockGuard { path, handle })
    }

    fn read_all(handle: HANDLE) -> RegistryResult<Vec<u8>> {
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let mut read: DWORD = 0;
            if unsafe {
                ReadFile(
                    handle,
                    chunk.as_mut_ptr() as LPVOID,
                    chunk.len() as DWORD,
                    &mut read,
                    ptr::null_mut(),
                )
            } != TRUE
            {
                return Err(Refusal::IoFailure);
            }
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..read as usize]);
        }
        Ok(bytes)
    }

    fn load_via_handle(path: &Path) -> RegistryResult<OwnerRegistry> {
        let handle = open_verified(path, false, false)?;
        let _guard = HandleGuard(handle);
        let bytes = read_all(handle)?;
        let registry: OwnerRegistry =
            serde_json::from_slice(&bytes).map_err(|_| Refusal::InvalidRegistry)?;
        validate_registry(&registry).map_err(|_| Refusal::InvalidRegistry)?;
        Ok(registry)
    }

    pub fn provision_registry(path: &Path, proposed: &OwnerRegistry) -> RegistryResult<()> {
        require_provisioning_authority()?;
        validate_registry_location(path)?;
        let bytes = canonical_json(proposed).map_err(|_| Refusal::InvalidRegistry)?;
        let parent = path.parent().ok_or(Refusal::UntrustedLocation)?;
        let _lock = acquire_lock(parent)?;
        let previous = match load_via_handle(path) {
            Ok(registry) => Some(registry),
            Err(Refusal::IoFailure) => None,
            Err(other) => return Err(other),
        };
        enforce_monotonic_update(previous.as_ref(), proposed)
            .map_err(|_| Refusal::NonMonotonicUpdate)?;

        let temp = parent.join(format!(".registry-{}-{}.tmp", std::process::id(), {
            use std::time::{SystemTime, UNIX_EPOCH};
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        }));
        let temp_handle = open_verified(&temp, true, true)?;
        {
            let _guard = HandleGuard(temp_handle);
            let mut written: DWORD = 0;
            if unsafe {
                WriteFile(
                    temp_handle,
                    bytes.as_ptr() as *const c_void,
                    bytes.len() as DWORD,
                    &mut written,
                    ptr::null_mut(),
                )
            } != TRUE
                || written as usize != bytes.len()
            {
                let _ = std::fs::remove_file(&temp);
                return Err(Refusal::IoFailure);
            }
        }
        if std::fs::rename(&temp, path).is_err() {
            let _ = std::fs::remove_file(&temp);
            return Err(Refusal::IoFailure);
        }
        validate_registry_location(path)
    }

    pub fn load_registry(path: &Path) -> RegistryResult<OwnerRegistry> {
        validate_registry_location(path)?;
        load_via_handle(path)
    }

    pub fn resolve_owner(path: &Path, sid: &str) -> RegistryResult<Option<String>> {
        let registry = load_registry(path)?;
        lookup_owner(&registry, sid).map_err(|_| Refusal::InvalidRegistry)
    }

    /// Test seam for the pure ACL-rights evaluation (no OS state required).
    pub fn ace_is_dangerous_for_test(mask: DWORD) -> bool {
        ace_is_dangerous(mask)
    }

    /// Reject reparse points on a path and its ancestors (test seam).
    pub fn reject_reparse(path: &Path) -> RegistryResult<()> {
        let mut current = path;
        loop {
            let handle = open_verified(current, false, false)?;
            let _guard = HandleGuard(handle);
            match current.parent() {
                Some(parent) if !parent.as_os_str().is_empty() => current = parent,
                _ => break,
            }
        }
        Ok(())
    }

    pub fn validate_file_security(path: &Path) -> RegistryResult<()> {
        let handle = open_verified(path, false, false)?;
        let _guard = HandleGuard(handle);
        Ok(())
    }
}

#[cfg(not(windows))]
pub fn refuse_off_windows() -> Refusal {
    Refusal::ElevatedAuthorityRequired
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(sid: &str, owner: &str, revoked: bool, revision: u64) -> OwnerBinding {
        OwnerBinding {
            sid: sid.to_owned(),
            owner: owner.to_owned(),
            revoked,
            revision,
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
            bindings: vec![binding(SID_ADMINISTRATORS, "owner-a", false, 1)],
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
            bindings: vec![binding(SID_ADMINISTRATORS, "owner-a", true, 2)],
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
            binding(SID_SYSTEM, "a", false, 1),
            binding(SID_SYSTEM, "b", false, 1),
        ];
        assert!(validate_registry(&registry).is_err());
        let mut registry = OwnerRegistry::empty(1);
        registry.bindings = vec![binding(SID_SYSTEM, "   ", false, 1)];
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
            bindings: vec![binding(SID_SYSTEM, "a", false, 1)],
        };
        assert_eq!(
            canonical_json(&registry).unwrap(),
            canonical_json(&registry).unwrap()
        );
    }

    #[test]
    fn monotonic_update_refuses_rollback_revocation_reversal_and_owner_change() {
        let old = OwnerRegistry {
            schema_version: OWNER_REGISTRY_SCHEMA.to_owned(),
            generation: 7,
            bindings: vec![binding(SID_ADMINISTRATORS, "owner-a", true, 4)],
        };
        let rollback = OwnerRegistry {
            generation: 1,
            ..old.clone()
        };
        assert!(enforce_monotonic_update(Some(&old), &rollback).is_err());
        let mut revive = old.clone();
        revive.generation = 8;
        revive.bindings[0].revoked = false;
        revive.bindings[0].revision = 5;
        assert!(enforce_monotonic_update(Some(&old), &revive).is_err());
        let mut change_owner = old.clone();
        change_owner.generation = 8;
        change_owner.bindings[0].owner = "owner-b".to_owned();
        change_owner.bindings[0].revision = 5;
        assert!(enforce_monotonic_update(Some(&old), &change_owner).is_err());
        let mut lower_revision = old.clone();
        lower_revision.generation = 8;
        lower_revision.bindings[0].revision = 3;
        assert!(enforce_monotonic_update(Some(&old), &lower_revision).is_err());
        let mut valid = old.clone();
        valid.generation = 8;
        valid
            .bindings
            .push(binding(SID_SYSTEM, "owner-b", false, 1));
        assert!(enforce_monotonic_update(Some(&old), &valid).is_ok());
    }

    #[test]
    fn initial_generation_must_be_positive() {
        assert!(enforce_monotonic_update(None, &OwnerRegistry::empty(0)).is_err());
        assert!(enforce_monotonic_update(None, &OwnerRegistry::empty(1)).is_ok());
    }

    #[cfg(not(windows))]
    #[test]
    fn off_windows_has_no_trusted_provisioner() {
        assert!(default_registry_path().is_err());
        assert_eq!(refuse_off_windows(), Refusal::ElevatedAuthorityRequired);
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
    fn old_acl_predicate_would_accept_unsafe_mask() {
        // The repaired implementation refuses these masks. The OLD predicate
        // (mask & (GENERIC_WRITE | WRITE_DAC | WRITE_OWNER)) saw zero and
        // accepted them, so this test fails against the reviewed head.
        let old_considered_safe = |mask: u32| {
            mask & (winapi::um::winnt::GENERIC_WRITE
                | winapi::um::winnt::WRITE_DAC
                | winapi::um::winnt::WRITE_OWNER)
                == 0
        };
        assert!(old_considered_safe(0x1000_0000)); // GENERIC_ALL (not GENERIC_WRITE)
        assert!(old_considered_safe(0x0000_0046)); // write data / append / delete child
        assert!(old_considered_safe(0x0000_0100)); // FILE_WRITE_ATTRIBUTES
        assert!(windows::ace_is_dangerous_for_test(0x1000_0000));
        assert!(windows::ace_is_dangerous_for_test(0x0000_0046));
        assert!(windows::ace_is_dangerous_for_test(0x0000_0100));
    }

    #[cfg(windows)]
    #[test]
    fn granular_and_generic_write_rights_are_dangerous() {
        // These masks were accepted by the old GENERIC_WRITE|WRITE_DAC|WRITE_OWNER
        // check. They must now be refused for untrusted SIDs.
        assert!(windows::ace_is_dangerous_for_test(0x001F_01FF)); // FILE_ALL_ACCESS
        assert!(windows::ace_is_dangerous_for_test(0x1000_0000)); // GENERIC_ALL
        assert!(windows::ace_is_dangerous_for_test(0x4000_0000)); // GENERIC_WRITE
        assert!(windows::ace_is_dangerous_for_test(0x0000_0002)); // FILE_WRITE_DATA
        assert!(windows::ace_is_dangerous_for_test(0x0000_0004)); // FILE_APPEND_DATA / ADD_SUBDIR
        assert!(windows::ace_is_dangerous_for_test(0x0000_0040)); // FILE_DELETE_CHILD
        assert!(windows::ace_is_dangerous_for_test(0x0001_0000)); // DELETE
        assert!(!windows::ace_is_dangerous_for_test(0x0012_0089)); // FILE_GENERIC_READ
    }

    #[cfg(windows)]
    #[test]
    fn real_windows_sid_validation_is_structural() {
        assert!(windows::validate_sid(SID_ADMINISTRATORS).is_ok());
        assert!(windows::validate_sid("S-1-5-999999999999999999999999").is_err());
        assert!(windows::validate_sid("S-1-5").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn unprivileged_provisioning_is_denied() {
        if windows::require_provisioning_authority().is_ok() {
            eprintln!("SKIP: elevated host; privileged provisioning acceptance NOT RUN");
            return;
        }
        let registry = OwnerRegistry::empty(1);
        assert_eq!(
            windows::provision_registry(Path::new("C:\\nonexistent\\registry.json"), &registry),
            Err(Refusal::ElevatedAuthorityRequired)
        );
    }

    #[cfg(windows)]
    #[test]
    fn missing_file_security_cannot_be_verified() {
        let path = std::env::temp_dir().join(format!(
            "prometheos-registry-missing-{}",
            std::process::id()
        ));
        assert_eq!(
            windows::validate_file_security(&path),
            Err(Refusal::IoFailure)
        );
    }

    #[cfg(windows)]
    #[test]
    fn ordinary_user_file_is_not_a_trusted_registry() {
        let path = std::env::temp_dir().join(format!(
            "prometheos-registry-ordinary-{}",
            std::process::id()
        ));
        std::fs::write(&path, b"{}").expect("create temp file");
        let result = windows::validate_file_security(&path);
        let _ = std::fs::remove_file(&path);
        assert!(result.is_err());
    }

    #[cfg(windows)]
    #[test]
    fn reparse_point_or_junction_is_detected() {
        let dir = std::env::temp_dir().join(format!(
            "prometheos-registry-reparse-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let target = dir.join("target");
        std::fs::create_dir_all(&target).expect("create target dir");
        let link = dir.join("link");
        if std::os::windows::fs::symlink_dir(&target, &link).is_err() {
            eprintln!("SKIP: junction creation unavailable; reparse acceptance NOT RUN");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        assert!(windows::reject_reparse(&link.join("registry.json")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn world_writable_directory_is_refused() {
        let dir =
            std::env::temp_dir().join(format!("prometheos-registry-world-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let Ok(output) = std::process::Command::new("icacls")
            .arg(&dir)
            .arg("/grant")
            .arg("Everyone:(OI)(CI)F")
            .output()
        else {
            eprintln!("SKIP: icacls unavailable; ACL negative test NOT RUN");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        };
        if !output.status.success() {
            eprintln!("SKIP: icacls could not modify ACL; ACL negative test NOT RUN");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        assert!(windows::validate_file_security(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn poisoned_programdata_environment_is_refused() {
        let os_root = Path::new("C:\\ProgramData");
        assert!(known_folder_matches_environment(os_root, None));
        assert!(known_folder_matches_environment(
            os_root,
            Some(Path::new("C:\\ProgramData"))
        ));
        assert!(!known_folder_matches_environment(
            os_root,
            Some(&os_root.join("attacker-controlled")),
        ));
    }
}
