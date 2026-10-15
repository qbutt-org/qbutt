use std::collections::{BTreeMap, HashMap, HashSet};

use scraper::{ElementRef, Html, Selector};
use url::Url;

use crate::{
    Category, Chapter, ChapterRef, Error, PublicationRef, Result, Source, TextChapter,
    TextImageAsset, Transport, Variant, VariantRef, VariantSelector, Work,
};

pub(super) const READER_ORIGIN: &str = "https://ncode.syosetu.com";
pub(super) const MAX_PAGES: usize = 100;
const MAX_CHAPTERS: usize = 10_000;
const MAX_IMAGES: usize = 64;

pub(super) struct BookPage {
    pub work: Work,
    pub chapters: Vec<Chapter>,
    pub total_pages: usize,
}

pub(super) struct Book {
    pub work: Work,
    pub chapters: Vec<Chapter>,
}

pub(super) struct ChapterPage {
    reference: ChapterRef,
    html: String,
    pub images: Vec<Url>,
}

impl ChapterPage {
    pub fn into_text(self, images: HashMap<Url, TextImageAsset>) -> Result<TextChapter> {
        TextChapter::from_html_with_images(
            self.reference,
            &self.html,
            &mut |source| {
                let url = image_url(source)?;
                let image = images
                    .get(&url)
                    .cloned()
                    .ok_or(Error::InvalidResponse("missing Syosetu chapter image"))?;
                Ok(Some(image))
            },
            None,
            None,
        )
    }
}

pub(super) fn search(body: &str) -> Result<Vec<Work>> {
    let document = Html::parse_document(body);
    if document.select(&selector("#main_search")).next().is_none() {
        return Err(Error::InvalidResponse("missing Syosetu search results"));
    }

    let mut works = Vec::new();
    let mut seen = HashSet::new();
    for row in document.select(&selector(".searchkekka_box")) {
        if works.len() == 100 {
            return Err(Error::ResponseTooLarge);
        }
        let title = row
            .select(&selector(".novel_h a.tl[href]"))
            .next()
            .ok_or(Error::InvalidResponse("missing Syosetu search title"))?;
        let url = site_url(title.attr("href").unwrap())?;
        let reference = publication(&url)?;
        if !seen.insert(reference.clone()) {
            return Err(Error::InvalidResponse("duplicate Syosetu search work"));
        }
        let mut work = work(reference, label(title.text().collect::<String>())?, true);
        work.description = row
            .select(&selector(".ex"))
            .next()
            .map(|summary| summary.text().collect::<String>().trim().to_owned())
            .filter(|summary| !summary.is_empty() && summary.len() <= 100_000);
        works.push(work);
    }
    Ok(works)
}

pub(super) fn book_page(
    body: &str,
    expected: &PublicationRef,
    page: usize,
    known_total: Option<usize>,
) -> Result<BookPage> {
    if expected.source != Source::Syosetu {
        return Err(Error::InvalidReference);
    }
    expected.validate()?;
    let document = Html::parse_document(body);
    check_publication(&document, expected)?;
    let title = one(&document, ".p-novel__title")?;
    let mut work = work(
        expected.clone(),
        label(title.text().collect::<String>())?,
        false,
    );
    work.description = document
        .select(&selector(".p-novel__summary"))
        .next()
        .map(|summary| summary.text().collect::<String>().trim().to_owned())
        .filter(|description| !description.is_empty() && description.len() <= 100_000);

    let total_pages = if let Some(total) = known_total {
        total
    } else {
        document
            .select(&selector("a.c-pager__item--last[href]"))
            .next()
            .map(|link| {
                let url = site_url_with_page(link.attr("href").unwrap())?;
                if publication(&url)? != *expected {
                    return Err(Error::InvalidResponse("foreign Syosetu pagination"));
                }
                url.query_pairs()
                    .find(|(key, _)| key == "p")
                    .and_then(|(_, value)| value.parse::<usize>().ok())
                    .ok_or(Error::InvalidResponse("invalid Syosetu pagination"))
            })
            .transpose()?
            .unwrap_or(1)
    };
    if total_pages == 0 || total_pages > MAX_PAGES || page > total_pages {
        return Err(Error::ResponseTooLarge);
    }

    let mut chapters = Vec::new();
    for entry in document.select(&selector(".p-eplist__sublist a[href]")) {
        let url = site_url(entry.attr("href").unwrap())?;
        let chapter = chapter_reference(&url)?;
        if chapter.variant.publication != *expected {
            return Err(Error::InvalidResponse("foreign Syosetu chapter"));
        }
        chapters.push(Chapter {
            reference: chapter,
            title: Some(label(entry.text().collect::<String>())?),
            volume: None,
            number: None,
            pages: 0,
        });
    }
    if chapters.is_empty() && page == 1 && total_pages == 1 {
        if document
            .select(&selector(
                ".p-novel__text:not(.p-novel__text--preface):not(.p-novel__text--afterword)",
            ))
            .next()
            .is_some()
        {
            chapters.push(Chapter {
                reference: ChapterRef {
                    variant: work.variants[0].reference.clone(),
                    id: "root".into(),
                },
                title: Some(work.title.clone()),
                volume: None,
                number: None,
                pages: 0,
            });
        }
    }
    if chapters.is_empty() || chapters.len() > MAX_CHAPTERS {
        return Err(Error::InvalidResponse("missing Syosetu chapters"));
    }
    Ok(BookPage {
        work,
        chapters,
        total_pages,
    })
}

