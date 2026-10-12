use std::collections::{BTreeMap, HashSet};

use scraper::{ElementRef, Html, Selector};
use url::Url;

use crate::model::canonical_decimal;
use crate::torrent::magnet;
use crate::{
    Category, Error, PublicationRef, ResolvedContent, Result, Source, Transport, Variant,
    VariantRef, VariantSelector, Work,
};

pub(super) struct Forums {
    movies: HashSet<u64>,
    software: HashSet<u64>,
}

impl Forums {
    pub(super) fn parse(body: &str) -> Result<Self> {
        let document = Html::parse_document(body);
        let form = unique(document.root_element(), "form#search_form")?;
        let list = unique(form, "select#fs[name='f[]']")?;
        let mut movies = HashSet::new();
        let mut software = HashSet::new();
        let mut movie_boundary = false;
        for group in list.select(&selector("optgroup[label]")) {
            let (destination, films) = match required(group, "label")?.trim() {
                "Трекер: Видео. Кино, Театр, Муз.видео" => {
                    (&mut movies, true)
                }
                "Трекер: Программы, Операционные системы" => {
                    (&mut software, false)
                }
                _ => continue,
            };
            for option in group.select(&selector("option[value]")) {
                if films && text(option) == "Театр и Музыкальное видео" {
                    movie_boundary = true;
                    break;
                }
                let id = decimal(required(option, "value")?)?;
                if !destination.insert(id) {
                    return Err(Error::InvalidResponse("duplicate NNM-Club forum"));
                }
            }
        }
        if movies.is_empty()
            || software.is_empty()
            || !movie_boundary
            || !movies.is_disjoint(&software)
        {
            return Err(Error::InvalidResponse("missing NNM-Club catalog forums"));
        }
        Ok(Self { movies, software })
    }

    pub(super) fn for_category(&self, category: Category) -> &HashSet<u64> {
        match category {
            Category::Movies => &self.movies,
            Category::Software => &self.software,
            _ => unreachable!("NNM-Club filters unsupported categories before this call"),
        }
    }

    fn category(&self, forum: u64) -> Option<Category> {
        if self.movies.contains(&forum) {
            Some(Category::Movies)
        } else if self.software.contains(&forum) {
            Some(Category::Software)
        } else {
            None
        }
    }
}

/// The source's search form accepts CP1251-encoded values. Forum IDs come
/// from the live form and are sorted to keep requests deterministic.
pub(super) fn search_form(encoded_query: &str, forums: &HashSet<u64>) -> String {
    let mut ids: Vec<_> = forums.iter().copied().collect();
    ids.sort_unstable();
    let mut form = format!("nm={encoded_query}&o=1&s=2&tm=-1&shf=1&sha=1&ta=-1&sns=-1&sds=-1");
    for id in ids {
        form.push_str(&format!("&f%5B%5D={id}"));
    }
    form.push_str("&submit=%CF%EE%E8%F1%EA");
    form
}

pub(super) struct Release {
    pub(super) work: Work,
    magnet: Url,
}

impl Release {
    pub(super) fn resolve(self, selected: &VariantRef) -> Result<ResolvedContent> {
        if self.work.variants[0].reference != *selected {
            return Err(Error::VariantUnavailable);
        }
        Ok(ResolvedContent::Torrent {
            reference: selected.clone(),
            title: self.work.title,
            magnet: self.magnet,
        })
    }
}

pub(super) fn search(
    body: &str,
    origin: &Url,
    category: Category,
    forums: &Forums,
) -> Result<Vec<Work>> {
    let document = Html::parse_document(body);
    let root = document.root_element();
    let table = unique(root, "table.forumline.tablesorter")?;
    let mut seen = HashSet::new();
    let mut works = Vec::new();
    for (index, row) in table
        .select(&selector("tbody > tr.prow1, tbody > tr.prow2"))
        .enumerate()
    {
        if index >= 500 {
            return Err(Error::InvalidResponse("excessive NNM-Club results"));
        }
        let title = unique(row, "a[href*='viewtopic.php?t=']")?;
        let reference = publication(required(title, "href")?, origin)?;
        if !seen.insert(reference.clone()) {
            return Err(Error::InvalidResponse("duplicate NNM-Club topic"));
        }
        let forum = unique(row, "a[href*='tracker.php?f=']")?;
        let forum = forum_id(required(forum, "href")?, origin, "tracker.php")?;
        if forums.category(forum) != Some(category) {
            continue;
        }
        let title = text(title);
        works.push(work(reference, origin, category, title, None, true)?);
    }
    Ok(works)
}

