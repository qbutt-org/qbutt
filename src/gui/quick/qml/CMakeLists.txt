qt_add_qml_module(qbt_quick_ui STATIC
    URI Qbutt.Views
    OUTPUT_DIRECTORY "${CMAKE_CURRENT_BINARY_DIR}/Qbutt/Views"
    IMPORT_PATH "${CMAKE_CURRENT_BINARY_DIR}/../cxxqt/qml_modules"
    SOURCES
        workfiltermodel.cpp
        workfiltermodel.h
        variantfiltermodel.cpp
        variantfiltermodel.h
    QML_FILES
        ActionButton.qml
        ChapterPicker.qml
        DownloadsPage.qml
        FacetField.qml
        IconButton.qml
        ModalPicker.qml
        BrowserSettingsModal.qml
        NavigationTab.qml
        QueryField.qml
        ReaderCanvas.qml
        ReaderAppearancePicker.qml
        ReaderChromeButton.qml
        ReaderColorEditor.qml
        ReaderColorField.qml
        ReaderColorPicker.qml
        ReaderColorSwatch.qml
        ReaderContinuousCanvas.qml
        ReaderFontPicker.qml
        ReaderImage.qml
        ReaderPage.qml
        ReaderStripPage.qml
        ReaderToolbar.qml
        ReaderTranslationPicker.qml
        SearchPage.qml
        SelectField.qml
        SourceList.qml
        SourcePicker.qml
        SourcesPage.qml
        TextReadingCanvas.qml
        Theme.qml
        VariantDelegate.qml
        VariantFilters.qml
        WorkDelegate.qml
        WorkDetailsModal.qml
        WorkPane.qml
)
target_link_libraries(qbt_quick_ui PRIVATE Qt::QuickControls2)
add_dependencies(qbt_quick_ui cargo-build_qbutt_quick)
add_dependencies(qbt_quick_ui_qmllint cargo-build_qbutt_quick)
