use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QByteArray, QHash, QModelIndex, QString, QVariant, QVector};
use qbutt_content::{Source, SourceFailureReason};

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
        include!("cxx-qt-lib/qhash.h");
        type QHash_i32_QByteArray = cxx_qt_lib::QHash<cxx_qt_lib::QHashPair_i32_QByteArray>;
        include!("cxx-qt-lib/qvector.h");
        type QVector_i32 = cxx_qt_lib::QVector<i32>;
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_uncreatable]
        #[base = QAbstractListModel]
        type SourceModel = super::SourceModelRust;

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
        #[rust_name = "create_source_model"]
        fn createSourceModel() -> UniquePtr<SourceModel>;
        #[rust_name = "source_error"]
        fn sourceError(reason: i32) -> QString;
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct SourceRow {
    pub source: Source,
    pub enabled: bool,
    pub failure: Option<SourceFailureReason>,
    pub pending: bool,
    pub retrying: bool,
}

#[derive(Default)]
pub struct SourceModelRust {
    rows: Vec<SourceRow>,
}

impl ffi::SourceModel {
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
            0 => QVariant::from(&QString::from(row.source.id())),
            1 => QVariant::from(&QString::from(row.source.name())),
            2 => QVariant::from(&row.enabled),
            3 => QVariant::from(&row.failure.is_some()),
            4 => QVariant::from(
                &row.failure
                    .map(|reason| {
                        ffi::source_error(match reason {
                            SourceFailureReason::Timeout => 1,
                            SourceFailureReason::Http(401 | 403) => 2,
                            SourceFailureReason::VariantUnavailable
                            | SourceFailureReason::Http(404 | 410) => 3,
                            _ => 0,
                        })
                    })
                    .unwrap_or_default(),
            ),
            5 => QVariant::from(&row.pending),
            6 => QVariant::from(&row.retrying),
            _ => QVariant::default(),
        }
    }

    fn role_names(&self) -> ffi::QHash_i32_QByteArray {
        let mut roles = QHash::default();
        for (index, name) in [
            "sourceId",
            "name",
            "enabled",
            "failed",
            "failureText",
            "pending",
            "retrying",
        ]
        .iter()
        .enumerate()
        {
            roles.insert(256 + index as i32, QByteArray::from(*name));
        }
        roles
    }

    pub fn replace(mut self: Pin<&mut Self>, rows: Vec<SourceRow>) {
        self.as_mut().begin_reset_model();
        self.as_mut().rust_mut().rows = rows;
        self.end_reset_model();
    }

    pub fn update(mut self: Pin<&mut Self>, row: SourceRow) {
        let Some(index) = self
            .rows
            .iter()
            .position(|existing| existing.source == row.source)
        else {
            return;
        };
        if self.rows[index] == row {
            return;
        }
        self.as_mut().rust_mut().rows[index] = row;
        let index = self.index(index as i32, 0, &QModelIndex::default());
        self.data_changed(&index, &index, &QVector::default());
    }
}
