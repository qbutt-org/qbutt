//! Owns only the PNG saved by this request's completed image-generation item.
//! The event path is checked against a derived path, never used as a delete target.

#[cfg(windows)]
mod windows {
    use std::fs::{File, OpenOptions};
    use std::io::{self, Read};
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use std::os::windows::io::AsRawHandle;
    use std::path::{Component, Path, PathBuf, Prefix};

    use serde_json::Value;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, DELETE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, FileDispositionInfo,
        GetFileInformationByHandle, SetFileInformationByHandle,
    };
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_Profile, SHGetKnownFolderPath};

    pub(crate) struct ImageArtifact {
        root: PathBuf,
        thread: Option<PathBuf>,
        parents: Vec<File>,
        file: Option<File>,
    }

    impl ImageArtifact {
        pub(crate) fn new() -> io::Result<Self> {
            let configured = std::env::var("CODEX_HOME")
                .ok()
                .filter(|value| !value.is_empty());
            let root = match &configured {
                Some(value) => PathBuf::from(value),
                None => profile_directory()?.join(".codex"),
            };
            // Lock the original spelling before canonicalizing an explicit CODEX_HOME.
            // An existing junction must never become an accepted canonical target.
            let parents = lock_directory_chain(&root)?;
            let root = if configured.is_some() {
                root.canonicalize()?
            } else {
                root
            };
            Ok(Self {
                root,
                thread: None,
                parents,
                file: None,
            })
        }

        pub(crate) fn claim_thread(&mut self, thread: &str) -> io::Result<()> {
            let thread = artifact_segment(thread)?;
            let images = self.root.join("generated_images");
            match lock_directory(&images) {
                Ok(parent) => self.parents.push(parent),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            let path = images.join(thread);
            match path.symlink_metadata() {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Ok(_) => {
                    return Err(io::Error::other(
                        "the generated-image thread directory already exists",
                    ));
                }
                Err(error) => return Err(error),
            }
            self.thread = Some(path);
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
            let expected = thread.join(format!("{}.png", artifact_segment(call)?));
            if saved != expected {
                return Err(io::Error::other(
                    "generated-image savedPath does not match the owned thread and call",
                ));
            }
            // Keep every directory handle open without FILE_SHARE_DELETE. The final
            // file also denies writers, so validation and deletion refer to one object.
            // Root ancestry is already pinned; only these two directories are new.
            self.parents
                .push(lock_directory(&self.root.join("generated_images"))?);
            self.parents.push(lock_directory(thread)?);
            let mut file = OpenOptions::new()
                .read(true)
                .access_mode(windows_sys::Win32::Foundation::GENERIC_READ | DELETE)
                .share_mode(FILE_SHARE_READ)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(&expected)?;
            let mut info = BY_HANDLE_FILE_INFORMATION::default();
            if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
                return Err(io::Error::last_os_error());
            }
            let size = (u64::from(info.nFileSizeHigh) << 32) | u64::from(info.nFileSizeLow);
            if info.dwFileAttributes & (FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DIRECTORY)
                != 0
                || info.nNumberOfLinks != 1
                || size != png.len() as u64
            {
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
            self.file = Some(file);
            Ok(())
        }

        /// Called only after the owned app-server process has been reaped.
        pub(crate) fn cleanup(&mut self) -> io::Result<()> {
            let Some(file) = self.file.take() else {
                return Ok(());
            };
            let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
            let deleted = unsafe {
                SetFileInformationByHandle(
                    file.as_raw_handle(),
                    FileDispositionInfo,
                    (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                    std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
                )
            };
            if deleted == 0 {
                return Err(io::Error::last_os_error());
            }
            // Closing this exact handle completes the unlink. No path is re-opened.
            drop(file);
            Ok(())
        }
    }

    fn profile_directory() -> io::Result<PathBuf> {
        let mut pointer = std::ptr::null_mut();
        let result = unsafe {
            SHGetKnownFolderPath(&FOLDERID_Profile, 0, std::ptr::null_mut(), &mut pointer)
        };
        if result < 0 || pointer.is_null() {
            return Err(io::Error::other(format!(
                "could not resolve the Codex profile directory: HRESULT {result:#x}"
            )));
        }
        let mut length = 0;
        while unsafe { *pointer.add(length) } != 0 {
            length += 1;
        }
        let path = {
            use std::os::windows::ffi::OsStringExt;
            std::ffi::OsString::from_wide(unsafe { std::slice::from_raw_parts(pointer, length) })
        };
        unsafe { CoTaskMemFree(pointer.cast()) };
        Ok(PathBuf::from(path))
    }

    fn artifact_segment(value: &str) -> io::Result<String> {
        if value.is_empty() || value.len() > 256 {
            return Err(io::Error::other("invalid generated-image identity"));
        }
        // Matches Codex image_generation_artifact_path. It cannot introduce separators.
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

    fn lock_directory_chain(path: &Path) -> io::Result<Vec<File>> {
        let mut components = path.components();
        let Some(Component::Prefix(prefix)) = components.next() else {
            return Err(io::Error::other(
                "generated-image root must be an absolute local path",
            ));
        };
        if !matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
            || components.next() != Some(Component::RootDir)
        {
            return Err(io::Error::other(
                "generated-image root must be an absolute local disk path",
            ));
        }
        let mut current = PathBuf::from(prefix.as_os_str());
        current.push(std::path::MAIN_SEPARATOR_STR);
        let mut handles = vec![lock_directory(&current)?];
        for component in components {
            let Component::Normal(name) = component else {
                return Err(io::Error::other(
                    "generated-image path contains an unsafe component",
                ));
            };
            current.push(name);
            handles.push(lock_directory(&current)?);
        }
        Ok(handles)
    }

    fn lock_directory(path: &Path) -> io::Result<File> {
        let file = OpenOptions::new()
            .read(true)
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::other(
                "generated-image parent is not a regular non-reparse directory",
            ));
        }
        Ok(file)
    }
}

#[cfg(windows)]
pub(super) use windows::ImageArtifact;

#[cfg(not(windows))]
pub(super) struct ImageArtifact;

#[cfg(not(windows))]
impl ImageArtifact {
    pub(crate) fn new() -> std::io::Result<Self> {
        Err(std::io::Error::other(
            "safe generated-image cleanup requires Windows",
        ))
    }
    pub(crate) fn claim_thread(&mut self, _: &str) -> std::io::Result<()> {
        unreachable!()
    }
    pub(crate) fn capture(&mut self, _: &serde_json::Value, _: &[u8]) -> std::io::Result<()> {
        unreachable!()
    }
    pub(crate) fn cleanup(&mut self) -> std::io::Result<()> {
        unreachable!()
    }
}
