use qbutt_content::TorrentMetadata;
use url::Url;

use super::metadata;

pub(super) fn response(
    url: &Url,
    origin: &Url,
    fault: &str,
) -> (&'static str, &'static str, String, Vec<u8>) {
    let query = url
        .query_pairs()
        .find(|(key, _)| key == "s")
        .map(|(_, value)| value.into_owned());
    if url.path() == "/" {
        let query = query.unwrap_or_default();
        let location = match query.as_str() {
            "one" => Some("/generated-game/"),
            "foreign-redirect" => Some("https://external.example/"),
            "http-redirect" => Some("http://fixture.example/"),
            "private-redirect" => Some("https://127.0.0.1/"),
            "redirect-loop" => Some("/?s=redirect-loop"),
            _ => None,
        };
        if let Some(location) = location {
            return (
                "302 Found",
                "text/html",
                format!("Location: {location}\r\n"),
                Vec::new(),
            );
        }
        if query == "challenge" {
            return (
                "403 Forbidden",
                "text/html",
                String::new(),
                b"challenge".to_vec(),
            );
        }
        if query == "oversized" {
            return (
                "200 OK",
                "text/html",
                String::new(),
                vec![b'x'; 2 * 1024 * 1024 + 1],
            );
        }
        let shown = if query == "wrong-query" {
            "other"
        } else {
            &query
        };
        let link = if query == "foreign-link" {
            "https://external.example/game/"
        } else {
            "/generated-game/"
        };
        let row = format!(
            "<article class='post'><h2 class='post-title'><a href='{link}'>Generated Legal Game</a></h2></article>"
        );
        let rows = match query.as_str() {
            "empty" => String::new(),
            "duplicate" => row.repeat(2),
            _ => format!(
                "{row}<article class='post'><h2 class='post-title'><a href='/another-game/'>Another Legal Game</a></h2></article>"
            ),
        };
        let count = if query == "empty" || query == "empty-with-results" {
            0
        } else {
            2
        };
        let note = if count == 0 {
            "ничего не найдено"
        } else {
            "найдено"
        };
        let body = if query == "malformed" {
            "<html>unexpected page</html>".to_owned()
        } else {
            format!(
                "<section class='content'>
                    <div class='page-title'><h1>{count} Search results</h1></div>
                    <div class='notebox'>По запросу <span>{shown}</span> {note}</div>
                    <div id='grid-wrapper'><div class='post-row'>{rows}</div></div>
                </section>"
            )
        };
        let mime = if query == "wrong-mime" {
            "application/json"
        } else {
            "text/html; charset=UTF-8"
        };
        return ("200 OK", mime, String::new(), body.into_bytes());
    }
    if url.path().starts_with("/download/") {
        if fault == "all-gone" {
            return ("404 Not Found", "text/html", String::new(), Vec::new());
        }
        if url.path().ends_with("Legal_Game_v1.torrent") {
            return (
                "200 OK",
                "application/octet-stream",
                String::new(),
                metadata("Legal Game 1"),
            );
        }
        return match fault {
            "changed" => (
                "200 OK",
                "application/octet-stream",
                String::new(),
                metadata("Changed Game 2"),
            ),
            "gone" => ("410 Gone", "text/html", String::new(), Vec::new()),
            "not-found" => ("404 Not Found", "text/html", String::new(), Vec::new()),
            "substituted" => (
                "302 Found",
                "text/html",
                "Location: /download/Legal_Game_v1.torrent\r\n".into(),
                Vec::new(),
            ),
            "foreign" => (
                "302 Found",
                "text/html",
                "Location: https://external.example/other.torrent\r\n".into(),
                Vec::new(),
            ),
            "wrong-mime" => (
                "200 OK",
                "text/html",
                String::new(),
                metadata("Legal Game 2"),
            ),
            "malformed" => (
                "200 OK",
                "application/octet-stream",
                String::new(),
                b"not a torrent".to_vec(),
            ),
            "oversized" => (
                "200 OK",
                "application/octet-stream",
                String::new(),
                vec![b'x'; TorrentMetadata::MAX_BYTES + 1],
            ),
            "challenge" => (
                "503 Unavailable",
                "text/html",
                String::new(),
                b"challenge".to_vec(),
            ),
            "server-error" => (
                "500 Server Error",
                "text/html",
                String::new(),
                b"temporary failure".to_vec(),
            ),
            "unauthorized" => (
                "401 Unauthorized",
                "text/html",
                String::new(),
                b"authorization required".to_vec(),
            ),
            _ => (
                "200 OK",
                "Application/Octet-Stream",
                String::new(),
                metadata("Legal Game 2"),
            ),
        };
    }
    let slug = url.path().trim_matches('/');
    match slug {
        "missing" => return ("404 Not Found", "text/html", String::new(), Vec::new()),
        "gone" => return ("410 Gone", "text/html", String::new(), Vec::new()),
        "redirect" => {
            return (
                "302 Found",
                "text/html",
                "Location: /generated-game/\r\n".into(),
                Vec::new(),
            );
        }
        "challenge" => {
            return (
                "200 OK",
                "text/html",
                String::new(),
                b"Just a moment cf-challenge".to_vec(),
            );
        }
        _ => {}
    }
    let canonical = if slug == "identity" {
        "another-game"
    } else {
        slug
    };
    let canonical_link = format!("<link rel='canonical' href='{origin}{canonical}/'>");
    let canonical_links = if slug == "duplicate-canonical" {
        canonical_link.repeat(2)
    } else {
        canonical_link
    };
    let links = match slug {
        "no-torrent" => "
            <a class='btn_green' href='https://external.example/game.torrent'>Foreign</a>
            <a class='btn_blue' href='/download/blue.torrent'>Tool</a>
        "
        .to_owned(),
        "too-many" => (0..17)
            .map(|index| {
                format!("<a class='btn_green' href='/download/{index}.torrent'>Torrent</a>")
            })
            .collect(),
        _ => "
            <a class='btn_green' href='/download/Legal_Game_v1.torrent'>Torrent</a>
            <a class='btn_green' href='/download/Legal_Game_v1.torrent'>Repeated link</a>
            <a class='btn_green' href='/download/Legal_Game_v2.torrent'>Torrent</a>
            <a class='btn_green' href='/download/alias.torrent'>Same hash</a>
            <a class='btn_blue' href='/download/blue.torrent'>Tool</a>
        "
        .to_owned(),
    };
    let instruction_fault = match slug {
        "long-instructions" => "x".repeat(16_001),
        "deep-instructions" => format!("{}deep{}", "<div>".repeat(34), "</div>".repeat(34)),
        _ => String::new(),
    };
    let introduction = if slug == "no-introduction" {
        ""
    } else {
        "<p>A generated one-byte legal fixture.</p>"
    };
    let body = format!(
        "<html>
            <head>{canonical_links}</head>
            <body>
                <section class='content'><div class='pad group'><article class='post'>
                    <h1 class='post-title'>Скачать игру Generated Legal Game [Новая Версия] на ПК (на Русском)</h1>
                    <div class='entry-inner'>
                        {introduction}
                        <div>
                            <center><strong>Информация о игре</strong></center>
                            <strong>Версия:</strong> v2 GOG Полная<br>
                            <strong>Язык интерфейса:</strong> английский, русский<br>
                        </div>
                        <div>
                            <strong>Важно</strong><br>
                            {instruction_fault}
                            Установка:<br>
                            <ol>
                                <li>Распакуйте архив в выбранную папку.</li>
                                <li>Откройте <strong>legal-fixture.txt</strong>.</li>
                            </ol>
                            <h3>Совместная игра</h3>
                            <p>Подключитесь к локальному серверу:</p>
                            <ul>
                                <li>Создайте комнату.</li>
                                <li>Пригласите друга.</li>
                            </ul>
                            <script>UNWANTED_SCRIPT</script>
                            <style>UNWANTED_STYLE</style>
                            <nav>UNWANTED_NAVIGATION</nav>
                            <div class='advertisement'>UNWANTED_AD</div>
                            <a class='btn_blue' href='/download/blue.torrent'>UNWANTED_TOOL</a>
                        </div>
                        <div>UNWANTED_PROMOTION</div>
                        {links}
                    </div>
                </article></div></section>
            </body>
        </html>"
    );
    (
        "200 OK",
        "text/html; charset=UTF-8",
        String::new(),
        body.into_bytes(),
    )
}
