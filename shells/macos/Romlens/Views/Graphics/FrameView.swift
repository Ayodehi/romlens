import RomlensKit
import SwiftUI

/// The screen at a frame of a recording, drawn from the PPU state
/// (checklist 3.12): hovering names what drew the pixel, clicking keeps it
/// and offers the views that show where it came from.
struct FrameView: View {
    let model: RomViewModel
    @Bindable var graphics: GraphicsModel

    var body: some View {
        VStack(spacing: 0) {
            controls
            Divider()
            HStack(alignment: .top, spacing: 0) {
                ScrollView([.horizontal, .vertical]) {
                    if let f = graphics.frameImage() {
                        screen(f)
                            .padding()
                    }
                }
                if let p = graphics.selectedPixel, let w = graphics.pixel(x: p.x, y: p.y) {
                    Divider()
                    PixelDetail(model: model, graphics: graphics, at: p, winner: w)
                        .frame(width: 260)
                }
            }
            Divider()
            status
        }
    }

    private func screen(_ f: FrameImageInfo) -> some View {
        let scale = CGFloat(graphics.frameScale)
        return ZStack(alignment: .topLeading) {
            PixelImage(bitmap: f.image, scale: scale)
            if let p = graphics.selectedPixel {
                Rectangle()
                    .strokeBorder(Color.accentColor, lineWidth: 2)
                    .frame(width: scale + 4, height: scale + 4)
                    .offset(x: CGFloat(p.x) * scale - 2, y: CGFloat(p.y) * scale - 2)
                    .allowsHitTesting(false)
            }
        }
        .contentShape(Rectangle())
        .onContinuousHover { phase in
            switch phase {
            case .active(let at):
                graphics.hoverPixel = (Int(at.x / scale), Int(at.y / scale))
            case .ended:
                graphics.hoverPixel = nil
            }
        }
        .onTapGesture(coordinateSpace: .local) { at in
            let x = Int(at.x / scale), y = Int(at.y / scale)
            guard x < Int(f.image.width), y < Int(f.image.height) else { return }
            graphics.selectedPixel = (x, y)
        }
        .accessibilityLabel("Frame \(graphics.frame), drawn from the PPU state")
    }

    private var controls: some View {
        HStack(spacing: 12) {
            Picker("Zoom", selection: $graphics.frameScale) {
                ForEach([1, 2, 3, 4], id: \.self) { Text("\($0)×").tag($0) }
            }
            .pickerStyle(.segmented)
            .fixedSize()
            if let f = graphics.frameImage() {
                Text("Mode \(f.bgMode), drawn from the PPU state \(f.perLine ? "line by line" : "as the frame ended")")
                    .foregroundStyle(.secondary)
                    .help(f.perLine
                        ? "The recording logged every register and memory write while the frame was drawn, so each line has the registers it had"
                        : "This recording has no line-by-line writes: every line is drawn with the registers as the frame ended, so a split screen (a status bar, a gradient) shows the bottom's settings throughout. Record with the current recorder script to get them.")
                if !f.unsupported.isEmpty {
                    Text("not drawn: \(f.unsupported.joined(separator: ", "))")
                        .foregroundStyle(.orange)
                        .help("The game turns these on here; the drawing leaves them out, so these pixels can differ from the real screen")
                }
            }
            Spacer()
        }
        .controlSize(.small)
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
    }

    private var status: some View {
        HStack {
            if let p = graphics.hoverPixel, let w = graphics.pixel(x: p.x, y: p.y) {
                Text("(\(p.x), \(p.y)): \(w.summary)")
            } else {
                Text("Point at a pixel to see what drew it; click to keep it").foregroundStyle(.secondary)
            }
            Spacer()
        }
        .font(.callout.monospaced())
        .lineLimit(1)
        .truncationMode(.tail)
        .padding(.horizontal, 12)
        .padding(.vertical, 5)
    }
}

/// The clicked pixel: what drew it, and buttons to the views that show it.
struct PixelDetail: View {
    let model: RomViewModel
    let graphics: GraphicsModel
    let at: (x: Int, y: Int)
    let winner: PixelWinnerInfo

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 10) {
                Text("Pixel (\(at.x), \(at.y))").font(.headline)
                Text(winner.summary)
                    .font(.callout)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
                VStack(alignment: .leading, spacing: 6) {
                    switch winner {
                    case .sprite:
                        button("Show Its OAM Entry", .sprite, "list.bullet.rectangle")
                        button("Show Its Tile", .tile, "square.grid.3x3")
                    case .background:
                        button("Show Its Tilemap Cell", .cell, "map")
                        button("Show Its Tile", .tile, "square.grid.3x3")
                    default:
                        EmptyView()
                    }
                    if winner.colour != nil {
                        button("Show Its Colour", .colour, "paintpalette")
                    }
                }
                .buttonStyle(.link)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(12)
        }
    }

    private func button(_ title: String, _ what: GraphicsModel.Reveal, _ image: String) -> some View {
        Button {
            if let tab = graphics.reveal(what, of: winner) { model.graphicsTab = tab }
        } label: {
            Label(title, systemImage: image)
        }
    }
}
