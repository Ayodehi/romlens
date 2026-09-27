import RomlensKit
import SwiftUI

/// The eight voices as channel strips (docs/23): each one's sample, note,
/// volume, envelope and flags, and beside them the DSP registers of the
/// voice selected, then the global ones, each explained.
struct VoicesView: View {
    @Bindable var audio: AudioModel

    var body: some View {
        let state = audio.state
        HSplitView {
            ScrollView {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 300), spacing: 12)], spacing: 12) {
                    ForEach(state?.voices ?? [], id: \.index) { v in
                        VoiceStrip(audio: audio, voice: v, selected: audio.selectedVoice == Int(v.index))
                            .contentShape(Rectangle())
                            .onTapGesture { audio.selectedVoice = Int(v.index) }
                    }
                }
                .padding(12)
            }
            .frame(minWidth: 320, maxWidth: .infinity, maxHeight: .infinity)
            DspRegistersList(registers: state?.registers ?? [], voice: audio.selectedVoice)
                .frame(minWidth: 260, idealWidth: 340, maxWidth: 480, maxHeight: .infinity)
        }
    }
}

/// One voice.
struct VoiceStrip: View {
    let audio: AudioModel
    let voice: VoiceInfo
    let selected: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Circle()
                    .fill(voice.sounding ? Color.green : Color.secondary.opacity(0.3))
                    .frame(width: 8, height: 8)
                    .help(voice.sounding ? "Sounding: its envelope is above silence" : "Silent")
                Text("Voice \(voice.index)").font(.headline)
                Spacer()
                Text(voice.note.map { "≈ \($0)" } ?? "")
                    .font(.callout.monospaced())
                    .help("The note, estimated from the loop of its sample: a pitch is a rate, not a note")
                let v = Int(voice.index)
                Toggle("M", isOn: Binding(get: { audio.muted & (1 << UInt8(v)) != 0 }, set: { _ in audio.toggleMute(v) }))
                    .toggleStyle(.button)
                    .controlSize(.small)
                    .help("Mute: leave this voice out of what you hear; it still runs")
                Toggle("S", isOn: Binding(get: { audio.isSolo(v) }, set: { _ in audio.toggleSolo(v) }))
                    .toggleStyle(.button)
                    .controlSize(.small)
                    .help("Solo: hear only this voice")
            }
            Text(sampleLine).font(.caption.monospaced()).foregroundStyle(.secondary)
            Text("pitch \(AudioStyle.hex(voice.pitch, 4)): \(voice.pitchWords)")
                .font(.caption).foregroundStyle(.secondary)
                .lineLimit(2)
            HStack(spacing: 8) {
                VolumeBar(label: "L", value: voice.volumeLeft)
                VolumeBar(label: "R", value: voice.volumeRight)
            }
            EnvelopeView(voice: voice)
                .frame(height: 64)
            Text(voice.envelopeWords)
                .font(.caption2).foregroundStyle(.secondary)
                .lineLimit(3)
                .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 4) {
                Flag("echo", on: voice.echo, help: "EON: it feeds the echo")
                Flag("noise", on: voice.noise, help: "NON: it plays the noise generator, not its sample")
                Flag("pitch mod", on: voice.modulated, help: "PMON: its pitch follows the wave of the voice before")
                Flag("keyed off", on: voice.keyedOff, help: "KOFF holds it off: its envelope releases")
                Flag("ended", on: voice.ended, help: "ENDX: its sample reached a block with the end flag")
            }
        }
        .padding(10)
        .background(RoundedRectangle(cornerRadius: 8).fill(Color(nsColor: .controlBackgroundColor)))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(selected ? Color.accentColor : Color.secondary.opacity(0.2), lineWidth: selected ? 2 : 1))
    }

    private var sampleLine: String {
        if let s = voice.sampleStart, let l = voice.sampleLoop {
            return "sample \(voice.source) at \(AudioStyle.hex(s, 4)), loop \(AudioStyle.hex(l, 4))"
        }
        return "sample \(voice.source)"
    }
}

/// A signed volume, centred: a negative one inverts the wave's phase.
struct VolumeBar: View {
    let label: String
    let value: Int8

    var body: some View {
        HStack(spacing: 4) {
            Text(label).font(.caption2.monospaced()).foregroundStyle(.secondary)
            GeometryReader { g in
                let w = g.size.width
                let mid = w / 2
                let len = CGFloat(abs(Int(value))) / 128 * mid
                ZStack(alignment: .leading) {
                    Capsule().fill(Color.secondary.opacity(0.15))
                    Rectangle().fill(value < 0 ? Color.orange : Color.accentColor)
                        .frame(width: len)
                        .offset(x: value < 0 ? mid - len : mid)
                    Rectangle().fill(Color.secondary.opacity(0.5)).frame(width: 1).offset(x: mid)
                }
            }
            .frame(height: 6)
            Text("\(value)").font(.caption2.monospaced()).frame(width: 30, alignment: .trailing)
        }
        .help(value < 0 ? "Volume \(value) of 127, phase inverted" : "Volume \(value) of 127")
    }
}

