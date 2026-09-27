import RomlensKit
import SwiftUI

extension SampleInfo: @retroactive Identifiable {
    public var id: UInt8 { index }
}

extension AramRegionInfo: @retroactive Identifiable {
    public var id: UInt16 { start }
}

/// Audio RAM's 64 KB coloured by what each byte is (docs/23), its parts
/// listed, and the part selected: the SPC700's code listed and explained,
/// or its bytes, with the upload blocks that filled it from the ROM.
struct AramView: View {
    @Bindable var audio: AudioModel
    @State private var hover: UInt16?

    var body: some View {
        let map = audio.state?.map ?? []
        HSplitView {
            VStack(alignment: .leading, spacing: 8) {
                AramMapImage(map: map, selected: audio.selectedPart, pc: audio.state?.pc, hover: $hover) { at in
                    audio.selectedPart = audio.part(containing: at)?.start
                }
                .aspectRatio(1, contentMode: .fit)
                .frame(maxWidth: 512)
                Text(hoverText(map))
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .frame(height: 14)
                AramLegend(map: map)
            }
            .padding(12)
            .frame(minWidth: 280, idealWidth: 400, maxWidth: 536, maxHeight: .infinity, alignment: .top)
            VSplitView {
                Table(map, selection: $audio.selectedPart) {
                    TableColumn("Range") { p in
                        Text("\(AudioStyle.hex(p.start, 4))–\(AudioStyle.hex(UInt32(p.start) + p.len - 1, 4))").monospaced()
                    }
                    .width(96)
                    TableColumn("Bytes") { p in Text("\(p.len)").monospacedDigit() }
                        .width(52)
                    TableColumn("What") { p in
                        HStack(spacing: 4) {
                            Circle().fill(AudioStyle.colour(p.kind)).frame(width: 8, height: 8)
                            Text(p.label).lineLimit(1)
                        }
                        .help(p.label)
                    }
                }
                .frame(minHeight: 160)
                AramPartDetail(audio: audio, part: map.first { $0.start == audio.selectedPart })
                    .frame(minHeight: 200)
            }
            .frame(minWidth: 360, maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private func hoverText(_ map: [AramRegionInfo]) -> String {
        guard let at = hover, let p = audio.part(containing: at) else { return "" }
        return "\(AudioStyle.hex(at, 4)): \(p.label)"
    }
}

/// A byte a pixel, 256 to a row, two screen pixels a byte.
struct AramMapImage: View {
    let map: [AramRegionInfo]
    let selected: UInt16?
    let pc: UInt16?
    @Binding var hover: UInt16?
    let pick: (UInt16) -> Void

    var body: some View {
        GeometryReader { g in
            let scale = g.size.width / 256
            ZStack(alignment: .topLeading) {
                if let image = Self.image(map) {
                    Image(decorative: image, scale: 1)
                        .interpolation(.none)
                        .resizable()
                }
                if let s = selected, let p = map.first(where: { $0.start == s }) {
                    Self.outline(start: UInt32(p.start), len: p.len, scale: scale)
                        .stroke(Color.white, lineWidth: 1.5)
                }
                if let pc {
                    Circle().stroke(Color.white, lineWidth: 1.5)
                        .frame(width: 8, height: 8)
                        .offset(x: CGFloat(pc & 0xFF) * scale - 4 + scale / 2, y: CGFloat(pc >> 8) * scale - 4 + scale / 2)
                        .help("The SPC700's program counter, \(AudioStyle.hex(pc, 4))")
                }
            }
            .contentShape(Rectangle())
            .onContinuousHover { phase in
                switch phase {
                case .active(let at): hover = Self.address(at, scale)
                case .ended: hover = nil
                }
            }
            .onTapGesture { at in
                if let a = Self.address(at, scale) { pick(a) }
            }
        }
        .help("Audio RAM a byte a pixel, 256 bytes a row: click a part to open it")
    }

    static func address(_ at: CGPoint, _ scale: CGFloat) -> UInt16? {
        let x = Int(at.x / scale), y = Int(at.y / scale)
        guard (0..<256).contains(x), (0..<256).contains(y) else { return nil }
        return UInt16(y * 256 + x)
    }

    /// The rows a part covers, as one path around them.
    static func outline(start: UInt32, len: UInt32, scale: CGFloat) -> Path {
        var p = Path()
        let end = start + max(len, 1)
        var at = start
        while at < end {
            let row = at / 256
            let rowEnd = min(end, (row + 1) * 256)
            p.addRect(CGRect(x: CGFloat(at % 256) * scale, y: CGFloat(row) * scale,
                             width: CGFloat(rowEnd - at) * scale, height: scale))
            at = rowEnd
        }
        return p
    }

    @MainActor private static var cache: (key: [UInt64], image: CGImage)?

    static func image(_ map: [AramRegionInfo]) -> CGImage? {
        let key = map.map { UInt64($0.start) << 40 | UInt64($0.len) << 8 | UInt64($0.kind.hashValue & 0xFF) }
        if let c = cache, c.key == key { return c.image }
        var rgba = [UInt8](repeating: 0, count: 256 * 256 * 4)
        for p in map {
            let (r, g, b) = AudioStyle.rgb(p.kind)
            // Alternate parts of one kind are shaded apart so a run of
            // samples reads as samples, not as one block.
            let shade: Double = (p.kind == .sample && (p.sample ?? 0) % 2 == 1) ? 0.8 : 1
            for i in UInt32(p.start)..<min(UInt32(p.start) + p.len, 0x10000) {
                let o = Int(i) * 4
                rgba[o] = UInt8(Double(r) * shade)
                rgba[o + 1] = UInt8(Double(g) * shade)
                rgba[o + 2] = UInt8(Double(b) * shade)
                rgba[o + 3] = 255
            }
        }
        let bitmap = BitmapInfo(width: 256, height: 256, rgba: Data(rgba))
        guard let image = bitmap.cgImage else { return nil }
        cache = (key, image)
        return image
    }
}

struct AramLegend: View {
    let map: [AramRegionInfo]

    var body: some View {
        let kinds = Dictionary(grouping: map, by: \.kind)
        let order: [AramKindInfo] = [.directPage, .io, .stack, .code, .driverData, .directory, .sample, .dspData, .echo, .boot, .other]
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 150), alignment: .leading)], alignment: .leading, spacing: 4) {
            ForEach(order.filter { kinds[$0] != nil }, id: \.self) { k in
                let parts = kinds[k] ?? []
                HStack(spacing: 4) {
                    RoundedRectangle(cornerRadius: 2).fill(AudioStyle.colour(k)).frame(width: 10, height: 10)
                    Text(parts[0].kindName).font(.caption)
                    Text("\(parts.reduce(0) { $0 + $1.len }) B").font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                }
            }
        }
    }
}

