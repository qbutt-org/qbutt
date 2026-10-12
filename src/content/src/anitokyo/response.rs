use std::collections::{BTreeMap, HashSet};

use scraper::{ElementRef, Html, Selector};
use serde::Deserialize;
use url::Url;

use crate::http::validate_https_url;
use crate::model::canonical_decimal;
use crate::{
    Category, Error, PublicationRef, Result, Source, Transport, Variant, VariantRef,
    VariantSelector, Work,
};

pub(super) struct Publication {
    pub work: Work,
    pub videos: Vec<Video>,
}

pub(super) struct Video {
    pub selector: VariantSelector,
    pub url: Url,
    pub episode: String,
    pub kodik_voice: String,
    pub subtitled: bool,
}

#[derive(Deserialize)]
struct Group {
    id: String,
    name: String,
    items: BTreeMap<String, Item>,
}

#[derive(Deserialize)]
struct Item {
    id: String,
    parent: String,
    aname: String,
    scode: String,
}

pub(super) fn search(body: &str, origin: &Url, query: &str) -> Result<Vec<Work>> {
    let document = Html::parse_document(body);
    let echoed = one(
        document.root_element(),
        "#fullsearch #searchinput[name='story']",
    )?;
    if echoed.value().attr("value") != Some(query)
        || one(document.root_element(), "#fullsearch input[name='do']")?
            .value()
            .attr("value")
            != Some("search")
    {
        return Err(Error::InvalidResponse(
            "AniTokyo search query was not confirmed",
        ));
    }

    let rows = Selector::parse("main.main > article.story.shortstory")
        .expect("fixed AniTokyo search selector");
    let mut works = Vec::new();
    let mut seen = HashSet::new();
    for row in document.select(&rows) {
        if works.len() >= 100 {
            return Err(Error::ResponseTooLarge);
        }
        let work = work(row, origin, None)?;
        if !seen.insert(work.reference.clone()) {
            return Err(Error::InvalidResponse("duplicate AniTokyo publication"));
        }
        works.push(work);
    }
    Ok(works)
}

pub(super) fn publication(
    body: &str,
    origin: &Url,
    final_url: &Url,
    expected: &PublicationRef,
) -> Result<Publication> {
    expected.anitokyo_id()?;
    let document = Html::parse_document(body);
    let canonical = one(document.root_element(), "link[rel='canonical'][href]")?;
    let canonical = site_url(
        canonical
            .value()
            .attr("href")
            .expect("selected canonical href"),
        origin,
    )?;
    if &publication_ref(&canonical)? != expected || canonical != *final_url {
        return Err(Error::VariantUnavailable);
    }

    let article = one(
        document.root_element(),
        "main.main > article.story.fullstory",
    )?;
    let mut work = work(article, origin, Some(&canonical))?;

    let Some(script) = optional(article, ".story-player script:not([src])")? else {
        let title = text(one(article, "h1.story-title")?);
        if no_released_episode(&title) {
            return Ok(Publication {
                work,
                videos: Vec::new(),
            });
        }
        return Err(Error::InvalidResponse("missing AniTokyo episode manifest"));
    };
    let script = script.text().collect::<String>();
    let marker = "RalodePlayer.init(";
    let mut calls = script.match_indices(marker);
    let (call, _) = calls
        .next()
        .ok_or(Error::InvalidResponse("missing AniTokyo episode manifest"))?;
    if calls.next().is_some() {
        return Err(Error::InvalidResponse(
            "ambiguous AniTokyo episode manifest",
        ));
    }
    let json = &script[call + marker.len()..];
    let mut deserializer = serde_json::Deserializer::from_str(json);
    let groups = BTreeMap::<String, Group>::deserialize(&mut deserializer).map_err(Error::Json)?;
    if groups.len() > 200 {
        return Err(Error::ResponseTooLarge);
    }

    let mut videos = Vec::new();
    let mut seen = HashSet::new();
    for (key, group) in groups {
        let group_id = decimal_id(&group.id)?;
        if key != group.id || group.name.trim().is_empty() || group.name.len() > 512 {
            return Err(Error::InvalidResponse("invalid AniTokyo dubbing identity"));
        }
        let subtitled = group.name.starts_with("Субтитры");
        let kodik_voice = if subtitled {
            let voice = group
                .name
                .strip_prefix("Субтитры „")
                .and_then(|voice| voice.strip_suffix('“'))
                .unwrap_or(&group.name);
            voice.strip_suffix(".Subtitles").unwrap_or(voice)
        } else {
            &group.name
        };
        if group.items.len() > 5000 || kodik_voice.trim().is_empty() {
            return Err(Error::ResponseTooLarge);
        }

        for (key, item) in group.items {
            let id = decimal_id(&item.id)?;
            if key != item.id || item.parent != group.id || !seen.insert(id) {
                return Err(Error::InvalidResponse("invalid AniTokyo video identity"));
            }
            let Some(episode) = item
                .aname
                .strip_suffix(" серия")
                .filter(|episode| canonical_decimal(episode))
            else {
                // Kodik cannot verify a nonnumeric episode against its player.
                continue;
            };
            let url = video_url(&item.scode, origin, id)?;
            let selector = VariantSelector::AniTokyoVideo {
                id,
                group_id,
                episode: episode.to_owned(),
                dubbing: group.name.clone(),
            };
            work.variants.push(Variant {
                reference: VariantRef {
                    publication: expected.clone(),
                    selector: selector.clone(),
                },
                title: None,
                transport: Transport::Video,
                episode: Some(episode.to_owned()),
                quality: None,
                translation: Some(group.name.clone()),
                languages: if subtitled {
                    vec!["ja".into(), "ru".into()]
                } else {
                    vec!["ru".into()]
                },
                player: None,
                size: None,
                seeders: None,
                info_hash: None,
                reported_video_ads: false,
            });
            videos.push(Video {
                selector,
                url,
                episode: episode.to_owned(),
                kodik_voice: kodik_voice.to_owned(),
                subtitled,
            });
        }
    }
    Ok(Publication { work, videos })
}

