//! Public AnimeSpirit catalog. The site serves Windows-1251 despite its UTF-8
//! HTML meta tag; search parameters must use the same encoding.

use std::collections::{BTreeMap, HashSet};

use encoding_rs::WINDOWS_1251;
use percent_encoding::{NON_ALPHANUMERIC, percent_encode};
use scraper::{ElementRef, Html, Selector};
use url::Url;

use crate::http::{SourceHttp, SourceSession, validate_https_url};
use crate::kodik::{self, KodikVideo};
use crate::{
    Category, Error, PublicationRef, ResolvedContent, Result, Source, Transport, Variant,
    VariantRef, VariantSelector, Work,
};

const ORIGIN: &str = "https://my.animespirit.ru";

pub struct AnimeSpirit {
    http: SourceHttp,
    origin: Url,
}

struct Player {
    voice: String,
    url: Url,
    kind: String,
    id: u64,
}

impl AnimeSpirit {
    pub fn new() -> Result<Self> {
        let origin = Url::parse(ORIGIN).expect("fixed AnimeSpirit origin");
        Ok(Self {
            http: SourceHttp::new(origin.clone(), reqwest::header::HeaderMap::new())?,
            origin,
        })
    }

    pub async fn search(&self, query: &str) -> Result<Vec<Work>> {
        let query = query.trim();
        if query.is_empty() || query.chars().count() > 500 || query.chars().any(char::is_control) {
            return Err(Error::InvalidQuery);
        }
        let (bytes, _, invalid) = WINDOWS_1251.encode(query);
        if invalid {
            return Err(Error::InvalidQuery);
        }
        let encoded = percent_encode(&bytes, NON_ALPHANUMERIC);
        let url = Url::parse(&format!(
            "{ORIGIN}/index.php?do=search&subaction=search&story={encoded}"
        ))
        .expect("fixed search URL and percent-encoded query");
        let document = self.document(url).await?;
        let cards = selector("#dle-content > .content-block");
        let mut works = Vec::new();
        for card in document.select(&cards) {
            if let Some(work) = self.work(&card, true)? {
                works.push(work);
            }
        }
        if works.len() > 50 {
            return Err(Error::ResponseTooLarge);
        }
        Ok(works)
    }

    pub async fn inspect(&self, publication: &PublicationRef) -> Result<Work> {
        let (document, mut work) = self.publication(publication).await?;
        let players = players(&document)?;
        let mut variants = Vec::new();
        let mut seen = HashSet::new();
        for player in players {
            let catalog = kodik::inspect(&player.url).await?;
            for episode in catalog.episodes {
                let reference = VariantRef {
                    publication: publication.clone(),
                    selector: VariantSelector::AnimeSpiritKodik {
                        player_kind: player.kind.clone(),
                        player_id: player.id,
                        episode: episode.number.clone(),
                        episode_id: episode.id,
                        episode_hash: episode.hash,
                        voice: player.voice.clone(),
                        kodik_translation: catalog.translation.clone(),
                        subtitled: catalog.subtitled,
                    },
                };
                reference.validate()?;
                if seen.insert(reference.clone()) {
                    variants.push(Variant {
                        reference,
                        title: None,
                        transport: Transport::Video,
                        episode: Some(episode.number),
                        quality: None,
                        translation: Some(player.voice.clone()),
                        languages: if catalog.subtitled {
                            vec!["ja".into(), "ru".into()]
                        } else {
                            vec!["ru".into()]
                        },
                        player: Some("Kodik".into()),
                        size: None,
                        seeders: None,
                        info_hash: None,
                        reported_video_ads: false,
                    });
                }
            }
        }
        work.variants = variants;
        work.needs_inspection = false;
        Ok(work)
    }

    pub async fn resolve(&self, selected: &VariantRef) -> Result<ResolvedContent> {
        selected.validate()?;
        let VariantSelector::AnimeSpiritKodik {
            player_kind,
            player_id,
            episode,
            episode_id,
            episode_hash,
            voice,
            kodik_translation,
            subtitled,
        } = &selected.selector
        else {
            return Err(Error::InvalidReference);
        };
        let (document, work) = self.publication(&selected.publication).await?;
        let mut matching = players(&document)?.into_iter().filter(|player| {
            player.kind == *player_kind && player.id == *player_id && player.voice == *voice
        });
        let mut player = matching.next().ok_or(Error::VariantUnavailable)?;
        if matching.next().is_some() {
            return Err(Error::InvalidResponse("ambiguous AnimeSpirit player"));
        }
        player.url.query_pairs_mut().append_pair("episode", episode);
        let (url, quality) = kodik::resolve(
            KodikVideo {
                episode,
                dubbing: kodik_translation,
                subtitled: *subtitled,
                video: Some((episode_id, episode_hash)),
            },
            &player.url,
        )
        .await?;
        Ok(ResolvedContent::DirectVideo {
            reference: selected.clone(),
            title: work.title,
            url,
            quality,
        })
    }

    async fn publication(&self, publication: &PublicationRef) -> Result<(Html, Work)> {
        let id = publication.animespirit_id()?;
        let url = self
            .origin
            .join(&format!("index.php?newsid={id}"))
            .expect("numeric AnimeSpirit publication URL");
        let document = self.document(url).await?;
        let card = document
            .select(&selector("#dle-content > .content-block"))
            .next()
            .ok_or(Error::VariantUnavailable)?;
        let work = self.work(&card, false)?.ok_or(Error::VariantUnavailable)?;
        if work.reference != *publication {
            return Err(Error::VariantUnavailable);
        }
        Ok((document, work))
    }

