use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QByteArray, QHash, QModelIndex, QString, QVariant};
use qbutt_content::{Chapter, ChapterRef};

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
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_uncreatable]
        #[base = QAbstractListModel]
        type ChapterModel = super::ChapterModelRust;

        #[qinvokable]
        #[cxx_name = "idAt"]
        fn id_at(&self, row: i32) -> QString;
        #[qinvokable]
        #[cxx_name = "indexOfId"]
        fn index_of_id(&self, id: QString) -> i32;

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
    }

    unsafe extern "C++" {
        include!("qbutt_quick/native.h");
        #[rust_name = "create_chapter_model"]
        fn createChapterModel() -> UniquePtr<ChapterModel>;
    }
}

#[derive(Default)]
pub struct ChapterModelRust {
    rows: Vec<Chapter>,
    visible: Vec<usize>,
    query: String,
}

impl ffi::ChapterModel {
    pub fn id_at(&self, row: i32) -> QString {
        self.visible
            .get(row as usize)
            .map(|index| QString::from(&self.rows[*index].reference.id))
            .unwrap_or_default()
    }

    pub fn index_of_id(&self, id: QString) -> i32 {
        let id = id.to_string();
        self.visible
            .iter()
            .position(|index| self.rows[*index].reference.id == id)
            .map(|index| index as i32)
            .unwrap_or(-1)
    }

    fn row_count(&self, parent: &QModelIndex) -> i32 {
        if parent.is_valid() {
            0
        } else {
            self.visible.len() as i32
        }
    }

    fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        if !index.is_valid() || index.column() != 0 {
            return QVariant::default();
        }
        let Some(row) = self.visible.get(index.row() as usize) else {
            return QVariant::default();
        };
        let chapter = &self.rows[*row];
        let text = match role - 256 {
            0 => chapter.reference.id.as_str(),
            1 => chapter.title.as_deref().unwrap_or_default(),
            2 => chapter.number.as_deref().unwrap_or_default(),
            3 => chapter.volume.as_deref().unwrap_or_default(),
            4 => return QVariant::from(&chapter.pages),
            _ => return QVariant::default(),
        };
        QVariant::from(&QString::from(text))
    }

    fn role_names(&self) -> ffi::QHash_i32_QByteArray {
        let mut roles = QHash::default();
        for (index, name) in ["chapterId", "title", "number", "volume", "pageCount"]
            .iter()
            .enumerate()
        {
            roles.insert(256 + index as i32, QByteArray::from(*name));
        }
        roles
    }

    pub fn find(&self, id: &str) -> Option<&Chapter> {
        self.rows.iter().find(|chapter| chapter.reference.id == id)
    }

    pub fn first(&self) -> Option<ChapterRef> {
        self.rows.first().map(|chapter| chapter.reference.clone())
    }

    pub fn replace(mut self: Pin<&mut Self>, chapters: Vec<Chapter>) {
        self.as_mut().begin_reset_model();
        self.as_mut().rust_mut().rows = chapters;
        self.as_mut().refilter();
        self.as_mut().end_reset_model();
    }

    pub fn filter(mut self: Pin<&mut Self>, query: &str) {
        let query = query.trim().to_lowercase();
        if self.query == query {
            return;
        }
        self.as_mut().begin_reset_model();
        self.as_mut().rust_mut().query = query;
        self.as_mut().refilter();
        self.as_mut().end_reset_model();
    }

    fn refilter(mut self: Pin<&mut Self>) {
        let query = &self.query;
        let visible = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, chapter)| {
                query.is_empty()
                    || [&chapter.title, &chapter.number, &chapter.volume]
                        .into_iter()
                        .flatten()
                        .any(|text| text.to_lowercase().contains(query))
            })
            .map(|(index, _)| index)
            .collect();
        self.as_mut().rust_mut().visible = visible;
    }
}
