import AppKit
import RomlensKit
import Testing
@testable import Romlens

/// The Phase 2 shell: Find, the overview strip, typed marks and importing.
/// Verified through the model and the views rather than by eye — screenshots
/// are unavailable on this machine (`docs/15`).
@MainActor
@Suite struct Phase2ShellTests {
    private func model() async throws -> RomViewModel {
        try await Fixture.analyzedModel(rom: Fixture.smallRom())
    }

    // MARK: Find

    @Test func findsBytesAndSteps() async throws {
        let m = try await model()
        m.search.query = "78 18 FB"  // the fixture's SEI; CLC; XCE
        m.runSearch()
        #expect(m.search.hasResults)
        #expect(m.search.hits.first?.fileOffset == 0)
        #expect(m.search.current == 0)
        #expect(m.selectedOffset == 0, "the first hit is selected")
        // Context either side, with the match located inside it.
        let hit = try #require(m.search.hits.first)
        #expect(hit.len == 3)
        #expect(hit.matchStart == 0, "a hit at offset 0 has nothing before it")
        #expect(hit.context.count > 3)
        #expect(hit.regionKind == "code")
    }

    @Test func findsTextInEitherCase() async throws {
        let m = try await model()
        m.search.query = "romlens"
        m.search.mode = .text
        m.search.ignoreCase = false
        m.runSearch()
        #expect(!m.search.hasResults, "the title is upper case")

        m.search.ignoreCase = true
        m.runSearch()
        #expect(m.search.hasResults, "ROMLENS TEST is in the header")
        #expect(m.search.summary.contains("match"))
    }

    // MARK: Find References

    @Test func findReferencesListsEveryReferrerAndVisitsThem() async throws {
        let m = try await model()
        let target = try #require(
            m.workbench.labels().first { !m.workbench.xrefsTo(snesAddress: $0.address).isEmpty },
            "the fixture's dispatch table gives its targets references"
        )
        m.jump(toSnesAddress: target.address)
        let named = try #require(m.referenceTarget)
        #expect(named.name == target.name)
        #expect(named.count == m.xrefsTo.count)

        m.findReferences()
        #expect(m.isResultsVisible)
        #expect(m.resultsKind == .references)
        #expect(m.references.targetName == target.name)
        #expect(m.references.rows.map(\.fileOffset) == m.xrefsTo.map(\.fromOffset))
        #expect(m.references.current == nil, "finding does not move the selection")

        let first = try #require(m.references.rows.first)
        m.goToReference(at: 0)
        #expect(m.selectedOffset == first.fileOffset)
        #expect(m.references.summary.hasPrefix("1 of "))
        #expect(m.canGoBack, "a reference visited is in the history")

