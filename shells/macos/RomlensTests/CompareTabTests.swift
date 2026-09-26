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
