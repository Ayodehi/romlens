import AppKit
import Foundation
import RomlensKit
import Testing
@testable import Romlens

/// The Graph tab (docs/19): the routine at the selection as blocks, or with
/// its callers and callees, kept in step with the listing.
@MainActor
@Suite struct GraphTabTests {
    private func model() async throws -> RomViewModel {
        let rom = try Rom.fromBytes(bytes: makeRoutinesTestRom(), name: "r.sfc")
        return try await Fixture.analyzedModel(rom: rom)
    }

    @Test func theTabFollowsTheSelectionToItsRoutine() async throws {
        let m = try await model()
        m.select(offset: 0x22)
        #expect(m.graph.state == .idle, "nothing is built until the tab shows")
        m.editorTab = .graph
        try await Fixture.settle(until: { m.graph.state == .ready })
        let g = try #require(m.graph.blocks)
        #expect(g.name == "SUB_008020")
        #expect(g.blocks.count == 3)
        #expect(m.graph.block(containing: 0x22) == 1)
        #expect(m.graph.block(containing: 0x28) == 2)

        // Another routine: the graph follows.
        m.select(offset: 0x44)
        try await Fixture.settle(until: { m.graph.blocks?.name == "SUB_008040" })

        // Calls: its caller.
        m.graph.mode = .calls
        m.refreshGraph()
        try await Fixture.settle(until: { m.graph.calls != nil })
        #expect(m.graph.calls?.callers.map(\.entry) == [0x008000])
    }

    @Test func changesKeepTheGraphUp() async throws {
        let m = try await model()
        m.select(offset: 0x22)
        m.editorTab = .graph
        try await Fixture.settle(until: { m.graph.state == .ready })
        let first = m.graph.resultGeneration
        var sawLoading = false
        try m.session.setLabel(address: 0x008020, name: "ClearTable")
        try await Fixture.settle(until: {
            if m.graph.state != .ready { sawLoading = true }
            return m.graph.resultGeneration > first
        })
        #expect(!sawLoading)
        #expect(m.graph.blocks?.name == "ClearTable")
    }

    @Test func theCanvasDrawsTheLinesAndAClickSelects() async throws {
        let m = try await model()
        m.select(offset: 0x22)
        m.editorTab = .graph
        let controller = RomWindowController(model: m)
        controller.window?.orderFront(nil)
        defer { controller.close() }
        try await Fixture.settle(until: { m.graph.state == .ready })
        let content = try #require(controller.window?.contentView)
        try await Fixture.settle(until: {
            content.layoutSubtreeIfNeeded()
            return Fixture.find(content, GraphCanvasView.self)?.scene.boxes.count == 3
        })
        let canvas = try #require(Fixture.find(content, GraphCanvasView.self))
        let scene = canvas.scene
        // The loop block: its label, the idiom's note, then its three
        // instructions.
        #expect(scene.boxes[1].lines.count == 5)
        #expect(scene.boxes[1].loopHeader)
        #expect(scene.edges.count == 3)
        // Boxes top to bottom, none on another.
        #expect(scene.boxes[0].rect.maxY < scene.boxes[1].rect.minY)
        #expect(scene.boxes[1].rect.maxY < scene.boxes[2].rect.minY)
        // A click on the RTS selects it.
        let rts = try #require(scene.rect(forOffset: 0x28))
        canvas.click(at: CGPoint(x: rts.midX, y: rts.midY))
        #expect(m.selectedOffset == 0x28)
        // Selecting elsewhere highlights that line.
        m.select(offset: 0x25)
        try await Fixture.settle(until: { canvas.selectedOffset == 0x25 })
    }

    @Test func zoomToFitShrinksALargeGraph() async throws {
        let m = try await model()
        m.select(offset: 0x00)
        m.editorTab = .graph
        let controller = RomWindowController(model: m)
        controller.window?.setContentSize(NSSize(width: 900, height: 300))
        controller.window?.orderFront(nil)
        defer { controller.close() }
        try await Fixture.settle(until: { m.graph.state == .ready })
        let content = try #require(controller.window?.contentView)
        try await Fixture.settle(until: {
            content.layoutSubtreeIfNeeded()
            return Fixture.find(content, GraphCanvasView.self)?.scene.boxes.isEmpty == false
        })
        let canvas = try #require(Fixture.find(content, GraphCanvasView.self))
        let scroll = try #require(canvas.enclosingScrollView)
        m.graph.requestZoom(.zoomOut)
        try await Fixture.settle(until: { scroll.magnification < 1 })
    }
}
