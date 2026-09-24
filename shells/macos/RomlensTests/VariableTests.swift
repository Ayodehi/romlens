import Foundation
import RomlensKit
import Testing
@testable import Romlens

/// Removing labels, and variables: a named, typed address that the listing
/// shows in place of the address.
@MainActor
@Suite struct VariableTests {
    private func model() async throws -> RomViewModel {
        try await Fixture.analyzedModel(rom: Fixture.smallRom())
    }

    @Test func anImportedLabelCanBeRemoved() async throws {
        let m = try await model()
        _ = try m.session.importSymbols(source: "theirs.sym", text: "[labels]\n00:8000 Boot\n")
        m.select(offset: 0)
        #expect(m.label?.source == .imported)
        #expect(m.canRemoveLabel)
        try m.removeLabel()
        #expect(m.label?.source != .imported)
        #expect(m.session.undoTitle == "Remove Label")
        #expect(m.session.undo())
        m.select(offset: 0)
        #expect(m.label?.name == "Boot", "undo brings it back")
    }

    @Test func anAutomaticLabelCannotBeRemoved() async throws {
        let m = try await model()
        m.select(offset: 0)
        #expect(m.label?.source == .auto)
        #expect(!m.canRemoveLabel)
    }

    /// `STA $2100` at $00:8007: name what it writes, and the listing uses it.
    @Test func aVariableNamesTheOperandThatUsesIt() async throws {
        let m = try await model()
        m.select(offset: 7)
        #expect(m.instruction?.text == "STA $2100")
        #expect(m.operandAddress == 0x00_2100)

        m.beginDefineVariable()
        #expect(m.activeSheet == .variable)
        #expect(m.variableDraft.address == formatSnesAddress(address: 0x00_2100))
        m.variableDraft.name = "ScreenControl"
        try m.defineVariable(m.variableDraft)
        #expect(m.session.undoTitle == "Define Variable", "name and type, one step")

        m.select(offset: 7)
        #expect(m.instruction?.text == "STA ScreenControl")
        let v = try #require(m.workbench.variables().first)
        #expect((v.name, v.description, v.memory) == ("ScreenControl", "byte", "register"))

        // Opening it again edits that variable; its Remove takes name and type.
        m.beginDefineVariable()
        #expect(m.variableDraft.existing == 0x00_2100)
        try m.removeVariable(address: 0x00_2100)
        #expect(m.session.undoTitle == "Remove Variable")
        #expect(m.workbench.variables().isEmpty)
        m.select(offset: 7)
        #expect(m.instruction?.text == "STA $2100")
    }

    @Test func addressesParseTheWaysPeopleWriteThem() {
        #expect(RomViewModel.parseAddress("$7E:0094") == 0x7E_0094)
        #expect(RomViewModel.parseAddress("7F8000") == 0x7F_8000)
        #expect(RomViewModel.parseAddress("$0094") == 0x7E_0094, "low RAM")
        #expect(RomViewModel.parseAddress("$2100") == 0x00_2100, "a register")
        #expect(RomViewModel.parseAddress("nope") == nil)
        #expect(RomViewModel.parseAddress("") == nil)
    }
}
