import Foundation
import Observation
import RomlensKit

/// The four graphics views' shared state: what they read, how they decode it,
/// and what is selected inside them.
///
/// They read either raw ROM bytes at an offset (the default, and all Phase 2
/// needs for a ROM nobody has recorded) or a recording's VRAM, CGRAM and OAM
/// at a frame. Every selection that has a ROM byte range selects that range
/// in the editor too, which is what keeps one selection across code and
/// graphics (docs/02); the byte range is the join key, so no second identity
/// model is needed.
@MainActor
@Observable
final class GraphicsModel {
    enum Tab: String, CaseIterable, Identifiable {
        case tiles, palette, oam, tilemap
        var id: String { rawValue }
        var title: String {
            switch self {
            case .tiles: "Tile Decoder"
            case .palette: "Palette"
            case .oam: "OAM"
            case .tilemap: "Tilemap"
            }
        }
        var systemImage: String {
            switch self {
            case .tiles: "square.grid.3x3"
            case .palette: "paintpalette"
            case .oam: "list.bullet.rectangle"
            case .tilemap: "map"
            }
        }
    }

    /// What the views read.
    enum Source: Equatable {
        /// ROM bytes from this file offset.
        case rom
        /// The attached recording at `frame`.
        case recording
        /// Bytes that are not in the ROM as they stand: a decompressed block.
        case bytes(label: String, data: Data)
    }

    /// Where a tile view's colours come from.
    enum PaletteChoice: Equatable {
        case grayscale
        /// BGR15 colours in the ROM at this file offset.
        case rom(UInt32)
        /// A CGRAM row of the attached recording.
        case cgram(row: UInt8)
    }

    enum OamSortChoice: String, CaseIterable, Identifiable {
        case table, screen, priority
        var id: String { rawValue }
        var title: String {
            switch self {
            case .table: "Table Order"
            case .screen: "Screen Position"
            case .priority: "Priority"
            }
        }
        var core: OamSort {
            switch self {
            case .table: .table
            case .screen: .screen
            case .priority: .priority
            }
        }
    }

    let rom: Rom
    var source: Source = .rom
    /// The ROM offset the views start at.
    var romOffset: UInt32 = 0
    /// With a recording: the VRAM byte offset the tile views start at.
    var vramOffset: UInt32 = 0

    // Tile decoder
    var format: TileFormat = .bpp4
    var columns: Int = 16
    var sheetTiles: Int = 256
    var palette: PaletteChoice = .grayscale
    /// Index into the sheet of the tile the decoder shows.
    var selectedTile: Int = 0
    /// The pixel under the pointer in the zoomed tile.
    var hoveredPixel: (x: Int, y: Int)?

    // Palette
    var selectedColour: Int?

    // OAM
    var obsel: UInt8 = 0
    var oamSort: OamSortChoice = .table
    var visibleSpritesOnly = true
    var selectedSprite: UInt8?

    // Tilemap
    var screenSize: ScreenSize = .s32x32
    var backgroundLayer: UInt8 = 1
    var selectedCell: Int?

    // Recording
    private(set) var recording: RecordingSession?
    private(set) var recordingInfo: RecordingInfo?
    private(set) var recordingName: String?
    var frame: UInt64 = 0 {
        didSet { if frame != oldValue { clampFrame() } }
    }

    /// Where selecting something with a ROM byte range should send it.
    @ObservationIgnored var selectBytes: ((Range<UInt32>) -> Void)?
    /// Switch the editor to the tile decoder.
    @ObservationIgnored var revealTile: (() -> Void)?

    init(rom: Rom) {
        self.rom = rom
    }

    // MARK: Recording

    /// Attach a recording, refusing one of another ROM with the core's words.
    func attach(_ session: RecordingSession, name: String) throws {
        try session.checkRom(rom: rom)
        recording = session
        recordingInfo = session.info()
        recordingName = name
        frame = 0
        source = .recording
        if let ppu = try? session.ppuSummary(frame: 0) {
            obsel = ppu.obsel
        }
    }

    func detach() {
        recording = nil
        recordingInfo = nil
        recordingName = nil
        if source == .recording { source = .rom }
    }

