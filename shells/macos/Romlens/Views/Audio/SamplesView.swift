import RomlensKit
import SwiftUI

/// The sample directory, and one sample opened (docs/23): its waveform
/// with the loop point, its BRR blocks, and a block's sixteen values
/// decoded step by step with the filter's formula in numbers.
struct SamplesView: View {
    @Bindable var audio: AudioModel

    var body: some View {
        let samples = audio.state?.samples ?? []
        HSplitView {
            directory(samples)
                .frame(minWidth: 300, idealWidth: 380, maxWidth: 520, maxHeight: .infinity)
            detail(samples)
                .frame(minWidth: 380, maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private func directory(_ samples: [SampleInfo]) -> some View {
        Table(samples, selection: selection) {
            TableColumn("#") { s in Text("\(s.index)").monospacedDigit() }
                .width(28)
            TableColumn("Start") { s in Text(AudioStyle.hex(s.start, 4)).monospaced() }
                .width(48)
            TableColumn("Loop") { s in Text(s.loops ? AudioStyle.hex(s.loopAt, 4) : "–").monospaced() }
                .width(48)
            TableColumn("Bytes") { s in Text("\(s.blocks * 9)").monospacedDigit() }
                .width(52)
            TableColumn("At $1000") { s in
                Text(s.tuningNote.map { "≈ \($0)" } ?? "–")
                    .help(s.tuningHz.map { String(format: "%.0f Hz at pitch $1000, estimated from the loop", $0) } ?? "No loop to estimate a note from")
            }
            TableColumn("Played") { s in
                Text(s.played.map { $0 ? "yes" : "no" } ?? "")
                    .foregroundStyle(.secondary)
                    .help("Whether the DSP read it while the SPC700's execution log was kept")
            }
            .width(44)
        }
    }

    private var selection: Binding<UInt8?> {
        Binding(
            get: { audio.selectedSample },
            set: {
                audio.selectedSample = $0
                audio.selectedBlock = nil
            }
        )
    }

    @ViewBuilder
    private func detail(_ samples: [SampleInfo]) -> some View {
        if let i = audio.selectedSample, let entry = samples.first(where: { $0.index == i }),
           let sample = audio.sample(i) {
            SampleDetail(audio: audio, entry: entry, sample: sample)
        } else {
            ContentUnavailableView {
                Label("Choose a Sample", systemImage: "waveform")
            } description: {
                Text("The directory at DIR × $100 lists each sample's start and loop point: four bytes an entry, the entry a voice's SRCN names.")
            }
        }
    }
}

struct SampleDetail: View {
    @Bindable var audio: AudioModel
    let entry: SampleInfo
    let sample: BrrSampleInfo

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                HStack(alignment: .firstTextBaseline) {
                    Text("Sample \(entry.index)").font(.headline)
                    Text(summary).font(.callout).foregroundStyle(.secondary)
                    Spacer()
                    if let rom = audio.romOrigin(entry.start) {
                        Button("Show in ROM") { audio.showInRom?(rom) }
                            .help("The ROM bytes the upload sent here, file offset \(formatFileOffset(offset: rom))")
                    }
                }
                WaveformView(sample: sample, selectedBlock: $audio.selectedBlock)
                    .frame(height: 140)
                BlockStrip(sample: sample, selectedBlock: $audio.selectedBlock)
                if let b = audio.selectedBlock, sample.blocks.indices.contains(b) {
                    BlockSteps(block: sample.blocks[b], index: b)
                } else {
                    Text("Click a block, in the strip or the waveform, to see its sixteen values decoded.")
                        .font(.callout).foregroundStyle(.secondary)
                }
            }
            .padding(12)
        }
    }

    private var summary: String {
        var s = "\(AudioStyle.hex(entry.start, 4)), \(sample.blocks.count) blocks, \(sample.samples.count) samples"
        if sample.loops, let lb = sample.loopBlock {
            s += ", loops from block \(lb)"
        } else if !sample.loops {
            s += ", plays once"
        }
        if sample.unterminated { s += ", no end block found" }
        return s
    }
}

/// The decoded values, the loop point, and the block boundaries.
struct WaveformView: View {
    let sample: BrrSampleInfo
    @Binding var selectedBlock: Int?

