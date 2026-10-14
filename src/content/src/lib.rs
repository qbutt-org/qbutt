//! Native content services, independent of Qt and the application runtime.
//!
//! Provider operations borrow their service and do not spawn tasks. The caller
//! owns the async runtime, request generation and cancellation: drop an operation's
//! future to stop awaiting its result. Apply completed results to Qt models only on
//! the GUI thread and only while their generation remains current.
//! Library storage is synchronous and must be scheduled outside the GUI thread.

mod aniliberty;
mod animespirit;
mod anitokyo;
mod association;
#[cfg(windows)]
pub mod browser;
mod byrut;
#[cfg(windows)]
mod captured_video;
mod dodi;
mod download;
mod error;
mod estar;
#[cfg(windows)]
mod external_video;
mod fitgirl;
mod freetp;
mod http;
mod igruha;
mod kakuyomu;
mod kodik;
mod lib_api;
mod library;
mod limetorrents;
mod local_books;
mod mangadex;
mod mangalib;
mod media;
mod model;
#[cfg(windows)]
mod nnmclub;
mod nyaa;
mod online_fix;
mod ranobehub;
mod ranobelib;
mod royalroad;
mod rutor;
#[cfg(windows)]
mod rutracker;
mod rutube;
mod secrets;
mod sharing;
mod sources;
mod syosetu;
mod text;
mod thelastgame;
mod torrent;
pub mod translation;
#[cfg(windows)]
mod vkvideo;
#[cfg(windows)]
pub mod webview;
mod weebcentral;
mod x1337;
mod yummyanime;

pub use aniliberty::AniLiberty;
pub use animespirit::AnimeSpirit;
pub use anitokyo::AniTokyo;
pub use association::{WorkAssociation, WorkGroup, rank_variants};
pub use byrut::ByRut;
#[cfg(windows)]
pub use captured_video::{CapturedVideoError, resolve_captured_video};
pub use dodi::Dodi;
pub use download::{
    DirectDownload, DownloadError, DownloadJob, DownloadOrigin, DownloadQueue, DownloadStatus,
    DownloadStorage, MediaTools,
};
pub use error::{Error, Result};
pub use estar::Estar;
#[cfg(windows)]
pub use external_video::{ExternalVideoError, resolve_external_video};
pub use fitgirl::FitGirl;
pub use freetp::FreeTP;
pub use igruha::Igruha;
pub use kakuyomu::Kakuyomu;
pub use lib_api::LibSession;
pub use library::{
    LastOpened, Library, LibraryEntry, LibraryError, LibraryResult, LibraryView,
    OpenRouterSettings, ReaderFit, ReaderMode, ReaderPosition, ReaderProgress, ReaderSettings,
    TextTranslationService, TextTranslationSettings, TranslationCacheKey, TranslationSelection,
    VariantFilters, VariantPreferences,
};
pub use limetorrents::LimeTorrents;
pub use local_books::{LocalBook, LocalBookChapter, LocalBookFormat};
pub use mangadex::MangaDex;
pub use mangalib::MangaLib;
pub use media::{MediaGateway, MediaGatewayError, PreparedPlayback, PreparedSubtitle};
pub use model::*;
#[cfg(windows)]
pub use nnmclub::NnmClub;
pub use nyaa::Nyaa;
pub use online_fix::OnlineFix;
pub use ranobehub::RanobeHub;
pub use ranobelib::RanobeLib;
pub use royalroad::RoyalRoad;
pub use rutor::RuTor;
#[cfg(windows)]
pub use rutracker::RuTracker;
pub use rutube::Rutube;
pub use sharing::{ShareError, SharedWork};
pub use sources::{SourceFailure, SourceFailureReason, SourceOperation, SourceSettings};
pub use syosetu::Syosetu;
pub use text::{TextBlock, TextBlockKind, TextChapter, TextChapterCache, TextImageAsset, TextSpan};
pub use thelastgame::TheLastGame;
#[cfg(windows)]
pub use vkvideo::VkVideo;
pub use weebcentral::WeebCentral;
pub use x1337::X1337;
pub use yummyanime::YummyAnime;
