use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crate::Error;
use crate::http::{SourceHttp, SourceSession};
use reqwest::header::HeaderMap;
use tokio::time::{sleep, timeout};

use super::{FixtureResult, tls::Fixture};

pub(super) async fn run(fixture: &Fixture, source: &mut SourceSession) -> FixtureResult {
    let http = fixture.http(true)?;
    let document = http
        .session_document(fixture.origin.join("/forum/start")?, source)
        .await?;
    assert_eq!(document.status, 403);
    assert_eq!(
        document.content_type.as_deref(),
        Some("text/html; charset=windows-1251")
    );
    assert_eq!(document.final_url, fixture.origin.join("/forum/final")?);
    let (text, _, invalid) = encoding_rs::WINDOWS_1251.decode(&document.body);
    assert!(!invalid);
    assert_eq!(text, "Проверка источника");
    sleep(Duration::from_millis(1100)).await;
    http.session_document(fixture.origin.join("/forum/after")?, source)
        .await?;
    http.session_document(fixture.origin.join("/forum/deleted")?, source)
        .await?;
    http.session_document(fixture.origin.join("/outside")?, source)
        .await?;
    println!("PASS scoped cookies, replacement, deletion, expiry and per-hop updates");
    println!("PASS raw status, charset, final URL and CP1251 challenge body");

    let accepted = fixture.accepted.load(Ordering::SeqCst);
    assert!(matches!(
        http.session_document("https://other.example/".parse()?, source)
            .await,
        Err(Error::UnsafeAddress)
    ));
    let mut other = SourceSession::anonymous("https://other.example/".parse()?)?;
    assert!(matches!(
        http.session_document(fixture.origin.clone(), &mut other)
            .await,
        Err(Error::UnsafeAddress)
    ));
    assert_eq!(accepted, fixture.accepted.load(Ordering::SeqCst));
    for route in ["/cross", "/scheme"] {
        assert!(matches!(
            http.session_document(fixture.origin.join(route)?, source)
                .await,
            Err(Error::UnsafeAddress)
        ));
    }
    assert!(matches!(
        http.session_document(fixture.origin.join("/loop")?, source)
            .await,
        Err(Error::InvalidRedirect)
    ));
    for route in ["/large-length", "/large-chunks"] {
        assert!(matches!(
            http.session_document(fixture.origin.join(route)?, source)
                .await,
            Err(Error::ResponseTooLarge)
        ));
    }
    let empty = http
        .session_document(fixture.origin.join("/empty")?, source)
        .await?;
    assert_eq!(empty.status, 401);
    assert!(empty.body.is_empty());
    println!("PASS exact HTTPS origin, redirect limits and 2 MiB body bounds");

    let mut anonymous = SourceSession::anonymous(fixture.origin.clone())?;
    assert!(matches!(
        http.session_document(fixture.origin.join("/cookie-length")?, &mut anonymous)
            .await,
        Err(Error::ResponseTooLarge)
    ));
    let mut anonymous = SourceSession::anonymous(fixture.origin.clone())?;
    for batch in 0..5 {
        http.session_document(
            fixture.origin.join(&format!("/cookie-count/{batch}"))?,
            &mut anonymous,
        )
        .await?;
    }
    assert!(matches!(
        http.session_document(fixture.origin.join("/cookie-count/5")?, &mut anonymous)
            .await,
        Err(Error::ResponseTooLarge)
    ));
    let mut anonymous = SourceSession::anonymous(fixture.origin.clone())?;
    http.session_document(fixture.origin.join("/cookie-header")?, &mut anonymous)
        .await?;
    let accepted = fixture.accepted.load(Ordering::SeqCst);
    assert!(matches!(
        http.session_document(fixture.origin.join("/empty")?, &mut anonymous)
            .await,
        Err(Error::ResponseTooLarge)
    ));
    assert_eq!(accepted, fixture.accepted.load(Ordering::SeqCst));
    println!("PASS cookie count, individual size and outgoing header limits");

    let untrusted = fixture.http(false)?;
    let mut anonymous = SourceSession::anonymous(fixture.origin.clone())?;
    assert!(matches!(
        untrusted
            .session_document(fixture.origin.join("/empty")?, &mut anonymous)
            .await,
        Err(Error::Network(_))
    ));
    assert!(SourceSession::anonymous("https://127.0.0.1/".parse()?).is_err());
    assert!(
        crate::http::public_addresses(vec!["1.1.1.1:443".parse()?, "127.0.0.1:443".parse()?])
            .is_err()
    );
    let origin = format!("https://localhost:{}/", fixture.origin.port().unwrap()).parse()?;
    let real_dns = SourceHttp::new(origin, HeaderMap::new())?;
    let mut local = SourceSession::anonymous(
        format!("https://localhost:{}/", fixture.origin.port().unwrap()).parse()?,
    )?;
    let accepted = fixture.accepted.load(Ordering::SeqCst);
    assert!(matches!(
        real_dns
            .session_document(
                format!("https://localhost:{}/", fixture.origin.port().unwrap()).parse()?,
                &mut local
            )
            .await,
        Err(Error::Network(_))
    ));
    assert_eq!(accepted, fixture.accepted.load(Ordering::SeqCst));
    println!("PASS certificate validation and production private-DNS rejection");

    assert!(
        timeout(
            Duration::from_millis(100),
            http.session_document(fixture.origin.join("/cancel")?, source)
        )
        .await
        .is_err()
    );
    timeout(Duration::from_secs(3), async {
        while !fixture.canceled.load(Ordering::SeqCst) {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    println!("PASS canceled HTTP future closes its incomplete response");
    let started = Instant::now();
    assert!(matches!(
        http.session_document(fixture.origin.join("/slow")?, source)
            .await,
        Err(Error::Timeout)
    ));
    assert!((Duration::from_secs(45)..Duration::from_secs(46)).contains(&started.elapsed()));
    println!("PASS 45-second total source deadline");
    Ok(())
}
