use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    CxxQtBuilder::new_qml_module(QmlModule::new("Qbutt.Content"))
        .qt_module("Qml")
        .qt_module("Gui")
        .files(["src/controller.rs", "src/models.rs", "src/reader_models.rs"])
        .cpp_file("src/native.cpp")
        .crate_include_root(Some("include".into()))
        .build();
}
