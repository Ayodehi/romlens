import AppKit
import Foundation
import RomlensKit
import Testing
@testable import Romlens

/// The Atlas tab (docs/22, A2): the whole ROM a bank a row, zoomed from
/// all of it down to single instructions, sharing the selection.
@MainActor
@Suite struct AtlasTabTests {
    private func shown() async throws -> (RomViewModel, RomWindowController, AtlasCanvasView) {
        let rom = try Rom.fromBytes(bytes: makeRoutinesTestRom(), name: "r.sfc")
        let m = try await Fixture.analyzedModel(rom: rom)
        m.editorTab = .atlas
        let controller = RomWindowController(model: m)
        controller.window?.orderFront(nil)
        let content = try #require(controller.window?.contentView)
        try await Fixture.settle(until: {
            content.layoutSubtreeIfNeeded()
            return (Fixture.find(content, AtlasCanvasView.self)?.bounds.width ?? 0) > 100
        })
        return (m, controller, try #require(Fixture.find(content, AtlasCanvasView.self)))
    }

    @Test func aClickSelectsAndADoubleClickOpensTheListing() async throws {
        let (m, controller, canvas) = try await shown()
        defer { controller.close() }
        // Zoomed out, a pixel is many bytes; zoomed in, one byte is many
        // pixels and a click lands on the byte asked for.
        for _ in 0..<40 where canvas.ppb < 8 { canvas.zoom(by: 1.5, at: canvas.point(ofOffset: 0)) }
        #expect(canvas.ppb >= 8)
        let p = canvas.point(ofOffset: 0x20)
        #expect(canvas.offset(at: p) == 0x20)
        canvas.mouseMovedForTesting(p)
        #expect(canvas.statusText.hasPrefix("0x000020  ·  $00:8020  ·  instruction"), "\(canvas.statusText)")
        canvas.click(at: p)
        #expect(m.selectedOffset == 0x20)
        #expect(m.editorTab == .atlas, "a click stays in the Atlas")
        canvas.click(at: canvas.point(ofOffset: 0x22), count: 2)
        #expect(m.editorTab == .disassembly)
        #expect(m.selectedOffset == 0x22)
    }

    @Test func theMenuZoomsAndFits() async throws {
        let (m, controller, canvas) = try await shown()
        defer { controller.close() }
        let fit = canvas.ppb
        controller.zoomGraphIn(nil)
        try await Fixture.settle(until: { canvas.ppb > fit })
        controller.zoomGraphToFit(nil)
        try await Fixture.settle(until: { canvas.ppb == fit })
        #expect(m.atlas.overlay == .kind)
        // Zooming out past the whole ROM does nothing.
        controller.zoomGraphOut(nil)
        try await Fixture.settle(until: { true })
        #expect(canvas.ppb == fit)
    }
}
