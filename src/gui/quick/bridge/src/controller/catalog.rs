//! Typed provider dispatch shared by search, inspection, playback and retries.

use std::sync::Arc;

use qbutt_content::{
    AniLiberty, Chapter, FitGirl, MangaDex, MangaLib, Nyaa, ResolvedContent, RuTor, Rutube, Source,
    SourceOperation, Work, YummyAnime,
};

pub enum SourceResponse {
    Search(Vec<Work>),
    Inspect(Work),
    Resolve(ResolvedContent),
    Chapters(Vec<Chapter>),
    Chapter,
    ReaderPage,
}

pub(super) struct Catalog {
    aniliberty: AniLiberty,
    yummyanime: YummyAnime,
    pub(super) mangadex: Arc<MangaDex>,
    pub(super) mangalib: Arc<MangaLib>,
    rutor: RuTor,
    nyaa: Nyaa,
    rutube: Rutube,
    fitgirl: FitGirl,
}

impl Catalog {
    pub(super) fn new() -> qbutt_content::Result<Self> {
        Ok(Self {
            aniliberty: AniLiberty::new()?,
            yummyanime: YummyAnime::new()?,
            mangadex: Arc::new(MangaDex::new()?),
            mangalib: Arc::new(MangaLib::new()?),
            rutor: RuTor::new()?,
            nyaa: Nyaa::new()?,
            rutube: Rutube::new()?,
            fitgirl: FitGirl::new()?,
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
                    Source::AniLiberty => self.aniliberty.search(query).await,
                    Source::YummyAnime => self.yummyanime.search(query).await,
                    Source::MangaDex => {
                        self.mangadex
                            .search(query, language.as_deref().unwrap())
                            .await
                    }
                    Source::MangaLib => self.mangalib.search(query).await,
                    Source::RuTor => self.rutor.search(query, *category).await,
                    Source::Nyaa => self.nyaa.search(query, *category).await,
                    Source::Rutube => self.rutube.search(query, *category).await,
                    Source::FitGirl => self.fitgirl.search(query, *category).await,
                }?;
                Ok(SourceResponse::Search(works))
            }
            SourceOperation::Inspect {
                reference,
                language,
            } => {
                let work = match reference.source {
                    Source::AniLiberty => self.aniliberty.inspect(reference).await,
                    Source::YummyAnime => self.yummyanime.inspect(reference).await,
                    Source::MangaDex => {
                        self.mangadex
                            .inspect(reference, language.as_deref().unwrap())
                            .await
                    }
                    Source::MangaLib => self.mangalib.inspect(reference).await,
                    Source::RuTor => self.rutor.inspect(reference).await,
                    Source::Nyaa => self.nyaa.inspect(reference).await,
                    Source::Rutube => self.rutube.inspect(reference).await,
                    Source::FitGirl => self.fitgirl.inspect(reference).await,
                }?;
                Ok(SourceResponse::Inspect(work))
            }
            SourceOperation::Resolve(reference) => {
                let resolved = match reference.publication.source {
                    Source::AniLiberty => self.aniliberty.resolve(reference).await,
                    Source::YummyAnime => self.yummyanime.resolve(reference).await,
                    Source::RuTor => self.rutor.resolve(reference).await,
                    Source::Nyaa => self.nyaa.resolve(reference).await,
                    Source::Rutube => self.rutube.resolve(reference).await,
                    Source::FitGirl => self.fitgirl.resolve(reference).await,
                    Source::MangaDex | Source::MangaLib => {
                        Err(qbutt_content::Error::InvalidReference)
                    }
                }?;
                Ok(SourceResponse::Resolve(resolved))
            }
            SourceOperation::Chapters(reference) => {
                let chapters = match reference.publication.source {
                    Source::MangaDex => self.mangadex.chapters(reference).await,
                    Source::MangaLib => self.mangalib.chapters(reference).await,
                    _ => Err(qbutt_content::Error::InvalidReference),
                }?;
                Ok(SourceResponse::Chapters(chapters))
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
}
