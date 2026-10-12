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
endif()

qt_add_executable(qbutt-quick-acceptance
    main.cpp
    category.cpp
    category.h
    library.cpp
    library.h
    playback.cpp
    playback.h
    history.cpp
    history.h
    reader.cpp
    reader.h
    reader-continuous.cpp
    reader-continuous.h
    reader-progress.cpp
    reader-progress.h
    reader-settings.cpp
    reader-settings.h
    reader-settings-ui.cpp
    reader-settings-ui.h
    torrent-source.cpp
    torrent-source.h
    sources.cpp
    sources.h
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
target_include_directories(qbutt-quick-acceptance PRIVATE "../../src")

if(MSVC)
    target_compile_options(qbutt-quick-acceptance PRIVATE /utf-8)
endif()