    async fn document(&self, url: Url) -> Result<Html> {
        let mut session = SourceSession::anonymous(self.origin.clone())?;
        let response = self.http.session_document(url, &mut session).await?;
        if response.status != 200 {
            return Err(Error::HttpStatus(response.status));
        }
        if !response
            .content_type
            .as_deref()
            .and_then(|value| value.split(';').next())
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/html"))
        {
            return Err(Error::InvalidResponse("AnimeSpirit response is not HTML"));
        }
        let (body, invalid) = WINDOWS_1251.decode_without_bom_handling(&response.body);
        if invalid {
            return Err(Error::InvalidResponse("invalid AnimeSpirit encoding"));
        }
        if body.contains("/cdn-cgi/challenge-platform/") || body.contains("_cf_chl_opt") {
            return Err(Error::Challenge);
        }
        let document = Html::parse_document(&body);
        if document.select(&selector("#dle-content")).next().is_none() {
            return Err(Error::InvalidResponse("missing AnimeSpirit catalog"));
        }
        Ok(document)
    }

    fn work(&self, card: &ElementRef<'_>, needs_inspection: bool) -> Result<Option<Work>> {
        let Some(link) = card
            .select(&selector(".content-block-title h2 a[href]"))
            .next()
        else {
            return Ok(None);
        };
        let url = self
            .origin
            .join(link.value().attr("href").unwrap_or_default())
            .map_err(|_| Error::UnsafeAddress)?;
        validate_https_url(&url)?;
        if url.origin() != self.origin.origin()
            || !(url.path().starts_with("/anime/")
                || url.path().starts_with("/dunhua-manhva-amerime/"))
        {
            return Ok(None);
        }
        let Some(id) = article_id(&url) else {
            return Ok(None);
        };
        let title = link.text().collect::<String>().trim().to_owned();
        if !label(&title, 2048) {
            return Err(Error::InvalidResponse("invalid AnimeSpirit title"));
        }
        let alias = card
            .select(&selector("h3"))
            .next()
            .map(|heading| heading.text().collect::<String>().trim().to_owned())
            .filter(|alias| alias != &title && label(alias, 2048));
        let cover = card
            .select(&selector("a.highslide img[src]"))
            .next()
            .and_then(|image| image.value().attr("src"))
            .map(|src| self.origin.join(src).map_err(|_| Error::UnsafeAddress))
            .transpose()?;
        if let Some(cover) = &cover {
            validate_https_url(cover)?;
            if cover.origin() != self.origin.origin() || !cover.path().starts_with("/uploads/") {
                return Err(Error::UnsafeAddress);
            }
        }
        let year = card
            .select(&selector("a[href*='/tags/']"))
            .filter_map(|link| link.text().collect::<String>().parse::<u16>().ok())
            .find(|year| (1900..=2100).contains(year));
        Ok(Some(Work {
            reference: PublicationRef {
                source: Source::AnimeSpirit,
                id: id.to_string(),
            },
            title,
            category: Category::Anime,
            url,
            cover,
            description: None,
            year,
            aliases: alias.into_iter().collect(),
            identifiers: BTreeMap::new(),
            variants: Vec::new(),
            needs_inspection,
        }))
    }
}

fn players(document: &Html) -> Result<Vec<Player>> {
    let mut players = Vec::new();
    for heading in document.select(&selector("#dle-content h3[id^='top_div_']")) {
        let Some(index) = heading
            .value()
            .attr("id")
            .and_then(|id| id.strip_prefix("top_div_"))
        else {
            continue;
        };
        if index.is_empty() || !index.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let voice = heading
            .text()
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if !label(&voice, 256) {
            return Err(Error::InvalidResponse("invalid AnimeSpirit voice"));
        }
        let Some(address) = document
            .select(&selector(&format!("#an_ul{index}")))
            .next()
            .map(|element| element.text().collect::<String>())
        else {
            continue;
        };
        let address = address.trim();
        if !address.starts_with("//kodikplayer.com/") {
            continue;
        }
        let url = Url::parse(&format!("https:{address}"))
            .map_err(|_| Error::InvalidResponse("invalid AnimeSpirit embed"))?;
        validate_https_url(&url)?;
        if url.host_str() != Some("kodikplayer.com") || url.port().is_some() {
            return Err(Error::UnsafeAddress);
        }
        let parts = url
            .path_segments()
            .ok_or(Error::UnsafeAddress)?
            .collect::<Vec<_>>();
        if !matches!(parts.first().copied(), Some("serial" | "season")) {
            continue;
        }
        if parts.len() != 4
            || parts[3] != "720p"
            || parts[2].len() != 32
            || !parts[2]
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(Error::InvalidResponse("invalid AnimeSpirit Kodik identity"));
        }
        let id = parts[1]
            .parse::<u64>()
            .ok()
            .filter(|id| *id != 0 && parts[1] == id.to_string())
            .ok_or(Error::InvalidResponse("invalid AnimeSpirit Kodik ID"))?;
        if url.query_pairs().any(|(name, _)| name == "episode") {
            return Err(Error::InvalidResponse(
                "unexpected AnimeSpirit episode override",
            ));
        }
        let kind = parts[0].to_owned();
        players.push(Player {
            voice,
            url,
            kind,
            id,
        });
    }
    if players.len() > 32 {
        return Err(Error::ResponseTooLarge);
    }
    Ok(players)
}

fn article_id(url: &Url) -> Option<u64> {
    let filename = url.path_segments()?.next_back()?;
    let (id, _) = filename.strip_suffix(".html")?.split_once('-')?;
    id.parse::<u64>()
        .ok()
        .filter(|number| *number != 0 && id == number.to_string())
}

fn selector(value: &str) -> Selector {
    Selector::parse(value).expect("fixed AnimeSpirit selector")
}

fn label(value: &str, max_bytes: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max_bytes && !value.chars().any(char::is_control)
}
