use std::sync::atomic::Ordering;

use super::{FixtureResult, tls::Fixture};
use crate::http::SourceSession;
use crate::{Error, TorrentMetadata};

pub(super) const TORRENT: &[u8] =
    b"d4:infod6:lengthi0e4:name9:Generated12:piece lengthi16384e6:pieces0:ee";

pub(super) async fn run(fixture: &Fixture) -> FixtureResult {
    let http = fixture.http(true)?;
    for code in [301, 302, 303, 307, 308] {
        let mut session = SourceSession::anonymous(fixture.origin.clone())?;
        let document = http
            .session_metadata(
                fixture.origin.join(&format!("/metadata/form/{code}"))?,
                &mut session,
                Some("id=fixture&op=download1"),
            )
            .await?;
        assert_eq!(document.status, 200);
        assert_eq!(
            document.content_type.as_deref(),
            Some("application/x-bittorrent")
        );
        assert_eq!(document.body, TORRENT);
        let final_path = if matches!(code, 307 | 308) {
            "/metadata/post"
        } else {
            "/metadata/get"
        };
        assert_eq!(document.final_url.path(), final_path);
        let metadata = TorrentMetadata::new(document.body)?;
        assert_eq!(metadata.bytes(), TORRENT);
    }
    println!("PASS metadata form redirect methods, exact raw bytes and per-hop cookies");

    let mut session = SourceSession::anonymous(fixture.origin.clone())?;
    let gate = http
        .session_metadata(fixture.origin.join("/metadata/gate")?, &mut session, None)
        .await?;
    assert_eq!(gate.status, 403);
    assert_eq!(gate.content_type.as_deref(), Some("text/html"));
    assert_eq!(gate.body, b"<html>Generated verification gate</html>");
    assert!(matches!(
        http.session_metadata(
            fixture.origin.join("/metadata/cross")?,
            &mut session,
            Some("id=fixture")
        )
        .await,
        Err(Error::UnsafeAddress)
    ));
    let accepted = fixture.accepted.load(Ordering::SeqCst);
    let form = "x".repeat(16 * 1024 + 1);
    assert!(matches!(
        http.session_metadata(
            fixture.origin.join("/metadata/cross")?,
            &mut session,
            Some(&form)
        )
        .await,
        Err(Error::ResponseTooLarge)
    ));
    assert_eq!(accepted, fixture.accepted.load(Ordering::SeqCst));
    println!("PASS metadata HTML gates remain raw, form bounds and foreign redirects");

    let document = http
        .session_metadata(fixture.origin.join("/metadata/large")?, &mut session, None)
        .await?;
    assert_eq!(document.body.len(), TorrentMetadata::MAX_BYTES);
    assert!(matches!(
        http.session_metadata(
            fixture.origin.join("/metadata/oversize")?,
            &mut session,
            None
        )
        .await,
        Err(Error::ResponseTooLarge)
    ));
    assert!(matches!(
        http.session_document(fixture.origin.join("/metadata/large")?, &mut session)
            .await,
        Err(Error::ResponseTooLarge)
    ));
    println!("PASS shared 10 MiB metadata bound and independent 2 MiB document bound");
    Ok(())
}
