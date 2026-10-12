use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QByteArray, QHash, QModelIndex, QString, QVariant, QVector};
use qbutt_content::{DownloadJob, DownloadOrigin, DownloadStatus, VariantRef};

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
        type DownloadModel = super::DownloadModelRust;

        #[cxx_override]
        #[rust_name = "row_count"]
        fn rowCount(&self, parent: &QModelIndex) -> i32;
        #[cxx_override]
        fn data(&self, index: &QModelIndex, role: i32) -> QVariant;
        #[cxx_override]
        #[rust_name = "role_names"]
        fn roleNames(&self) -> QHash_i32_QByteArray;

        #[inherit]
        #[rust_name = "begin_insert_rows"]
        fn beginInsertRows(self: Pin<&mut Self>, parent: &QModelIndex, first: i32, last: i32);
        #[inherit]
        #[rust_name = "end_insert_rows"]
        fn endInsertRows(self: Pin<&mut Self>);
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
        #[rust_name = "create_download_model"]
        fn createDownloadModel() -> UniquePtr<DownloadModel>;
    }
}

#[derive(Default)]
pub struct DownloadModelRust {
    rows: Vec<DownloadJob>,
}

impl ffi::DownloadModel {
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
        let Some(job) = self.rows.get(index.row() as usize) else {
            return QVariant::default();
        };
        match role - 256 {
            0 => QVariant::from(&QString::from(&job.id)),
            1 => QVariant::from(&QString::from(&job.title)),
            2 => QVariant::from(&QString::from(match job.status {
                DownloadStatus::Queued => "queued",
                DownloadStatus::Downloading => "downloading",
                DownloadStatus::Publishing => "publishing",
                DownloadStatus::Complete => "complete",
                DownloadStatus::Cancelled => "cancelled",
                DownloadStatus::Error => "error",
            })),
            3 => QVariant::from(&(job.bytes as f64)),
            4 => job
                .total_bytes
                .map(|total| QVariant::from(&(total as f64)))
                .unwrap_or_default(),
            5 => QVariant::from(&QString::from(
                job.path
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            )),
            6 => QVariant::from(&QString::from(job.error.as_deref().unwrap_or_default())),
            7 => QVariant::from(&matches!(&job.origin, DownloadOrigin::Catalog { .. })),
            _ => QVariant::default(),
        }
    }

    fn role_names(&self) -> ffi::QHash_i32_QByteArray {
        let mut roles = QHash::default();
        for (index, name) in [
            "jobId",
            "title",
            "status",
            "bytes",
            "totalBytes",
            "path",
            "errorText",
            "canRetry",
        ]
        .iter()
        .enumerate()
        {
            roles.insert(256 + index as i32, QByteArray::from(*name));
        }
        roles
    }

    pub fn retryable_reference(&self, id: &str) -> Option<VariantRef> {
        self.rows
            .iter()
            .find(|job| {
                job.id == id
                    && matches!(
                        job.status,
                        DownloadStatus::Cancelled | DownloadStatus::Error
                    )
            })
            .and_then(|job| match &job.origin {
                DownloadOrigin::Catalog { reference } => Some(reference.clone()),
                DownloadOrigin::Browser { .. } => None,
            })
    }

    pub fn cancellable(&self, id: &str) -> bool {
        self.rows.iter().any(|job| {
            job.id == id
                && matches!(
                    job.status,
                    DownloadStatus::Queued | DownloadStatus::Downloading
                )
        })
    }

    pub fn apply(mut self: Pin<&mut Self>, jobs: Vec<DownloadJob>) {
        for job in jobs {
            if let Some(row) = self.rows.iter().position(|current| current.id == job.id) {
                if self.rows[row] == job {
                    continue;
                }
                self.as_mut().rust_mut().rows[row] = job;
                let index = self.index(row as i32, 0, &QModelIndex::default());
                self.as_mut()
                    .data_changed(&index, &index, &QVector::default());
            } else {
                let row = self.rows.len() as i32;
                self.as_mut()
                    .begin_insert_rows(&QModelIndex::default(), row, row);
                self.as_mut().rust_mut().rows.push(job);
                self.as_mut().end_insert_rows();
            }
        }
    }
}
