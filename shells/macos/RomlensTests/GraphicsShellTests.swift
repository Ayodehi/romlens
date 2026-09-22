import AppKit
import RomlensKit
import SwiftUI
import Testing
@testable import Romlens

/// Track 2B's shell: the four graphics views, the shared selection, the
/// recording they can read, and the inspector's previews. Verified through the
/// model and off-screen windows — screenshots are unavailable here (docs/15).
@MainActor
@Suite struct GraphicsShellTests {
    private func model() async throws -> RomViewModel {
        let rom = try Rom.fromBytes(bytes: makeGraphicsTestRom(), name: "g.sfc")
        let m = try await Fixture.analyzedModel(rom: rom)
        m.session.reanalysisDelay = .milliseconds(1)
        return m
    }

    private func recordingURL(frames: UInt32 = 40) throws -> URL {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("romlens-shell-\(UUID().uuidString).romrec")
        try makeTestRecording(frames: frames).write(to: url)
        return url
    }

    // MARK: Tabs and selection

    @Test func theGraphicsPickerSharesTheEditorWithTheTextTabs() async throws {
        let m = try await model()
        m.jump(to: 0x1000)
        m.openGraphics(.tiles)
        #expect(m.graphicsTab == .tiles)
        #expect(m.graphics.romOffset == 0x1000, "the view reads the bytes at the selection")
        m.editorTab = .disassembly
        #expect(m.graphicsTab == nil, "choosing a text tab closes the graphics view")
    }

    @Test func aTileClickSelectsItsBytesInTheEditor() async throws {
        let m = try await model()
        m.jump(to: 0x1000)
        m.openGraphics(.tiles)
        m.graphics.selectTile(8)  // the lens's top-left quarter
        #expect(m.highlightedRange == 0x1100..<0x1120)
        #expect(m.graphicsTab == .tiles, "selecting does not leave the view")
        let tile = m.graphics.selectedTileInfo()
        #expect(tile.indices.contains(3), "the lens has its highlight")
        let sheet = m.graphics.sheet()
        #expect(sheet.width == 128 && sheet.height == 128)
        #expect(sheet.cgImage?.width == 128)
    }

    @Test func hoveringNeedsNoCallAndAgreesWithTheDecoder() async throws {
        let m = try await model()
        let g = m.graphics
        for format in [TileFormat.bpp2, .bpp4, .bpp8, .mode7] {
            g.format = format
            let s = g.bitSource(x: 6, y: 2, plane: format.bitsPerPixel - 1)
            var bytes = Data(count: g.tileLen)
            bytes[s.byte] = 1 << s.bit
            let tile = decodeTile(bytes: bytes, format: format)
            #expect(tile.indices[2 * 8 + 6] == 1 << (format.bitsPerPixel - 1), "\(format)")
        }
    }

    @Test func paletteSpritesAndCellsSelectTheirBytes() async throws {
        let m = try await model()
        let g = m.graphics
        g.romOffset = 0x1800
        #expect(g.colours()[15].rgb == 0xFFFFFF, "the gray ramp ends at white")
        g.selectColour(3)
        #expect(m.highlightedRange == 0x1806..<0x1808)

        g.romOffset = 0x1A00
        #expect(g.sprites().count == 4)
        g.visibleSpritesOnly = false
        #expect(g.sprites().count == 128)
        g.selectSprite(2)
        #expect(m.highlightedRange == 0x1A08..<0x1A0C)

        g.romOffset = 0x2000
        let cells = g.cells()
        #expect(cells.count == 1024)
        #expect(cells[14 * 32 + 14].tile == 0x08)
        g.selectCell(14 * 32 + 14)
        let cell: Range<UInt32> = 0x239C..<0x239E  // entry 462, two bytes each
        #expect(m.highlightedRange == cell)
        // A 64-wide map keeps its right half in a second sub-map.
        g.screenSize = .s64x32
        g.selectCell(33)  // row 0, column 33
        #expect(m.highlightedRange?.lowerBound == UInt32(0x2802), "\(String(describing: m.highlightedRange))")
    }

    // MARK: Recordings