struct Flag: View {
    let text: String
    let on: Bool
    let help: String

    init(_ text: String, on: Bool, help: String) {
        self.text = text
        self.on = on
        self.help = help
    }

    var body: some View {
        Text(text)
            .font(.caption2)
            .padding(.horizontal, 5)
            .padding(.vertical, 1)
            .background(Capsule().fill(on ? Color.accentColor.opacity(0.25) : Color.clear))
            .overlay(Capsule().stroke(Color.secondary.opacity(on ? 0.6 : 0.2)))
            .foregroundStyle(on ? .primary : .tertiary)
            .help(help)
    }
}

/// The envelope a voice's ADSR or GAIN makes, run through the DSP itself:
/// held for 1.5 s, then released, with ENVX now marked as a line.
struct EnvelopeView: View {
    let voice: VoiceInfo
    static let holdMs: UInt32 = 1500
    static let releaseMs: UInt32 = 250

    var body: some View {
        let curve = Self.curve(voice.adsr1, voice.adsr2, voice.gain)
        Canvas { ctx, size in
            let n = curve.count
            guard n > 1 else { return }
            let x = { (i: Int) in CGFloat(i) / CGFloat(n - 1) * size.width }
            let y = { (v: UInt16) in size.height - CGFloat(v) / 2047 * size.height }
            var p = Path()
            p.move(to: CGPoint(x: 0, y: y(curve[0])))
            for i in 1..<n { p.addLine(to: CGPoint(x: x(i), y: y(curve[i]))) }
            ctx.stroke(p, with: .color(.accentColor), lineWidth: 1.5)
            // Key off.
            let off = CGFloat(Self.holdMs) / CGFloat(Self.holdMs + Self.releaseMs) * size.width
            ctx.stroke(Path { $0.move(to: CGPoint(x: off, y: 0)); $0.addLine(to: CGPoint(x: off, y: size.height)) },
                       with: .color(.secondary.opacity(0.5)), style: StrokeStyle(lineWidth: 1, dash: [3, 3]))
            // ENVX now, 0–127 of the 11-bit level.
            let now = size.height - CGFloat(voice.envx) / 127 * size.height
            ctx.stroke(Path { $0.move(to: CGPoint(x: 0, y: now)); $0.addLine(to: CGPoint(x: size.width, y: now)) },
                       with: .color(.green.opacity(0.8)), lineWidth: 1)
        }
        .background(Color.secondary.opacity(0.06))
        .overlay(alignment: .topTrailing) {
            Text("ENVX \(voice.envx)").font(.caption2.monospaced()).foregroundStyle(.green).padding(2)
        }
        .overlay(alignment: .bottomTrailing) {
            Text("key off at 1.5 s").font(.caption2).foregroundStyle(.tertiary).padding(2)
        }
        .help("The envelope these settings make, keyed on then off at 1.5 s, run through Romlens's DSP; the green line is ENVX now")
    }

    @MainActor private static var curves: [UInt32: [UInt16]] = [:]

    @MainActor static func curve(_ a1: UInt8, _ a2: UInt8, _ g: UInt8) -> [UInt16] {
        let key = UInt32(a1) << 16 | UInt32(a2) << 8 | UInt32(g)
        if let c = curves[key] { return c }
        let c = envelopeCurve(adsr1: a1, adsr2: a2, gain: g, holdMs: holdMs, releaseMs: releaseMs, stride: 64)
        curves[key] = c
        return c
    }
}

/// The DSP's registers: the selected voice's ten, then the globals, each
/// with its value in words and, opened, field by field.
struct DspRegistersList: View {
    let registers: [DspRegisterInfo]
    let voice: Int?

    var body: some View {
        List {
            if let v = voice {
                Section("Voice \(v)") {
                    ForEach(registers.filter { $0.voice == UInt8(v) && !$0.unused }, id: \.register) { r in
                        DspRegisterRow(register: r)
                    }
                }
            }
            Section("Global") {
                ForEach(registers.filter { $0.voice == nil && !$0.unused }, id: \.register) { r in
                    DspRegisterRow(register: r)
                }
            }
        }
        .listStyle(.sidebar)
    }
}

struct DspRegisterRow: View {
    let register: DspRegisterInfo
    @State private var open = false

    var body: some View {
        DisclosureGroup(isExpanded: $open) {
            RegisterPartsView(parts: register.parts)
                .padding(.vertical, 4)
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Text(AudioStyle.hex(register.register, 2))
                    .font(.caption.monospaced()).foregroundStyle(.secondary)
                Text(register.short)
                    .font(.caption.monospaced())
                    .lineLimit(2)
            }
        }
    }
}
