//! VK Video publications found through qcatch's selected browser session.

use std::collections::BTreeMap;

use url::Url;

use crate::browser::{
    BrowserChoice, BrowserCommand, BrowserError, BrowserHandle, BrowserReply, BrowserResolver,
    ResolveKind, Selection, VkSearchVideo,
};
use crate::{
    Category, Error, PlaybackMedia, PublicationRef, ResolvedContent, Result, Source, Transport,
    Variant, VariantRef, VariantSelector, Work,
};

pub struct VkVideo;

impl VkVideo {
    pub async fn search(
        &self,
        query: &str,
        category: Category,
        browser: &BrowserHandle,
        choice: BrowserChoice,
    ) -> Result<Vec<Work>> {
        if category != Category::Video {
            return Ok(Vec::new());
        }
        if query.is_empty()
            || query.trim() != query
            || query.encode_utf16().count() > 200
            || query.chars().any(char::is_control)
        {
            return Err(Error::InvalidQuery);
        }

        let command = BrowserCommand::vk_search(query.to_owned()).map_err(browser_error)?;
        let reply = browser
            .request(choice, command)
            .await
            .map_err(browser_error)?;
        let BrowserReply::VkSearch(reply) = reply else {
            return Err(Error::InvalidResponse(
                "VK search returned another result type",
            ));
        };

        reply
            .videos
            .into_iter()
            .filter(|video| matches_query(&video.title, query))
            .map(video_work)
            .collect()
    }

    pub async fn inspect(
        &self,
        publication: &PublicationRef,
        browser: &BrowserHandle,
        choice: BrowserChoice,
    ) -> Result<Work> {
        let (title, _) = self.video(publication, browser, choice).await?;
        Ok(work(publication.clone(), title))
    }

    pub async fn resolve(
        &self,
        selected: &VariantRef,
        browser: &BrowserHandle,
        choice: BrowserChoice,
    ) -> Result<ResolvedContent> {
        selected.validate()?;
        let (title, media) = self.video(&selected.publication, browser, choice).await?;
        Ok(ResolvedContent::BrowserVideo {
            reference: selected.clone(),
            title,
            media,
        })
    }

    async fn video(
        &self,
        publication: &PublicationRef,
        browser: &BrowserHandle,
        choice: BrowserChoice,
    ) -> Result<(String, PlaybackMedia)> {
        let id = publication.vkvideo_id()?;
        let url = video_url(id);
        let reply = browser
            .request(
                choice,
                BrowserCommand::Resolve {
                    url: url.clone(),
                    selection: Selection::default(),
                    kind: ResolveKind::Video,
                    referrer: None,
                },
            )
            .await
            .map_err(browser_error)?;
        let BrowserReply::Resolved(media) = reply else {
            return Err(Error::InvalidResponse(
                "VK inspection returned another result type",
            ));
        };
        if !matches!(media.resolver, BrowserResolver::Direct)
            || media.source_url.as_ref() != Some(&url)
        {
            return Err(Error::VariantUnavailable);
        }
        let title = media
            .title
            .as_deref()
            .filter(|title| !title.trim().is_empty())
            .ok_or(Error::InvalidResponse("VK video has no title"))?;
        let title = title.to_owned();
        Ok((title, (*media).into()))
    }
}

fn video_work(video: VkSearchVideo) -> Result<Work> {
    let publication = PublicationRef {
        source: Source::VkVideo,
        id: video.id,
    };
    publication
        .validate()
        .map_err(|_| Error::InvalidResponse("invalid VK publication identity"))?;
    Ok(work(publication, video.title))
}

fn work(reference: PublicationRef, title: String) -> Work {
    let variant = Variant {
        reference: VariantRef {
            publication: reference.clone(),
            selector: VariantSelector::VkVideo,
        },
        title: Some(title.clone()),
        transport: Transport::Video,
        episode: None,
        quality: None,
        translation: None,
        languages: Vec::new(),
        player: None,
        size: None,
        seeders: None,
        info_hash: None,
        reported_video_ads: false,
    };
    let url = video_url(&reference.id);
    // A qualifier anywhere in the upload label applies to the whole
    // publication, including a plain original name after the pipe.
    let aliases = title
        .split_once(" | ")
        .filter(|_| !is_promotional(&title.to_lowercase()))
        .map(|(_, original)| vec![original.to_owned()])
        .unwrap_or_default();
    Work {
        reference,
        title,
        category: Category::Video,
        url,
        cover: None,
        description: None,
        year: None,
        aliases,
        identifiers: BTreeMap::new(),
        variants: vec![variant],
        notes: Vec::new(),
        needs_inspection: false,
    }
}

fn video_url(id: &str) -> Url {
    Url::parse(&format!("https://vkvideo.ru/video{id}")).expect("validated VK identity")
}

fn matches_query(title: &str, query: &str) -> bool {
    let normalize = |value: &str| value.to_lowercase().replace('ё', "е");
    let title = normalize(title);
    let query = normalize(query);

    if is_promotional(&title) && !is_promotional(&query) {
        return false;
    }

    query
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .all(|word| title.contains(word))
}

fn is_promotional(value: &str) -> bool {
    const ENGLISH: &[&str] = &[
        "trailer",
        "teaser",
        "reaction",
        "review",
        "amv",
        "highlights",
        "promo",
        "preview",
        "clip",
        "clips",
        "excerpt",
        "excerpts",
    ];
    const RUSSIAN: &[&str] = &[
        "трейлер",
        "тизер",
        "реакци",
        "обзор",
        "нарезк",
        "промо",
        "превью",
        "отрывок",
        "отрывк",
        "фрагмент",
    ];
    value
        .split(|character: char| !character.is_alphanumeric())
        .any(|word| {
            ENGLISH.contains(&word) || RUSSIAN.iter().any(|prefix| word.starts_with(prefix))
        })
}

fn browser_error(error: BrowserError) -> Error {
    match error {
        BrowserError::LoginRequired => Error::LoginRequired,
        BrowserError::ChoiceRequired => Error::BrowserChoiceRequired,
        BrowserError::BrowserCheckRequired | BrowserError::VerificationRequired => Error::Challenge,
        BrowserError::Timeout => Error::Timeout,
        error => Error::BrowserSession(error),
    }
}
