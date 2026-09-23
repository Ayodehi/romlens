import AppKit
import Foundation
import RomlensKit
import Testing
@testable import Romlens

/// Shared fixtures: ROMs, analyzed view models and off-screen windows.
@MainActor
enum Fixture {
    static let romType = "io.github.ayodehi.romlens.sfc"
    static let projectType = "io.github.ayodehi.romlens.project"

    static func rom(megabytes: Int = 1) throws -> Rom {
        if let dir = ProcessInfo.processInfo.environment["ROMLENS_ROM_DIR"] {
            let url = URL(fileURLWithPath: dir).appendingPathComponent("SuperMetroid.F8DF.sfc")
            if FileManager.default.fileExists(atPath: url.path) {
                return try Rom.open(path: url.path)
            }
        }
        var bytes = makeTestRom(mapping: .loRom)
        bytes.append(Data(repeating: 0xEA, count: (megabytes << 20) - bytes.count))
        return try Rom.fromBytes(bytes: bytes, name: "big.sfc")
    }

    static func smallRom() throws -> Rom {
        try Rom.fromBytes(bytes: makeTestRom(mapping: .loRom), name: "t.sfc")
    }

    /// A view model whose first analysis has landed in the session.
    static func analyzedModel(rom: Rom) async throws -> RomViewModel {
        let model = RomViewModel(rom: rom, startAnalysis: false)
        _ = try await model.workbench.analyze()
        try await settle(until: { model.session.hasSnapshot && model.asmLineCount > 0 })
        return model
    }

    /// Spin the main run loop until a condition holds (or time runs out).
    static func settle(timeout: TimeInterval = 5, until condition: @MainActor () -> Bool) async throws {
        let deadline = Date().addingTimeInterval(timeout)
        while !condition() {
            if Date() > deadline { throw TimeoutError() }
            spin(0.01)
            await Task.yield()
        }
    }

    struct TimeoutError: Error {}

    /// An execution log of `rom`, as the fork's `emu.getExecutionLog()` writes
    /// it: the boot instruction at $00:8000 and an unreachable one at
    /// $00:800C it was seen to call, both with 8-bit A and X.
    static func execLog(rom: [UInt8]) -> Data {
        func le32(_ v: UInt32) -> [UInt8] { withUnsafeBytes(of: v.littleEndian, Array.init) }
        var crc: UInt32 = 0xFFFF_FFFF
        for b in rom {
            crc ^= UInt32(b)
            for _ in 0..<8 { crc = crc & 1 != 0 ? (crc >> 1) ^ 0xEDB8_8320 : crc >> 1 }
        }
        var log = Array("MXLG".utf8) + [1, 0, 0, 0] + le32(~crc) + le32(UInt32(rom.count)) + le32(4)
        log += Array("INST".utf8) + le32(16) + le32(2)
        for (pc, abs) in [(0x00_8000, 0), (0x00_800C, 0x0C)] as [(UInt32, UInt32)] {
            log += le32(pc) + le32(abs) + [1, 1 << 3, 0, 0] + le32(1)
        }
        log += Array("ACCS".utf8) + le32(24) + le32(0)
        log += Array("FLOW".utf8) + le32(16) + le32(1)
        log += le32(0x00_8000) + le32(0x00_800C) + [5, 0, 0, 0] + le32(1)  // indirect call
        log += Array("DMA ".utf8) + le32(24) + le32(0)
        return Data(log)
    }

    /// Run the main run loop for a moment (callable from async tests).
    static func spin(_ seconds: TimeInterval) {
        RunLoop.main.run(until: Date().addingTimeInterval(seconds))
    }

    static func find<T: NSView>(_ view: NSView, _: T.Type) -> T? {
        if let t = view as? T { return t }
        for sub in view.subviews { if let t = find(sub, T.self) { return t } }
        return nil
    }

    static func window(_ model: RomViewModel) throws -> (RomWindowController, NSScrollView, HexCanvasView) {
        let controller = RomWindowController(model: model)
        controller.window?.orderFront(nil)
        let content = try #require(controller.window?.contentView)
        content.layoutSubtreeIfNeeded()
        spin(0.05)
        content.layoutSubtreeIfNeeded()
        let canvas = try #require(find(content, HexCanvasView.self))
        let scroll = try #require(canvas.enclosingScrollView)
        return (controller, scroll, canvas)
    }

    static func asmWindow(_ model: RomViewModel) throws -> (RomWindowController, NSScrollView, AsmCanvasView) {
        model.editorTab = .disassembly
        let controller = RomWindowController(model: model)
        controller.window?.orderFront(nil)
        let content = try #require(controller.window?.contentView)
        content.layoutSubtreeIfNeeded()
        Fixture.spin(0.05)
        content.layoutSubtreeIfNeeded()
        let canvas = try #require(find(content, AsmCanvasView.self))
        let scroll = try #require(canvas.enclosingScrollView)
        return (controller, scroll, canvas)
    }
}
