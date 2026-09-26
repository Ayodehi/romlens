import Foundation
import Testing
@testable import RomlensKit

/// The track 2B surface as Swift sees it: decoders over bytes, images as one
/// RGBA buffer, and a recording opened from bytes.
@Suite struct GraphicsTests {
    @Test func romBytesDecodeAsTilesPalettesAndSprites() throws {
        let rom = try Rom.fromBytes(bytes: makeGraphicsTestRom(), name: "g.sfc")
        let tiles = rom.bytes(fileOffset: 0x1000, len: 1024)
        #expect(tiles.count == 1024)
        let sheet = tileSheet(bytes: tiles, format: .bpp4, count: 32, columns: 16, palette: .grayscale)
        #expect(sheet.width == 128 && sheet.height == 16)
        #expect(sheet.rgba.count == 128 * 16 * 4)
        let palette = paletteEntries(bytes: rom.bytes(fileOffset: 0x1800, len: 512), count: 256)
        #expect(palette[15].rgb == 0xFFFFFF)
        let sprites = oamEntries(bytes: rom.bytes(fileOffset: 0x1A00, len: 544), obsel: 0, sort: .table)
        #expect(sprites.filter(\.onScreen).count == 4)
        let bits = tileBitSources(format: .bpp4)
        #expect(bits.count == 64 * 4)
        #expect(tileByteLen(format: .bpp2) == 16)
    }

    @Test func mode7CellsReadLowBytes() {
        var vram = Data(count: 0x10000)
        vram[2 * (128 + 3)] = 0x42
        let cells = mode7Cells(vram: vram)
        #expect(cells.count == 128 * 128)
        #expect(cells[128 + 3].tile == 0x42 && cells[128 + 3].byteOffset == 262)
    }

    @Test func aRecordingRendersItsLayers() throws {
        let rec = try RecordingSession.fromBytes(bytes: makeTestRecording(frames: 12))
        #expect(rec.info().frameCount == 12)
        let bg = try rec.renderBg(frame: 0, bg: 1)
        #expect(bg.width == 256 && bg.height == 256)
        #expect(throws: RomlensError.self) { try rec.renderBg(frame: 0, bg: 4) }
        let rom = try Rom.fromBytes(bytes: makeGraphicsTestRom(), name: "g.sfc")
        try rec.checkRom(rom: rom)
        let other = try Rom.fromBytes(bytes: makeTestRom(mapping: .loRom), name: "t.sfc")
        #expect(throws: RomlensError.self) { try rec.checkRom(rom: other) }
    }

    @Test func aFrameNamesWhatDrewEachPixel() throws {
        let rec = try RecordingSession.fromBytes(bytes: makeTestRecording(frames: 12))
        let f = try rec.renderFrame(frame: 8)
        #expect(f.image.width == 256 && f.image.height == 224)
        #expect(f.bgMode == 1)
        guard case .sprite(let sprite, _, _, _, _, _, _, _) = try rec.framePixel(frame: 8, x: 150, y: 55) else {
            Issue.record("no sprite at (150, 55)")
            return
        }
        #expect(sprite == 3)
        #expect(try rec.framePixel(frame: 8, x: 4, y: 4) == .backdrop)
        #expect(try rec.priorityOrder(frame: 8).contains("sprites of priority 3"))
        let objs = try rec.renderFrameLayer(frame: 8, layer: 5, colourMath: false)
        #expect(objs.width == 256)
    }
}
