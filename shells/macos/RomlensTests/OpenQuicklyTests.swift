import AppKit
import RomlensKit
import Testing
@testable import Romlens

/// Open Quickly and the address columns' new homes (docs/29, W6).
@MainActor
@Suite struct OpenQuicklyTests {
    private func model() async throws -> RomViewModel {
        let rom = try Rom.fromBytes(bytes: makeRoutinesTestRom(), name: "r.sfc")
        let m = try await Fixture.analyzedModel(rom: rom)
        try await Fixture.settle { !m.navigator.labels.isEmpty }
        return m
    }

    @Test func anAddressComesFirstAndGoesThere() async throws {
        let m = try await model()
        let found = OpenQuicklySheet.results(for: "$00:8040", in: m)
        let first = try #require(found.first)
        #expect(first.kind == .address)
        first.action()
        #expect(m.selectedOffset == 0x40)
    }

    @Test func viewsAndLabelsByName() async throws {
        let m = try await model()
        let tiles = OpenQuicklySheet.results(for: "tile", in: m)
        let decoder = try #require(tiles.first { $0.kind == .view && $0.title == "Tile Decoder" })
        decoder.action()
        #expect(m.graphicsTab == .tiles)
        let name = try #require(m.navigator.labels.first?.name)
        let labels = OpenQuicklySheet.results(for: name, in: m)
        #expect(labels.contains { $0.kind == .label && $0.title == name })
        #expect(OpenQuicklySheet.results(for: "", in: m).isEmpty)
    }

    @Test func theAddressColumnsAreInTheEditorsMenus() {
        let menu = EditorContextMenu.build(hasSelection: true)
        let addresses = menu.items.first { $0.title == "Addresses" }?.submenu
        #expect(addresses?.items.map(\.title) == ["File Offset and SNES Address", "SNES Address Only", "File Offset Only"])
    }
}
