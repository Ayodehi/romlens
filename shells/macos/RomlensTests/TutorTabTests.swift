import AppKit
import RomlensKit
import Testing
@testable import Romlens

/// The tutor as a tab (docs/29, W9): citations open beside it rather than
/// over it, and pointing at a paragraph outlines what it cites.
@MainActor
@Suite struct TutorTabTests {
    private func model() async throws -> RomViewModel {
        let rom = try Rom.fromBytes(bytes: makeRoutinesTestRom(), name: "r.sfc")
        return try await Fixture.analyzedModel(rom: rom)
    }

    @Test func theTutorOpensAsATab() async throws {
        let m = try await model()
        m.showTutorInDrawer()
        m.showTutorTab()
        #expect(m.workspace.focusedItem?.content == .tutor)
        #expect(m.rightPane == .inspector, "the drawer goes back to the inspector, not two tutors side by side")
        let tutor = try #require(m.tutor)
        m.showTutorTab()
        #expect(m.workspace.layout.items.filter { $0.content == .tutor }.count == 1)
        #expect(m.tutor === tutor)
    }

    @Test func aCitationOpensBesideTheTutorsTab() async throws {
        let m = try await model()
        let only = m.workspace.focusedGroup
        // The tutor's tab, alone in the window's only group: the code tab
        // is in the same group, behind it.
        m.showTutorTab()
        let tutorTab = try #require(m.workspace.focusedItem?.id)
        #expect(try #require(m.tutor).follow(URL(string: "romlens://a/008040")!))
        #expect(m.selectedOffset == 0x40)
        #expect(m.workspace.layout.groups.count == 2, "a new group beside")
        #expect(m.workspace.layout.group(only)?.selected == tutorTab, "the tutor stays in view")
        #expect(m.workspace.focusedGroup != only)
        // Again, from the tutor: the code group is used, nothing new opens.
        m.focus(item: tutorTab)
        #expect(m.tutor!.follow(URL(string: "romlens://a/008020")!))
        #expect(m.workspace.layout.groups.count == 2)
        #expect(m.selectedOffset == 0x20)
        // A routine's C: that representation, beside the tutor still.
        m.focus(item: tutorTab)
        #expect(m.tutor!.follow(URL(string: "romlens://c/008040")!))
        #expect(m.editorTab == .c)
        #expect(m.workspace.layout.group(only)?.selected == tutorTab)
    }

    @Test func pointingAtAParagraphOutlinesWhatItCites() async throws {
        let m = try await model()
        let text = MessageText.prose("Clears `$00:8020` then returns at $00:8040.")
        let cited = MessageText.citedAddresses(text)
        #expect(cited == [0x008020, 0x008040])
        m.pointAtCitations(cited)
        #expect(m.citationHighlight.count == 2)
        #expect(m.citationHighlight[0].lowerBound == 0x20 && m.citationHighlight[0].count >= 1)
        m.pointAtCitations([])
        #expect(m.citationHighlight.isEmpty)
    }
}
