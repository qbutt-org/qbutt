use std::ffi::c_void;
use std::io;
use std::mem::size_of;
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::ptr;

use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    TokenUser,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// The descriptor is used synchronously by Windows while creating an object.
pub(crate) struct CurrentUserSecurity(LocalAllocation);

struct LocalAllocation(*mut c_void);

impl Drop for LocalAllocation {
    fn drop(&mut self) {
        // SAFETY: Windows allocated this pointer with LocalAlloc.
        unsafe {
            LocalFree(self.0);
        }
    }
}

impl CurrentUserSecurity {
    pub(crate) fn new() -> io::Result<Self> {
        let mut token = ptr::null_mut();
        // SAFETY: token is writable; GetCurrentProcess returns a borrowed handle.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: OpenProcessToken returned a new owned handle.
        let _token_owner = unsafe { OwnedHandle::from_raw_handle(token) };
        let mut bytes = 0;
        // SAFETY: first call obtains the required buffer size.
        unsafe {
            GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut bytes);
        }
        if bytes < size_of::<TOKEN_USER>() as u32 || bytes > 64 * 1024 {
            return Err(io::Error::last_os_error());
        }
        // TOKEN_USER contains a pointer and therefore needs pointer alignment.
        let mut storage = vec![0usize; (bytes as usize).div_ceil(size_of::<usize>())];
        // SAFETY: storage is aligned and at least bytes long.
        if unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                storage.as_mut_ptr().cast(),
                bytes,
                &mut bytes,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: successful GetTokenInformation initialized TOKEN_USER.
        let user = unsafe { &*storage.as_ptr().cast::<TOKEN_USER>() };
        let mut sid = ptr::null_mut();
        // SAFETY: the SID belongs to storage and remains live throughout this call.
        if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let _sid_owner = LocalAllocation(sid.cast());
        let mut length = 0;
        // SAFETY: ConvertSidToStringSidW returned a null-terminated UTF-16 SID.
        while unsafe { *sid.add(length) } != 0 {
            length += 1;
        }
        // SAFETY: length covers only the initialized string before its terminator.
        let sid = String::from_utf16(unsafe { std::slice::from_raw_parts(sid, length) })
            .map_err(|_| io::Error::other("Invalid Windows user SID"))?;
        let sddl: Vec<u16> = format!("O:{sid}D:P(A;;GA;;;{sid})")
            .encode_utf16()
            .chain([0])
            .collect();
        let mut descriptor = ptr::null_mut();
        // SAFETY: sddl is terminated, descriptor is writable and receives owned memory.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(LocalAllocation(descriptor)))
    }

    pub(crate) fn descriptor(&self) -> PSECURITY_DESCRIPTOR {
        self.0.0
    }

    pub(crate) fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.descriptor(),
            bInheritHandle: 0,
        }
    }
}

pub(crate) fn create_pipe(name: &str, first: bool) -> io::Result<NamedPipeServer> {
    let security = CurrentUserSecurity::new()?;
    let mut attributes = security.attributes();
    // SAFETY: attributes and its descriptor stay valid until CreateNamedPipeW
    // returns. The protected DACL grants only the process user's SID access.
    unsafe {
        ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .max_instances(5)
            .in_buffer_size(4096)
            .out_buffer_size(4096)
            .create_with_security_attributes_raw(
                name,
                (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
            )
    }
}
