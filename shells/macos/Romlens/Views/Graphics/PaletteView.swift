import RomlensKit
import SwiftUI

/// CGRAM as sixteen rows of sixteen (checklist 2.22). Clicking a swatch
/// shows its bit fields and, reading ROM, selects its two bytes.
struct PaletteView: View {
    @Bindable var graphics: GraphicsModel
    static let swatch: CGFloat = 22

    var body: some View {
        let entries = graphics.colours()
        let changed = graphics.change(.cgram)?.bytes ?? IndexSet()
        HStack(alignment: .top, spacing: 24) {
            VStack(alignment: .leading, spacing: 2) {
                ForEach(0..<16, id: \.self) { row in
                    HStack(spacing: 2) {
                        Text(label(row))
                            .font(.caption2.monospaced())
                            .foregroundStyle(.secondary)
                            .frame(width: 44, alignment: .trailing)
                        ForEach(0..<16, id: \.self) { col in
                            let i = row * 16 + col
                            swatch(
                                entries.indices.contains(i) ? entries[i] : nil,
                                index: i,
                                changed: changed.contains(i * 2) || changed.contains(i * 2 + 1)
                            )
                        }
                    }
                }
            }
            if let i = graphics.selectedColour, entries.indices.contains(i) {
                VStack(alignment: .leading, spacing: 8) {
                    ColourDetail(entry: entries[i])
                    ChangeHistoryRow(graphics: graphics, region: .cgram, offset: UInt32(i * 2), len: 2)
                }
            } else {
                Text("Click a swatch").foregroundStyle(.tertiary)
            }
            Spacer()
        }
        .padding()
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    private func label(_ row: Int) -> String {
        row < 8 ? "BG \(row)" : "OBJ \(row - 8)"
    }

    @ViewBuilder
    private func swatch(_ entry: PaletteEntryInfo?, index: Int, changed: Bool) -> some View {
        let selected = graphics.selectedColour == index
        Rectangle()
            .fill(entry.map { GraphicsStyle.colour($0.rgb) } ?? .clear)
            .frame(width: Self.swatch, height: Self.swatch)
            .overlay(Rectangle().stroke(selected ? Color.accentColor : .secondary.opacity(0.3), lineWidth: selected ? 2 : 0.5))
            .overlay {
                if entry?.unusedBit == true {
                    Image(systemName: "exclamationmark").font(.caption2).foregroundStyle(.red)
                }
            }
            .overlay(alignment: .topTrailing) {
                if changed { ChangeDot().offset(x: 2, y: -2) }
            }
            .onTapGesture { graphics.selectColour(index) }
            .help(entry.map { "\(index): $\(GraphicsStyle.hex($0.raw, 4))" } ?? "")
    }
}

/// One entry decoded: the raw word, the three five-bit fields, and the
/// eight-bit colour they expand to.
struct ColourDetail: View {
    let entry: PaletteEntryInfo

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Rectangle().fill(GraphicsStyle.colour(entry.rgb)).frame(width: 64, height: 64)
                .border(Color.secondary.opacity(0.4))
            Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 4) {
                row("Entry", "\(entry.index) (row \(entry.index / 16), column \(entry.index % 16))")
                row("Raw", "$" + GraphicsStyle.hex(entry.raw, 4))
                row("Bits", bits)
                row("Blue", "\(entry.blue5) of 31")
                row("Green", "\(entry.green5) of 31")
                row("Red", "\(entry.red5) of 31")
                row("RGB", "#" + GraphicsStyle.hex(entry.rgb, 6))
                if entry.unusedBit {
                    row("Bit 15", "set: the PPU ignores it, so real palettes rarely have it")
                }
            }
            .font(.callout)
            Text("0bbbbbgggggrrrrr; each field expands as (c << 3) | (c >> 2).")
                .font(.caption).foregroundStyle(.tertiary)
        }
    }

    private var bits: String {
        let s = String(entry.raw, radix: 2)
        let b = String(repeating: "0", count: 16 - s.count) + s
        let chars = Array(b)
        return "\(chars[0]) \(String(chars[1...5])) \(String(chars[6...10])) \(String(chars[11...15]))"
    }

    @ViewBuilder
    private func row(_ label: String, _ value: String) -> some View {
        GridRow {
            Text(label).foregroundStyle(.secondary)
            Text(value).monospaced().textSelection(.enabled)
        }
    }
}
