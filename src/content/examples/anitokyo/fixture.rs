use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer};
use url::Url;

use super::{Error, http, response};

fn page(query: &str, origin: &Url) -> String {
    if query == "challenge" {
        return "<form id='challenge-form'></form>".into();
    }
    let echoed = if query == "wrong echo" {
        "another"
    } else {
        query
    };
    let form = format!(
        "<form id='fullsearch'><input name='do' value='search'><input id='searchinput' name='story' value='{echoed}'></form>"
    );
    let rows = if query == "empty" {
        String::new()
    } else {
        let article = |path: &str, title: &str| {
            format!(
                "<article class='story shortstory'><h2 class='story-title'><a href='{origin}{path}'>{title}</a></h2></article>"
            )
        };
        let first = article("anime/42-generated-anime.html", "Generated Anime");
        let second = if query == "duplicate" {
            first.clone()
        } else {
            article("ONA/43-generated-original.html", "Generated Original")
        };
        let rows = format!("{first}{second}");
        if query == "foreign" {
            rows.replace(origin.as_str(), "https://ads.example/")
        } else {
            rows
        }
    };
    format!("{form}<main class='main'>{rows}</main>")
}

pub(super) async fn run() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let certificate = rcgen::generate_simple_self_signed(vec!["fixture.example".into()])?;
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![certificate.cert.der().clone()],
            PrivatePkcs8KeyDer::from(certificate.signing_key.serialize_der()).into(),
        )?;
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let origin = Url::parse(&format!("https://fixture.example:{}/", address.port()))?;
    let client = reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .add_root_certificate(reqwest::Certificate::from_der(certificate.cert.der())?)
        .resolve("fixture.example", address)
        .timeout(Duration::from_secs(3))
        .build()?;
    let (stop, mut stopped) = oneshot::channel();
    let server = tokio::spawn({
        let origin = origin.clone();
        async move {
            let mut clients = JoinSet::new();
            loop {
                let socket = tokio::select! {
                    _ = &mut stopped => break,
                    socket = listener.accept() => socket?.0,
                };
                let acceptor = acceptor.clone();
                let origin = origin.clone();
                clients.spawn(async move {
                    let mut stream = acceptor.accept(socket).await?;
                    let mut request = Vec::new();
                    while !request.ends_with(b"\r\n\r\n") && request.len() < 8192 {
                        request.push(stream.read_u8().await?);
                    }
                    let request = String::from_utf8(request)?;
                    let target = request
                        .split_whitespace()
                        .nth(1)
                        .ok_or("missing request target")?;
                    let url = origin.join(target)?;
                    assert_eq!(url.path(), "/index.php");
                    let params = url
                        .query_pairs()
                        .collect::<std::collections::BTreeMap<_, _>>();
                    assert_eq!(params.get("do").map(|value| value.as_ref()), Some("search"));
                    assert_eq!(
                        params.get("subaction").map(|value| value.as_ref()),
                        Some("search")
                    );
                    let query = params.get("story").ok_or("missing story")?;
                    let status = if query == "forbidden" {
                        "403 Forbidden"
                    } else {
                        "200 OK"
                    };
                    let body = page(query, &origin);
                    let response = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    stream.write_all(response.as_bytes()).await?;
                    Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
                });
            }
            while let Some(result) = clients.join_next().await {
                result??;
            }
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        }
    });
    let http = http::fixture(client, origin.clone());
    for query in [
        "Re:Zero",
        "empty",
        "wrong echo",
        "duplicate",
        "foreign",
        "challenge",
        "forbidden",
    ] {
        let mut url = origin.join("index.php")?;
        url.query_pairs_mut()
            .append_pair("do", "search")
            .append_pair("subaction", "search")
            .append_pair("story", query);
        let fetched = http.html_page(url.clone()).await;
        if query == "forbidden" {
            assert!(matches!(fetched, Err(Error::HttpStatus(403))));
            continue;
        }
        let (arrived, body) = fetched?;
        assert_eq!(arrived, url);
        let result = response::search(&body, &origin, query);
        match query {
            "Re:Zero" => {
                let works = result?;
                assert_eq!(works.len(), 2);
                assert_eq!(works[1].reference.id, "43");
                assert_eq!(works[1].url.path(), "/ONA/43-generated-original.html");
            }
            "empty" => assert!(result?.is_empty()),
            "foreign" => assert!(matches!(result, Err(Error::UnsafeAddress))),
            _ => assert!(matches!(result, Err(Error::InvalidResponse(_)))),
        }
    }
    stop.send(()).ok();
    server
        .await?
        .map_err(|error| error as Box<dyn std::error::Error>)?;
    Ok(())
}
