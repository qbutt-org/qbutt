cmake_minimum_required(VERSION 3.28)

if(CMAKE_SOURCE_DIR STREQUAL CMAKE_CURRENT_SOURCE_DIR)
    project(qbutt_quick_acceptance LANGUAGES CXX)

    set(CMAKE_CXX_STANDARD 20)
    set(CMAKE_CXX_STANDARD_REQUIRED ON)
    set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreadedDLL")
    set(minQt6Version 6.12)

    find_package(Qt6 ${minQt6Version} REQUIRED COMPONENTS Quick QuickControls2 QuickWidgets)
    qt_standard_project_setup(REQUIRES 6.12)

    add_subdirectory("../../src/gui/quick" "quick")
endif()

qt_add_executable(qbutt-quick-acceptance main.cpp)
target_link_libraries(qbutt-quick-acceptance
    PRIVATE
        qbt_quick
        qbutt_quick
)
target_include_directories(qbutt-quick-acceptance PRIVATE "../../src")

if(MSVC)
    target_compile_options(qbutt-quick-acceptance PRIVATE /utf-8)
endif()
