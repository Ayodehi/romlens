import RomlensKit
import SwiftUI

/// The tile decoder (checklist 2.20, 2.21): one tile as bytes, planes,
/// indices and pixels side by side, and the sheet it sits in.
///
/// Hovering a pixel lights its bit in every plane and the bytes those bits
/// live in. The lookup is a table fetched once per format, so moving the
/// pointer never crosses the FFI.
struct TileDecoderView: View {
    @Bindable var graphics: GraphicsModel

    var body: some View {
        let tile = graphics.selectedTileInfo()
        let colours = graphics.paletteRGB(count: graphics.format.colours)
        ScrollView([.vertical, .horizontal]) {
            VStack(alignment: .leading, spacing: 16) {
                controls
                HStack(alignment: .top, spacing: 20) {
                    ZoomedTile(tile: tile, colours: colours, graphics: graphics)
                    IndexGrid(tile: tile, graphics: graphics)
                    PlaneGrids(tile: tile, graphics: graphics)
                    TileBytes(tile: tile, graphics: graphics)
                }
                Divider()
                TileSheet(graphics: graphics)
            }
            .padding()
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private var controls: some View {
        HStack(spacing: 16) {
            Picker("Format", selection: $graphics.format) {
                ForEach([TileFormat.bpp2, .bpp4, .bpp8, .mode7], id: \.self) { f in
                    Text(f.title).tag(f)
                }
            }
            .pickerStyle(.segmented)
            .fixedSize()
            Menu(paletteTitle) {
                Button("Grayscale") { graphics.palette = .grayscale }
                Button("Colours at Tile Start in ROM") { graphics.palette = .rom(graphics.romOffset) }
                if graphics.hasRecording {
                    Menu("Recording CGRAM") {
                        ForEach(0..<16, id: \.self) { row in
                            Button(row < 8 ? "BG palette \(row)" : "OBJ palette \(row - 8)") {
                                graphics.palette = .cgram(row: UInt8(row))
                            }
                        }
                    }
                }
            }
            .fixedSize()
            Stepper("\(graphics.columns) across", value: $graphics.columns, in: 1...32)
                .fixedSize()
            Spacer()
        }
        .controlSize(.small)
    }

    private var paletteTitle: String {
        switch graphics.palette {
        case .grayscale: "Grayscale"
        case .rom(let o): "Colours at \(formatFileOffset(offset: o))"
        case .cgram(let row): "CGRAM row \(row)"
        }
    }
}

/// The tile at 24 points a pixel. Hover picks the pixel the other panes light.
struct ZoomedTile: View {
    let tile: TileInfo
    let colours: [UInt32]
    let graphics: GraphicsModel
    static let cell: CGFloat = 24

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Pixels").font(.caption).foregroundStyle(.secondary)
            Canvas { ctx, _ in
                for y in 0..<8 {
                    for x in 0..<8 {
                        let index = Int(tile.indices[y * 8 + x])
                        let rect = CGRect(x: CGFloat(x) * Self.cell, y: CGFloat(y) * Self.cell, width: Self.cell, height: Self.cell)
                        ctx.fill(Path(rect), with: .color(GraphicsStyle.colour(colours[index % colours.count])))
                    }
                }
                if let (x, y) = graphics.hoveredPixel {
                    let rect = CGRect(x: CGFloat(x) * Self.cell, y: CGFloat(y) * Self.cell, width: Self.cell, height: Self.cell)
                    ctx.stroke(Path(rect.insetBy(dx: 1, dy: 1)), with: .color(.accentColor), lineWidth: 2)
                }
            }
            .frame(width: Self.cell * 8, height: Self.cell * 8)
            .border(Color.secondary.opacity(0.4))
            .onContinuousHover { phase in
                switch phase {
                case .active(let p):
                    let x = Int(p.x / Self.cell), y = Int(p.y / Self.cell)
                    graphics.hoveredPixel = (0..<8).contains(x) && (0..<8).contains(y) ? (x, y) : nil
                case .ended:
                    graphics.hoveredPixel = nil
                }
            }
            if let (x, y) = graphics.hoveredPixel {
                let index = tile.indices[y * 8 + x]
                Text("(\(x), \(y)) = index \(index), \(bits(index))")
                    .font(.caption.monospaced())
            } else {
                Text("Hover a pixel").font(.caption).foregroundStyle(.tertiary)
            }
        }
    }

    private func bits(_ index: UInt8) -> String {
        let n = tile.format.bitsPerPixel
        return (0..<n).reversed().map { "p\($0)=\(index >> $0 & 1)" }.joined(separator: " ")
    }
}

/// The 64 colour indices as hex digits.
struct IndexGrid: View {
    let tile: TileInfo
    let graphics: GraphicsModel

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Indices").font(.caption).foregroundStyle(.secondary)
            Grid(horizontalSpacing: 4, verticalSpacing: 2) {
                ForEach(0..<8, id: \.self) { y in
                    GridRow {
                        ForEach(0..<8, id: \.self) { x in
                            let v = tile.indices[y * 8 + x]
                            let hot = graphics.hoveredPixel.map { $0.x == x && $0.y == y } ?? false
                            Text(graphics.format.bitsPerPixel == 8 ? GraphicsStyle.hex(v, 2) : GraphicsStyle.hex(v, 1))
                                .font(.callout.monospaced())
                                .foregroundStyle(v == 0 ? .tertiary : .primary)
                                .padding(.horizontal, 1)
                                .background(hot ? Color.accentColor.opacity(0.3) : .clear)
                        }
                    }
                }
            }
        }
    }
}

