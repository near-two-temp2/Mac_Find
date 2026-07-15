cmake_minimum_required(VERSION 3.21)

project(MacFindRoadCCpp
    VERSION 0.1.0
    LANGUAGES CXX
    DESCRIPTION "Road C (C++): macOS hybrid file search — binary index + searchfs() fallback")

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

if(NOT APPLE)
    message(FATAL_ERROR "This project uses searchfs()/fsgetpath()/fts and only builds on macOS.")
endif()

# --- Shared hybrid engine (index + searchfs fallback, no Qt) ---
# Kept Qt-free so the CLI, the GUI, and CI can all link the same core.
add_library(hybrid_engine STATIC
    IndexEngine.cpp     IndexEngine.h
    SearchFsEngine.cpp  SearchFsEngine.h
    HybridEngine.cpp    HybridEngine.h
    SearchTypes.h)
target_include_directories(hybrid_engine PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})

# --- CLI smoke-test binary (no Qt required) ---
add_executable(macfind-c-cli main_cli.cpp)
target_link_libraries(macfind-c-cli PRIVATE hybrid_engine)

# --- Qt6 GUI (.app bundle) ---
# Optional so the CLI/engine still build if Qt is somehow absent from the runner.
find_package(Qt6 COMPONENTS Widgets QUIET)

if(Qt6_FOUND)
    set(CMAKE_AUTOMOC ON)

    qt_add_executable(MacFindRoadCCpp MACOSX_BUNDLE
        main_gui.cpp)
    target_link_libraries(MacFindRoadCCpp PRIVATE hybrid_engine Qt6::Widgets)

    set_target_properties(MacFindRoadCCpp PROPERTIES
        MACOSX_BUNDLE TRUE
        MACOSX_BUNDLE_BUNDLE_NAME "Mac_Find Road C (C++)"
        MACOSX_BUNDLE_GUI_IDENTIFIER "org.macfind.roadc.cpp"
        MACOSX_BUNDLE_BUNDLE_VERSION "${PROJECT_VERSION}"
        MACOSX_BUNDLE_SHORT_VERSION_STRING "${PROJECT_VERSION}")
else()
    message(WARNING "Qt6 not found — building CLI/engine only, skipping GUI .app.")
endif()
