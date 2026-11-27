// Copyright (c) 2026 Memento Mori Labs LLC. All rights reserved.
// Proprietary. Trusted Windows SID-to-Lite-owner provisioning (issue #340).
//
// STAGE 1 ONLY: a versioned, machine-local, administrator/SYSTEM-controlled
// owner binding registry. No transport, client enrollment, credential issuer
// or execution authority is added here.
//
// Security invariants (dispatch 056 repair):
//  - every Windows check is bound to ONE explicitly opened process token;
//  - ACL evaluation expands generic and specific rights, rejects unknown
//    ACE forms, and never trusts an inherited personal-account ACL;
//  - writes use a SINGLE stable OS-backed mutex shared by all processes,
//    held across read/compare/replace; the update rule compares against the
//    full previous registry, so revoked bindings cannot be dropped;
//  - a genuinely absent registry under a verified, explicitly secured parent
//    tree is the only path into first-install provisioning; every other
//    failure (access denied, invalid metadata, sharing violation, corrupt
//    document, unsafe path, I/O error) fails closed and is distinguishable;
//  - registry/temp/lock creation installs an explicit owner + DACL and the
//    created object is verified before it becomes authoritative;
//  - reads are bounded before allocation;
//  - the ProgramData root comes from the Known Folder API and a poisoned
//    process environment is refused.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

pub const OWNER_REGISTRY_SCHEMA: &str = "prometheos-owner-registry-v1";
pub const PROGRAMDATA_RELATIVE_PATH: &str = "PrometheOS/Lite/owner-registry/registry.json";
pub const REGISTRY_FILE_NAME: &str = "registry.json";

/// Hard ceiling applied before any registry allocation. A registry is a small
/// machine-local binding table; anything larger is rejected as corrupt.
pub const MAX_REGISTRY_BYTES: usize = 1_048_576;

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

/// Bounded parse. Oversized input is refused *before* deserialization or any
/// unbounded allocation; malformed JSON is a distinct `CorruptRegistry`.
pub fn parse_registry_bytes(bytes: &[u8]) -> RegistryResult<OwnerRegistry> {
    if bytes.len() > MAX_REGISTRY_BYTES {
        return Err(Refusal::CorruptRegistry);
    }
    let registry: OwnerRegistry =
        serde_json::from_slice(bytes).map_err(|_| Refusal::CorruptRegistry)?;
    validate_registry(&registry).map_err(|_| Refusal::InvalidRegistry)?;
    Ok(registry)
}

/// Pure split of a ProgramData-relative registry path into parent directory
/// components and the registry file leaf. The leaf is never returned as a
/// directory component, so it can never be passed to directory bootstrap.
pub fn registry_path_parts(relative: &Path) -> RegistryResult<(Vec<String>, String)> {
    let mut parts: Vec<String> = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    let leaf = parts.pop().ok_or(Refusal::UntrustedLocation)?;
    if !leaf.eq_ignore_ascii_case(REGISTRY_FILE_NAME) || parts.is_empty() {
        return Err(Refusal::UntrustedLocation);
    }
    Ok((parts, leaf))
}