    var frameCount: UInt64 { recordingInfo?.frameCount ?? 0 }
    var hasRecording: Bool { recording != nil }

    func step(by delta: Int) {
        guard frameCount > 0 else { return }
        let next = Int64(frame) + Int64(delta)
        frame = UInt64(min(max(next, 0), Int64(frameCount) - 1))
    }

    private func clampFrame() {
        if frameCount > 0, frame >= frameCount { frame = frameCount - 1 }
    }

    /// The registers at the current frame, when reading a recording.
    var ppu: PpuSummary? {
        guard source == .recording, let recording else { return nil }
        return try? recording.ppuSummary(frame: frame)
    }

    private func region(_ r: StateRegion) -> Data? {
        guard let recording else { return nil }
        return try? recording.region(frame: frame, region: r)
    }

    // MARK: Bytes

    /// `len` bytes the tile views decode, from wherever the source says.
    func tileSourceBytes(_ len: Int) -> Data {
        switch source {
        case .rom:
            return rom.bytes(fileOffset: romOffset, len: UInt32(len))
        case .recording:
            guard let vram = region(.vram) else { return Data() }
            let start = min(Int(vramOffset), vram.count)
            return vram.subdata(in: start..<min(start + len, vram.count))
        case .bytes(_, let data):
            return data.prefix(len)
        }
    }

    /// 512 bytes of colours: CGRAM, or ROM from the offset.
    func paletteBytes() -> Data {
        switch source {
        case .recording: region(.cgram) ?? Data()
        case .rom: rom.bytes(fileOffset: romOffset, len: 512)
        case .bytes(_, let data): data.prefix(512)
        }
    }

    func oamBytes() -> Data {
        switch source {
        case .recording: region(.oam) ?? Data()
        case .rom: rom.bytes(fileOffset: romOffset, len: 544)
        case .bytes(_, let data): data.prefix(544)
        }
    }

    /// The map bytes: VRAM at the layer's map address, or ROM.
    func tilemapBytes() -> Data {
        let len = screenSize.cells.columns * screenSize.cells.rows * 2
        switch source {
        case .recording:
            guard let vram = region(.vram), let layer = currentLayer else { return Data() }
            let start = Int(layer.mapWord) * 2
            // A map runs off the end of VRAM and wraps, like the hardware.
            let doubled = vram + vram
            return doubled.subdata(in: start..<start + len)
        case .rom: return rom.bytes(fileOffset: romOffset, len: UInt32(len))
        case .bytes(_, let data): return data.prefix(len)
        }
    }

    var currentLayer: BgLayerInfo? {
        ppu?.layers.first { $0.bg == backgroundLayer }
    }

    /// What the header says the views are reading.
    var sourceDescription: String {
        switch source {
        case .rom:
            let snes = rom.snesAddressFor(fileOffset: romOffset).map { formatSnesAddress(address: $0) + " · " } ?? ""
            return "ROM " + snes + formatFileOffset(offset: romOffset)
        case .recording:
            return "\(recordingName ?? "Recording"), frame \(frame) of \(frameCount)"
        case .bytes(let label, let data):
            return "\(label), \(data.count) bytes"
        }
    }

    // MARK: Tile decoder

    var tileLen: Int { Int(tileByteLen(format: format)) }

    /// The whole table of pixel-to-bit sources for the format, fetched once.
    @ObservationIgnored private var bitTable: (TileFormat, [UInt16])?

    func bitSource(x: Int, y: Int, plane: Int) -> (byte: Int, bit: Int) {
        if bitTable?.0 != format { bitTable = (format, tileBitSources(format: format)) }
        let entry = bitTable!.1[(y * 8 + x) * format.bitsPerPixel + plane]
        return (Int(entry >> 3), Int(entry & 7))
    }

    var paletteSource: PaletteSource {
        switch palette {
        case .grayscale:
            return .grayscale
        case .rom(let offset):
            return .colours(bytes: rom.bytes(fileOffset: offset, len: UInt32(format.colours * 2)))
        case .cgram(let row):
            return region(.cgram).map { .cgram(cgram: $0, row: row, obj: false) } ?? .grayscale
        }
    }

