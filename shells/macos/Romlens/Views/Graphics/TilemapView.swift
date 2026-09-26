import RomlensKit
import SwiftUI

/// A BG tilemap (checklist 2.24, 2.26): each cell's `vhopppcc cccccccc`
/// decoded, over the rendered layer when a recording supplies VRAM and the
/// registers, or as tile numbers over ROM bytes.
struct TilemapView: View {
    let model: RomViewModel
    @Bindable var graphics: GraphicsModel

    var body: some View {
        let cells = graphics.cells()
        let size = graphics.mapCells
        let image = graphics.layerImage()
        // Map pixels a cell: 8, or 16 for 16×16 tiles.
        let unit = image.map { CGFloat($0.width) / CGFloat(max(size.columns, 1)) } ?? 8
        VStack(spacing: 0) {
            controls
            Divider()
            HStack(alignment: .top, spacing: 0) {
                if graphics.tilemapScale == 0 {
                    GeometryReader { geo in
                        let pad: CGFloat = 16
                        let across = (geo.size.width - 2 * pad) / (CGFloat(size.columns) * unit)
                        let down = (geo.size.height - 2 * pad) / (CGFloat(size.rows) * unit)
                        map(cells, size: size, image: image, cell: unit * max(0.1, min(across, down)))
                            .padding(pad)
                    }
                } else {
                    ScrollView([.horizontal, .vertical]) {
                        map(cells, size: size, image: image, cell: unit * CGFloat(graphics.tilemapScale))
                            .padding()
                    }
                }
                if let i = graphics.selectedCell, cells.indices.contains(i) {
                    Divider()
                    CellDetail(cell: cells[i], graphics: graphics).frame(width: 220)
                }
            }
        }
    }

    private func map(
        _ cells: [TilemapCellInfo], size: (columns: Int, rows: Int), image: BitmapInfo?, cell: CGFloat
    ) -> some View {
        ZStack(alignment: .topLeading) {
            if let image {
                PixelImage(bitmap: image, scale: cell * CGFloat(size.columns) / CGFloat(max(image.width, 1)))
            } else {
                numbers(cells, size: size, cell: cell)
            }
            changedCells(cells, size: size, cell: cell)
            grid(size: size, cell: cell)
        }
        .frame(width: CGFloat(size.columns) * cell, height: CGFloat(size.rows) * cell, alignment: .topLeading)
        .contentShape(Rectangle())
        .onTapGesture(coordinateSpace: .local) { p in
            let col = Int(p.x / cell), row = Int(p.y / cell)
            if let i = cells.firstIndex(where: { Int($0.col) == col && Int($0.row) == row }) {
                graphics.selectCell(i)
            }
        }
    }

    private var controls: some View {
        HStack(spacing: 12) {
            if graphics.source == .recording {
                Picker("Layer", selection: $graphics.backgroundLayer) {
                    ForEach(1...4, id: \.self) { Text("BG\($0)").tag(UInt8($0)) }
                }
                .pickerStyle(.segmented)
                .fixedSize()
                if let layer = graphics.currentLayer, let ppu = graphics.ppu {
                    if graphics.isMode7 {
                        Text("Mode 7: the 128×128 plane at $0000, 8 bpp, drawn untransformed (the M7A–M7D rotation and scaling are not applied)")
                            .foregroundStyle(.secondary)
                    } else if let format = layer.format {
                        Text("Mode \(ppu.bgMode): \(format.title), \(layer.size.title), map $\(GraphicsStyle.hex(layer.mapWord, 4)), tiles $\(GraphicsStyle.hex(layer.charWord, 4))\(layer.tile16 ? ", 16×16" : "")")
                            .foregroundStyle(.secondary)
                    } else {
                        Text("Mode \(ppu.bgMode) has no BG\(graphics.backgroundLayer)").foregroundStyle(.secondary)
                    }
                }
            } else {
                Picker("Size", selection: $graphics.screenSize) {
                    ForEach([ScreenSize.s32x32, .s64x32, .s32x64, .s64x64], id: \.self) { Text($0.title).tag($0) }
                }
                .fixedSize()
                Text("Tile numbers; set a tile address on the marked range to see it drawn in the inspector")
                    .foregroundStyle(.secondary)
            }
            Spacer()
            Picker("Size", selection: $graphics.tilemapScale) {
                Text("Fit").tag(0)
                Text("1:1").tag(1)
                ForEach([2, 3, 4], id: \.self) { Text("\($0)×").tag($0) }
            }
            .pickerStyle(.segmented)
            .fixedSize()
            .help("Fit the whole map in the window, or draw each map pixel as 1 to 4 screen pixels")
        }
        .controlSize(.small)
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
    }

    private func numbers(_ cells: [TilemapCellInfo], size: (columns: Int, rows: Int), cell: CGFloat) -> some View {
        Canvas { ctx, _ in
            for c in cells {
                let rect = CGRect(x: CGFloat(c.col) * cell, y: CGFloat(c.row) * cell, width: cell, height: cell)
                let shade = Double(c.palette) / 8
                ctx.fill(Path(rect), with: .color(Color(hue: shade, saturation: 0.25, brightness: 0.95).opacity(0.5)))
                // Too small to read below 12 points a cell.
                if cell >= 12 {
                    ctx.draw(
                        Text(GraphicsStyle.hex(c.tile, 3)).font(.system(size: cell * 0.375).monospaced()),
                        at: CGPoint(x: rect.midX, y: rect.midY)
                    )
                }
            }
        }
        .frame(width: CGFloat(size.columns) * cell, height: CGFloat(size.rows) * cell)
    }

