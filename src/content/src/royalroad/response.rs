use std::collections::{BTreeMap, HashSet};

use scraper::{ElementRef, Html, Selector};
use serde::Deserialize;
use url::Url;

use crate::http::validate_https_url;
use crate::model::canonical_decimal;
use crate::{
    Category, Chapter, ChapterRef, Error, PublicationRef, Result, Source, TextChapter, Transport,
    Variant, VariantRef, VariantSelector, Work,
};

pub(crate) struct Book {
    pub(crate) work: Work,
    pub(crate) chapters: Vec<Entry>,
}

pub(crate) struct Entry {
    pub(crate) chapter: Chapter,
    pub(crate) url: Url,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BookMetadata {
    #[serde(rename = "@type")]
    kind: String,
    name: String,
    description: String,
    image: Option<String>,
    url: String,
    in_language: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChapterMetadata {
    id: u64,
    title: String,
    order: u64,
    visible: u8,
    subscription_tiers: serde_json::Value,
    url: String,
}

pub(crate) fn is_challenge(body: &str, status: u16) -> bool {
    if body.contains("_cf_chl_opt")
        || (matches!(status, 403 | 503) && body.contains("/cdn-cgi/challenge-platform/"))
    {
        return true;
    }
    let document = Html::parse_document(body);
    let challenge =
        Selector::parse("#challenge-form, #cf-challenge-running, #challenge-stage, .cf-turnstile")
            .expect("fixed Cloudflare challenge selector");
    document.select(&challenge).next().is_some()
}

pub(crate) fn search(body: &str, origin: &Url) -> Result<Vec<Work>> {
    let document = Html::parse_document(body);
    let results = one(&document, ".fiction-list")?;
    let rows = Selector::parse(".fiction-list-item").expect("fixed fiction row selector");
    let title = Selector::parse(".fiction-title a[href]").expect("fixed fiction title selector");
    let cover = Selector::parse("img[data-type='cover']").expect("fixed cover selector");
    let mut works = Vec::new();
    let mut seen = HashSet::new();
    for row in results.select(&rows) {
        if works.len() >= 100 {
            return Err(Error::ResponseTooLarge);
        }
        let mut titles = row.select(&title);
        let title = titles
            .next()
            .ok_or(Error::InvalidResponse("missing fiction title"))?;
        if titles.next().is_some() {
            return Err(Error::InvalidResponse("ambiguous fiction title"));
        }
        let url = site_url(title.attr("href").unwrap(), origin)?;
        let reference = publication(&url)?;
        if !seen.insert(reference.clone()) {
            return Err(Error::InvalidResponse("duplicate fiction"));
        }
        let mut work = work(reference, text(title), url, true)?;
        work.cover = row
            .select(&cover)
            .next()
            .and_then(|image| image.attr("src"))
            .filter(|value| *value != "/dist/img/nocover-new-min.png")
            .map(cover_url)
            .transpose()?;
        works.push(work);
    }
    Ok(works)
}

pub(crate) fn book(body: &str, origin: &Url, expected: &PublicationRef) -> Result<Book> {
    expected.royalroad_id()?;
    let document = Html::parse_document(body);
    let canonical = canonical(&document, origin)?;
    if publication(&canonical)? != *expected {
        return Err(Error::VariantUnavailable);
    }
    let metadata: BookMetadata = serde_json::from_str(
        &one(&document, "script[type='application/ld+json']")?
            .text()
            .collect::<String>(),
    )
    .map_err(Error::Json)?;
    if metadata.kind != "Book" || publication(&site_url(&metadata.url, origin)?)? != *expected {
        return Err(Error::InvalidResponse("invalid fiction metadata identity"));
    }
    // Search does not report language. Admit the exact original only after the
    // publication's own structured metadata confirms English.
    if !matches!(metadata.in_language.as_str(), "en" | "en-US" | "en-GB") {
        return Err(Error::VariantUnavailable);
    }
    let mut work = work(expected.clone(), metadata.name, canonical, false)?;
    let description = Html::parse_fragment(&metadata.description);
    let description = text(description.root_element());
    if description.len() > 100_000 {
        return Err(Error::ResponseTooLarge);
    }
    work.description = (!description.is_empty()).then_some(description);
    work.cover = metadata.image.as_deref().map(cover_url).transpose()?;
    work.variants[0].languages.push("en".into());

    let scripts = Selector::parse("script").expect("fixed scripts selector");
    let mut entries = None;
    for script in document.select(&scripts) {
        for line in script.text().collect::<String>().lines() {
            let Some(value) = line.trim().strip_prefix("window.chapters") else {
                continue;
            };
            let value = value
                .trim_start()
                .strip_prefix('=')
                .and_then(|value| value.trim().strip_suffix(';'))
                .ok_or(Error::InvalidResponse("invalid chapter manifest"))?;
            let parsed: Vec<ChapterMetadata> = serde_json::from_str(value).map_err(Error::Json)?;
            if entries.replace(parsed).is_some() {
                return Err(Error::InvalidResponse("ambiguous chapter manifest"));
            }
        }
    }
    let entries = entries.ok_or(Error::InvalidResponse("missing chapter manifest"))?;
    if entries.len() > 20_000 {
        return Err(Error::ResponseTooLarge);
    }
    let mut seen = HashSet::new();
    let mut chapters = Vec::new();
    for entry in entries {
        let url = site_url(&entry.url, origin)?;
        let reference = chapter_reference(&url)?;
        if reference.id != format!("{}/{}", expected.id, entry.id)
            || !seen.insert(entry.id)
            || entry.visible > 1
        {
            return Err(Error::InvalidResponse("invalid chapter manifest identity"));
        }
        let paid = match entry.subscription_tiers {
            serde_json::Value::Null => false,
            serde_json::Value::Array(tiers) => !tiers.is_empty(),
            _ => {
                return Err(Error::InvalidResponse(
                    "invalid chapter subscription metadata",
                ));
            }
        };
        if entry.visible == 0 || paid {
            continue;
        }
        let title = label(entry.title)?;
        chapters.push((
            entry.order,
            Entry {
                chapter: Chapter {
                    reference,
                    title: Some(title),
                    volume: None,
                    number: None,
                    pages: 0,
                },
                url,
            },
        ));
    }
    chapters.sort_by_key(|(order, _)| *order);
    Ok(Book {
        work,
        chapters: chapters.into_iter().map(|(_, entry)| entry).collect(),
    })
}

pub(crate) fn chapter(body: &str, origin: &Url, expected: &ChapterRef) -> Result<TextChapter> {
    expected.validate()?;
    let mut document = Html::parse_document(body);
    if chapter_reference(&canonical(&document, origin)?)? != *expected {
        return Err(Error::VariantUnavailable);
    }
    let content = one(&document, "div.chapter-inner.chapter-content")?.id();
    // Royal Road marks invisible noise with page-local CSS classes. Apply only
    // its simple class rules; no scripts or remote stylesheets are evaluated.
    let styles = Selector::parse("style").expect("fixed style selector");
    let mut hidden = HashSet::new();
    for style in document.select(&styles) {
        for rule in style.text().collect::<String>().split('}') {
            let Some((selector, declarations)) = rule.split_once('{') else {
                continue;
            };
            if !declarations.split(';').any(|declaration| {
                declaration.split_once(':').is_some_and(|(name, value)| {
                    name.trim().eq_ignore_ascii_case("display")
                        && value.trim().eq_ignore_ascii_case("none")
                })
            }) {
                continue;
            }
            let Some(class) = selector.trim().strip_prefix('.') else {
                continue;
            };
            if !class.is_empty()
                && class
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            {
                hidden.insert(class.to_owned());
            }
        }
    }
    let content = ElementRef::wrap(document.tree.get(content).unwrap()).unwrap();
    let removed: Vec<_> = content
        .descendants()
        .filter_map(ElementRef::wrap)
        .filter(|element| {
            element
                .value()
                .classes()
                .any(|class| hidden.contains(class))
        })
        .map(|element| element.id())
        .collect();
    for id in removed {
        document.tree.get_mut(id).unwrap().detach();
    }
    TextChapter::from_html(
        expected.clone(),
        &one(&document, "div.chapter-inner.chapter-content")?.inner_html(),
    )
}

fn one<'a>(document: &'a Html, selector: &str) -> Result<ElementRef<'a>> {
    let selector = Selector::parse(selector).expect("fixed Royal Road selector");
    let mut elements = document.select(&selector);
    let element = elements
        .next()
        .ok_or(Error::InvalidResponse("missing Royal Road page element"))?;
    if elements.next().is_some() {
        return Err(Error::InvalidResponse("ambiguous Royal Road page element"));
    }
    Ok(element)
}

