use std::collections::{BTreeMap, HashSet};

use scraper::{ElementRef, Html, Selector};
use url::Url;

use crate::http::validate_https_url;
use crate::torrent::magnet;
use crate::{
    Category, Error, PublicationRef, ResolvedContent, Result, Source, Transport, Variant,
    VariantRef, VariantSelector, Work,
};

pub(crate) struct Release {
    pub(crate) work: Work,
    magnets: BTreeMap<String, Url>,
}

impl Release {
    pub(crate) fn resolve(mut self, selected: &VariantRef) -> Result<ResolvedContent> {
        let variant = self
            .work
            .variants
            .iter()
            .find(|variant| variant.reference == *selected)
            .ok_or(Error::VariantUnavailable)?;
        let VariantSelector::TorrentHash { info_hash } = &selected.selector else {
            return Err(Error::InvalidReference);
        };
        let magnet = self
            .magnets
            .remove(info_hash)
            .ok_or(Error::VariantUnavailable)?;
        Ok(ResolvedContent::Torrent {
            reference: selected.clone(),
            title: variant.title.clone().unwrap_or(self.work.title),
            magnet,
        })
    }
}

pub(crate) fn search(body: &str, origin: &Url, url: &Url, query: &str) -> Result<Vec<Work>> {
    if url.origin() != origin.origin()
        || url.path() != "/"
        || url.query_pairs().collect::<Vec<_>>() != [("s".into(), query.into())]
    {
        return Err(Error::InvalidResponse("search redirected to another query"));
    }
    let document = Html::parse_document(body);
    let root = document.root_element();
    let forms =
        Selector::parse("form.search-form[method='get']").expect("fixed search form selector");
    let mut confirmed = false;
    for form in document.select(&forms) {
        let action = origin
            .join(
                form.attr("action")
                    .ok_or(Error::InvalidResponse("missing search action"))?,
            )
            .map_err(|_| Error::InvalidResponse("invalid search form"))?;
        if action != *origin
            || !unique(form, "input[name='s']")?
                .attr("value")
                .is_some_and(|value| value.eq_ignore_ascii_case(query))
        {
            return Err(Error::InvalidResponse("search query was not confirmed"));
        }
        confirmed = true;
    }
    if !confirmed {
        return Err(Error::InvalidResponse("missing search form"));
    }

    let body = unique(root, "body")?;
    let articles = Selector::parse("#content > article").expect("fixed search article selector");
    let rows = document.select(&articles).collect::<Vec<_>>();
    if body
        .value()
        .has_class("search-no-results", scraper::CaseSensitivity::CaseSensitive)
    {
        if !rows.is_empty() || text(unique(root, "#content .page-title")?) != "Nothing Found" {
            return Err(Error::InvalidResponse("invalid empty search"));
        }
        return Ok(Vec::new());
    }
    if !body
        .value()
        .has_class("search-results", scraper::CaseSensitivity::CaseSensitive)
        || rows.is_empty()
        || rows.len() > 100
    {
        return Err(Error::InvalidResponse("missing search results"));
    }

    let mut works = Vec::new();
    let mut seen = HashSet::new();
    for row in rows {
        if !is_repack(row) {
            continue;
        }
        let title = unique(row, ".entry-title a[rel='bookmark'][href]")?;
        let (reference, url) = publication(title.attr("href").unwrap(), origin)?;
        if !seen.insert(reference.clone()) {
            return Err(Error::InvalidResponse("duplicate game publication"));
        }
        works.push(work(reference, url, title_text(title)?, true));
    }
    Ok(works)
}

