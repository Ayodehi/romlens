import Foundation
import RomlensKit
import Testing
@testable import Romlens

/// A live session through the shell: the recorder's stream arriving in the
/// graphics model as frames it follows.
///
/// Fed by `LiveSession.replay`, which reads a stream exactly as a connection
/// would, because the sandboxed test host may listen but not connect; the
/// socket itself is covered by the core's `tests/live_session.rs`.
@MainActor
@Suite(.serialized) struct LiveSessionTests {
    @Test func framesArriveAndAreFollowed() async throws {
        let rom = try Fixture.smallRom()
        let graphics = GraphicsModel(rom: rom)
        let session = try LiveSession.replay(
            rom: rom, stream: makeTestStream(rom: rom, frames: 8, execLog: nil), listener: LiveBridge(graphics: graphics))
        defer { session.stop() }
        try graphics.attachLive(session)
        #expect(graphics.isLive)

        do {
            try await Fixture.settle(until: { graphics.frameCount == 8 && graphics.frame == 7 })
        } catch {
            Issue.record("frameCount \(graphics.frameCount) frame \(graphics.frame) follow \(graphics.followLive) status \(graphics.liveStatus ?? "-") latest \(String(describing: session.latestFrame())) info \(String(describing: graphics.recordingInfo?.frameCount))")
            throw error
        }
        #expect(graphics.followLive)
        // Frame n wrote n + 1 into VRAM block n (`encode::fixture`).
        #expect(graphics.regionBytes(.vram)?[7 * 256] == 8)
        do {
            try await Fixture.settle(until: { graphics.liveStatus?.contains("stopped") == true })
        } catch {
            Issue.record("status \(graphics.liveStatus ?? "-") session \(String(describing: session.status()))")
            throw error
        }
        #expect(graphics.sourceDescription.hasPrefix("Live, frame 7"))

        // Stepping back pauses following, so a newer frame does not move it;
        // stepping back to the newest resumes.
        graphics.step(by: -3)
        #expect(!graphics.followLive)
        graphics.liveArrived(latest: 7)
        #expect(graphics.frame == 4)
        graphics.step(by: 3)
        #expect(graphics.followLive)

        graphics.stopLive()
        #expect(!graphics.isLive)
        #expect(graphics.recordingName == "Live (stopped)")
        #expect(graphics.regionBytes(.vram) != nil, "the frames received stay readable")
    }

    /// Live code discovery: the execution log the stream carries joins the
    /// project as it arrives, without costing the undo history.
    @Test func theLiveExecutionLogJoinsTheProject() async throws {
        let m = try await Fixture.analyzedModel(rom: Fixture.smallRom())
        m.select(offset: 0)
        try m.setLabel(name: "Boot")
        #expect(m.session.canUndo)
        let before = m.session.analysisGeneration
        let workbench = m.workbench
        let bridge = LiveBridge(
            graphics: m.graphics,
            merge: { log in try? workbench.mergeLiveLog(source: "live session", bytes: log) },
            merged: { added in
                m.session.liveLogMerged()
                m.graphics.liveLogMerged(added: added)
            }
        )
        let stream = makeTestStream(rom: m.rom, frames: 3, execLog: Fixture.execLog(rom: [UInt8](makeTestRom(mapping: .loRom))))
        let session = try LiveSession.replay(rom: m.rom, stream: stream, listener: bridge)
        defer { session.stop() }
        try m.graphics.attachLive(session)

        try await Fixture.settle(until: { m.graphics.liveDiscovered == 2 })
        try await Fixture.settle(until: { m.session.analysisGeneration > before && !m.session.analysis.isRunning })
        let target = try #require(m.rom.snesAddressFor(fileOffset: 0x0C))
        #expect(m.workbench.xrefsTo(snesAddress: target).contains { $0.observed && $0.kindName == "call" })
        #expect(m.session.canUndo, "a live merge keeps the undo history")
        #expect(m.graphics.sourceDescription.contains("2 instructions found"))
    }

    @Test func aStreamOfAnotherRomIsRefused() async throws {
        let rom = try Fixture.smallRom()
        let other = try Rom.fromBytes(bytes: makeTestRom(mapping: .hiRom), name: "other.sfc")
        let graphics = GraphicsModel(rom: rom)
        let session = try LiveSession.replay(
            rom: rom, stream: makeTestStream(rom: other, frames: 3, execLog: nil), listener: LiveBridge(graphics: graphics))
        defer { session.stop() }
        try graphics.attachLive(session)
        try await Fixture.settle(until: { graphics.liveStatus?.hasPrefix("refused") == true })
        #expect(graphics.frameCount == 0)
    }

    /// The app may listen: the sandbox needs `network.server` for that.
    @Test func theAppCanListen() throws {
        let rom = try Fixture.smallRom()
        let session = try LiveSession.start(rom: rom, port: 0, listener: LiveBridge(graphics: GraphicsModel(rom: rom)))
        #expect(session.port() > 0)
        #expect(session.isListening())
        session.stop()
        #expect(!session.isListening())
    }
}
