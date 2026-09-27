import RomlensKit
import SwiftUI

/// Echo, noise and pitch modulation (docs/23, A13): the echo's buffer in
/// audio RAM, its delay and feedback, the FIR filter's eight taps and what
/// they do to each frequency.
struct EchoView: View {
    let audio: AudioModel

    var body: some View {
        let regs = audio.state?.registers ?? []
        let r = { (i: Int) in regs.indices.contains(i) ? regs[i] : nil }
        let v = { (i: Int) in r(i)?.value ?? 0 }
        let taps = (0..<8).map { Int8(bitPattern: v($0 << 4 | 0xF)) }
        let edl = Int(v(0x7D) & 0xF)
        let esa = Int(v(0x6D)) << 8
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                GroupBox("The echo") {
                    VStack(alignment: .leading, spacing: 6) {
                        Text("A ring buffer in audio RAM at \(AudioStyle.hex(esa, 4)), \(edl == 0 ? 4 : edl * 2048) bytes: \(edl * 16) ms of delay (EDL × 16 ms). Each sample, the voices EON names are added in, and what comes back out is fed through the FIR filter, mixed in at EVOL and fed back at EFB.")
                            .font(.callout).fixedSize(horizontal: false, vertical: true)
                        if v(0x6C) & 0x20 != 0 {
                            Label("FLG bit 5 is set: echo writes are off, so the buffer is not written (the driver has its RAM for other things, or the echo is being set up).", systemImage: "exclamationmark.triangle")
                                .font(.caption).foregroundStyle(.orange)
                        }
                        ForEach([0x4D, 0x0D, 0x2C, 0x3C, 0x6D, 0x7D], id: \.self) { i in
                            if let reg = r(i) { DspRegisterRow(register: reg) }
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                GroupBox("The FIR filter") {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("Eight taps, each a signed fraction of 128, weight the last eight echo samples: FIR0 the oldest, FIR7 the newest. Their sum shapes the echo's sound: taps that add up smoothly keep the low frequencies and dull the high, as a room does.")
                            .font(.callout).fixedSize(horizontal: false, vertical: true)
                        HStack(alignment: .top, spacing: 20) {
                            TapBars(taps: taps).frame(width: 220, height: 120)
                            FirResponse(taps: taps).frame(height: 120)
                        }
                        Text("Taps: " + taps.enumerated().map { "FIR\($0.offset) \($0.element)" }.joined(separator: " · "))
                            .font(.caption.monospaced()).foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                GroupBox("Noise and pitch modulation") {
                    VStack(alignment: .leading, spacing: 6) {
                        Text("A voice NON names plays the noise generator in place of its sample, at FLG's noise clock. A voice PMON names bends its pitch by the wave of the voice before it.")
                            .font(.callout).fixedSize(horizontal: false, vertical: true)
                        ForEach([0x3D, 0x2D, 0x6C], id: \.self) { i in
                            if let reg = r(i) { DspRegisterRow(register: reg) }
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            .padding(12)
        }
    }
}

struct TapBars: View {
    let taps: [Int8]

    var body: some View {
        Canvas { ctx, size in
            let mid = size.height / 2
            let w = size.width / 8
            for (i, t) in taps.enumerated() {
                let h = CGFloat(t) / 128 * mid
                let rect = CGRect(x: CGFloat(i) * w + 3, y: h >= 0 ? mid - h : mid, width: w - 6, height: max(1, abs(h)))
                ctx.fill(Path(rect), with: .color(t >= 0 ? .accentColor : .orange))
            }
            ctx.stroke(Path { $0.move(to: CGPoint(x: 0, y: mid)); $0.addLine(to: CGPoint(x: size.width, y: mid)) },
                       with: .color(.secondary.opacity(0.4)), lineWidth: 0.5)
        }
        .background(Color.secondary.opacity(0.05))
        .help("Each tap, FIR0 (oldest) to FIR7 (newest)")
    }
}

/// |H(f)| of the taps from 0 to 16 kHz, in decibels.
struct FirResponse: View {
    let taps: [Int8]

    var body: some View {
        let points = Self.response(taps, count: 128)
        Canvas { ctx, size in
            let y = { (db: Double) in size.height * CGFloat((12 - min(max(db, -36), 12)) / 48) }
            for db in stride(from: 12.0, through: -36, by: -12) {
                ctx.stroke(Path { $0.move(to: CGPoint(x: 0, y: y(db))); $0.addLine(to: CGPoint(x: size.width, y: y(db))) },
                           with: .color(.secondary.opacity(db == 0 ? 0.5 : 0.15)), lineWidth: 0.5)
            }
            var p = Path()
            for (i, db) in points.enumerated() {
                let pt = CGPoint(x: CGFloat(i) / CGFloat(points.count - 1) * size.width, y: y(db))
                if i == 0 { p.move(to: pt) } else { p.addLine(to: pt) }
            }
            ctx.stroke(p, with: .color(.accentColor), lineWidth: 1.5)
        }
        .background(Color.secondary.opacity(0.05))
        .overlay(alignment: .bottomLeading) { Text("0 Hz").font(.caption2).foregroundStyle(.secondary).padding(2) }
        .overlay(alignment: .bottomTrailing) { Text("16 kHz").font(.caption2).foregroundStyle(.secondary).padding(2) }
        .overlay(alignment: .topLeading) { Text("+12 dB").font(.caption2).foregroundStyle(.secondary).padding(2) }
        .help("How loud the echo comes back at each frequency, from 0 to 16 kHz: the line at 0 dB is unchanged")
    }

    static func response(_ taps: [Int8], count: Int) -> [Double] {
        (0..<count).map { i in
            let w = Double.pi * Double(i) / Double(count - 1)
            var re = 0.0, im = 0.0
            for (k, t) in taps.enumerated() {
                let c = Double(t) / 128
                re += c * cos(w * Double(k))
                im -= c * sin(w * Double(k))
            }
            return 20 * log10(max(sqrt(re * re + im * im), 1e-4))
        }
    }
}