pub(super) fn player(body: &str, origin: &Url) -> Result<Url> {
    let document = Html::parse_document(body);
    let iframe = one(document.root_element(), "body > iframe.vdd-element[src]")?;
    let raw = iframe.value().attr("src").expect("selected player src");
    let url = origin.join(raw).map_err(|_| Error::UnsafeAddress)?;
    validate_https_url(&url)?;
    Ok(url)
}

fn work(article: ElementRef<'_>, origin: &Url, canonical: Option<&Url>) -> Result<Work> {
    let needs_inspection = canonical.is_none();
    let title_link = if needs_inspection {
        one(article, ".story-title a[href]")?
    } else {
        one(article, "h1.story-title")?
    };
    let raw_title = text(title_link);
    let raw_title = episode_suffix(&raw_title);
    let (title, other_title) = raw_title
        .split_once(" / ")
        .map(|(main, other)| (main, Some(other)))
        .unwrap_or((raw_title, None));
    if title.is_empty() || title.len() > 2048 {
        return Err(Error::InvalidResponse("invalid AniTokyo title"));
    }
    let url = match canonical {
        Some(canonical) => canonical.clone(),
        None => site_url(
            title_link
                .value()
                .attr("href")
                .expect("selected title href"),
            origin,
        )?,
    };
    let reference = publication_ref(&url)?;

    let poster = optional(
        article,
        ".story-poster img.poster[src], .story-poster-list img.poster[src]",
    )?;
    let cover = poster
        .map(|image| {
            let cover = origin
                .join(image.value().attr("src").expect("selected poster src"))
                .map_err(|_| Error::UnsafeAddress)?;
            validate_https_url(&cover)?;
            if cover.host_str() != Some("atw.picmap.top") || cover.query().is_some() {
                return Err(Error::UnsafeAddress);
            }
            Ok(cover)
        })
        .transpose()?;
    let description = optional(article, ".story-description")?
        .map(text)
        .map(|description| {
            description
                .strip_prefix("Описание:")
                .unwrap_or(&description)
                .trim()
                .to_owned()
        })
        .filter(|description| !description.is_empty());
    if description
        .as_ref()
        .is_some_and(|description| description.len() > 100_000)
    {
        return Err(Error::ResponseTooLarge);
    }
    let year = optional(article, ".story-info a[href^='/year/']")?
        .and_then(|link| text(link).parse::<u16>().ok());
    let mut aliases = Vec::new();
    if let Some(other) = other_title.filter(|other| !other.is_empty()) {
        aliases.push(other.to_owned());
    }
    if let Some(other) = optional(article, ".item-alt-title b")?.map(text) {
        if !other.is_empty() && other != title && !aliases.contains(&other) {
            aliases.push(other);
        }
    }

    Ok(Work {
        reference,
        title: title.to_owned(),
        category: Category::Anime,
        url,
        cover,
        description,
        year,
        aliases,
        identifiers: BTreeMap::new(),
        variants: Vec::new(),
        needs_inspection,
    })
}

