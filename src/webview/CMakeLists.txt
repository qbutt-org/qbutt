cmake_minimum_required(VERSION 3.16)
if(CMAKE_SOURCE_DIR STREQUAL CMAKE_CURRENT_SOURCE_DIR)
    project(qbutt-webview LANGUAGES CXX)
    find_package(Qt6 REQUIRED COMPONENTS Core Network)
endif()

if(NOT WIN32)
    return()
endif()

set(QBUTT_WEBVIEW2_SDK "" CACHE PATH "Verified Microsoft.Web.WebView2 SDK directory")
if(NOT EXISTS "${QBUTT_WEBVIEW2_SDK}/build/native/include/WebView2.h")
    message(FATAL_ERROR "QBUTT_WEBVIEW2_SDK must point to the pinned, verified WebView2 SDK")
endif()

add_executable(qbutt-webview WIN32 main.cpp session.cpp session.h)
target_compile_features(qbutt-webview PRIVATE cxx_std_20)
target_compile_definitions(qbutt-webview PRIVATE UNICODE _UNICODE NOMINMAX WIN32_LEAN_AND_MEAN)
target_include_directories(qbutt-webview PRIVATE "${QBUTT_WEBVIEW2_SDK}/build/native/include")
target_link_libraries(qbutt-webview PRIVATE Qt6::Core Qt6::Network user32 ole32 advapi32 shlwapi version
    "${QBUTT_WEBVIEW2_SDK}/build/native/x64/WebView2LoaderStatic.lib")
set_target_properties(qbutt-webview PROPERTIES RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}")
