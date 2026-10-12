//! Local generated MP4 integration. The fixture maps a public-shaped URL to
//! loopback; the product queue resolves public addresses through its guard.

use std::fs;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use qbutt_content::{
    DirectDownload, DownloadJob, DownloadQueue, DownloadStatus, DownloadStorage, Library,
    LibraryResult, PublicationRef, Source, VariantRef, VariantSelector,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::Url;

#[derive(Clone)]
struct Store(Arc<Mutex<Library>>);

impl DownloadStorage for Store {
    async fn load(&self) -> LibraryResult<Vec<DownloadJob>> {
        let store = Arc::clone(&self.0);
        tokio::task::spawn_blocking(move || store.lock().unwrap().download_jobs())
            .await
            .expect("fixture storage thread")
    }

    async fn save(&self, job: DownloadJob) -> LibraryResult<()> {
        let store = Arc::clone(&self.0);
        tokio::task::spawn_blocking(move || store.lock().unwrap().save_download_job(&job))
            .await
            .expect("fixture storage thread")
    }
}

fn request(port: u16, path: &str, title: &str) -> DirectDownload {
    DirectDownload::new(
        VariantRef {
            publication: PublicationRef {
                source: Source::Rutube,
                id: "0123456789abcdef0123456789abcdef".into(),
            },
            selector: VariantSelector::RutubeVideo,
        },
        title.into(),
        Url::parse(&format!("http://fixture.example:{port}/{path}")).unwrap(),
    )
    .unwrap()
}

async fn wait_for(
    updates: &mut tokio::sync::watch::Receiver<Vec<DownloadJob>>,
    id: &str,
    condition: impl Fn(&DownloadJob) -> bool,
) -> DownloadJob {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Some(job) = updates.borrow().iter().find(|job| job.id == id)
                && condition(job)
            {
                return job.clone();
            }
            updates.changed().await.expect("queue updates remain live");
        }
    })
    .await
    .expect("download reached expected state")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("pass an isolated output directory")?,
    );
    let source = PathBuf::from(std::env::args().nth(2).ok_or("pass a generated MP4")?);
    let payload = Arc::new(fs::read(&source)?);
    if root.exists() {
        return Err("pass a fresh isolated output directory".into());
    }
    let profile = root.join("profile");
    let destination = root.join("downloads");
    fs::create_dir_all(&destination)?;
    let marker = destination.join("user-file.mp4");
    fs::write(&marker, b"unrelated user bytes")?;

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let address: SocketAddr = listener.local_addr()?;
    let bytes = Arc::clone(&payload);
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let bytes = Arc::clone(&bytes);
            tokio::spawn(async move {
                let mut head = [0u8; 4096];
                let count = socket.read(&mut head).await.unwrap();
                let request = String::from_utf8_lossy(&head[..count]);
                let slow = request.starts_with("GET /slow.mp4 ");
                let incomplete = request.starts_with("GET /incomplete.mp4 ");
                let advertised = if incomplete {
                    bytes.len() + 100
                } else {
                    bytes.len()
                };
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: {advertised}\r\nConnection: close\r\n\r\n"
                );
                socket.write_all(header.as_bytes()).await.unwrap();
                for chunk in bytes.chunks(if slow { 1024 } else { 64 * 1024 }) {
                    if socket.write_all(chunk).await.is_err() {
                        break;
                    }
                    if slow {
                        tokio::time::sleep(Duration::from_millis(30)).await;
                    }
                }
            });
        }
    });
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .resolve("fixture.example", address)
        .build()?;
    let store = Store(Arc::new(Mutex::new(Library::open(&profile)?)));
    let queue =
        DownloadQueue::open_fixture(store.clone(), destination.clone(), client.clone()).await?;
    let mut updates = queue.subscribe();

    let collision = destination.join("Generated.mp4");
    fs::write(&collision, b"existing user payload")?;
    let first = queue
        .enqueue(request(address.port(), "media.mp4", "Generated"))
        .await?;
    let completed = wait_for(&mut updates, &first.id, |job| {
        job.status == DownloadStatus::Complete
    })
    .await;
    let published = completed.path.ok_or("completed job has no path")?;
    assert_eq!(published, destination.join("Generated (2).mp4"));
    assert_eq!(fs::read(&published)?, *payload);
    assert_eq!(fs::read(&collision)?, b"existing user payload");

    let stopped = queue
        .enqueue(request(address.port(), "slow.mp4", "Stopped"))
        .await?;
    wait_for(&mut updates, &stopped.id, |job| job.bytes > 0).await;
    queue.cancel(&stopped.id).await?;
    wait_for(&mut updates, &stopped.id, |job| {
        job.status == DownloadStatus::Cancelled
    })
    .await;
    assert!(!destination.join("Stopped.mp4").exists());
    assert!(
        !destination
            .join(format!(".qbutt-staging/{}.part", stopped.id))
            .exists()
    );

    queue
        .retry(&stopped.id, request(address.port(), "media.mp4", "Stopped"))
        .await?;
    let retried = wait_for(&mut updates, &stopped.id, |job| {
        job.status == DownloadStatus::Complete
    })
    .await;
    assert_eq!(fs::read(retried.path.unwrap())?, *payload);

    let broken = queue
        .enqueue(request(address.port(), "incomplete.mp4", "Incomplete"))
        .await?;
    wait_for(&mut updates, &broken.id, |job| {
        job.status == DownloadStatus::Error
    })
    .await;
    assert!(!destination.join("Incomplete.mp4").exists());
    assert!(
        !destination
            .join(format!(".qbutt-staging/{}.part", broken.id))
            .exists()
    );

    let interrupted = queue
        .enqueue(request(address.port(), "slow.mp4", "Interrupted"))
        .await?;
    wait_for(&mut updates, &interrupted.id, |job| job.bytes > 0).await;
    queue.close().await;
    assert!(!destination.join("Interrupted.mp4").exists());

    let restored = DownloadQueue::open_fixture(store, destination.clone(), client).await?;
    let mut restored_updates = restored.subscribe();
    let interrupted_job = restored_updates
        .borrow()
        .iter()
        .find(|job| job.id == interrupted.id)
        .cloned()
        .ok_or("interrupted job was lost")?;
    assert_eq!(interrupted_job.status, DownloadStatus::Error);
    assert!(
        !destination
            .join(format!(".qbutt-staging/{}.part", interrupted.id))
            .exists()
    );
    restored
        .retry(
            &interrupted.id,
            request(address.port(), "media.mp4", "Interrupted"),
        )
        .await?;
    let completed_after_restart = wait_for(&mut restored_updates, &interrupted.id, |job| {
        job.status == DownloadStatus::Complete
    })
    .await;
    assert_eq!(fs::read(completed_after_restart.path.unwrap())?, *payload);
    assert_eq!(restored_updates.borrow().len(), 4);
    assert_eq!(fs::read(marker)?, b"unrelated user bytes");
    restored.close().await;
    server.abort();
    assert_eq!(fs::read(&published)?, *payload);
    let database = fs::read(profile.join("content-library.sqlite"))?;
    assert!(
        !database
            .windows(b"fixture.example".len())
            .any(|bytes| bytes == b"fixture.example")
    );
    println!(
        "PASS native_downloads bytes={} complete=3 cancel=1 retry=2 incomplete=1 interrupted_restart=1 collision=1",
        payload.len()
    );
    Ok(())
}
