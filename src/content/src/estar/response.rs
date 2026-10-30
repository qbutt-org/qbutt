use std::collections::{BTreeMap, HashSet};

use scraper::{Html, Selector};
use serde_json::Value;
use url::Url;

use crate::{
    Category, Chapter, ChapterRef, Error, PublicationRef, Result, Source, Transport, Variant,
    VariantRef, VariantSelector, Work,
};

const MAX_EPISODES: usize = 3_000;
const MAX_EPISODE_PAGES: u32 = 256;

pub(super) struct Book {
    pub work: Work,
    pub episode_count: usize,
    page_count: u32,
}

pub(super) struct Entry {
    pub chapter: Chapter,
    pub start: u32,
    pub end: u32,
}

pub(super) struct Page {
    pub html: String,
    pub images: Vec<String>,
}

pub(super) struct Episode {
    title: String,
    volume: Option<String>,
    start: u32,
}

/// Nuxt serializes objects as array slots and object fields as slot indices.
struct State {
    slots: Vec<Value>,
}

impl State {
    fn parse(document: &Html) -> Result<Self> {
        let state_selector = selector("script#__NUXT_DATA__");
        let mut scripts = document.select(&state_selector);
        let script = scripts
            .next()
            .ok_or(Error::InvalidResponse("missing Estar page state"))?;
        if scripts.next().is_some() {
            return Err(Error::InvalidResponse("ambiguous Estar page state"));
        }
        let slots: Vec<Value> =
            serde_json::from_str(&script.text().collect::<String>()).map_err(Error::Json)?;
        if slots.len() > 200_000 {
            return Err(Error::ResponseTooLarge);
        }
        Ok(Self { slots })
    }

    fn slot(&self, reference: &Value) -> Result<&Value> {
        let index = reference
            .as_u64()
            .ok_or(Error::InvalidResponse("invalid Estar state reference"))?;
        self.slots
            .get(index as usize)
            .ok_or(Error::InvalidResponse("missing Estar state slot"))
    }

    fn field(&self, object: &Value, name: &str) -> Result<&Value> {
        let reference = object
            .get(name)
            .ok_or(Error::InvalidResponse("missing Estar state field"))?;
        self.slot(reference)
    }

    fn tagged(&self, value: &Value, kind: &str) -> Result<&Value> {
        let values = value
            .as_array()
            .ok_or(Error::InvalidResponse("invalid Estar state model"))?;
        if values.len() != 2 || values[0].as_str() != Some(kind) {
            return Err(Error::InvalidResponse("changed Estar state model"));
        }
        self.slot(&values[1])
    }

    fn root(&self) -> Result<&Value> {
        let root = self
            .slots
            .first()
            .ok_or(Error::InvalidResponse("empty Estar page state"))?;
        self.tagged(root, "ShallowReactive")
    }

    fn data(&self) -> Result<&Value> {
        self.tagged(self.field(self.root()?, "data")?, "ShallowReactive")
    }

    fn novel(&self, key: &str) -> Result<&Value> {
        let page = self.field(self.data()?, key)?;
        self.tagged(self.field(page, "novel")?, "ModelNovel")
    }

    fn string<'a>(&'a self, object: &Value, name: &str) -> Result<&'a str> {
        self.field(object, name)?
            .as_str()
            .ok_or(Error::InvalidResponse("invalid Estar text field"))
    }

    fn number(&self, object: &Value, name: &str) -> Result<u64> {
        self.field(object, name)?
            .as_u64()
            .ok_or(Error::InvalidResponse("invalid Estar number field"))
    }

    fn boolean(&self, object: &Value, name: &str) -> Result<bool> {
        self.field(object, name)?
            .as_bool()
            .ok_or(Error::InvalidResponse("invalid Estar boolean field"))
    }
}

