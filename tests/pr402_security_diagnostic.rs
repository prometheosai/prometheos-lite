//! Bounded native-Win32 diagnostic for the PR341 first-install refusal.
//! Read-only/offline except for one Local mutex created and closed by this test.
//! Nonprivileged execution may report an explicit, environment-limited SKIP.
//! Strict guest mode (`PR402_STRICT_DIAGNOSTIC=1`) fails closed on every missing
//! security stage and never converts a skip into success.
#![cfg(windows)]

use prometheos_lite::work::owner_registry::windows::validate_acl_for_test;
use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::time::{SystemTime, UNIX_EPOCH};
use winapi::shared::minwindef::{DWORD, FALSE, LPVOID, TRUE};
use winapi::shared::sddl::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use winapi::shared::winerror::{
    ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_INVALID_DATATYPE, ERROR_INVALID_OWNER,
    ERROR_SUCCESS,
};
use winapi::um::accctrl::SE_KERNEL_OBJECT;
use winapi::um::aclapi::GetSecurityInfo;
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
use winapi::um::securitybaseapi::{
    GetAce, GetAclInformation, IsValidAcl, IsValidSecurityDescriptor,
};
use winapi::um::synchapi::{CreateMutexW, ReleaseMutex, WaitForSingleObject};
use winapi::um::winbase::{LocalFree, WAIT_OBJECT_0};
use winapi::um::winnt::{
    ACCESS_ALLOWED_ACE, ACCESS_ALLOWED_ACE_TYPE, ACE_HEADER, ACL_SIZE_INFORMATION,
    DACL_SECURITY_INFORMATION, HANDLE, OWNER_SECURITY_INFORMATION, PACL, PSID,
};

const STRICT_MODE_VAR: &str = "PR402_STRICT_DIAGNOSTIC";
const TRUSTED_OWNERS: [&str; 2] = ["S-1-5-32-544", "S-1-5-18"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verdict {
    Skip,
    Fail,
}

struct LocalGuard(*mut winapi::ctypes::c_void);
impl Drop for LocalGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { LocalFree(self.0) };
        }
    }
}

struct HandleGuard(HANDLE);
impl Drop for HandleGuard {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(self.0) };
        }
    }
}

fn strict_mode() -> bool {
    std::env::var_os(STRICT_MODE_VAR).as_deref() == Some(OsStr::new("1"))
}

fn win32_error_name(code: DWORD) -> &'static str {
    match code {
        ERROR_INVALID_DATATYPE => "ERROR_INVALID_DATATYPE",
        ERROR_INVALID_OWNER => "ERROR_INVALID_OWNER",
        ERROR_ACCESS_DENIED => "ERROR_ACCESS_DENIED",
        ERROR_ALREADY_EXISTS => "ERROR_ALREADY_EXISTS",
        ERROR_SUCCESS => "ERROR_SUCCESS",
        _ => "UNKNOWN_WIN32_ERROR",
    }
}

fn classify_creation_failure(strict: bool, error: DWORD) -> Verdict {
    // ERROR_ALREADY_EXISTS is authoritative: the name belongs to an object we
    // did not create. Never inspect or acquire that object as our own.
    if error == ERROR_ALREADY_EXISTS {
        return Verdict::Fail;
    }
    if !strict && error == ERROR_INVALID_OWNER {
        Verdict::Skip
    } else {
        Verdict::Fail
    }
}

fn classify_verification_failure(strict: bool, error: DWORD) -> Verdict {
    if !strict && error == ERROR_ACCESS_DENIED {
        Verdict::Skip
    } else {
        Verdict::Fail
    }
}

fn production_sddl(name: &str) -> String {
    let source = include_str!("../src/work/owner_registry.rs");
    let line = source
        .lines()
        .find(|line| line.contains(name))
        .unwrap_or_else(|| panic!("missing production constant {name}"));
    line.split('"')
        .nth(1)
        .unwrap_or_else(|| panic!("missing SDDL literal for {name}"))
        .to_string()
}

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn wcslen(value: *const u16) -> usize {
    let mut len = 0;
    while unsafe { *value.add(len) } != 0 {
        len += 1;
    }
    len
}

fn sid_text(sid: PSID) -> String {
    if sid.is_null() {
        return "<null>".to_string();
    }
    let mut text: *mut u16 = std::ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 || text.is_null() {
        let error = unsafe { GetLastError() };
        return format!("<unconvertible:{error}:{}>", win32_error_name(error));
    }
    let result = unsafe { OsString::from_wide(std::slice::from_raw_parts(text, wcslen(text))) }
        .to_string_lossy()
        .into_owned();
    unsafe { LocalFree(text as *mut _) };
    result
}