fn episode_suffix(title: &str) -> &str {
    let Some((name, count)) = title.rsplit_once(" (") else {
        return title;
    };
    let Some((current, total)) = count
        .strip_suffix(')')
        .and_then(|count| count.split_once(" из "))
    else {
        return title;
    };
    if (current.is_empty() || canonical_decimal(current))
        && (canonical_decimal(total) || total == "??")
    {
        name
    } else {
        title
    }
}

fn no_released_episode(title: &str) -> bool {
    title
        .rsplit_once(" ( из ")
        .and_then(|(_, total)| total.strip_suffix(')'))
        .is_some_and(canonical_decimal)
}

fn video_url(scode: &str, origin: &Url, id: u64) -> Result<Url> {
    let document = Html::parse_fragment(scode);
    let iframe = one(document.root_element(), "iframe[src]")?;
    let url = origin
        .join(iframe.value().attr("src").expect("selected video src"))
        .map_err(|_| Error::UnsafeAddress)?;
    if url.origin() != origin.origin()
        || url.path() != "/video.php"
        || url.fragment().is_some()
        || url.query_pairs().collect::<Vec<_>>()
            != [
                ("id".into(), id.to_string().into()),
                ("cat".into(), "1,5".into()),
            ]
    {
        return Err(Error::InvalidResponse("invalid AniTokyo video address"));
    }
    Ok(url)
}

fn publication_ref(url: &Url) -> Result<PublicationRef> {
    let path = url.path().trim_start_matches('/');
    let (section, name) = path
        .split_once('/')
        .ok_or(Error::InvalidResponse("invalid AniTokyo publication path"))?;
    if !matches!(section, "anime" | "movie" | "ova" | "ona" | "special") || !name.ends_with(".html")
    {
        return Err(Error::InvalidResponse("unsupported AniTokyo publication"));
    }
    let (id, slug) =
        name.trim_end_matches(".html")
            .split_once('-')
            .ok_or(Error::InvalidResponse(
                "invalid AniTokyo publication identity",
            ))?;
    if !canonical_decimal(id)
        || slug.is_empty()
        || slug.len() > 512
        || !slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(Error::InvalidResponse(
            "invalid AniTokyo publication identity",
        ));
    }
    Ok(PublicationRef {
        source: Source::AniTokyo,
        id: id.to_owned(),
    })
}

fn site_url(raw: &str, origin: &Url) -> Result<Url> {
    let url = origin.join(raw).map_err(|_| Error::UnsafeAddress)?;
    validate_https_url(&url)?;
    if url.origin() != origin.origin() || url.query().is_some() || url.fragment().is_some() {
        return Err(Error::UnsafeAddress);
    }
    Ok(url)
}

fn decimal_id(value: &str) -> Result<u64> {
    if !canonical_decimal(value) {
        return Err(Error::InvalidResponse("invalid AniTokyo decimal identity"));
    }
    value
        .parse()
        .map_err(|_| Error::InvalidResponse("invalid AniTokyo decimal identity"))
}

fn one<'a>(root: ElementRef<'a>, selector: &str) -> Result<ElementRef<'a>> {
    optional(root, selector)?.ok_or(Error::InvalidResponse("missing AniTokyo element"))
}

fn optional<'a>(root: ElementRef<'a>, selector: &str) -> Result<Option<ElementRef<'a>>> {
    let selector = Selector::parse(selector).expect("fixed AniTokyo selector");
    let mut matches = root.select(&selector);
    let found = matches.next();
    if matches.next().is_some() {
        return Err(Error::InvalidResponse("ambiguous AniTokyo element"));
    }
    Ok(found)
}

fn text(element: ElementRef<'_>) -> String {
    element
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
