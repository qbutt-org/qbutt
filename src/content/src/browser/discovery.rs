use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use windows_sys::Win32::Foundation::{
    ERROR_INVALID_PARAMETER, GENERIC_READ, GENERIC_WRITE, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::{
    DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, SetKernelObjectSecurity,
};
use windows_sys::Win32::Storage::FileSystem::{
    DELETE, FILE_DISPOSITION_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ, FileDispositionInfo,
    SetFileInformationByHandle, WRITE_DAC,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
};

use super::BrowserError;
use super::security::CurrentUserSecurity;

pub(crate) struct Discovery {
    file: File,
    pipe: String,
}

#[derive(Deserialize, Serialize)]
struct Entry {
    pipe: String,
    pid: u32,
}

impl Discovery {
    pub(crate) fn open(path: PathBuf) -> Result<Self, BrowserError> {
        let parent = path.parent().ok_or(BrowserError::StartFailed)?;
        fs::create_dir_all(parent).map_err(|_| BrowserError::StartFailed)?;
        // Holding write access without FILE_SHARE_WRITE is the owner lock. Native
        // hosts may read discovery; another profile cannot truncate or replace it.
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .access_mode(GENERIC_READ | GENERIC_WRITE | WRITE_DAC | DELETE)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
            .open(path)
            .map_err(|error| {
                if error.raw_os_error() == Some(32) {
                    BrowserError::OwnerExists
                } else {
                    BrowserError::StartFailed
                }
            })?;
        let mut previous = Vec::new();
        (&mut file)
            .take(8193)
            .read_to_end(&mut previous)
            .map_err(|_| BrowserError::StartFailed)?;
        if previous.len() > 8192 {
            return Err(BrowserError::StartFailed);
        }
        if let Ok(entry) = serde_json::from_slice::<Entry>(&previous)
            && process_alive(entry.pid)
        {
            return Err(BrowserError::OwnerExists);
        }
        let security = CurrentUserSecurity::new().map_err(|_| BrowserError::StartFailed)?;
        // SAFETY: the file owns the handle and security owns a valid descriptor.
        if unsafe {
            SetKernelObjectSecurity(
                file.as_raw_handle(),
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                security.descriptor(),
            )
        } == 0
        {
            return Err(BrowserError::StartFailed);
        }
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(|_| BrowserError::StartFailed)?;
        random[6] = (random[6] & 0x0f) | 0x40;
        random[8] = (random[8] & 0x3f) | 0x80;
        let hex: String = random
            .into_iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let pipe = format!(
            r"\\.\pipe\qbutt-content-browser-{}-{}-{}-{}-{}",
            &hex[..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..]
        );
        Ok(Self { file, pipe })
    }

    pub(crate) fn publish(&mut self) -> Result<(), BrowserError> {
        let bytes = serde_json::to_vec(&Entry {
            pipe: self.pipe.clone(),
            pid: std::process::id(),
        })
        .map_err(|_| BrowserError::StartFailed)?;
        self.file
            .seek(SeekFrom::Start(0))
            .map_err(|_| BrowserError::StartFailed)?;
        self.file
            .set_len(0)
            .map_err(|_| BrowserError::StartFailed)?;
        self.file
            .write_all(&bytes)
            .map_err(|_| BrowserError::StartFailed)?;
        self.file.flush().map_err(|_| BrowserError::StartFailed)
    }

    pub(crate) fn pipe_name(&self) -> &str {
        &self.pipe
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
        // SAFETY: deletion applies to our still-locked file object, never a path
        // which another profile could have replaced between close and unlink.
        unsafe {
            SetFileInformationByHandle(
                self.file.as_raw_handle(),
                FileDispositionInfo,
                (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                std::mem::size_of_val(&disposition) as u32,
            );
        }
    }
}

fn process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    // SAFETY: no pointers are passed; the returned handle is owned on success.
    let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        return std::io::Error::last_os_error().raw_os_error()
            != Some(ERROR_INVALID_PARAMETER as i32);
    }
    // SAFETY: OpenProcess returned an owned process handle.
    let _owner = unsafe { OwnedHandle::from_raw_handle(handle) };
    // SAFETY: handle remains live; zero timeout never blocks the runtime.
    unsafe { WaitForSingleObject(handle, 0) == WAIT_TIMEOUT }
}
