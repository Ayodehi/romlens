import RomlensKit
import SwiftUI

/// The band under the toolbar: the whole ROM as a strip, and what the analyzer
/// made of it.
///
/// The percentages used to be a toolbar item, which was the wrong place twice
/// over. A toolbar holds *controls*, and a thirty-character readout wedged
/// between a popup button and the window edge reads as neither a control nor a
/// label — it just makes the trailing edge look arbitrary. And the numbers are
/// about the same thing the strip is: what the whole image turned out to be.
/// Together they explain each other, the swatches doubling as the strip's
/// legend, and the toolbar is left holding only things you click.
struct RomHeaderBand: View {
    let model: RomViewModel

    private var session: WorkbenchSession { model.session }

    var body: some View {
        VStack(spacing: 6) {
            if model.isStripVisible {
                RegionStripView(model: model)
                    .frame(height: 20)
                    .clipShape(RoundedRectangle(cornerRadius: 3))
                    .overlay(
                        RoundedRectangle(cornerRadius: 3)
                            .strokeBorder(.separator, lineWidth: 1)
                    )
                    .help("The whole ROM, one column per pixel. Click to jump.")
            }
            status
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }

    @ViewBuilder
    private var status: some View {
        HStack(spacing: 14) {
            switch session.analysis {
            case .running(let fraction, let phase):
                ProgressView(value: fraction)
                    .progressViewStyle(.linear)
                    .frame(width: 120)
                Text(phase)
                Spacer(minLength: 8)
                Button("Cancel") { session.cancelAnalysis() }
                    .buttonStyle(.link)
                    .keyboardShortcut(".", modifiers: .command)
            case .failed(let message):
                Label("Analysis failed", systemImage: "exclamationmark.triangle")
                    .foregroundStyle(.orange)
                    .help(message)
                Spacer(minLength: 8)
                Button("Retry") { session.startAnalysis() }
                    .buttonStyle(.link)
            case .idle:
                if let stats = session.stats {
                    let total = max(1, Double(stats.codeBytes + stats.dataBytes + stats.unknownBytes))
                    // The swatches are the strip's own colours, so the legend
                    // and the numbers are one thing rather than two.
                    swatch(kindCode: 1, "code", Double(stats.codeBytes) / total)
                    swatch(kindCode: 2, "data", Double(stats.dataBytes) / total)
                    swatch(kindCode: 0, "unknown", Double(stats.unknownBytes) / total)
                    Spacer(minLength: 8)
                    Text("\(stats.regions) regions")
                        .foregroundStyle(.tertiary)
                    Button {
                        session.startAnalysis()
                    } label: {
                        Image(systemName: "arrow.clockwise")
                    }
                    .buttonStyle(.borderless)
                    .help("Analyze again")
                } else {
                    Text("Not analyzed").foregroundStyle(.secondary)
                    Spacer(minLength: 8)
                    Button("Analyze") { session.startAnalysis() }
                        .buttonStyle(.link)
                }
            }
        }
        .font(.caption.monospacedDigit())
        .foregroundStyle(.secondary)
        .frame(height: 16)
    }

    private func swatch(kindCode: UInt8, _ name: String, _ fraction: Double) -> some View {
        HStack(spacing: 5) {
            RoundedRectangle(cornerRadius: 2)
                .fill(Color(nsColor: RegionStrip.color(forKindCode: kindCode)))
                .overlay(
                    RoundedRectangle(cornerRadius: 2)
                        .strokeBorder(.separator, lineWidth: kindCode == 0 ? 1 : 0)
                )
                .frame(width: 9, height: 9)
            Text(name)
            Text(String(format: "%.1f%%", fraction * 100))
                .foregroundStyle(.primary)
        }
        .help("\(name): \(String(format: "%.1f%%", fraction * 100)) of the image")
    }
}
