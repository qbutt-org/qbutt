//! Typed provider dispatch shared by search, inspection, playback and retries.

use std::sync::Arc;

use qbutt_content::browser::BrowserChoice;
use qbutt_content::{
    AniLiberty, Chapter, ChapterRef, FitGirl, Kakuyomu, MangaDex, MangaLib, Nyaa, RanobeHub,
    RanobeLib, ResolvedContent, RoyalRoad, RuTor, RuTracker, Rutube, Source, SourceOperation,
    TextChapter, Work, YummyAnime,
};

use super::browser::BrowserClient;

pub enum SourceResponse {
    Search(Vec<Work>),
    Inspect(Work),
    Resolve(ResolvedContent),
    Chapters(Vec<Chapter>),
    Chapter,
    ReaderPage,
}

pub(super) struct Catalog {
    browser: BrowserClient,
    aniliberty: AniLiberty,
    yummyanime: YummyAnime,
    pub(super) mangadex: Arc<MangaDex>,
    pub(super) mangalib: Arc<MangaLib>,
    rutor: RuTor,
    rutracker: RuTracker,
    nyaa: Nyaa,
    rutube: Rutube,
    fitgirl: FitGirl,
    ranobelib: RanobeLib,
    ranobehub: RanobeHub,
    royalroad: RoyalRoad,
    kakuyomu: Kakuyomu,
}

impl Catalog {
    pub(super) fn new(browser: BrowserClient) -> qbutt_content::Result<Self> {
        Ok(Self {
            browser,
            aniliberty: AniLiberty::new()?,
            yummyanime: YummyAnime::new()?,
            mangadex: Arc::new(MangaDex::new()?),
            mangalib: Arc::new(MangaLib::new()?),
            rutor: RuTor::new()?,
            rutracker: RuTracker::new()?,
            nyaa: Nyaa::new()?,
            rutube: Rutube::new()?,
            fitgirl: FitGirl::new()?,
            ranobelib: RanobeLib::new()?,
            ranobehub: RanobeHub::new()?,
            royalroad: RoyalRoad::new()?,
            kakuyomu: Kakuyomu::new()?,
        })
    }

