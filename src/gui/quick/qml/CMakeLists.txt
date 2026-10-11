qt_add_qml_module(qbt_quick_ui STATIC
    URI Qbutt.Views
    OUTPUT_DIRECTORY "${CMAKE_CURRENT_BINARY_DIR}/Qbutt/Views"
    IMPORT_PATH "${CMAKE_CURRENT_BINARY_DIR}/../cxxqt/qml_modules"
    QML_FILES
        ActionButton.qml
        NavigationTab.qml
        QueryField.qml
        SearchPage.qml
        Theme.qml
        VariantDelegate.qml
        WorkDelegate.qml
        WorkPane.qml
)
target_link_libraries(qbt_quick_ui PRIVATE Qt::QuickControls2)
add_dependencies(qbt_quick_ui qbutt_quick)