/// One 8×8 grid of bits per plane; the hovered pixel's bit is outlined in
/// each, which is the lesson: one pixel is one bit from every plane.
struct PlaneGrids: View {
    let tile: TileInfo
    let graphics: GraphicsModel

    var body: some View {
        let n = graphics.format.bitsPerPixel
        VStack(alignment: .leading, spacing: 4) {
            Text(graphics.format == .mode7 ? "Bits of each pixel's byte" : "Bitplanes")
                .font(.caption).foregroundStyle(.secondary)
            LazyVGrid(columns: Array(repeating: GridItem(.fixed(8 * 9 + 4)), count: min(n, 4)), alignment: .leading, spacing: 8) {
                ForEach(0..<n, id: \.self) { plane in
                    VStack(alignment: .leading, spacing: 2) {
                        Text("plane \(plane)").font(.caption2.monospaced()).foregroundStyle(.secondary)
                        Canvas { ctx, _ in
                            for y in 0..<8 {
                                for x in 0..<8 {
                                    let on = tile.indices[y * 8 + x] >> plane & 1 == 1
                                    let rect = CGRect(x: CGFloat(x) * 9, y: CGFloat(y) * 9, width: 8, height: 8)
                                    ctx.fill(Path(rect), with: .color(on ? .primary : .secondary.opacity(0.15)))
                                    if let h = graphics.hoveredPixel, h.x == x, h.y == y {
                                        ctx.stroke(Path(rect.insetBy(dx: -1, dy: -1)), with: .color(.accentColor), lineWidth: 2)
                                    }
                                }
                            }
                        }
                        .frame(width: 8 * 9, height: 8 * 9)
                    }
                }
            }
        }
    }
}

/// The tile's bytes, with the ones holding the hovered pixel's bits lit.
struct TileBytes: View {
    let tile: TileInfo
    let graphics: GraphicsModel

    var body: some View {
        let hot: Set<Int> = {
            guard let (x, y) = graphics.hoveredPixel else { return [] }
            return Set((0..<graphics.format.bitsPerPixel).map { graphics.bitSource(x: x, y: y, plane: $0).byte })
        }()
        VStack(alignment: .leading, spacing: 4) {
            Text("Bytes").font(.caption).foregroundStyle(.secondary)
            let perRow = graphics.format == .mode7 ? 8 : 2
            Grid(horizontalSpacing: 6, verticalSpacing: 2) {
                ForEach(0..<(tile.bytes.count / perRow), id: \.self) { row in
                    GridRow {
                        Text(GraphicsStyle.hex(row * perRow, 2))
                            .font(.caption.monospaced()).foregroundStyle(.tertiary)
                        ForEach(0..<perRow, id: \.self) { col in
                            let i = row * perRow + col
                            Text(GraphicsStyle.hex(tile.bytes[i], 2))
                                .font(.callout.monospaced())
                                .padding(.horizontal, 2)
                                .background(hot.contains(i) ? Color.accentColor.opacity(0.35) : .clear)
                        }
                    }
                }
            }
            if graphics.format != .mode7 {
                Text("Rows 0–7 of planes 0 and 1, then 2 and 3…")
                    .font(.caption2).foregroundStyle(.tertiary)
            }
        }
    }
}

/// The sheet of consecutive tiles; clicking one decodes it above and selects
/// its bytes in the editor.
struct TileSheet: View {
    @Bindable var graphics: GraphicsModel
    static let scale: CGFloat = 3

    var body: some View {
        let sheet = graphics.sheet()
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text("Tile \(graphics.selectedTile) of \(graphics.sheetTiles), \(graphics.tileLen) bytes each")
                    .font(.caption).foregroundStyle(.secondary)
                Spacer()
                if graphics.source == .rom {
                    Button("Previous \(graphics.sheetTiles)") { page(-1) }
                        .disabled(graphics.romOffset == 0)
                    Button("Next \(graphics.sheetTiles)") { page(1) }
                }
            }
            .controlSize(.small)
            ZStack(alignment: .topLeading) {
                PixelImage(bitmap: sheet, scale: Self.scale)
                selectionBox
            }
            .onTapGesture(coordinateSpace: .local) { p in
                let col = Int(p.x / (8 * Self.scale)), row = Int(p.y / (8 * Self.scale))
                guard col < graphics.columns else { return }
                let index = row * graphics.columns + col
                if index < graphics.sheetTiles { graphics.selectTile(index) }
            }
        }
    }

    private var selectionBox: some View {
        let s = 8 * Self.scale
        let col = graphics.selectedTile % max(graphics.columns, 1)
        let row = graphics.selectedTile / max(graphics.columns, 1)
        return Rectangle()
            .stroke(Color.accentColor, lineWidth: 2)
            .frame(width: s, height: s)
            .offset(x: CGFloat(col) * s, y: CGFloat(row) * s)
    }

    private func page(_ delta: Int) {
        let step = Int64(graphics.sheetTiles * graphics.tileLen) * Int64(delta)
        graphics.romOffset = UInt32(max(0, Int64(graphics.romOffset) + step))
        graphics.selectTile(0)
    }
}
