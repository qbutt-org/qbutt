use std::collections::{HashMap, VecDeque};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QString, QUrl, QVector};
use qbutt_content::browser::BrowserChoice;
use qbutt_content::{
    Category, MediaGateway, PreparedPlayback, ResolvedContent, SourceOperation, Transport,
    VariantPreferences, VariantRef, VariantSelector, Work, WorkGroup, resolve_external_video,
};

use super::catalog::SourceResponse;
use super::library::LibraryAction;
use super::{Services, ffi};
use crate::models;

static NEXT_LAUNCH: AtomicU64 = AtomicU64::new(1);
const MAX_PLAYBACK_INTENTS: usize = 64;

#[derive(Clone)]
pub struct PlaybackSave {
    pub group: WorkGroup,
    pub variant: VariantRef,
    pub preferences: VariantPreferences,
}

pub(super) struct PlaybackLaunch {
    pub gateway: MediaGateway,
    save: Option<PlaybackSave>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SaveState {
    Ready,
    Pending,
    Failed,
}

struct ConfirmedSave {
    token: u64,
    save: PlaybackSave,
    state: SaveState,
}

#[derive(Default)]
pub(super) struct PlaybackState {
    pub launches: HashMap<u64, PlaybackLaunch>,
    saves: VecDeque<ConfirmedSave>,
}

impl PlaybackState {
    pub(super) fn failed(&self) -> bool {
        self.saves
            .front()
            .is_some_and(|save| save.state == SaveState::Failed)
    }
}

impl Services {
    pub(super) fn retire_gateway(&mut self, gateway: MediaGateway) {
        gateway.retire();
        self.retired_gateways.retain(|task| !task.is_finished());
        self.retired_gateways
            .push(self.runtime.spawn(async move { gateway.close().await }));
    }
}

impl Drop for Services {
    fn drop(&mut self) {
        // Services is retired onto Qt's global pool before destruction. Drain
        // gateway connections there, alongside accepted SQLite work.
        self.runtime.block_on(async {
            self.browser.shutdown().await;
            for task in self.retired_gateways.drain(..) {
                let _ = task.await;
            }
            for task in self.retired_downloads.drain(..) {
                let _ = task.await;
            }
        });
    }
}

impl ffi::ContentController {
    pub(super) fn accept_video(
        mut self: Pin<&mut Self>,
        gateway: MediaGateway,
        save: PlaybackSave,
        url: QUrl,
        title: QString,
        prepared: Option<PreparedPlayback>,
    ) {
        if !self.playback_capacity() {
            self.as_mut()
                .rust_mut()
                .services
                .as_mut()
                .unwrap()
                .retire_gateway(gateway);
            self.set_error(Some(ffi::ContentError::PlaybackPending));
            return;
        }
        let token = NEXT_LAUNCH.fetch_add(1, Ordering::Relaxed);
        let identity = QString::from(format!("video:{}", models::variant_id(&save.variant)));
        let content_kind = QString::from(
            match save.group.anchor_work().map(|work| work.category).unwrap() {
                Category::Movies => "movie",
                category => models::category_key(category),
            },
        );
        let proxy = if prepared.is_some() {
            QUrl::default()
        } else {
            QUrl::from(&QString::from(gateway.proxy_url().as_str()))
        };
        let (
            audio_url,
            audio_title,
            audio_type,
            audio_language,
            audio_codec,
            audio_channels,
            subtitles,
        ) = if let Some(prepared) = prepared {
            let track = prepared.audio_track;
            let subtitles: QVector<QUrl> = prepared
                .subtitles
                .into_iter()
                .map(|subtitle| QUrl::from(&QString::from(subtitle.url.as_str())))
                .collect();
            (
                prepared.audio_url.map_or_else(QUrl::default, |url| {
                    QUrl::from(&QString::from(url.as_str()))
                }),
                QString::from(track.as_ref().map_or("", |track| track.title.as_str())),
                QString::from(
                    track
                        .as_ref()
                        .and_then(|track| track.kind.as_deref())
                        .unwrap_or_default(),
                ),
                QString::from(
                    track
                        .as_ref()
                        .and_then(|track| track.language.as_deref())
                        .unwrap_or_default(),
                ),
                QString::from(
                    track
                        .as_ref()
                        .and_then(|track| track.codec.as_deref())
                        .unwrap_or_default(),
                ),
                i32::from(
                    track
                        .as_ref()
                        .and_then(|track| track.channels)
                        .unwrap_or_default(),
                ),
                subtitles,
            )
        } else {
            (
                QUrl::default(),
                QString::default(),
                QString::default(),
                QString::default(),
                QString::default(),
                0,
                QVector::default(),
            )
        };
        self.as_mut().rust_mut().playback.launches.insert(
            token,
            PlaybackLaunch {
                gateway,
                save: Some(save),
            },
        );
        self.video_ready(
            token,
            identity,
            url,
            proxy,
            title,
            content_kind,
            audio_url,
            audio_title,
            audio_type,
            audio_language,
            audio_codec,
            audio_channels,
            subtitles,
        );
    }