pub(crate) fn release(
    body: &str,
    origin: &Url,
    url: &Url,
    expected: &PublicationRef,
) -> Result<Release> {
    let document = Html::parse_document(body);
    let root = document.root_element();
    let canonical = unique(root, "link[rel='canonical'][href]")?;
    let (reference, canonical_url) = publication(canonical.attr("href").unwrap(), origin)?;
    let (arrived, _) = publication(url.as_str(), origin)?;
    if reference != *expected || arrived != *expected {
        return Err(Error::VariantUnavailable);
    }
    let article = unique(root, "#content > article")?;
    if !is_repack(article) {
        return Err(Error::VariantUnavailable);
    }
    let mut work = work(
        reference,
        canonical_url,
        title_text(unique(article, ".entry-title")?)?,
        false,
    );
    let content = unique(article, ".entry-content")?;
    let cover =
        Selector::parse("meta[property='og:image'][content]").expect("fixed cover selector");
    if let Some(image) = document.select(&cover).next() {
        let image = origin
            .join(image.attr("content").unwrap())
            .map_err(|_| Error::InvalidResponse("invalid game cover"))?;
        validate_https_url(&image)?;
        if image.query().is_some() {
            return Err(Error::InvalidResponse("game cover contains a query"));
        }
        work.cover = Some(image);
    }

    let links = Selector::parse("a[href^='magnet:']").expect("fixed torrent selector");
    let mut magnets = BTreeMap::new();
    for (index, link) in content.select(&links).enumerate() {
        if index >= 100 {
            return Err(Error::InvalidResponse("excessive game torrents"));
        }
        let (url, hash) = magnet(link.attr("href").unwrap())?;
        if magnets.contains_key(&hash) {
            continue;
        }
        let mut names = url.query_pairs().filter(|(key, _)| key == "dn");
        let title = names.next().map(|(_, value)| value.into_owned());
        if names.next().is_some()
            || title.as_ref().is_some_and(|value| {
                value.trim().is_empty() || value.len() > 2048 || value.chars().any(char::is_control)
            })
        {
            return Err(Error::InvalidResponse("invalid torrent title"));
        }
        work.variants.push(Variant {
            reference: VariantRef {
                publication: work.reference.clone(),
                selector: VariantSelector::TorrentHash {
                    info_hash: hash.clone(),
                },
            },
            title,
            transport: Transport::Torrent,
            episode: None,
            quality: None,
            translation: None,
            languages: Vec::new(),
            player: None,
            size: None,
            seeders: None,
            info_hash: Some(hash.clone()),
            reported_video_ads: false,
        });
        magnets.insert(hash, url);
    }
    if work.variants.is_empty() {
        return Err(Error::UnsupportedAcquisition(
            "FitGirl direct mirrors and torrent-file mirrors are not supported",
        ));
    }
    Ok(Release { work, magnets })
}

fn publication(input: &str, origin: &Url) -> Result<(PublicationRef, Url)> {
    let url = origin.join(input).map_err(|_| Error::InvalidReference)?;
    validate_https_url(&url)?;
    if url.origin() != origin.origin() || url.query().is_some() {
        return Err(Error::InvalidReference);
    }
    let slug = url
        .path()
        .strip_prefix('/')
        .and_then(|path| path.strip_suffix('/'))
        .ok_or(Error::InvalidReference)?;
    let reference = PublicationRef {
        source: Source::FitGirl,
        id: slug.into(),
    };
    reference.validate()?;
    Ok((reference, url))
}

fn work(reference: PublicationRef, url: Url, title: String, needs_inspection: bool) -> Work {
    Work {
        reference,
        title,
        category: Category::Games,
        url,
        cover: None,
        description: None,
        year: None,
        aliases: Vec::new(),
        identifiers: BTreeMap::new(),
        variants: Vec::new(),
        needs_inspection,
    }
}

fn is_repack(article: ElementRef<'_>) -> bool {
    article.value().has_class(
        "category-lossless-repack",
        scraper::CaseSensitivity::CaseSensitive,
    )
}

fn title_text(element: ElementRef<'_>) -> Result<String> {
    let title = text(element);
    if title.is_empty() || title.len() > 2048 || title.chars().any(char::is_control) {
        return Err(Error::InvalidResponse("invalid publication title"));
    }
    Ok(title)
}

fn text(element: ElementRef<'_>) -> String {
    element
        .text()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn unique<'a>(root: ElementRef<'a>, selector: &str) -> Result<ElementRef<'a>> {
    let selector = Selector::parse(selector).expect("fixed metadata selector");
    let mut elements = root.select(&selector);
    let first = elements
        .next()
        .ok_or(Error::InvalidResponse("missing FitGirl metadata"))?;
    if elements.next().is_some() {
        return Err(Error::InvalidResponse("ambiguous FitGirl metadata"));
    }
    Ok(first)
}
