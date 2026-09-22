import AppKit
import Foundation
import RomlensKit
import Testing
@testable import Romlens

/// Shared fixtures: ROMs, analyzed view models and off-screen windows.
@MainActor
enum Fixture {
    static let romType = "io.github.placeholder.romlens.sfc"
    static let projectType = "io.github.placeholder.romlens.project"

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