    pub(super) async fn execute(
        &self,
        operation: &SourceOperation,
    ) -> qbutt_content::Result<SourceResponse> {
        operation.validate()?;
        match operation {
            SourceOperation::Search {
                source,
                query,
                category,
                language,
            } => {
                let works = match source {
                    Source::LocalBook => Err(qbutt_content::Error::InvalidReference),
                    Source::AniLiberty => self.aniliberty.search(query).await,
                    Source::YummyAnime => self.yummyanime.search(query).await,
                    Source::MangaDex => {
                        self.mangadex
                            .search(query, language.as_deref().unwrap())
                            .await
                    }
                    Source::MangaLib => self.mangalib.search(query).await,
                    Source::RuTor => self.rutor.search(query, *category).await,
                    Source::RuTracker => {
                        self.rutracker
                            .search(
                                query,
                                *category,
                                self.browser.existing_handle().await.as_ref(),
                                BrowserChoice::Auto,
                            )
                            .await
                    }
                    Source::Nyaa => self.nyaa.search(query, *category).await,
                    Source::Rutube => self.rutube.search(query, *category).await,
                    Source::FitGirl => self.fitgirl.search(query, *category).await,
                    Source::RanobeLib => self.ranobelib.search(query).await,
                    Source::RanobeHub => self.ranobehub.search(query).await,
                    Source::RoyalRoad => self.royalroad.search(query).await,
                    Source::Kakuyomu => self.kakuyomu.search(query).await,
                }?;
                Ok(SourceResponse::Search(works))
            }
            SourceOperation::Inspect {
                reference,
                language,
            } => {
                let work = match reference.source {
                    Source::LocalBook => Err(qbutt_content::Error::InvalidReference),
                    Source::AniLiberty => self.aniliberty.inspect(reference).await,
                    Source::YummyAnime => self.yummyanime.inspect(reference).await,
                    Source::MangaDex => {
                        self.mangadex
                            .inspect(reference, language.as_deref().unwrap())
                            .await
                    }
                    Source::MangaLib => self.mangalib.inspect(reference).await,
                    Source::RuTor => self.rutor.inspect(reference).await,
                    Source::RuTracker => {
                        self.rutracker
                            .inspect(
                                reference,
                                self.browser.existing_handle().await.as_ref(),
                                BrowserChoice::Auto,
                            )
                            .await
                    }
                    Source::Nyaa => self.nyaa.inspect(reference).await,
                    Source::Rutube => self.rutube.inspect(reference).await,
                    Source::FitGirl => self.fitgirl.inspect(reference).await,
                    Source::RanobeLib => self.ranobelib.inspect(reference).await,
                    Source::RanobeHub => self.ranobehub.inspect(reference).await,
                    Source::RoyalRoad => self.royalroad.inspect(reference).await,
                    Source::Kakuyomu => self.kakuyomu.inspect(reference).await,
                }?;
                Ok(SourceResponse::Inspect(work))
            }
            SourceOperation::Resolve(reference) => {
                let resolved = match reference.publication.source {
                    Source::AniLiberty => self.aniliberty.resolve(reference).await,
                    Source::YummyAnime => self.yummyanime.resolve(reference).await,
                    Source::RuTor => self.rutor.resolve(reference).await,
                    Source::RuTracker => {
                        self.rutracker
                            .resolve(
                                reference,
                                self.browser.existing_handle().await.as_ref(),
                                BrowserChoice::Auto,
                            )
                            .await
                    }
                    Source::Nyaa => self.nyaa.resolve(reference).await,
                    Source::Rutube => self.rutube.resolve(reference).await,
                    Source::FitGirl => self.fitgirl.resolve(reference).await,
                    Source::MangaDex
                    | Source::LocalBook
                    | Source::MangaLib
                    | Source::RanobeLib
                    | Source::RanobeHub
                    | Source::RoyalRoad
                    | Source::Kakuyomu => Err(qbutt_content::Error::InvalidReference),
                }?;
                Ok(SourceResponse::Resolve(resolved))
            }
            SourceOperation::Chapters(reference) => {
                let chapters = match reference.publication.source {
                    Source::MangaDex => self.mangadex.chapters(reference).await,
                    Source::MangaLib => self.mangalib.chapters(reference).await,
                    Source::RanobeLib => self.ranobelib.chapters(reference).await,
                    Source::RanobeHub => self.ranobehub.chapters(reference).await,
                    Source::RoyalRoad => self.royalroad.chapters(reference).await,
                    Source::Kakuyomu => self.kakuyomu.chapters(reference).await,
                    _ => Err(qbutt_content::Error::InvalidReference),
                }?;
                Ok(SourceResponse::Chapters(chapters))
            }
            SourceOperation::Chapter(reference)
                if matches!(
                    reference.variant.publication.source,
                    Source::RanobeLib | Source::RanobeHub | Source::RoyalRoad | Source::Kakuyomu
                ) =>
            {
                self.text_chapter(reference).await?;
                Ok(SourceResponse::Chapter)
            }
            SourceOperation::Chapter(reference) | SourceOperation::ReaderPage { reference, .. } => {
                let pages = match reference.variant.publication.source {
                    Source::MangaDex => self.mangadex.chapter_pages(reference).await,
                    Source::MangaLib => self.mangalib.chapter_pages(reference).await,
                    _ => Err(qbutt_content::Error::InvalidReference),
                }?;
                if let SourceOperation::ReaderPage { page, .. } = operation {
                    let image = pages
                        .images
                        .get(*page as usize)
                        .ok_or(qbutt_content::Error::VariantUnavailable)?;
                    match reference.variant.publication.source {
                        Source::MangaDex => self.mangadex.image_bytes(image).await?,
                        Source::MangaLib => self.mangalib.image_bytes(image).await?,
                        _ => return Err(qbutt_content::Error::InvalidReference),
                    };
                    Ok(SourceResponse::ReaderPage)
                } else {
                    Ok(SourceResponse::Chapter)
                }
            }
        }
    }

    pub(super) async fn text_chapter(
        &self,
        reference: &ChapterRef,
    ) -> qbutt_content::Result<TextChapter> {
        match reference.variant.publication.source {
            Source::RanobeLib => self.ranobelib.chapter_text(reference).await,
            Source::RanobeHub => self.ranobehub.chapter_text(reference).await,
            Source::RoyalRoad => self.royalroad.chapter_text(reference).await,
            Source::Kakuyomu => self.kakuyomu.chapter_text(reference).await,
            _ => Err(qbutt_content::Error::InvalidReference),
        }
    }
}