pub(super) fn chapter(body: &str, expected: &ChapterRef) -> Result<ChapterPage> {
    expected.validate()?;
    let document = Html::parse_document(body);
    let published = site_url(
        one(&document, "meta[property='og:url'][content]")?
            .attr("content")
            .unwrap(),
    )?;
    if chapter_reference(&published)? != *expected {
        return Err(Error::VariantUnavailable);
    }

    let mut sections = [None; 3];
    let mut images = Vec::new();
    for section in document.select(&selector(".p-novel__text")) {
        let order = if section
            .value()
            .classes()
            .any(|class| class == "p-novel__text--preface")
        {
            0
        } else if section
            .value()
            .classes()
            .any(|class| class == "p-novel__text--afterword")
        {
            2
        } else {
            1
        };
        if sections[order].replace(section).is_some() {
            return Err(Error::InvalidResponse("duplicate Syosetu chapter section"));
        }
        for image in section.select(&selector("img[src]")) {
            let url = image_url(image.attr("src").unwrap())?;
            if !images.contains(&url) {
                if images.len() == MAX_IMAGES {
                    return Err(Error::ResponseTooLarge);
                }
                images.push(url);
            }
        }
    }
    if sections[1].is_none() {
        return Err(Error::InvalidResponse("missing Syosetu chapter body"));
    }
    Ok(ChapterPage {
        reference: expected.clone(),
        html: sections
            .into_iter()
            .flatten()
            .map(|section| section.inner_html())
            .collect::<Vec<_>>()
            .join("<hr>"),
        images,
    })
}

pub(super) fn image_url(value: &str) -> Result<Url> {
    let origin = Url::parse(READER_ORIGIN).expect("fixed Syosetu origin");
    let url = origin.join(value).map_err(|_| Error::UnsafeAddress)?;
    let account = url
        .host_str()
        .and_then(|host| host.strip_suffix(".mitemin.net"));
    let image = url
        .path()
        .strip_prefix("/userpageimage/viewimagebig/icode/i")
        .and_then(|path| path.strip_suffix('/'));
    if !account
        .is_some_and(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        || !image.is_some_and(|value| {
            !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
        })
        || url.port().is_some()
        || url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::UnsafeAddress);
    }
    Ok(url)
}

