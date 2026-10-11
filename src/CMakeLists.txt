option(QBUTT_COMPLETION_FAULTS "Enable completion crash and Qt event-loop integration hooks" OFF)

include_directories(${CMAKE_CURRENT_SOURCE_DIR})

add_subdirectory(app)
add_subdirectory(base)

if (GUI)
    add_subdirectory(gui)
endif()

if (WIN32 AND GUI)
    add_subdirectory(webview)
endif()

if (WEBUI)
    add_subdirectory(webui)
endif()
