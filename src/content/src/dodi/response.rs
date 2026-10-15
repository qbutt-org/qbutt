use std::collections::{BTreeMap, HashSet};

use scraper::{ElementRef, Html, Selector};
use url::Url;

use crate::http::{validate_https_url, validate_media_url};
use crate::{
    Category, Error, PublicationRef, Result, Source, Transport, Variant, VariantRef,
    VariantSelector, Work,
};

pub(crate) fn search(body: &str, origin: &Url, query: &str) -> Result<Vec<Work>> {
    let document = Html::parse_document(body);
    let body_selector = Selector::parse("body.search").expect("fixed search body selector");
    let page = document
        .select(&body_selector)
        .next()
        .ok_or(Error::InvalidResponse("missing DODI search page"))?;
    let input_selector = Selector::parse("input[name='s']").expect("fixed search input selector");
    let mut inputs = page.select(&input_selector).peekable();
    if inputs.peek().is_none() || inputs.any(|input| input.attr("value") != Some(query)) {
        return Err(Error::InvalidResponse(
            "DODI search query was not confirmed",
        ));
    }
    let main_selector = Selector::parse("main#main").expect("fixed results selector");
    let main = single(
        page.select(&main_selector),
        "missing or ambiguous DODI results",
    )?;
    let article_selector = Selector::parse("article.post").expect("fixed article selector");
    let title_selector = Selector::parse(".entry-title a[rel='bookmark']")
        .expect("fixed publication title selector");
    let bookmark_selector =
        Selector::parse(".entry-meta a[rel='bookmark']").expect("fixed post bookmark selector");
    let empty = page
        .value()
        .classes()
        .any(|class| class == "search-no-results");
    let mut works = Vec::new();
    let mut seen_posts = HashSet::new();
    for article in main.select(&article_selector) {
        if works.len() >= 100 || empty {
            return Err(Error::InvalidResponse("unexpected DODI search rows"));
        }
        let mut titles = article.select(&title_selector);
        let Some(title) = titles.next() else {
            let bookmark = single(
                article.select(&bookmark_selector),
                "missing DODI search title",
            )?;
            let url = publication_url(bookmark.attr("href").unwrap_or_default(), origin)?;
            if url.path() == "/upcoming-repacks/" {
                continue;
            }
            return Err(Error::InvalidResponse("missing DODI search title"));
        };
        if titles.next().is_some() {
            return Err(Error::InvalidResponse("ambiguous DODI search title"));
        }
        let url = publication_url(title.attr("href").unwrap_or_default(), origin)?;
        let reference = publication(article, &url)?;
        if !seen_posts.insert(reference.dodi_parts()?.0) {
            return Err(Error::InvalidResponse("duplicate DODI publication"));
        }
        works.push(work(reference, text(title), url, true)?);
    }
    if works.is_empty() && !empty {
        return Err(Error::InvalidResponse("missing DODI search rows"));
    }
    Ok(works)
}

pub(crate) fn release(body: &str, origin: &Url, expected: &PublicationRef) -> Result<Work> {
    expected.dodi_parts()?;
    let document = Html::parse_document(body);
    let canonical_selector =
        Selector::parse("link[rel='canonical']").expect("fixed canonical selector");
    let canonical = single(
        document.select(&canonical_selector),
        "missing or ambiguous DODI canonical URL",
    )?;
    let url = publication_url(canonical.attr("href").unwrap_or_default(), origin)?;
    let article_selector =
        Selector::parse("main#main article.post").expect("fixed article selector");
    let article = single(
        document.select(&article_selector),
        "missing or ambiguous DODI publication",
    )?;
    if publication(article, &url)? != *expected {
        return Err(Error::VariantUnavailable);
    }
    let title_selector = Selector::parse("h1.entry-title").expect("fixed title selector");
    let title = single(
        article.select(&title_selector),
        "invalid DODI publication title",
    )?;
    let mut work = work(expected.clone(), text(title), url, false)?;
    let content_selector = Selector::parse(".entry-content").expect("fixed content selector");
    let content = single(
        article.select(&content_selector),
        "invalid DODI publication content",
    )?;
    let link_selector = Selector::parse("a[href]").expect("fixed download selector");
    let mut downloads = false;
    let mut seen = HashSet::new();
    for block in content.child_elements() {
        let label = text(block);
        if label.eq_ignore_ascii_case("Download Links") {
            downloads = true;
            continue;
        }
        if !downloads {
            continue;
        }
        if !label.starts_with("Torrent ") && label != "Torrent" {
            continue;
        }
        for link in block.select(&link_selector) {
            let Some(id) = torrent_id(link.attr("href").unwrap())? else {
                continue;
            };
            let reference = VariantRef {
                publication: expected.clone(),
                selector: VariantSelector::DodiTorrent { id },
            };
            reference.validate()?;
            if !seen.insert(reference.clone()) {
                continue;
            }
            if seen.len() > 100 || label.len() > 2048 {
                return Err(Error::InvalidResponse("excessive DODI torrent variants"));
            }
            work.variants.push(Variant {
                reference,
                title: Some("Torrent [file-me.top]".into()),
                transport: Transport::Torrent,
                episode: None,
                quality: None,
                translation: None,
                languages: Vec::new(),
                player: None,
                size: None,
                seeders: None,
                info_hash: None,
                reported_video_ads: false,
            });
        }
    }
    if !downloads {
        return Err(Error::InvalidResponse("missing DODI download section"));
    }
    if work.variants.is_empty() {
        return Err(Error::UnsupportedAcquisition(
            "This DODI publication has no supported FileMe torrent",
        ));
    }
    Ok(work)
}

