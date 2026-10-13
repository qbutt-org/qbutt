// Current-user protected settings. Plaintext never enters SQLite or diagnostics.

use std::io;

const MAX_SECRET_BYTES: usize = 8192;
const MAX_PROTECTED_BYTES: usize = 64 * 1024;

pub(crate) struct Secret(Vec<u8>);

impl Secret {
    pub(crate) fn from_plaintext(value: &[u8]) -> io::Result<Self> {
        if value.is_empty()
            || value.len() > MAX_SECRET_BYTES
            || !value.iter().all(|byte| byte.is_ascii_graphic())
        {
            return Err(io::Error::other("the API key is invalid"));
        }
        Ok(Self(value.to_vec()))
    }

    pub(crate) fn authorization(&self) -> io::Result<reqwest::header::HeaderValue> {
        let mut value = b"Bearer ".to_vec();
        value.extend_from_slice(&self.0);
        let result = reqwest::header::HeaderValue::from_bytes(&value);
        erase(&mut value);
        let mut header = result.map_err(|_| io::Error::other("the API key is invalid"))?;
        header.set_sensitive(true);
        Ok(header)
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        erase(&mut self.0);
    }
}

fn erase(bytes: &mut [u8]) {
    for byte in bytes {
        // Volatile stores keep the secret wipe observable to the compiler.
        unsafe { std::ptr::write_volatile(byte, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

#[cfg(windows)]
fn dpapi(input: &[u8], decrypt: bool) -> io::Result<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(input.len()).map_err(|_| io::Error::other("secret is too large"))?,
        pbData: input.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    let succeeded = unsafe {
        if decrypt {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if succeeded == 0 {
        return Err(io::Error::last_os_error());
    }
    let limit = if decrypt {
        MAX_SECRET_BYTES
    } else {
        MAX_PROTECTED_BYTES
    };
    let valid = output.cbData > 0 && output.cbData as usize <= limit && !output.pbData.is_null();
    let result = if valid {
        Ok(unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec())
    } else {
        Err(io::Error::other(
            "Windows returned invalid protected settings",
        ))
    };
    if !output.pbData.is_null() {
        if decrypt {
            erase(unsafe { std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize) });
        }
        unsafe { LocalFree(output.pbData.cast()) };
    }
    result
}

#[cfg(not(windows))]
fn dpapi(_: &[u8], _: bool) -> io::Result<Vec<u8>> {
    Err(io::Error::other("protected API keys require Windows"))
}

pub(crate) fn protect(value: &str) -> io::Result<Vec<u8>> {
    let secret = Secret::from_plaintext(value.as_bytes())?;
    dpapi(&secret.0, false)
}

pub(crate) fn unprotect(value: &[u8]) -> io::Result<Secret> {
    if value.is_empty() || value.len() > MAX_PROTECTED_BYTES {
        return Err(io::Error::other("saved protected settings are invalid"));
    }
    let mut plaintext = dpapi(value, true)?;
    let secret = Secret::from_plaintext(&plaintext);
    erase(&mut plaintext);
    secret
}
