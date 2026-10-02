import Foundation
import RomlensKit
import Testing
@testable import Romlens

/// The window's tabs on the model (docs/29, W2): the old one-editor
/// properties over the focused tab, scroll requests for the tabs meant,
/// a C and a graph per tab, and Back into the tab a place was seen in.
@MainActor
@Suite struct WorkspaceModelTests {
    private func model() async throws -> RomViewModel {
        let rom = try Rom.fromBytes(bytes: makeRoutinesTestRom(), name: "r.sfc")
        return try await Fixture.analyzedModel(rom: rom)
    }

    @Test func aRepresentationChangesTheFocusedTabInPlace() async throws {
        let m = try await model()
        let first = try #require(m.workspace.focusedItem?.id)
        m.editorTab = .c
        #expect(m.workspace.focusedItem?.id == first)
        #expect(m.workspace.focusedItem?.content == .code(.c))
        #expect(m.workspace.layout.items.count == 1)
    }

    @Test func graphicsAndSoundOpenTheirOwnTabsAndLeaveTheCodeTab() async throws {
        let m = try await model()
        m.editorTab = .disassembly
        let code = try #require(m.workspace.focusedItem?.id)
        m.graphicsTab = .tiles
        #expect(m.graphicsTab == .tiles && !m.showsTextEditor)
        #expect(m.editorTab == .disassembly, "the last text view is still what editorTab says")
        m.audioTab = .voices
        #expect(m.audioTab == .voices && m.graphicsTab == nil)
        #expect(m.workspace.layout.items.count == 3)
        // Choosing a text view again shows the code tab; the others stay.
        m.editorTab = .disassembly
        #expect(m.workspace.focusedItem?.id == code && m.showsTextEditor)
        #expect(m.workspace.layout.items.count == 3)
        // The Tiles tab is shown again rather than opened twice.
        m.graphicsTab = .tiles
        #expect(m.workspace.layout.items.count == 3)
        // Clearing it the old way goes back to the text tab.
        m.graphicsTab = nil
        #expect(m.workspace.focusedItem?.id == code)
    }

    @Test func aJumpScrollsTheFocusedTabAndTheFollowersOnly() async throws {
        let m = try await model()
        m.editorTab = .disassembly
        let a = try #require(m.workspace.focusedItem?.id)
        let group = m.workspace.focusedGroup
        // A second assembly tab does not follow: it would always show the
        // same place as the first.
        let b = try #require(m.workspace.open(.code(.assembly)))
        let bItem = try #require(m.workspace.layout.item(b))
        #expect(!bItem.followsSelection)
        m.workspace.split(group, .right, with: b)
        let hex = try #require(m.workspace.open(.code(.hex)))
        m.focus(item: a)
        m.jump(to: 0x40)
        let request = try #require(m.scrollRequest)
        #expect(request.applies(to: a))
        #expect(request.applies(to: hex), "a hex tab follows the selection")
        #expect(!request.applies(to: b))
        #expect(request.applies(to: nil), "a view outside any tab acts on every request")
        // In b, b scrolls.
        m.focus(item: b)
        m.jump(to: 0x20)
        #expect(m.scrollRequest?.applies(to: b) == true)
    }

    @Test func eachCTabKeepsItsOwnRoutine() async throws {
        let m = try await model()
        m.select(offset: 0x44)
        m.editorTab = .c
        let a = try #require(m.workspace.focusedItem?.id)
        // A second C tab, beside the first, not following the selection: it
        // opens on the routine at the selection and stays there.
        let b = try #require(m.workspace.open(.code(.c)))
        m.workspace.split(m.workspace.focusedGroup, .right, with: b)
        m.focus(item: b)
        try await Fixture.settle(until: { m.workspace.decompiler(for: b).result?.name == "SUB_008040" })
        #expect(m.decompiler === m.workspace.decompiler(for: b), "the model's C is the focused tab's")
        // Reading on in a: a moves to the new routine, b does not.
        m.focus(item: a)
        m.select(offset: 0x22)
        try await Fixture.settle(until: { m.workspace.decompiler(for: a).result?.name == "SUB_008020" })
        #expect(m.workspace.decompiler(for: b).result?.name == "SUB_008040")
    }

    @Test func backGoesToTheTabAPlaceWasSeenIn() async throws {
        let m = try await model()
        m.editorTab = .disassembly
        let a = try #require(m.workspace.focusedItem?.id)
        m.select(offset: 0x10)
        m.jump(to: 0x20)
        m.graphicsTab = .tiles
        let tiles = try #require(m.workspace.focusedItem?.id)
        m.jump(to: 0x40)
        // 0x20 was left from the Tiles tab, 0x10 from the assembly tab.
        m.goBack()
        #expect(m.selectedOffset == 0x20 && m.workspace.focusedItem?.id == tiles)
        m.goBack()
        #expect(m.selectedOffset == 0x10 && m.workspace.focusedItem?.id == a)
        m.goForward()
        #expect(m.selectedOffset == 0x20)
    }
}