        // A Find afterwards takes the pane back without losing the list.
        m.search.query = "78 18 FB"
        m.runSearch()
        #expect(m.resultsKind == .find)
        #expect(m.references.rows.count == named.count)
    }

    @Test func aReferenceIsNamedByTheRoutineItIsIn() {
        func label(_ name: String, _ address: UInt32, _ source: LabelSource = .auto) -> LabelInfo {
            LabelInfo(address: address, name: name, source: source, origin: "", fileOffset: nil)
        }
        let routines = ReferencesModel.routineLabels([
            label("CODE_808010", 0x80_8010),
            label("SUB_808000", 0x80_8000),
            label("DATA_808100", 0x80_8100),
            label("CODE_Loop", 0x80_8040, .user),
            label("SUB_818000", 0x81_8000),
        ])
        #expect(routines.map(\.name) == ["SUB_808000", "CODE_Loop", "SUB_818000"],
                "the analyzer's inner labels are skipped; a user's name is kept")
        #expect(ReferencesModel.routine(containing: 0x80_8000, in: routines) == "SUB_808000")
        #expect(ReferencesModel.routine(containing: 0x80_801C, in: routines) == "SUB_808000+1C")
        #expect(ReferencesModel.routine(containing: 0x80_9000, in: routines) == "CODE_Loop+FC0")
        #expect(ReferencesModel.routine(containing: 0x80_7FFF, in: routines) == nil)
        #expect(ReferencesModel.routine(containing: 0x82_8000, in: routines) == nil,
                "a label in another bank does not contain the reference")
    }

    @Test func steppingWrapsAndReportsPosition() async throws {
        let m = try await model()
        // `00` occurs many times, so there is something to step through.
        m.search.query = "00 00"
        m.runSearch()
        let count = m.search.hits.count
        try #require(count > 2)
        #expect(m.search.summary.hasPrefix("1 of "))
        m.stepSearch(by: 1)
        #expect(m.search.current == 1)
        m.stepSearch(by: -1)
        m.stepSearch(by: -1)
        #expect(m.search.current == count - 1, "stepping back from the first wraps")
    }

    @Test func aBadPatternReportsRatherThanClearing() async throws {
        let m = try await model()
        m.search.query = "zz"
        m.runSearch()
        #expect(!m.search.hasResults)
        #expect(m.search.summary.isEmpty == false)
        m.search.clear()
        #expect(m.search.summary.isEmpty)
    }

    // MARK: Overview strip

    @Test func theStripTilesTheImage() async throws {
        let m = try await model()
        let columns = RegionStrip.decode(m.workbench.regionMap(buckets: 64))
        #expect(columns.count == 64)
        #expect(columns.first?.start == 0)
        #expect(columns.last?.end == m.byteCount)
        for (a, b) in zip(columns, columns.dropFirst()) {
            #expect(a.end == b.start, "a gap or an overlap in the strip")
        }
        // One column is 512 bytes here, and the boot routine is thirteen of
        // them, so the column is not "code" — but it is not all one thing
        // either, and it has to say so rather than claim the majority.
        #expect(columns[0].share < 1.0)
        #expect(columns.allSatisfy { $0.confidence >= 0 && $0.confidence <= 1 })
    }

    @Test func aStaleBatchDecodesToNothing() {
        #expect(RegionStrip.decode(Data()).isEmpty)
        #expect(RegionStrip.decode(Data([99, 0, 1, 0, 0, 0, 0, 0])).isEmpty, "wrong version")
        // A header promising more records than the buffer holds is refused
        // rather than read past.
        #expect(RegionStrip.decode(Data([1, 0, 4, 0, 0, 0, 0, 0])).isEmpty)
    }

    // MARK: Typed marks

    @Test func markingATableRendersItsTargets() async throws {
        let m = try await model()
        // The emulation vectors at 0x7FFC name the boot routine and the
        // catch-all: a table of code pointers by definition.
        m.select(offset: 0x7FFC)
        m.extendSelection(to: 0x7FFF)
        m.mark(.data, dataKind: .table, stride: 2, elem: .code, bank: .sameBank)
        try await Fixture.settle(until: { m.region?.dataKind == .table })
        let region = try #require(m.region)
        #expect(region.stride == 2)
        #expect(region.elem == .code)
        #expect(region.bank == .sameBank)
        // The listing names them rather than printing numbers.
        let text = m.workbench.asmLinesText(
            startLine: m.workbench.lineForOffset(fileOffset: 0x7FFC) ?? 0,
            count: 3,
            style: .snes
        )
        #expect(text.contains("dw "), "\(text)")
        #expect(text.contains("_00"), "entries are rendered as labels: \(text)")
    }

    @Test func aPlainDataMarkStillMeansByte() async throws {
        let m = try await model()
        m.select(offset: 0x20)
        m.mark(.data)
        try await Fixture.settle(until: { m.region?.kind == .data })
        #expect(m.region?.dataKind == .byte)
        #expect(m.region?.elem == nil, "a byte region has no element kind")
    }

    // MARK: Importing

    @Test func importingATraceReportsWhatItFound() async throws {
        let m = try await model()
        var payload = [UInt8](repeating: 0, count: Int(m.byteCount))
        // The two unreachable NOPs ran, with 8-bit A and X.
        payload[0x0C] = 0x01 | 0x04 | 0x20 | 0x10
        payload[0x0D] = 0x01 | 0x20 | 0x10
        var cdl = Array("CDLv2".utf8)
        cdl.append(contentsOf: [0, 0, 0, 0])
        cdl.append(contentsOf: payload)

        let result = try m.session.importTrace(source: "play.cdl", bytes: Data(cdl))
        #expect(result.format == "cdl")
        #expect(result.executedBytes == 2)
        #expect(result.hasWidths)
        let summary = ImportController.summary(.trace, result)
        #expect(summary.contains("2 bytes executed"))
        #expect(summary.contains("M/X widths"))

        try await Fixture.settle(until: { m.session.hasSnapshot })
        m.select(offset: 0x0C)
        try await Fixture.settle(until: { m.region?.kind == .code })
        #expect(m.region?.confidence ?? 0 > 0.9, "a trace outranks the descent")
    }

    /// An execution log, as the fork's `emu.getExecutionLog()` writes it: the
    /// boot instruction and an unreachable one it was seen to call.
    @Test func importingAnExecutionLogAddsSeenReferences() async throws {
        let m = try await model()
        let log = Fixture.execLog(rom: [UInt8](makeTestRom(mapping: .loRom)))
        let before = m.session.analysisGeneration
        let result = try m.session.importTrace(source: "play.mxlog", bytes: Data(log))
        #expect(result.format == "mxlog")
        #expect(result.detail.hasPrefix("2 instructions"), "\(result.detail)")
        #expect(ImportController.summary(.trace, result).contains("Execution log: 2 instructions"))

        // The import re-analyzes after a short pause; wait for that run.
        try await Fixture.settle(until: { m.session.analysisGeneration > before && !m.session.analysis.isRunning })
        let target = try #require(m.rom.snesAddressFor(fileOffset: 0x0C))
        let seen = m.workbench.xrefsTo(snesAddress: target).filter { $0.observed }
        #expect(seen.count == 1)
        #expect(seen.first?.kindName == "call")
        #expect(seen.first?.fromOffset == 0)
    }

    @Test func importingSymbolsKeepsTheUsersNames() async throws {
        let m = try await model()
        try m.setLabelAt(offset: 0, name: "MyOwnBoot")
        let result = try m.session.importSymbols(
            source: "theirs.sym",
            text: "; Licence: 0BSD\n\n[labels]\n00:8000 Their::Boot\n00:800E Idle\n"
        )
        #expect(result.labelsAdded == 1)
        #expect(result.keptUser == 1)
        #expect(result.rewritten == ["Their::Boot -> Their_Boot"])
        #expect(result.notice == "Licence: 0BSD")

        // Nothing is hidden: the report names every rewrite and the notice.
        let summary = ImportController.summary(.symbols, result)
        #expect(summary.contains("Their::Boot -> Their_Boot"))
        #expect(summary.contains("you had already named them"))
        #expect(summary.contains("0BSD"))

        try await Fixture.settle(until: { m.session.hasSnapshot })
        m.select(offset: 0)
        #expect(m.label?.name == "MyOwnBoot")
    }

    // MARK: Navigator

    @Test func theNavigatorAsksForABoundedList() async throws {
        let m = try await model()
        try await Fixture.settle(until: { !m.navigator.regions.isEmpty })
        #expect(m.navigator.regions.count <= Int(NavigatorModel.regionLimit) * 2)
        #expect(m.navigator.regions.allSatisfy { $0.kind != .unknown })
        // Address order, because the navigator lists regions rather than
        // ranking them.
        for (a, b) in zip(m.navigator.regions, m.navigator.regions.dropFirst()) {
            #expect(a.start <= b.start)
        }
        // The fixture has far fewer regions than the limit, so nothing is
        // hidden and the list must not claim otherwise.
        #expect(!m.navigator.regionsTruncated)
    }

    // MARK: Toolbar and titlebar

    /// The subtitle truncated at the window's minimum width, which is how a
    /// reader lost the size entirely.
    @Test func theSubtitleIsShortAndReadable() async throws {
        let m = try await model()
        let subtitle = RomWindowController.subtitle(for: m.info)
        #expect(subtitle == "LoROM · SlowROM · 32 KB", "\(subtitle)")
        #expect(!subtitle.contains("32768"), "a raw byte count is not a size")
        #expect(subtitle.count < 40, "long enough to truncate again: \(subtitle)")
    }

    /// Two unrelated controls both said "Both", so the toolbar had a word that
    /// meant one thing on the left and another on the right.
    @Test func noTwoToolbarControlsShareALabel() {
        let tabs = Set(RomViewModel.EditorTab.allCases.map(\.title))
        let addresses = Set(AddressStyle.allCases.map(\.label))
        #expect(tabs.isDisjoint(with: addresses), "shared: \(tabs.intersection(addresses))")
        #expect(addresses.contains("File + SNES"))
    }

    // MARK: Menu wiring

    @Test func theNewMenuItemsExist() throws {
        let menu = MainMenu.build()
        func find(_ path: [String]) -> NSMenuItem? {
            var items = menu.items
            var found: NSMenuItem?
            for name in path {
                found = items.first { $0.title == name }
                items = found?.submenu?.items ?? []
            }
            return found
        }
        #expect(find(["File", "Import", "Execution Trace…"])?.action
            == #selector(RomWindowController.importTrace(_:)))
        #expect(find(["File", "Import", "Symbols…"])?.action
            == #selector(RomWindowController.importSymbols(_:)))
        #expect(find(["Edit", "Mark as", "Data…"])?.action
            == #selector(RomWindowController.markAsDataWithOptions(_:)))
        // Phase 1's three keep their wording and their place at the top.
        let markAs = try #require(find(["Edit", "Mark as"])?.submenu?.items)
        #expect(markAs.prefix(3).map(\.title) == ["Code", "Data", "Unknown"])
        let findItem = try #require(find(["Go", "Find…"]))
        #expect(findItem.keyEquivalent == "f")
        #expect(find(["Go", "Find Next"])?.keyEquivalent == "g")
        #expect(find(["View", "Show Overview Strip"]) != nil)
    }
}

private extension RomViewModel {
    func setLabelAt(offset: UInt32, name: String) throws {
        select(offset: offset)
        try setLabel(name: name)
    }
}
