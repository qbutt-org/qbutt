use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::BufReader;
use tokio::process::Command;

use super::{Client, CodexError, ImageArtifact, RequestMode, Result};

pub(super) const DISABLED_FEATURES: &[&str] = &[
    "shell_tool",
    "unified_exec",
    "shell_snapshot",
    "apps",
    "hooks",
    "multi_agent",
    "plugins",
    "memories",
    "browser_use",
    "computer_use",
    "view_image",
    "image_generation",
    "skill_search",
    "skill_mcp_dependency_install",
    "workspace_dependencies",
    "code_mode",
    "code_mode_only",
];

pub(super) struct Scratch(PathBuf);

impl Scratch {
    pub(super) fn new() -> Result<Self> {
        let mut random = [0u8; 8];
        getrandom::fill(&mut random).map_err(|_| CodexError::Provider)?;
        let name = format!("qbutt-codex-{}", u64::from_le_bytes(random));
        let path = std::env::temp_dir().join(name);
        let directory = &mut std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            directory.mode(0o700);
        }
        directory.create(&path).map_err(|_| CodexError::Provider)?;
        Ok(Self(path))
    }

    pub(super) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Client {
    pub(super) async fn start(
        executable: &Path,
        cwd: &Path,
        disabled_servers: &[String],
        mode: RequestMode,
    ) -> Result<Self> {
        let artifact = if mode == RequestMode::ImageGeneration {
            Some(ImageArtifact::new().map_err(|error| CodexError::artifact_cleanup(error, None))?)
        } else {
            None
        };
        let mut command = Command::new(executable);
        command.current_dir(cwd).env_clear();
        for name in [
            "APPDATA",
            "LOCALAPPDATA",
            "CODEX_HOME",
            "HOME",
            "USERPROFILE",
            "PATH",
            "PATHEXT",
            "SYSTEMROOT",
            "WINDIR",
            "TEMP",
            "TMP",
            "SYSTEMDRIVE",
            "HTTPS_PROXY",
            "HTTP_PROXY",
            "ALL_PROXY",
            "NO_PROXY",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "XDG_RUNTIME_DIR",
            "DBUS_SESSION_BUS_ADDRESS",
            "LANG",
            "LC_ALL",
            "SSL_CERT_FILE",
            "SSL_CERT_DIR",
        ] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        for feature in DISABLED_FEATURES {
            let enabled = *feature == "image_generation" && mode == RequestMode::ImageGeneration;
            command
                .arg("-c")
                .arg(format!("features.{feature}={enabled}"));
        }
        for server in disabled_servers {
            if !server
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
            {
                return Err(CodexError::Isolation);
            }
            command
                .arg("-c")
                .arg(format!("mcp_servers.{server}.enabled=false"));
        }
        command
            .arg("-c")
            .arg("web_search=\"disabled\"")
            .arg("-c")
            .arg("notify=[]")
            .arg("-c")
            .arg("history.persistence=\"none\"")
            .arg("-c")
            .arg("project_doc_max_bytes=0")
            .arg("app-server")
            .arg("--stdio")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        #[cfg(unix)]
        command.process_group(0);

        let mut child = command.spawn().map_err(|_| CodexError::Unavailable)?;
        let stdin = child.stdin.take().ok_or(CodexError::Protocol)?;
        let stdout = BufReader::new(child.stdout.take().ok_or(CodexError::Protocol)?);
        Ok(Self {
            mode,
            artifact,
            child,
            stdin,
            stdout,
            next_id: 1,
            queued: VecDeque::new(),
            usage: None,
            answer: None,
        })
    }

    pub(super) async fn stop(&mut self) -> std::io::Result<()> {
        #[cfg(unix)]
        if let Some(pid) = self.child.id() {
            // The child leads a private group; helpers cannot outlive a request.
            unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) };
        }
        let _ = self.child.start_kill();
        let reaped = tokio::time::timeout(Duration::from_secs(3), self.child.wait()).await;
        if let Some(artifact) = &mut self.artifact {
            match reaped {
                Ok(Ok(_)) => artifact.cleanup()?,
                Ok(Err(error)) => return Err(error),
                Err(_) => {
                    return Err(std::io::Error::other(
                        "app-server was not reaped; PNG cleanup skipped",
                    ));
                }
            }
        }
        Ok(())
    }
}

pub(super) fn installed_codex() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let program_files = std::env::var_os("ProgramFiles").map(PathBuf::from);
        if let Some(directory) = program_files {
            let native = directory
                .join("nodejs/node_modules/@openai/codex/node_modules")
                .join("@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/bin/codex.exe");
            if native.is_file() {
                return Some(native);
            }
        }
        let local = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)?
            .join("OpenAI/Codex/bin");
        let mut candidates: Vec<_> = std::fs::read_dir(local)
            .ok()?
            .flatten()
            .map(|entry| entry.path().join("codex.exe"))
            .filter(|path| path.is_file())
            .collect();
        candidates.sort_by_key(|path| path.metadata().and_then(|info| info.modified()).ok());
        candidates.pop()
    }
    #[cfg(unix)]
    {
        std::env::split_paths(&std::env::var_os("PATH")?)
            .map(|path| path.join("codex"))
            .find_map(|path| native_codex(&path))
    }
}

#[cfg(unix)]
fn native_codex(path: &Path) -> Option<PathBuf> {
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;

    let path = path.canonicalize().ok()?;
    let native = |path: &Path| {
        let metadata = path.metadata().ok()?;
        if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
            return None;
        }
        let mut signature = [0u8; 4];
        std::fs::File::open(path)
            .ok()?
            .read_exact(&mut signature)
            .ok()?;
        (signature == *b"\x7fELF").then(|| path.to_path_buf())
    };
    if let Some(path) = native(&path) {
        return Some(path);
    }
    // npm's public executable is a JavaScript shim. Resolve its installed native
    // optional package directly, retaining the application's no-runtime boundary.
    let root = path.parent()?.parent()?;
    let (package, triple) = match std::env::consts::ARCH {
        "x86_64" => ("codex-linux-x64", "x86_64-unknown-linux-musl"),
        "aarch64" => ("codex-linux-arm64", "aarch64-unknown-linux-musl"),
        _ => return None,
    };
    for vendor in [
        root.join("vendor"),
        root.join("node_modules/@openai")
            .join(package)
            .join("vendor"),
        root.parent()?.join(package).join("vendor"),
    ] {
        for executable in ["codex/codex", "bin/codex"] {
            if let Some(path) = native(&vendor.join(triple).join(executable)) {
                return Some(path);
            }
        }
    }
    None
}

impl Drop for Client {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.child.id() {
            // Covers cancellation by dropping the owning future before stop().
            unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) };
        }
    }
}
