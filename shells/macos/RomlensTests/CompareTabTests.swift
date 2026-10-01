import AppKit
import Foundation
import RomlensKit
import Testing
@testable import Romlens

/// The Compare tab (docs/22, D2): another version beside this one, what
/// changed, and its names carried over as one undo step.
@MainActor
@Suite struct CompareTabTests {
    @Test func aSecondVersionComparesAndLendsItsNames() async throws {
        let roms = makeCompareTestRoms()
        let this = try await Fixture.analyzedModel(rom: try Rom.fromBytes(bytes: roms[1], name: "new.sfc"))
        let other = Workbench(rom: try Rom.fromBytes(bytes: roms[0], name: "old.sfc"))
        try other.execute(command: .setLabel(address: 0x008020, name: "ClearTable"))
        #expect(!this.editorTabs.contains(.compare), "no Compare tab until comparing")

        CompareController.compare(other: other, name: "old", model: this)
        #expect(this.editorTab == .compare)
        try await Fixture.settle(timeout: 10, until: { this.compare.state == .ready })
        #expect(this.editorTabs.contains(.compare))
        let info = try #require(this.compare.info)
        #expect(info.insertedBytes == 64)
        #expect(info.routines.map(\.pairing) == [.changed])
        #expect(info.routines[0].lines.contains { $0.op == .changed && $0.bText == "LDX #$1F" })

        // Carrying the name over is one step, and the comparison follows.
        let before = this.asmGeneration
        try this.session.carryNames(info.namesToCarry)
        try await Fixture.settle(timeout: 5, until: { this.asmGeneration != before })
        try await Fixture.settle(timeout: 10, until: {
            this.compare.info?.namesToCarry.isEmpty == true
        })
        #expect(this.workbench.labelAt(snesAddress: 0x008020)?.name == "ClearTable")
        this.undo()
        #expect(this.workbench.labelAt(snesAddress: 0x008020)?.name != "ClearTable")

        this.compare.close()
        #expect(!this.editorTabs.contains(.compare))
    }

    /// A second comparison started while the first runs is the one shown,
    /// and a run closed before it lands leaves the tab closed.
    @Test func aNewerRunWinsAndAClosedOneStaysClosed() async throws {
        let roms = makeCompareTestRoms()
        let this = try await Fixture.analyzedModel(rom: try Rom.fromBytes(bytes: roms[1], name: "new.sfc"))
        let first = Workbench(rom: try Rom.fromBytes(bytes: roms[0], name: "old.sfc"))
        let second = Workbench(rom: try Rom.fromBytes(bytes: roms[0], name: "older.sfc"))
        let compare = this.compare
        let a = Task { await compare.start(this: this.workbench, other: first, name: "first", generation: 0) }
        try await Fixture.settle(until: { compare.otherName == "first" })
        let b = Task { await compare.start(this: this.workbench, other: second, name: "second", generation: 0) }
        await a.value
        await b.value
        #expect(compare.state == .ready)
        #expect(compare.otherName == "second")
        #expect(compare.other === second)

        let c = Task { await compare.start(this: this.workbench, other: first, name: "first", generation: 0) }
        try await Fixture.settle(until: { compare.otherName == "first" })
        compare.close()
        await c.value
        #expect(compare.state == .idle)
        #expect(compare.info == nil && compare.other == nil)
    }

    @Test func theTabDrawsInAWindow() async throws {
        let roms = makeCompareTestRoms()
        let this = try await Fixture.analyzedModel(rom: try Rom.fromBytes(bytes: roms[1], name: "new.sfc"))
        CompareController.compare(other: Workbench(rom: try Rom.fromBytes(bytes: roms[0], name: "old.sfc")), name: "old", model: this)
        try await Fixture.settle(timeout: 10, until: { this.compare.state == .ready })
        this.compare.selected = .routine(0)
        let controller = RomWindowController(model: this)
        controller.window?.orderFront(nil)
        defer { controller.close() }
        let content = try #require(controller.window?.contentView)
        content.layoutSubtreeIfNeeded()
        Fixture.spin(0.2)
        #expect(content.bounds.width > 0)
    }
}