pub(super) fn join_pages(pages: Vec<BookPage>) -> Result<Book> {
    let mut pages = pages.into_iter();
    let first = pages
        .next()
        .ok_or(Error::InvalidResponse("missing Syosetu table of contents"))?;
    let mut chapters = first.chapters;
    let mut seen = chapters
        .iter()
        .map(|chapter| chapter.reference.id.clone())
        .collect::<HashSet<_>>();
    if seen.len() != chapters.len() {
        return Err(Error::InvalidResponse("duplicate Syosetu chapter"));
    }
    for page in pages {
        if page.work.reference != first.work.reference || page.work.title != first.work.title {
            return Err(Error::InvalidResponse("Syosetu table of contents changed"));
        }
        for chapter in page.chapters {
            if !seen.insert(chapter.reference.id.clone()) {
                return Err(Error::InvalidResponse("duplicate Syosetu chapter"));
            }
            chapters.push(chapter);
            if chapters.len() > MAX_CHAPTERS {
                return Err(Error::ResponseTooLarge);
            }
        }
    }
    for (index, chapter) in chapters.iter_mut().enumerate() {
        chapter.number = Some((index + 1).to_string());
    }
    Ok(Book {
        work: first.work,
        chapters,
    })
}

fn work(reference: PublicationRef, title: String, needs_inspection: bool) -> Work {
    let url = publication_url(&reference);
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
        languages: vec!["ja".into()],
        player: None,
        size: None,
        seeders: None,
        info_hash: None,
        reported_video_ads: false,
    };
    Work {
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
    }
}

pub(super) fn publication_url(reference: &PublicationRef) -> Url {
    Url::parse(&format!("{READER_ORIGIN}/{}/", reference.id))
        .expect("validated Syosetu publication ID")
}

pub(super) fn chapter_url(reference: &ChapterRef) -> Url {
    if reference.id == "root" {
        publication_url(&reference.variant.publication)
    } else {
        Url::parse(&format!(
            "{READER_ORIGIN}/{}/{}/",
            reference.variant.publication.id, reference.id
        ))
        .expect("validated Syosetu chapter ID")
    }
}

fn check_publication(document: &Html, expected: &PublicationRef) -> Result<()> {
    let value = one(document, "meta[property='og:url'][content]")?
        .attr("content")
        .unwrap();
    if site_url(value)? != publication_url(expected) {
        return Err(Error::VariantUnavailable);
    }
    Ok(())
}

fn chapter_reference(url: &Url) -> Result<ChapterRef> {
    let segments: Vec<_> = url
        .path_segments()
        .ok_or(Error::InvalidReference)?
        .filter(|part| !part.is_empty())
        .collect();
    if !(1..=2).contains(&segments.len()) {
        return Err(Error::InvalidReference);
    }
    let reference = PublicationRef {
        source: Source::Syosetu,
        id: segments[0].to_owned(),
    };
    reference.validate()?;
    let chapter = ChapterRef {
        variant: VariantRef {
            publication: reference,
            selector: VariantSelector::OriginalText,
        },
        id: segments.get(1).map_or("root", |id| id).to_string(),
    };
    chapter.validate()?;
    Ok(chapter)
}

fn publication(url: &Url) -> Result<PublicationRef> {
    let chapter = chapter_reference(url)?;
    if chapter.id != "root" {
        return Err(Error::InvalidReference);
    }
    Ok(chapter.variant.publication)
}

fn site_url(value: &str) -> Result<Url> {
    let url = site_url_with_page(value)?;
    if url.query().is_some() {
        return Err(Error::UnsafeAddress);
    }
    Ok(url)
}

fn site_url_with_page(value: &str) -> Result<Url> {
    let origin = Url::parse(READER_ORIGIN).expect("fixed Syosetu origin");
    let url = origin.join(value).map_err(|_| Error::UnsafeAddress)?;
    if url.origin() != origin.origin()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::UnsafeAddress);
    }
    Ok(url)
}

fn label(value: String) -> Result<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(Error::InvalidResponse("invalid Syosetu title"));
    }
    Ok(value.into())
}

fn one<'a>(document: &'a Html, css: &str) -> Result<ElementRef<'a>> {
    let selected = selector(css);
    let mut found = document.select(&selected);
    let first = found
        .next()
        .ok_or(Error::InvalidResponse("missing Syosetu page element"))?;
    if found.next().is_some() {
        return Err(Error::InvalidResponse("ambiguous Syosetu page element"));
    }
    Ok(first)
}

fn selector(css: &str) -> Selector {
    Selector::parse(css).expect("fixed Syosetu selector")
}
