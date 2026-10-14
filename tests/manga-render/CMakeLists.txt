cmake_minimum_required(VERSION 3.22)

project(qbutt_manga_render_acceptance LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTORCC ON)

find_package(Qt6 6.12 REQUIRED COMPONENTS Gui)
qt_standard_project_setup(REQUIRES 6.12)

qt_add_executable(qbutt-manga-render-acceptance
    main.cpp
    longstrip.cpp
    longstrip.h
    ../../src/gui/content/mangarender.cpp
    ../../src/gui/content/mangarender.h
    ../../src/gui/content/reader.qrc
)

target_include_directories(qbutt-manga-render-acceptance PRIVATE ../../src)
target_link_libraries(qbutt-manga-render-acceptance PRIVATE Qt6::Gui)