fn canonical(document: &Html, origin: &Url) -> Result<Url> {
    let value = one(document, "link[rel='canonical'][href]")?
        .attr("href")
        .unwrap();
    site_url(value, origin)
}

fn site_url(value: &str, origin: &Url) -> Result<Url> {
    let url = origin.join(value).map_err(|_| Error::UnsafeAddress)?;
    validate_https_url(&url)?;
    if url.origin() != origin.origin() || url.query().is_some() || url.fragment().is_some() {
        return Err(Error::UnsafeAddress);
    }
    Ok(url)
}

fn publication(url: &Url) -> Result<PublicationRef> {
    let parts: Vec<_> = url
        .path_segments()
        .ok_or(Error::InvalidReference)?
        .collect();
    if parts.len() != 3
        || parts[0] != "fiction"
        || !canonical_decimal(parts[1])
        || parts[2].is_empty()
    {
        return Err(Error::InvalidResponse("invalid fiction URL"));
    }
    Ok(PublicationRef {
        source: Source::RoyalRoad,
        id: parts[1].into(),
    })
}

fn chapter_reference(url: &Url) -> Result<ChapterRef> {
    let parts: Vec<_> = url
        .path_segments()
        .ok_or(Error::InvalidReference)?
        .collect();
    if parts.len() != 6
        || parts[0] != "fiction"
        || !canonical_decimal(parts[1])
        || parts[2].is_empty()
        || parts[3] != "chapter"
        || !canonical_decimal(parts[4])
        || parts[5].is_empty()
    {
        return Err(Error::InvalidResponse("invalid chapter URL"));
    }
    Ok(ChapterRef {
        variant: VariantRef {
            publication: PublicationRef {
                source: Source::RoyalRoad,
                id: parts[1].into(),
            },
            selector: VariantSelector::OriginalText,
        },
        id: format!("{}/{}", parts[1], parts[4]),
    })
}

