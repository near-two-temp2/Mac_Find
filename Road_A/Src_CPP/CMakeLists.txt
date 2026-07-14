cmake_minimum_required(VERSION 3.21)

project(MacFindRoadACpp
    VERSION 0.1.0
    LANGUAGES CXX
    DESCRIPTION "Road A (C++): macOS real-time filename search via searchfs()")

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

if(NOT APPLE)
    message(FATAL_ERROR "This project uses searchfs()/fsgetpath() and only builds on macOS.")
endif()

# --- Shared engine (searchfs wrapper, no Qt) ---
add_library(searchfs_engine STATIC
    SearchEngine.cpp
    SearchEngine.h)
target_include_directories(searchfs_engine PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})

# --- CLI smoke-test binary (no Qt required) ---
add_executable(macfind-a-cli main_cli.cpp)
target_link_libraries(macfind-a-cli PRIVATE searchfs_engine)

# --- Qt6 GUI (.app bundle) ---
# Optional so the CLI/engine can still build if Qt is absent (e.g. odd CI).
find_package(Qt6 COMPONENTS Widgets QUIET)

if(Qt6_FOUND)
    set(CMAKE_AUTOMOC ON)

    qt_add_executable(MacFindRoadACpp MACOSX_BUNDLE
        main_gui.cpp)
    target_link_libraries(MacFindRoadACpp PRIVATE searchfs_engine Qt6::Widgets)

    set_target_properties(MacFindRoadACpp PROPERTIES
        MACOSX_BUNDLE TRUE
        MACOSX_BUNDLE_BUNDLE_NAME "Mac_Find Road A (C++)"
        MACOSX_BUNDLE_GUI_IDENTIFIER "org.macfind.roada.cpp"
        MACOSX_BUNDLE_BUNDLE_VERSION "${PROJECT_VERSION}"
        MACOSX_BUNDLE_SHORT_VERSION_STRING "${PROJECT_VERSION}")
else()
    message(WARNING "Qt6 not found — building CLI/engine only, skipping GUI .app.")
endif()
