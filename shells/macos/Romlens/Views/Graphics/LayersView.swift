import RomlensKit
import SwiftUI

/// The frame's layers apart (checklist 3.13): each background the mode has
/// and the sprites, alone where they show on the main screen, with the
/// mode's order front to back.
struct LayersView: View {
    @Bindable var graphics: GraphicsModel

    var body: some View {
        HStack(alignment: .top, spacing: 0) {
            ScrollView {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 256 * 1.5 + 24), alignment: .top)], spacing: 16) {
                    ForEach(layers, id: \.id) { layer in
                        VStack(alignment: .leading, spacing: 4) {
                            Text(layer.title).font(.headline)
                            Text(layer.detail).font(.caption).foregroundStyle(.secondary)
                            if let image = graphics.frameLayer(layer.id) {
                                PixelImage(bitmap: image, scale: 1.5)
                            }
                        }
                    }
                }
                .padding()
            }
            Divider()
            legend.frame(width: 220)
        }
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
