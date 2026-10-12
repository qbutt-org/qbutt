use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QByteArray, QHash, QModelIndex, QString, QVariant, QVector};
use qbutt_content::{
    Category, PublicationRef, Source, Transport, Variant, VariantRef, VariantSelector, Work,
};

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
        type WorkModel = super::WorkModelRust;

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
        #[inherit]
        #[rust_name = "begin_insert_rows"]
        fn beginInsertRows(self: Pin<&mut Self>, parent: &QModelIndex, first: i32, last: i32);
        #[inherit]
        #[rust_name = "end_insert_rows"]
        fn endInsertRows(self: Pin<&mut Self>);
        #[inherit]
        #[rust_name = "begin_remove_rows"]
        fn beginRemoveRows(self: Pin<&mut Self>, parent: &QModelIndex, first: i32, last: i32);
        #[inherit]
        #[rust_name = "end_remove_rows"]
        fn endRemoveRows(self: Pin<&mut Self>);
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

    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_uncreatable]
        #[base = QAbstractListModel]
        type VariantModel = super::VariantModelRust;

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
        #[inherit]
        #[rust_name = "begin_insert_rows"]
        fn beginInsertRows(self: Pin<&mut Self>, parent: &QModelIndex, first: i32, last: i32);
        #[inherit]
        #[rust_name = "end_insert_rows"]
        fn endInsertRows(self: Pin<&mut Self>);
        #[inherit]
        #[rust_name = "begin_remove_rows"]
        fn beginRemoveRows(self: Pin<&mut Self>, parent: &QModelIndex, first: i32, last: i32);
        #[inherit]
        #[rust_name = "end_remove_rows"]
        fn endRemoveRows(self: Pin<&mut Self>);
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
        #[rust_name = "create_work_model"]
        fn createWorkModel() -> UniquePtr<WorkModel>;
        #[rust_name = "create_variant_model"]
        fn createVariantModel() -> UniquePtr<VariantModel>;
        #[rust_name = "format_size"]
        fn formatSize(bytes: u64) -> QString;
        #[rust_name = "format_translation_label"]
        fn formatTranslationLabel(language: &QString, translation: &QString) -> QString;
    }
}

const USER_ROLE: i32 = 256;

pub fn work_id(reference: &PublicationRef) -> String {
    match reference.source {
        Source::AniLiberty => format!("aniliberty:{}", reference.id),
        Source::MangaDex => format!("mangadex:{}", reference.id),
    }
}

pub fn variant_id(reference: &VariantRef) -> String {
    let publication = work_id(&reference.publication);
    match &reference.selector {
        VariantSelector::Torrent { id } => format!("{publication}:torrent:{id}"),
        VariantSelector::Episode { id, quality } => {
            format!("{publication}:episode:{id}:{}", quality.height())
        }
        VariantSelector::Rutube { episode_id } => format!("{publication}:rutube:{episode_id}"),
        VariantSelector::MangaTranslation { language, groups } => {
            format!("{publication}:translation:{language}:{}", groups.join("+"))
        }
    }
}

fn role_names(names: &[&str]) -> ffi::QHash_i32_QByteArray {
    let mut roles = QHash::default();
    for (index, name) in names.iter().enumerate() {
        roles.insert(USER_ROLE + index as i32, QByteArray::from(*name));
    }
    roles
}

#[derive(Default)]
pub struct WorkModelRust {
    rows: Vec<Work>,
}

impl ffi::WorkModel {
    pub fn id_at(&self, row: i32) -> QString {
        self.rows
            .get(row as usize)
            .map(|work| QString::from(work_id(&work.reference)))
            .unwrap_or_default()
    }

    pub fn index_of_id(&self, id: QString) -> i32 {
        let id = id.to_string();
        self.rows
            .iter()
            .position(|work| work_id(&work.reference) == id)
            .map(|row| row as i32)
            .unwrap_or(-1)
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
        let Some(work) = self.rows.get(index.row() as usize) else {
            return QVariant::default();
        };
        let text = match role - USER_ROLE {
            0 => work_id(&work.reference),
            1 => work.title.clone(),
            2 => match work.reference.source {
                Source::AniLiberty => "AniLiberty".into(),
                Source::MangaDex => "MangaDex".into(),
            },
            3 => match work.category {
                Category::Anime => "anime".into(),
                Category::Manga => "manga".into(),
            },
            4 => work.year.map(|year| year.to_string()).unwrap_or_default(),
            _ => return QVariant::default(),
        };
        QVariant::from(&QString::from(text))
    }

    fn role_names(&self) -> ffi::QHash_i32_QByteArray {
        role_names(&["workId", "title", "source", "category", "year"])
    }

    pub fn find(&self, id: &str) -> Option<&Work> {
        self.rows.iter().find(|work| work_id(&work.reference) == id)
    }

    pub fn clear(mut self: Pin<&mut Self>) {
        if self.rows.is_empty() {
            return;
        }
        self.as_mut().begin_reset_model();
        self.as_mut().rust_mut().rows.clear();
        self.as_mut().end_reset_model();
    }

    /// Publish the library's confirmed order without sorting or retaining
    /// persistence metadata in the presentation model.
    pub fn replace(mut self: Pin<&mut Self>, works: Vec<Work>) {
        if self.rows == works {
            return;
        }
        self.as_mut().begin_reset_model();
        self.as_mut().rust_mut().rows = works;
        self.end_reset_model();
    }

