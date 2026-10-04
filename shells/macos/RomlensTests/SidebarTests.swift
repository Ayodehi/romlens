import AppKit
import RomlensKit
import SwiftUI
import Testing
@testable import Romlens

/// The sidebar (docs/29, W5): every view listed by its chip, the ones that
/// cannot open yet saying why, and labels dragged out as new tabs.
@MainActor
@Suite struct SidebarTests {
    private func model() async throws -> RomViewModel {
        let rom = try Rom.fromBytes(bytes: makeRoutinesTestRom(), name: "r.sfc")
        return try await Fixture.analyzedModel(rom: rom)
    }

    @Test func everyViewHasARow() {
        let contents = Set(SidebarView.groups.flatMap(\.entries).compactMap(\.content))
        for r in CodeRepresentation.allCases { #expect(contents.contains(.code(r)), "\(r)") }
        for t in GraphicsModel.Tab.allCases { #expect(contents.contains(.graphics(t)), "\(t)") }
        for t in AudioModel.Tab.allCases { #expect(contents.contains(.audio(t)), "\(t)") }
        for c in [EditorContent.atlas, .compare, .source, .tutor] { #expect(contents.contains(c), "\(c)") }
        #expect(SidebarView.groups.map(\.title) == ["Cartridge", "CPU · 65816", "PPU · Picture", "APU · Sound", "Learn"])
    }

    @Test func viewsThatCannotOpenYetSayWhy() async throws {
        let m = try await model()
        #expect(m.unavailableReason(.graphics(.frame)) == "needs a recording")
        #expect(m.unavailableReason(.graphics(.layers)) == "needs a recording")
        #expect(m.unavailableReason(.graphics(.tiles)) == nil)
        #expect(m.unavailableReason(.compare) == "needs a ROM")
        #expect(m.unavailableReason(.source) == "no sources")
        #expect(m.unavailableReason(.code(.assembly)) == nil, "analyzed")
        #expect(m.unavailableReason(.audio(.voices)) == nil, "sound views explain themselves")
    }

    @Test func aLabelDraggedOutOpensANewTabThere() async throws {
        let m = try await model()
        m.showTab(.disassembly)
        let g = m.workspace.focusedGroup
        m.drop(.openAt(.assembly, address: 0x008040), on: g, at: .zone(.edge(.right)))
        #expect(m.workspace.layout.groups.count == 2)
        #expect(m.workspace.focusedGroup != g)
        #expect(m.workspace.focusedItem?.content == .code(.assembly))
        #expect(m.selectedOffset == 0x40)
    }

    @Test func theSidebarLaysOutInAWindow() async throws {
        let m = try await model()
        try await Fixture.settle { !m.navigator.labels.isEmpty }
        let controller = RomWindowController(model: m)
        controller.window?.setContentSize(NSSize(width: 1440, height: 900))
        controller.window?.orderFront(nil)
        defer { controller.window?.close() }
        let content = try #require(controller.window?.contentView)
        for _ in 0..<4 {
            content.layoutSubtreeIfNeeded()
            Fixture.spin(0.1)
        }
        #expect(Fixture.find(content, NSOutlineView.self) != nil || Fixture.find(content, NSTableView.self) != nil)
        try WorkspaceShellTests.snapshot(content, "sidebar")
    }
}