    /// Cells whose entry changed since the previous frame, outlined.
    private func changedCells(_ cells: [TilemapCellInfo], size: (columns: Int, rows: Int), cell: CGFloat) -> some View {
        let changed = cells.filter { c in
            guard let at = graphics.cellVram(c) else { return false }
            return graphics.changed(.vram, offset: Int(at.offset), len: Int(at.len))
        }
        return Canvas { ctx, _ in
            for c in changed {
                let rect = CGRect(x: CGFloat(c.col) * cell, y: CGFloat(c.row) * cell, width: cell, height: cell)
                ctx.stroke(Path(rect.insetBy(dx: 1, dy: 1)), with: .color(.orange), lineWidth: 1.5)
            }
        }
        .frame(width: CGFloat(size.columns) * cell, height: CGFloat(size.rows) * cell)
        .allowsHitTesting(false)
    }

    private func grid(size: (columns: Int, rows: Int), cell: CGFloat) -> some View {
        Canvas { ctx, canvas in
            var path = Path()
            // Lines every cell would hide a map drawn this small.
            let lines = cell >= 12
            for c in 0...size.columns {
                path.move(to: CGPoint(x: CGFloat(c) * cell, y: 0))
                path.addLine(to: CGPoint(x: CGFloat(c) * cell, y: canvas.height))
            }
            for r in 0...size.rows {
                path.move(to: CGPoint(x: 0, y: CGFloat(r) * cell))
                path.addLine(to: CGPoint(x: canvas.width, y: CGFloat(r) * cell))
            }
            if lines {
                ctx.stroke(path, with: .color(.secondary.opacity(0.25)), lineWidth: 0.5)
            }
            if let i = graphics.selectedCell {
                let cols = size.columns
                let rect = CGRect(x: CGFloat(i % cols) * cell, y: CGFloat(i / cols) * cell, width: cell, height: cell)
                ctx.stroke(Path(rect), with: .color(.accentColor), lineWidth: 2)
            }
        }
        .frame(width: CGFloat(size.columns) * cell, height: CGFloat(size.rows) * cell)
        .allowsHitTesting(false)
    }
}

/// One cell's entry, field by field, with a way to its tile.
struct CellDetail: View {
    let cell: TilemapCellInfo
    let graphics: GraphicsModel

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Cell (\(cell.col), \(cell.row))").font(.headline)
            if graphics.isMode7 {
                // A Mode 7 entry is one byte: the tile number and nothing else.
                Grid(alignment: .leading, horizontalSpacing: 10, verticalSpacing: 4) {
                    row("Tile", "$" + GraphicsStyle.hex(cell.tile, 2))
                    row("VRAM", "$" + GraphicsStyle.hex(cell.byteOffset, 4))
                }
                .font(.callout)
                Text("Low byte of word row × 128 + column; the tile's pixels are the high bytes of words 64 × tile onward")
                    .font(.caption)
                    .foregroundStyle(.tertiary)
                    .fixedSize(horizontal: false, vertical: true)
            } else {
                entryDetail
            }
            if let at = graphics.cellVram(cell) {
                ChangeHistoryRow(graphics: graphics, region: .vram, offset: at.offset, len: at.len)
            }
            Spacer()
        }
        .padding()
    }

    private var entryDetail: some View {
        VStack(alignment: .leading, spacing: 8) {
            Grid(alignment: .leading, horizontalSpacing: 10, verticalSpacing: 4) {
                row("Entry", "$" + GraphicsStyle.hex(cell.raw, 4))
                row("Tile", "$" + GraphicsStyle.hex(cell.tile, 3))
                row("Palette", "\(cell.palette)")
                row("Priority", cell.priority ? "high" : "low")
                row("Flips", [cell.hflip ? "h" : nil, cell.vflip ? "v" : nil].compactMap { $0 }.joined(separator: " ").ifEmpty("none"))
                row("Bytes", "+$" + GraphicsStyle.hex(cell.byteOffset, 4))
            }
            .font(.callout)
            Text("vhopppcc cccccccc").font(.caption.monospaced()).foregroundStyle(.tertiary)
            if graphics.source == .recording, let layer = graphics.currentLayer, let format = layer.format {
                Button("Show Tile in Decoder") {
                    graphics.format = format
                    graphics.vramOffset = UInt32(layer.charWord) * 2 + UInt32(cell.tile) * tileByteLen(format: format)
                    graphics.selectTile(0)
                    graphics.revealTile?()
                }
                .controlSize(.small)
            }
        }
    }

    @ViewBuilder
    private func row(_ label: String, _ value: String) -> some View {
        GridRow {
            Text(label).foregroundStyle(.secondary)
            Text(value).monospaced()
        }
    }
}

private extension String {
    func ifEmpty(_ fallback: String) -> String { isEmpty ? fallback : self }
}