/// Captures GetLastError immediately after the failing native call, before
/// any LocalFree, formatting, logging or subsequent Win32 operation.
fn convert_sddl(label: &str, sddl: &str) -> bool {
    let mut descriptor: *mut winapi::ctypes::c_void = std::ptr::null_mut();
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide(sddl).as_ptr(),
            SDDL_REVISION_1 as DWORD,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    };
    let error = unsafe { GetLastError() };
    let guard = LocalGuard(descriptor);
    let valid = !guard.0.is_null() && unsafe { IsValidSecurityDescriptor(guard.0) } == TRUE;
    let ok = converted == TRUE && valid;
    if ok {
        println!("{label}_CONVERT ok=true converted=TRUE valid=TRUE");
    } else {
        println!(
            "{label}_CONVERT ok=false converted={} valid={} error={error} name={}",
            converted == TRUE,
            valid,
            win32_error_name(error)
        );
    }
    ok
}

#[test]
fn diagnose_native_security_stages() {
    let strict = strict_mode();
    println!("DIAGNOSTIC_MODE strict={strict}");

    let file_sddl = production_sddl("const TRUSTED_FILE_SDDL");
    let directory_sddl = production_sddl("const TRUSTED_DIR_SDDL");
    assert!(convert_sddl("FILE_SDDL", &file_sddl));
    assert!(convert_sddl("DIRECTORY_SDDL", &directory_sddl));

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("wall clock")
        .as_nanos();
    let name = format!("Local\\PR341-DIAGNOSTIC-{}-{nonce}", std::process::id());
    let name_wide = wide(&name);

    // No racy pre-check: the authoritative CreateMutexW status decides whether
    // this process created the object. ERROR_ALREADY_EXISTS means the name was
    // collided and must be refused before any security inspection or acquisition.

    let mut descriptor: *mut winapi::ctypes::c_void = std::ptr::null_mut();
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide(&file_sddl).as_ptr(),
            SDDL_REVISION_1 as DWORD,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    };
    let convert_error = unsafe { GetLastError() };
    assert_eq!(
        converted,
        TRUE,
        "trusted FILE SDDL conversion failed: {convert_error} {}",
        win32_error_name(convert_error)
    );
    let descriptor_guard = LocalGuard(descriptor);
    let mut attributes = winapi::um::minwinbase::SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<winapi::um::minwinbase::SECURITY_ATTRIBUTES>() as DWORD,
        lpSecurityDescriptor: descriptor_guard.0,
        bInheritHandle: FALSE,
    };
    let mutex = unsafe { CreateMutexW(&mut attributes, FALSE, name_wide.as_ptr()) };
    let create_error = unsafe { GetLastError() };
    if create_error == ERROR_ALREADY_EXISTS {
        // CreateMutexW returns a handle to the EXISTING object. Close only
        // that unowned handle; never inspect or acquire the collided object.
        if !mutex.is_null() && mutex != INVALID_HANDLE_VALUE {
            drop(HandleGuard(mutex));
        }
        println!("MUTEX_COLLISION name={name}; existing object preserved");
        panic!("diagnostic mutex name collision; refusing to inspect unintended object");
    }
    if mutex.is_null() || mutex == INVALID_HANDLE_VALUE {
        let verdict = classify_creation_failure(strict, create_error);
        println!(
            "MUTEX_CREATE_FAILED error={create_error} name={} verdict={verdict:?}",
            win32_error_name(create_error)
        );
        if verdict == Verdict::Skip {
            println!("DIAGNOSTIC_SKIP mutex_owner_assignment_unavailable_off_guest");
            return;
        }
        panic!("mutex creation failed in strict mode: {create_error}");
    }
    let mutex_guard = HandleGuard(mutex);
    println!("MUTEX_CREATE ok name={name}");

    let mut owner: PSID = std::ptr::null_mut();
    let mut group: PSID = std::ptr::null_mut();
    let mut dacl: PACL = std::ptr::null_mut();
    let mut sacl: PACL = std::ptr::null_mut();
    let mut security_descriptor: *mut winapi::ctypes::c_void = std::ptr::null_mut();
    let status = unsafe {
        GetSecurityInfo(
            mutex,
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            &mut group,
            &mut dacl,
            &mut sacl,
            &mut security_descriptor,
        )
    };
    // GetSecurityInfo returns an authoritative DWORD status. Never substitute a
    // stale GetLastError value for that status.
    let _descriptor_guard = LocalGuard(security_descriptor);
    if status != ERROR_SUCCESS {
        let verdict = classify_verification_failure(strict, status);
        println!(
            "MUTEX_GET_SECURITY_INFO_FAILED status={status} ({}) verdict={verdict:?}",
            win32_error_name(status)
        );
        if verdict == Verdict::Skip {
            println!("DIAGNOSTIC_SKIP kernel_security_query_unavailable_off_guest");
            return;
        }
        panic!("GetSecurityInfo failed in strict mode: status={status}");
    }
    println!(
        "MUTEX_GET_SECURITY_INFO ok status={status} dacl_null={}",
        dacl.is_null()
    );

    let owner_sid = sid_text(owner);
    let owner_trusted = TRUSTED_OWNERS.contains(&owner_sid.as_str());
    println!("MUTEX_OWNER sid={owner_sid} trusted={owner_trusted}");
    if strict {
        assert!(
            owner_trusted,
            "strict guest mode requires trusted mutex owner"
        );
    }

    assert!(!dacl.is_null() && unsafe { IsValidAcl(dacl) } == TRUE);
    let mut info: ACL_SIZE_INFORMATION = unsafe { std::mem::zeroed() };
    assert_eq!(
        unsafe {
            GetAclInformation(
                dacl,
                &mut info as *mut _ as LPVOID,
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as DWORD,
                winapi::um::winnt::AclSizeInformation,
            )
        },
        TRUE
    );
    println!("MUTEX_DACL ace_count={}", info.AceCount);
    for index in 0..info.AceCount {
        let mut ace: LPVOID = std::ptr::null_mut();
        assert_eq!(unsafe { GetAce(dacl, index, &mut ace) }, TRUE);
        let header = unsafe { &*(ace as *const ACE_HEADER) };
        let allowed = unsafe { &*(ace as *const ACCESS_ALLOWED_ACE) };
        println!(
            "MUTEX_ACE[{index}] type={} mask=0x{:08x} sid={}",
            header.AceType,
            allowed.Mask,
            sid_text((&allowed.SidStart as *const DWORD) as PSID)
        );
        assert_eq!(header.AceType, ACCESS_ALLOWED_ACE_TYPE);
    }
    let verdict = validate_acl_for_test(dacl);
    println!("MUTEX_DACL_VALIDATOR verdict={verdict:?}");
    assert!(
        verdict.is_ok(),
        "trusted mutex DACL was refused: {verdict:?}"
    );

    // Prove acquisition, then explicitly release it and verify success. The
    // single HandleGuard closes the handle exactly once at scope exit.
    assert_eq!(
        unsafe { WaitForSingleObject(mutex, 0) },
        WAIT_OBJECT_0,
        "created mutex could not be acquired"
    );
    assert_ne!(
        unsafe { ReleaseMutex(mutex) },
        0,
        "ReleaseMutex failed for the diagnostic-owned mutex"
    );
    drop(mutex_guard);
}

