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

    /// Each view chosen gets its own tab (the user's report, 2 October
    /// 2026): choosing C does not turn the Hex tab into C.
    @Test func aViewChosenGetsItsOwnTab() async throws {
        let m = try await model()
        let hex = try #require(m.workspace.focusedItem?.id)
        m.showTab(.c)
        let c = try #require(m.workspace.focusedItem?.id)
        #expect(c != hex)
        #expect(m.workspace.layout.item(hex)?.content == .code(.hex), "the Hex tab is still Hex")
        #expect(m.workspace.layout.items.count == 2)
        // Choosing Hex again shows its tab rather than opening another.
        m.showTab(.hex)
        #expect(m.workspace.focusedItem?.id == hex)
        #expect(m.workspace.layout.items.count == 2)
        // A view shown in another group is brought forward there.
        m.workspace.split(m.workspace.focusedGroup, .right, with: c)
        m.focus(item: hex)
        m.showTab(.c)
        #expect(m.workspace.focusedItem?.id == c)
        #expect(m.workspace.layout.items.count == 2)
    }

    @Test func aCodeTabIsNamedByItsViewAndTwinsByTheirRoutine() async throws {
        let m = try await model()
        try await Fixture.settle { !m.navigator.labels.isEmpty }
        m.select(offset: 0x44)
        let hex = try #require(m.workspace.focusedItem)
        #expect(m.title(of: hex) == "Hex")
        m.splitFocused(.right)
        let routine = try #require(m.routineName(at: 0x008044))
        #expect(m.workspace.layout.items.map { m.title(of: $0) } == ["Hex · \(routine)", "Hex · \(routine)"])
        // Moving to another routine renames the tab that follows.
        m.select(offset: 0x22)
        let other = try #require(m.routineName(at: 0x008022))
        #expect(other != routine)
        let first = m.workspace.layout.items.first { $0.followsSelection }
        let follower = try #require(first)
        #expect(m.title(of: follower) == "Hex · \(other)")
    }

    @Test func aSavedHeaderTabIsLeftOutOnReopening() async throws {
        let m = try await model()
        let g = m.workspace.focusedGroup
        m.workspace.open(.header, in: g)
        let record = m.workspaceRecord
        let back = try await model()
        back.restore(record)
        #expect(!back.workspace.layout.items.contains { $0.content == .header })
        #expect(!SidebarView.groups.flatMap(\.entries).contains { $0.content == .header }, "not offered in the sidebar")
    }

    @Test func graphicsAndSoundOpenTheirOwnTabsBesideTheCodeTab() async throws {
        let m = try await model()
        m.showTab(.disassembly)
        let code = try #require(m.workspace.focusedItem?.id)
        m.show(.graphics(.tiles))
        #expect(m.graphicsTab == .tiles && !m.showsTextEditor)
        #expect(m.editorTab == .disassembly, "the last text view is still what editorTab says")
        m.show(.audio(.voices))
        #expect(m.audioTab == .voices && m.graphicsTab == nil)
        // Hex, Disassembly, Tile Decoder, Voices.
        #expect(m.workspace.layout.items.count == 4)
        // Choosing a text view again shows the code tab; the others stay.
        m.showTab(.disassembly)
        #expect(m.workspace.focusedItem?.id == code && m.showsTextEditor)
        #expect(m.workspace.layout.items.count == 4)
        // The Tiles tab is shown again rather than opened twice.
        m.show(.graphics(.tiles))
        #expect(m.workspace.layout.items.count == 4)
        // Closing it shows the tab on its right, the sound view.
        m.closeFocusedTab()
        #expect(m.audioTab == .voices)
        _ = code
    }

    @Test func aJumpScrollsTheFocusedTabAndTheFollowersOnly() async throws {
        let m = try await model()
        let hex = try #require(m.workspace.focusedItem?.id)
        m.showTab(.disassembly)
        let a = try #require(m.workspace.focusedItem?.id)
        let group = m.workspace.focusedGroup
        // A second assembly tab does not follow: it would always show the
        // same place as the first.
        let b = try #require(m.workspace.open(.code(.assembly)))
        let bItem = try #require(m.workspace.layout.item(b))
        #expect(!bItem.followsSelection)
        m.workspace.split(group, .right, with: b)
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
        m.showTab(.c)
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
        m.showTab(.disassembly)
        let a = try #require(m.workspace.focusedItem?.id)
        m.select(offset: 0x10)
        m.jump(to: 0x20)
        m.show(.graphics(.tiles))
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

    // MARK: Drops (W4)

    @Test func droppingATabMovesSplitsOrInserts() async throws {
        let m = try await model()
        let a = try #require(m.workspace.focusedItem?.id)
        let left = m.workspace.focusedGroup
        let b = try #require(m.workspace.open(.atlas))
        let c = try #require(m.workspace.open(.graphics(.palette)))
        // On the right edge: a new group there, holding the tab.
        m.drop(.item(c), on: left, at: .zone(.edge(.right)))
        let right = try #require(m.workspace.layout.group(containing: c)?.id)
        #expect(right != left && m.workspace.layout.groups.count == 2)
        #expect(m.workspace.focusedGroup == right)
        // In the middle of the other group: moved there, shown.
        m.drop(.item(b), on: right, at: .zone(.center))
        #expect(m.workspace.layout.group(right)?.items.map(\.id) == [c, b])
        #expect(m.workspace.layout.group(right)?.selected == b)
        // On a tab bar: inserted at that place.
        m.drop(.item(a), on: right, at: .tabBar(index: 1))
        #expect(m.workspace.layout.group(right)?.items.map(\.id) == [c, a, b])
        // The left group lost its last tab, so it went.
        #expect(m.workspace.layout.groups.count == 1)
    }

    @Test func droppingSomethingToOpenOpensItThere() async throws {
        let m = try await model()
        let left = m.workspace.focusedGroup
        m.drop(.open(.graphics(.tilemap)), on: left, at: .zone(.edge(.bottom)))
        #expect(m.workspace.layout.groups.count == 2)
        #expect(m.graphicsTab == .tilemap)
        let bottom = m.workspace.focusedGroup
        #expect(bottom != left)
        // A view with one tab, dropped on another group, comes to it.
        m.drop(.open(.graphics(.tilemap)), on: left, at: .zone(.center))
        #expect(m.workspace.layout.group(containing: m.workspace.focusedItem!.id)?.id == left)
        #expect(m.workspace.layout.items.filter { $0.content == .graphics(.tilemap) }.count == 1)
        #expect(m.workspace.layout.groups.count == 1, "the bottom group emptied and went")
        // Code opens a new tab each time.
        m.drop(.open(.code(.c)), on: left, at: .tabBar(index: 0))
        #expect(m.workspace.layout.groups[0].items.first?.content == .code(.c))
    }

    @Test func droppingAGroupsOnlyTabOnItselfChangesNothing() async throws {
        let m = try await model()
        let g = m.workspace.focusedGroup
        let a = try #require(m.workspace.focusedItem?.id)
        let before = m.workspace.layout
        m.drop(.item(a), on: g, at: .zone(.edge(.left)))
        #expect(m.workspace.layout == before)
    }

    // MARK: The Pseudo-C tab (2 October 2026)

    @Test func theRoutineListHoldsRoutinesNotLoops() async throws {
        let m = try await model()
        try await Fixture.settle { !m.navigator.labels.isEmpty }
        let names = m.routines.map(\.name)
        #expect(names.contains { $0.hasPrefix("SUB_") })
        #expect(names.contains { $0.hasPrefix("RESET") })
        #expect(!names.contains { $0.hasPrefix("LOOP_") || $0.hasPrefix("SKIP_") || $0.hasPrefix("DATA_") }, "\(names)")
        #expect(m.routines.map(\.address) == m.routines.map(\.address).sorted())
        // A label of the student's on a routine is listed by that name.
        m.select(offset: 0x20)
        try m.setLabel(name: "ClearSlots")
        try await Fixture.settle { m.navigator.labels.contains { $0.name == "ClearSlots" } }
        #expect(m.routines.contains { $0.name == "ClearSlots" })
    }

    @Test func choosingARoutineShowsItsC() async throws {
        let m = try await model()
        try await Fixture.settle { !m.navigator.labels.isEmpty }
        m.showTab(.c)
        let c = try #require(m.workspace.focusedItem?.id)
        let sub = try #require(m.routines.first { $0.address == 0x008040 })
        m.jump(toSnesAddress: sub.address)
        try await Fixture.settle(timeout: 20) { m.workspace.decompiler(for: c).result?.entry == 0x008040 }
    }
}