fn cover_url(value: &str) -> Result<Url> {
    let mut url = Url::parse(value).map_err(|_| Error::UnsafeAddress)?;
    validate_https_url(&url)?;
    if url.host_str() != Some("www.royalroadcdn.com")
        || url.port().is_some()
        || url.fragment().is_some()
        || !url.path().starts_with("/public/covers-")
    {
        return Err(Error::UnsafeAddress);
    }
    url.set_query(None);
    Ok(url)
}

fn work(
    reference: PublicationRef,
    title: String,
    url: Url,
    needs_inspection: bool,
) -> Result<Work> {
    let title = label(title)?;
    let variant = Variant {
        reference: VariantRef {
            publication: reference.clone(),
            selector: VariantSelector::OriginalText,
        },
        title: None,
        transport: Transport::Text,
        episode: None,
        quality: None,
        translation: None,
        languages: Vec::new(),
        player: None,
        size: None,
        seeders: None,
        info_hash: None,
        reported_video_ads: false,
    };
    Ok(Work {
        reference,
        title,
        category: Category::Books,
        url,
        cover: None,
        description: None,
        year: None,
        aliases: Vec::new(),
        identifiers: BTreeMap::new(),
        variants: vec![variant],
        notes: Vec::new(),
        needs_inspection,
    })
}

fn label(value: String) -> Result<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 2048 || value.chars().any(char::is_control) {
        return Err(Error::InvalidResponse("invalid Royal Road title"));
    }
    Ok(value.into())
}

fn text(element: ElementRef<'_>) -> String {
    element
        .text()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
