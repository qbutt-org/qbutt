mod response;

use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::lib_api::LibHttp;
use crate::{
    Chapter, ChapterImage, ChapterPages, ChapterRef, Error, LibSession, PublicationRef, Result,
    Source, Transport, Variant, VariantRef, VariantSelector, Work,
};
use response::{ChapterRecord, ChapterResponse, Constants, MangaRecord, Response};

const MAX_CHAPTERS: usize = 10_000;

/// MangaLib's public catalog and exact translation branches. The caller owns
/// the runtime, bounded concurrency and cancellation of each operation.
pub struct MangaLib {
    http: LibHttp,
}

impl MangaLib {
    pub fn new() -> Result<Self> {
        Ok(Self {
            http: LibHttp::new(Source::MangaLib)?,
        })
    }

    pub async fn search(&self, query: &str, session: &mut LibSession) -> Result<Vec<Work>> {
        let query = query.trim();
        if query.is_empty() || query.chars().count() > 500 || query.chars().any(char::is_control) {
            return Err(Error::InvalidQuery);
        }
        let mut url = self.http.endpoint("/api/manga");
        url.query_pairs_mut()
            .append_pair("site_id[0]", "1")
            .append_pair("q", query);
        let listing: Response<Vec<MangaRecord>> = self.http.json(url, session).await?;
        if listing.data.len() > 60 {
            return Err(Error::InvalidResponse("oversized MangaLib search"));
        }
        let mut seen = HashSet::new();
        listing
            .data
            .into_iter()
            .map(|record| {
                let work = record.into_work(true)?;
                if !seen.insert(work.reference.clone()) {
                    return Err(Error::InvalidResponse("duplicate MangaLib publication"));
                }
                Ok(work)
            })
            .collect()
    }

    pub async fn inspect(
        &self,
        publication: &PublicationRef,
        session: &mut LibSession,
    ) -> Result<Work> {
        publication.mangalib_id()?;
        let mut url = self
            .http
            .endpoint(&format!("/api/manga/{}", publication.id));
        for field in ["summary", "otherNames"] {
            url.query_pairs_mut().append_pair("fields[]", field);
        }
        let metadata = self
            .http
            .json::<Response<MangaRecord>>(url, session)
            .await?;
        let chapters = self.listing(publication, session).await?;
        let mut work = metadata.data.into_work(false)?;
        if work.reference != *publication {
            return Err(Error::InvalidResponse(
                "MangaLib publication identity changed",
            ));
        }
        let mut translations = BTreeMap::<Option<u64>, BTreeSet<String>>::new();
        for chapter in chapters {
            for branch in chapter.branches {
                let teams = translations.entry(branch.branch_id).or_default();
                for team in branch.teams {
                    teams.insert(team.name);
                }
            }
        }
        work.variants = translations
            .into_iter()
            .map(|(branch, teams)| Variant {
                reference: VariantRef {
                    publication: publication.clone(),
                    selector: VariantSelector::LibTranslation { branch },
                },
                title: None,
                transport: Transport::Manga,
                episode: None,
                quality: None,
                translation: (!teams.is_empty())
                    .then(|| teams.into_iter().collect::<Vec<_>>().join(", ")),
                languages: Vec::new(),
                player: None,
                size: None,
                seeders: None,
                info_hash: None,
                reported_video_ads: false,
            })
            .collect();
        Ok(work)
    }

    pub async fn chapters(
        &self,
        selected: &VariantRef,
        session: &mut LibSession,
    ) -> Result<Vec<Chapter>> {
        selected.validate()?;
        let VariantSelector::LibTranslation { branch } = selected.selector else {
            return Err(Error::InvalidReference);
        };
        let mut chapters = Vec::new();
        for record in self.listing(&selected.publication, session).await? {
            if let Some(upload) = record
                .branches
                .iter()
                .find(|upload| upload.branch_id == branch)
            {
                chapters.push(Chapter {
                    reference: ChapterRef {
                        variant: selected.clone(),
                        id: upload.id.to_string(),
                    },
                    title: record.name.filter(|title| !title.trim().is_empty()),
                    volume: Some(record.volume),
                    number: Some(record.number),
                    // The listing does not report page counts. The exact manifest
                    // supplies this when the chapter is opened.
                    pages: 0,
                });
            }
        }
        if chapters.is_empty() {
            return Err(Error::VariantUnavailable);
        }
        Ok(chapters)
    }

    pub async fn chapter_pages(
        &self,
        selected: &ChapterRef,
        session: &mut LibSession,
    ) -> Result<ChapterPages> {
        selected.validate()?;
        let VariantSelector::LibTranslation { branch } = selected.variant.selector else {
            return Err(Error::InvalidReference);
        };
        let chapters = self.listing(&selected.variant.publication, session).await?;
        let record = chapters
            .iter()
            .find(|chapter| {
                chapter.branches.iter().any(|upload| {
                    upload.id.to_string() == selected.id && upload.branch_id == branch
                })
            })
            .ok_or(Error::VariantUnavailable)?;
        let mut url = self.http.endpoint(&format!(
            "/api/manga/{}/chapter",
            selected.variant.publication.id
        ));
        url.query_pairs_mut()
            .append_pair("branch_id", &branch.unwrap_or(0).to_string())
            .append_pair("number", &record.number)
            .append_pair("volume", &record.volume);
        let chapter: Response<ChapterResponse> = self.http.json(url, session).await?;
        chapter.data.validate(selected, record)?;
        let mut url = self.http.endpoint("/api/constants");
        url.query_pairs_mut()
            .append_pair("fields[]", "imageServers");
        let constants: Response<Constants> = self.http.json(url, session).await?;
        let server = constants.data.download_server()?;
        chapter.data.into_pages(selected.clone(), &server)
    }

    pub async fn image_bytes(
        &self,
        image: &ChapterImage,
        session: &LibSession,
    ) -> Result<Vec<u8>> {
        response::validate_image_url(&image.url)?;
        self.http
            .image(image.url.clone(), response::IMAGE_HOSTS, session)
            .await
    }

    async fn listing(
        &self,
        publication: &PublicationRef,
        session: &mut LibSession,
    ) -> Result<Vec<ChapterRecord>> {
        publication.mangalib_id()?;
        let url = self
            .http
            .endpoint(&format!("/api/manga/{}/chapters", publication.id));
        let listing: Response<Vec<ChapterRecord>> = self.http.json(url, session).await?;
        if listing.data.len() > MAX_CHAPTERS {
            return Err(Error::InvalidResponse("too many MangaLib chapters"));
        }
        let mut seen = HashSet::new();
        let mut ordering = HashSet::new();
        for record in &listing.data {
            record.validate()?;
            if !ordering.insert(record.index) {
                return Err(Error::InvalidResponse("duplicate chapter order"));
            }
            for branch in &record.branches {
                if !seen.insert(branch.id) {
                    return Err(Error::InvalidResponse("duplicate chapter upload"));
                }
            }
        }
        let mut chapters = listing.data;
        chapters.sort_by_key(|chapter| chapter.index);
        Ok(chapters)
    }
}
