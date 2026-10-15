//! Typed provider dispatch shared by search, inspection, playback and retries.

use std::path::PathBuf;
use std::sync::Arc;

use qbutt_content::{
    AniLiberty, AniTokyo, AnimeSpirit, ByRut, Chapter, ChapterRef, Dodi, Estar, FitGirl, FreeTP,
    Igruha, Kakuyomu, LibSession, LimeTorrents, MangaDex, MangaLib, Nhentai, NnmClub, Nyaa,
    OnlineFix, RanobeHub, RanobeLib, ResolvedContent, RoyalRoad, RuTor, RuTracker, Rutube, Source,
    SourceOperation, Syosetu, TextChapter, TheLastGame, VkVideo, WeebCentral, Work, X1337,
    YummyAnime,
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
    anitokyo: AniTokyo,
    yummyanime: YummyAnime,
    animespirit: AnimeSpirit,
    pub(super) mangadex: Arc<MangaDex>,
    pub(super) weebcentral: Arc<WeebCentral>,
    pub(super) nhentai: Arc<Nhentai>,
    pub(super) mangalib: Arc<MangaLib>,
    rutor: RuTor,
    rutracker: RuTracker,
    nnmclub: NnmClub,
    nyaa: Nyaa,
    rutube: Rutube,
    fitgirl: FitGirl,
    dodi: Dodi,
    online_fix: OnlineFix,
    freetp: FreeTP,
    byrut: ByRut,
    igruha: Igruha,
    thelastgame: TheLastGame,
    limetorrents: LimeTorrents,
    x1337: X1337,
    ranobelib: RanobeLib,
    ranobehub: RanobeHub,
    royalroad: RoyalRoad,
    kakuyomu: Kakuyomu,
    syosetu: Syosetu,
    estar: Estar,
}

impl Catalog {
    /// An explicit retry first borrows the selected browser's current session.
    /// Only a remaining access check may open the provider's existing WebView.
    pub(super) async fn retry(
        &self,
        operation: &SourceOperation,
    ) -> qbutt_content::Result<SourceResponse> {
        let result = self.execute(operation).await;
        if !matches!(
            result,
            Err(qbutt_content::Error::LoginRequired | qbutt_content::Error::SourceCheckRequired)
        ) {
            return result;
        }
        match operation.source() {
            Source::RuTracker => self.rutracker.complete_check().await?,
            Source::NnmClub => self.nnmclub.complete_check().await?,
            Source::X1337 => self.x1337.complete_check().await?,
            _ => return result,
        }
        self.execute(operation).await
    }

    pub(super) fn initialize_webview(&self, profile_dir: PathBuf, app_dir: PathBuf) {
        self.rutracker
            .initialize_webview(profile_dir.clone(), app_dir.clone());
        self.nnmclub
            .initialize_webview(profile_dir.clone(), app_dir.clone());
        self.x1337.initialize_webview(profile_dir, app_dir);
    }

    pub(super) fn close(&self) {
        self.rutracker.close();
        self.nnmclub.close();
        self.x1337.close();
    }

    pub(super) fn new(browser: BrowserClient) -> qbutt_content::Result<Self> {
        Ok(Self {
            browser,
            aniliberty: AniLiberty::new()?,
            anitokyo: AniTokyo::new()?,
            yummyanime: YummyAnime::new()?,
            animespirit: AnimeSpirit::new()?,
            mangadex: Arc::new(MangaDex::new()?),
            weebcentral: Arc::new(WeebCentral::new()?),
            nhentai: Arc::new(Nhentai::new()?),
            mangalib: Arc::new(MangaLib::new()?),
            rutor: RuTor::new()?,
            rutracker: RuTracker::new()?,
            nnmclub: NnmClub::new()?,
            nyaa: Nyaa::new()?,
            rutube: Rutube::new()?,
            fitgirl: FitGirl::new()?,
            dodi: Dodi::new()?,
            online_fix: OnlineFix::new()?,
            freetp: FreeTP::new()?,
            byrut: ByRut::new()?,
            igruha: Igruha::new()?,
            thelastgame: TheLastGame::new()?,
            limetorrents: LimeTorrents::new()?,
            x1337: X1337::new()?,
            ranobelib: RanobeLib::new()?,
            ranobehub: RanobeHub::new()?,
            royalroad: RoyalRoad::new()?,
            kakuyomu: Kakuyomu::new()?,
            syosetu: Syosetu::new()?,
            estar: Estar::new()?,
        })
    }

