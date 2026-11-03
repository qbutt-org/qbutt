// SPDX-License-Identifier: GPL-2.0-or-later

#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;

use reqwest::redirect::Policy;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::fs::{self, File};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::time::timeout;
use url::Url;

use super::ffi;

const EXTENSION_ID: &str = "jnmnnglmejlbjnjmmjampnhfncpcbffh";
const NATIVE_HOST: &str = "org.qbutt.qcatch";
const MAX_OUTPUT: u64 = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SetupError {
    Cancelled,
    Unavailable,
    Integrity,
    Download,
    Install,
    Connect,
}

#[derive(Default)]
pub(super) struct SetupControl {
    cancelled: bool,
    side_effect_started: bool,
}

impl SetupControl {
    pub(super) fn cancel(&mut self) -> bool {
        self.cancelled = true;
        !self.side_effect_started
    }

    fn ensure_active(&self) -> Result<(), SetupError> {
        if self.cancelled {
            Err(SetupError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn spawn_action(&mut self, command: &mut Command) -> Result<tokio::process::Child, SetupError> {
        self.ensure_active()?;
        let child = command.spawn().map_err(|_| SetupError::Unavailable)?;
        self.side_effect_started = true;
        Ok(child)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArtifactPin {
    url: String,
    size: u64,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct QcatchPin {
    version: String,
    protocol_version: u32,
    artifacts: QcatchArtifacts,
}

#[derive(Deserialize)]
struct QcatchArtifacts {
    #[cfg_attr(target_os = "linux", serde(rename = "linuxPortable"))]
    #[cfg_attr(not(target_os = "linux"), serde(rename = "setup"))]
    installation: ArtifactPin,
}

#[derive(Deserialize)]
struct ProductLock {
    products: ProductPins,
}

#[derive(Deserialize)]
struct ProductPins {
    qcatch: QcatchPin,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VersionInfo {
    product: String,
    version: String,
    protocol_version: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectInfo {
    browser: String,
    extension_directory: PathBuf,
    extension_id: String,
    native_host: String,
}

fn pin() -> Result<QcatchPin, SetupError> {
    let lock: ProductLock = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../../upstream-lock.json"
    )))
    .map_err(|_| SetupError::Integrity)?;
    let pin = lock.products.qcatch;
    let url = Url::parse(&pin.artifacts.installation.url).map_err(|_| SetupError::Integrity)?;
    #[cfg(target_os = "linux")]
    let filename = "qcatch-linux-x64.zip";
    #[cfg(not(target_os = "linux"))]
    let filename = "qcatch-setup.exe";
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || !url
            .path()
            .starts_with("/qbutt-org/qcatch/releases/download/")
        || url.path_segments().and_then(Iterator::last) != Some(filename)
        || pin.artifacts.installation.size == 0
        || pin.artifacts.installation.size > 2 * 1024 * 1024 * 1024
        || pin.artifacts.installation.sha256.len() != 64
        || !pin
            .artifacts
            .installation
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(SetupError::Integrity);
    }
    Ok(pin)
}

fn local_executable() -> Option<PathBuf> {
    let path = PathBuf::from(ffi::qcatch_executable().to_string());
    (!path.as_os_str().is_empty()).then_some(path)
}

fn command(executable: &Path) -> Command {
    let mut command = Command::new(executable);
    command.stdin(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    command.as_std_mut().creation_flags(0x0800_0000);
    command
}

async fn output(mut child: tokio::process::Child) -> Result<Vec<u8>, SetupError> {
    let stdout = child.stdout.take().ok_or(SetupError::Unavailable)?;
    let mut data = Vec::new();
    let read = timeout(Duration::from_secs(30), async {
        stdout
            .take(MAX_OUTPUT + 1)
            .read_to_end(&mut data)
            .await
            .map_err(|_| SetupError::Unavailable)?;
        if data.len() as u64 > MAX_OUTPUT {
            return Err(SetupError::Unavailable);
        }
        child.wait().await.map_err(|_| SetupError::Unavailable)
    })
    .await;
    match read {
        Ok(Ok(status)) if status.success() => Ok(data),
        _ => {
            let _ = child.kill().await;
            Err(SetupError::Unavailable)
        }
    }
}

async fn compatible(executable: &Path, pin: &QcatchPin) -> bool {
    let mut command = command(executable);
    command
        .arg("--version-json")
        .stdout(Stdio::piped())
        .kill_on_drop(true);
    let Ok(child) = command.spawn() else {
        return false;
    };
    let Ok(data) = output(child).await else {
        return false;
    };
    let Ok(info) = serde_json::from_slice::<VersionInfo>(&data) else {
        return false;
    };
    let minimum = pin
        .version
        .split('.')
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>();
    let current = info
        .version
        .split('.')
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>();
    let (Ok(minimum), Ok(current)) = (minimum, current) else {
        return false;
    };
    info.product == "qcatch"
        && info.protocol_version == pin.protocol_version
        && minimum.len() == 3
        && current.len() == 3
        && current >= minimum
}

async fn valid_cached(path: &Path, pin: &ArtifactPin) -> Result<bool, SetupError> {
    let metadata = match fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err(SetupError::Integrity),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(SetupError::Integrity);
    }
    if metadata.len() != pin.size {
        return Ok(false);
    }
    let mut file = File::open(path).await.map_err(|_| SetupError::Integrity)?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .await
            .map_err(|_| SetupError::Integrity)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()) == pin.sha256)
}

fn allowed_redirect(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(
            url.host_str(),
            Some(
                "github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
            )
        )
}

async fn verified_setup(
    pin: &QcatchPin,
    control: &Mutex<SetupControl>,
) -> Result<PathBuf, SetupError> {
    let directory = PathBuf::from(ffi::qcatch_setup_cache_directory().to_string());
    if !directory.is_absolute() {
        return Err(SetupError::Unavailable);
    }
    #[cfg(target_os = "linux")]
    let filename = format!("qcatch-{}-linux-x64.zip", pin.version);
    #[cfg(not(target_os = "linux"))]
    let filename = format!("qcatch-{}-setup.exe", pin.version);
    let destination = directory.join(filename);
    if valid_cached(&destination, &pin.artifacts.installation).await? {
        return Ok(destination);
    }
    control.lock().expect("setup lock").ensure_active()?;
    fs::create_dir_all(&directory)
        .await
        .map_err(|_| SetupError::Download)?;
    let client = reqwest::Client::builder()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|_| SetupError::Download)?;
    let mut url = Url::parse(&pin.artifacts.installation.url).map_err(|_| SetupError::Integrity)?;
    let mut response = None;
    for _ in 0..=5 {
        control.lock().expect("setup lock").ensure_active()?;
        let current = client
            .get(url.clone())
            .send()
            .await
            .map_err(|_| SetupError::Download)?;
        if current.status().is_redirection() {
            let location = current
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or(SetupError::Download)?;
            url = url.join(location).map_err(|_| SetupError::Integrity)?;
            if !allowed_redirect(&url) {
                return Err(SetupError::Integrity);
            }
        } else {
            response = Some(current);
            break;
        }
    }
    let mut response = response.ok_or(SetupError::Download)?;
    if !response.status().is_success() {
        return Err(SetupError::Download);
    }

    let temporary = tempfile::Builder::new()
        .prefix("qcatch-")
        .suffix(".partial")
        .tempfile_in(&directory)
        .map_err(|_| SetupError::Download)?;
    let (file, path) = temporary.into_parts();
    let mut file = File::from_std(file);
    let mut size = 0_u64;
    let mut hash = Sha256::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| SetupError::Download)? {
        control.lock().expect("setup lock").ensure_active()?;
        size += chunk.len() as u64;
        if size > pin.artifacts.installation.size {
            return Err(SetupError::Integrity);
        }
        hash.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|_| SetupError::Download)?;
    }
    file.sync_all().await.map_err(|_| SetupError::Download)?;
    drop(file);
    if size != pin.artifacts.installation.size
        || format!("{:x}", hash.finalize()) != pin.artifacts.installation.sha256
    {
        return Err(SetupError::Integrity);
    }
    control.lock().expect("setup lock").ensure_active()?;
    if fs::symlink_metadata(&destination).await.is_ok() {
        fs::remove_file(&destination)
            .await
            .map_err(|_| SetupError::Integrity)?;
    }
    path.persist(&destination)
        .map_err(|_| SetupError::Download)?;
    Ok(destination)
}

