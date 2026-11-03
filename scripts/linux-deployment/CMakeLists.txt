cmake_minimum_required(VERSION 3.22)

project(QbuttLinuxDeployment LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

find_package(Qt6 6.12 REQUIRED COMPONENTS
    Core
    Gui
    Widgets
    Qml
    Quick
    QuickWidgets
    QuickControls2
    Svg
    Sql
    WebEngineWidgets
)
qt_standard_project_setup(REQUIRES 6.12)

include("${QBUTT_BUILD_ROOT}/qml-plugins.cmake")
set(QT_DEPLOY_BIN_DIR ".")
set(QT_DEPLOY_LIB_DIR "lib")
set(QT_DEPLOY_LIBEXEC_DIR "libexec")
set(QT_DEPLOY_PLUGINS_DIR "plugins")
set(QT_DEPLOY_QML_DIR "qml")

# Keep the semicolon-separated QML plugin list inside the CONTENT argument.
qt6_generate_deploy_script(
    NAME qbutt
    OUTPUT_SCRIPT deploy_script
    CONTENT "
qt_deploy_runtime_dependencies(
    EXECUTABLE [==[${QBUTT_BUNDLE_ROOT}/qbutt]==]
    ADDITIONAL_EXECUTABLES
        qbutt-webview
    ADDITIONAL_MODULES ${QBUTT_QML_PLUGINS}
    INCLUDE_PLUGINS qoffscreen qwayland
    EXCLUDE_PLUGIN_TYPES qmltooling
    EXCLUDE_PLUGINS qsqlibase qsqlmimer qsqlmysql qsqloci qsqlodbc qsqlpsql qtposition_nmea
    POST_EXCLUDE_REGEXES
        \"/ld-linux[^/]*$\"
        \"/lib(c|m|dl|pthread|rt|resolv)\\\\.so[^/]*$\"
        \"/lib(GL|EGL|GLX|OpenGL|vulkan|drm|gbm)[^/]*\\\\.so[^/]*$\"
)
")
install(SCRIPT "${deploy_script}")