pub(super) fn release(
    body: &str,
    origin: &Url,
    expected: &PublicationRef,
    forums: &Forums,
) -> Result<Release> {
    expected.nnmclub_id()?;
    let document = Html::parse_document(body);
    let root = document.root_element();
    let title = unique(root, "h1 a.maintitle")?;
    if publication(required(title, "href")?, origin)? != *expected {
        return Err(Error::VariantUnavailable);
    }
    let mut category = None;
    for link in root.select(&selector("a.nav[href*='viewforum.php?f=']")) {
        let forum = forum_id(required(link, "href")?, origin, "viewforum.php")?;
        if let Some(found) = forums.category(forum) {
            if category.is_some_and(|previous| previous != found) {
                return Err(Error::InvalidResponse("ambiguous NNM-Club topic category"));
            }
            category = Some(found);
        }
    }
    let category = category.ok_or(Error::InvalidResponse("unknown NNM-Club topic forum"))?;
    let magnet_selector = selector("table.btTbl a[href^='magnet:']");
    let mut magnets = root.select(&magnet_selector);
    let magnet_link = magnets.next().ok_or(Error::UnsupportedAcquisition(
        "NNM-Club publication has no public magnet",
    ))?;
    if magnets.next().is_some() {
        return Err(Error::InvalidResponse("ambiguous NNM-Club magnet"));
    }
    let (magnet, hash) = magnet(required(magnet_link, "href")?)?;
    let title = text(title);
    let mut work = work(expected.clone(), origin, category, title, Some(hash), false)?;
    if let Some(post) = root
        .select(&selector("table[id^='post_'] .postbody"))
        .next()
    {
        let description = text(post);
        if !description.is_empty() {
            work.description = Some(description.chars().take(16_000).collect());
        }
    }
    Ok(Release { work, magnet })
}

pub(super) fn topic_url(origin: &Url, publication: &PublicationRef) -> Url {
    origin
        .join(&format!("/forum/viewtopic.php?t={}", publication.id))
        .expect("canonical NNM-Club topic ID")
}

fn work(
    reference: PublicationRef,
    origin: &Url,
    category: Category,
    title: String,
    hash: Option<String>,
    needs_inspection: bool,
) -> Result<Work> {
    if title.is_empty() || title.len() > 4096 {
        return Err(Error::InvalidResponse("invalid NNM-Club topic title"));
    }
    let variants = hash
        .into_iter()
        .map(|hash| Variant {
            reference: VariantRef {
                publication: reference.clone(),
                selector: VariantSelector::TorrentHash {
                    info_hash: hash.clone(),
                },
            },
            title: Some(title.clone()),
            transport: Transport::Torrent,
            episode: None,
            quality: None,
            translation: None,
            languages: Vec::new(),
            player: None,
            size: None,
            seeders: None,
            info_hash: Some(hash),
            reported_video_ads: false,
        })
        .collect();
    Ok(Work {
        url: topic_url(origin, &reference),
        reference,
        title,
        category,
        cover: None,
        description: None,
        year: None,
        aliases: Vec::new(),
        identifiers: BTreeMap::new(),
        variants,
        needs_inspection,
    })
}

fn publication(href: &str, origin: &Url) -> Result<PublicationRef> {
    let url = site_url(href, origin)?;
    if url.path() != "/forum/viewtopic.php" || url.query_pairs().count() != 1 {
        return Err(Error::InvalidResponse("invalid NNM-Club topic path"));
    }
    let id = query_id(&url, "t")?;
    let reference = PublicationRef {
        source: Source::NnmClub,
        id: id.to_string(),
    };
    reference.nnmclub_id()?;
    Ok(reference)
}

fn forum_id(href: &str, origin: &Url, path: &str) -> Result<u64> {
    let url = site_url(href, origin)?;
    if url.path() != format!("/forum/{path}") {
        return Err(Error::InvalidResponse("invalid NNM-Club forum path"));
    }
    query_id(&url, "f")
}

fn query_id(url: &Url, key: &str) -> Result<u64> {
    let mut id = None;
    for (name, value) in url.query_pairs() {
        if name == key {
            if id.replace(decimal(&value)?).is_some() {
                return Err(Error::InvalidResponse("duplicate NNM-Club identity"));
            }
        } else if name != "nm" {
            return Err(Error::InvalidResponse("unexpected NNM-Club URL field"));
        }
    }
    id.ok_or(Error::InvalidResponse("missing NNM-Club identity"))
}

fn site_url(href: &str, origin: &Url) -> Result<Url> {
    let base = origin.join("/forum/").unwrap();
    let url = base
        .join(href)
        .map_err(|_| Error::InvalidResponse("invalid NNM-Club link"))?;
    if url.origin() != origin.origin()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::InvalidResponse("foreign NNM-Club link"));
    }
    Ok(url)
}

fn decimal(value: &str) -> Result<u64> {
    if !canonical_decimal(value) {
        return Err(Error::InvalidResponse("invalid NNM-Club numeric identity"));
    }
    value
        .parse()
        .map_err(|_| Error::InvalidResponse("NNM-Club numeric identity overflow"))
}

fn required<'a>(element: ElementRef<'a>, name: &str) -> Result<&'a str> {
    element
        .attr(name)
        .ok_or(Error::InvalidResponse("missing NNM-Club attribute"))
}

fn selector(value: &str) -> Selector {
    Selector::parse(value).expect("fixed NNM-Club selector")
}

fn unique<'a>(parent: ElementRef<'a>, value: &str) -> Result<ElementRef<'a>> {
    let query = selector(value);
    let mut matches = parent.select(&query);
    let first = matches
        .next()
        .ok_or(Error::InvalidResponse("missing NNM-Club metadata"))?;
    if matches.next().is_some() {
        return Err(Error::InvalidResponse("ambiguous NNM-Club metadata"));
    }
    Ok(first)
}

fn text(element: ElementRef<'_>) -> String {
    element
        .text()
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
