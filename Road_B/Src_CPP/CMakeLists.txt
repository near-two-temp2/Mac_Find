cmake_minimum_required(VERSION 3.21)
project(MacFindRoadBCpp LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

if(NOT CMAKE_BUILD_TYPE)
  set(CMAKE_BUILD_TYPE Release)
endif()

# ---- core engine (no Qt dependency) --------------------------------------
# Kept framework-agnostic so the CLI and any GUI share one implementation.
add_library(mffb_engine STATIC
  src/scanner.cpp
  src/index_engine.cpp
)
target_include_directories(mffb_engine PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/src)
target_compile_options(mffb_engine PRIVATE -O2 -Wall -Wextra)

# ---- CLI (index / search / selftest) -------------------------------------
# Always built: gives CI a fast, GUI-free smoke test target.
add_executable(mff-b src/main_cli.cpp)
target_link_libraries(mff-b PRIVATE mffb_engine)

# ---- Qt6 GUI (.app) -------------------------------------------------------
# Optional so the project still configures on machines without Qt; CI installs
# Qt6 and expects the bundle to build.
set(CMAKE_AUTOMOC ON)
find_package(Qt6 COMPONENTS Widgets QUIET)

if(Qt6_FOUND)
  message(STATUS "Qt6 found — building GUI app bundle")
  qt_standard_project_setup()
  qt_add_executable(MacFindRoadB MACOSX_BUNDLE src/main_gui.cpp)
  target_link_libraries(MacFindRoadB PRIVATE mffb_engine Qt6::Widgets)
  set_target_properties(MacFindRoadB PROPERTIES
    MACOSX_BUNDLE TRUE
    MACOSX_BUNDLE_BUNDLE_NAME "MacFindRoadB"
    MACOSX_BUNDLE_GUI_IDENTIFIER "com.mff.roadb-cpp"
    MACOSX_BUNDLE_BUNDLE_VERSION "0.1.0"
    MACOSX_BUNDLE_SHORT_VERSION_STRING "0.1"
  )
else()
  message(WARNING "Qt6 not found — building CLI only (GUI skipped)")
endif()
