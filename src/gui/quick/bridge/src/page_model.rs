use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::Arc;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QByteArray, QHash, QModelIndex, QString, QUrl, QVariant, QVector};

use crate::controller::ffi as controller;
use crate::controller::reader_pages::CachedPage;

#[cxx_qt::bridge(namespace = "Qbutt")]
pub mod ffi {
    #[namespace = ""]
    unsafe extern "C++" {
        include!(<QAbstractListModel>);
        type QAbstractListModel;
        include!("cxx-qt-lib/qmodelindex.h");
        type QModelIndex = cxx_qt_lib::QModelIndex;
        include!("cxx-qt-lib/qvariant.h");
        type QVariant = cxx_qt_lib::QVariant;
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qhash.h");
        type QHash_i32_QByteArray = cxx_qt_lib::QHash<cxx_qt_lib::QHashPair_i32_QByteArray>;
        include!("cxx-qt-lib/qvector.h");
        type QVector_i32 = cxx_qt_lib::QVector<i32>;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_uncreatable]
        #[base = QAbstractListModel]
        #[qproperty(u64, generation, READ = chapter_generation, NOTIFY = generation_changed)]
        type PageModel = super::PageModelRust;

        fn chapter_generation(&self) -> u64;
        #[qsignal]
        #[cxx_name = "generationChanged"]
        fn generation_changed(self: Pin<&mut Self>);

        #[cxx_override]
        #[rust_name = "row_count"]
        fn rowCount(&self, parent: &QModelIndex) -> i32;
        #[cxx_override]
        fn data(&self, index: &QModelIndex, role: i32) -> QVariant;
        #[cxx_override]
        #[rust_name = "role_names"]
        fn roleNames(&self) -> QHash_i32_QByteArray;

        #[inherit]
        #[rust_name = "begin_reset_model"]
        fn beginResetModel(self: Pin<&mut Self>);
        #[inherit]
        #[rust_name = "end_reset_model"]
        fn endResetModel(self: Pin<&mut Self>);
        #[inherit]
        fn index(&self, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;
        #[inherit]
        #[qsignal]
        #[rust_name = "data_changed"]
        fn dataChanged(
            self: Pin<&mut Self>,
            first: &QModelIndex,
            last: &QModelIndex,
            roles: &QVector_i32,
        );
    }

    unsafe extern "C++" {
        include!("qbutt_quick/native.h");
        #[rust_name = "create_page_model"]
        fn createPageModel() -> UniquePtr<PageModel>;
    }
}

#[derive(Default)]
struct PageRow {
    width: i32,
    height: i32,
    load: PageLoad,
}

#[derive(Default)]
enum PageLoad {
    #[default]
    Unloaded,
    Loading,
    Ready {
        page: Arc<CachedPage>,
        shown: bool,
    },
    Failed,
}

enum Update {
    Reset(u64, usize),
    Window(u64, usize, usize, usize, usize, usize),
    Loading(u64, usize),
    Ready(u64, usize, Arc<CachedPage>),
    Failed(u64, usize),
}

#[derive(Default)]
pub struct PageModelRust {
    rows: Vec<PageRow>,
    generation: u64,
    publishing: bool,
    pending: VecDeque<Update>,
}

impl ffi::PageModel {
    pub(crate) fn chapter_generation(&self) -> u64 {
        self.generation
    }

    fn row_count(&self, parent: &QModelIndex) -> i32 {
        if parent.is_valid() {
            0
        } else {
            self.rows.len() as i32
        }
    }

    fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        if !index.is_valid() || index.column() != 0 {
            return QVariant::default();
        }
        let Some(row) = self.rows.get(index.row() as usize) else {
            return QVariant::default();
        };
        match role - 256 {
            0 => QVariant::from(&index.row()),
            1 => QVariant::from(&self.url(index.row() as usize)),
            2 => QVariant::from(&row.width),
            3 => QVariant::from(&row.height),
            4 => {
                let state = match row.load {
                    PageLoad::Unloaded => controller::ReaderPageState::PageUnloaded,
                    PageLoad::Loading => controller::ReaderPageState::PageLoading,
                    PageLoad::Ready { .. } => controller::ReaderPageState::PageReady,
                    PageLoad::Failed => controller::ReaderPageState::PageFailed,
                };
                QVariant::from(&state.repr)
            }
            5 => QVariant::from(&if matches!(row.load, PageLoad::Failed) {
                controller::reader_error(controller::ReaderError::Page)
            } else {
                QString::default()
            }),
            _ => QVariant::default(),
        }
    }

    fn role_names(&self) -> ffi::QHash_i32_QByteArray {
        let mut roles = QHash::default();
        for (index, name) in [
            "pageIndex",
            "pageUrl",
            "imageWidth",
            "imageHeight",
            "pageState",
            "errorText",
        ]
        .iter()
        .enumerate()
        {
            roles.insert(256 + index as i32, QByteArray::from(*name));
        }
        roles
    }

    pub(crate) fn count(&self) -> usize {
        self.rows.len()
    }

    pub(crate) fn dimensions(&self, index: usize) -> (i32, i32) {
        self.rows
            .get(index)
            .map(|row| (row.width, row.height))
            .unwrap_or_default()
    }

    pub(crate) fn url(&self, index: usize) -> QUrl {
        match self.rows.get(index).map(|row| &row.load) {
            Some(PageLoad::Ready { page, .. }) => QUrl::from(&page.url),
            _ => QUrl::default(),
        }
    }

    pub(crate) fn shown(&self, index: usize) -> bool {
        matches!(
            self.rows.get(index).map(|row| &row.load),
            Some(PageLoad::Ready { shown: true, .. })
        )
    }

    pub(crate) fn failed(&self, index: usize) -> bool {
        matches!(
            self.rows.get(index).map(|row| &row.load),
            Some(PageLoad::Failed)
        )
    }

    pub(crate) fn confirm(mut self: Pin<&mut Self>, index: usize, url: &QUrl) -> bool {
        let mut state = self.as_mut().rust_mut();
        let Some(PageRow {
            load: PageLoad::Ready { page, shown },
            ..
        }) = state.rows.get_mut(index)
        else {
            return false;
        };
        if QUrl::from(&page.url) != *url {
            return false;
        }
        *shown = true;
        true
    }

    pub(crate) fn reset(self: Pin<&mut Self>, generation: u64, count: usize) {
        self.publish(Update::Reset(generation, count));
    }

    pub(crate) fn window(
        self: Pin<&mut Self>,
        generation: u64,
        first: usize,
        last: usize,
        keep_first: usize,
        keep_last: usize,
        anchor: usize,
    ) {
        self.publish(Update::Window(
            generation, first, last, keep_first, keep_last, anchor,
        ));
    }

    pub(crate) fn loading(self: Pin<&mut Self>, generation: u64, index: usize) {
        self.publish(Update::Loading(generation, index));
    }

    pub(crate) fn ready(
        self: Pin<&mut Self>,
        generation: u64,
        index: usize,
        page: Arc<CachedPage>,
    ) {
        self.publish(Update::Ready(generation, index, page));
    }

    pub(crate) fn fail(self: Pin<&mut Self>, generation: u64, index: usize) {
        self.publish(Update::Failed(generation, index));
    }

    fn publish(mut self: Pin<&mut Self>, update: Update) {
        self.as_mut().rust_mut().pending.push_back(update);
        if self.publishing {
            return;
        }
        self.as_mut().rust_mut().publishing = true;
        while let Some(update) = self.as_mut().rust_mut().pending.pop_front() {
            if let Update::Reset(generation, count) = update {
                self.as_mut().begin_reset_model();
                {
                    let mut state = self.as_mut().rust_mut();
                    state.rows = (0..count).map(|_| PageRow::default()).collect();
                    state.generation = generation;
                }
                self.as_mut().end_reset_model();
                self.as_mut().generation_changed();
                continue;
            }
            let (generation, first, last) = match update {
                Update::Window(generation, _, _, _, _, _) => {
                    (generation, 0, self.rows.len().saturating_sub(1))
                }
                Update::Loading(generation, index)
                | Update::Ready(generation, index, _)
                | Update::Failed(generation, index) => (generation, index, index),
                Update::Reset(_, _) => unreachable!(),
            };
            if self.generation != generation || last >= self.rows.len() {
                continue;
            }
            match update {
                Update::Window(_, first, last, keep_first, keep_last, anchor) => {
                    for (index, row) in self.as_mut().rust_mut().rows.iter_mut().enumerate() {
                        if index != anchor && (index < keep_first || index > keep_last) {
                            row.load = PageLoad::Unloaded;
                        } else if let PageLoad::Ready { shown, .. } = &mut row.load
                            && (index < first || index > last)
                        {
                            *shown = false;
                        }
                    }
                }
                Update::Loading(_, index) => {
                    self.as_mut().rust_mut().rows[index].load = PageLoad::Loading
                }
                Update::Ready(_, index, page) => {
                    let row = &mut self.as_mut().rust_mut().rows[index];
                    if matches!(&row.load, PageLoad::Ready { page: previous, .. } if previous.url == page.url)
                    {
                        continue;
                    }
                    row.width = page.width;
                    row.height = page.height;
                    row.load = PageLoad::Ready { page, shown: false };
                }
                Update::Failed(_, index) => {
                    self.as_mut().rust_mut().rows[index].load = PageLoad::Failed
                }
                Update::Reset(_, _) => unreachable!(),
            }
            let first = self.index(first as i32, 0, &QModelIndex::default());
            let last = self.index(last as i32, 0, &QModelIndex::default());
            self.as_mut()
                .data_changed(&first, &last, &QVector::default());
        }
        self.as_mut().rust_mut().publishing = false;
    }
}
