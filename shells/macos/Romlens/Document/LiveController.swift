import AppKit
import RomlensKit

/// File › Start Live Session: listen for the recorder script's stream and
/// show it in the graphics views as it arrives (`romlens_core::recording::live`).
///
/// The session is a recording like any other to the views, so everything
/// that reads one works live. It listens on the loopback address only, and
/// refuses a stream recorded from another ROM.
@MainActor
enum LiveController {
    static func toggle(model: RomViewModel, window: NSWindow?) {
        if model.graphics.isLive {
            model.graphics.stopLive()
        } else {
            start(model: model, window: window)
        }
    }

    static func start(model: RomViewModel, window: NSWindow?) {
        let bridge = LiveBridge(graphics: model.graphics)
        do {
            let session = try LiveSession.start(rom: model.rom, port: liveDefaultPort(), listener: bridge)
            try model.graphics.attachLive(session)
            if model.graphicsTab == nil { model.graphicsTab = .tilemap }
        } catch {
            let alert = NSAlert()
            alert.messageText = "The live session could not start"
            alert.informativeText = "\(error.localizedDescription)\n\nAnother Romlens window may already be listening."
            alert.alertStyle = .warning
            if let window { alert.beginSheetModal(for: window) } else { alert.runModal() }
        }
    }
}

/// Receives the session's news on its own thread and passes it to the main
/// actor. Frames arrive sixty times a second; the views are told at most
/// thirty times, with only the newest frame, so rendering never falls behind.
final class LiveBridge: LiveListener, @unchecked Sendable {
    private weak var graphics: GraphicsModel?
    private let lock = NSLock()
    private var latest: UInt64?
    private var scheduled = false

    @MainActor init(graphics: GraphicsModel) {
        self.graphics = graphics
    }

    func onFrame(frame: UInt64) {
        let schedule: Bool = lock.withLock {
            latest = frame
            defer { scheduled = true }
            return !scheduled
        }
        guard schedule else { return }
        // Strong, not weak: the session releases its listener when its stream
        // ends, and the updates still queued must be delivered after that.
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.0 / 30) {
            let frame: UInt64? = self.lock.withLock {
                self.scheduled = false
                return self.latest
            }
            MainActor.assumeIsolated {
                if let frame { self.graphics?.liveArrived(latest: frame) }
            }
        }
    }

    func onStatus(status: LiveStatus) {
        DispatchQueue.main.async {
            MainActor.assumeIsolated { self.graphics?.liveStatusChanged(status) }
        }
    }
}
