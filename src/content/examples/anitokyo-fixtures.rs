//! Generated legal AniTokyo-shaped HTML through the production parser and models.

#[path = "../src/model.rs"]
mod model;
pub use model::*;
#[path = "../src/error.rs"]
mod error;
pub use error::{Error, Result};
#[cfg(windows)]
pub use qbutt_content::webview;

#[cfg(windows)]
#[allow(dead_code)]
#[path = "../src/browser/mod.rs"]
mod browser;

#[allow(dead_code)]
mod http {
    include!("../src/http.rs");

    pub(super) fn fixture(client: reqwest::Client, origin: Url) -> SourceHttp {
        SourceHttp { client, origin }
    }
}

#[path = "anitokyo/fixture.rs"]
mod fixture;
#[path = "../src/anitokyo/response.rs"]
mod response;

use url::Url;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let origin = Url::parse("https://anitokyo.tv/")?;
    let canonical = origin.join("anime/42-generated-commons.html")?;
    let reference = PublicationRef {
        source: Source::AniTokyo,
        id: "42".into(),
    };
    let cover = Url::parse("https://atw.picmap.top/generated-primary.jpg")?;
    let search = format!(
        r#"<form id='fullsearch'>
                <input name='do' value='search'>
                <input id='searchinput' name='story' value='commons'>
            </form>
            <main class='main'>
                <article class='story shortstory'>
                    <h2 class='story-title'>
                        <a href='{canonical}'>Generated Commons / Open Demo (2 из 4)</a>
                    </h2>
                    <div class='story-poster'><a><img class='poster' src='{cover}'></a></div>
                </article>
            </main>"#
    );
    let results = response::search(&search, &origin, "commons")?;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Generated Commons");
    assert_eq!(results[0].aliases, ["Open Demo"]);
    assert_eq!(results[0].reference, reference);
    assert_eq!(results[0].cover.as_ref(), Some(&cover));
    assert!(results[0].needs_inspection && results[0].variants.is_empty());
    assert!(response::search(&search, &origin, "changed").is_err());
    assert!(response::search(
        "<form id='fullsearch'><input name='do' value='search'><input id='searchinput' name='story' value='empty'></form><main class='main'></main>",
        &origin,
        "empty",
    )?
    .is_empty());

    let groups = serde_json::json!({
        "100": {
            "id": "100",
            "name": "Demo Studio",
            "items": {
                "500": {
                    "id": "500",
                    "parent": "100",
                    "aname": "1 серия",
                    "scode": "<iframe src=\"/video.php?id=500&cat=1,5\"></iframe>"
                }
            }
        },
        "101": {
            "id": "101",
            "name": "Субтитры „Open.Subtitles“",
            "items": {
                "501": {
                    "id": "501",
                    "parent": "101",
                    "aname": "2 серия",
                    "scode": "<iframe src=\"/video.php?id=501&cat=1,5\"></iframe>"
                }
            }
        }
    });
    let page = |groups: &serde_json::Value| {
        format!(
            r#"<link rel='canonical' href='{canonical}'>
                <main class='main'>
                    <article class='story fullstory'>
                        <h1 class='story-title'>Generated Commons / Open Demo (2 из 4)</h1>
                        <div class='story-poster-list'>
                            <a><img class='poster' src='{cover}'></a>
                        </div>
                        <section class='story-player'>
                            <script>RalodePlayer.init({groups},{{}},[]);</script>
                        </section>
                        <article class='related-item'>
                            <div class='related-poster'>
                                <a><img class='poster' src='https://atw.picmap.top/other-work.jpg'></a>
                            </div>
                        </article>
                    </article>
                </main>"#
        )
    };
    let publication = response::publication(&page(&groups), &origin, &canonical, &reference)?;
    assert_eq!(publication.work.cover.as_ref(), Some(&cover));
    assert_eq!(publication.work.variants.len(), 2);
    assert_eq!(publication.videos.len(), 2);
    assert_eq!(publication.videos[0].url.path(), "/video.php");
    assert_eq!(publication.videos[0].episode, "1");
    assert_eq!(
        publication.videos[0].selector,
        publication.work.variants[0].reference.selector
    );
    assert_eq!(publication.videos[1].kodik_voice, "Open");
    assert!(publication.videos[1].subtitled);
    assert_eq!(publication.work.variants[1].languages, ["ja", "ru"]);
    for variant in &publication.work.variants {
        variant.reference.validate()?;
    }

    let upcoming = format!(
        "<link rel='canonical' href='{canonical}'><main class='main'><article class='story fullstory'><h1 class='story-title'>Generated Commons / Open Demo ( из 4)</h1></article></main>"
    );
    let upcoming = response::publication(&upcoming, &origin, &canonical, &reference)?;
    assert!(upcoming.work.variants.is_empty());

    let changed = groups
        .to_string()
        .replace("\"parent\":\"100\"", "\"parent\":\"999\"");
    let changed: serde_json::Value = serde_json::from_str(&changed)?;
    assert!(response::publication(&page(&changed), &origin, &canonical, &reference).is_err());
    let wrong = origin.join("anime/43-other-work.html")?;
    assert!(response::publication(&page(&groups), &origin, &wrong, &reference).is_err());

    let original = origin.join("ONA/42-generated-commons.html")?;
    let original_page = page(&groups).replace(canonical.as_str(), original.as_str());
    assert_eq!(
        response::publication(&original_page, &origin, &original, &reference)?
            .work
            .url,
        original
    );

    let player = response::player(
        "<html><body><iframe class='vdd-element' src='//kodikplayer.com/seria/10/0123456789abcdef0123456789abcdef/720p'></iframe></body></html>",
        &origin,
    )?;
    assert_eq!(player.host_str(), Some("kodikplayer.com"));
    assert!(response::player(
        "<html><body><iframe class='vdd-element' src='http://127.0.0.1/video'></iframe></body></html>",
        &origin,
    )
    .is_err());

    fixture::run().await?;

    println!(
        "PASS anitokyo-fixtures search=exact uppercase_ona=preserved inspect=exact voice_episode=exact upcoming=empty player=guarded https=positive_empty_faults"
    );
    Ok(())
}
