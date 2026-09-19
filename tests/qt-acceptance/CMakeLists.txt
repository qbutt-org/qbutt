add_executable(qbutt-qt-acceptance
    main.cpp
    ${qBittorrent_SOURCE_DIR}/src/app/application.h
    ${qBittorrent_SOURCE_DIR}/src/app/application.cpp
    ${qBittorrent_SOURCE_DIR}/src/app/applicationinstancemanager.h
    ${qBittorrent_SOURCE_DIR}/src/app/applicationinstancemanager.cpp
    ${qBittorrent_SOURCE_DIR}/src/app/cmdoptions.h
    ${qBittorrent_SOURCE_DIR}/src/app/cmdoptions.cpp
    ${qBittorrent_SOURCE_DIR}/src/app/filelogger.h
    ${qBittorrent_SOURCE_DIR}/src/app/filelogger.cpp
    ${qBittorrent_SOURCE_DIR}/src/app/legalnotice.h
    ${qBittorrent_SOURCE_DIR}/src/app/legalnotice.cpp
    ${qBittorrent_SOURCE_DIR}/src/app/qtlocalpeer/qtlocalpeer.h
    ${qBittorrent_SOURCE_DIR}/src/app/qtlocalpeer/qtlocalpeer.cpp
    ${qBittorrent_SOURCE_DIR}/src/app/signalhandler.h
    ${qBittorrent_SOURCE_DIR}/src/app/signalhandler.cpp
    ${qBittorrent_SOURCE_DIR}/src/app/upgrade.h
    ${qBittorrent_SOURCE_DIR}/src/app/upgrade.cpp
    ${qBittorrent_SOURCE_DIR}/src/icons/icons.qrc
)

target_include_directories(qbutt-qt-acceptance PRIVATE
    ${qBittorrent_SOURCE_DIR}/src
    ${qBittorrent_SOURCE_DIR}/src/app
)

target_link_libraries(qbutt-qt-acceptance PRIVATE qbt_base qbt_gui)
if (WEBUI)
    target_link_libraries(qbutt-qt-acceptance PRIVATE qbt_webui)
endif()

if (STACKTRACE)
    target_compile_definitions(qbutt-qt-acceptance PRIVATE STACKTRACE)
    target_sources(qbutt-qt-acceptance PRIVATE
        ${qBittorrent_SOURCE_DIR}/src/app/stacktrace.h
        ${qBittorrent_SOURCE_DIR}/src/app/stacktrace.cpp
    )
endif()

set_target_properties(qbutt-qt-acceptance PROPERTIES
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}"
)
