cmake_minimum_required(VERSION 3.22)

if(CMAKE_SOURCE_DIR STREQUAL CMAKE_CURRENT_SOURCE_DIR)
    project(qbutt_quick_acceptance LANGUAGES CXX)

    set(CMAKE_CXX_STANDARD 20)
    set(CMAKE_CXX_STANDARD_REQUIRED ON)
    set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreadedDLL")
    set(minQt6Version 6.12)

    find_package(Qt6 ${minQt6Version} REQUIRED COMPONENTS Quick QuickControls2 QuickWidgets LinguistTools Sql)
    qt_standard_project_setup(REQUIRES 6.12)

    add_subdirectory("../../src/gui/quick" "quick")
    target_include_directories(qbt_quick PRIVATE "../../src")
endif()

get_filename_component(qbutt_product_lock_path "../../upstream-lock.json" ABSOLUTE BASE_DIR "${CMAKE_CURRENT_SOURCE_DIR}")
set_property(DIRECTORY APPEND PROPERTY CMAKE_CONFIGURE_DEPENDS "${qbutt_product_lock_path}")
file(READ "${qbutt_product_lock_path}" qbutt_product_lock)
string(JSON qbutt_qplay_minimum_version GET "${qbutt_product_lock}" products qplay version)
string(JSON qbutt_qplay_protocol_version GET "${qbutt_product_lock}" products qplay protocolVersion)
set_property(SOURCE ../../src/base/media/runtime.cpp APPEND PROPERTY COMPILE_DEFINITIONS
    "QBUTT_QPLAY_MINIMUM_VERSION=\"${qbutt_qplay_minimum_version}\""
    "QBUTT_QPLAY_PROTOCOL_VERSION=${qbutt_qplay_protocol_version}"
)

qt_add_executable(qbutt-qcatch-cli-fixture qcatch-fixture.cpp)
set_target_properties(qbutt-qcatch-cli-fixture PROPERTIES
    OUTPUT_NAME qcatch
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_CURRENT_BINARY_DIR}/qcatch-fixture"
)
target_link_libraries(qbutt-qcatch-cli-fixture PRIVATE Qt::Core)

qt_add_executable(qbutt-codex-text-peer codex-text-peer.cpp)
set_target_properties(qbutt-codex-text-peer PROPERTIES
    OUTPUT_NAME codex
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_CURRENT_BINARY_DIR}/codex-text-peer"
)
target_link_libraries(qbutt-codex-text-peer PRIVATE Qt::Core)
if(MSVC)
    target_compile_options(qbutt-codex-text-peer PRIVATE /utf-8)
endif()

qt_add_executable(qbutt-quick-acceptance
    main.cpp
    ../../src/base/media/runtime.cpp
    ../../src/base/media/runtime.h
    ../../src/gui/content/browsercapture.cpp
    ../../src/gui/content/browsercapture.h
    browser-relay.cpp
    browser-relay.h
    browser-registration.cpp
    browser-registration.h
    category.cpp
    category.h
    downloads.cpp
    downloads.h
    library.cpp
    library.h
    local-books-ui.cpp
    local-books-ui.h
    playback.cpp
    playback.h
    mangalib.cpp
    mangalib.h
    weebcentral.cpp
    weebcentral.h
    history.cpp
    history.h
    reader.cpp
    reader.h
    reader-text.cpp
    reader-text.h
    reader-continuous.cpp
    reader-continuous.h
    reader-progress.cpp
    reader-progress.h
    reader-settings.cpp
    reader-settings.h
    reader-settings-ui.cpp
    reader-settings-ui.h
    reader-translation-choice.cpp
    reader-translation-choice.h
    reader-text-translation.cpp
    reader-text-translation.h
    reader-text-ahead.cpp
    reader-text-ahead.h
    reader-manga-translation.cpp
    reader-manga-translation.h
    reader-longstrip.cpp
    reader-longstrip.h
    sharing.cpp
    sharing.h
    torrent-source.cpp
    torrent-source.h
    sources.cpp
    sources.h
    variant-filters.cpp
    variant-filters.h
    work-details.cpp
    work-details.h
    video-actions.cpp
    video-actions.h
    work-filter.cpp
    work-filter.h
    worklist.cpp
    worklist.h
)
target_compile_definitions(qbutt-quick-acceptance PRIVATE QT_QML_DEBUG)
set_target_properties(qbutt-quick-acceptance PROPERTIES AUTORCC ON)
target_sources(qbutt-quick-acceptance PRIVATE "../../src/icons/icons.qrc")
qt_add_translation(QUICK_TRANSLATIONS "../../src/lang/qbittorrent_ru.ts")
qt_add_resources(qbutt-quick-acceptance "quick_translations"
    PREFIX "/lang"
    BASE "${CMAKE_CURRENT_BINARY_DIR}"
    FILES ${QUICK_TRANSLATIONS}
)
target_link_libraries(qbutt-quick-acceptance
    PRIVATE
        qbt_quick
        qbutt_quick
        Qt::Sql
        Qt::Network
)
if(WIN32)
    target_link_libraries(qbutt-quick-acceptance PRIVATE advapi32)
endif()
target_include_directories(qbutt-quick-acceptance PRIVATE "../../src")

if(MSVC)
    target_compile_options(qbutt-quick-acceptance PRIVATE /utf-8)
endif()
