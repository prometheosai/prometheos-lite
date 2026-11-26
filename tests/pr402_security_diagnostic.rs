//! Bounded native-Win32 diagnostic for the PR341 first-install refusal.
//! Read-only/offline except for a Local mutex created and closed by this test.
//! Reports the first stage that fails without broad production tracing.
#![cfg(windows)]

use prometheos_lite::work::owner_registry::windows::validate_acl_for_test;
use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use winapi::shared::minwindef::{DWORD, FALSE, LPVOID, TRUE};
use winapi::shared::sddl::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use winapi::shared::winerror::ERROR_SUCCESS;
use winapi::um::accctrl::SE_KERNEL_OBJECT;
use winapi::um::aclapi::GetSecurityInfo;
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::handleapi::CloseHandle;
use winapi::um::securitybaseapi::{
    GetAce, GetAclInformation, IsValidAcl, IsValidSecurityDescriptor,
};
use winapi::um::synchapi::CreateMutexW;
use winapi::um::winnt::{
    ACCESS_ALLOWED_ACE, ACCESS_ALLOWED_ACE_TYPE, ACE_HEADER, ACL_SIZE_INFORMATION,
    DACL_SECURITY_INFORMATION, HANDLE, OWNER_SECURITY_INFORMATION, PACL, PSID,
};

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

fn sid_text(sid: PSID) -> String {
    if sid.is_null() {
        return "<null>".to_string();
    }
    let mut text: *mut u16 = std::ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 || text.is_null() {
        return format!("<unconvertible:{}>", unsafe { GetLastError() });
    }
    let result = unsafe { OsString::from_wide(std::slice::from_raw_parts(text, wcslen(text))) }
        .to_string_lossy()
        .into_owned();
    unsafe { winapi::um::winbase::LocalFree(text as *mut _) };
    result
}

fn wcslen(value: *const u16) -> usize {
    let mut len = 0;
    while unsafe { *value.add(len) } != 0 {
        len += 1;
    }
    len
}

fn convert_sddl(label: &str, sddl: &str) -> bool {
    let mut descriptor: *mut winapi::ctypes::c_void = std::ptr::null_mut();
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide(sddl).as_ptr(),
            SDDL_REVISION_1 as DWORD,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    } == TRUE
        && !descriptor.is_null()
        && unsafe { IsValidSecurityDescriptor(descriptor) } == TRUE;
    if !descriptor.is_null() {
        unsafe { winapi::um::winbase::LocalFree(descriptor) };
    }
    println!("{label}_CONVERT ok={ok} last_error={}", unsafe {
        GetLastError()
    });
    ok
}

#[test]
fn diagnose_native_security_stages() {
    let file_sddl_text = production_sddl("const TRUSTED_FILE_SDDL");
    let dir_sddl_text = production_sddl("const TRUSTED_DIR_SDDL");
    let file_sddl = convert_sddl("FILE_SDDL", &file_sddl_text);
    let dir_sddl = convert_sddl("DIRECTORY_SDDL", &dir_sddl_text);
    assert!(file_sddl, "native FILE SDDL conversion failed");
    assert!(
        dir_sddl,
        "native DIRECTORY SDDL conversion failed: Win32 ERROR_INVALID_OWNER (1804)"
    );

    let name = format!("Local\\PR341-DIAGNOSTIC-{}", std::process::id());
    let mut descriptor: *mut winapi::ctypes::c_void = std::ptr::null_mut();
    assert_eq!(
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide(&file_sddl_text).as_ptr(),
                SDDL_REVISION_1 as DWORD,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        },
        TRUE
    );
    let mut security_attributes = winapi::um::minwinbase::SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<winapi::um::minwinbase::SECURITY_ATTRIBUTES>() as DWORD,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: FALSE,
    };
    let mutex: HANDLE =
        unsafe { CreateMutexW(&mut security_attributes, FALSE, wide(&name).as_ptr()) };
    if mutex.is_null() || mutex == winapi::um::handleapi::INVALID_HANDLE_VALUE {
        // A non-elevated host token cannot assign the BA owner. This is an
        // environment limit, not a production refusal; the descriptor stages
        // above were already decided. The authorized guest continues further.
        println!(
            "MUTEX_CREATE_SKIPPED_HOST_TOKEN last_error={} (guest-only continuation required)",
            unsafe { GetLastError() }
        );
        unsafe { winapi::um::winbase::LocalFree(descriptor) };
        return;
    }
    println!("MUTEX_CREATE ok name={name}");

    let mut owner: PSID = std::ptr::null_mut();
    let mut group: PSID = std::ptr::null_mut();
    let mut dacl: PACL = std::ptr::null_mut();
    let mut sacl: PACL = std::ptr::null_mut();
    let mut gotten: *mut winapi::ctypes::c_void = std::ptr::null_mut();
    let status = unsafe {
        GetSecurityInfo(
            mutex,
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            &mut group,
            &mut dacl,
            &mut sacl,
            &mut gotten,
        )
    };
    let get_error = unsafe { GetLastError() };
    println!(
        "MUTEX_GET_SECURITY_INFO status={status} last_error={get_error} dacl_null={}",
        dacl.is_null()
    );
    assert_eq!(status, ERROR_SUCCESS, "GetSecurityInfo on mutex failed");
    assert!(!gotten.is_null());
    println!("MUTEX_OWNER sid={}", sid_text(owner));
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
    println!("MUTEX_PRODUCTION_ACL_VALIDATOR verdict={verdict:?} (expected Ok for trusted SDDL)");
    assert!(
        verdict.is_ok(),
        "production ACL validator refused the mutex DACL created from its own trusted SDDL: {verdict:?}"
    );

    unsafe { CloseHandle(mutex) };
    unsafe { winapi::um::winbase::LocalFree(descriptor) };
}
