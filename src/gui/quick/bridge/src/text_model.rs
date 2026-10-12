use std::pin::Pin;
use std::sync::Arc;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QByteArray, QHash, QModelIndex, QString, QVariant};
use qbutt_content::{TextBlockKind, TextChapter, TextSpan};

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
        type TextBlockModel = super::TextBlockModelRust;

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
        #[rust_name = "create_text_block_model"]
        fn createTextBlockModel() -> UniquePtr<TextBlockModel>;
    }
}

#[derive(Default)]
pub struct TextBlockModelRust {
    chapter: Option<Arc<TextChapter>>,
    pending: Option<Option<Arc<TextChapter>>>,
    replacing: bool,
}

impl ffi::TextBlockModel {
    pub fn id_at(&self, row: i32) -> QString {
        self.chapter
            .as_ref()
            .and_then(|chapter| chapter.blocks.get(row as usize))
            .map(|block| QString::from(&block.id))
            .unwrap_or_default()
    }

    pub fn index_of_id(&self, id: QString) -> i32 {
        let id = id.to_string();
        self.chapter
            .as_ref()
            .and_then(|chapter| chapter.blocks.iter().position(|block| block.id == id))
            .map(|index| index as i32)
            .unwrap_or(-1)
    }

    fn row_count(&self, parent: &QModelIndex) -> i32 {
        if parent.is_valid() {
            return 0;
        }
        self.chapter
            .as_ref()
            .map_or(0, |chapter| chapter.blocks.len() as i32)
    }

    fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        if !index.is_valid() || index.column() != 0 {
            return QVariant::default();
        }
        let Some(block) = self
            .chapter
            .as_ref()
            .and_then(|chapter| chapter.blocks.get(index.row() as usize))
        else {
            return QVariant::default();
        };
        match role - 256 {
            0 => QVariant::from(&QString::from(&block.id)),
            1 => QVariant::from(
                &(match block.kind {
                    TextBlockKind::Paragraph => 0i32,
                    TextBlockKind::Heading => 1,
                    TextBlockKind::Separator => 2,
                }),
            ),
            2 => QVariant::from(&QString::from(block.plain_text())),
            3 => QVariant::from(&QString::from(styled_text(&block.spans))),
            _ => QVariant::default(),
        }
    }

    fn role_names(&self) -> ffi::QHash_i32_QByteArray {
        let mut roles = QHash::default();
        for (index, name) in ["blockId", "blockKind", "plainText", "richText"]
            .iter()
            .enumerate()
        {
            roles.insert(256 + index as i32, QByteArray::from(*name));
        }
        roles
    }

    pub fn replace(mut self: Pin<&mut Self>, chapter: Option<Arc<TextChapter>>) {
        self.as_mut().rust_mut().pending = Some(chapter);
        if self.replacing {
            return;
        }
        self.as_mut().rust_mut().replacing = true;
        while let Some(chapter) = self.as_mut().rust_mut().pending.take() {
            self.as_mut().begin_reset_model();
            self.as_mut().rust_mut().chapter = chapter;
            self.as_mut().end_reset_model();
        }
        self.as_mut().rust_mut().replacing = false;
    }
}

fn styled_text(spans: &[TextSpan]) -> String {
    let mut output = String::new();
    for span in spans {
        match span {
            TextSpan::Text { text, bold, italic } => {
                if *bold {
                    output.push_str("<b>");
                }
                if *italic {
                    output.push_str("<i>");
                }
                escape_text(text, &mut output);
                if *italic {
                    output.push_str("</i>");
                }
                if *bold {
                    output.push_str("</b>");
                }
            }
            TextSpan::Ruby { text, reading } => {
                escape_text(text, &mut output);
                if !reading.is_empty() {
                    output.push('(');
                    escape_text(reading, &mut output);
                    output.push(')');
                }
            }
        }
    }
    output
}

fn escape_text(text: &str, output: &mut String) {
    for character in text.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '\n' => output.push_str("<br>"),
            _ => output.push(character),
        }
    }
}
