import RomlensKit
import SwiftUI

/// The frame's layers apart (checklist 3.13): each background the mode has
/// and the sprites, alone where they show on the main screen, in a grid two
/// wide (three when a mode has four backgrounds), with the mode's order
/// front to back.
struct LayersView: View {
    @Bindable var graphics: GraphicsModel

    private static let spacing: CGFloat = 16
    private static let header: CGFloat = 22

    var body: some View {
        HStack(alignment: .top, spacing: 0) {
            VStack(spacing: 0) {
                controls
                Divider()
                panels
            }
            Divider()
            legend.frame(width: 220)
        }
    }

    private var controls: some View {
        HStack {
            Picker("Size", selection: $graphics.layersFit) {
                Text("Fit").tag(true)
                Text("1:1").tag(false)
            }
            .pickerStyle(.segmented)
            .fixedSize()
            Spacer()
        }
        .padding(.horizontal)
        .padding(.vertical, 8)
    }

    @ViewBuilder private var panels: some View {
        let shown = layers.map { ($0, graphics.frameLayer($0.id)) }
        let columns = shown.count > 4 ? 3 : 2
        let rows = (shown.count + columns - 1) / columns
        let size = shown.lazy.compactMap(\.1).first.map { CGSize(width: Int($0.width), height: Int($0.height)) }
            ?? CGSize(width: 256, height: 224)
        if graphics.layersFit {
            GeometryReader { geo in
                let s = Self.spacing
                let across = (geo.size.width - s * CGFloat(columns + 1)) / CGFloat(columns) / size.width
                let down = (geo.size.height - s * CGFloat(rows + 1) - Self.header * CGFloat(rows))
                    / CGFloat(rows) / size.height
                grid(shown, columns: columns, scale: max(0.25, min(across, down)))
                    .frame(width: geo.size.width, height: geo.size.height)
            }
        } else {
            ScrollView([.horizontal, .vertical]) {
                grid(shown, columns: columns, scale: 1)
            }
        }
    }

    private func grid(_ shown: [(Layer, BitmapInfo?)], columns: Int, scale: CGFloat) -> some View {
        Grid(alignment: .topLeading, horizontalSpacing: Self.spacing, verticalSpacing: Self.spacing) {
            ForEach(Array(stride(from: 0, to: shown.count, by: columns)), id: \.self) { start in
                GridRow {
                    ForEach(shown[start..<min(start + columns, shown.count)], id: \.0.id) { layer, image in
                        VStack(alignment: .leading, spacing: 4) {
                            HStack(alignment: .firstTextBaseline, spacing: 6) {
                                Text(layer.title).font(.headline)
                                Text(layer.detail).font(.caption).foregroundStyle(.secondary)
                            }
                            .lineLimit(1)
                            .frame(height: Self.header - 4, alignment: .leading)
                            if let image {
                                PixelImage(bitmap: image, scale: scale)
                            }
                        }
                    }
                }
            }
        }
        .padding(Self.spacing)
    }

    private struct Layer {
        let id: UInt8
        let title: String
        let detail: String
    }

    private var layers: [Layer] {
        guard let ppu = graphics.ppu else { return [] }
        var out: [Layer] = ppu.layers.compactMap { l in
            guard let format = l.format else { return nil }
            let on = ppu.mainScreen & (1 << (l.bg - 1)) != 0
            return Layer(
                id: l.bg,
                title: "BG\(l.bg)",
                detail: "\(format.title)\(on ? "" : ", off on the main screen as the frame ended")"
            )
        }
        out.append(Layer(
            id: 5,
            title: "Sprites",
            detail: "4 bpp\(ppu.mainScreen & 0x10 != 0 ? "" : ", off on the main screen as the frame ended")"
        ))
        return out
    }

    private var legend: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Front to back").font(.headline)
            if let ppu = graphics.ppu {
                Text("Mode \(ppu.bgMode) puts the layers in this order; each pixel shows the first that draws there.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            ForEach(Array(graphics.priorityOrder().enumerated()), id: \.offset) { i, name in
                Text("\(i + 1). \(name)").font(.callout)
            }
            Text("then the backdrop, CGRAM colour 0")
                .font(.callout)
                .foregroundStyle(.secondary)
            Spacer()
        }
        .padding(12)
    }
}
