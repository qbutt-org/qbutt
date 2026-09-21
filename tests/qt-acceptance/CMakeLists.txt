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

find_package(Qt6 ${minQt6Version} REQUIRED COMPONENTS Widgets Network)
foreach(target qbutt-update-acceptance qbutt-update-live-acceptance)
    add_executable(${target}
        release-updater.cpp
        ${qBittorrent_SOURCE_DIR}/src/base/releaseupdater.cpp
        ${qBittorrent_SOURCE_DIR}/src/base/releaseupdater.h
        ${qBittorrent_SOURCE_DIR}/src/gui/releaseupdatedialog.cpp
        ${qBittorrent_SOURCE_DIR}/src/gui/releaseupdatedialog.h
    )
    target_include_directories(${target} PRIVATE ${qBittorrent_SOURCE_DIR}/src)
    target_link_libraries(${target} PRIVATE qbt_common_cfg Qt6::Widgets Qt6::Network OpenSSL::Crypto)
    set_target_properties(${target} PROPERTIES RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}")
endforeach()
target_compile_definitions(qbutt-update-acceptance PRIVATE QBUTT_UPDATE_ACCEPTANCE)
set_target_properties(qbutt-update-live-acceptance PROPERTIES EXCLUDE_FROM_ALL TRUE)
