# -*- mode: python ; coding: utf-8 -*-
# PyInstaller spec for the Road_A Python GUI (.app bundle).
#
# Build:  pyinstaller --noconfirm MacFindA.spec
# Output: dist/MacFindA.app

block_cipher = None

a = Analysis(
    ['main.py'],
    pathex=['.'],
    binaries=[],
    datas=[],
    hiddenimports=['mac_find_a', 'mac_find_a.app', 'mac_find_a.searchfs_engine'],
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[],
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
    name='MacFindA',
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=False,
    console=False,
    disable_windowed_traceback=False,
    argv_emulation=False,
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
    name='MacFindA',
)

app = BUNDLE(
    coll,
    name='MacFindA.app',
    icon=None,
    bundle_identifier='org.macfind.roada.python',
    info_plist={
        'CFBundleName': 'Mac Find (Road A)',
        'CFBundleDisplayName': 'Mac Find — Road A',
        'CFBundleShortVersionString': '0.1.0',
        'NSHighResolutionCapable': True,
        # searchfs walks the whole volume; Full Disk Access gives full results.
        'NSDesktopFolderUsageDescription': 'Search filenames across your disk.',
        # PyQt6 6.11 wheels require macOS 13+ at runtime.
        'LSMinimumSystemVersion': '13.0',
    },
)