fn torrent_id(input: &str) -> Result<Option<String>> {
    let url = Url::parse(input).map_err(|_| Error::UnsafeAddress)?;
    if url.host_str() != Some("file-me.top") {
        return Ok(None);
    }
    validate_media_url(&url)?;
    if url.query().is_some() || url.port().is_some() {
        return Err(Error::UnsafeAddress);
    }
    let id = url
        .path()
        .strip_prefix('/')
        .and_then(|path| path.strip_suffix(".html"))
        .ok_or(Error::InvalidReference)?;
    Ok(Some(id.into()))
}

fn publication_url(input: &str, origin: &Url) -> Result<Url> {
    let url = origin.join(input).map_err(|_| Error::UnsafeAddress)?;
    validate_https_url(&url)?;
    if url.origin() != origin.origin() || url.query().is_some() {
        return Err(Error::UnsafeAddress);
    }
    let slug = url
        .path()
        .strip_prefix('/')
        .and_then(|path| path.strip_suffix('/'));
    if !slug.is_some_and(|slug| !slug.is_empty() && !slug.contains('/')) {
        return Err(Error::InvalidResponse("invalid DODI publication address"));
    }
    Ok(url)
}

fn publication(article: ElementRef<'_>, url: &Url) -> Result<PublicationRef> {
    let id = article
        .attr("id")
        .and_then(|id| id.strip_prefix("post-"))
        .ok_or(Error::InvalidResponse("missing DODI post identity"))?;
    let reference = PublicationRef {
        source: Source::Dodi,
        id: format!("{id}--{}", url.path().trim_matches('/')),
    };
    reference
        .validate()
        .map_err(|_| Error::InvalidResponse("invalid DODI publication address"))?;
    Ok(reference)
}

fn work(
    reference: PublicationRef,
    title: String,
    url: Url,
    needs_inspection: bool,
) -> Result<Work> {
    if title.is_empty() || title.len() > 2048 {
        return Err(Error::InvalidResponse("invalid DODI publication title"));
    }
    let aliases = title
        .split_once("- ")
        .filter(|(number, name)| {
            !number.is_empty()
                && number.bytes().all(|byte| byte.is_ascii_digit())
                && !name.is_empty()
        })
        .map(|(_, name)| vec![name.to_owned()])
        .unwrap_or_default();
    Ok(Work {
        reference,
        title,
        category: Category::Games,
        url,
        cover: None,
        description: None,
        year: None,
        aliases,
        identifiers: BTreeMap::new(),
        variants: Vec::new(),
        notes: Vec::new(),
        needs_inspection,
    })
}

fn single<'a>(
    mut elements: impl Iterator<Item = ElementRef<'a>>,
    message: &'static str,
) -> Result<ElementRef<'a>> {
    let element = elements.next().ok_or(Error::InvalidResponse(message))?;
    if elements.next().is_some() {
        return Err(Error::InvalidResponse(message));
    }
    Ok(element)
}

fn text(element: ElementRef<'_>) -> String {
    element
        .text()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
