import Foundation
import CoreServices

/// Thin FSEventsStream wrapper for incremental index freshness.
///
/// It does not rewrite the mmap index in place (that would need a compacting
/// writer — see README TODO); instead it debounces change notifications and
/// asks the owner to schedule a rebuild, which is the pragmatic first cut of the
/// "FSEvents 增量更新" requirement.
public final class FSEventsWatcher {

    private var stream: FSEventStreamRef?
    private let paths: [String]
    private let latency: CFTimeInterval
    private let onChange: () -> Void
    private let queue = DispatchQueue(label: "com.machaifind.c.fsevents")

    public init(paths: [String], latency: CFTimeInterval = 2.0, onChange: @escaping () -> Void) {
        self.paths = paths
        self.latency = latency
        self.onChange = onChange
    }

    public func start() {
        queue.async { [weak self] in self?.startLocked() }
    }

    private func startLocked() {
        guard stream == nil else { return }

        var ctx = FSEventStreamContext(
            version: 0,
            info: Unmanaged.passUnretained(self).toOpaque(),
            retain: nil, release: nil, copyDescription: nil
        )

        let cb: FSEventStreamCallback = { _, info, _, _, _, _ in
            guard let info else { return }
            let watcher = Unmanaged<FSEventsWatcher>.fromOpaque(info).takeUnretainedValue()
            watcher.onChange()
        }

        let flags = UInt32(kFSEventStreamCreateFlagFileEvents | kFSEventStreamCreateFlagNoDefer)
        guard let s = FSEventStreamCreate(
            kCFAllocatorDefault, cb, &ctx,
            paths as CFArray,
            FSEventStreamEventId(kFSEventStreamEventIdSinceNow),
            latency, flags
        ) else { return }

        stream = s
        FSEventStreamSetDispatchQueue(s, queue)
        FSEventStreamStart(s)
    }

    public func stop() {
        queue.async { [weak self] in
            guard let self, let s = self.stream else { return }
            FSEventStreamStop(s)
            FSEventStreamInvalidate(s)
            FSEventStreamRelease(s)
            self.stream = nil
        }
    }

    deinit { stop() }
}
