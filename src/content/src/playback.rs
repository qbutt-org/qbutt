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
    pub(crate) subtitles: Vec<PlaybackSubtitle>,
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