/// Atomic, monotonic compare/update rule. Every existing binding must survive:
/// omission (including of a revoked tombstone), reappearance as active,
/// owner change, generation/revision rollback and stale concurrent updates
/// are all hard refusals. There is no tombstone-pruning path.
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
        bail!("registry generation must strictly increase (stale update refused)");
    }
    let old: BTreeMap<&str, &OwnerBinding> = previous
        .bindings
        .iter()
        .map(|b| (b.sid.as_str(), b))
        .collect();
    let new: BTreeMap<&str, &OwnerBinding> = proposed
        .bindings
        .iter()
        .map(|b| (b.sid.as_str(), b))
        .collect();
    for (sid, before) in &old {
        let after = new
            .get(sid)
            .ok_or_else(|| anyhow::anyhow!("removal of binding {sid} is refused"))?;
        if before.revoked && !after.revoked {
            bail!("a revoked binding cannot be reactivated");
        }
        if before.owner != after.owner {
            bail!("an existing SID's owner cannot change");
        }
        if after.revision < before.revision {
            bail!("binding revision cannot decrease");
        }
        if before.revoked != after.revoked && after.revision == before.revision {
            bail!("a revocation change requires a strictly greater revision");
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
    InvalidSecurityMetadata,
    UnsafePath,
    CorruptRegistry,
    InvalidRegistry,
    RegistryAbsent,
    AccessDenied,
    SharingViolation,
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
            Self::InvalidSecurityMetadata => "invalid or unsupported security metadata",
            Self::UnsafePath => "unsafe reparse/junction/replacement path",
            Self::CorruptRegistry => "corrupt or oversized owner registry document",
            Self::InvalidRegistry => "invalid owner registry document",
            Self::RegistryAbsent => "registry is genuinely absent beneath a verified parent tree",
            Self::AccessDenied => "access denied while opening the registry",
            Self::SharingViolation => "registry is held by another process",
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
    use sha2::{Digest, Sha256};
    use std::cell::RefCell;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    use winapi::ctypes::c_void;
    use winapi::shared::minwindef::{BOOL, DWORD, FALSE, LPVOID, TRUE};
    use winapi::shared::sddl::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        ConvertStringSidToSidW, SDDL_REVISION_1,
    };
    use winapi::shared::winerror::{
        ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS, ERROR_FILE_NOT_FOUND,
        ERROR_PATH_NOT_FOUND, ERROR_PRIVILEGE_NOT_HELD, ERROR_SHARING_VIOLATION, ERROR_SUCCESS,
        WAIT_TIMEOUT,
    };
    use winapi::um::accctrl::{SE_FILE_OBJECT, SE_KERNEL_OBJECT};
    use winapi::um::aclapi::GetSecurityInfo;
    use winapi::um::errhandlingapi::GetLastError;
    use winapi::um::fileapi::{
        BY_HANDLE_FILE_INFORMATION, CREATE_NEW, CreateDirectoryW, CreateFileW,
        FILE_DISPOSITION_INFO, FlushFileBuffers, GetFileInformationByHandle, GetFileType,
        GetFinalPathNameByHandleW, OPEN_EXISTING, ReadFile, SetFileInformationByHandle, WriteFile,
    };
    use winapi::um::handleapi::CloseHandle;
    use winapi::um::minwinbase::{
        FILE_INFO_BY_HANDLE_CLASS, FileDispositionInfo, SECURITY_ATTRIBUTES,
    };
    use winapi::um::processthreadsapi::{
        GetCurrentProcess, GetCurrentThread, OpenProcessToken, OpenThreadToken,
    };
    use winapi::um::securitybaseapi::{
        CheckTokenMembership, DuplicateToken, GetAce, GetAclInformation, GetLengthSid,
        GetSidSubAuthorityCount, GetTokenInformation, IsValidAcl, IsValidSecurityDescriptor,
        IsValidSid,
    };
    use winapi::um::synchapi::{CreateMutexW, ReleaseMutex, WaitForSingleObject};
    use winapi::um::winbase::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_TYPE_DISK, LocalFree,
        VOLUME_NAME_DOS, WAIT_ABANDONED, WAIT_OBJECT_0,
    };
    use winapi::um::winnt::{
        ACCESS_ALLOWED_ACE, ACCESS_ALLOWED_ACE_TYPE, ACCESS_SYSTEM_SECURITY, ACE_HEADER,
        ACL_SIZE_INFORMATION, AclSizeInformation, CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION,
        DELETE, FILE_ALL_ACCESS, FILE_APPEND_DATA, FILE_ATTRIBUTE_DIRECTORY,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_DELETE_CHILD, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_WRITE_ATTRIBUTES, FILE_WRITE_DATA, FILE_WRITE_EA, GENERIC_ALL, GENERIC_READ,
        GENERIC_WRITE, HANDLE, INHERIT_ONLY_ACE, INHERITED_ACE, MAXIMUM_ALLOWED,
        NO_PROPAGATE_INHERIT_ACE, OBJECT_INHERIT_ACE, OWNER_SECURITY_INFORMATION, PACL, PSID,
        READ_CONTROL, SID, SecurityImpersonation, TOKEN_DUPLICATE, TOKEN_ELEVATION, TOKEN_QUERY,
        TOKEN_USER, TokenElevation, TokenUser, WRITE_DAC, WRITE_OWNER,
    };

    const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;
    /// Native kernel object full access for a mutex: STANDARD_RIGHTS_REQUIRED
    /// (0x000F0000) | SYNCHRONIZE (0x00100000) | MUTEX_MODIFY_STATE (0x0001).
    /// The Windows kernel maps GENERIC_ALL to this value on mutex DACLs.
    const MUTEX_ALL_ACCESS: DWORD = 0x001F0001;
    /// Operation codes: 0=none, 1=temp write/flush, 2=publication,
    /// 3=post-publication verification, 4=directory bootstrap.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct NativeIoDiagnostic {
        pub operation: u32,
        pub win32_error: u32,
        pub ntstatus: u32,
        pub temp_created: bool,
        pub publication_attempted: bool,
        pub publication_completed: bool,
        pub verification_completed: bool,
    }

    // Thread-local so concurrent attempts cannot mix diagnostic fields.
    thread_local! {
        static LAST_IO: RefCell<NativeIoDiagnostic> = RefCell::new(NativeIoDiagnostic::default());
    }

    /// Read-only, last-operation native I/O evidence for receipts. No secrets
    /// or protected registry contents are exposed.
    pub fn last_native_io_diagnostic() -> NativeIoDiagnostic {
        LAST_IO.with(|cell| *cell.borrow())
    }

    fn reset_io_diagnostic() {
        LAST_IO.with(|cell| *cell.borrow_mut() = NativeIoDiagnostic::default());
    }

    fn update_io_diagnostic<F: FnOnce(&mut NativeIoDiagnostic)>(update: F) {
        LAST_IO.with(|cell| update(&mut cell.borrow_mut()));
    }
    /// Upper bound on a TokenUser structure before it is treated as malformed.
    const MAX_TOKEN_BYTES: DWORD = 4096;
    /// Not exported by winapi 0.3; documented value of FILE_NAME_NORMALIZED.
    const FILE_NAME_NORMALIZED: DWORD = 0;
    /// A competing writer that holds the shared mutex this long is refused as
    /// a conflict rather than waited on indefinitely.
    const LOCK_WAIT_MS: DWORD = 5_000;
    const TEMP_ATTEMPTS: u8 = 8;
    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Owner = Administrators, DACL = full control for Administrators/SYSTEM
    /// only, protected so no inherited personal-account ACE can slip in.
    const TRUSTED_FILE_SDDL: &str = "O:BAD:P(A;;GA;;;SY)(A;;GA;;;BA)";
    const TRUSTED_DIR_SDDL: &str = "O:BAD:P(A;OICI;GA;;;SY)(A;OICI;GA;;;BA)";
    const FILE_RENAME_INFORMATION_CLASS: u32 = 10;

    #[repr(C)]
    struct IoStatusBlock {
        status: i32,
        information: usize,
    }

    #[repr(C)]
    struct FileRenameInformation {
        replace_if_exists: u8,
        root_directory: HANDLE,
        file_name_length: u32,
        file_name: [u16; 1],
    }

    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtSetInformationFile(
            file_handle: HANDLE,
            io_status_block: *mut IoStatusBlock,
            file_information: *mut FileRenameInformation,
            length: DWORD,
            information_class: u32,
        ) -> i32;
        fn RtlNtStatusToDosError(status: i32) -> DWORD;
    }

    struct HandleGuard(HANDLE);
    impl Drop for HandleGuard {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    struct SidGuard(*mut c_void);
    impl Drop for SidGuard {
        fn drop(&mut self) {
            unsafe { LocalFree(self.0) };
        }
    }

    struct DescriptorGuard(*mut c_void);
    impl Drop for DescriptorGuard {
        fn drop(&mut self) {
            unsafe { LocalFree(self.0) };
        }
    }

    struct MutexGuard {
        handle: HANDLE,
    }
    impl Drop for MutexGuard {
        fn drop(&mut self) {
            unsafe { ReleaseMutex(self.handle) };
            unsafe { CloseHandle(self.handle) };
        }
    }

    /// Test seam: the same kernel mutex, without the privileged owner SDDL, so
    /// cross-process mutual exclusion can be proven on a non-elevated host.
    pub struct TestMutexGuard {
        handle: HANDLE,
    }
    impl Drop for TestMutexGuard {
        fn drop(&mut self) {
            unsafe { ReleaseMutex(self.handle) };
            unsafe { CloseHandle(self.handle) };
        }
    }

    struct TempFile {
        handle: HANDLE,
    }
    impl Drop for TempFile {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.handle) };
        }
    }

    #[derive(Clone, Copy)]
    enum AclMode {
        /// Fully trusted object we create or install ourselves.
        Strict,
        /// OS-managed ProgramData root: owner and trusted full-control ACE are
        /// required, but inherited OS ACEs (e.g. authenticated-user create
        /// rights) are tolerated. We never rewrite the OS-owned root.
        OsManagedRoot,
    }

    /// Object type asserted from kernel handle metadata. Directory components
    /// and the registry file leaf are NEVER interchangeable.
    #[derive(Clone, Copy)]
    enum ObjectKind {
        Directory,
        File,
        Any,
    }

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    fn sid_to_string(sid: *mut SID) -> RegistryResult<String> {
        if sid.is_null() || unsafe { IsValidSid(sid as *mut c_void) } != TRUE {
            return Err(Refusal::InvalidSecurityMetadata);
        }
        let mut text: *mut u16 = ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(sid as *mut c_void, &mut text) } != TRUE
            || text.is_null()
        {
            return Err(Refusal::InvalidSecurityMetadata);
        }
        let result = unsafe {
            let len = (0..).take_while(|&i| *text.offset(i) != 0).count();
            String::from_utf16_lossy(std::slice::from_raw_parts(text, len))
        };
        unsafe { LocalFree(text as LPVOID) };
        Ok(result)
    }

    fn well_known_sid(text: &str) -> RegistryResult<PSID> {
        let mut wide_text: Vec<u16> = OsStr::new(text)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut sid: PSID = ptr::null_mut();
        if unsafe { ConvertStringSidToSidW(wide_text.as_mut_ptr(), &mut sid) } != TRUE
            || sid.is_null()
        {
            return Err(Refusal::InvalidSecurityMetadata);
        }
        Ok(sid)
    }

    /// Structural SID validation using Windows SID APIs, not decimal grammar.
    pub fn validate_sid(text: &str) -> RegistryResult<()> {
        let sid = well_known_sid(text)?;
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

    fn trusted_sid(sid: &str) -> bool {
        sid.eq_ignore_ascii_case(SID_ADMINISTRATORS) || sid.eq_ignore_ascii_case(SID_SYSTEM)
    }

    /// Full access-right evaluation: generic rights are expanded, specific
    /// write/delete/take-ownership/replacement rights are rejected for
    /// untrusted SIDs. ACCESS_SYSTEM_SECURITY and MAXIMUM_ALLOWED also fail
    /// closed. Known read-only rights are allowed for service identities.
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
                | FILE_WRITE_EA
                | ACCESS_SYSTEM_SECURITY
                | MAXIMUM_ALLOWED);
        generic_all || generic_write || specific != 0
    }

    fn has_full_control(mask: DWORD) -> bool {
        mask & GENERIC_ALL != 0 || mask & FILE_ALL_ACCESS == FILE_ALL_ACCESS
    }

    fn has_mutex_full_control(mask: DWORD) -> bool {
        mask & GENERIC_ALL != 0 || mask & MUTEX_ALL_ACCESS == MUTEX_ALL_ACCESS
    }

    /// Mutex/kernel-object DACL policy. File-specific rights are not applicable
    /// to synchronization objects: Windows maps GENERIC_ALL on a mutex DACL to
    /// MUTEX_ALL_ACCESS (0x001F0001). This validator accepts only that native
    /// full-control form for trusted SIDs, and refuses every other effective
    /// access, including SYNCHRONIZE alone, partial rights, unknown bits,
    /// inheritance flags and unsupported ACE forms.
    fn validate_mutex_acl(acl: PACL) -> RegistryResult<()> {
        if acl.is_null() || unsafe { IsValidAcl(acl) } != TRUE {
            return Err(Refusal::InvalidSecurityMetadata);
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
            return Err(Refusal::InvalidSecurityMetadata);
        }
        const BAD_ACE_FLAGS: u8 = OBJECT_INHERIT_ACE
            | CONTAINER_INHERIT_ACE
            | NO_PROPAGATE_INHERIT_ACE
            | INHERIT_ONLY_ACE
            | INHERITED_ACE;
        let known_bits = MUTEX_ALL_ACCESS | GENERIC_ALL;
        let mut trusted_full = false;
        for index in 0..info.AceCount {
            let mut ace: LPVOID = ptr::null_mut();
            if unsafe { GetAce(acl, index, &mut ace) } != TRUE || ace.is_null() {
                return Err(Refusal::InvalidSecurityMetadata);
            }
            let header = unsafe { &*(ace as *const ACE_HEADER) };
            if header.AceType != ACCESS_ALLOWED_ACE_TYPE
                || (header.AceSize as usize) < std::mem::size_of::<ACCESS_ALLOWED_ACE>()
                || header.AceFlags & BAD_ACE_FLAGS != 0
            {
                return Err(Refusal::InvalidSecurityMetadata);
            }
            let allowed = unsafe { &*(ace as *const ACCESS_ALLOWED_ACE) };
            let sid = sid_to_string((&allowed.SidStart as *const DWORD) as *mut SID)?;
            if !trusted_sid(&sid) {
                // Every allow ACE in a kernel DACL grants effective access.
                // Untrusted access (including SYNCHRONIZE) is refused.
                return Err(Refusal::InvalidSecurityMetadata);
            }
            let mask = allowed.Mask;
            if mask == 0 || mask & !known_bits != 0 || !has_mutex_full_control(mask) {
                return Err(Refusal::InvalidSecurityMetadata);
            }
            trusted_full = true;
        }
        if !trusted_full {
            return Err(Refusal::InvalidSecurityMetadata);
        }
        Ok(())
    }

    fn validate_acl_inner(acl: PACL, mode: AclMode) -> RegistryResult<()> {
        if acl.is_null() || unsafe { IsValidAcl(acl) } != TRUE {
            return Err(Refusal::InvalidSecurityMetadata);
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
            return Err(Refusal::InvalidSecurityMetadata);
        }
        let mut trusted_full = false;
        for index in 0..info.AceCount {
            let mut ace: LPVOID = ptr::null_mut();
            if unsafe { GetAce(acl, index, &mut ace) } != TRUE || ace.is_null() {
                return Err(Refusal::InvalidSecurityMetadata);
            }
            let header = unsafe { &*(ace as *const ACE_HEADER) };
            // Deny, callback, object and conditional forms are not safely
            // evaluated here; fail closed for every mode.
            if header.AceType != ACCESS_ALLOWED_ACE_TYPE
                || (header.AceSize as usize) < std::mem::size_of::<ACCESS_ALLOWED_ACE>()
            {
                return Err(Refusal::InvalidSecurityMetadata);
            }
            let allowed = unsafe { &*(ace as *const ACCESS_ALLOWED_ACE) };
            let sid = sid_to_string((&allowed.SidStart as *const DWORD) as *mut SID)?;
            if trusted_sid(&sid) {
                if has_full_control(allowed.Mask) {
                    trusted_full = true;
                }
            } else if matches!(mode, AclMode::Strict) && ace_is_dangerous(allowed.Mask) {
                return Err(Refusal::InvalidSecurityMetadata);
            }
        }
        if !trusted_full {
            return Err(Refusal::InvalidSecurityMetadata);
        }
        Ok(())
    }

    fn validate_acl(acl: PACL) -> RegistryResult<()> {
        validate_acl_inner(acl, AclMode::Strict)
    }

    /// Test seam for crafted ACLs (isolates DACL evaluation from file ownership).
    pub fn validate_acl_for_test(acl: PACL) -> RegistryResult<()> {
        validate_acl(acl)
    }

    /// Test seam for the production mutex DACL validator, used by the native
    /// diagnostic and crafted ACL regressions. No parallel validator exists.
    pub fn validate_mutex_acl_for_test(acl: PACL) -> RegistryResult<()> {
        validate_mutex_acl(acl)
    }

    /// Read-only observation of an OS object's DACL: does it grant full control
    /// to the given SID? Lets an isolated negative-control test prove the
    /// intended ACL was actually installed before its refusal assertion counts.
    /// Nothing is changed; the caller owns cleanup of the object.
    pub fn dacl_grants_full_control_to(path: &Path, sid: &str) -> RegistryResult<bool> {
        let handle = open_raw(
            path,
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_DELETE,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )?;
        let _guard = HandleGuard(handle);
        let mut owner: PSID = ptr::null_mut();
        let mut group: PSID = ptr::null_mut();
        let mut dacl: PACL = ptr::null_mut();
        let mut sacl: PACL = ptr::null_mut();
        let mut descriptor: *mut c_void = ptr::null_mut();
        let status = unsafe {
            GetSecurityInfo(
                handle,
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                &mut owner,
                &mut group,
                &mut dacl,
                &mut sacl,
                &mut descriptor,
            )
        };
        if status != ERROR_SUCCESS || descriptor.is_null() {
            return Err(Refusal::InvalidSecurityMetadata);
        }
        let _desc_guard = DescriptorGuard(descriptor);
        if dacl.is_null() || unsafe { IsValidAcl(dacl) } != TRUE {
            return Err(Refusal::InvalidSecurityMetadata);
        }
        let mut info: ACL_SIZE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe {
            GetAclInformation(
                dacl,
                &mut info as *mut _ as LPVOID,
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as DWORD,
                AclSizeInformation,
            )
        } != TRUE
        {
            return Err(Refusal::InvalidSecurityMetadata);
        }
        for index in 0..info.AceCount {
            let mut ace: LPVOID = ptr::null_mut();
            if unsafe { GetAce(dacl, index, &mut ace) } != TRUE || ace.is_null() {
                return Err(Refusal::InvalidSecurityMetadata);
            }
            let header = unsafe { &*(ace as *const ACE_HEADER) };
            if header.AceType != ACCESS_ALLOWED_ACE_TYPE
                || (header.AceSize as usize) < std::mem::size_of::<ACCESS_ALLOWED_ACE>()
            {
                continue;
            }
            let allowed = unsafe { &*(ace as *const ACCESS_ALLOWED_ACE) };
            let ace_sid = sid_to_string((&allowed.SidStart as *const DWORD) as *mut SID)?;
            if ace_sid.eq_ignore_ascii_case(sid) && has_full_control(allowed.Mask) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn validate_handle_security_inner(
        handle: HANDLE,
        mode: AclMode,
        kind: ObjectKind,
    ) -> RegistryResult<()> {
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(handle, &mut info) } != TRUE {
            return Err(Refusal::InvalidSecurityMetadata);
        }
        if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(Refusal::UnsafePath);
        }
        // Assert the object type from the handle before any trust decision. A
        // directory masquerading as the registry leaf (or a file masquerading
        // as a parent) is always refused.
        let is_directory = info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0;
        match kind {
            ObjectKind::Directory if !is_directory => return Err(Refusal::UnsafePath),
            ObjectKind::File if is_directory => return Err(Refusal::UnsafePath),
            ObjectKind::Any | ObjectKind::Directory | ObjectKind::File => {}
        }
        if unsafe { GetFileType(handle) } != FILE_TYPE_DISK {
            return Err(Refusal::UnsafePath);
        }
        let mut owner: PSID = ptr::null_mut();
        let mut group: PSID = ptr::null_mut();
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
            return Err(Refusal::InvalidSecurityMetadata);
        }
        let _guard = DescriptorGuard(descriptor);
        if !trusted_sid(&sid_to_string(owner as *mut SID)?) {
            return Err(Refusal::InvalidSecurityMetadata);
        }
        validate_acl_inner(dacl, mode)
    }

    fn validate_kernel_security(handle: HANDLE) -> RegistryResult<()> {
        let mut owner: PSID = ptr::null_mut();
        let mut group: PSID = ptr::null_mut();
        let mut dacl: PACL = ptr::null_mut();
        let mut sacl: PACL = ptr::null_mut();
        let mut descriptor: *mut c_void = ptr::null_mut();
        let status = unsafe {
            GetSecurityInfo(
                handle,
                SE_KERNEL_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                &mut group,
                &mut dacl,
                &mut sacl,
                &mut descriptor,
            )
        };
        if status != ERROR_SUCCESS || descriptor.is_null() {
            return Err(Refusal::InvalidSecurityMetadata);
        }
        let _guard = DescriptorGuard(descriptor);
        if !trusted_sid(&sid_to_string(owner as *mut SID)?) {
            return Err(Refusal::InvalidSecurityMetadata);
        }
        validate_mutex_acl(dacl)
    }

    /// Kernel-resolved absolute path of an already verified handle, including
    /// the `\\?\` prefix required by native FileRenameInfo absolute names.
    fn final_path_string(handle: HANDLE) -> RegistryResult<String> {
        let mut buf: Vec<u16> = vec![0; 512];
        loop {
            let len = unsafe {
                GetFinalPathNameByHandleW(
                    handle,
                    buf.as_mut_ptr(),
                    buf.len() as DWORD,
                    VOLUME_NAME_DOS | FILE_NAME_NORMALIZED,
                )
            };
            if len == 0 {
                return Err(Refusal::UntrustedLocation);
            }
            if len as usize > buf.len() {
                buf.resize(len as usize + 1, 0);
                continue;
            }
            return Ok(String::from_utf16_lossy(&buf[..len as usize]));
        }
    }

    /// Bind the opened handle to the exact trusted path: a junction or a
    /// replacement in any ancestor makes the final path differ.
    fn final_path_matches(handle: HANDLE, expected: &Path) -> RegistryResult<()> {
        let mut buf: Vec<u16> = vec![0; 512];
        loop {
            let len = unsafe {
                GetFinalPathNameByHandleW(
                    handle,
                    buf.as_mut_ptr(),
                    buf.len() as DWORD,
                    VOLUME_NAME_DOS | FILE_NAME_NORMALIZED,
                )
            };
            if len == 0 {
                return Err(Refusal::UntrustedLocation);
            }
            if len as usize > buf.len() {
                buf.resize(len as usize + 1, 0);
                continue;
            }
            let got = String::from_utf16_lossy(&buf[..len as usize]);
            let got = got.strip_prefix(r"\\?\").unwrap_or(&got);
            if !got.eq_ignore_ascii_case(&expected.to_string_lossy()) {
                return Err(Refusal::UntrustedLocation);
            }
            return Ok(());
        }
    }

    /// Win32 error classification. Absence is the ONLY code permitted to enter
    /// bootstrap; access denied, sharing, unsafe paths and I/O fail closed.
    pub fn classify_win32_error(code: DWORD) -> Refusal {
        match code {
            ERROR_FILE_NOT_FOUND => Refusal::RegistryAbsent,
            ERROR_PATH_NOT_FOUND => Refusal::UntrustedLocation,
            ERROR_ACCESS_DENIED | ERROR_PRIVILEGE_NOT_HELD => Refusal::AccessDenied,
            ERROR_SHARING_VIOLATION => Refusal::SharingViolation,
            ERROR_ALREADY_EXISTS | ERROR_FILE_EXISTS => Refusal::ConcurrentUpdate,
            _ => Refusal::IoFailure,
        }
    }

    fn classify_last_error() -> Refusal {
        classify_win32_error(unsafe { GetLastError() })
    }

    fn open_raw(
        path: &Path,
        desired: DWORD,
        share: DWORD,
        creation: DWORD,
        flags: DWORD,
        sd: Option<*mut c_void>,
    ) -> RegistryResult<HANDLE> {
        let mut sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as DWORD,
            lpSecurityDescriptor: ptr::null_mut(),
            bInheritHandle: FALSE,
        };
        if let Some(descriptor) = sd {
            sa.lpSecurityDescriptor = descriptor;
        }
        let sa_ptr = if sd.is_some() {
            &mut sa as *mut SECURITY_ATTRIBUTES
        } else {
            ptr::null_mut()
        };
        let handle = unsafe {
            CreateFileW(
                wide(path.as_os_str()).as_ptr(),
                desired,
                share,
                sa_ptr,
                creation,
                flags,
                ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE || handle.is_null() {
            return Err(classify_last_error());
        }
        Ok(handle)
    }

    fn open_verified(
        path: &Path,
        expected: &Path,
        write: bool,
        create_new: bool,
        sd: Option<*mut c_void>,
        mode: AclMode,
        kind: ObjectKind,
    ) -> RegistryResult<HANDLE> {
        let desired = GENERIC_READ | READ_CONTROL | if write { GENERIC_WRITE | DELETE } else { 0 };
        let share = if write {
            FILE_SHARE_READ | FILE_SHARE_DELETE
        } else {
            FILE_SHARE_READ
        };
        let creation = if create_new {
            CREATE_NEW
        } else {
            OPEN_EXISTING
        };
        let handle = open_raw(
            path,
            desired,
            share,
            creation,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            sd,
        )?;
        if let Err(refusal) = validate_handle_security_inner(handle, mode, kind) {
            unsafe { CloseHandle(handle) };
            return Err(refusal);
        }
        if let Err(refusal) = final_path_matches(handle, expected) {
            unsafe { CloseHandle(handle) };
            return Err(refusal);
        }
        Ok(handle)
    }

    /// Stable kernel identity of a file handle, used to prove the temp object
    /// published is the exact object this run created and flushed.
    fn file_identity(handle: HANDLE) -> RegistryResult<(u32, u32, u32)> {
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(handle, &mut info) } != TRUE {
            return Err(Refusal::IoFailure);
        }
        Ok((
            info.dwVolumeSerialNumber,
            info.nFileIndexHigh,
            info.nFileIndexLow,
        ))
    }

    fn read_all(handle: HANDLE) -> RegistryResult<Vec<u8>> {
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(handle, &mut info) } != TRUE {
            return Err(Refusal::IoFailure);
        }
        let size = ((info.nFileSizeHigh as u64) << 32) | info.nFileSizeLow as u64;
        if size > MAX_REGISTRY_BYTES as u64 {
            return Err(Refusal::CorruptRegistry);
        }
        let mut bytes = Vec::with_capacity(size as usize);
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
            if bytes.len() + read as usize > MAX_REGISTRY_BYTES {
                return Err(Refusal::CorruptRegistry);
            }
            bytes.extend_from_slice(&chunk[..read as usize]);
        }
        Ok(bytes)
    }

    fn load_via_handle(path: &Path) -> RegistryResult<OwnerRegistry> {
        let handle = open_verified(
            path,
            path,
            false,
            false,
            None,
            AclMode::Strict,
            ObjectKind::File,
        )?;
        let _guard = HandleGuard(handle);
        parse_registry_bytes(&read_all(handle)?)
    }

    fn trusted_sd(is_dir: bool) -> RegistryResult<DescriptorGuard> {
        let sddl = if is_dir {
            TRUSTED_DIR_SDDL
        } else {
            TRUSTED_FILE_SDDL
        };
        let mut descriptor: *mut c_void = ptr::null_mut();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide(OsStr::new(sddl)).as_ptr(),
                SDDL_REVISION_1 as DWORD,
                &mut descriptor,
                ptr::null_mut(),
            )
        } != TRUE
            || descriptor.is_null()
            || unsafe { IsValidSecurityDescriptor(descriptor) } != TRUE
        {
            return Err(Refusal::InvalidSecurityMetadata);
        }
        Ok(DescriptorGuard(descriptor))
    }

    /// Open a directory pin that does not share DELETE: while this handle is
    /// alive the directory object cannot be renamed or replaced by another
    /// writer, including by a process using the same pathname.
    fn open_directory_pin(
        path: &Path,
        mode: AclMode,
        validate_security: bool,
    ) -> RegistryResult<HandleGuard> {
        let handle = open_raw(
            path,
            GENERIC_READ | READ_CONTROL,
            FILE_SHARE_READ,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )?;
        if validate_security
            && let Err(refusal) =
                validate_handle_security_inner(handle, mode, ObjectKind::Directory)
        {
            unsafe { CloseHandle(handle) };
            return Err(refusal);
        }
        if let Err(refusal) = final_path_matches(handle, path) {
            unsafe { CloseHandle(handle) };
            return Err(refusal);
        }
        Ok(HandleGuard(handle))
    }

    fn verify_root(root: &Path) -> RegistryResult<HandleGuard> {
        open_directory_pin(root, AclMode::OsManagedRoot, true)
    }

    fn verify_or_create_dir(dir: &Path, create: bool) -> RegistryResult<HandleGuard> {
        match open_directory_pin(dir, AclMode::Strict, true) {
            Ok(guard) => Ok(guard),
            Err(Refusal::RegistryAbsent) if create => {
                let descriptor = trusted_sd(true)?;
                let mut sa = SECURITY_ATTRIBUTES {
                    nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as DWORD,
                    lpSecurityDescriptor: descriptor.0,
                    bInheritHandle: FALSE,
                };
                if unsafe { CreateDirectoryW(wide(dir.as_os_str()).as_ptr(), &mut sa) } != TRUE {
                    let code = unsafe { GetLastError() };
                    if code != ERROR_ALREADY_EXISTS {
                        update_io_diagnostic(|diag| {
                            diag.operation = 4;
                            diag.win32_error = code;
                        });
                        return Err(classify_win32_error(code));
                    }
                }
                open_directory_pin(dir, AclMode::Strict, true)
            }
            Err(other) => Err(other),
        }
    }

    fn expected_path(path: &Path) -> RegistryResult<PathBuf> {
        let expected = default_registry_path().map_err(|_| Refusal::UntrustedLocation)?;
        if !path.is_absolute()
            || !path
                .to_string_lossy()
                .eq_ignore_ascii_case(&expected.to_string_lossy())
        {
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
        Ok(expected)
    }

    /// Verify the exact trusted location and every PARENT directory. When
    /// `create` is true, missing ancestors are bootstrapped with an explicit
    /// trusted owner/DACL instead of inheriting an administrator's personal ACL.
    ///
    /// The registry leaf is deliberately excluded: it is a regular file, never
    /// a directory component. An existing leaf is verified separately by
    /// `validate_registry_leaf` / `load_via_handle` as a non-reparse file with a
    /// trusted owner and DACL. Leaf absence is only accepted by the trusted
    /// first-install path, and an incorrectly created directory leaf is refused
    /// (never deleted or silently replaced).
    fn verify_location_structure(path: &Path, create: bool) -> RegistryResult<Vec<HandleGuard>> {
        let expected = expected_path(path)?;
        let root = programdata_root().map_err(|_| Refusal::UntrustedLocation)?;
        let mut pins = vec![verify_root(&root)?];
        let relative = expected
            .strip_prefix(&root)
            .map_err(|_| Refusal::UntrustedLocation)?;
        let (parent_parts, _leaf) = registry_path_parts(relative)?;
        let mut current = root.clone();
        for part in parent_parts {
            current = current.join(part);
            pins.push(verify_or_create_dir(&current, create)?);
        }
        Ok(pins)
    }

    /// One stable OS-backed mutex identity per registry path. No PID, timestamp
    /// or random suffix: every competing process contends on the same object.
    fn lock_name(path: &Path, global: bool) -> String {
        let key = path.to_string_lossy().to_lowercase();
        let digest = Sha256::digest(key.as_bytes());
        let suffix: String = digest[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        format!(
            "{}\\PrometheOS.OwnerRegistry.{}",
            if global { "Global" } else { "Local" },
            suffix
        )
    }

    /// Test seam: the Local (session-scoped) identity of the shared mutex.
    pub fn test_mutex_name(path: &Path) -> String {
        lock_name(path, false)
    }

    fn wait_for_mutex(handle: HANDLE) -> RegistryResult<()> {
        let wait = unsafe { WaitForSingleObject(handle, LOCK_WAIT_MS) };
        if wait == WAIT_OBJECT_0 || wait == WAIT_ABANDONED {
            // WAIT_ABANDONED means the previous owner died; the registry is
            // re-read and compared under the lock, so no stale state is used.
            Ok(())
        } else if wait == WAIT_TIMEOUT {
            Err(Refusal::ConcurrentUpdate)
        } else {
            Err(Refusal::IoFailure)
        }
    }

    fn acquire_update_lock(path: &Path) -> RegistryResult<MutexGuard> {
        let name = lock_name(path, true);
        let descriptor = trusted_sd(false)?;
        let mut sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as DWORD,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: FALSE,
        };
        let handle = unsafe {
            CreateMutexW(
                &mut sa as *mut SECURITY_ATTRIBUTES,
                FALSE,
                wide(OsStr::new(&name)).as_ptr(),
            )
        };
        if handle.is_null() {
            return Err(classify_last_error());
        }
        let guard = HandleGuard(handle);
        validate_kernel_security(handle)?;
        wait_for_mutex(handle)?;
        // Ownership of the handle transfers to the mutex guard.
        std::mem::forget(guard);
        Ok(MutexGuard { handle })
    }

    /// Test-only acquisition of the same named-mutex *mechanism* with a default
    /// DACL, so unprivileged processes can prove cross-process exclusion.
    pub fn acquire_test_mutex(name: &str) -> RegistryResult<TestMutexGuard> {
        let handle =
            unsafe { CreateMutexW(ptr::null_mut(), FALSE, wide(OsStr::new(name)).as_ptr()) };
        if handle.is_null() {
            return Err(Refusal::IoFailure);
        }
        if let Err(refusal) = wait_for_mutex(handle) {
            unsafe { CloseHandle(handle) };
            return Err(refusal);
        }
        Ok(TestMutexGuard { handle })
    }

    /// Create the temp object through a handle that does not share DELETE, so
    /// another writer cannot rename or replace it between validation and use.
    fn open_temp_new(
        path: &Path,
        descriptor: *mut c_void,
        validate_security: bool,
    ) -> RegistryResult<HANDLE> {
        let handle = open_raw(
            path,
            GENERIC_READ | READ_CONTROL | GENERIC_WRITE | DELETE,
            FILE_SHARE_READ,
            CREATE_NEW,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            Some(descriptor),
        )?;
        if validate_security
            && let Err(refusal) =
                validate_handle_security_inner(handle, AclMode::Strict, ObjectKind::File)
        {
            unsafe { CloseHandle(handle) };
            return Err(refusal);
        }
        if let Err(refusal) = final_path_matches(handle, path) {
            unsafe { CloseHandle(handle) };
            return Err(refusal);
        }
        update_io_diagnostic(|diag| diag.temp_created = true);
        Ok(handle)
    }

    /// Mark an operation-owned temp file for deletion through its own verified
    /// handle. No pathname lookup participates; an object that cannot be proven
    /// owned or whose disposition fails is preserved, not deleted by path.
    /// Test seam returning a raw pinned directory handle. Caller closes once.
    pub fn open_directory_pin_for_test(path: &Path) -> RegistryResult<HANDLE> {
        let guard = open_directory_pin(path, AclMode::Strict, false)?;
        let handle = guard.0;
        std::mem::forget(guard);
        Ok(handle)
    }

    /// Test seam returning a raw operation-owned temp handle. Caller closes once.
    pub fn write_temp_for_test(parent: &Path, bytes: &[u8]) -> RegistryResult<HANDLE> {
        let temp = write_temp_inner(parent, bytes, ptr::null_mut(), false)?;
        let handle = temp.handle;
        std::mem::forget(temp);
        Ok(handle)
    }

    /// Test seam for handle-bound temp disposition. Caller closes once.
    pub fn cleanup_temp_for_test(handle: HANDLE) -> RegistryResult<()> {
        cleanup_temp(handle)
    }

    fn cleanup_temp(handle: HANDLE) -> RegistryResult<()> {
        let mut info = FILE_DISPOSITION_INFO {
            DeleteFile: TRUE as u8,
        };
        if unsafe {
            SetFileInformationByHandle(
                handle,
                FileDispositionInfo as FILE_INFO_BY_HANDLE_CLASS,
                &mut info as *mut _ as *mut c_void,
                std::mem::size_of::<FILE_DISPOSITION_INFO>() as DWORD,
            )
        } != TRUE
        {
            let code = unsafe { GetLastError() };
            update_io_diagnostic(|diag| {
                diag.operation = 1;
                diag.win32_error = code;
            });
            return Err(classify_win32_error(code));
        }
        // The caller owns and closes the handle exactly once.
        Ok(())
    }

    fn write_temp(parent: &Path, bytes: &[u8]) -> RegistryResult<TempFile> {
        let descriptor = trusted_sd(false)?;
        write_temp_inner(parent, bytes, descriptor.0, true)
    }

    /// Same write/flush/identity path with a caller-supplied descriptor. The
    /// production caller supplies the trusted SDDL; nonprivileged fixtures use
    /// a default descriptor because a standard user cannot assign BA ownership.
    fn write_temp_inner(
        parent: &Path,
        bytes: &[u8],
        descriptor: *mut c_void,
        validate_security: bool,
    ) -> RegistryResult<TempFile> {
        for attempt in 0..TEMP_ATTEMPTS {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let name = format!(
                ".registry-{}-{}-{}-{attempt}.tmp",
                std::process::id(),
                nanos,
                TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
            );
            let path = parent.join(name);
            let handle = match open_temp_new(&path, descriptor, validate_security) {
                Ok(handle) => handle,
                Err(Refusal::ConcurrentUpdate) | Err(Refusal::RegistryAbsent) => continue,
                Err(other) => return Err(other),
            };
            let mut written_total = 0usize;
            let mut zero_progress = 0u8;
            while written_total < bytes.len() {
                let mut written: DWORD = 0;
                let ok = unsafe {
                    WriteFile(
                        handle,
                        bytes[written_total..].as_ptr() as *const c_void,
                        (bytes.len() - written_total) as DWORD,
                        &mut written,
                        ptr::null_mut(),
                    )
                };
                if ok != TRUE {
                    // Only a failed native call has a meaningful last error;
                    // a successful zero-byte write does not.
                    let code = unsafe { GetLastError() };
                    update_io_diagnostic(|diag| {
                        diag.operation = 1;
                        diag.win32_error = code;
                    });
                    let _ = cleanup_temp(handle);
                    unsafe { CloseHandle(handle) };
                    return Err(Refusal::IoFailure);
                }
                if written == 0 {
                    zero_progress += 1;
                    if zero_progress > 8 {
                        update_io_diagnostic(|diag| {
                            diag.operation = 1;
                            diag.win32_error = 0;
                        });
                        let _ = cleanup_temp(handle);
                        unsafe { CloseHandle(handle) };
                        return Err(Refusal::IoFailure);
                    }
                    continue;
                }
                written_total += written as usize;
            }
            if unsafe { FlushFileBuffers(handle) } != TRUE {
                let code = unsafe { GetLastError() };
                update_io_diagnostic(|diag| {
                    diag.operation = 1;
                    diag.win32_error = code;
                });
                let _ = cleanup_temp(handle);
                unsafe { CloseHandle(handle) };
                return Err(Refusal::IoFailure);
            }
            return Ok(TempFile { handle });
        }
        Err(Refusal::ConcurrentUpdate)
    }

    /// Publish the verified temp file through its own kernel handle. The
    /// destination is a simple leaf name resolved by the kernel relative to the
    /// SOURCE FILE'S OWN parent, with RootDirectory=NULL. No global pathname
    /// lookup and no retained-path string participates in the write effect.
    /// The native NtSetInformationFile(FileRenameInformation) form succeeds on
    /// the reviewed Windows build where SetFileInformationByHandle returned
    /// ERROR_INVALID_PARAMETER (87).
    fn publish_via_handle(temp_handle: HANDLE) -> RegistryResult<()> {
        let name = wide(OsStr::new(REGISTRY_FILE_NAME));
        let total = std::mem::offset_of!(FileRenameInformation, file_name) + (name.len() - 1) * 2;
        let mut storage = vec![0u64; total.div_ceil(8)];
        let info = storage.as_mut_ptr() as *mut FileRenameInformation;
        unsafe {
            (*info).replace_if_exists = TRUE as u8;
            (*info).root_directory = ptr::null_mut();
            (*info).file_name_length = ((name.len() - 1) * 2) as DWORD;
            ptr::copy_nonoverlapping(name.as_ptr(), (*info).file_name.as_mut_ptr(), name.len());
        }
        let mut status_block = IoStatusBlock {
            status: 0,
            information: 0,
        };
        update_io_diagnostic(|diag| {
            diag.operation = 2;
            diag.publication_attempted = true;
        });
        let status = unsafe {
            NtSetInformationFile(
                temp_handle,
                &mut status_block,
                info,
                total as DWORD,
                FILE_RENAME_INFORMATION_CLASS,
            )
        };
        if status < 0 {
            // Capture the immediate mapped Win32 error before any other call.
            let dos = unsafe { RtlNtStatusToDosError(status) };
            update_io_diagnostic(|diag| {
                diag.ntstatus = status as u32;
                diag.win32_error = dos;
            });
            return Err(classify_win32_error(dos));
        }
        update_io_diagnostic(|diag| {
            diag.ntstatus = status as u32;
            diag.publication_completed = true;
        });
        Ok(())
    }

    /// Test seam for the production publication implementation on a disposable
    /// user-owned fixture. No path rewriting or alternate rename is used.
    pub fn publish_via_handle_for_test(temp_handle: HANDLE) -> RegistryResult<()> {
        publish_via_handle(temp_handle)
    }

    /// Provision the registry under verified elevated authority.
    ///
    /// REPLACEMENT GUARANTEE: the new document is written and flushed to a
    /// uniquely named temp file carrying an explicit trusted security
    /// descriptor, then published with SetFileInformationByHandle(FileRenameInfo)
    /// using ReplaceIfExists=TRUE. On NTFS this is an atomic namespace rename:
    /// at every instant `registry.json` names either the whole old document or
    /// the whole new one, so partial bytes are never visible at that name and a
    /// failed publish leaves the previous trusted document untouched.
    ///
    /// DURABILITY: file data is flushed before the rename and the rename is
    /// journaled, but the parent directory is not explicitly flushed. This is
    /// therefore NOT a fully transactional or crash-atomic multi-file update.
    ///
    /// PATH TRUST: the source is identified by its verified temp handle and the
    /// simple destination leaf is resolved by the kernel relative to that
    /// source object's own parent (NtSetInformationFile/FileRenameInformation
    /// with RootDirectory=NULL). No global pathname is looked up for the write
    /// effect. The verified parent handle is retained through publication, every
    /// ancestor is bound to its kernel-resolved final path, and the
    /// admin/SYSTEM-only DACL plus shared mutex exclude unprivileged substitution.
    pub fn provision_registry(path: &Path, proposed: &OwnerRegistry) -> RegistryResult<()> {
        reset_io_diagnostic();
        require_provisioning_authority()?;
        let expected = expected_path(path)?;
        let bytes = canonical_json(proposed).map_err(|_| Refusal::InvalidRegistry)?;
        // The single shared mutex is acquired BEFORE any read/compare/write.
        let _lock = acquire_update_lock(&expected)?;
        let directory_pins = verify_location_structure(&expected, true)?;
        let previous = match load_via_handle(&expected) {
            Ok(registry) => Some(registry),
            Err(Refusal::RegistryAbsent) => None,
            // Access denied, invalid metadata, sharing violation, corrupt
            // registry, unsafe path and I/O failures all fail closed.
            Err(other) => return Err(other),
        };
        enforce_monotonic_update(previous.as_ref(), proposed)
            .map_err(|_| Refusal::NonMonotonicUpdate)?;
        let parent = expected.parent().ok_or(Refusal::UntrustedLocation)?;
        // The directory pins from verify_location_structure are retained alive
        // until this function returns. They do not share DELETE, so the pinned
        // ancestors cannot be renamed or replaced while the temp is created,
        // published and verified.
        let parent_pin = directory_pins.last().ok_or(Refusal::UntrustedLocation)?;
        let parent_final = final_path_string(parent_pin.0)?;
        let temp = write_temp(parent, &bytes)?;
        // Prove the source object belongs to the pinned parent before any
        // publication effect. The native rename then resolves the leaf relative
        // to that same source object's kernel parent.
        let temp_final = final_path_string(temp.handle)?;
        let prefix = format!("{}\\", parent_final.trim_end_matches('\\'));
        if !temp_final.starts_with(&prefix) {
            let _ = cleanup_temp(temp.handle);
            drop(temp);
            return Err(Refusal::UntrustedLocation);
        }
        let temp_identity = file_identity(temp.handle)?;
        if let Err(refusal) = publish_via_handle(temp.handle) {
            // Handle-bound disposition of the proven owned temp object; no
            // pathname deletion and no loss of the original refusal evidence.
            let _ = cleanup_temp(temp.handle);
            drop(temp);
            return Err(refusal);
        }
        // All installed verification uses the SAME verified source handle.
        update_io_diagnostic(|diag| diag.operation = 3);
        if file_identity(temp.handle)? != temp_identity {
            drop(temp);
            return Err(Refusal::IoFailure);
        }
        validate_handle_security_inner(temp.handle, AclMode::Strict, ObjectKind::File)?;
        final_path_matches(temp.handle, &expected)?;
        let installed =
            parse_registry_bytes(&read_all(temp.handle)?).map_err(|_| Refusal::InvalidRegistry)?;
        if canonical_json(&installed).map_err(|_| Refusal::InvalidRegistry)? != bytes {
            drop(temp);
            return Err(Refusal::IoFailure);
        }
        update_io_diagnostic(|diag| diag.verification_completed = true);
        drop(temp);
        Ok(())
    }

    pub fn load_registry(path: &Path) -> RegistryResult<OwnerRegistry> {
        let _directory_pins = verify_location_structure(path, false)?;
        load_via_handle(path)
    }

    pub fn resolve_owner(path: &Path, sid: &str) -> RegistryResult<Option<String>> {
        let registry = load_registry(path)?;
        lookup_owner(&registry, sid).map_err(|_| Refusal::InvalidRegistry)
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
        // DuplicateToken below needs TOKEN_DUPLICATE on this exact handle.
        // Requesting only TOKEN_QUERY makes a genuinely elevated process fail
        // with ERROR_ACCESS_DENIED at the membership check.
        if unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_QUERY | TOKEN_DUPLICATE,
                &mut token,
            )
        } != TRUE
        {
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
        // Query the required size first and reject malformed or oversized token
        // information rather than reading a fixed, possibly truncating buffer.
        let mut required: DWORD = 0;
        let _ = unsafe { GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut required) };
        if required == 0 || required > MAX_TOKEN_BYTES {
            return Err(Refusal::ElevatedAuthorityRequired);
        }
        let mut user_buf = vec![0u8; required as usize];
        returned = 0;
        if unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                user_buf.as_mut_ptr() as LPVOID,
                required,
                &mut returned,
            )
        } != TRUE
            || returned < std::mem::size_of::<TOKEN_USER>() as DWORD
        {
            return Err(Refusal::ElevatedAuthorityRequired);
        }
        let user = unsafe { &*(user_buf.as_ptr() as *const TOKEN_USER) };
        if user.User.Sid.is_null() || unsafe { IsValidSid(user.User.Sid) } != TRUE {
            return Err(Refusal::ElevatedAuthorityRequired);
        }
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

    /// Test seam for the pure ACL-rights evaluation (no OS state required).
    pub fn ace_is_dangerous_for_test(mask: DWORD) -> bool {
        ace_is_dangerous(mask)
    }

    /// Reject reparse points on a path and its ancestors (test seam). This
    /// check deliberately does not require trusted ownership, so it isolates
    /// reparse detection from ACL policy.
    pub fn reject_reparse(path: &Path) -> RegistryResult<()> {
        let mut current = path;
        loop {
            let handle = open_raw(
                current,
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_DELETE,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                None,
            )?;
            let _guard = HandleGuard(handle);
            let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
            if unsafe { GetFileInformationByHandle(handle, &mut info) } != TRUE {
                return Err(Refusal::InvalidSecurityMetadata);
            }
            if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return Err(Refusal::UnsafePath);
            }
            match current.parent() {
                Some(parent) if !parent.as_os_str().is_empty() => current = parent,
                _ => break,
            }
        }
        Ok(())
    }

    pub fn validate_file_security(path: &Path) -> RegistryResult<()> {
        let handle = open_verified(
            path,
            path,
            false,
            false,
            None,
            AclMode::Strict,
            ObjectKind::Any,
        )?;
        let _guard = HandleGuard(handle);
        Ok(())
    }

    /// Verify the registry leaf strictly as a regular, non-reparse file with a
    /// trusted owner/DACL. A directory named `registry.json` is refused here;
    /// it is never deleted or silently replaced.
    pub fn validate_registry_leaf(path: &Path) -> RegistryResult<()> {
        let handle = open_verified(
            path,
            path,
            false,
            false,
            None,
            AclMode::Strict,
            ObjectKind::File,
        )?;
        let _guard = HandleGuard(handle);
        Ok(())
    }

    /// Verify the exact protected location and the full security of the file
    /// installed there. No directory is created on this read-only path.
    pub fn validate_registry_location(path: &Path) -> RegistryResult<()> {
        let _directory_pins = verify_location_structure(path, false)?;
        validate_registry_leaf(path)
    }
}