    var body: some View {
        GeometryReader { g in
            let values = sample.samples
            let n = max(values.count, 1)
            Canvas { ctx, size in
                let mid = size.height / 2
                let x = { (i: Int) in CGFloat(i) / CGFloat(n) * size.width }
                if let b = selectedBlock {
                    ctx.fill(Path(CGRect(x: x(b * 16), y: 0, width: max(1, x(16) - x(0)), height: size.height)),
                             with: .color(.accentColor.opacity(0.15)))
                }
                if let lb = sample.loopBlock {
                    let lx = x(Int(lb) * 16)
                    ctx.fill(Path(CGRect(x: lx, y: 0, width: size.width - lx, height: size.height)),
                             with: .color(.green.opacity(0.06)))
                    ctx.stroke(Path { $0.move(to: CGPoint(x: lx, y: 0)); $0.addLine(to: CGPoint(x: lx, y: size.height)) },
                               with: .color(.green), lineWidth: 1)
                }
                ctx.stroke(Path { $0.move(to: CGPoint(x: 0, y: mid)); $0.addLine(to: CGPoint(x: size.width, y: mid)) },
                           with: .color(.secondary.opacity(0.3)), lineWidth: 0.5)
                var p = Path()
                for (i, v) in values.enumerated() {
                    let pt = CGPoint(x: x(i), y: mid - CGFloat(v) / 32768 * mid)
                    if i == 0 { p.move(to: pt) } else { p.addLine(to: pt) }
                }
                ctx.stroke(p, with: .color(.primary), lineWidth: 1)
            }
            .contentShape(Rectangle())
            .onTapGesture { at in
                let i = Int(at.x / g.size.width * CGFloat(n))
                selectedBlock = min(max(i / 16, 0), sample.blocks.count - 1)
            }
        }
        .background(Color.secondary.opacity(0.05))
        .overlay(alignment: .topLeading) {
            if sample.loopBlock != nil {
                Text("loop").font(.caption2).foregroundStyle(.green).padding(3)
            }
        }
    }
}

/// Each block's header byte: shift, filter and the loop and end flags.
struct BlockStrip: View {
    let sample: BrrSampleInfo
    @Binding var selectedBlock: Int?

    var body: some View {
        ScrollView(.horizontal) {
            LazyHStack(spacing: 2) {
                ForEach(Array(sample.blocks.enumerated()), id: \.offset) { i, b in
                    VStack(spacing: 1) {
                        Text(AudioStyle.hex(b.header, 2)).font(.caption2.monospaced())
                        Text("s\(b.shift) f\(b.filter)").font(.caption2.monospaced()).foregroundStyle(.secondary)
                        Text((b.loops ? "L" : " ") + (b.end ? "E" : " ")).font(.caption2.monospaced())
                            .foregroundStyle(.green)
                    }
                    .padding(3)
                    .background(RoundedRectangle(cornerRadius: 3)
                        .fill(selectedBlock == i ? Color.accentColor.opacity(0.3)
                              : (sample.loopBlock.map { Int($0) == i } ?? false) ? Color.green.opacity(0.15) : Color.secondary.opacity(0.08)))
                    .onTapGesture { selectedBlock = i }
                    .help("Block \(i) at \(AudioStyle.hex(b.offset, 4)): shift \(b.shift), filter \(b.filter)\(b.loops ? ", loop flag" : "")\(b.end ? ", end flag" : "")")
                }
            }
            .padding(.vertical, 2)
        }
        .frame(height: 52)
    }
}

/// A block's sixteen values: each nibble shifted, the filter's prediction
/// from the two before, clamped, wrapped, doubled.
struct BlockSteps: View {
    let block: BrrBlockInfo
    let index: Int

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Block \(index) at \(AudioStyle.hex(block.offset, 4)): header \(AudioStyle.hex(block.header, 2))")
                .font(.headline)
            Text("shift \(block.shift) · filter \(block.filter): \(block.filterFormula) · \(block.filterMeaning)\(block.loops ? " · loop flag" : "")\(block.end ? " · end flag" : "")")
                .font(.callout).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Grid(alignment: .trailing, horizontalSpacing: 12, verticalSpacing: 2) {
                GridRow {
                    ForEach(["#", "nibble", "shifted", "p1", "p2", "prediction", "sum, clamped", "kept (15 bits)", "played"], id: \.self) {
                        Text($0).font(.caption.weight(.medium)).foregroundStyle(.secondary)
                    }
                }
                Divider()
                ForEach(Array(block.steps.enumerated()), id: \.offset) { i, s in
                    GridRow {
                        Text("\(i)").foregroundStyle(.secondary)
                        Text("\(s.nibble)")
                        Text("\(s.shifted)")
                        Text("\(s.p1)").foregroundStyle(.secondary)
                        Text("\(s.p2)").foregroundStyle(.secondary)
                        Text("\(s.prediction)")
                        Text("\(s.clamped)").foregroundStyle(s.clipped ? .orange : .primary)
                            .help(s.clipped ? "\(s.shifted) + \(s.prediction) is past 16 bits: clamped" : "\(s.shifted) + \(s.prediction)")
                        Text("\(s.result)").foregroundStyle(s.wrapped ? .orange : .primary)
                            .help(s.wrapped ? "Wrapped to 15 bits: the DSP keeps one bit less than it clamps to" : "")
                        Text("\(s.output)")
                    }
                    .font(.caption.monospaced())
                }
            }
            if let first = block.steps.first {
                Text("Value 0: \(first.nibble) shifted by \(block.shift) is \(first.shifted); the filter predicts \(first.prediction) from \(first.p1) and \(first.p2); \(first.shifted) + \(first.prediction) = \(first.clamped), kept as \(first.result) and played doubled as \(first.output).")
                    .font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}
