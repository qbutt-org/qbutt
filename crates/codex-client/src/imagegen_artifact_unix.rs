use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};

use serde_json::Value;

pub(crate) struct ImageArtifact {
    root: PathBuf,
    directory: File,
    thread: Option<String>,
    artifact: Option<(File, CString, File)>,
}

impl ImageArtifact {
    pub(crate) fn new() -> io::Result<Self> {
        let root = std::env::var_os("CODEX_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
            .ok_or_else(|| io::Error::other("the Codex profile directory is unavailable"))?;
        if !root.is_absolute() {
            return Err(io::Error::other("generated-image root must be absolute"));
        }
        let mut directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open("/")?;
        for component in root.components() {
            match component {
                Component::RootDir => {}
                Component::Normal(name) => {
                    directory = open_at(&directory, name.as_encoded_bytes(), true)?
                }
                _ => {
                    return Err(io::Error::other(
                        "generated-image root contains an unsafe component",
                    ));
                }
            }
        }
        Ok(Self {
            root,
            directory,
            thread: None,
            artifact: None,
        })
    }

    pub(crate) fn claim_thread(&mut self, thread: &str) -> io::Result<()> {
        let thread = artifact_segment(thread)?;
        match open_at(&self.directory, b"generated_images", true) {
            Ok(images) => match open_at(&images, thread.as_bytes(), true) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Ok(_) => {
                    return Err(io::Error::other(
                        "the generated-image thread directory already exists",
                    ));
                }
                Err(error) => return Err(error),
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        self.thread = Some(thread);
        Ok(())
    }

    pub(crate) fn capture(&mut self, item: &Value, png: &[u8]) -> io::Result<()> {
        let saved = match item.get("savedPath") {
            None | Some(Value::Null) => return Ok(()),
            Some(Value::String(path)) if path.len() <= 32768 => Path::new(path),
            _ => return Err(io::Error::other("invalid generated-image savedPath")),
        };
        let thread = self
            .thread
            .as_ref()
            .ok_or_else(|| io::Error::other("generated-image thread was not claimed"))?;
        let call = item
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| io::Error::other("generated-image call ID is missing"))?;
        let filename = format!("{}.png", artifact_segment(call)?);
        let expected = self
            .root
            .join("generated_images")
            .join(thread)
            .join(&filename);
        if saved != expected {
            return Err(io::Error::other(
                "generated-image savedPath does not match the owned thread and call",
            ));
        }
        let images = open_at(&self.directory, b"generated_images", true)?;
        let directory = open_at(&images, thread.as_bytes(), true)?;
        let mut file = open_at(&directory, filename.as_bytes(), false)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.nlink() != 1 || metadata.len() != png.len() as u64 {
            return Err(io::Error::other(
                "generated-image artifact is not the expected single-link regular PNG",
            ));
        }
        let mut buffer = [0u8; 8192];
        for chunk in png.chunks(buffer.len()) {
            file.read_exact(&mut buffer[..chunk.len()])?;
            if buffer[..chunk.len()] != *chunk {
                return Err(io::Error::other(
                    "generated-image artifact bytes do not match the completed event",
                ));
            }
        }
        self.artifact = Some((directory, CString::new(filename)?, file));
        Ok(())
    }

    /// Called after reaping the owned process group. Rename into a private
    /// directory before checking identity, so a replaced path is never deleted.
    pub(crate) fn cleanup(&mut self) -> io::Result<()> {
        let Some((directory, name, file)) = self.artifact.take() else {
            return Ok(());
        };
        let mut random = [0u8; 8];
        getrandom::fill(&mut random).map_err(io::Error::other)?;
        let quarantine = CString::new(format!(
            ".qbutt-cleanup-{:016x}",
            u64::from_le_bytes(random)
        ))?;
        if unsafe { libc::mkdirat(directory.as_raw_fd(), quarantine.as_ptr(), 0o700) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let private = open_at(&directory, quarantine.as_bytes(), true)?;
        let result = (|| {
            if unsafe {
                libc::renameat(
                    directory.as_raw_fd(),
                    name.as_ptr(),
                    private.as_raw_fd(),
                    c"image.png".as_ptr(),
                )
            } != 0
            {
                return Err(io::Error::last_os_error());
            }
            let moved = open_at(&private, b"image.png", false)?.metadata()?;
            let owned = file.metadata()?;
            if moved.dev() != owned.dev() || moved.ino() != owned.ino() || moved.nlink() != 1 {
                // Preserve a replacement and avoid overwriting a concurrently recreated name.
                unsafe {
                    libc::renameat2(
                        private.as_raw_fd(),
                        c"image.png".as_ptr(),
                        directory.as_raw_fd(),
                        name.as_ptr(),
                        libc::RENAME_NOREPLACE,
                    )
                };
                return Err(io::Error::other(
                    "generated-image artifact changed before cleanup; files were preserved",
                ));
            }
            if unsafe { libc::unlinkat(private.as_raw_fd(), c"image.png".as_ptr(), 0) } != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        })();
        let removed = unsafe {
            libc::unlinkat(
                directory.as_raw_fd(),
                quarantine.as_ptr(),
                libc::AT_REMOVEDIR,
            )
        };
        result?;
        if removed != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

fn open_at(directory: &File, name: &[u8], is_directory: bool) -> io::Result<File> {
    let name = CString::new(name)?;
    let flags = libc::O_RDONLY
        | libc::O_NOFOLLOW
        | libc::O_CLOEXEC
        | if is_directory {
            libc::O_DIRECTORY
        } else {
            libc::O_NONBLOCK
        };
    let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: openat returned a new owned descriptor.
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn artifact_segment(value: &str) -> io::Result<String> {
    if value.is_empty() || value.len() > 256 {
        return Err(io::Error::other("invalid generated-image identity"));
    }
    Ok(value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect())
}