/// The part selected: its listing or its bytes, and where it came from.
struct AramPartDetail: View {
    @Bindable var audio: AudioModel
    let part: AramRegionInfo?
    @State private var selectedLine: UInt16?

    var body: some View {
        if let part {
            VStack(alignment: .leading, spacing: 6) {
                HStack {
                    Text(part.label).font(.headline).lineLimit(2)
                    Spacer()
                }
                let blocks = audio.blocks(filling: part)
                if !blocks.isEmpty {
                    ForEach(Array(blocks.enumerated()), id: \.offset) { _, b in
                        HStack(spacing: 6) {
                            Text("sent by the upload at \(formatSnesAddress(address: b.upload.list)): ROM \(formatFileOffset(offset: b.block.romOffset)), \(b.block.len) bytes to \(AudioStyle.hex(b.block.aram, 4))")
                                .font(.caption)
                            Button("Show in ROM") {
                                let into = UInt32(max(Int(part.start) - Int(b.block.aram), 0))
                                audio.showInRom?(b.block.romOffset + into)
                            }
                            .buttonStyle(.link)
                            .font(.caption)
                        }
                    }
                }
                Divider()
                if part.kind == .code || part.kind == .boot {
                    listing(part)
                } else {
                    hex(part)
                }
            }
            .padding(8)
        } else {
            ContentUnavailableView {
                Label("Choose a Part", systemImage: "memorychip")
            } description: {
                Text("Audio RAM is the sound CPU's only memory: its driver's code and data, the sample directory, the samples, and the echo buffer all share these 64 KB.")
            }
        }
    }