    pub(super) fn playback_capacity(&self) -> bool {
        let retiring = self.services.as_ref().map_or(0, |services| {
            services
                .retired_gateways
                .iter()
                .filter(|task| !task.is_finished())
                .count()
        });
        self.playback.launches.len() + retiring < MAX_PLAYBACK_INTENTS
            && self
                .playback
                .launches
                .values()
                .filter(|launch| launch.save.is_some())
                .count()
                + self.playback.saves.len()
                < MAX_PLAYBACK_INTENTS
    }

    pub fn confirm_playback(mut self: Pin<&mut Self>, token: u64) -> bool {
        let save = {
            let mut state = self.as_mut().rust_mut();
            let Some(launch) = state.playback.launches.get_mut(&token) else {
                return false;
            };
            let Some(save) = launch.save.take() else {
                return false;
            };
            save
        };
        self.as_mut()
            .rust_mut()
            .playback
            .saves
            .push_back(ConfirmedSave {
                token,
                save,
                state: SaveState::Ready,
            });
        self.submit_playback_history();
        true
    }

    pub fn finish_playback(mut self: Pin<&mut Self>, token: u64) -> bool {
        let Some(launch) = self.as_mut().rust_mut().playback.launches.remove(&token) else {
            return false;
        };
        self.as_mut()
            .rust_mut()
            .services
            .as_mut()
            .unwrap()
            .retire_gateway(launch.gateway);
        true
    }

    fn submit_playback_history(mut self: Pin<&mut Self>) -> bool {
        let Some(intent) = self.playback.saves.front() else {
            return false;
        };
        if intent.state != SaveState::Ready {
            return false;
        }
        let token = intent.token;
        let save = intent.save.clone();
        self.as_mut()
            .rust_mut()
            .playback
            .saves
            .front_mut()
            .unwrap()
            .state = SaveState::Pending;
        let accepted = self
            .as_mut()
            .submit_library(LibraryAction::PlaybackHistory { token, save });
        if !accepted
            && let Some(save) = self.as_mut().rust_mut().playback.saves.front_mut()
            && save.token == token
        {
            save.state = SaveState::Failed;
        }
        self.as_mut().library_error_text_changed();
        accepted
    }

    pub(super) fn complete_playback_history(mut self: Pin<&mut Self>, token: u64, succeeded: bool) {
        if !self
            .playback
            .saves
            .front()
            .is_some_and(|save| save.token == token && save.state == SaveState::Pending)
        {
            return;
        }
        if succeeded {
            self.as_mut().rust_mut().playback.saves.pop_front();
            self.as_mut().submit_playback_history();
        } else {
            self.as_mut()
                .rust_mut()
                .playback
                .saves
                .front_mut()
                .unwrap()
                .state = SaveState::Failed;
        }
        self.library_error_text_changed();
    }

    pub fn retry_playback_history(mut self: Pin<&mut Self>) -> bool {
        if !self.playback.failed() {
            return false;
        }
        self.as_mut()
            .rust_mut()
            .playback
            .saves
            .front_mut()
            .unwrap()
            .state = SaveState::Ready;
        self.submit_playback_history()
    }

    pub(super) fn playback_library_failed(mut self: Pin<&mut Self>) {
        if let Some(save) = self.as_mut().rust_mut().playback.saves.front_mut() {
            save.state = SaveState::Failed;
        }
        self.library_error_text_changed();
    }

