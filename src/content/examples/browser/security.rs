use std::ptr;

use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Authorization::{
    ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1,
    SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION};

pub(super) fn private_acl(handle: std::os::windows::io::RawHandle) {
    let mut descriptor = ptr::null_mut();
    // SAFETY: handle stays owned by the caller; descriptor receives local memory.
    assert_eq!(
        unsafe {
            GetSecurityInfo(
                handle,
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                &mut descriptor,
            )
        },
        0
    );
    let mut encoded = ptr::null_mut();
    // SAFETY: GetSecurityInfo returned a valid descriptor and encoded is writable.
    assert_ne!(
        unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor,
                SDDL_REVISION_1,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut encoded,
                ptr::null_mut(),
            )
        },
        0
    );
    let mut length = 0;
    // SAFETY: encoded is a Windows-owned null-terminated UTF-16 string.
    while unsafe { *encoded.add(length) } != 0 {
        length += 1;
    }
    let sddl = String::from_utf16(unsafe { std::slice::from_raw_parts(encoded, length) }).unwrap();
    // SAFETY: both allocations came from Windows LocalAlloc APIs.
    unsafe {
        LocalFree(encoded.cast());
        LocalFree(descriptor);
    }
    let (owner, dacl) = sddl.split_once("D:P").expect("Protected DACL");
    let owner = owner.strip_prefix("O:").expect("Owner SID");
    assert_eq!(dacl.matches('(').count(), 1);
    assert!(dacl.starts_with("(A;;"));
    assert!(dacl.ends_with(&format!(";;;{owner})")));
}
