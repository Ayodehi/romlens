import RomlensKit
import SwiftUI

/// The sprite table (checklist 2.23): 128 entries decoded from the low
/// table and the high table's two bits.
struct OamTableView: View {
    @Bindable var graphics: GraphicsModel

    struct Row: Identifiable {
        let entry: OamEntryInfo
        var id: UInt8 { entry.index }
    }

    var body: some View {
        let rows = graphics.sprites().map(Row.init)
        VStack(spacing: 0) {
            controls
            Divider()
            HStack(spacing: 0) {
                Table(rows, selection: selection) {
                    TableColumn("#") { Text("\($0.entry.index)").monospacedDigit() }.width(32)
                    TableColumn("X") { Text("\($0.entry.x)").monospacedDigit() }.width(40)
                    TableColumn("Y") { Text("\($0.entry.y)").monospacedDigit() }.width(40)
                    TableColumn("Tile") { Text("$" + GraphicsStyle.hex($0.entry.tile, 3)).monospaced() }.width(48)
                    TableColumn("Pal") { Text("\($0.entry.palette)").monospacedDigit() }.width(32)
                    TableColumn("Pri") { Text("\($0.entry.priority)").monospacedDigit() }.width(32)
                    TableColumn("Flip") { Text(flip($0.entry)).monospaced() }.width(36)
                    TableColumn("Size") { Text("\($0.entry.width)×\($0.entry.height)").monospacedDigit() }.width(52)
                    TableColumn("Table") { Text("\($0.entry.nameTable)").monospacedDigit() }.width(40)
                    TableColumn("VRAM") { Text("$" + GraphicsStyle.hex($0.entry.tileWord, 4)).monospaced() }.width(52)
                }
                if let i = graphics.selectedSprite, let e = graphics.sprites().first(where: { $0.index == i })
                    ?? oamEntries(bytes: graphics.oamBytes(), obsel: graphics.obsel, sort: .table).first(where: { $0.index == i }) {
                    Divider()
                    SpriteDetail(entry: e, graphics: graphics)
                        .frame(width: 220)
                }
            }
        }
    }

    private var selection: Binding<UInt8?> {
        Binding(get: { graphics.selectedSprite }, set: { graphics.selectSprite($0) })
    }

    private var controls: some View {
        HStack(spacing: 12) {
            Picker("Order", selection: $graphics.oamSort) {
                ForEach(GraphicsModel.OamSortChoice.allCases) { Text($0.title).tag($0) }
            }
            .fixedSize()
            Toggle("On screen only", isOn: $graphics.visibleSpritesOnly)
            if graphics.source == .recording {
                Text("OBSEL $" + GraphicsStyle.hex(graphics.obsel, 2) + " from the recording")
                    .foregroundStyle(.secondary)
            } else {
                HStack(spacing: 4) {
                    Text("OBSEL $")
                    TextField("00", value: $graphics.obsel, format: .number.precision(.integerLength(1...3)))
                        .frame(width: 44)
                }
                .help("OBSEL ($2101) sizes the sprites and places their tiles; it is not in OAM")
            }
            Spacer()
        }
        .controlSize(.small)
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
    }

    private func flip(_ e: OamEntryInfo) -> String {
        switch (e.hflip, e.vflip) {
        case (false, false): "–"
        case (true, false): "h"
        case (false, true): "v"
        case (true, true): "hv"
        }
    }
}

/// One sprite's bytes and, with a recording, its picture.
struct SpriteDetail: View {
    let entry: OamEntryInfo
    let graphics: GraphicsModel

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Sprite \(entry.index)").font(.headline)
            if let image = graphics.spriteImage(entry.index) {
                PixelImage(bitmap: image, scale: max(1, 96 / CGFloat(max(image.width, 1))))
            }
            Grid(alignment: .leading, horizontalSpacing: 10, verticalSpacing: 4) {
                GridRow { Text("Low table").foregroundStyle(.secondary); Text("bytes $" + GraphicsStyle.hex(entry.lowOffset, 3) + "–$" + GraphicsStyle.hex(entry.lowOffset + 3, 3)).monospaced() }
                GridRow { Text("High table").foregroundStyle(.secondary); Text("byte $" + GraphicsStyle.hex(entry.highOffset, 3) + ", bits \(Int(entry.index % 4) * 2)–\(Int(entry.index % 4) * 2 + 1)").monospaced() }
                GridRow { Text("Palette").foregroundStyle(.secondary); Text("OBJ \(entry.palette), CGRAM \(128 + Int(entry.palette) * 16)–\(143 + Int(entry.palette) * 16)").monospaced() }
            }
            .font(.callout)
            Text("Byte 3 is vhoopppn: flips, priority, palette, and the tile's ninth bit.")
                .font(.caption).foregroundStyle(.tertiary)
            Spacer()
        }
        .padding()
    }
}
