use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use rcgen::generate_simple_self_signed;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::timeout;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer};

use crate::browser::{BrowserBroker, BrowserChoice, BrowserCommand, BrowserError};
use crate::lib_api::LibSession;
use crate::{Error, Source};

type FixtureResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[path = "../session/pipe.rs"]
mod pipe;

pub(super) async fn run() -> FixtureResult {
    let directory = std::env::temp_dir().join(format!("qbutt-lib-session-{}", std::process::id()));
    fs::create_dir(&directory)?;
    let discovery = directory.join("browser.json");
    let broker = BrowserBroker::start(discovery.clone()).await?;
    let handle = broker.handle();
    let mut anonymous =
        LibSession::from_browser(Source::RanobeLib, Some(&handle), BrowserChoice::Auto).await?;
    let edge = pipe::connect(&discovery, "edge", "generated old extension").await?;
    pipe::wait_agent(&handle, "generated old extension").await?;
    assert!(matches!(
        LibSession::from_browser(Source::RanobeLib, Some(&handle), BrowserChoice::Edge).await,
        Err(Error::BrowserSession(BrowserError::UpdateRequired))
    ));
    drop(edge);
    let discovery_value: Value = serde_json::from_slice(&fs::read(&discovery)?)?;
    let mut edge = tokio::net::windows::named_pipe::ClientOptions::new()
        .open(discovery_value["pipe"].as_str().unwrap())?;
    pipe::write(&mut edge, &json!({"type":"hello","version":1,"browser":"edge","userAgent":"generated lib extension","capabilities":["lib-session"]})).await?;
    pipe::wait_agent(&handle, "generated lib extension").await?;
    println!("PASS no connected browser is anonymous; missing capability requires update");

    let generated =
        generate_simple_self_signed(vec!["api.cdnlibs.org".into(), "img3.cdnlibs.org".into()])?;
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![generated.cert.der().clone()],
            PrivatePkcs8KeyDer::from(generated.signing_key.serialize_der()).into(),
        )?;
    let listener = TcpListener::bind("127.0.0.1:443").await?;
    let requests = Arc::new(AtomicUsize::new(0));
    let captured = requests.clone();
    let server = tokio::spawn(async move {
        let acceptor = TlsAcceptor::from(Arc::new(config));
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            let mut stream = acceptor.accept(socket).await.unwrap();
            let mut bytes = Vec::new();
            while !bytes.ends_with(b"\r\n\r\n") {
                let byte = stream.read_u8().await.unwrap();
                bytes.push(byte);
                assert!(bytes.len() < 64 * 1024);
            }
            let request = String::from_utf8(bytes).unwrap().to_ascii_lowercase();
            captured.fetch_add(1, Ordering::SeqCst);
            let is_image = request.starts_with("get /image ");
            let (status, mime, body) = if is_image {
                assert!(!request.contains("authorization:"));
                assert!(request.contains("cookie: image=generated"));
                assert!(request.contains("user-agent: generated lib extension"));
                (200, "image/png", "generated image")
            } else if request.starts_with("get /redirect ") {
                let response = "HTTP/1.1 302 Found\r\nLocation: https://img3.cdnlibs.org/image\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                stream.write_all(response.as_bytes()).await.unwrap();
                continue;
            } else if request.contains("authorization: bearer generated.stale") {
                (401, "application/json", "{}")
            } else if request.contains("authorization: bearer generated.current") {
                assert!(request.contains("cookie: api=generated"));
                assert!(request.contains("user-agent: generated lib extension"));
                (200, "application/json", "{\"data\":[1,2]}")
            } else {
                (200, "application/json", "{\"data\":[1]}")
            };
            let response = format!(
                "HTTP/1.1 {status} Fixture\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        }
    });
    let client = reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .add_root_certificate(reqwest::Certificate::from_der(generated.cert.der())?)
        .resolve("api.cdnlibs.org", "127.0.0.1:443".parse()?)
        .resolve("img3.cdnlibs.org", "127.0.0.1:443".parse()?)
        .build()?;
    let http = crate::lib_api::fixture(client);
    let public: Value = http
        .json(http.endpoint("/api/manga"), &mut anonymous)
        .await?;
    assert_eq!(public["data"].as_array().unwrap().len(), 1);

    let selected = handle.clone();
    let pending = tokio::spawn(async move {
        LibSession::from_browser(Source::RanobeLib, Some(&selected), BrowserChoice::Edge).await
    });
    let wire = pipe::read(&mut edge).await?;
    assert_eq!(wire["method"], "lib-session");
    assert_eq!(wire["url"], "https://ranobelib.me/");
    pipe::write(
        &mut edge,
        &json!({"id":wire["id"],"result":fixture_session("generated.stale")}),
    )
    .await?;
    let mut session = pending.await??;

    let pending = tokio::spawn(async move {
        let value: Value = http.json(http.endpoint("/api/manga"), &mut session).await?;
        Ok::<_, Error>((http, session, value))
    });
    let wire = pipe::read(&mut edge).await?;
    assert_eq!(wire["method"], "lib-session");
    pipe::write(
        &mut edge,
        &json!({"id":wire["id"],"result":fixture_session("generated.current")}),
    )
    .await?;
    let (http, mut session, value) = pending.await??;
    assert_eq!(value["data"].as_array().unwrap().len(), 2);
    assert_eq!(requests.load(Ordering::SeqCst), 3);
    println!(
        "PASS partial anonymous search, pre-request account import, one 401 reimport and authenticated retry"
    );

    let before = requests.load(Ordering::SeqCst);
    assert!(matches!(
        http.json::<Value>(http.endpoint("/redirect"), &mut session)
            .await,
        Err(Error::UnsafeAddress)
    ));
    assert_eq!(requests.load(Ordering::SeqCst), before + 1);
    let pending = tokio::spawn(async move {
        let image = http
            .image(
                "https://img3.cdnlibs.org/image".parse().unwrap(),
                &["img3.cdnlibs.org"],
                &mut session,
            )
            .await?;
        Ok::<_, Error>((http, session, image))
    });
    let wire = pipe::read(&mut edge).await?;
    assert_eq!(wire["method"], "cookies");
    assert_eq!(wire["url"], "https://img3.cdnlibs.org/image");
    pipe::write(
        &mut edge,
        &json!({"id":wire["id"],"result":[cookie("image", "img3.cdnlibs.org", "/")]}),
    )
    .await?;
    let (mut http, _, image) = pending.await??;
    assert_eq!(image, b"generated image");
    let mut foreign = LibSession::anonymous(Source::MangaLib)?;
    assert!(matches!(
        http.json::<Value>(http.endpoint("/api/manga"), &mut foreign)
            .await,
        Err(Error::InvalidReference)
    ));
    println!(
        "PASS bearer cannot cross API redirect; CDN receives only scoped cookies and UA; wrong Lib source rejected"
    );

    for refreshed in [Some("generated.stale"), None] {
        let selected = handle.clone();
        let pending = tokio::spawn(async move {
            LibSession::from_browser(Source::RanobeLib, Some(&selected), BrowserChoice::Edge).await
        });
        let wire = pipe::read(&mut edge).await?;
        pipe::write(
            &mut edge,
            &json!({"id":wire["id"],"result":fixture_session("generated.stale")}),
        )
        .await?;
        let mut stale = pending.await??;
        let before = requests.load(Ordering::SeqCst);
        let pending = tokio::spawn(async move {
            let result = http
                .json::<Value>(http.endpoint("/api/manga"), &mut stale)
                .await;
            (http, result)
        });
        let wire = pipe::read(&mut edge).await?;
        pipe::write(
            &mut edge,
            &json!({"id":wire["id"],"result":{"accessToken":refreshed,"cookies":[]}}),
        )
        .await?;
        let (returned, result) = pending.await?;
        http = returned;
        assert!(matches!(result, Err(Error::LoginRequired)));
        assert_eq!(
            requests.load(Ordering::SeqCst),
            before + if refreshed.is_some() { 2 } else { 1 }
        );
    }
    println!(
        "PASS rejected reimport stops after one retry; removed credentials never retry anonymously"
    );

    for invalid in [
        json!({"accessToken":"bad\r\nheader","cookies":[]}),
        json!({"accessToken":"x".repeat(16*1024+1),"cookies":[]}),
        json!({"accessToken":"generated.current","cookies":[cookie("bad", "other.example", "/")]}),
    ] {
        let selected = handle.clone();
        let pending = tokio::spawn(async move {
            LibSession::from_browser(Source::MangaLib, Some(&selected), BrowserChoice::Edge).await
        });
        let wire = pipe::read(&mut edge).await?;
        pipe::write(&mut edge, &json!({"id":wire["id"],"result":invalid})).await?;
        assert!(matches!(
            pending.await?,
            Err(Error::BrowserSession(BrowserError::InvalidResponse))
        ));
    }
    assert!(
        BrowserCommand::LibSession {
            url: "https://evil.example/".parse()?
        }
        .validate()
        .is_err()
    );
    let selected = handle.clone();
    let pending = tokio::spawn(async move {
        LibSession::from_browser(Source::MangaLib, Some(&selected), BrowserChoice::Edge).await
    });
    let wire = pipe::read(&mut edge).await?;
    pending.abort();
    let canceled = pipe::read(&mut edge).await?;
    assert_eq!(canceled["method"], "cancel");
    assert_eq!(canceled["id"], wire["id"]);
    println!("PASS bounded token/cookie/origin validation and in-flight cancellation");
    drop(edge);
    broker.shutdown().await?;
    assert!(!discovery.exists());
    fs::remove_dir(directory)?;
    server.abort();
    assert!(
        timeout(Duration::from_secs(2), server)
            .await?
            .unwrap_err()
            .is_cancelled()
    );
    Ok(())
}

fn cookie(name: &str, domain: &str, path: &str) -> Value {
    json!({"name":name,"value":"generated","domain":domain,"path":path,"secure":true,"hostOnly":true})
}

fn fixture_session(token: &str) -> Value {
    json!({"accessToken":token,"cookies":[cookie("api", "api.cdnlibs.org", "/api")]})
}
