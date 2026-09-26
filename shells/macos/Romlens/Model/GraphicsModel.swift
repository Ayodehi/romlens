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
        case frame, layers, tiles, palette, oam, tilemap
        var id: String { rawValue }
        var title: String {
            switch self {
            case .frame: "Frame"
            case .layers: "Layers"
            case .tiles: "Tile Decoder"
            case .palette: "Palette"
            case .oam: "OAM"
            case .tilemap: "Tilemap"
            }
        }
        var systemImage: String {
            switch self {
            case .frame: "photo"
            case .layers: "square.3.layers.3d"
            case .tiles: "square.grid.3x3"
            case .palette: "paintpalette"
            case .oam: "list.bullet.rectangle"
            case .tilemap: "map"
            }
        }
        /// Views that need a recording: the screen exists only in one.
        var needsRecording: Bool { self == .frame || self == .layers }
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

    // Frame
    /// The pixel clicked in the Frame view, and the one under the pointer.
    var selectedPixel: (x: Int, y: Int)?
    var hoverPixel: (x: Int, y: Int)?
    /// How many screen pixels a frame pixel takes.
    var frameScale: Int = 2
    /// The Tilemap view's size: fitted to the window (0), or screen pixels
    /// a map pixel.
    var tilemapScale: Int = 2
    /// The Layers view scales its grid to the window; off, 1:1.
    var layersFit = true
    /// The Layers view shows each layer with its colour math done, as it
    /// shows on screen; off, in its own colours.
    var layersColourMath = true

    // Recording
    private(set) var recording: RecordingSession?
    private(set) var recordingInfo: RecordingInfo?
    private(set) var recordingName: String?
    var frame: UInt64 = 0 {
        didSet {
            if frame != oldValue { clampFrame() }
            // Moving off the newest frame pauses following; coming back to
            // it resumes. Only a person moves the frame while live, since a
            // new frame arriving sets it under `applyingLive`.
            if live != nil, !applyingLive { followLive = frame + 1 >= frameCount }
        }
    }

    // Live
    /// The live session the recording is fed from, while one runs.
    private(set) var live: LiveSession?
    /// What the live session is doing, in words.
    private(set) var liveStatus: String?
    /// Show each frame as it arrives.
    var followLive = true
    /// Instructions the live session's execution log has added to the
    /// project so far.
    private(set) var liveDiscovered: UInt64 = 0
    @ObservationIgnored private var applyingLive = false

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
        screenLinesCache = nil
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
        live?.stop()
        live = nil
        liveStatus = nil
        screenLinesCache = nil
        recording = nil
        recordingInfo = nil
        recordingName = nil
        if source == .recording { source = .rom }
    }

    /// Read a live session's frames as the recording, following the newest.
    func attachLive(_ session: LiveSession) throws {
        try attach(session.recording(), name: "Live")
        live = session
        followLive = true
        liveDiscovered = 0
        liveStatus = "waiting for Mesen on port \(session.port())"
        // The session may have heard from Mesen before this attached.
        if let status = session.status() { liveStatusChanged(status) }
        if let latest = session.latestFrame() { liveArrived(latest: latest) }
    }

    /// New frames have arrived; `latest` is the newest.
    func liveArrived(latest: UInt64) {
        guard let recording, live != nil else { return }
        recordingInfo = recording.info()
        guard followLive, latest != frame else { return }
        applyingLive = true
        frame = latest
        applyingLive = false
    }

    func liveStatusChanged(_ status: LiveStatus) {
        guard live != nil else { return }
        switch status {
        case .listening(let port):
            if liveStatus?.hasPrefix("streaming") == true || liveStatus == nil {
                liveStatus = "waiting for Mesen on port \(port)"
            }
        case .connected:
            liveStatus = "streaming"
        case .disconnected(let reason):
            liveStatus = "\(reason); waiting for Mesen"
        case .refused(let reason):
            liveStatus = "refused: \(reason)"
        }
    }

    /// The project gained `added` instructions from the live execution log.
    func liveLogMerged(added: UInt64) {
        liveDiscovered += added
    }

    /// Stop listening, keeping the frames already received.
    func stopLive() {
        live?.stop()
        live = nil
        liveStatus = nil
        if recording != nil { recordingName = "Live (stopped)" }
        recordingInfo = recording?.info()
    }

    var isLive: Bool { live != nil }

    var frameCount: UInt64 { recordingInfo?.frameCount ?? 0 }
    /// The oldest readable frame: a live session keeps only its latest.
    var firstFrame: UInt64 { recordingInfo?.firstFrame ?? 0 }
    var hasRecording: Bool { recording != nil }

    func step(by delta: Int) {
        guard frameCount > 0 else { return }
        let next = Int64(frame) + Int64(delta)
        frame = UInt64(min(max(next, Int64(firstFrame)), Int64(frameCount) - 1))
    }

    private func clampFrame() {
        if frameCount > 0, frame >= frameCount { frame = frameCount - 1 }
        if frame < firstFrame { frame = firstFrame }
    }

    /// The registers at the current frame, when reading a recording.
    var ppu: PpuSummary? {
        guard source == .recording, let recording else { return nil }
        // A frame never changes once recorded, and the tilemap asks once per
        // cell while drawing: a thousand calls into the core per frame kept a
        // live session's main thread busy full time.
        let id = ObjectIdentifier(recording)
        if let c = ppuCache, c.session == id, c.frame == frame { return c.value }
        let value = try? recording.ppuSummary(frame: frame)
        ppuCache = (id, frame, value)
        return value
    }
    @ObservationIgnored private var ppuCache: (session: ObjectIdentifier, frame: UInt64, value: PpuSummary?)?

    private func region(_ r: StateRegion) -> Data? {
        guard let recording else { return nil }
        return try? recording.region(frame: frame, region: r)
    }

    /// A region at the current frame, for File › Export Frame Region….
    func regionBytes(_ r: StateRegion) -> Data? { region(r) }

    // MARK: Changes

    /// One region's comparison with the previous frame: which bytes differ,
    /// and both frames' bytes for the checks finer than a byte.
    struct FrameChange {
        let frame: UInt64
        let bytes: IndexSet
        let before: [UInt8]
        let now: [UInt8]
    }

    @ObservationIgnored private var changeCache: [StateRegion: FrameChange] = [:]

    /// Exactly what changed in `r` since the previous frame. The recording's
    /// change runs are a superset (nearby changes share a run), so they only
    /// narrow the search; the two frames' bytes decide. Cached per frame,
    /// since every swatch and cell asks.
    func change(_ r: StateRegion) -> FrameChange? {
        guard source == .recording, let recording, frame > 0 else { return nil }
        if let c = changeCache[r], c.frame == frame { return c }
        guard let flat = try? recording.changes(from: frame - 1, to: frame, region: r) else { return nil }
        var bytes = IndexSet()
        var before: [UInt8] = [], now: [UInt8] = []
        if !flat.isEmpty,
           let a = try? recording.region(frame: frame - 1, region: r),
           let b = try? recording.region(frame: frame, region: r) {
            before = [UInt8](a)
            now = [UInt8](b)
            for i in stride(from: 0, to: flat.count - 1, by: 2) {
                let start = Int(flat[i]), end = min(start + Int(flat[i + 1]), before.count, now.count)
                for at in start..<max(start, end) where before[at] != now[at] { bytes.insert(at) }
            }
        }
        let c = FrameChange(frame: frame, bytes: bytes, before: before, now: now)
        changeCache[r] = c
        return c
    }

    /// Whether any of `len` bytes from `offset` changed since the previous frame.
    func changed(_ r: StateRegion, offset: Int, len: Int) -> Bool {
        guard let c = change(r), !c.bytes.isEmpty else { return false }
        return c.bytes.intersects(integersIn: offset..<offset + max(len, 1))
    }

    /// Whether sprite `index` changed: its four low-table bytes, or its own
    /// two bits of the high-table byte it shares with three others.
    func spriteChanged(_ index: UInt8) -> Bool {
        let low = Int(index) * 4
        if changed(.oam, offset: low, len: 4) { return true }
        guard let c = change(.oam) else { return false }
        let high = 0x200 + Int(index) / 4, shift = Int(index % 4) * 2
        guard c.bytes.contains(high), high < c.before.count, high < c.now.count else { return false }
        return (c.before[high] >> shift) & 3 != (c.now[high] >> shift) & 3
    }

    /// When these bytes last changed at or before this frame and next change
    /// after it; nil unless reading a recording.
    func history(_ r: StateRegion, offset: UInt32, len: UInt32) -> ChangeHistory? {
        guard source == .recording, let recording else { return nil }
        return try? recording.history(region: r, offset: offset, len: len, frame: frame)
    }

    /// Where a tilemap cell's entry sits in VRAM, reading a recording.
    func cellVram(_ cell: TilemapCellInfo) -> (offset: UInt32, len: UInt32)? {
        guard source == .recording else { return nil }
        if isMode7 { return (cell.byteOffset, 1) }
        guard let layer = currentLayer else { return nil }
        return ((UInt32(layer.mapWord) * 2 + cell.byteOffset) % 0x10000, 2)
    }

    /// Whether the screen was off or dimmed at this frame (INIDISP), which
    /// is why a frame can show nothing that makes sense. From the lines the
    /// frame drew where the recording has its register writes: a game turns
    /// the screen off in vertical blank for its uploads and on again before
    /// drawing, so INIDISP as the frame ended says "off" of a frame drawn in
    /// full.
    var screenNote: String? {
        if let lines = screenLines() {
            if lines.blank == lines.lines { return "screen off (forced blank)" }
            if lines.blank > 0 { return "screen off on \(lines.blank) of \(lines.lines) lines" }
            if lines.brightnessMax < 15 {
                return lines.brightnessMin == lines.brightnessMax
                    ? "brightness \(lines.brightnessMax)/15"
                    : "brightness \(lines.brightnessMin)–\(lines.brightnessMax)/15"
            }
            return nil
        }
        guard let inidisp = ppu?.inidisp else { return nil }
        if inidisp & 0x80 != 0 { return "screen off (forced blank)" }
        let brightness = inidisp & 0x0F
        return brightness < 15 ? "brightness \(brightness)/15" : nil
    }

    @ObservationIgnored private var screenLinesCache: (frame: UInt64, info: ScreenLinesInfo?)?

    /// The screen's state on the drawn lines of this frame, kept per frame.
    private func screenLines() -> ScreenLinesInfo? {
        guard source == .recording, let recording else { return nil }
        if let c = screenLinesCache, c.frame == frame { return c.info }
        let info = (try? recording.screenLines(frame: frame)) ?? nil
        screenLinesCache = (frame, info)
        return info
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
            if let liveStatus {
                let found = liveDiscovered > 0 ? ", \(liveDiscovered) instructions found" : ""
                return "Live, frame \(frame), \(liveStatus)\(found)"
            }
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

    // MARK: Frame and layers

    /// The screen at the current frame, drawn from the PPU state. Kept per
    /// frame: the view redraws on every pointer move.
    func frameImage() -> FrameImageInfo? {
        guard let recording else { return nil }
        let id = ObjectIdentifier(recording)
        if let c = frameCache, c.session == id, c.frame == frame { return c.value }
        let value = try? recording.renderFrame(frame: frame)
        frameCache = (id, frame, value)
        return value
    }
    @ObservationIgnored private var frameCache: (session: ObjectIdentifier, frame: UInt64, value: FrameImageInfo?)?

    /// What drew a pixel of the current frame.
    func pixel(x: Int, y: Int) -> PixelWinnerInfo? {
        guard let recording, x >= 0, y >= 0 else { return nil }
        return (try? recording.framePixel(frame: frame, x: UInt32(x), y: UInt32(y))) ?? nil
    }

    /// One layer alone: 1–4 a background, 5 the sprites.
    func frameLayer(_ layer: UInt8) -> BitmapInfo? {
        guard let recording else { return nil }
        return try? recording.renderFrameLayer(frame: frame, layer: layer, colourMath: layersColourMath)
    }

    /// The layers some line of this frame draws, and its modes line by
    /// line (a game can change mode part way down the screen).
    func frameLayers() -> FrameLayersInfo? {
        guard let recording else { return nil }
        return try? recording.frameLayers(frame: frame)
    }

    func priorityOrder() -> [String] {
        guard let recording else { return [] }
        return (try? recording.priorityOrder(frame: frame)) ?? []
    }

    /// Where the views should go for a pixel's winner.
    enum Reveal { case sprite, tile, cell, colour }

    /// Point the other views at what drew a pixel: its OAM entry, its tile in
    /// the Tile Decoder, its tilemap cell, or its colour. Returns the view to
    /// show.
    func reveal(_ what: Reveal, of winner: PixelWinnerInfo) -> Tab? {
        source = .recording
        switch (what, winner) {
        case (.sprite, .sprite(let sprite, _, _, _, _, _, _, _)):
            visibleSpritesOnly = false
            selectedSprite = sprite
            return .oam
        case (.tile, .sprite(_, _, let tileWord, _, _, _, let colour, _)):
            showTile(word: tileWord, format: .bpp4, colour: colour)
            return .tiles
        case (.tile, .background(let layer, _, _, _, let tileWord, _, _, _, let colour)):
            // Mode 7's tiles sit in VRAM's high bytes, interleaved with the
            // map: the Tilemap view shows them, the Tile Decoder cannot.
            guard let format = ppu?.layers.first(where: { $0.bg == layer })?.format, format != .mode7 else {
                return nil
            }
            showTile(word: tileWord, format: format, colour: colour)
            return .tiles
        case (.cell, .background(let layer, let mapWord, _, _, _, _, _, _, _)):
            backgroundLayer = layer
            let all = cells()
            let offset: UInt32 = isMode7
                ? UInt32(mapWord) * 2
                : UInt32(mapWord &- (currentLayer?.mapWord ?? 0)) * 2
            selectedCell = all.firstIndex { $0.byteOffset == offset }
            return .tilemap
        case (.colour, _):
            guard let colour = winner.colour else { return nil }
            selectedColour = Int(colour)
            return .palette
        default:
            return nil
        }
    }

    private func showTile(word: UInt16, format: TileFormat, colour: UInt8) {
        self.format = format
        vramOffset = UInt32(word) * 2
        selectedTile = 0
        hoveredPixel = nil
        let colours = max(format.colours, 1)
        palette = .cgram(row: UInt8(Int(colour) / colours))
    }
}

