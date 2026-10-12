use std::collections::BTreeMap;

use url::Url;

#[cfg(windows)]
use crate::browser::ResolvedMedia;

/// Fresh browser acquisition data. Credentials and remote URLs stay inside the
/// native content layer; the gateway publishes only its local resource URLs.
#[derive(Clone, PartialEq, Eq)]
pub struct PlaybackMedia {
    pub(crate) url: Url,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) audio_url: Option<Url>,
    pub(crate) audio_track: Option<PlaybackAudioTrack>,
    pub(crate) video_height: Option<u16>,
    pub(crate) subtitles: Vec<PlaybackSubtitle>,
}

impl PlaybackMedia {
    pub fn direct(url: Url) -> Self {
        Self {
            url,
            headers: BTreeMap::new(),
            audio_url: None,
            audio_track: None,
            video_height: None,
            subtitles: Vec::new(),
        }
    }

    pub fn with_audio_url(mut self, url: Url) -> Self {
        self.audio_url = Some(url);
        self
    }

    pub fn with_audio_track(mut self, track: PlaybackAudioTrack) -> Self {
        self.audio_track = Some(track);
        self
    }

    pub fn with_video_height(mut self, height: u16) -> Self {
        self.video_height = Some(height);
        self
    }

    pub fn with_subtitle(mut self, title: String, url: Url, language: Option<String>) -> Self {
        self.subtitles.push(PlaybackSubtitle {
            title,
            url,
            language,
        });
        self
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct PlaybackAudioTrack {
    pub title: String,
    pub kind: Option<String>,
    pub language: Option<String>,
    pub codec: Option<String>,
    pub channels: Option<u16>,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct PlaybackSubtitle {
    pub title: String,
    pub(crate) url: Url,
    pub language: Option<String>,
}

#[cfg(windows)]
impl From<ResolvedMedia> for PlaybackMedia {
    fn from(media: ResolvedMedia) -> Self {
        let video_height = media.quality.as_deref().and_then(|quality| {
            quality
                .strip_suffix('p')
                .and_then(|height| height.parse::<u16>().ok())
                .or_else(|| match quality {
                    "4K" => Some(2160),
                    "2K" => Some(1440),
                    _ => None,
                })
        });
        Self {
            url: media.url,
            headers: media.headers,
            audio_url: media.audio_url,
            audio_track: media.audio_track.map(|track| PlaybackAudioTrack {
                title: track.title,
                kind: track.kind,
                language: track.language,
                codec: track.codec,
                channels: track.channels,
            }),
            video_height,
            subtitles: media
                .subtitles
                .into_iter()
                .map(|subtitle| PlaybackSubtitle {
                    title: subtitle.title,
                    url: subtitle.url,
                    language: subtitle.language,
                })
                .collect(),
        }
    }
}