    @Test func aRecordingFeedsTheViewsAtAFrame() async throws {
        let m = try await model()
        let url = try recordingURL()
        defer { try? FileManager.default.removeItem(at: url) }
        #expect(RecordingController.attach(url: url, model: m, window: nil))
        let g = m.graphics
        #expect(g.source == .recording)
        #expect(m.graphicsTab == .tilemap)
        #expect(g.frameCount == 40)
        let bg = try #require(g.layerImage())
        #expect(bg.width == 256 && bg.height == 256)
        #expect(g.currentLayer?.mapWord == 0x1000)
        #expect(g.cells().count == 1024)
        g.backgroundLayer = 4
        #expect(g.layerImage() == nil, "mode 1 has no BG4")
        // Stepping moves sprite 0 a pixel a frame.
        g.visibleSpritesOnly = false
        let x0 = g.sprites()[0].x
        g.step(by: 5)
        #expect(g.frame == 5)
        #expect(g.sprites()[0].x == x0 + 5)
        g.step(by: 1000)
        #expect(g.frame == 39, "clamped to the last frame")
        #expect(g.spriteImage(0)?.width == 16)
        // The tile decoder reads VRAM.
        g.vramOffset = 0
        g.format = .bpp4
        #expect(g.sheet().width == 128)
        g.palette = .cgram(row: 1)
        #expect(g.paletteRGB(count: 16).count == 16)
        g.detach()
        #expect(g.source == .rom && !g.hasRecording)
    }

    @Test func aRecordingOfAnotherRomIsRefused() async throws {
        let m = try await Fixture.analyzedModel(rom: Fixture.smallRom())
        let session = try RecordingSession.fromBytes(bytes: makeTestRecording(frames: 1))
        #expect(throws: RomlensError.self) { try m.graphics.attach(session, name: "t.romrec") }
        #expect(!m.graphics.hasRecording)
    }

    // MARK: Previews

    @Test func aMarkedRangePreviewsAndOpensItsView() async throws {
        let m = try await model()
        m.jump(to: 0x1000)
        m.extendSelection(to: 0x13FF)
        m.mark(.data, dataKind: .graphics, bpp: 4)
        try await Fixture.settle { m.preview?.kind == "graphics" }
        let preview = try #require(m.preview)
        #expect(preview.bitmap?.width == 128)
        m.open(preview: preview)
        #expect(m.graphicsTab == .tiles)
        #expect(m.graphics.romOffset == 0x1000)
        #expect(m.graphics.format == .bpp4)
    }

    @Test func compressedDataOpensDecompressed() async throws {
        let m = try await model()
        m.jump(to: 0x3000)
        m.extendSelection(to: 0x3000 + 432)
        m.mark(.data, dataKind: .compressed)
        try await Fixture.settle { m.preview?.kind == "compressed" }
        let preview = try #require(m.preview)
        #expect(preview.decompressed?.count == 1024)
        m.open(preview: preview)
        guard case .bytes(_, let data) = m.graphics.source else {
            Issue.record("expected decompressed bytes")
            return
        }
        #expect(data == m.rom.bytes(fileOffset: 0x1000, len: 1024))
    }

    @Test func previewOptionsAreUndoable() async throws {
        let m = try await model()
        m.jump(to: 0x2000)
        m.extendSelection(to: 0x27FF)
        m.mark(.data, dataKind: .tilemap)
        let tiles = try #require(m.rom.snesAddressFor(fileOffset: 0x1000))
        try m.session.setRegionParams(
            start: 0x2000,
            params: RegionParamsInfo(palette: nil, columns: nil, screenSize: .s32x32, tiles: tiles)
        )
        #expect(m.session.undoTitle == "Set Preview Options")
        #expect(m.workbench.regionParamsAt(fileOffset: 0x2000)?.tiles == tiles)
        m.undo()
        #expect(m.workbench.regionParamsAt(fileOffset: 0x2000)?.tiles == nil)
    }

    // MARK: Views render

    @Test func everyViewLaysOutInAWindow() async throws {
        let m = try await model()
        let url = try recordingURL(frames: 3)
        defer { try? FileManager.default.removeItem(at: url) }
        let controller = RomWindowController(model: m)
        controller.window?.orderFront(nil)
        defer { controller.window?.close() }
        let content = try #require(controller.window?.contentView)
        for withRecording in [false, true] {
            if withRecording {
                #expect(RecordingController.attach(url: url, model: m, window: nil))
            }
            for tab in GraphicsModel.Tab.allCases {
                m.jump(to: 0x1000)
                m.openGraphics(tab)
                if tab == .tiles { m.graphics.hoveredPixel = (3, 4) }
                if tab == .palette { m.graphics.selectColour(5) }
                if tab == .oam { m.graphics.selectSprite(0) }
                if tab == .tilemap { m.graphics.selectCell(40) }
                content.layoutSubtreeIfNeeded()
                Fixture.spin(0.05)
                content.layoutSubtreeIfNeeded()
                content.display()
                #expect(m.graphicsTab == tab)
            }
        }
    }
}
