use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::BrowserError;

pub(crate) struct Discovery {
    file: File,
    path: PathBuf,
    directory: PathBuf,
    pipe: String,
}

#[derive(Deserialize, Serialize)]
struct Entry {
    pipe: String,
    pid: u32,
}

impl Discovery {
    pub(crate) fn open(path: PathBuf) -> Result<Self, BrowserError> {
        Self::open_private(path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::WouldBlock {
                BrowserError::OwnerExists
            } else {
                BrowserError::StartFailed
            }
        })
    }

    fn open_private(path: PathBuf) -> std::io::Result<Self> {
        let parent = path.parent().ok_or_else(invalid)?;
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(parent)?;
        if directory.metadata()?.uid() != unsafe { libc::geteuid() } {
            return Err(invalid());
        }
        directory.set_permissions(fs::Permissions::from_mode(0o700))?;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
        {
            return Err(invalid());
        }
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        // flock remains attached to this inode until discovery is retired.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut previous = Vec::new();
        (&mut file).take(8193).read_to_end(&mut previous)?;
        if previous.len() > 8192 {
            return Err(invalid());
        }
        if let Ok(entry) = serde_json::from_slice::<Entry>(&previous) {
            if entry.pid != 0
                && entry.pid <= libc::pid_t::MAX as u32
                && (unsafe { libc::kill(entry.pid as libc::pid_t, 0) } == 0
                    || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM))
            {
                return Err(std::io::Error::from(std::io::ErrorKind::WouldBlock));
            }
            retire_stale_socket(Path::new(&entry.pipe))?;
        }
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute() && private_directory(path).is_ok())
            .unwrap_or_else(std::env::temp_dir);
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(std::io::Error::other)?;
        let name: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let directory = base.join(format!("qbutt-browser-{name}"));
        let socket = directory.join("broker.sock");
        // Linux sockaddr_un has 108 bytes including its terminating NUL.
        if socket.as_os_str().as_encoded_bytes().len() >= 108 {
            return Err(invalid());
        }
        let pipe = socket.to_str().ok_or_else(invalid)?.into();
        DirBuilder::new().mode(0o700).create(&directory)?;
        Ok(Self {
            file,
            path,
            directory,
            pipe,
        })
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
        // A replacement path must never be removed on behalf of our locked inode.
        if let (Ok(owned), Ok(current)) = (self.file.metadata(), fs::symlink_metadata(&self.path))
            && owned.dev() == current.dev()
            && owned.ino() == current.ino()
        {
            let _ = fs::remove_file(&self.path);
        }
        let _ = fs::remove_file(&self.pipe);
        let _ = fs::remove_dir(&self.directory);
    }
}

fn private_directory(path: &Path) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir()
        && metadata.uid() == unsafe { libc::geteuid() }
        && metadata.mode() & 0o777 == 0o700
    {
        Ok(())
    } else {
        Err(invalid())
    }
}

fn retire_stale_socket(path: &Path) -> std::io::Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    let owned_name = parent
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix("qbutt-browser-"))
        .is_some_and(|name| name.len() == 32 && name.bytes().all(|byte| byte.is_ascii_hexdigit()));
    if !path.is_absolute()
        || path.file_name() != Some(std::ffi::OsStr::new("broker.sock"))
        || !owned_name
    {
        return Ok(());
    }
    if !parent.try_exists()? {
        return Ok(());
    }
    private_directory(parent)?;
    match fs::symlink_metadata(path) {
        Ok(metadata)
            if metadata.file_type().is_socket()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o777 == 0o600 =>
        {
            fs::remove_file(path)?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        _ => return Err(invalid()),
    }
    fs::remove_dir(parent)
}

fn invalid() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "Invalid private browser path",
    )
}