pub(super) fn search(body: &str, query: &str) -> Result<Vec<Work>> {
    let document = Html::parse_document(body);
    let state = State::parse(&document)?;
    let page = state.field(state.data()?, "novels")?;
    let filter = state.field(page, "filter")?;
    if state.string(filter, "keyword")? != query || state.string(filter, "ex_ticket")? != "1" {
        return Err(Error::InvalidResponse("changed Estar search filters"));
    }

    let result = state.field(page, "novels")?;
    let nodes = state
        .field(result, "nodes")?
        .as_array()
        .ok_or(Error::InvalidResponse("invalid Estar search results"))?;
    if nodes.len() > 30 {
        return Err(Error::InvalidResponse("too many Estar search results"));
    }

    let mut seen = HashSet::new();
    let mut works = Vec::new();
    for node in nodes {
        let record = state.slot(node)?;
        let id = state.string(record, "workId")?;
        if !seen.insert(id.to_owned()) {
            return Err(Error::InvalidResponse("invalid Estar search identity"));
        }
        let body_count = state
            .string(record, "publishedBodyCount")?
            .parse::<u64>()
            .map_err(|_| Error::InvalidResponse("invalid Estar body count"))?;
        if state.string(record, "status")? != "published" || body_count == 0 {
            continue;
        }
        let mut work = work(id, state.string(record, "title")?, true)?;
        let description = state.string(record, "catchphrase")?.trim();
        if !description.is_empty() && description.len() <= 100_000 {
            work.description = Some(description.into());
        }
        works.push(work);
    }
    Ok(works)
}

pub(super) fn book(body: &str, expected: &PublicationRef) -> Result<Book> {
    if expected.source != Source::Estar {
        return Err(Error::InvalidReference);
    }
    expected.validate()?;
    let document = Html::parse_document(body);
    let canonical = document
        .select(&selector("link[rel='canonical']"))
        .collect::<Vec<_>>();
    let expected_url = format!("https://estar.jp/novels/{}", expected.id);
    if canonical.len() != 1 || canonical[0].value().attr("href") != Some(expected_url.as_str()) {
        return Err(Error::VariantUnavailable);
    }

    let state = State::parse(&document)?;
    let record = state.novel(&format!("novel_{}", expected.id))?;
    if state.string(record, "workId")? != expected.id
        || !state.boolean(record, "isNovel")?
        || state.string(record, "status")? != "published"
    {
        return Err(Error::VariantUnavailable);
    }
    let episode_count = usize::try_from(state.number(record, "episodeCount")?)
        .map_err(|_| Error::ResponseTooLarge)?;
    let page_count = u32::try_from(state.number(record, "publishedPageCount")?)
        .map_err(|_| Error::ResponseTooLarge)?;
    if episode_count == 0 || episode_count > MAX_EPISODES || page_count == 0 || page_count > 100_000
    {
        return Err(Error::VariantUnavailable);
    }

    let mut work = work(&expected.id, state.string(record, "title")?, false)?;
    let description = state.string(record, "description")?.trim();
    if description.len() > 100_000 {
        return Err(Error::ResponseTooLarge);
    }
    work.description = (!description.is_empty()).then(|| description.into());
    if let Some(cover) = document
        .select(&selector("meta[property='og:image'][content]"))
        .next()
        .and_then(|meta| meta.value().attr("content"))
    {
        let mut url =
            Url::parse(cover).map_err(|_| Error::InvalidResponse("invalid Estar cover"))?;
        if url.scheme() != "https" || url.host_str() != Some("img.estar.jp") {
            return Err(Error::InvalidResponse("invalid Estar cover host"));
        }
        if url.fragment().is_some() || url.query_pairs().any(|(key, _)| key != "timestamp") {
            return Err(Error::InvalidResponse("invalid Estar cover query"));
        }
        url.set_query(None);
        work.cover = Some(url);
    }

    Ok(Book {
        work,
        episode_count,
        page_count,
    })
}

pub(super) fn episodes(
    body: &str,
    publication: &PublicationRef,
    page: usize,
) -> Result<Vec<Episode>> {
    let document = Html::parse_document(body);
    let state = State::parse(&document)?;
    if state.string(state.root()?, "path")?
        != format!("/novels/{}/episodes?page={page}", publication.id)
    {
        return Err(Error::InvalidResponse("changed Estar episode page"));
    }
    let record = state.novel(&format!("novel_episode_{}", publication.id))?;
    if state.string(record, "workId")? != publication.id {
        return Err(Error::VariantUnavailable);
    }

    let mut episodes = Vec::new();
    for item in document.select(&selector(".episodeList > .item")) {
        let volume = item
            .select(&selector(".chapterTitle"))
            .next()
            .map(|label| text(label));
        let link = item.select(&selector("a.link")).next();
        let Some(link) = link else { continue };
        let title = link
            .select(&selector(".label"))
            .next()
            .map(text)
            .ok_or(Error::InvalidResponse("missing Estar episode title"))?;
        let start = link
            .select(&selector(".meta"))
            .next()
            .map(text)
            .and_then(|value| value.parse::<u32>().ok())
            .ok_or(Error::InvalidResponse("missing Estar episode page"))?;
        if title.is_empty() || title.len() > 4096 || start == 0 {
            return Err(Error::InvalidResponse("invalid Estar episode"));
        }
        episodes.push(Episode {
            title,
            volume,
            start,
        });
    }
    if episodes.is_empty() || episodes.len() > 30 {
        return Err(Error::InvalidResponse("invalid Estar episode listing"));
    }
    Ok(episodes)
}

