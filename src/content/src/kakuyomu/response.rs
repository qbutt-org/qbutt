mod state;

use std::collections::{BTreeMap, HashSet};

use scraper::{Html, Selector};
use serde::Deserialize;
use url::Url;

use crate::{
    Category, Chapter, ChapterRef, Error, PublicationRef, Result, Source, TextChapter, Transport,
    Variant, VariantRef, VariantSelector, Work,
};
use state::{NodeRef, State};

const MAX_CHAPTERS: usize = 10_000;

pub(super) struct Listing {
    pub work: Work,
    pub chapters: Vec<Chapter>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkRecord {
    id: String,
    title: String,
    introduction: Option<String>,
    public_episode_count: usize,
    #[serde(deserialize_with = "Option::deserialize")]
    kakuyomu_next_work: Option<NodeRef>,
    table_of_contents_v2: Option<Vec<NodeRef>>,
}

impl WorkRecord {
    fn into_work(self, needs_inspection: bool) -> Result<Work> {
        let reference = PublicationRef {
            source: Source::Kakuyomu,
            id: self.id,
        };
        reference.validate()?;
        if self.title.trim().is_empty()
            || self.title.len() > 4096
            || self
                .introduction
                .as_ref()
                .is_some_and(|text| text.len() > 128 * 1024)
            || self.public_episode_count > MAX_CHAPTERS
        {
            return Err(Error::InvalidResponse("invalid Kakuyomu work"));
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
            url: Url::parse(&format!("https://kakuyomu.jp/works/{}", reference.id))
                .expect("validated Kakuyomu work ID"),
            reference,
            title: self.title,
            category: Category::Books,
            cover: None,
            description: self.introduction.filter(|text| !text.trim().is_empty()),
            year: None,
            aliases: Vec::new(),
            identifiers: BTreeMap::new(),
            variants: vec![variant],
            notes: Vec::new(),
            needs_inspection,
        })
    }
}

pub(super) fn search(document: &str, query: &str) -> Result<Vec<Work>> {
    let parsed = Html::parse_document(document);
    let state = State::parse(&parsed)?;
    let nodes = state.search_nodes(query)?;
    let mut seen = HashSet::new();
    let mut works = Vec::new();
    for node in nodes {
        let record: WorkRecord = state.record(&node, "Work")?;
        if !seen.insert(record.id.clone()) {
            return Err(Error::InvalidResponse("duplicate Kakuyomu search work"));
        }
        if record.kakuyomu_next_work.is_none() && record.public_episode_count != 0 {
            works.push(record.into_work(true)?);
        }
    }
    Ok(works)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TocSection {
    episode_unions: Vec<NodeRef>,
    chapter: Option<NodeRef>,
}

#[derive(Deserialize)]
struct Episode {
    id: String,
    title: String,
}

#[derive(Deserialize)]
struct Arc {
    title: String,
}

pub(super) fn publication(document: &str, reference: &PublicationRef) -> Result<Listing> {
    let parsed = Html::parse_document(document);
    canonical(&parsed, &format!("/works/{}", reference.id))?;
    let state = State::parse(&parsed)?;
    let node = NodeRef {
        key: format!("Work:{}", reference.id),
    };
    let mut record: WorkRecord = state.record(&node, "Work")?;
    if record.kakuyomu_next_work.is_some() || record.public_episode_count == 0 {
        return Err(Error::VariantUnavailable);
    }
    let sections = record
        .table_of_contents_v2
        .take()
        .ok_or(Error::InvalidResponse("missing Kakuyomu table of contents"))?;
    if sections.len() > MAX_CHAPTERS {
        return Err(Error::ResponseTooLarge);
    }
    let expected_count = record.public_episode_count;
    let work = record.into_work(false)?;
    let mut chapters = Vec::new();
    let mut seen = HashSet::new();
    for section in sections {
        let section: TocSection = state.record(&section, "TableOfContentsChapter")?;
        let volume = section
            .chapter
            .map(|node| state.record::<Arc>(&node, "Chapter").map(|arc| arc.title))
            .transpose()?;
        if volume
            .as_ref()
            .is_some_and(|title| title.trim().is_empty() || title.len() > 4096)
        {
            return Err(Error::InvalidResponse("invalid Kakuyomu chapter heading"));
        }
        for episode in section.episode_unions {
            if chapters.len() == MAX_CHAPTERS {
                return Err(Error::ResponseTooLarge);
            }
            let episode: Episode = state.record(&episode, "Episode")?;
            let chapter = ChapterRef {
                variant: work.variants[0].reference.clone(),
                id: format!("{}/{}", reference.id, episode.id),
            };
            chapter.validate()?;
            if episode.title.trim().is_empty()
                || episode.title.len() > 4096
                || !seen.insert(chapter.clone())
            {
                return Err(Error::InvalidResponse("invalid Kakuyomu episode"));
            }
            chapters.push(Chapter {
                reference: chapter,
                title: Some(episode.title),
                volume: volume.clone(),
                number: Some((chapters.len() + 1).to_string()),
                pages: 0,
            });
        }
    }
    if chapters.len() != expected_count {
        return Err(Error::InvalidResponse(
            "incomplete Kakuyomu table of contents",
        ));
    }
    Ok(Listing { work, chapters })
}

pub(super) fn chapter(document: &str, reference: &ChapterRef, path: &str) -> Result<TextChapter> {
    let parsed = Html::parse_document(document);
    canonical(&parsed, path)?;
    let selector = Selector::parse("div.widget-episodeBody").expect("fixed body selector");
    let mut bodies = parsed.select(&selector);
    let body = bodies
        .next()
        .ok_or(Error::InvalidResponse("missing Kakuyomu chapter body"))?;
    if bodies.next().is_some()
        || body.value().attr("data-viewer-history-path") != Some(format!("{path}/history").as_str())
    {
        return Err(Error::InvalidResponse("invalid Kakuyomu chapter identity"));
    }
    TextChapter::from_html(reference.clone(), &body.inner_html())
}

fn canonical(document: &Html, path: &str) -> Result<()> {
    let selector = Selector::parse("link[rel='canonical']").expect("fixed canonical selector");
    let mut links = document.select(&selector);
    let expected = format!("https://kakuyomu.jp{path}");
    if links.next().and_then(|link| link.value().attr("href")) != Some(expected.as_str())
        || links.next().is_some()
    {
        return Err(Error::VariantUnavailable);
    }
    Ok(())
}