#[cfg(target_os = "linux")]
fn unpack_setup(package: &Path) -> Result<(tempfile::TempDir, PathBuf), SetupError> {
    use std::os::unix::fs::PermissionsExt;

    let file = std::fs::File::open(package).map_err(|_| SetupError::Integrity)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|_| SetupError::Integrity)?;
    if archive.len() > 4096 {
        return Err(SetupError::Integrity);
    }
    let directory = tempfile::Builder::new()
        .prefix("qcatch-install-")
        .tempdir()
        .map_err(|_| SetupError::Install)?;
    let mut total_size = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|_| SetupError::Integrity)?;
        let relative = entry.enclosed_name().ok_or(SetupError::Integrity)?;
        let destination = directory.path().join(relative);
        let mode = entry.unix_mode().unwrap_or(0o644);
        if entry.is_dir() {
            if !matches!(mode & 0o170000, 0 | 0o040000) {
                return Err(SetupError::Integrity);
            }
            std::fs::create_dir_all(destination).map_err(|_| SetupError::Install)?;
            continue;
        }
        if !matches!(mode & 0o170000, 0 | 0o100000) {
            return Err(SetupError::Integrity);
        }
        total_size = total_size
            .checked_add(entry.size())
            .filter(|size| *size <= 4 * 1024 * 1024 * 1024)
            .ok_or(SetupError::Integrity)?;
        let parent = destination.parent().ok_or(SetupError::Integrity)?;
        std::fs::create_dir_all(parent).map_err(|_| SetupError::Install)?;
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .map_err(|_| SetupError::Install)?;
        let expected_size = entry.size();
        let mut bounded = std::io::Read::take(&mut entry, expected_size + 1);
        let copied = std::io::copy(&mut bounded, &mut output).map_err(|_| SetupError::Integrity)?;
        if copied != expected_size {
            return Err(SetupError::Integrity);
        }
        output.sync_all().map_err(|_| SetupError::Install)?;
        std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(mode & 0o755))
            .map_err(|_| SetupError::Install)?;
    }
    let executable = directory.path().join("qcatch");
    let metadata = std::fs::symlink_metadata(&executable).map_err(|_| SetupError::Integrity)?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o100 == 0 {
        return Err(SetupError::Integrity);
    }
    Ok((directory, executable))
}