extension PixelWinnerInfo {
    /// The CGRAM colour it shows, where it has one.
    var colour: UInt8? {
        switch self {
        case .blank: nil
        case .backdrop: 0
        case .background(_, _, _, _, _, _, _, _, let colour): colour
        case .sprite(_, _, _, _, _, _, let colour, _): colour
        }
    }

    /// One line for the status bar.
    var summary: String {
        switch self {
        case .blank:
            return "the screen is off here (forced blank)"
        case .backdrop:
            return "the backdrop: no layer drew here, so it shows CGRAM colour 0"
        case .background(let layer, let mapWord, let entry, let tile, _, let x, let y, let index, let colour):
            let pal = (entry >> 10) & 7
            let flips = (entry & 0x4000 != 0 ? ", h-flip" : "") + (entry & 0x8000 != 0 ? ", v-flip" : "")
            return "BG\(layer): tilemap entry at VRAM $\(GraphicsStyle.hex(mapWord, 4)) = $\(GraphicsStyle.hex(entry, 4)) (tile $\(GraphicsStyle.hex(tile, 3)), palette \(pal)\(entry & 0x2000 != 0 ? ", high priority" : "")\(flips)); pixel (\(x), \(y)) of the tile, index \(index), colour \(colour)"
        case .sprite(let sprite, let tile, let tileWord, let x, let y, let index, let colour, let priority):
            return "sprite \(sprite) (priority \(priority)): tile $\(GraphicsStyle.hex(tile, 3)) at VRAM $\(GraphicsStyle.hex(tileWord, 4)); pixel (\(x), \(y)) of the tile, index \(index), colour \(colour)"
        }
    }
}
