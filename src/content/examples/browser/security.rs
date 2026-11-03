#[cfg(windows)]
mod windows {
    use std::ptr;

    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1,
        SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION};

    pub(crate) fn private_acl(handle: std::os::windows::io::RawHandle) {
        let mut descriptor = ptr::null_mut();
        // SAFETY: handle stays owned by the caller; descriptor receives local memory.
        assert_eq!(
            unsafe {
                GetSecurityInfo(
                    handle,
                    SE_FILE_OBJECT,
                    OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    &mut descriptor,
                )
            },
            0
        );
        let mut encoded = ptr::null_mut();
        // SAFETY: GetSecurityInfo returned a valid descriptor and encoded is writable.
        assert_ne!(
            unsafe {
                ConvertSecurityDescriptorToStringSecurityDescriptorW(
                    descriptor,
                    SDDL_REVISION_1,
                    OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                    &mut encoded,
                    ptr::null_mut(),
                )
            },
            0
        );
        let mut length = 0;
        // SAFETY: encoded is a Windows-owned null-terminated UTF-16 string.
        while unsafe { *encoded.add(length) } != 0 {
            length += 1;
        }
        let sddl =
            String::from_utf16(unsafe { std::slice::from_raw_parts(encoded, length) }).unwrap();
        // SAFETY: both allocations came from Windows LocalAlloc APIs.
        unsafe {
            LocalFree(encoded.cast());
            LocalFree(descriptor);
        }
        let (owner, dacl) = sddl.split_once("D:P").expect("Protected DACL");
        let owner = owner.strip_prefix("O:").expect("Owner SID");
        assert_eq!(dacl.matches('(').count(), 1);
        assert!(dacl.starts_with("(A;;"));
        assert!(dacl.ends_with(&format!(";;;{owner})")));
    }
}

use super::fixture::{BrowserClient, FixtureResult};
use std::path::Path;

pub(super) fn private_discovery(path: &Path) -> FixtureResult {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        windows::private_acl(std::fs::File::open(path)?.as_raw_handle());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::symlink_metadata(path)?;
        assert!(metadata.is_file());
        assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
        assert_eq!(metadata.mode() & 0o777, 0o600);
        assert_eq!(metadata.nlink(), 1);
        let parent = std::fs::symlink_metadata(path.parent().ok_or("Missing discovery parent")?)?;
        assert_eq!(parent.uid(), unsafe { libc::geteuid() });
        assert_eq!(parent.mode() & 0o777, 0o700);
        let entry: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
        let socket = Path::new(entry["pipe"].as_str().ok_or("Missing socket")?);
        let metadata = std::fs::symlink_metadata(socket)?;
        assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
        assert_eq!(metadata.mode() & 0o777, 0o600);
        let parent = std::fs::symlink_metadata(socket.parent().ok_or("Missing socket parent")?)?;
        assert_eq!(parent.uid(), unsafe { libc::geteuid() });
        assert_eq!(parent.mode() & 0o777, 0o700);
    }
    Ok(())
}

pub(super) fn private_connection(client: &BrowserClient) -> FixtureResult {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        windows::private_acl(client.as_raw_handle());
    }
    #[cfg(unix)]
    assert_eq!(client.peer_cred()?.uid(), unsafe { libc::geteuid() });
    Ok(())
}

#[cfg(unix)]
pub(super) async fn filesystem_faults(directory: &Path) -> FixtureResult {
    use qbutt_content::browser::{BrowserBroker, BrowserError};
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    let discovery = directory.join("unsafe-discovery.json");
    let preserved = directory.join("preserved.txt");
    std::fs::write(&preserved, b"generated fixture must survive")?;
    std::fs::set_permissions(&preserved, std::fs::Permissions::from_mode(0o644))?;
    symlink(&preserved, &discovery)?;
    assert!(matches!(
        BrowserBroker::start(discovery.clone()).await,
        Err(BrowserError::StartFailed)
    ));
    assert_eq!(
        std::fs::read(&preserved)?,
        b"generated fixture must survive"
    );
    assert_eq!(std::fs::metadata(&preserved)?.mode() & 0o777, 0o644);
    std::fs::remove_file(&discovery)?;
    std::fs::hard_link(&preserved, &discovery)?;
    assert!(matches!(
        BrowserBroker::start(discovery.clone()).await,
        Err(BrowserError::StartFailed)
    ));
    assert_eq!(
        std::fs::read(&preserved)?,
        b"generated fixture must survive"
    );
    assert_eq!(std::fs::metadata(&preserved)?.mode() & 0o777, 0o644);
    std::fs::remove_file(&discovery)?;

    let linked_parent = directory.join("linked-parent");
    symlink(directory, &linked_parent)?;
    assert!(matches!(
        BrowserBroker::start(linked_parent.join("broker.json")).await,
        Err(BrowserError::StartFailed)
    ));
    std::fs::remove_file(linked_parent)?;

    let broker = BrowserBroker::start(discovery.clone()).await?;
    let entry: serde_json::Value = serde_json::from_slice(&std::fs::read(&discovery)?)?;
    let socket = Path::new(entry["pipe"].as_str().ok_or("Missing socket")?);
    let socket_parent = socket.parent().ok_or("Missing socket parent")?.to_owned();
    std::fs::remove_file(&discovery)?;
    std::fs::write(&discovery, b"replacement must survive shutdown")?;
    broker.shutdown().await?;
    assert_eq!(
        std::fs::read(&discovery)?,
        b"replacement must survive shutdown"
    );
    assert!(!socket.exists());
    assert!(!socket_parent.exists());
    std::fs::remove_file(discovery)?;
    let crash_discovery = directory.join("crashed.json");
    let mut child = tokio::process::Command::new(std::env::current_exe()?)
        .arg("--abandon-broker")
        .arg(&crash_discovery)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while !crash_discovery.exists() || std::fs::read(&crash_discovery)?.is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        Ok::<_, std::io::Error>(())
    })
    .await??;
    let entry: serde_json::Value = serde_json::from_slice(&std::fs::read(&crash_discovery)?)?;
    let abandoned_socket =
        Path::new(entry["pipe"].as_str().ok_or("Missing abandoned socket")?).to_owned();
    let abandoned_parent = abandoned_socket
        .parent()
        .ok_or("Missing abandoned parent")?
        .to_owned();
    child.start_kill()?;
    child.wait().await?;
    assert!(abandoned_socket.exists());
    let recovered = BrowserBroker::start(crash_discovery.clone()).await?;
    assert!(!abandoned_socket.exists());
    assert!(!abandoned_parent.exists());
    recovered.shutdown().await?;
    assert!(!crash_discovery.exists());
    std::fs::remove_file(preserved)?;
    println!(
        "PASS browser_unix_security symlink hardlink parent_alias replacement_preserved socket_cleanup crash_recovery"
    );
    Ok(())
}