    private func listing(_ part: AramRegionInfo) -> some View {
        let lines = audio.listing(from: part.start, count: min(max(part.len / 2, 16), 600))
            .filter { UInt32($0.address) < UInt32(part.start) + part.len }
        return HSplitView {
            List(selection: $selectedLine) {
                ForEach(lines, id: \.address) { l in
                    SpcLineRow(line: l, pc: audio.state?.pc)
                        .tag(l.address)
                }
            }
            .listStyle(.plain)
            .font(.caption.monospaced())
            .frame(minWidth: 300)
            if let at = selectedLine, let l = lines.first(where: { $0.address == at }) {
                ScrollView {
                    VStack(alignment: .leading, spacing: 8) {
                        Text(l.text).font(.callout.monospaced().weight(.medium))
                        if let i = l.idiom {
                            Text(i.title).font(.callout.weight(.medium))
                            Text(i.summary).font(.caption)
                            Text(i.why).font(.caption).foregroundStyle(.secondary)
                        }
                        if !l.write.isEmpty {
                            Text(l.writeToDsp ? "Writes the DSP" : "Writes an I/O register").font(.caption).foregroundStyle(.secondary)
                            RegisterPartsView(parts: l.write)
                        } else if let c = l.comment {
                            Text(c).font(.caption)
                        }
                        if !l.code {
                            Text("Not reached from the program counter or the execution log: these bytes may be data read as code.")
                                .font(.caption).foregroundStyle(.orange)
                        }
                    }
                    .padding(8)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(minWidth: 200, idealWidth: 260)
            }
        }
    }

    private func hex(_ part: AramRegionInfo) -> some View {
        let len = min(part.len, 4096)
        let bytes = [UInt8](audio.aram(start: part.start, len: len))
        let rows = (bytes.count + 15) / 16
        return ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                ForEach(0..<rows, id: \.self) { r in
                    let slice = bytes[(r * 16)..<min(r * 16 + 16, bytes.count)]
                    Text("\(AudioStyle.hex(UInt32(part.start) + UInt32(r * 16), 4))  " + slice.map { GraphicsStyle.hex($0, 2) }.joined(separator: " "))
                        .font(.caption.monospaced())
                }
                if part.len > len {
                    Text("… \(part.len - len) more bytes").font(.caption).foregroundStyle(.secondary)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .textSelection(.enabled)
        }
    }
}

struct SpcLineRow: View {
    let line: SpcLineInfo
    let pc: UInt16?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let label = line.label {
                Text("\(label):").foregroundStyle(.blue)
            }
            if let i = line.idiom {
                Text("; ▸ \(i.title): \(i.summary)").foregroundStyle(.secondary).lineLimit(1)
            }
            HStack(spacing: 8) {
                Text(pc == line.address ? "▶" : " ").foregroundStyle(.green)
                Text(AudioStyle.hex(line.address, 4)).foregroundStyle(.secondary)
                Text(line.bytes.map { GraphicsStyle.hex($0, 2) }.joined(separator: " "))
                    .foregroundStyle(.secondary)
                    .frame(width: 76, alignment: .leading)
                Text(line.text).foregroundStyle(line.code ? .primary : .secondary)
                    .lineLimit(1)
                    .fixedSize()
                if let c = line.comment {
                    Text("; \(c)").foregroundStyle(.green).lineLimit(1)
                }
            }
        }
    }
}