    pub fn resolve_variant(mut self: Pin<&mut Self>, id: QString) {
        if self.resolving || self.inspecting {
            return;
        }
        let Some(variant) = self.variants.find(&id.to_string()) else {
            return;
        };
        let transport = variant.transport;
        let reference = variant.reference.clone();
        let Some(request) = self.source_request(SourceOperation::Resolve(reference.clone())) else {
            return;
        };
        let selection_generation = self.inspection.generation;
        self.as_mut().select_variant(id.clone());
        if self.inspection.generation != selection_generation
            || self.selected_variant_id != id
            || !self.source_request_current(&request)
        {
            return;
        }
        if self.services.is_none() {
            self.as_mut()
                .set_error(Some(ffi::ContentError::ServicesUnavailable));
            return;
        }

        let Some(group) = self.selected_group.clone() else {
            return;
        };
        let mut preferences = self
            .work_preferences
            .get(&group.anchor)
            .cloned()
            .unwrap_or_default();
        preferences.selected = Some(reference.clone());
        let save = PlaybackSave {
            group,
            variant: reference.clone(),
            preferences,
        };
        if transport == Transport::Video && self.library_worker.is_none() {
            self.as_mut()
                .set_error(Some(ffi::ContentError::LibraryOpen));
            return;
        }
        if transport == Transport::Video && !self.playback_capacity() {
            self.as_mut()
                .set_error(Some(ffi::ContentError::PlaybackPending));
            return;
        }

        self.as_mut().rust_mut().resolution.cancel();
        let generation = self.resolution.generation;
        self.as_mut().set_error(None);
        if self.resolution.generation != generation {
            return;
        }
        self.as_mut().set_resolving(true);
        if self.resolution.generation != generation {
            return;
        }
        let qt_thread = self.qt_thread();
        let services = self.services.as_ref().unwrap();
        let catalog = Arc::clone(&services.catalog);
        let browser = services.browser.client();
        let task = services.runtime.spawn(async move {
            let result = catalog.execute(&request.operation).await;
            let gateway = if matches!(
                &result,
                Ok(SourceResponse::Resolve(ResolvedContent::DirectVideo { .. }))
            ) {
                Some(MediaGateway::start().await)
            } else {
                None
            };
            let external = if let Ok(SourceResponse::Resolve(ResolvedContent::ExternalVideo {
                reference,
                url,
                ..
            })) = &result
            {
                if matches!(&reference.selector, VariantSelector::YummyAnimeVideo { player, .. } if player == "VK") {
                    Some(async {
                        let handle = browser.handle().await.map_err(|_| ())?;
                        let media = resolve_external_video(
                            &handle,
                            BrowserChoice::Auto,
                            reference,
                            url.clone(),
                        )
                        .await
                        .map_err(|_| ())?;
                        MediaGateway::prepare(media).await.map_err(|_| ())
                    }
                    .await)
                } else {
                    None
                }
            } else {
                None
            };
            let _ = qt_thread.queue(move |mut controller| {
                if controller.resolution.generation != generation
                    || !controller.source_request_current(&request)
                {
                    return;
                }
                controller.as_mut().rust_mut().resolution.task = None;
                controller.as_mut().source_outcome(&request, &result);
                if controller.resolution.generation != generation
                    || !controller.source_request_current(&request)
                {
                    return;
                }
                controller.as_mut().set_resolving(false);
                if controller.resolution.generation != generation
                    || !controller.source_request_current(&request)
                {
                    return;
                }
                match result {
                    Ok(SourceResponse::Resolve(ResolvedContent::Torrent {
                        reference: resolved,
                        magnet,
                        ..
                    })) if resolved == reference => controller
                        .as_mut()
                        .torrent_ready(QString::from(magnet.as_str())),
                    Ok(SourceResponse::Resolve(ResolvedContent::TorrentFile {
                        reference: resolved,
                        metadata,
                    })) if resolved == reference => controller.as_mut().torrent_metadata_ready(
                        QString::from(models::variant_id(&reference)),
                        QByteArray::from(metadata.bytes()),
                    ),
                    Ok(SourceResponse::Resolve(ResolvedContent::DirectVideo {
                        reference: resolved,
                        title,
                        url,
                        ..
                    })) if resolved == reference => match gateway.unwrap() {
                        Ok(gateway) => controller.as_mut().accept_video(
                            gateway,
                            save,
                            QUrl::from(&QString::from(url.as_str())),
                            QString::from(title),
                            None,
                        ),
                        Err(_) => controller
                            .as_mut()
                            .set_error(Some(ffi::ContentError::Playback)),
                    },
                    Ok(SourceResponse::Resolve(ResolvedContent::ExternalVideo {
                        reference: resolved,
                        title,
                        ..
                    })) if resolved == reference => match external {
                        Some(Ok((gateway, prepared))) => {
                            let url = QUrl::from(&QString::from(prepared.url.as_str()));
                            controller.as_mut().accept_video(
                                gateway, save, url, QString::from(title), Some(prepared),
                            );
                        }
                        Some(Err(())) => controller
                            .as_mut()
                            .set_error(Some(ffi::ContentError::BrowserPlayback)),
                        None => controller
                            .as_mut()
                            .set_error(Some(ffi::ContentError::ExternalPlayback)),
                    },
                    Err(_) => {}
                    Ok(_) => {
                        ffi::log_content_error(
                            &QString::from("resolve"),
                            &QString::from("Resolved variant does not match the selected action"),
                        );
                        let error = if transport == Transport::Video {
                            ffi::ContentError::Playback
                        } else {
                            ffi::ContentError::Download
                        };
                        controller.as_mut().set_error(Some(error));
                    }
                }
            });
        });
        self.as_mut().rust_mut().resolution.task = Some(task);
    }
}