    pub(super) async fn execute(
        &self,
        operation: &SourceOperation,
    ) -> qbutt_content::Result<SourceResponse> {
        operation.validate()?;
        let choice = self.browser.choice();
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
                    Source::AniTokyo => self.anitokyo.search(query).await,
                    Source::YummyAnime => self.yummyanime.search(query).await,
                    Source::AnimeSpirit => self.animespirit.search(query).await,
                    Source::WeebCentral => self.weebcentral.search(query).await,
                    Source::Nhentai => self.nhentai.search(query).await,
                    Source::MangaDex => {
                        self.mangadex
                            .search(query, language.as_deref().unwrap())
                            .await
                    }
                    Source::MangaLib => {
                        self.mangalib
                            .search(query, &mut self.lib_session(*source).await?)
                            .await
                    }
                    Source::RuTor => self.rutor.search(query, *category).await,
                    Source::RuTracker => {
                        self.rutracker
                            .search(
                                query,
                                *category,
                                self.browser.existing_handle().await.as_ref(),
                                choice,
                            )
                            .await
                    }
                    Source::NnmClub => {
                        self.nnmclub
                            .search(
                                query,
                                *category,
                                self.browser.existing_handle().await.as_ref(),
                                choice,
                            )
                            .await
                    }
                    Source::Nyaa => self.nyaa.search(query, *category).await,
                    Source::Rutube => self.rutube.search(query, *category).await,
                    Source::VkVideo => {
                        let browser = self
                            .browser
                            .handle()
                            .await
                            .map_err(qbutt_content::Error::BrowserSession)?;
                        VkVideo.search(query, *category, &browser, choice).await
                    }
                    Source::FitGirl => self.fitgirl.search(query, *category).await,
                    Source::Dodi => self.dodi.search(query).await,
                    Source::OnlineFix => self.online_fix.search(query, *category).await,
                    Source::FreeTP => self.freetp.search(query, *category).await,
                    Source::ByRut => {
                        self.byrut
                            .search(
                                query,
                                *category,
                                self.browser.existing_handle().await.as_ref(),
                                choice,
                            )
                            .await
                    }
                    Source::Igruha => {
                        self.igruha
                            .search(
                                query,
                                *category,
                                self.browser.existing_handle().await.as_ref(),
                                choice,
                            )
                            .await
                    }
                    Source::TheLastGame => self.thelastgame.search(query, *category).await,
                    Source::LimeTorrents => self.limetorrents.search(query, *category).await,
                    Source::X1337 => {
                        self.x1337
                            .search(
                                query,
                                *category,
                                self.browser.existing_handle().await.as_ref(),
                                choice,
                            )
                            .await
                    }
                    Source::RanobeLib => {
                        self.ranobelib
                            .search(query, &mut self.lib_session(*source).await?)
                            .await
                    }
                    Source::RanobeHub => self.ranobehub.search(query).await,
                    Source::RoyalRoad => self.royalroad.search(query).await,
                    Source::Kakuyomu => self.kakuyomu.search(query).await,
                    Source::Syosetu => self.syosetu.search(query).await,
                    Source::Estar => self.estar.search(query).await,
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
                    Source::AniTokyo => self.anitokyo.inspect(reference).await,
                    Source::YummyAnime => self.yummyanime.inspect(reference).await,
                    Source::AnimeSpirit => self.animespirit.inspect(reference).await,
                    Source::WeebCentral => self.weebcentral.inspect(reference).await,
                    Source::Nhentai => self.nhentai.inspect(reference).await,
                    Source::MangaDex => {
                        self.mangadex
                            .inspect(reference, language.as_deref().unwrap())
                            .await
                    }
                    Source::MangaLib => {
                        self.mangalib
                            .inspect(reference, &mut self.lib_session(reference.source).await?)
                            .await
                    }
                    Source::RuTor => self.rutor.inspect(reference).await,
                    Source::RuTracker => {
                        self.rutracker
                            .inspect(
                                reference,
                                self.browser.existing_handle().await.as_ref(),
                                choice,
                            )
                            .await
                    }
                    Source::NnmClub => {
                        self.nnmclub
                            .inspect(
                                reference,
                                self.browser.existing_handle().await.as_ref(),
                                choice,
                            )
                            .await
                    }
                    Source::Nyaa => self.nyaa.inspect(reference).await,
                    Source::Rutube => self.rutube.inspect(reference).await,
                    Source::VkVideo => {
                        let browser = self
                            .browser
                            .handle()
                            .await
                            .map_err(qbutt_content::Error::BrowserSession)?;
                        VkVideo.inspect(reference, &browser, choice).await
                    }
                    Source::FitGirl => self.fitgirl.inspect(reference).await,
                    Source::Dodi => self.dodi.inspect(reference).await,
                    Source::OnlineFix => self.online_fix.inspect(reference).await,
                    Source::FreeTP => self.freetp.inspect(reference).await,
                    Source::ByRut => {
                        self.byrut
                            .inspect(
                                reference,
                                self.browser.existing_handle().await.as_ref(),
                                choice,
                            )
                            .await
                    }
                    Source::Igruha => {
                        self.igruha
                            .inspect(
                                reference,
                                self.browser.existing_handle().await.as_ref(),
                                choice,
                            )
                            .await
                    }
                    Source::TheLastGame => self.thelastgame.inspect(reference).await,
                    Source::LimeTorrents => self.limetorrents.inspect(reference).await,
                    Source::X1337 => {
                        self.x1337
                            .inspect(
                                reference,
                                self.browser.existing_handle().await.as_ref(),
                                choice,
                            )
                            .await
                    }
                    Source::RanobeLib => {
                        self.ranobelib
                            .inspect(reference, &mut self.lib_session(reference.source).await?)
                            .await
                    }
                    Source::RanobeHub => self.ranobehub.inspect(reference).await,
                    Source::RoyalRoad => self.royalroad.inspect(reference).await,
                    Source::Kakuyomu => self.kakuyomu.inspect(reference).await,
                    Source::Syosetu => self.syosetu.inspect(reference).await,
                    Source::Estar => self.estar.inspect(reference).await,
                }?;
                Ok(SourceResponse::Inspect(work))
            }
            SourceOperation::Resolve(reference) => {
                let resolved = match reference.publication.source {
                    Source::AniLiberty => self.aniliberty.resolve(reference).await,
                    Source::AniTokyo => self.anitokyo.resolve(reference).await,
                    Source::YummyAnime => self.yummyanime.resolve(reference).await,
                    Source::AnimeSpirit => self.animespirit.resolve(reference).await,
                    Source::RuTor => self.rutor.resolve(reference).await,
                    Source::RuTracker => {
                        self.rutracker
                            .resolve(
                                reference,
                                self.browser.existing_handle().await.as_ref(),
                                choice,
                            )
                            .await
                    }
                    Source::NnmClub => {
                        self.nnmclub
                            .resolve(
                                reference,
                                self.browser.existing_handle().await.as_ref(),
                                choice,
                            )
                            .await
                    }
                    Source::Nyaa => self.nyaa.resolve(reference).await,
                    Source::Rutube => self.rutube.resolve(reference).await,
                    Source::VkVideo => {
                        let browser = self
                            .browser
                            .handle()
                            .await
                            .map_err(qbutt_content::Error::BrowserSession)?;
                        VkVideo.resolve(reference, &browser, choice).await
                    }
                    Source::FitGirl => self.fitgirl.resolve(reference).await,
                    Source::Dodi => self.dodi.resolve(reference).await,
                    Source::OnlineFix => self.online_fix.resolve(reference).await,
                    Source::FreeTP => self.freetp.resolve(reference).await,
                    Source::ByRut => {
                        self.byrut
                            .resolve(
                                reference,
                                self.browser.existing_handle().await.as_ref(),
                                choice,
                            )
                            .await
                    }
                    Source::Igruha => {
                        self.igruha
                            .resolve(
                                reference,
                                self.browser.existing_handle().await.as_ref(),
                                choice,
                            )
                            .await
                    }
                    Source::TheLastGame => self.thelastgame.resolve(reference).await,
                    Source::LimeTorrents => self.limetorrents.resolve(reference).await,
                    Source::X1337 => {
                        self.x1337
                            .resolve(
                                reference,
                                self.browser.existing_handle().await.as_ref(),
                                choice,
                            )
                            .await
                    }
                    Source::MangaDex
                    | Source::WeebCentral
                    | Source::Nhentai
                    | Source::LocalBook
                    | Source::MangaLib
                    | Source::RanobeLib
                    | Source::RanobeHub
                    | Source::RoyalRoad
                    | Source::Kakuyomu
                    | Source::Syosetu
                    | Source::Estar => Err(qbutt_content::Error::InvalidReference),
                }?;
                Ok(SourceResponse::Resolve(resolved))
            }
            SourceOperation::Chapters(reference) => {
                let chapters = match reference.publication.source {
                    Source::MangaDex => self.mangadex.chapters(reference).await,
                    Source::WeebCentral => self.weebcentral.chapters(reference).await,
                    Source::Nhentai => self.nhentai.chapters(reference).await,
                    Source::MangaLib => {
                        self.mangalib
                            .chapters(
                                reference,
                                &mut self.lib_session(reference.publication.source).await?,
                            )
                            .await
                    }
                    Source::RanobeLib => {
                        self.ranobelib
                            .chapters(
                                reference,
                                &mut self.lib_session(reference.publication.source).await?,
                            )
                            .await
                    }
                    Source::RanobeHub => self.ranobehub.chapters(reference).await,
                    Source::RoyalRoad => self.royalroad.chapters(reference).await,
                    Source::Kakuyomu => self.kakuyomu.chapters(reference).await,
                    Source::Syosetu => self.syosetu.chapters(reference).await,
                    Source::Estar => self.estar.chapters(reference).await,
                    _ => Err(qbutt_content::Error::InvalidReference),
                }?;
                Ok(SourceResponse::Chapters(chapters))
            }
            SourceOperation::Chapter(reference)
                if matches!(
                    reference.variant.publication.source,
                    Source::RanobeLib
                        | Source::RanobeHub
                        | Source::RoyalRoad
                        | Source::Kakuyomu
                        | Source::Syosetu
                        | Source::Estar
                ) =>
            {
                self.text_chapter(reference).await?;
                Ok(SourceResponse::Chapter)
            }
            SourceOperation::Chapter(reference) | SourceOperation::ReaderPage { reference, .. } => {
                let mut source = self
                    .reader_source(reference.variant.publication.source)
                    .await?;
                let pages = source.chapter_pages(reference).await?;
                if let SourceOperation::ReaderPage { page, .. } = operation {
                    let image = pages
                        .images
                        .get(*page as usize)
                        .ok_or(qbutt_content::Error::VariantUnavailable)?;
                    source.remote_image_bytes(image).await?;
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
            Source::RanobeLib => {
                let mut session = self.lib_session(Source::RanobeLib).await?;
                self.ranobelib.chapter_text(reference, &mut session).await
            }
            Source::RanobeHub => self.ranobehub.chapter_text(reference).await,
            Source::RoyalRoad => self.royalroad.chapter_text(reference).await,
            Source::Kakuyomu => self.kakuyomu.chapter_text(reference).await,
            Source::Syosetu => self.syosetu.chapter_text(reference).await,
            Source::Estar => self.estar.chapter_text(reference).await,
            _ => Err(qbutt_content::Error::InvalidReference),
        }
    }

    pub(super) async fn lib_session(&self, source: Source) -> qbutt_content::Result<LibSession> {
        let choice = self.browser.choice();
        LibSession::from_browser(
            source,
            self.browser.existing_handle().await.as_ref(),
            choice,
        )
        .await
    }
}