pub(super) fn listing(book: &Book, pages: Vec<Vec<Episode>>) -> Result<Vec<Entry>> {
    let episodes = pages.into_iter().flatten().collect::<Vec<_>>();
    if episodes.len() != book.episode_count {
        return Err(Error::InvalidResponse("incomplete Estar episode listing"));
    }
    let mut volume = None;
    let mut entries = Vec::with_capacity(episodes.len());
    for (index, episode) in episodes.iter().enumerate() {
        let end = episodes
            .get(index + 1)
            .map_or(book.page_count + 1, |next| next.start);
        if episode.start >= end || (index == 0 && episode.start != 1) {
            return Err(Error::InvalidResponse("invalid Estar episode order"));
        }
        let pages = end - episode.start;
        if pages > MAX_EPISODE_PAGES {
            return Err(Error::ResponseTooLarge);
        }
        if let Some(chapter_title) = &episode.volume {
            volume = Some(chapter_title.clone());
        }
        entries.push(Entry {
            chapter: Chapter {
                reference: ChapterRef {
                    variant: book.work.variants[0].reference.clone(),
                    id: format!("{}/{}", book.work.reference.id, episode.start),
                },
                title: Some(episode.title.clone()),
                volume: volume.clone(),
                number: Some((index + 1).to_string()),
                pages: 0,
            },
            start: episode.start,
            end,
        });
    }
    Ok(entries)
}

pub(super) fn viewer(
    body: &str,
    publication: &PublicationRef,
    chapter: &Chapter,
    page: u32,
) -> Result<Page> {
    let document = Html::parse_document(body);
    let state = State::parse(&document)?;
    let record = state.novel("novel_viewer")?;
    if state.string(record, "workId")? != publication.id {
        return Err(Error::VariantUnavailable);
    }
    let pages = state.field(state.field(record, "pages")?, "nodes")?;
    let first = pages
        .as_array()
        .and_then(|pages| pages.first())
        .ok_or(Error::InvalidResponse("missing Estar viewer page"))?;
    let selected = state.slot(first)?;
    let page_title = state.string(selected, "title")?;
    if state.number(selected, "pageNo")? != u64::from(page)
        || (!page_title.is_empty() && Some(page_title) != chapter.title.as_deref())
        || state.number(selected, "episodeNo")?.to_string()
            != chapter.number.as_deref().unwrap_or_default()
    {
        return Err(Error::VariantUnavailable);
    }
    if state.boolean(selected, "payRequired")? {
        return Err(Error::UnsupportedAcquisition(
            "This Estar page requires a ticket",
        ));
    }

    let content_selector = selector(".content[lang='ja']");
    let mut contents = document.select(&content_selector);
    let content = contents
        .next()
        .ok_or(Error::InvalidResponse("missing Estar chapter text"))?;
    if contents.next().is_some() {
        return Err(Error::InvalidResponse("invalid Estar chapter text"));
    }
    let images = content
        .select(&selector("img[src]"))
        .filter_map(|image| image.value().attr("src"))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if images.len() > 16 {
        return Err(Error::ResponseTooLarge);
    }
    if text(content).is_empty() && images.is_empty() {
        return Err(Error::InvalidResponse("empty Estar chapter text"));
    }
    let html = content
        .inner_html()
        .replace("\r\n", "\n")
        .replace('\n', "<br>");
    Ok(Page { html, images })
}

fn work(id: &str, title: &str, needs_inspection: bool) -> Result<Work> {
    let reference = PublicationRef {
        source: Source::Estar,
        id: id.into(),
    };
    reference.validate()?;
    if title.trim().is_empty() || title.len() > 4096 {
        return Err(Error::InvalidResponse("invalid Estar title"));
    }
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
    Ok(Work {
        url: Url::parse(&format!("https://estar.jp/novels/{id}")).expect("validated Estar work ID"),
        reference,
        title: title.into(),
        category: Category::Books,
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

fn selector(value: &str) -> Selector {
    Selector::parse(value).expect("fixed Estar selector")
}

fn text(element: scraper::ElementRef<'_>) -> String {
    element.text().collect::<String>().trim().to_owned()
}
