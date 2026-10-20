use std::collections::{HashMap, VecDeque};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use qbutt_content::{VariantPreferences, VariantRef, WorkGroup};

use super::ffi;
use super::library::LibraryAction;

static NEXT_HISTORY_TOKEN: AtomicU64 = AtomicU64::new(1);
const MAX_HISTORY_INTENTS: usize = 64;

#[derive(Clone)]
pub(super) struct HistorySave {
    pub group: WorkGroup,
    pub variant: VariantRef,
    pub preferences: VariantPreferences,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SaveState {
    Ready,
    Pending,
    Failed,
}

struct ConfirmedSave {
    token: u64,
    save: HistorySave,
    state: SaveState,
}

#[derive(Default)]
pub(super) struct HistoryState {
    torrents: HashMap<String, HistorySave>,
    saves: VecDeque<ConfirmedSave>,
}

impl HistoryState {
    pub(super) fn has_capacity(&self, playback_intents: usize) -> bool {
        self.torrents.len() + self.saves.len() + playback_intents < MAX_HISTORY_INTENTS
    }

    pub(super) fn failed(&self) -> bool {
        self.saves
            .front()
            .is_some_and(|save| save.state == SaveState::Failed)
    }
}

impl ffi::ContentController {
    pub(super) fn begin_torrent_addition(
        mut self: Pin<&mut Self>,
        save: HistorySave,
    ) -> Option<QString> {
        if !self
            .history_writes
            .has_capacity(self.playback.pending_history())
        {
            self.set_error(Some(ffi::ContentError::ActionPending));
            return None;
        }
        // Each attempt has its own source identity, including repeated clicks
        // for the same infohash. An existing dialog keeps its original attempt.
        let token = NEXT_HISTORY_TOKEN.fetch_add(1, Ordering::Relaxed);
        let source = format!("qbutt-content:{token}");
        self.as_mut()
            .rust_mut()
            .history_writes
            .torrents
            .insert(source.clone(), save);
        Some(QString::from(source))
    }

    pub fn finish_torrent_addition(mut self: Pin<&mut Self>, source: QString, added: bool) -> bool {
        let Some(save) = self
            .as_mut()
            .rust_mut()
            .history_writes
            .torrents
            .remove(&source.to_string())
        else {
            return false;
        };
        if added {
            self.record_history(save);
        }
        true
    }

    pub(super) fn record_history(mut self: Pin<&mut Self>, save: HistorySave) {
        let token = NEXT_HISTORY_TOKEN.fetch_add(1, Ordering::Relaxed);
        self.as_mut()
            .rust_mut()
            .history_writes
            .saves
            .push_back(ConfirmedSave {
                token,
                save,
                state: SaveState::Ready,
            });
        self.submit_history_write();
    }

    fn submit_history_write(mut self: Pin<&mut Self>) -> bool {
        let Some(intent) = self.history_writes.saves.front() else {
            return false;
        };
        if intent.state != SaveState::Ready {
            return false;
        }
        let token = intent.token;
        let save = intent.save.clone();
        self.as_mut()
            .rust_mut()
            .history_writes
            .saves
            .front_mut()
            .unwrap()
            .state = SaveState::Pending;
        let accepted = self
            .as_mut()
            .submit_library(LibraryAction::Opened { token, save });
        if !accepted
            && let Some(save) = self.as_mut().rust_mut().history_writes.saves.front_mut()
            && save.token == token
        {
            save.state = SaveState::Failed;
        }
        self.as_mut().library_error_text_changed();
        accepted
    }

    pub(super) fn complete_history_write(mut self: Pin<&mut Self>, token: u64, succeeded: bool) {
        if !self
            .history_writes
            .saves
            .front()
            .is_some_and(|save| save.token == token && save.state == SaveState::Pending)
        {
            return;
        }
        if succeeded {
            self.as_mut().rust_mut().history_writes.saves.pop_front();
            self.as_mut().submit_history_write();
        } else {
            self.as_mut()
                .rust_mut()
                .history_writes
                .saves
                .front_mut()
                .unwrap()
                .state = SaveState::Failed;
        }
        self.library_error_text_changed();
    }

    pub fn retry_history_write(mut self: Pin<&mut Self>) -> bool {
        if !self.history_writes.failed() {
            return false;
        }
        self.as_mut()
            .rust_mut()
            .history_writes
            .saves
            .front_mut()
            .unwrap()
            .state = SaveState::Ready;
        self.submit_history_write()
    }

    pub(super) fn history_library_failed(mut self: Pin<&mut Self>) {
        if let Some(save) = self.as_mut().rust_mut().history_writes.saves.front_mut() {
            save.state = SaveState::Failed;
        }
        self.library_error_text_changed();
    }
}