    pub fn remove(mut self: Pin<&mut Self>, reference: &PublicationRef) {
        if let Some(row) = self
            .rows
            .iter()
            .position(|work| &work.reference == reference)
        {
            self.as_mut()
                .begin_remove_rows(&QModelIndex::default(), row as i32, row as i32);
            self.as_mut().rust_mut().rows.remove(row);
            self.as_mut().end_remove_rows();
        }
    }

    pub fn update(mut self: Pin<&mut Self>, work: Work) {
        let parent = QModelIndex::default();
        if let Some(row) = self
            .rows
            .iter()
            .position(|item| item.reference == work.reference)
        {
            if self.rows[row] == work {
                return;
            }
            self.as_mut().rust_mut().rows[row] = work;
            let index = self.index(row as i32, 0, &parent);
            self.as_mut()
                .data_changed(&index, &index, &QVector::default());
        } else {
            let row = self.rows.len() as i32;
            self.as_mut().begin_insert_rows(&parent, row, row);
            self.as_mut().rust_mut().rows.push(work);
            self.as_mut().end_insert_rows();
        }
    }
}

#[derive(Default)]
pub struct VariantModelRust {
    rows: Vec<Variant>,
    work_title: String,
    revision: u64,
}

impl ffi::VariantModel {
    pub fn id_at(&self, row: i32) -> QString {
        self.rows
            .get(row as usize)
            .map(|variant| QString::from(variant_id(&variant.reference)))
            .unwrap_or_default()
    }

    pub fn index_of_id(&self, id: QString) -> i32 {
        let id = id.to_string();
        self.rows
            .iter()
            .position(|variant| variant_id(&variant.reference) == id)
            .map(|row| row as i32)
            .unwrap_or(-1)
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
        let Some(variant) = self.rows.get(index.row() as usize) else {
            return QVariant::default();
        };
        if role == USER_ROLE + 5 {
            let size = variant.size.map(ffi::format_size).unwrap_or_default();
            return QVariant::from(&size);
        }
        if role == USER_ROLE + 1 && variant.transport == Transport::Manga {
            let language = QString::from(
                variant
                    .languages
                    .first()
                    .map(String::as_str)
                    .unwrap_or_default(),
            );
            let translation = QString::from(variant.translation.as_deref().unwrap_or_default());
            return QVariant::from(&ffi::format_translation_label(&language, &translation));
        }
        let text = match role - USER_ROLE {
            0 => variant_id(&variant.reference),
            1 => variant
                .title
                .clone()
                .unwrap_or_else(|| self.work_title.clone()),
            2 => match variant.reference.publication.source {
                Source::AniLiberty => "AniLiberty".into(),
                Source::MangaDex => "MangaDex".into(),
            },
            3 => match variant.transport {
                Transport::Torrent => "torrent".into(),
                Transport::Video => "video".into(),
                Transport::Manga => "manga".into(),
            },
            4 => variant.quality.clone().unwrap_or_default(),
            6 => variant
                .seeders
                .map(|seeders| seeders.to_string())
                .unwrap_or_default(),
            7 => variant.info_hash.clone().unwrap_or_default(),
            _ => return QVariant::default(),
        };
        QVariant::from(&QString::from(text))
    }

    fn role_names(&self) -> ffi::QHash_i32_QByteArray {
        role_names(&[
            "variantId",
            "label",
            "source",
            "kind",
            "quality",
            "sizeText",
            "seedersText",
            "infoHash",
        ])
    }

    pub fn find(&self, id: &str) -> Option<&Variant> {
        self.rows
            .iter()
            .find(|variant| variant_id(&variant.reference) == id)
    }

    pub fn clear(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().revision = self.revision.wrapping_add(1);
        if self.rows.is_empty() {
            return;
        }
        self.as_mut().begin_reset_model();
        self.as_mut().rust_mut().rows.clear();
        self.as_mut().end_reset_model();
    }

    pub fn update(mut self: Pin<&mut Self>, work: &Work) {
        self.as_mut().rust_mut().revision = self.revision.wrapping_add(1);
        let revision = self.revision;
        self.as_mut().rust_mut().work_title.clone_from(&work.title);
        let variants: Vec<_> = work
            .variants
            .iter()
            .filter(|variant| matches!(variant.transport, Transport::Torrent | Transport::Manga))
            .collect();
        let parent = QModelIndex::default();

        for row in (0..self.rows.len()).rev() {
            if variants
                .iter()
                .any(|variant| variant.reference == self.rows[row].reference)
            {
                continue;
            }
            self.as_mut()
                .begin_remove_rows(&parent, row as i32, row as i32);
            self.as_mut().rust_mut().rows.remove(row);
            self.as_mut().end_remove_rows();
            if self.revision != revision {
                return;
            }
        }

        for variant in variants {
            if let Some(row) = self
                .rows
                .iter()
                .position(|item| item.reference == variant.reference)
            {
                self.as_mut().rust_mut().rows[row] = variant.clone();
                let index = self.index(row as i32, 0, &parent);
                self.as_mut()
                    .data_changed(&index, &index, &QVector::default());
            } else {
                let row = self.rows.len() as i32;
                self.as_mut().begin_insert_rows(&parent, row, row);
                self.as_mut().rust_mut().rows.push(variant.clone());
                self.as_mut().end_insert_rows();
            }
            if self.revision != revision {
                return;
            }
        }
    }
}