    func sheet() -> BitmapInfo {
        tileSheet(
            bytes: tileSourceBytes(tileLen * sheetTiles),
            format: format,
            count: UInt32(sheetTiles),
            columns: UInt32(columns),
            palette: paletteSource
        )
    }

    func selectedTileInfo() -> TileInfo {
        let all = tileSourceBytes(tileLen * (selectedTile + 1))
        let start = min(selectedTile * tileLen, all.count)
        return decodeTile(bytes: all.subdata(in: start..<all.count), format: format)
    }

    /// `0xRRGGBB` per index under the current palette choice, for drawing
    /// the zoomed tile.
    func paletteRGB(count: Int) -> [UInt32] {
        switch paletteSource {
        case .grayscale:
            return (0..<count).map { i in
                let v = UInt32(i * 255 / max(count - 1, 1))
                return v << 16 | v << 8 | v
            }
        case .colours(let bytes):
            return paletteEntries(bytes: bytes, count: UInt16(count)).map(\.rgb)
        case .cgram(let cgram, let row, _):
            let entries = paletteEntries(bytes: cgram, count: 256)
            let first = count >= 256 ? 0 : Int(row) * count
            return (0..<count).map { entries[(first + $0) % 256].rgb }
        }
    }

    /// Select tile `index` of the sheet, and its bytes in the editor.
    func selectTile(_ index: Int) {
        selectedTile = max(0, index)
        hoveredPixel = nil
        if source == .rom {
            let start = romOffset + UInt32(selectedTile * tileLen)
            selectBytes?(start..<start + UInt32(tileLen))
        }
    }

    // MARK: Palette, OAM, tilemap

    func colours() -> [PaletteEntryInfo] {
        paletteEntries(bytes: paletteBytes(), count: 256)
    }

    func selectColour(_ index: Int) {
        selectedColour = index
        if source == .rom {
            let start = romOffset + UInt32(index * 2)
            selectBytes?(start..<start + 2)
        }
    }

    func sprites() -> [OamEntryInfo] {
        let rows = oamEntries(bytes: oamBytes(), obsel: obsel, sort: oamSort.core)
        return visibleSpritesOnly ? rows.filter(\.onScreen) : rows
    }

    /// Selecting a sprite selects its four low-table bytes; the high-table
    /// bits sit in a byte four sprites share, so they are named in the row
    /// rather than selected.
    func selectSprite(_ index: UInt8?) {
        selectedSprite = index
        guard let index, source == .rom else { return }
        let start = romOffset + UInt32(index) * 4
        selectBytes?(start..<start + 4)
    }

    func cells() -> [TilemapCellInfo] {
        if isMode7 {
            return mode7Cells(vram: region(.vram) ?? Data())
        }
        return tilemapCells(bytes: tilemapBytes(), size: currentLayer?.size ?? screenSize)
    }

    /// Whether the Tilemap view is showing the Mode 7 plane: one fixed
    /// 128×128 map of byte entries, not a `BGnSC` map.
    var isMode7: Bool {
        source == .recording && currentLayer?.format == .mode7
    }

    /// The map's cells across and down, as the view lays them out.
    var mapCells: (columns: Int, rows: Int) {
        isMode7 ? (128, 128) : (currentLayer?.size ?? screenSize).cells
    }

    /// Select cell `index` in reading order. Its bytes come from the cell,
    /// not the index: a 64-wide map stores its right half in a second
    /// sub-map, so reading order and storage order differ.
    func selectCell(_ index: Int) {
        selectedCell = index
        guard source == .rom else { return }
        let all = cells()
        guard all.indices.contains(index) else { return }
        let start = romOffset + all[index].byteOffset
        selectBytes?(start..<start + 2)
    }

    /// The rendered layer: from the recording's registers, or nil for ROM
    /// bytes, where the inspector's preview (with a tile address set) is the
    /// drawing and this view shows the entries.
    func layerImage() -> BitmapInfo? {
        guard source == .recording, let recording else { return nil }
        return try? recording.renderBg(frame: frame, bg: backgroundLayer)
    }

    func spriteImage(_ index: UInt8) -> BitmapInfo? {
        guard source == .recording, let recording else { return nil }
        return try? recording.renderSprite(frame: frame, index: index)
    }
}
