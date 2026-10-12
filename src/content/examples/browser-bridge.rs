//! Real Windows pipe/lifecycle acceptance with a simulated qcatch extension.
//! Optional arguments: an existing qcatch.exe and its pinned extension ID.
//! This proves transport and native-host relay, not a real browser media session.

#[cfg(windows)]
#[path = "browser/mod.rs"]
mod windows;

#[cfg(not(windows))]
fn main() {
    panic!("Browser bridge acceptance requires Windows");
}

#[cfg(windows)]
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    windows::run().await
}
