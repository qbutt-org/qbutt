include(FetchContent)
set(rustToolchainFile "${CMAKE_CURRENT_SOURCE_DIR}/../../../rust-toolchain.toml")
set_property(DIRECTORY APPEND PROPERTY CMAKE_CONFIGURE_DEPENDS "${rustToolchainFile}")
file(STRINGS "${rustToolchainFile}" rustChannelLine REGEX "^channel = \"[0-9]+\\.[0-9]+\\.[0-9]+\"$")
if (NOT rustChannelLine MATCHES "\"([0-9]+\\.[0-9]+\\.[0-9]+)\"")
    message(FATAL_ERROR "rust-toolchain.toml must pin an exact Rust release")
endif()
set(Rust_TOOLCHAIN "${CMAKE_MATCH_1}")

FetchContent_Declare(Corrosion
    GIT_REPOSITORY https://github.com/corrosion-rs/corrosion.git
    GIT_TAG a1a1aaa057a5da656c06c3d8505b767a4e941709
)
FetchContent_Declare(CxxQt
    GIT_REPOSITORY https://github.com/KDAB/cxx-qt-cmake.git
    GIT_TAG 64e63f560737b3b7f85ef8267c5918e4b8176d5a
)
FetchContent_MakeAvailable(CxxQt)

cxx_qt_import_crate(
    MANIFEST_PATH "${CMAKE_CURRENT_SOURCE_DIR}/bridge/Cargo.toml"
    CRATES qbutt_quick
    LOCKED
    QT_MODULES Qt::Core Qt::Gui Qt::Qml
)
cxx_qt_import_qml_module(qbt_content_plugin
    URI Qbutt.Content
    SOURCE_CRATE qbutt_quick
)
if (WIN32)
    target_link_libraries(qbutt_quick
        INTERFACE
            bcrypt
            ntdll
    )
endif()

qt_add_library(qbt_quick STATIC
    contentview.cpp
    contentview.h
    readerimageprovider.cpp
    readerimageprovider.h
)
target_link_libraries(qbt_quick
    PRIVATE qbutt_quick qbt_content_plugin
    PUBLIC Qt::QuickWidgets
)

add_subdirectory(qml)
target_link_libraries(qbt_quick PRIVATE qbt_quick_ui qbt_quick_uiplugin)
