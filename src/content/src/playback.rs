use std::collections::BTreeMap;

use url::Url;

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
