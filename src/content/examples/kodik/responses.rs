use base64::Engine;
use serde_json::json;
use url::Url;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum Case {
    Season = 1,
    Serial,
    Episode,
    Movie,
    Alias,
    Subtitles,
    Encoded,
    Form307,
    PageRedirect,
    OtherSeasonRedirect,
    OtherEpisodeRedirect,
    OtherVoice,
    SimilarAlias,
    AliasOtherTranslation,
    OtherSubtitleType,
    WrongEpisode,
    WrongId,
    WrongHash,
    OtherTranslation,
    DuplicateEpisode,
    UnselectedEpisode,
    ConflictingTitle,
    WrongVideoType,
    ChangedMovieId,
    DuplicateQuery,
    UnsupportedQuality,
    PaddedQuality,
    UnknownQuality,
    AmbiguousStream,
    MalformedEncoding,
    UnsafeAddress,
    UnsafeRedirect,
    ForeignFormRedirect,
    Form303,
    PageTooLarge,
    FormTooLarge,
    PlaylistTooLarge,
    PlaylistStreamTooLarge,
    InvalidPlaylist,
    RedirectLoop,
    Cancel,
    Timeout,
}

pub(super) fn response(id: u64, url: &Url) -> String {
    let case = |value: Case| id == value as u64;
    let is_api = url.path().starts_with("/anime/");
    let is_page = url.path().starts_with("/season/")
        || url.path().starts_with("/serial/")
        || url.path().starts_with("/seria/")
        || url.path().starts_with("/video/");
    let is_form = url.path() == "/ftor" || url.path() == "/ftor-final";
    let mut status = "200 OK";
    let mut extra = String::new();
    let mut mime = "application/json";
    let mut body = if is_api {
        let kind = if case(Case::Serial) {
            "serial"
        } else if case(Case::Episode) {
            "seria"
        } else if case(Case::Movie) || case(Case::ChangedMovieId) {
            "video"
        } else {
            "season"
        };
        let duplicate = if case(Case::DuplicateQuery) {
            "&episode=1"
        } else {
            ""
        };
        let voice =
            if case(Case::Alias) || case(Case::SimilarAlias) || case(Case::AliasOtherTranslation) {
                "Озвучка AniLibria"
            } else if case(Case::Subtitles) || case(Case::OtherSubtitleType) {
                "Субтитры Fixture studio"
            } else {
                "Озвучка Fixture studio"
            };
        json!({
            "response": {
                "anime_id": id,
                "anime_url": format!("legal-{id}"),
                "title": "Generated legal fixture",
                "videos": [{
                    "video_id": 1000 + id,
                    "number": "1",
                    "data": {"player": "Плеер Kodik", "dubbing": voice, "player_id": 4},
                    "iframe_url": format!("https://kodikplayer.com/{kind}/{id}/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/720p?episode=1&only_episode=true&only_season=true{duplicate}")
                }]
            }
        }).to_string()
    } else if is_page {
        assert!(
            !url.query_pairs()
                .any(|(key, _)| key == "only_episode" || key == "only_season")
        );
        mime = "text/html; charset=utf-8";
        if case(Case::OtherSeasonRedirect) {
            status = "302 Found";
            extra = format!(
                "Location: /season/{}/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/720p?episode=1\r\n",
                id + 100
            );
        } else if case(Case::PageRedirect) || case(Case::OtherEpisodeRedirect) {
            let redirected = url.query_pairs().any(|(key, _)| key == "redirected");
            if !redirected {
                let episode = if case(Case::OtherEpisodeRedirect) {
                    2
                } else {
                    1
                };
                status = "302 Found";
                extra = format!(
                    "Location: {}?episode={episode}&redirected=1\r\n",
                    url.path()
                );
            }
        }
        player_page(id)
    } else if is_form {
        if case(Case::ForeignFormRedirect) {
            status = "307 Temporary Redirect";
            extra = "Location: https://cdn.fixture.example/ftor\r\n".into();
        }
        if case(Case::Form303) {
            status = "303 See Other";
            extra = "Location: /ftor-final\r\n".into();
        }
        if case(Case::Form307) && url.path() == "/ftor" {
            status = "307 Temporary Redirect";
            extra = "Location: /ftor-final\r\n".into();
        }
        let address = if case(Case::UnsafeAddress) {
            "https://127.0.0.1/private".into()
        } else {
            format!("https://cdn.fixture.example/{id}/advertised.m3u8")
        };
        let address = if case(Case::Encoded) {
            base64::engine::general_purpose::STANDARD_NO_PAD
                .encode(address)
                .bytes()
                .map(|byte| match byte {
                    b'A'..=b'Z' => b'A' + (byte - b'A' + 8) % 26,
                    b'a'..=b'z' => b'a' + (byte - b'a' + 8) % 26,
                    _ => byte,
                })
                .map(char::from)
                .collect()
        } else if case(Case::MalformedEncoding) {
            "%%%".into()
        } else {
            address
        };
        let source = json!({"src":address,"type":"application/x-mpegURL"});
        let choices = if case(Case::AmbiguousStream) {
            vec![source.clone(), source]
        } else {
            vec![source]
        };
        let mut links = json!({
            "360": [{"src": "https://cdn.fixture.example/lower.m3u8", "type": "application/x-mpegURL"}],
            "720": choices
        });
        if case(Case::UnsupportedQuality) {
            links["2160"] = links["720"].clone();
        } else if case(Case::PaddedQuality) {
            links["0720"] = links["720"].clone();
        } else if case(Case::UnknownQuality) {
            links["2160p"] = links["720"].clone();
        }
        json!({"links":links}).to_string()
    } else {
        mime = "application/vnd.apple.mpegurl";
        if case(Case::UnsafeRedirect) {
            status = "302 Found";
            extra = "Location: https://127.0.0.1/private\r\n".into();
        } else if case(Case::RedirectLoop) || url.path().ends_with("advertised.m3u8") {
            status = "302 Found";
            extra = format!("Location: https://cdn.fixture.example/{id}/final.m3u8\r\n");
        }
        if case(Case::InvalidPlaylist) {
            "<html>Not media</html>".into()
        } else {
            "#EXTM3U\n#EXTINF:1,\ngenerated.ts\n#EXT-X-ENDLIST\n".into()
        }
    };
    let oversized = if is_page && case(Case::PageTooLarge)
        || !is_api && !is_page && !is_form && case(Case::PlaylistTooLarge)
    {
        Some(2 * 1024 * 1024 + 1)
    } else if is_form && case(Case::FormTooLarge) {
        Some(24 * 1024 * 1024 + 1)
    } else {
        None
    };
    if let Some(length) = oversized {
        extra.push_str(&format!("Content-Length: {length}\r\n"));
        body.clear();
    } else if !is_api && !is_page && !is_form && case(Case::PlaylistStreamTooLarge) {
        body = "x".repeat(2 * 1024 * 1024 + 1);
    } else {
        extra.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    format!("HTTP/1.1 {status}\r\nContent-Type: {mime}\r\n{extra}Connection: close\r\n\r\n{body}")
}

fn player_page(id: u64) -> String {
    let case = |value: Case| id == value as u64;
    let title = if case(Case::OtherVoice) {
        "Other studio"
    } else if case(Case::SimilarAlias) {
        "AniLibria.TV.extra"
    } else if case(Case::Alias) || case(Case::AliasOtherTranslation) {
        "AniLibria.TV"
    } else if case(Case::Subtitles) {
        "Fixture studio.Subtitles"
    } else {
        "Fixture studio"
    };
    // The public AniLibria selection reports AniLibria.TV twice, and its
    // selected episode explicitly has data-other-translation=false.
    let second_title = if case(Case::ConflictingTitle) {
        "Other studio"
    } else {
        title
    };
    let kind = if case(Case::Movie) || case(Case::ChangedMovieId) || case(Case::WrongVideoType) {
        "video"
    } else {
        "seria"
    };
    let video_id = if case(Case::ChangedMovieId) {
        id + 1
    } else {
        id
    };
    let option_id = if case(Case::WrongId) { id + 1 } else { id };
    let hash = if case(Case::WrongHash) {
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    } else {
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    };
    let episode = if case(Case::WrongEpisode) { "2" } else { "1" };
    let selected = if case(Case::UnselectedEpisode) {
        ""
    } else {
        "selected"
    };
    let other = if case(Case::OtherTranslation) || case(Case::AliasOtherTranslation) {
        "true"
    } else {
        "false"
    };
    let option = format!(
        "<option value='{episode}' data-id='{option_id}' data-hash='{hash}' data-other-translation='{other}' {selected}>Generated episode</option>"
    );
    let duplicate = if case(Case::DuplicateEpisode) {
        option.as_str()
    } else {
        ""
    };
    format!(
        r#"<html>
            <script>
                var translationTitle = {title:?};
                var translationTitle = {second_title:?};
                vInfo.type = '{kind}';
                vInfo.id = '{video_id}';
                vInfo.hash = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
                var domain = "fixture-domain";
                var d_sign = "generated-signature";
                var pd = "generated-pd";
                var pd_sign = "generated-pd-signature";
                var ref = "generated-ref";
                var ref_sign = "generated-ref-signature";
            </script>
            <div class='serial-series-box'><select>{option}{duplicate}</select></div>
        </html>"#
    )
}
