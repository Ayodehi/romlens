import Testing
@testable import RomlensKit

@Suite struct RomlensKitTests {
    @Test func opensTheHomebrewFixture() throws {
        let rom = try Rom.fromBytes(bytes: makeTestRom(mapping: .loRom), name: "t.sfc")
        let info = rom.info()
        #expect(info.title == "ROMLENS TEST")
        #expect(info.mapping == .loRom)
        #expect(info.checksumOk)
        #expect(rom.rowCount() == 2048)
    }

    @Test func hexRowsAreFlatBatches() throws {
        let rom = try Rom.fromBytes(bytes: makeTestRom(mapping: .loRom), name: "t.sfc")
        let batch = rom.hexRows(startRow: 0, count: 4)
        #expect(batch.count == 8 + 4 * Int(hexRowStride()))
        #expect(Int(hexBatchHeaderLen()) == 8)
        #expect(batch[8 + 12] == 0x78)
        #expect(batch[8 + 13] == 0x18)
        #expect(batch[8 + 14] == 0xFB)
    }

    @Test func resolvesAndFormats() throws {
        let rom = try Rom.fromBytes(bytes: makeTestRom(mapping: .loRom), name: "t.sfc")
        let r = try rom.resolve(text: "$00:8000")
        #expect(r.fileOffset == 0)
        #expect(r.row == 0)
        #expect(formatSnesAddress(address: 0x80841C) == "$80:841C")
        #expect(formatFileOffset(offset: 0x41C) == "0x00041C")
        #expect(throws: RomlensError.self) { try rom.resolve(text: "$7E:0000") }
        #expect(apiVersion() == "0.2.0")
    }

    @Test func inspectorAndSpans() throws {
        let rom = try Rom.fromBytes(bytes: makeTestRom(mapping: .hiRom), name: "t.sfc")
        let spans = rom.spans()
        #expect(spans.count == 22)
        let reset = try #require(spans.first { $0.name == "Emulation RESET" })
        #expect(reset.start == 0xFFFC)
        let byte = try #require(rom.inspect(fileOffset: 0xFFFC))
        #expect(byte.valueU16Le == 0x8000)
        #expect(byte.spanName == "Emulation RESET")
    }
}