#[cfg(not(windows))]
pub fn refuse_off_windows() -> Refusal {
    Refusal::ElevatedAuthorityRequired
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    use winapi::shared::minwindef::{DWORD, LPVOID, TRUE};
    #[cfg(windows)]
    use winapi::um::winbase::LocalFree;
    #[cfg(windows)]
    use winapi::um::winnt::{PACL, PSID};

    fn binding(sid: &str, owner: &str, revoked: bool, revision: u64) -> OwnerBinding {
        OwnerBinding {
            sid: sid.to_owned(),
            owner: owner.to_owned(),
            revoked,
            revision,
        }
    }

    fn registry(generation: u64, bindings: Vec<OwnerBinding>) -> OwnerRegistry {
        OwnerRegistry {
            schema_version: OWNER_REGISTRY_SCHEMA.to_owned(),
            generation,
            bindings,
        }
    }

    #[test]
    fn registry_leaf_is_never_a_parent_directory_component() {
        // Deterministic structural regression for the Dispatch 337 defect:
        // the old code iterated every component, including registry.json, and
        // passed it to verify_or_create_dir(). The pure split must exclude it.
        let relative_text = PROGRAMDATA_RELATIVE_PATH.replace('/', "\\");
        let (parents, leaf) =
            registry_path_parts(Path::new(&relative_text)).expect("split registry path");
        assert_eq!(parents, vec!["PrometheOS", "Lite", "owner-registry"]);
        assert_eq!(leaf, "registry.json");
        assert!(!parents.iter().any(|p| p == &leaf));
        assert!(registry_path_parts(Path::new("PrometheOS\\Lite")).is_err());
        assert!(registry_path_parts(Path::new("registry.json")).is_err());
    }

    #[test]
    fn rejected_update_does_not_mutate_previous_state() {
        let previous = registry(5, vec![binding(SID_SYSTEM, "owner-a", false, 1)]);
        let snapshot = previous.clone();
        let stale = registry(4, vec![binding(SID_SYSTEM, "attacker", false, 1)]);
        assert!(enforce_monotonic_update(Some(&previous), &stale).is_err());
        assert_eq!(previous, snapshot);
    }

    #[cfg(windows)]
    #[test]
    fn existing_directory_named_registry_json_is_refused() {
        // Isolated, unprivileged proof that the leaf is treated only as a
        // regular file. Nothing is deleted or replaced here.
        let dir = std::env::temp_dir().join(format!("prometheos-leaf-dir-{}", std::process::id()));
        let leaf = dir.join(REGISTRY_FILE_NAME);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&leaf).expect("create a directory named registry.json");
        let result = windows::validate_registry_leaf(&leaf);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(result, Err(Refusal::UnsafePath));
    }

    #[cfg(windows)]
    #[test]
    fn first_install_distinguishes_absent_leaf_from_absent_parent_and_errors() {
        use windows::classify_win32_error as classify;
        assert_eq!(classify(2), Refusal::RegistryAbsent); // ERROR_FILE_NOT_FOUND
        assert_eq!(classify(3), Refusal::UntrustedLocation); // ERROR_PATH_NOT_FOUND
        assert_eq!(classify(5), Refusal::AccessDenied);
        assert_eq!(classify(32), Refusal::SharingViolation);
        // A valid existing document still parses; a corrupt one is refused.
        let valid = canonical_json(&registry(1, vec![binding(SID_SYSTEM, "a", false, 1)]))
            .expect("serialize registry");
        assert!(parse_registry_bytes(&valid).is_ok());
        assert_eq!(
            parse_registry_bytes(b"{not json"),
            Err(Refusal::CorruptRegistry)
        );
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
        let reg = registry(1, vec![binding(SID_ADMINISTRATORS, "owner-a", false, 1)]);
        assert!(validate_registry(&reg).is_ok());
        assert_eq!(
            lookup_owner(&reg, SID_ADMINISTRATORS).unwrap().as_deref(),
            Some("owner-a")
        );
        assert_eq!(lookup_owner(&reg, SID_SYSTEM).unwrap(), None);
    }

    #[test]
    fn revoked_binding_is_a_permanent_miss() {
        let reg = registry(2, vec![binding(SID_ADMINISTRATORS, "owner-a", true, 2)]);
        assert_eq!(lookup_owner(&reg, SID_ADMINISTRATORS).unwrap(), None);
    }

    #[test]
    fn bad_schema_duplicate_sid_and_empty_owner_are_refused() {
        let mut reg = OwnerRegistry::empty(1);
        reg.schema_version = "other-v1".to_owned();
        assert!(validate_registry(&reg).is_err());
        let reg = registry(
            1,
            vec![
                binding(SID_SYSTEM, "a", false, 1),
                binding(SID_SYSTEM, "b", false, 1),
            ],
        );
        assert!(validate_registry(&reg).is_err());
        let reg = registry(1, vec![binding(SID_SYSTEM, "   ", false, 1)]);
        assert!(validate_registry(&reg).is_err());
        assert!(validate_registry(&OwnerRegistry::empty(0)).is_err());
    }

    #[test]
    fn malformed_sid_is_never_resolved() {
        assert!(lookup_owner(&OwnerRegistry::empty(1), "not-a-sid").is_err());
    }

    #[test]
    fn canonical_json_is_deterministic() {
        let reg = registry(7, vec![binding(SID_SYSTEM, "a", false, 1)]);
        assert_eq!(canonical_json(&reg).unwrap(), canonical_json(&reg).unwrap());
    }

    #[test]
    fn monotonic_update_refuses_rollback_revocation_reversal_and_owner_change() {
        let old = registry(7, vec![binding(SID_ADMINISTRATORS, "owner-a", true, 4)]);
        assert!(enforce_monotonic_update(Some(&old), &registry(1, old.bindings.clone())).is_err());
        assert!(
            enforce_monotonic_update(
                Some(&old),
                &registry(8, vec![binding(SID_ADMINISTRATORS, "owner-a", false, 5)])
            )
            .is_err()
        );
        assert!(
            enforce_monotonic_update(
                Some(&old),
                &registry(8, vec![binding(SID_ADMINISTRATORS, "owner-b", true, 5)])
            )
            .is_err()
        );
        assert!(
            enforce_monotonic_update(
                Some(&old),
                &registry(8, vec![binding(SID_ADMINISTRATORS, "owner-a", true, 3)])
            )
            .is_err()
        );
        // Keeping the tombstone and adding a fresh binding is a valid update.
        assert!(
            enforce_monotonic_update(
                Some(&old),
                &registry(
                    8,
                    vec![
                        binding(SID_ADMINISTRATORS, "owner-a", true, 4),
                        binding(SID_SYSTEM, "owner-b", false, 1),
                    ]
                )
            )
            .is_ok()
        );
    }

    #[test]
    fn revocation_tombstone_cannot_be_dropped_by_omission_then_readded() {
        let old = registry(
            3,
            vec![
                binding(SID_ADMINISTRATORS, "owner-a", true, 2),
                binding(SID_SYSTEM, "owner-b", false, 1),
            ],
        );
        // Omit the revoked SID entirely in a higher generation.
        let omitted = registry(4, vec![binding(SID_SYSTEM, "owner-b", false, 1)]);
        assert!(enforce_monotonic_update(Some(&old), &omitted).is_err());
        // Then try to re-add it as "new" and active. This is refused against
        // the real trusted previous state: the tombstone is immutable, so the
        // omission above can never become the committed prior state.
        let readded = registry(
            5,
            vec![
                binding(SID_SYSTEM, "owner-b", false, 1),
                binding(SID_ADMINISTRATORS, "owner-a", false, 1),
            ],
        );
        assert!(enforce_monotonic_update(Some(&old), &readded).is_err());
        // An omitted identity cannot be locally "forgotten" into a state that
        // would permit reintroduction; the only accepted history is one where
        // every binding identity persists.
        assert!(enforce_monotonic_update(Some(&omitted), &old).is_err());
    }

    #[test]
    fn removal_of_any_existing_binding_is_refused() {
        let old = registry(
            2,
            vec![
                binding(SID_SYSTEM, "owner-a", false, 1),
                binding(SID_ADMINISTRATORS, "owner-b", false, 1),
            ],
        );
        let proposed = registry(3, vec![binding(SID_SYSTEM, "owner-a", false, 1)]);
        assert!(enforce_monotonic_update(Some(&old), &proposed).is_err());
    }

    #[test]
    fn stale_concurrent_update_is_refused() {
        let committed = registry(9, vec![binding(SID_SYSTEM, "owner-a", false, 1)]);
        // A writer that read generation 7 and proposes generation 7 or lower
        // must not overwrite the state committed by another writer.
        let stale = registry(7, vec![binding(SID_SYSTEM, "attacker", false, 1)]);
        assert!(enforce_monotonic_update(Some(&committed), &stale).is_err());
        let equal = registry(9, vec![binding(SID_SYSTEM, "attacker", false, 1)]);
        assert!(enforce_monotonic_update(Some(&committed), &equal).is_err());
    }

    #[test]
    fn initial_generation_must_be_positive() {
        assert!(enforce_monotonic_update(None, &OwnerRegistry::empty(0)).is_err());
        assert!(enforce_monotonic_update(None, &OwnerRegistry::empty(1)).is_ok());
    }

    #[test]
    fn oversized_and_malformed_registry_bytes_are_refused() {
        let oversized = vec![b'x'; MAX_REGISTRY_BYTES + 1];
        assert_eq!(
            parse_registry_bytes(&oversized),
            Err(Refusal::CorruptRegistry)
        );
        assert_eq!(
            parse_registry_bytes(b"{not json"),
            Err(Refusal::CorruptRegistry)
        );
        let valid = canonical_json(&registry(1, vec![binding(SID_SYSTEM, "a", false, 1)])).unwrap();
        assert!(parse_registry_bytes(&valid).is_ok());
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
    fn win32_error_classification_distinguishes_first_install_from_failures() {
        use windows::classify_win32_error as classify;
        assert_eq!(classify(2), Refusal::RegistryAbsent);
        assert_eq!(classify(5), Refusal::AccessDenied);
        assert_eq!(classify(32), Refusal::SharingViolation);
        assert_eq!(classify(3), Refusal::UntrustedLocation);
        assert_eq!(classify(80), Refusal::ConcurrentUpdate);
        assert_eq!(classify(183), Refusal::ConcurrentUpdate);
        assert_eq!(classify(1314), Refusal::AccessDenied);
        assert_eq!(classify(122), Refusal::IoFailure);
    }

    #[cfg(windows)]
    #[test]
    fn shared_lock_identity_is_stable_across_calls_and_processes() {
        let path = Path::new("C:\\ProgramData\\PrometheOS\\Lite\\owner-registry\\registry.json");
        let first = windows::test_mutex_name(path);
        let second = windows::test_mutex_name(path);
        assert_eq!(first, second);
        assert!(!first.contains(&std::process::id().to_string()));
        assert_ne!(
            first,
            windows::test_mutex_name(Path::new("C:\\ProgramData\\other\\registry.json"))
        );
    }

    #[cfg(windows)]
    #[test]
    fn concurrent_writers_share_one_mutex() {
        // Worker mode: acquire the SAME named mutex the provisioner uses,
        // perform a read/modify/write of a shared counter, and exit.
        if let Ok(spec) = std::env::var("PROMETHEOS_LOCK_WORKER") {
            let (name, counter) = spec.split_once('|').expect("worker spec");
            let _guard = windows::acquire_test_mutex(name).expect("acquire shared mutex");
            let value: usize = std::fs::read_to_string(counter)
                .unwrap_or_else(|_| "0".to_owned())
                .trim()
                .parse()
                .unwrap_or(0);
            std::thread::sleep(std::time::Duration::from_millis(200));
            std::fs::write(counter, (value + 1).to_string()).expect("write counter");
            return;
        }
        let dir = std::env::temp_dir().join(format!("prometheos-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let counter = dir.join("counter.txt");
        std::fs::write(&counter, "0").expect("seed counter");
        let name = windows::test_mutex_name(&dir.join("registry.json"));
        let spec = format!("{}|{}", name, counter.to_string_lossy());
        let exe = std::env::current_exe().expect("test executable");
        let mut children = Vec::new();
        for _ in 0..2 {
            children.push(
                std::process::Command::new(&exe)
                    .arg("concurrent_writers_share_one_mutex")
                    .env("PROMETHEOS_LOCK_WORKER", &spec)
                    .spawn()
                    .expect("spawn writer"),
            );
        }
        for mut child in children {
            assert!(child.wait().expect("wait for writer").success());
        }
        let value: usize = std::fs::read_to_string(&counter)
            .expect("read counter")
            .trim()
            .parse()
            .expect("parse counter");
        let _ = std::fs::remove_dir_all(&dir);
        // Exactly two serialized read/modify/write cycles: a per-process
        // unique lock would allow a lost update and yield 1.
        assert_eq!(value, 2, "concurrent writers were not mutually exclusive");
    }

    #[cfg(windows)]
    #[test]
    fn old_acl_predicate_would_accept_unsafe_mask() {
        let old_considered_safe = |mask: u32| {
            mask & (winapi::um::winnt::GENERIC_WRITE
                | winapi::um::winnt::WRITE_DAC
                | winapi::um::winnt::WRITE_OWNER)
                == 0
        };
        assert!(old_considered_safe(0x1000_0000));
        assert!(old_considered_safe(0x0000_0046));
        assert!(old_considered_safe(0x0000_0100));
        assert!(windows::ace_is_dangerous_for_test(0x1000_0000));
        assert!(windows::ace_is_dangerous_for_test(0x0000_0046));
        assert!(windows::ace_is_dangerous_for_test(0x0000_0100));
    }

    #[cfg(windows)]
    #[test]
    fn granular_and_generic_write_rights_are_dangerous() {
        assert!(windows::ace_is_dangerous_for_test(0x001F_01FF));
        assert!(windows::ace_is_dangerous_for_test(0x1000_0000));
        assert!(windows::ace_is_dangerous_for_test(0x4000_0000));
        assert!(windows::ace_is_dangerous_for_test(0x0000_0002));
        assert!(windows::ace_is_dangerous_for_test(0x0000_0004));
        assert!(windows::ace_is_dangerous_for_test(0x0000_0040));
        assert!(windows::ace_is_dangerous_for_test(0x0001_0000));
        assert!(windows::ace_is_dangerous_for_test(0x0100_0000)); // ACCESS_SYSTEM_SECURITY
        assert!(windows::ace_is_dangerous_for_test(0x0200_0000)); // MAXIMUM_ALLOWED
        assert!(!windows::ace_is_dangerous_for_test(0x0012_0089)); // FILE_GENERIC_READ
    }

    #[cfg(windows)]
    #[repr(align(8))]
    struct AclBuf([u8; 256]);

    #[cfg(windows)]
    #[test]
    fn everyone_full_control_dacl_is_refused_with_trusted_owner() {
        // Isolates DACL evaluation from ownership: the ACL has a trusted
        // full-control ACE plus an untrusted full-control ACE.
        use std::os::windows::ffi::OsStrExt;
        use winapi::um::securitybaseapi::{AddAccessAllowedAce, InitializeAcl};
        use winapi::um::winnt::{ACL_REVISION, GENERIC_ALL, PACL};
        let revision = ACL_REVISION as u32;
        let mut buf = AclBuf([0u8; 256]);
        let acl: PACL = buf.0.as_mut_ptr() as *mut _;
        assert_eq!(
            unsafe { InitializeAcl(acl, buf.0.len() as u32, revision) },
            winapi::shared::minwindef::TRUE
        );
        for (sid, mask) in [
            (SID_ADMINISTRATORS, GENERIC_ALL),
            (SID_EVERYONE, GENERIC_ALL),
        ] {
            let mut text: Vec<u16> = std::ffi::OsStr::new(sid)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let mut psid: winapi::um::winnt::PSID = std::ptr::null_mut();
            assert_eq!(
                unsafe {
                    winapi::shared::sddl::ConvertStringSidToSidW(text.as_mut_ptr(), &mut psid)
                },
                winapi::shared::minwindef::TRUE
            );
            assert_eq!(
                unsafe { AddAccessAllowedAce(acl, revision, mask, psid) },
                winapi::shared::minwindef::TRUE
            );
            unsafe { winapi::um::winbase::LocalFree(psid) };
        }
        assert_eq!(
            windows::validate_acl_for_test(acl),
            Err(Refusal::InvalidSecurityMetadata)
        );

        // A read-only service identity alongside the trusted ACE is allowed.
        let mut buf2 = AclBuf([0u8; 256]);
        let acl2: PACL = buf2.0.as_mut_ptr() as *mut _;
        assert_eq!(
            unsafe { InitializeAcl(acl2, buf2.0.len() as u32, revision) },
            winapi::shared::minwindef::TRUE
        );
        for (sid, mask) in [
            (SID_ADMINISTRATORS, GENERIC_ALL),
            (SID_EVERYONE, 0x0012_0089), // FILE_GENERIC_READ
        ] {
            let mut text: Vec<u16> = std::ffi::OsStr::new(sid)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let mut psid: winapi::um::winnt::PSID = std::ptr::null_mut();
            assert_eq!(
                unsafe {
                    winapi::shared::sddl::ConvertStringSidToSidW(text.as_mut_ptr(), &mut psid)
                },
                winapi::shared::minwindef::TRUE
            );
            assert_eq!(
                unsafe { AddAccessAllowedAce(acl2, revision, mask, psid) },
                winapi::shared::minwindef::TRUE
            );
            unsafe { winapi::um::winbase::LocalFree(psid) };
        }
        assert!(windows::validate_acl_for_test(acl2).is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn unknown_and_deny_ace_forms_fail_closed() {
        use std::os::windows::ffi::OsStrExt;
        use winapi::um::securitybaseapi::{AddAccessDeniedAce, InitializeAcl};
        use winapi::um::winnt::{ACL_REVISION, GENERIC_ALL, PACL};
        let revision = ACL_REVISION as u32;
        let mut buf = AclBuf([0u8; 256]);
        let acl: PACL = buf.0.as_mut_ptr() as *mut _;
        unsafe { InitializeAcl(acl, buf.0.len() as u32, revision) };
        let mut text: Vec<u16> = std::ffi::OsStr::new(SID_EVERYONE)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut psid: winapi::um::winnt::PSID = std::ptr::null_mut();
        unsafe { winapi::shared::sddl::ConvertStringSidToSidW(text.as_mut_ptr(), &mut psid) };
        assert_eq!(
            unsafe { AddAccessDeniedAce(acl, revision, GENERIC_ALL, psid) },
            winapi::shared::minwindef::TRUE
        );
        unsafe { winapi::um::winbase::LocalFree(psid) };
        assert_eq!(
            windows::validate_acl_for_test(acl),
            Err(Refusal::InvalidSecurityMetadata)
        );
    }

    #[cfg(windows)]
    fn crafted_acl(entries: &[(&str, DWORD, bool)]) -> AclBuf {
        use std::os::windows::ffi::OsStrExt;
        use winapi::um::securitybaseapi::{AddAccessAllowedAce, AddAccessDeniedAce, InitializeAcl};
        use winapi::um::winnt::{ACL_REVISION, PACL};
        let revision = ACL_REVISION as u32;
        let mut buf = AclBuf([0u8; 256]);
        let acl: PACL = buf.0.as_mut_ptr() as *mut _;
        assert_eq!(
            unsafe { InitializeAcl(acl, buf.0.len() as u32, revision) },
            TRUE
        );
        for (sid_text, mask, allow) in entries {
            let wide: Vec<u16> = std::ffi::OsStr::new(sid_text)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let mut sid: PSID = std::ptr::null_mut();
            assert_eq!(
                unsafe { winapi::shared::sddl::ConvertStringSidToSidW(wide.as_ptr(), &mut sid) },
                TRUE
            );
            let added = if *allow {
                unsafe { AddAccessAllowedAce(acl, revision, *mask, sid) }
            } else {
                unsafe { AddAccessDeniedAce(acl, revision, *mask, sid) }
            };
            assert_eq!(added, TRUE);
            unsafe { LocalFree(sid) };
        }
        buf
    }

    #[cfg(windows)]
    #[test]
    fn mutex_acl_accepts_exact_observed_native_full_access() {
        const MUTEX_FULL: DWORD = 0x001F0001;
        let buf = crafted_acl(&[
            (SID_SYSTEM, MUTEX_FULL, true),
            (SID_ADMINISTRATORS, MUTEX_FULL, true),
        ]);
        let acl: PACL = buf.0.as_ptr() as *mut _;
        assert_eq!(windows::validate_mutex_acl_for_test(acl), Ok(()));
    }

    #[cfg(windows)]
    #[test]
    fn mutex_acl_negative_rights_matrix_fails_closed() {
        const MUTEX_FULL: DWORD = 0x001F0001;
        use winapi::um::winnt::{DELETE, GENERIC_ALL, WRITE_DAC, WRITE_OWNER};
        let bad: &[(&str, DWORD)] = &[
            (SID_EVERYONE, MUTEX_FULL),
            (SID_EVERYONE, 0x0010_0000), // SYNCHRONIZE only
            (SID_EVERYONE, 0x0000_0001), // MUTEX_MODIFY_STATE only
            (SID_EVERYONE, WRITE_DAC),
            (SID_EVERYONE, WRITE_OWNER),
            (SID_EVERYONE, DELETE),
            (SID_EVERYONE, GENERIC_ALL),
            (SID_ADMINISTRATORS, 0x0000_0001),
            (SID_ADMINISTRATORS, 0x0010_0000),
            (SID_ADMINISTRATORS, MUTEX_FULL | 0x0000_0002),
        ];
        for (sid, mask) in bad {
            let buf = crafted_acl(&[(*sid, *mask, true)]);
            let acl: PACL = buf.0.as_ptr() as *mut _;
            assert_eq!(
                windows::validate_mutex_acl_for_test(acl),
                Err(Refusal::InvalidSecurityMetadata),
                "mutex ACL must refuse sid={sid} mask=0x{mask:08x}"
            );
        }

        let buf = crafted_acl(&[
            (SID_ADMINISTRATORS, MUTEX_FULL, true),
            (SID_EVERYONE, MUTEX_FULL, false),
        ]);
        let acl: PACL = buf.0.as_ptr() as *mut _;
        assert_eq!(
            windows::validate_mutex_acl_for_test(acl),
            Err(Refusal::InvalidSecurityMetadata)
        );

        let mut empty = AclBuf([0u8; 256]);
        assert_eq!(
            unsafe {
                winapi::um::securitybaseapi::InitializeAcl(
                    empty.0.as_mut_ptr() as *mut _,
                    empty.0.len() as u32,
                    winapi::um::winnt::ACL_REVISION as u32,
                )
            },
            TRUE
        );
        assert_eq!(
            windows::validate_mutex_acl_for_test(empty.0.as_ptr() as *mut _),
            Err(Refusal::InvalidSecurityMetadata)
        );
        assert_eq!(
            windows::validate_mutex_acl_for_test(std::ptr::null_mut()),
            Err(Refusal::InvalidSecurityMetadata)
        );
    }

    #[cfg(windows)]
    #[test]
    fn mutex_inheritance_flag_is_refused() {
        use winapi::um::securitybaseapi::GetAce;
        use winapi::um::winnt::ACE_HEADER;
        const MUTEX_FULL: DWORD = 0x001F0001;
        let mut buf = crafted_acl(&[(SID_ADMINISTRATORS, MUTEX_FULL, true)]);
        let acl: PACL = buf.0.as_mut_ptr() as *mut _;
        let mut ace: LPVOID = std::ptr::null_mut();
        assert_eq!(unsafe { GetAce(acl, 0, &mut ace) }, TRUE);
        unsafe { (*(ace as *mut ACE_HEADER)).AceFlags = winapi::um::winnt::OBJECT_INHERIT_ACE };
        assert_eq!(
            windows::validate_mutex_acl_for_test(acl),
            Err(Refusal::InvalidSecurityMetadata)
        );
    }

    #[cfg(windows)]
    #[test]
    fn production_publication_accepts_kernel_derived_absolute_name() {
        use std::os::windows::ffi::OsStrExt;
        use winapi::shared::minwindef::FALSE;
        use winapi::um::fileapi::{CREATE_NEW, CreateFileW, FlushFileBuffers, WriteFile};
        use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
        use winapi::um::minwinbase::SECURITY_ATTRIBUTES;
        use winapi::um::winbase::FILE_FLAG_BACKUP_SEMANTICS;
        use winapi::um::winnt::{
            DELETE, FILE_SHARE_DELETE, FILE_SHARE_READ, GENERIC_READ, GENERIC_WRITE, READ_CONTROL,
        };

        let base = std::env::temp_dir().join(format!(
            "pr412-lib-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&base).unwrap();
        let wide = |value: &Path| {
            value
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect::<Vec<u16>>()
        };
        let desired = GENERIC_READ | READ_CONTROL | GENERIC_WRITE | DELETE;
        let share = FILE_SHARE_READ | FILE_SHARE_DELETE;
        let mut sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: FALSE,
        };
        let dir = unsafe {
            CreateFileW(
                wide(&base).as_ptr(),
                desired,
                share,
                &mut sa,
                winapi::um::fileapi::OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                std::ptr::null_mut(),
            )
        };
        assert!(dir != INVALID_HANDLE_VALUE && !dir.is_null());

        for (generation, existing) in [(1u64, false), (2u64, true)] {
            let dest_path = base.join("registry.json");
            if existing {
                std::fs::write(&dest_path, b"old").unwrap();
            }
            let temp_path = base.join(format!("temp-{generation}.tmp"));
            let bytes = format!("{{\"generation\":{generation}}}").into_bytes();
            let temp = unsafe {
                CreateFileW(
                    wide(&temp_path).as_ptr(),
                    desired,
                    share,
                    &mut sa,
                    CREATE_NEW,
                    FILE_FLAG_BACKUP_SEMANTICS,
                    std::ptr::null_mut(),
                )
            };
            assert!(temp != INVALID_HANDLE_VALUE && !temp.is_null());
            let mut written = 0u32;
            assert_eq!(
                unsafe {
                    WriteFile(
                        temp,
                        bytes.as_ptr() as *const _,
                        bytes.len() as u32,
                        &mut written,
                        std::ptr::null_mut(),
                    )
                },
                TRUE
            );
            assert_eq!(unsafe { FlushFileBuffers(temp) }, TRUE);
            assert_eq!(windows::publish_via_handle_for_test(temp), Ok(()));
            unsafe { CloseHandle(temp) };
            assert_eq!(std::fs::read(&dest_path).unwrap(), bytes);
        }
        unsafe { CloseHandle(dir) };
        let diag = windows::last_native_io_diagnostic();
        assert!(diag.publication_attempted && diag.publication_completed);
        assert_eq!(diag.win32_error, 0);
        let _ = std::fs::remove_file(base.join("registry.json"));
        let _ = std::fs::remove_file(base.join("temp-1.tmp"));
        let _ = std::fs::remove_file(base.join("temp-2.tmp"));
        let _ = std::fs::remove_dir(&base);
    }

    #[cfg(windows)]
    #[test]
    fn failed_publication_preserves_existing_destination_bytes() {
        use std::os::windows::ffi::OsStrExt;
        use winapi::shared::minwindef::FALSE;
        use winapi::um::fileapi::{
            CREATE_NEW, CreateFileW, FlushFileBuffers, OPEN_EXISTING, WriteFile,
        };
        use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
        use winapi::um::minwinbase::SECURITY_ATTRIBUTES;
        use winapi::um::winbase::FILE_FLAG_BACKUP_SEMANTICS;
        use winapi::um::winnt::{
            DELETE, FILE_SHARE_DELETE, FILE_SHARE_READ, GENERIC_READ, GENERIC_WRITE, READ_CONTROL,
        };

        let base = std::env::temp_dir().join(format!(
            "pr412-fail-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&base).unwrap();
        let destination = base.join("registry.json");
        let old_bytes = b"old-document";
        std::fs::write(&destination, old_bytes).unwrap();
        let wide = |value: &Path| {
            value
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect::<Vec<u16>>()
        };
        let mut sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: FALSE,
        };
        let temp_path = base.join("temp.tmp");
        let new_bytes = b"new-document";
        let temp = unsafe {
            CreateFileW(
                wide(&temp_path).as_ptr(),
                GENERIC_READ | READ_CONTROL | GENERIC_WRITE | DELETE,
                FILE_SHARE_READ | FILE_SHARE_DELETE,
                &mut sa,
                CREATE_NEW,
                FILE_FLAG_BACKUP_SEMANTICS,
                std::ptr::null_mut(),
            )
        };
        assert!(temp != INVALID_HANDLE_VALUE && !temp.is_null());
        let mut written = 0u32;
        assert_eq!(
            unsafe {
                WriteFile(
                    temp,
                    new_bytes.as_ptr() as *const _,
                    new_bytes.len() as u32,
                    &mut written,
                    std::ptr::null_mut(),
                )
            },
            TRUE
        );
        assert_eq!(unsafe { FlushFileBuffers(temp) }, TRUE);

        // Hold the actual destination without FILE_SHARE_DELETE so publication
        // against the existing document fails at the real destination.
        let holder = unsafe {
            CreateFileW(
                wide(&destination).as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ,
                std::ptr::null_mut(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                std::ptr::null_mut(),
            )
        };
        assert!(holder != INVALID_HANDLE_VALUE && !holder.is_null());
        assert_eq!(
            windows::publish_via_handle_for_test(temp),
            Err(Refusal::AccessDenied)
        );
        let diag = windows::last_native_io_diagnostic();
        assert!(diag.publication_attempted && !diag.publication_completed);
        assert_eq!(diag.win32_error, 5);
        unsafe {
            CloseHandle(temp);
            CloseHandle(holder)
        };
        assert_eq!(std::fs::read(&destination).unwrap(), old_bytes);
        let _ = std::fs::remove_file(&destination);
        let _ = std::fs::remove_file(&temp_path);
        let _ = std::fs::remove_dir(&base);
    }

    #[cfg(windows)]
    #[test]
    fn file_policy_still_refuses_mutex_only_full_access() {
        const MUTEX_FULL: DWORD = 0x001F0001;
        let buf = crafted_acl(&[(SID_ADMINISTRATORS, MUTEX_FULL, true)]);
        let acl: PACL = buf.0.as_ptr() as *mut _;
        assert_eq!(
            windows::validate_acl_for_test(acl),
            Err(Refusal::InvalidSecurityMetadata),
            "filesystem policy must remain unchanged"
        );
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
    fn unprivileged_provisioning_is_denied_before_any_write() {
        if windows::require_provisioning_authority().is_ok() {
            eprintln!("SKIP: elevated host; privileged provisioning acceptance NOT RUN");
            return;
        }
        // A valid protected path, not a nonexistent placeholder.
        let path = default_registry_path().expect("Known Folder path");
        let proposed = registry(1, Vec::new());
        assert_eq!(
            windows::provision_registry(&path, &proposed),
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
        let _ = std::fs::remove_file(&path);
        assert_eq!(
            windows::validate_file_security(&path),
            Err(Refusal::RegistryAbsent)
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

    /// Unique disposable directory whose cleanup is bound to scope exit, so
    /// the test resource is removed even when an assertion panics.
    #[cfg(windows)]
    struct TempDirGuard(PathBuf);

    #[cfg(windows)]
    impl TempDirGuard {
        fn unique(prefix: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "{}-{}-{}",
                prefix,
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ));
            // create_dir never removes a pre-existing path; a collision is a
            // loud failure rather than silent deletion of someone else's data.
            std::fs::create_dir(&path).expect("create unique temp dir");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    #[cfg(windows)]
    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Install a full-control ACE with a locale-invariant SID. Setup failure
    /// is returned to the caller: the caller must fail the test, never SKIP.
    #[cfg(windows)]
    fn grant_full_control_via_icacls(dir: &Path, sid: &str) -> Result<(), String> {
        let output = std::process::Command::new("icacls")
            .arg(dir)
            .arg("/grant")
            .arg(format!("*{sid}:(OI)(CI)F"))
            .output()
            .expect("run icacls");
        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "icacls exit {:?}: {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    }

    #[cfg(windows)]
    #[test]
    fn world_writable_directory_is_refused() {
        // "Everyone" is a localized account name on non-English Windows
        // (pt-BR resolves it to "Todos"); icacls then fails with 1332 and the
        // negative assertion is never exercised. The SID is locale-invariant.
        const EVERYONE_SID: &str = "S-1-1-0";
        let dir = TempDirGuard::unique("prometheos-registry-world");
        grant_full_control_via_icacls(dir.path(), EVERYONE_SID).unwrap_or_else(|err| {
            panic!("ACL setup failed; refusal assertion NOT exercised: {err}")
        });
        // Prove the intended negative condition exists on the OS object before
        // treating the refusal assertion as exercised. A grant that reports
        // success but does not install the ACE fails loudly here.
        let granted = windows::dacl_grants_full_control_to(dir.path(), EVERYONE_SID)
            .expect("read temp dir DACL");
        assert!(
            granted,
            "icacls reported success but no full-control Everyone ACE was installed"
        );
        assert!(
            windows::validate_file_security(dir.path()).is_err(),
            "a fully writable DACL must be refused"
        );
        // TempDirGuard removes this test's own directory on success or panic.
    }

    #[cfg(windows)]
    #[test]
    fn acl_setup_failure_is_detected_instead_of_skipped() {
        // Negative control for the former fail-open branch: an unresolvable
        // SID makes icacls fail. The helper must surface that as an error so
        // the primary test fails instead of reporting a green SKIP.
        let dir = TempDirGuard::unique("prometheos-registry-acl-negctl");
        let err = grant_full_control_via_icacls(dir.path(), "S-1-1-999999999999999999999999")
            .expect_err("an unresolvable SID must fail ACL setup");
        assert!(
            err.contains("icacls exit"),
            "setup failure must be an explicit error, not a silent skip: {err}"
        );
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
