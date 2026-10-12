use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::BufReader;
use tokio::process::Command;

use super::{Client, CodexError, Result};

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
];

pub(super) struct Scratch(PathBuf);

impl Scratch {
    pub(super) fn new() -> Result<Self> {
        let mut random = [0u8; 8];
        getrandom::fill(&mut random).map_err(|_| CodexError::Provider)?;
        let name = format!("qbutt-translation-{}", u64::from_le_bytes(random));
        let path = std::env::temp_dir().join(name);
        std::fs::create_dir(&path).map_err(|_| CodexError::Provider)?;
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
    ) -> Result<Self> {
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
        ] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        for feature in DISABLED_FEATURES {
            command.arg("-c").arg(format!("features.{feature}=false"));
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

        let mut child = command.spawn().map_err(|_| CodexError::Unavailable)?;
        let stdin = child.stdin.take().ok_or(CodexError::Protocol)?;
        let stdout = BufReader::new(child.stdout.take().ok_or(CodexError::Protocol)?);
        Ok(Self {
            child,
            stdin,
            stdout,
            next_id: 1,
            queued: VecDeque::new(),
            usage: None,
            answer: None,
        })
    }

    pub(super) async fn stop(&mut self) {
        let _ = self.child.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(3), self.child.wait()).await;
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
    #[cfg(not(windows))]
    {
        std::env::split_paths(&std::env::var_os("PATH")?)
            .map(|path| path.join("codex"))
            .find(|path| path.is_file())
    }
}