#[test]
fn strict_mode_fails_on_owner_assignment_error() {
    assert_eq!(
        classify_creation_failure(true, ERROR_INVALID_OWNER),
        Verdict::Fail
    );
    assert_eq!(
        classify_creation_failure(false, ERROR_INVALID_OWNER),
        Verdict::Skip
    );
    assert_eq!(
        classify_creation_failure(false, ERROR_ACCESS_DENIED),
        Verdict::Fail
    );
    assert_eq!(
        classify_creation_failure(false, ERROR_ALREADY_EXISTS),
        Verdict::Fail
    );
    assert_eq!(
        classify_creation_failure(true, ERROR_ALREADY_EXISTS),
        Verdict::Fail
    );
}

#[test]
fn strict_mode_fails_on_security_verification_error() {
    assert_eq!(
        classify_verification_failure(true, ERROR_ACCESS_DENIED),
        Verdict::Fail
    );
    assert_eq!(
        classify_verification_failure(false, ERROR_ACCESS_DENIED),
        Verdict::Skip
    );
    assert_eq!(
        classify_verification_failure(false, ERROR_INVALID_DATATYPE),
        Verdict::Fail
    );
}

#[test]
fn existing_mutex_name_is_an_authoritative_collision_and_is_preserved() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("wall clock")
        .as_nanos();
    let name = format!("Local\\PR341-COLLISION-{}-{nonce}", std::process::id());
    let name_wide = wide(&name);
    let first = unsafe { CreateMutexW(std::ptr::null_mut(), FALSE, name_wide.as_ptr()) };
    assert!(!first.is_null() && first != INVALID_HANDLE_VALUE);
    let owner = HandleGuard(first);
    let second = unsafe { CreateMutexW(std::ptr::null_mut(), FALSE, name_wide.as_ptr()) };
    let error = unsafe { GetLastError() };
    assert_eq!(error, ERROR_ALREADY_EXISTS);
    // The extra handle refers to the pre-existing object; close it once and
    // never use it for security inspection or acquisition.
    if !second.is_null() && second != INVALID_HANDLE_VALUE {
        drop(HandleGuard(second));
    }
    assert_eq!(classify_creation_failure(false, error), Verdict::Fail);
    assert_eq!(
        unsafe { WaitForSingleObject(first, 0) },
        WAIT_OBJECT_0,
        "pre-existing object must remain open and usable"
    );
    assert_ne!(unsafe { ReleaseMutex(first) }, 0);
    drop(owner);
}

#[test]
fn win32_error_constants_have_accurate_names() {
    assert_eq!(win32_error_name(1804), "ERROR_INVALID_DATATYPE");
    assert_eq!(win32_error_name(1307), "ERROR_INVALID_OWNER");
    assert_eq!(ERROR_INVALID_DATATYPE, 1804);
    assert_eq!(ERROR_INVALID_OWNER, 1307);
}
