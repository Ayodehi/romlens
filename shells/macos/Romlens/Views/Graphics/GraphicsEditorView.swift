import RomlensKit
import SwiftUI

/// The editor area when a graphics view is open: a header naming what the
/// views read, then the view.
struct GraphicsEditorView: View {
    @Bindable var model: RomViewModel

    var body: some View {
        VStack(spacing: 0) {
            GraphicsSourceBar(model: model, graphics: model.graphics)
            Divider()
            switch model.graphicsTab {
            case .tiles: TileDecoderView(graphics: model.graphics)
            case .palette: PaletteView(graphics: model.graphics)
            case .oam: OamTableView(graphics: model.graphics)
            case .tilemap: TilemapView(model: model, graphics: model.graphics)
            case nil: EmptyView()
            }
        }
    }
}

/// Where the bytes come from: ROM at an offset, the attached recording at a
/// frame, or a decompressed block.
struct GraphicsSourceBar: View {
    let model: RomViewModel
    @Bindable var graphics: GraphicsModel

    var body: some View {
        HStack(spacing: 12) {
            Picker("Source", selection: sourceBinding) {
                Text("ROM").tag(SourceTag.rom)
                if graphics.hasRecording {
                    Text("Recording").tag(SourceTag.recording)
                }
                if case .bytes(let label, _) = graphics.source {
                    Text(label).tag(SourceTag.bytes)
                }
            }
            .pickerStyle(.segmented)
            .fixedSize()
            .labelsHidden()
            Text(graphics.sourceDescription)
                .font(.callout.monospaced())
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
            if let note = graphics.screenNote {
                Text(note)
                    .font(.callout)
                    .foregroundStyle(.orange)
                    .help("INIDISP ($2100) at this frame: what the views show was not what the screen showed")
            }
            Spacer()
            switch graphics.source {
            case .rom:
                Button("Read From Selection") {
                    if let range = model.highlightedRange { graphics.romOffset = range.lowerBound }
                }
                .disabled(model.selectedOffset == nil)
                .help("Decode the bytes at the editor's selection")
            case .recording:
                FrameStepper(graphics: graphics)
            case .bytes:
                EmptyView()
            }
        }
        .controlSize(.small)
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
    }

    private enum SourceTag: Hashable { case rom, recording, bytes }

    private var sourceBinding: Binding<SourceTag> {
        Binding(
            get: {
                switch graphics.source {
                case .rom: .rom
                case .recording: .recording
                case .bytes: .bytes
                }
            },
            set: { tag in
                switch tag {
                case .rom: graphics.source = .rom
                case .recording: if graphics.hasRecording { graphics.source = .recording }
                case .bytes: break
                }
            }
        )
    }
}

/// A frame field with previous and next: Phase 2 has no scrubber.
struct FrameStepper: View {
    @Bindable var graphics: GraphicsModel

    var body: some View {
        HStack(spacing: 4) {
            Button { graphics.step(by: -1) } label: { Image(systemName: "chevron.left") }
                .disabled(graphics.frame <= graphics.firstFrame)
                .help("Previous frame")
            TextField("Frame", value: $graphics.frame, format: .number.grouping(.never))
                .frame(width: 64)
                .multilineTextAlignment(.trailing)
            Button { graphics.step(by: 1) } label: { Image(systemName: "chevron.right") }
                .disabled(graphics.frame + 1 >= graphics.frameCount)
                .help("Next frame")
            if graphics.isLive {
                Toggle(isOn: $graphics.followLive) {
                    Image(systemName: "dot.radiowaves.left.and.right")
                }
                .toggleStyle(.button)
                .help(graphics.followLive
                    ? "Following the game: each frame shows as it arrives"
                    : "Paused on this frame; click to follow the game again")
            }
        }
    }
}

/// The mark on an entry that changed since the previous frame.
struct ChangeDot: View {
    var body: some View {
        Circle()
            .fill(Color.orange)
            .frame(width: 6, height: 6)
            .help("Changed since the previous frame")
    }
}

/// "Changed at frame N · Next at M", each a button that goes there: when the
/// selected bytes last changed at or before this frame, and next change.
struct ChangeHistoryRow: View {
    let graphics: GraphicsModel
    let region: StateRegion
    let offset: UInt32
    let len: UInt32

    var body: some View {
        if let h = graphics.history(region, offset: offset, len: len) {
            if !h.indexed {
                Text("Kept in keyframes only, so not frame by frame")
                    .font(.caption).foregroundStyle(.secondary)
            } else {
                HStack(spacing: 8) {
                    if let last = h.last {
                        Button(last == 0 ? "Unchanged since frame 0" : "Changed at frame \(last)") {
                            graphics.frame = last
                        }
                        .disabled(last == graphics.frame)
                    }
                    if let next = h.next {
                        Button("Next at frame \(next)") { graphics.frame = next }
                    } else {
                        Text("No later change").foregroundStyle(.secondary)
                    }
                }
                .buttonStyle(.link)
                .font(.caption)
            }
        }
    }
}

/// Shared drawing helpers.
enum GraphicsStyle {
    static func colour(_ rgb: UInt32) -> Color {
        Color(
            .sRGB,
            red: Double(rgb >> 16 & 0xFF) / 255,
            green: Double(rgb >> 8 & 0xFF) / 255,
            blue: Double(rgb & 0xFF) / 255
        )
    }

    static func hex(_ v: some BinaryInteger, _ digits: Int) -> String {
        let s = String(v, radix: 16, uppercase: true)
        return String(repeating: "0", count: max(0, digits - s.count)) + s
    }
}

/// A core bitmap drawn with square pixels at an integer scale.
struct PixelImage: View {
    let bitmap: BitmapInfo
    let scale: CGFloat

    var body: some View {
        if let image = bitmap.cgImage {
            Image(decorative: image, scale: 1)
                .interpolation(.none)
                .resizable()
                .frame(width: CGFloat(bitmap.width) * scale, height: CGFloat(bitmap.height) * scale)
                .background(CheckerBackground())
        }
    }
}

/// What transparent looks like.
struct CheckerBackground: View {
    var body: some View {
        Canvas { ctx, size in
            let s: CGFloat = 8
            for y in stride(from: 0, to: size.height, by: s) {
                for x in stride(from: 0, to: size.width, by: s) where (Int(x / s) + Int(y / s)).isMultiple(of: 2) {
                    ctx.fill(Path(CGRect(x: x, y: y, width: s, height: s)), with: .color(.gray.opacity(0.18)))
                }
            }
        }
    }
}
