use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    let builder =
        CxxQtBuilder::new_qml_module(QmlModule::new("Qbutt.Content").depend("QtQml.Models"))
            .qt_module("Qml")
            .qt_module("Gui")
            .files(["src/controller.rs", "src/models.rs", "src/reader_models.rs"])
            .cpp_file("src/native.cpp")
            .crate_include_root(Some("include".into()));

    // SAFETY: This only matches CMake's UTF-8 source and execution character sets.
    let builder = unsafe {
        builder.cc_builder(|cc| {
            cc.flag_if_supported("/utf-8");
        })
    };
    builder.build();
}
