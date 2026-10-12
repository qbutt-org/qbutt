use encoding_rs::WINDOWS_1251;
use url::Url;

const PLAYER_HASH: &str = "cccccccccccccccccccccccccccccccc";
const EPISODE_ONE_HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const EPISODE_TWO_HASH: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

pub(super) fn response(host: &str, target: &str) -> Vec<u8> {
    let url = Url::parse(&format!("https://{host}{target}")).unwrap();
    match (url.host_str().unwrap(), url.path()) {
        ("my.animespirit.ru", "/index.php") => {
            let query = url.query().unwrap_or_default();
            if query.starts_with("do=search&") {
                assert_eq!(
                    target,
                    "/index.php?do=search&subaction=search&story=%C4%E5%EC%EE"
                );
                html(&format!(
                    "<html><meta charset='utf-8'><div id='dle-content'>{}</div></html>",
                    card(42)
                ))
            } else {
                let id = url
                    .query_pairs()
                    .find(|(name, _)| name == "newsid")
                    .unwrap()
                    .1
                    .parse::<u64>()
                    .unwrap();
                assert!((42..=44).contains(&id));
                html(&publication_page(id))
            }
        }
        ("kodikplayer.com", path) if path.starts_with("/serial/") => {
            let parts = url.path_segments().unwrap().collect::<Vec<_>>();
            assert_eq!(parts.len(), 4);
            assert_eq!(parts[2], PLAYER_HASH);
            assert_eq!(parts[3], "720p");
            let id = parts[1].parse::<u64>().unwrap();
            assert!((42..=43).contains(&id));
            let episode = url
                .query_pairs()
                .find(|(name, _)| name == "episode")
                .map(|(_, value)| value.into_owned());
            assert_eq!(
                url.query_pairs()
                    .find(|(name, _)| name == "only_translations")
                    .map(|(_, value)| value.into_owned()),
                Some("1".into())
            );
            if id == 43 {
                utf8(
                    "text/html; charset=utf-8",
                    "<html><div class='serial-series-box'></div></html>",
                )
            } else {
                utf8("text/html; charset=utf-8", &player_page(episode.as_deref()))
            }
        }
        ("kodikplayer.com", "/ftor") => utf8(
            "application/json",
            r#"{"links":{"720":[{"src":"https://cdn.fixture.example/42/advertised.m3u8","type":"application/x-mpegURL"}]}}"#,
        ),
        ("cdn.fixture.example", "/42/advertised.m3u8") => http(
            "302 Found",
            "application/vnd.apple.mpegurl",
            Vec::new(),
            Some("https://cdn.fixture.example/42/final.m3u8"),
        ),
        ("cdn.fixture.example", "/42/final.m3u8") => utf8(
            "application/vnd.apple.mpegurl",
            "#EXTM3U\n#EXTINF:1,\ngenerated.ts\n#EXT-X-ENDLIST\n",
        ),
        _ => panic!("unexpected fixture request: {host}{target}"),
    }
}

fn publication_page(id: u64) -> String {
    format!(
        "<html><meta charset='utf-8'><div id='dle-content'>{}
            <h3 id='top_div_1'>Субтитры Демо студии</h3>
            <p id='an_ul1'>//kodikplayer.com/serial/{id}/{PLAYER_HASH}/720p?only_translations=1</p>
        </div></html>",
        card(id)
    )
}

fn card(id: u64) -> String {
    format!(
        "<article class='content-block'><div class='content-block-title'>
            <h2><a href='/anime/{id}-generated.html'>Демо архив</a></h2>
        </div></article>"
    )
}

fn player_page(episode: Option<&str>) -> String {
    let (id, hash) = if episode == Some("2") {
        ("4202", EPISODE_TWO_HASH)
    } else {
        ("4201", EPISODE_ONE_HASH)
    };
    let selected_one = if episode == Some("2") { "" } else { "selected" };
    let selected_two = if episode == Some("2") { "selected" } else { "" };
    format!(
        r#"<html><script>
            var translationTitle = "Fixture Studio.Subtitles";
            vInfo.type = 'seria';
            vInfo.id = '{id}';
            vInfo.hash = '{hash}';
            var domain = "fixture-domain";
            var d_sign = "generated-signature";
            var pd = "generated-pd";
            var pd_sign = "generated-pd-signature";
            var ref = "generated-ref";
            var ref_sign = "generated-ref-signature";
        </script>
        <div class='serial-series-box'><select>
            <option value='1' data-id='4201' data-hash='{EPISODE_ONE_HASH}' data-other-translation='false' {selected_one}>Generated episode one</option>
            <option value='2' data-id='4202' data-hash='{EPISODE_TWO_HASH}' data-other-translation='false' {selected_two}>Generated episode two</option>
        </select></div></html>"#
    )
}

fn html(body: &str) -> Vec<u8> {
    let (encoded, _, invalid) = WINDOWS_1251.encode(body);
    assert!(!invalid);
    http(
        "200 OK",
        "text/html; charset=utf-8",
        encoded.into_owned(),
        None,
    )
}

fn utf8(mime: &str, body: &str) -> Vec<u8> {
    http("200 OK", mime, body.as_bytes().to_vec(), None)
}

fn http(status: &str, mime: &str, body: Vec<u8>, location: Option<&str>) -> Vec<u8> {
    let location = location
        .map(|address| format!("Location: {address}\r\n"))
        .unwrap_or_default();
    let mut response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\n{location}Connection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    response.extend(body);
    response
}
