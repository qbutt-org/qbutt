cmake_minimum_required(VERSION 3.22)
project(qbutt_linux_repair_fixture LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

if (NOT CMAKE_SYSTEM_NAME STREQUAL "Linux")
    message(FATAL_ERROR "This integration fixture requires Linux")
endif()

find_package(Qt6 6.6 REQUIRED COMPONENTS Core)
qt_standard_project_setup(REQUIRES 6.6)
find_package(LibtorrentRasterbar 2.0 REQUIRED)

qt_add_executable(qbutt-linux-repair-fixture
    main.cpp
    ../../../src/base/bittorrent/repairanalysis.cpp
    ../../../src/base/bittorrent/repairfileguard.cpp
    ../../../src/base/bittorrent/repairfileguard_linux.cpp
)
target_compile_options(qbutt-linux-repair-fixture PRIVATE
    -Wall
    -Wextra
    -Wpedantic
    -Werror
)
target_include_directories(qbutt-linux-repair-fixture PRIVATE
    ../../../src
)
target_link_libraries(qbutt-linux-repair-fixture PRIVATE
    Qt6::Core
    LibtorrentRasterbar::torrent-rasterbar
)
