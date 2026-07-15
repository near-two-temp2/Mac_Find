import Foundation
import SwiftUI
import AppKit

/// Notifications the AppKit shell posts and the SwiftUI model observes. Keeping
/// this as a loose coupling lets the (nonisolated) AppDelegate stay free of the
/// @MainActor `AppModel`, mirroring the structure that compiles cleanly on CI.
extension Notification.Name {
    static let mhfcRebuildIndex = Notification.Name("mhfc.rebuildIndex")
}

/// Boots the SwiftUI GUI. Called from main.swift when no CLI args are present.
///
/// This target is a plain SwiftPM executable, not an Xcode .app, so there is no
/// Info.plist telling LaunchServices this is a GUI app until CI wraps it in a
/// bundle. We create the NSApplication ourselves, promote it to a regular
/// (Dock-visible) app, install a menu-bar item + global hotkey, and host the
/// SwiftUI view — which owns the search model — in a standard window.
func runGUI() -> Never {
    let app = NSApplication.shared
    let delegate = AppDelegate()
    app.delegate = delegate
    app.setActivationPolicy(.regular)
    app.activate(ignoringOtherApps: true)
    app.run()
    exit(0) // NSApplication.run() never returns normally.
}

/// AppKit delegate: owns the window, the menu-bar status item, and the global
/// hotkey. Deliberately holds no reference to the @MainActor `AppModel` (the
/// view owns that) so it stays a plain nonisolated NSObject.
private final class AppDelegate: NSObject, NSApplicationDelegate {

    private var window: NSWindow?
    private var statusItem: NSStatusItem?

    func applicationDidFinishLaunching(_ notification: Notification) {
        installMainMenu()
        makeWindow()
        makeStatusItem()

        GlobalHotkey.shared.register { [weak self] in
            self?.toggleWindow()
        }
        NSApp.activate(ignoringOtherApps: true)
    }

    // MARK: Window

    private func makeWindow() {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 760, height: 520),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        window.title = "MacFind · Swift · Road_C"
        window.titlebarAppearsTransparent = true
        window.center()
        window.setFrameAutosaveName("MacHaiFindCMainWindow")
        window.contentView = NSHostingView(rootView: ContentView())
        window.isReleasedWhenClosed = false
        window.makeKeyAndOrderFront(nil)
        self.window = window
    }

    private func toggleWindow() {
        guard let w = window else { return }
        if w.isVisible && NSApp.isActive {
            w.orderOut(nil)
        } else {
            w.makeKeyAndOrderFront(nil)
            NSApp.activate(ignoringOtherApps: true)
        }
    }

    // MARK: Menu-bar status item

    private func makeStatusItem() {
        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        if let button = item.button {
            button.image = NSImage(systemSymbolName: "magnifyingglass", accessibilityDescription: "Mac Hai Find")
        }
        let menu = NSMenu()
        menu.addItem(withTitle: "显示搜索  (⌥⌘Space)", action: #selector(showFromMenu), keyEquivalent: "")
        menu.addItem(withTitle: "重建索引", action: #selector(rebuildFromMenu), keyEquivalent: "r")
        menu.addItem(.separator())
        menu.addItem(withTitle: "退出", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        for item in menu.items { item.target = self }
        item.menu = menu
        statusItem = item
    }

    @objc private func showFromMenu() {
        window?.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
    }

    @objc private func rebuildFromMenu() {
        NotificationCenter.default.post(name: .mhfcRebuildIndex, object: nil)
    }

    // MARK: Main menu (Cmd-Q / Cmd-W)

    private func installMainMenu() {
        let mainMenu = NSMenu()
        let appMenuItem = NSMenuItem()
        mainMenu.addItem(appMenuItem)
        let appMenu = NSMenu()
        let appName = ProcessInfo.processInfo.processName
        appMenu.addItem(withTitle: "关于 \(appName)",
                        action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)), keyEquivalent: "")
        appMenu.addItem(.separator())
        appMenu.addItem(withTitle: "隐藏 \(appName)",
                        action: #selector(NSApplication.hide(_:)), keyEquivalent: "h")
        appMenu.addItem(withTitle: "退出 \(appName)",
                        action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        appMenuItem.submenu = appMenu

        let windowMenuItem = NSMenuItem()
        mainMenu.addItem(windowMenuItem)
        let windowMenu = NSMenu(title: "窗口")
        windowMenu.addItem(withTitle: "关闭", action: #selector(NSWindow.performClose(_:)), keyEquivalent: "w")
        windowMenu.addItem(withTitle: "最小化", action: #selector(NSWindow.performMiniaturize(_:)), keyEquivalent: "m")
        windowMenuItem.submenu = windowMenu

        NSApplication.shared.mainMenu = mainMenu
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }
}
