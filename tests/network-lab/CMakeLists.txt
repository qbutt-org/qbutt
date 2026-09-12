cmake_minimum_required(VERSION 3.16)
project(qbutt_selector_integration LANGUAGES CXX)

find_package(LibtorrentRasterbar REQUIRED)
add_executable(selector-integration
    selector.cpp
    ../../src/base/net/peerrouteselector.cpp
)
target_compile_features(selector-integration PRIVATE cxx_std_20)
target_link_libraries(selector-integration PRIVATE LibtorrentRasterbar::torrent-rasterbar)
