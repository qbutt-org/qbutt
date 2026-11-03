//! The desktop keyring owns the encryption key; SQLite contains authenticated
//! ciphertext only. A missing or locked Secret Service fails without a fallback.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::fs::OpenOptions;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::ptr;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;

use super::erase;

const MAGIC: &[u8] = b"qbutt-secret-v1\0";
const ASSOCIATED_DATA: &[u8] = b"org.qbutt.ContentSettings.v1";

#[link(name = "secret-1")]
unsafe extern "C" {
    fn secret_schema_new(name: *const c_char, flags: c_int, ...) -> *mut c_void;
    fn secret_schema_unref(schema: *mut c_void);
    fn secret_password_lookup_sync(
        schema: *const c_void,
        cancellable: *mut c_void,
        error: *mut *mut c_void,
        ...
    ) -> *mut c_char;
    fn secret_password_store_sync(
        schema: *const c_void,
        collection: *const c_char,
        label: *const c_char,
        password: *const c_char,
        cancellable: *mut c_void,
        error: *mut *mut c_void,
        ...
    ) -> c_int;
    fn secret_password_free(password: *mut c_char);
}

#[link(name = "glib-2.0")]
unsafe extern "C" {
    fn g_error_free(error: *mut c_void);
}

struct Schema(*mut c_void);

impl Drop for Schema {
    fn drop(&mut self) {
        unsafe { secret_schema_unref(self.0) };
    }
}

struct Key([u8; 32]);

impl Drop for Key {
    fn drop(&mut self) {
        erase(&mut self.0);
    }
}

pub(super) fn protect(plaintext: &[u8]) -> io::Result<Vec<u8>> {
    let key = key(true)?;
    let cipher = Aes256Gcm::new_from_slice(&key.0).map_err(|_| unavailable())?;
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut nonce).map_err(|_| unavailable())?;
    let encrypted = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: ASSOCIATED_DATA,
            },
        )
        .map_err(|_| unavailable())?;
    let mut result = Vec::with_capacity(MAGIC.len() + nonce.len() + encrypted.len());
    result.extend_from_slice(MAGIC);
    result.extend_from_slice(&nonce);
    result.extend_from_slice(&encrypted);
    Ok(result)
}

pub(super) fn unprotect(value: &[u8]) -> io::Result<Vec<u8>> {
    let body = value
        .strip_prefix(MAGIC)
        .filter(|body| body.len() > 12 + 16)
        .ok_or_else(|| io::Error::other("saved protected settings are invalid"))?;
    let key = key(false)?;
    let cipher = Aes256Gcm::new_from_slice(&key.0).map_err(|_| unavailable())?;
    cipher
        .decrypt(
            Nonce::from_slice(&body[..12]),
            Payload {
                msg: &body[12..],
                aad: ASSOCIATED_DATA,
            },
        )
        .map_err(|_| io::Error::other("saved protected settings could not be authenticated"))
}

fn key(create: bool) -> io::Result<Key> {
    // Serialize the first key creation across qbutt profiles and processes. The
    // file contains no key material and is intentionally kept for stable flock.
    let lock_path = std::path::Path::new("/tmp")
        .join(format!("qbutt-keyring-{}.lock", unsafe { libc::geteuid() }));
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(lock_path)?;
    let metadata = lock.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o600
        || metadata.nlink() != 1
    {
        return Err(unavailable());
    }
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // String attribute type and default schema flags are both zero in libsecret.
    let schema = Schema(unsafe {
        secret_schema_new(
            c"org.qbutt.ContentSettings".as_ptr(),
            0,
            c"application".as_ptr(),
            0 as c_int,
            ptr::null::<c_char>(),
        )
    });
    if schema.0.is_null() {
        return Err(unavailable());
    }
    let mut error = ptr::null_mut();
    let password = unsafe {
        secret_password_lookup_sync(
            schema.0,
            ptr::null_mut(),
            &mut error,
            c"application".as_ptr(),
            c"qbutt".as_ptr(),
            ptr::null::<c_char>(),
        )
    };
    if !error.is_null() {
        if !password.is_null() {
            unsafe { secret_password_free(password) };
        }
        unsafe { g_error_free(error) };
        return Err(unavailable());
    }
    if !password.is_null() {
        let encoded = unsafe { CStr::from_ptr(password) }.to_bytes();
        let decoded = if encoded.len() == 44 {
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|_| unavailable())
        } else {
            Err(unavailable())
        };
        unsafe { secret_password_free(password) };
        let mut bytes = decoded?;
        let key = bytes
            .as_slice()
            .try_into()
            .map(Key)
            .map_err(|_| unavailable());
        erase(&mut bytes);
        return key;
    }
    if !create {
        return Err(unavailable());
    }
    let mut key = Key([0u8; 32]);
    getrandom::fill(&mut key.0).map_err(|_| unavailable())?;
    let password = CString::new(base64::engine::general_purpose::STANDARD.encode(&key.0))
        .map_err(|_| unavailable())?;
    let stored = unsafe {
        secret_password_store_sync(
            schema.0,
            ptr::null(),
            c"qbutt protected settings".as_ptr(),
            password.as_ptr(),
            ptr::null_mut(),
            &mut error,
            c"application".as_ptr(),
            c"qbutt".as_ptr(),
            ptr::null::<c_char>(),
        )
    };
    let mut bytes = password.into_bytes_with_nul();
    erase(&mut bytes);
    if !error.is_null() {
        unsafe { g_error_free(error) };
    }
    if stored == 0 || !error.is_null() {
        return Err(unavailable());
    }
    Ok(key)
}

fn unavailable() -> io::Error {
    io::Error::other("The desktop Secret Service keyring is unavailable or locked")
}
