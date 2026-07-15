# PyInstaller spec for MacFind Road_C (Python) — builds a macOS .app bundle.
#
# Build locally or in CI with:
#     pyinstaller --noconfirm MacFindRoadC.spec
#
# Produces:
#     dist/MacFindRoadCPython.app   (double-clickable GUI)
#
# The entry script is macfind_c/app.py, which launches the PyQt6 GUI by default
# and routes `index`/`search`/`status` args to the headless CLI.

# -*- mode: python ; coding: utf-8 -*-

block_cipher = None


a = Analysis(
    ["macfind_c/app.py"],
    pathex=[],
    binaries=[],
    datas=[],
    hiddenimports=["macfind_c.gui", "macfind_c.cli"],
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    # Trim heavy PyQt6 modules we do not use to keep the bundle small.
    excludes=[
        "PyQt6.QtWebEngineCore",
        "PyQt6.QtWebEngineWidgets",
        "PyQt6.QtBluetooth",
        "PyQt6.QtMultimedia",
        "PyQt6.QtQuick",
        "PyQt6.QtQml",
        "PyQt6.Qt3DCore",
        "tkinter",
    ],
    win_no_prefer_redirects=False,
    win_private_assemblies=False,
    cipher=block_cipher,
    noarchive=False,
)

pyz = PYZ(a.pure, a.zipped_data, cipher=block_cipher)

exe = EXE(
    pyz,
    a.scripts,
    [],
    exclude_binaries=True,
    name="MacFindRoadCPython",
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=False,
    console=False,  # GUI app — no terminal window
    disable_windowed_traceback=False,
    argv_emulation=True,  # let macOS pass file-open args to argv
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
)

coll = COLLECT(
    exe,
    a.binaries,
    a.zipfiles,
    a.datas,
    strip=False,
    upx=False,
    upx_exclude=[],
    name="MacFindRoadCPython",
)

app = BUNDLE(
    coll,
    name="MacFindRoadCPython.app",
    icon=None,
    bundle_identifier="com.macfind.roadc.python",
    info_plist={
        "CFBundleName": "MacFind Road C",
        "CFBundleDisplayName": "MacFind Road C (Python)",
        "CFBundleShortVersionString": "0.1.0",
        "NSHighResolutionCapable": True,
        # Ask for Full Disk Access rationale so searchfs()/scans see more.
        "NSDesktopFolderUsageDescription": "MacFind searches your files.",
        "NSDocumentsFolderUsageDescription": "MacFind searches your files.",
        "NSDownloadsFolderUsageDescription": "MacFind searches your files.",
    },
)
