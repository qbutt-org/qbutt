//! Real Windows pipe/lifecycle acceptance with a simulated qcatch extension.
//! Optional arguments: an existing qcatch.exe and its pinned extension ID.
//! This proves transport and native-host relay, not a real browser media session.

#[cfg(not(windows))]
fn main() {
    panic!("Browser bridge acceptance requires Windows");
}

#[cfg(windows)]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    windows::run().await
}

#[cfg(windows)]
mod windows {
    use std::fs::{self, File};
    use std::io::{Read, Write};
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::ptr;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use qbutt_content::browser::{
        BrowserBroker, BrowserChoice, BrowserCommand, BrowserError, BrowserHandle, BrowserId,
        BrowserReply, BrowserStatus,
    };
    use serde_json::{Value, json};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
    use tokio::task::JoinHandle;
    use tokio::time::{sleep, timeout};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1,
        SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION};

    const MAX_FRAME: usize = 768 * 1024;
    type FixtureResult = Result<(), Box<dyn std::error::Error>>;

    struct NativeHost(Child);

    impl Drop for NativeHost {
        fn drop(&mut self) {
            if !matches!(self.0.try_wait(), Ok(Some(_))) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }

    pub async fn run() -> FixtureResult {
        let directory = std::env::temp_dir().join(format!(
            "qbutt-native-browser-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        ));
        fs::create_dir(&directory)?;
        let discovery = directory.join("browser-bridge.json");
        let broker = BrowserBroker::start(discovery.clone()).await?;
        let handle = broker.handle();
        assert!(handle.status().connections.is_empty());
        assert!(matches!(
            BrowserBroker::start(discovery.clone()).await,
            Err(BrowserError::OwnerExists)
        ));
        private_acl(File::open(&discovery)?.as_raw_handle());
        assert_error(
            handle.request(BrowserChoice::Auto, cookies()).await,
            BrowserError::BrowserRequired,
        );
        for address in [
            "http://127.0.0.1/",
            "https://10.0.0.1/",
            "https://localhost/",
            "https://host.local/",
            "https://[::1]/",
            "https://user:pass@example.com/",
            "file:///C:/Windows/",
        ] {
            assert_error(
                handle
                    .request(
                        BrowserChoice::Auto,
                        serde_json::from_value(json!({"method":"cookies","url":address}))?,
                    )
                    .await,
                BrowserError::InvalidRequest,
            );
        }

        framing_faults(&discovery).await?;
        let mut edge = connect(&discovery).await?;
        private_acl(edge.as_raw_handle());
        write_fragmented(&mut edge, &hello("edge", "fixture edge", true)).await?;
        wait_status(&handle, |status| {
            status
                .connections
                .iter()
                .any(|connection| connection.user_agent == "fixture edge")
        })
        .await?;
        command_replies(&handle, &mut edge).await?;
        let mut chrome = extension(&discovery, "chrome", "fixture chrome", false).await?;
        wait_status(&handle, |status| status.connections.len() == 2).await?;
        assert_error(
            handle.request(BrowserChoice::Auto, cookies()).await,
            BrowserError::ChoiceRequired,
        );
        assert_error(
            handle
                .request(
                    BrowserChoice::Chrome,
                    BrowserCommand::vk_search("fixture".into())?,
                )
                .await,
            BrowserError::UpdateRequired,
        );
        connection_identity(&handle, &mut edge, &mut chrome).await?;
        cancellation(&handle, &mut edge).await?;
        capacity(&handle, &mut edge).await?;

        let pending = request(&handle, BrowserChoice::Edge, cookies());
        let _wire = read(&mut edge).await?;
        let mut replacement = extension(&discovery, "edge", "replacement edge", true).await?;
        wait_status(&handle, |status| {
            status
                .connections
                .iter()
                .any(|connection| connection.user_agent == "replacement edge")
        })
        .await?;
        assert_error(pending.await?, BrowserError::Disconnected);
        closed(&mut edge).await?;
        exchange(
            &handle,
            &mut replacement,
            json!({"method":"cookies","url":"https://fixture.example/"}),
            json!([]),
            None,
        )
        .await?;
        let pending = request(&handle, BrowserChoice::Edge, cookies());
        let _wire = read(&mut replacement).await?;
        drop(replacement);
        assert_error(pending.await?, BrowserError::Disconnected);
        wait_status(&handle, |status| {
            status.connections.len() == 1 && status.connections[0].browser == BrowserId::Chrome
        })
        .await?;
        let pending = request(&handle, BrowserChoice::Chrome, cookies());
        let _wire = read(&mut chrome).await?;
        timeout(Duration::from_secs(3), broker.shutdown()).await??;
        assert_error(pending.await?, BrowserError::Stopped);
        assert!(handle.status().stopped);
        assert!(!discovery.exists());
        closed(&mut chrome).await?;
        assert_error(
            handle.request(BrowserChoice::Chrome, cookies()).await,
            BrowserError::Stopped,
        );

        active_faults(&discovery).await?;
        stale_discovery(&discovery).await?;
        let broker = BrowserBroker::start(discovery.clone()).await?;
        let dropped = broker.handle();
        drop(broker);
        wait_status(&dropped, |status| status.stopped).await?;
        assert!(!discovery.exists());
        let arguments: Vec<_> = std::env::args_os().skip(1).collect();
        let native_host = match arguments.as_slice() {
            [executable, extension_id] => {
                native_host_relay(
                    &discovery,
                    PathBuf::from(executable),
                    extension_id.to_string_lossy().into_owned(),
                )
                .await?;
                "ok"
            }
            [] => "not_run",
            _ => return Err("Expected qcatch.exe and extension ID arguments".into()),
        };
        assert!(!discovery.exists());
        fs::remove_dir(&directory)?;
        println!(
            "PASS browser_bridge=ok real_windows_pipe=ok private_acl=ok framing=ok exact_selection=ok connection_identity=ok cancellation=ok bounded_pending=ok shutdown=ok native_host={native_host}"
        );
        Ok(())
    }

    async fn framing_faults(discovery: &Path) -> FixtureResult {
        for value in [
            json!({"type":"hello","version":2,"browser":"edge","userAgent":"fixture"}),
            json!({"type":"hello","version":1,"browser":"firefox","userAgent":"fixture"}),
            json!({"type":"hello","version":1,"browser":"edge","userAgent":"fixture\r\nsecret"}),
            json!({"type":"hello","version":1,"browser":"edge","userAgent":"fixture","capabilities":["bad_capability"]}),
        ] {
            let mut client = connect(discovery).await?;
            write(&mut client, &value).await?;
            closed(&mut client).await?;
        }
        for bytes in [
            0u32.to_le_bytes().to_vec(),
            (MAX_FRAME as u32 + 1).to_le_bytes().to_vec(),
            vec![1, 0, 0, 0, 0xff],
        ] {
            let mut client = connect(discovery).await?;
            client.write_all(&bytes).await?;
            closed(&mut client).await?;
        }
        // These three independent slow clients exercise the same five-second
        // hello deadline without extending acceptance into a serial sleep loop.
        let mut idle = connect(discovery).await?;
        let mut header = connect(discovery).await?;
        header.write_all(&[20]).await?;
        let mut body = connect(discovery).await?;
        body.write_all(&20u32.to_le_bytes()).await?;
        body.write_all(b"{\"type\"").await?;
        let (a, b, c) = tokio::join!(closed(&mut idle), closed(&mut header), closed(&mut body));
        a?;
        b?;
        c?;
        Ok(())
    }

    async fn command_replies(handle: &BrowserHandle, edge: &mut NamedPipeClient) -> FixtureResult {
        let cookie = json!({"name":"session","value":"ephemeral-fixture-secret","domain":".fixture.example","path":"/","secure":true,"hostOnly":false,"httpOnly":true,"sameSite":"lax","expirationDate":2000000000});
        let reply = exchange(
            handle,
            edge,
            json!({"method":"cookies","url":"https://fixture.example/"}),
            json!([cookie.clone()]),
            None,
        )
        .await?;
        assert_eq!(reply[0]["value"], "ephemeral-fixture-secret");
        let mut foreign_cookie = cookie.clone();
        foreign_cookie["domain"] = json!("other.example");
        exchange(
            handle,
            edge,
            json!({"method":"cookies","url":"https://fixture.example/"}),
            json!([foreign_cookie]),
            Some(BrowserError::InvalidResponse),
        )
        .await?;
        let mut foreign_path = cookie.clone();
        foreign_path["path"] = json!("/another-publication");
        exchange(
            handle,
            edge,
            json!({"method":"cookies","url":"https://fixture.example/"}),
            json!([foreign_path]),
            Some(BrowserError::InvalidResponse),
        )
        .await?;
        exchange(
            handle,
            edge,
            json!({"method":"cookies","url":"https://fixture.example/"}),
            Value::Array(vec![cookie; 513]),
            Some(BrowserError::InvalidResponse),
        )
        .await?;
        let fetch = json!({"method":"fetch","url":"https://fixture.example/api","init":{"method":"POST","headers":{"Content-Type":"application/json"},"body":"{}"}});
        let response = json!({"status":200,"url":"https://fixture.example/api","headers":{"content-type":"application/json"},"body":"{\"ok\":true}"});
        exchange(handle, edge, fetch.clone(), response.clone(), None).await?;
        for (field, value) in [
            ("status", json!(999)),
            ("url", json!("http://192.168.1.1/")),
            ("body", json!("x".repeat(512 * 1024 + 1))),
            ("headers", json!({"x-test":"value\r\nInjected: true"})),
        ] {
            let mut invalid = response.clone();
            invalid[field] = value;
            exchange(
                handle,
                edge,
                fetch.clone(),
                invalid,
                Some(BrowserError::InvalidResponse),
            )
            .await?;
        }
        let resolve = json!({"method":"resolve","url":"https://fixture.example/player","kind":"video","referrer":"https://fixture.example/title","selection":{"episode":"7","season":"2","translationId":"13","player":"VK"}});
        let media = json!({"kind":"video","resolver":"direct","url":"https://media.example/video.m3u8?key=ephemeral-fixture-secret","selection":resolve["selection"],"headers":{"Cookie":"session=ephemeral-fixture-secret"},"audioUrl":"https://media.example/audio.m3u8","audioTrack":{"title":"Original","language":"ja","channels":2},"subtitles":[{"title":"Russian","url":"https://media.example/ru.vtt","language":"ru"}]});
        let reply = exchange(handle, edge, resolve.clone(), media.clone(), None).await?;
        assert_eq!(reply["audioTrack"]["channels"], 2);
        assert_eq!(reply["subtitles"][0]["language"], "ru");
        let mut wrong_selection = media.clone();
        wrong_selection["selection"]["episode"] = json!("8");
        exchange(
            handle,
            edge,
            resolve.clone(),
            wrong_selection,
            Some(BrowserError::SelectionChanged),
        )
        .await?;
        for (field, value) in [
            ("kind", json!("file")),
            ("resolver", json!("yt-dlp")),
            ("audioUrl", json!("http://10.0.0.1/audio")),
            (
                "subtitles",
                json!([{"title":"local","url":"file:///C:/Windows/file.vtt"}]),
            ),
        ] {
            let mut invalid = media.clone();
            invalid[field] = value;
            exchange(
                handle,
                edge,
                resolve.clone(),
                invalid,
                Some(BrowserError::InvalidResponse),
            )
            .await?;
        }
        let capture = json!({"captureId":"00000000-0000-4000-8000-000000000001"});
        exchange(handle, edge, json!({"method":"resolve","url":"https://fixture.example/player","selection":capture}), json!({"kind":"video","resolver":"yt-dlp","url":"https://fixture.example/player","selection":capture}), None).await?;
        exchange(handle, edge, json!({"method":"resolve","url":"https://fixture.example/download","kind":"file"}), json!({"kind":"torrent","resolver":"direct","url":"https://fixture.example/file.torrent"}), None).await?;
        let search = serde_json::to_value(BrowserCommand::vk_search("fixture anime".into())?)?;
        let videos = json!({"query":"fixture anime","videos":[{"id":"-123_456","title":"Episode 7","url":"https://vkvideo.ru/video-123_456","author":"Fixture","duration":1440,"cover":"https://cover.example/image.jpg"}]});
        exchange(handle, edge, search.clone(), videos.clone(), None).await?;
        let mut wrong_query = videos.clone();
        wrong_query["query"] = json!("other");
        exchange(
            handle,
            edge,
            search.clone(),
            wrong_query,
            Some(BrowserError::InvalidResponse),
        )
        .await?;
        let mut wrong_video = videos;
        wrong_video["videos"][0]["url"] = json!("https://vkvideo.ru/video-123_999");
        exchange(
            handle,
            edge,
            search,
            wrong_video,
            Some(BrowserError::InvalidResponse),
        )
        .await?;
        let pending = request(handle, BrowserChoice::Edge, cookies());
        let wire = read(edge).await?;
        write(edge, &json!({"id":wire["id"],"error":{"code":"verification_required","message":"ephemeral-fixture-secret https://fixture.example/?token=secret","url":"https://fixture.example/?token=secret"}})).await?;
        let error = match pending.await? {
            Err(error) => error,
            Ok(_) => panic!("Expected remote failure"),
        };
        assert_eq!(error, BrowserError::VerificationRequired);
        assert!(!format!("{error:?} {error}").contains("secret"));
        Ok(())
    }

    async fn connection_identity(
        handle: &BrowserHandle,
        edge: &mut NamedPipeClient,
        chrome: &mut NamedPipeClient,
    ) -> FixtureResult {
        let pending = request(handle, BrowserChoice::Edge, cookies());
        let wire = read(edge).await?;
        write(chrome, &json!({"id":wire["id"],"result":[]})).await?;
        sleep(Duration::from_millis(30)).await;
        assert!(!pending.is_finished());
        write(edge, &json!({"id":wire["id"],"result":[]})).await?;
        assert!(matches!(pending.await?, Ok(BrowserReply::Cookies(_))));
        Ok(())
    }

    async fn cancellation(handle: &BrowserHandle, edge: &mut NamedPipeClient) -> FixtureResult {
        let pending = request(handle, BrowserChoice::Edge, cookies());
        let wire = read(edge).await?;
        pending.abort();
        assert!(matches!(pending.await, Err(error) if error.is_cancelled()));
        let cancel = read(edge).await?;
        assert_eq!(cancel["method"], "cancel");
        assert_eq!(cancel["id"], wire["id"]);
        write(edge, &json!({"id":wire["id"],"result":[]})).await?;
        exchange(
            handle,
            edge,
            json!({"method":"cookies","url":"https://fixture.example/"}),
            json!([]),
            None,
        )
        .await?;
        Ok(())
    }

    async fn capacity(handle: &BrowserHandle, edge: &mut NamedPipeClient) -> FixtureResult {
        let mut pending = Vec::new();
        for _ in 0..16 {
            let request = request(handle, BrowserChoice::Edge, cookies());
            let wire = read(edge).await?;
            pending.push((request, wire));
        }
        assert_error(
            handle.request(BrowserChoice::Edge, cookies()).await,
            BrowserError::Busy,
        );
        for (request, wire) in pending {
            write(edge, &json!({"id":wire["id"],"result":[]})).await?;
            assert!(matches!(request.await?, Ok(BrowserReply::Cookies(_))));
        }
        Ok(())
    }

    async fn stale_discovery(path: &Path) -> FixtureResult {
        fs::write(path, json!({"pipe":r"\\.\pipe\qbutt-content-browser-00000000-0000-4000-8000-000000000000","pid":0}).to_string())?;
        let broker = BrowserBroker::start(path.to_path_buf()).await?;
        assert!(path.exists());
        broker.shutdown().await?;
        assert!(!path.exists());
        fs::write(path, json!({"pipe":r"\\.\pipe\qbutt-content-browser-00000000-0000-4000-8000-000000000000","pid":std::process::id()}).to_string())?;
        let before = fs::read(path)?;
        assert!(matches!(
            BrowserBroker::start(path.to_path_buf()).await,
            Err(BrowserError::OwnerExists)
        ));
        assert_eq!(fs::read(path)?, before);
        fs::remove_file(path)?;
        Ok(())
    }

    async fn active_faults(discovery: &Path) -> FixtureResult {
        let broker = BrowserBroker::start(discovery.to_path_buf()).await?;
        let handle = broker.handle();
        for malformed in [
            json!({"id":1,"result":[],"error":{"code":"browser_error","message":"invalid"}}),
            json!({"id":1,"result":[],"extra":"invalid"}),
            json!({"id":1.5,"result":[]}),
            json!({"id":0,"result":[]}),
        ] {
            let mut edge = extension(discovery, "edge", "malformed response", true).await?;
            wait_status(&handle, |status| status.connections.len() == 1).await?;
            let pending = request(&handle, BrowserChoice::Edge, cookies());
            let wire = read(&mut edge).await?;
            let mut malformed = malformed;
            if malformed["id"] == 1 {
                malformed["id"] = wire["id"].clone();
            }
            write(&mut edge, &malformed).await?;
            assert_error(pending.await?, BrowserError::Disconnected);
            closed(&mut edge).await?;
            wait_status(&handle, |status| status.connections.is_empty()).await?;
        }
        let mut edge = extension(discovery, "edge", "interleaved partial response", true).await?;
        wait_status(&handle, |status| status.connections.len() == 1).await?;
        let first = request(&handle, BrowserChoice::Edge, cookies());
        let first_wire = read(&mut edge).await?;
        let bytes = serde_json::to_vec(&json!({"id":first_wire["id"],"result":[]}))?;
        edge.write_all(&(bytes.len() as u32).to_le_bytes()[..2])
            .await?;
        let second = request(&handle, BrowserChoice::Edge, cookies());
        let second_wire = read(&mut edge).await?;
        edge.write_all(&(bytes.len() as u32).to_le_bytes()[2..])
            .await?;
        edge.write_all(&bytes).await?;
        write(&mut edge, &json!({"id":second_wire["id"],"result":[]})).await?;
        assert!(matches!(first.await?, Ok(BrowserReply::Cookies(_))));
        assert!(matches!(second.await?, Ok(BrowserReply::Cookies(_))));

        // A complete idle connection may stay open. An active request expires
        // after its fixed deadline and sends the same ID's cancellation frame.
        let began = std::time::Instant::now();
        let pending = request(&handle, BrowserChoice::Edge, cookies());
        let wire = read(&mut edge).await?;
        let cancel = timeout(Duration::from_secs(23), async {
            let length = edge.read_u32_le().await? as usize;
            let mut body = vec![0u8; length];
            edge.read_exact(&mut body).await?;
            serde_json::from_slice::<Value>(&body).map_err(std::io::Error::other)
        })
        .await??;
        assert_eq!(cancel, json!({"id":wire["id"],"method":"cancel"}));
        assert_error(pending.await?, BrowserError::Timeout);
        assert!(began.elapsed() >= Duration::from_secs(20));
        drop(edge);
        wait_status(&handle, |status| status.connections.is_empty()).await?;

        let mut blocked = extension(discovery, "edge", "blocked writer", true).await?;
        wait_status(&handle, |status| status.connections.len() == 1).await?;
        let mut pending = Vec::new();
        for _ in 0..16 {
            pending.push(request(&handle, BrowserChoice::Edge, serde_json::from_value(json!({
                "method":"fetch","url":"https://fixture.example/","init":{"method":"POST","body":"x".repeat(512 * 1024)}
            }))?));
        }
        for request in pending {
            assert_error(
                timeout(Duration::from_secs(7), request).await??,
                BrowserError::Disconnected,
            );
        }
        // Its unread command bytes may remain buffered after disconnect. We
        // verify owner state and task drain rather than interpreting them as EOF.
        wait_status(&handle, |status| status.connections.is_empty()).await?;
        blocked.shutdown().await?;
        drop(blocked);
        let mut partial = connect(discovery).await?;
        partial.write_all(&[20]).await?;
        timeout(Duration::from_secs(3), broker.shutdown()).await??;
        closed(&mut partial).await?;
        assert!(!discovery.exists());
        Ok(())
    }

    async fn native_host_relay(
        discovery: &Path,
        executable: PathBuf,
        extension_id: String,
    ) -> FixtureResult {
        if extension_id.len() != 32
            || !extension_id
                .bytes()
                .all(|byte| (b'a'..=b'p').contains(&byte))
        {
            return Err("Invalid fixture extension ID".into());
        }
        let broker = BrowserBroker::start(discovery.to_path_buf()).await?;
        let handle = broker.handle();
        let mut child = NativeHost(
            Command::new(executable)
                .arg(format!("chrome-extension://{extension_id}/"))
                .env("QBUTT_BROWSER_BRIDGE_DISCOVERY", discovery)
                .creation_flags(0x0800_0000)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()?,
        );
        let mut input = child.0.stdin.take().ok_or("Native host stdin missing")?;
        let mut output = child.0.stdout.take().ok_or("Native host stdout missing")?;
        let hello = serde_json::to_vec(&hello("edge", "actual qcatch relay", true))?;
        input.write_all(&(hello.len() as u32).to_le_bytes())?;
        input.write_all(&hello)?;
        input.flush()?;
        wait_status(&handle, |status| {
            status
                .connections
                .iter()
                .any(|connection| connection.user_agent == "actual qcatch relay")
        })
        .await?;
        let pending = request(&handle, BrowserChoice::Edge, cookies());
        let wire = tokio::task::spawn_blocking(move || {
            let mut header = [0u8; 4];
            output.read_exact(&mut header)?;
            let count = u32::from_le_bytes(header) as usize;
            if count == 0 || count > MAX_FRAME {
                return Err(std::io::Error::other("Native host frame invalid"));
            }
            let mut body = vec![0u8; count];
            output.read_exact(&mut body)?;
            serde_json::from_slice::<Value>(&body).map_err(std::io::Error::other)
        });
        let wire = timeout(Duration::from_secs(3), wire).await???;
        assert_eq!(wire["method"], "cookies");
        let result = serde_json::to_vec(&json!({"id":wire["id"],"result":[]}))?;
        input.write_all(&(result.len() as u32).to_le_bytes())?;
        input.write_all(&result)?;
        input.flush()?;
        assert!(matches!(pending.await?, Ok(BrowserReply::Cookies(_))));
        drop(input);
        let exit = timeout(Duration::from_secs(3), async {
            loop {
                if let Some(exit) = child.0.try_wait()? {
                    return Ok::<_, std::io::Error>(exit);
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await??;
        assert!(exit.success());
        broker.shutdown().await?;
        Ok(())
    }

    fn cookies() -> BrowserCommand {
        BrowserCommand::Cookies {
            url: "https://fixture.example/".parse().unwrap(),
        }
    }

    fn request(
        handle: &BrowserHandle,
        choice: BrowserChoice,
        command: BrowserCommand,
    ) -> JoinHandle<Result<BrowserReply, BrowserError>> {
        let handle = handle.clone();
        tokio::spawn(async move { handle.request(choice, command).await })
    }

    async fn exchange(
        handle: &BrowserHandle,
        edge: &mut NamedPipeClient,
        command: Value,
        result: Value,
        expected: Option<BrowserError>,
    ) -> Result<Value, Box<dyn std::error::Error>> {
        let command: BrowserCommand = serde_json::from_value(command)?;
        let pending = request(handle, BrowserChoice::Edge, command);
        let wire = read(edge).await?;
        write_fragmented(edge, &json!({"id":wire["id"],"result":result})).await?;
        match (pending.await?, expected) {
            (Ok(reply), None) => Ok(serde_json::to_value(reply)?),
            (Err(error), Some(expected)) if error == expected => Ok(Value::Null),
            (Err(error), expected) => {
                panic!("Unexpected bounded error {error:?}; expected {expected:?}")
            }
            (Ok(_), Some(expected)) => panic!("Expected {expected:?}"),
        }
    }

    fn assert_error(result: Result<BrowserReply, BrowserError>, expected: BrowserError) {
        match result {
            Err(error) => assert_eq!(error, expected),
            Ok(_) => panic!("Expected {expected:?}"),
        }
    }

    fn hello(browser: &str, user_agent: &str, vk_search: bool) -> Value {
        json!({"type":"hello","version":1,"browser":browser,"userAgent":user_agent,"capabilities":if vk_search {vec!["vk-search"]} else {vec![]}})
    }

    async fn connect(discovery: &Path) -> Result<NamedPipeClient, Box<dyn std::error::Error>> {
        let value: Value = serde_json::from_slice(&fs::read(discovery)?)?;
        let pipe = value["pipe"].as_str().ok_or("Missing discovery pipe")?;
        for _ in 0..200 {
            match ClientOptions::new().open(pipe) {
                Ok(client) => return Ok(client),
                Err(error) if error.raw_os_error() == Some(231) => {
                    sleep(Duration::from_millis(10)).await
                }
                Err(error) => return Err(error.into()),
            }
        }
        Err("Pipe connection timeout".into())
    }

    async fn extension(
        discovery: &Path,
        browser: &str,
        agent: &str,
        vk_search: bool,
    ) -> Result<NamedPipeClient, Box<dyn std::error::Error>> {
        let mut client = connect(discovery).await?;
        write(&mut client, &hello(browser, agent, vk_search)).await?;
        Ok(client)
    }

    async fn wait_status(
        handle: &BrowserHandle,
        predicate: impl Fn(&BrowserStatus) -> bool,
    ) -> FixtureResult {
        let mut status = handle.subscribe();
        timeout(Duration::from_secs(3), async {
            while !predicate(&status.borrow()) {
                status.changed().await?;
            }
            Ok::<_, Box<dyn std::error::Error>>(())
        })
        .await??;
        Ok(())
    }

    async fn closed(client: &mut NamedPipeClient) -> FixtureResult {
        let mut byte = [0u8; 1];
        let result = timeout(Duration::from_secs(7), client.read(&mut byte)).await?;
        assert!(
            matches!(result, Ok(0))
                || matches!(result, Err(ref error) if matches!(error.raw_os_error(), Some(109 | 232 | 233)))
        );
        Ok(())
    }

    async fn read(client: &mut NamedPipeClient) -> Result<Value, Box<dyn std::error::Error>> {
        timeout(Duration::from_secs(3), async {
            let length = client.read_u32_le().await? as usize;
            if length == 0 || length > MAX_FRAME {
                return Err("Invalid fixture frame".into());
            }
            let mut body = vec![0u8; length];
            client.read_exact(&mut body).await?;
            Ok(serde_json::from_slice(&body)?)
        })
        .await?
    }

    async fn write(client: &mut NamedPipeClient, value: &Value) -> FixtureResult {
        let bytes = serde_json::to_vec(value)?;
        client
            .write_all(&(bytes.len() as u32).to_le_bytes())
            .await?;
        client.write_all(&bytes).await?;
        Ok(())
    }

    async fn write_fragmented(client: &mut NamedPipeClient, value: &Value) -> FixtureResult {
        let body = serde_json::to_vec(value)?;
        let bytes: Vec<_> = (body.len() as u32)
            .to_le_bytes()
            .into_iter()
            .chain(body)
            .collect();
        for piece in bytes.chunks(31) {
            client.write_all(piece).await?;
            tokio::task::yield_now().await;
        }
        Ok(())
    }

    fn private_acl(handle: std::os::windows::io::RawHandle) {
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