pub(super) async fn connect(
    browser: &str,
    control: &Mutex<SetupControl>,
) -> Result<PathBuf, SetupError> {
    let pin = pin()?;
    let mut executable = local_executable();
    let installed = if let Some(path) = &executable {
        compatible(path, &pin).await
    } else {
        false
    };
    if !installed {
        let setup = timeout(Duration::from_secs(600), verified_setup(&pin, control))
            .await
            .map_err(|_| SetupError::Download)??;
        #[cfg(target_os = "linux")]
        let (_directory, setup) = tokio::task::spawn_blocking(move || unpack_setup(&setup))
            .await
            .map_err(|_| SetupError::Install)??;
        let mut command = command(&setup);
        #[cfg(target_os = "linux")]
        command.arg("--install");
        #[cfg(not(target_os = "linux"))]
        command.args([
            "/CURRENTUSER",
            "/VERYSILENT",
            "/SUPPRESSMSGBOXES",
            "/NORESTART",
            "/SP-",
        ]);
        command.stdout(Stdio::null());
        let mut child = control
            .lock()
            .expect("setup lock")
            .spawn_action(&mut command)?;
        let status = child.wait().await.map_err(|_| SetupError::Install)?;
        if !status.success() {
            return Err(SetupError::Install);
        }
        control.lock().expect("setup lock").ensure_active()?;
        executable = local_executable();
        if !executable.as_ref().is_some_and(|path| path.is_file()) {
            return Err(SetupError::Install);
        }
    }
    control.lock().expect("setup lock").ensure_active()?;
    let executable = executable.ok_or(SetupError::Install)?;
    if !compatible(&executable, &pin).await {
        return Err(SetupError::Install);
    }
    let mut command = command(&executable);
    command
        .args(["--connect-browser", browser])
        .stdout(Stdio::piped());
    let child = control
        .lock()
        .expect("setup lock")
        .spawn_action(&mut command)?;
    let data = output(child).await.map_err(|_| SetupError::Connect)?;
    let result: ConnectInfo = serde_json::from_slice(&data).map_err(|_| SetupError::Connect)?;
    if result.browser != browser
        || result.extension_id != EXTENSION_ID
        || result.native_host != NATIVE_HOST
        || !result.extension_directory.is_absolute()
    {
        return Err(SetupError::Connect);
    }
    Ok(result.extension_directory)
}
