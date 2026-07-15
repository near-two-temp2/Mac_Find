import Foundation
import AppKit
import Carbon.HIToolbox

/// Registers a single system-wide hotkey via the Carbon Hot Key API (the only
/// dependency-free way to do global hotkeys on macOS). Default is ⌥⌘Space, the
/// same "summon the search bar" gesture Spotlight-style tools use.
///
/// The handler runs on the main thread; wire it to bring the window forward.
final class GlobalHotkey {

    static let shared = GlobalHotkey()

    private var ref: EventHotKeyRef?
    private var handler: (() -> Void)?
    private var installed = false

    private init() {}

    /// keyCode 49 = Space; modifiers default to ⌥ (option) + ⌘ (command).
    func register(keyCode: UInt32 = 49,
                  modifiers: UInt32 = UInt32(optionKey | cmdKey),
                  handler: @escaping () -> Void) {
        self.handler = handler
        if !installed { installEventHandler(); installed = true }

        let hotKeyID = EventHotKeyID(signature: fourCharCode("MHFC"), id: 1)
        var newRef: EventHotKeyRef?
        let status = RegisterEventHotKey(keyCode, modifiers, hotKeyID, GetApplicationEventTarget(), 0, &newRef)
        if status == noErr { ref = newRef }
    }

    private func installEventHandler() {
        var spec = EventTypeSpec(eventClass: OSType(kEventClassKeyboard),
                                 eventKind: UInt32(kEventHotKeyPressed))
        let selfPtr = Unmanaged.passUnretained(self).toOpaque()
        InstallEventHandler(GetApplicationEventTarget(), { _, _, userData in
            guard let userData else { return noErr }
            let me = Unmanaged<GlobalHotkey>.fromOpaque(userData).takeUnretainedValue()
            DispatchQueue.main.async { me.handler?() }
            return noErr
        }, 1, &spec, selfPtr, nil)
    }

    func unregister() {
        if let ref { UnregisterEventHotKey(ref); self.ref = nil }
    }
}

private func fourCharCode(_ s: String) -> OSType {
    var result: OSType = 0
    for ch in s.utf8.prefix(4) { result = (result << 8) + OSType(ch) }
    return result
}
