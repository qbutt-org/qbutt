#[cfg(windows)]
mod implementation {
    use anyhow::{Context, Result, bail};
    use std::fs::{self, File, OpenOptions};
    use std::os::windows::fs::OpenOptionsExt;
    use std::path::{Path, PathBuf};
    use std::thread;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION;

    const GPU_LOCK_TIMEOUT: Duration = Duration::ZERO;
    const INSTALLER_LOCK_TIMEOUT: Duration = Duration::from_secs(2);
    const LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(100);
    const GPU_LOCK_FILE: &str = "framegen-gpu.lock";
    const INSTALLER_LOCK_FILE: &str = "framegen-installer.lock";

    // The sentinel persists; only its live share-none handle represents ownership.
    struct ExclusiveFileLock {
        _file: File,
    }

    impl ExclusiveFileLock {
        fn acquire(
            file_name: &str,
            description: &str,
            busy_owner: &str,
            timeout: Duration,
        ) -> Result<Self> {
            let path = lock_path(file_name)?;
            Self::acquire_at(&path, description, busy_owner, timeout)
        }

        fn acquire_at(
            path: &Path,
            description: &str,
            busy_owner: &str,
            timeout: Duration,
        ) -> Result<Self> {
            let parent = path
                .parent()
                .context("frame generation lock has no parent")?;
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create frame generation lock directory: {}",
                    parent.display()
                )
            })?;

            let started = Instant::now();
            loop {
                match OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .share_mode(0)
                    .open(path)
                {
                    Ok(file) => return Ok(Self { _file: file }),
                    Err(error) if error.raw_os_error() == Some(ERROR_SHARING_VIOLATION as i32) => {
                        let elapsed = started.elapsed();
                        if elapsed >= timeout {
                            bail!(
                                "{description} is busy: {busy_owner} owns the cross-session lock {}",
                                path.display()
                            )
                        }
                        thread::sleep(std::cmp::min(LOCK_RETRY_INTERVAL, timeout - elapsed));
                    }
                    Err(error) => {
                        return Err(error).with_context(|| {
                            format!("failed to acquire {description} lock: {}", path.display())
                        });
                    }
                }
            }
        }
    }

    fn lock_path(file_name: &str) -> Result<PathBuf> {
        let root = crate::platform::runtime_root()?;
        Ok(root
            .parent()
            .context("runtime has no parent")?
            .join("locks")
            .join(file_name))
    }

    pub struct GpuLock {
        _guard: ExclusiveFileLock,
    }

    impl GpuLock {
        pub fn acquire() -> Result<Self> {
            Ok(Self {
                _guard: ExclusiveFileLock::acquire(
                    GPU_LOCK_FILE,
                    "frame generation GPU",
                    "mpv playback or another GPU job",
                    GPU_LOCK_TIMEOUT,
                )?,
            })
        }
    }

    pub struct InstallerLock {
        _guard: ExclusiveFileLock,
    }

    impl InstallerLock {
        pub fn acquire() -> Result<Self> {
            Ok(Self {
                _guard: ExclusiveFileLock::acquire(
                    INSTALLER_LOCK_FILE,
                    "frame generation installer",
                    "another installer",
                    INSTALLER_LOCK_TIMEOUT,
                )?,
            })
        }
    }
}

#[cfg(not(windows))]
mod implementation {
    use anyhow::Result;
    pub struct GpuLock;
    impl GpuLock {
        pub fn acquire() -> Result<Self> {
            Ok(Self)
        }
    }
    pub struct InstallerLock;
    impl InstallerLock {
        pub fn acquire() -> Result<Self> {
            Ok(Self)
        }
    }
}

pub use implementation::{GpuLock, InstallerLock};
