import RomlensKit
import SwiftUI

/// The bar under the tab groups (docs/29): the whole ROM as a slim strip,
/// a map of it to click, and what the analyzer made of it.
///
/// It was a band over the editor, where it sat above every view, the
/// graphics and sound views too, and said nothing about what they showed.
/// Under the editor it reads as the window's status, which it is. The
/// swatches are the strip's legend.
struct WorkspaceStatusBar: View {
    let model: RomViewModel

    private var session: WorkbenchSession { model.session }

    var body: some View {
        VStack(spacing: 4) {
            if model.isStripVisible {
                RegionStripView(model: model)
                    .frame(height: 10)
                    .clipShape(RoundedRectangle(cornerRadius: 2))
                    .overlay(
                        RoundedRectangle(cornerRadius: 2)
                            .strokeBorder(.separator, lineWidth: 1)
                    )
                    .help("The whole ROM, one column per pixel. Click to jump.")
            }
            status
        }
        .padding(.horizontal, 10)
        .padding(.top, model.isStripVisible ? 6 : 4)
        .padding(.bottom, 4)
    }

    @ViewBuilder
    private var status: some View {
        HStack(spacing: 14) {
            switch session.analysis {
            case .running where session.stats != nil:
                // A rerun (every second in a live session) keeps showing the
                // last numbers rather than swapping them for a bar and back.
                statsView(session.stats!)
                ProgressView().controlSize(.mini)
                    .help("Analyzing again")
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
                    statsView(stats)
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

    @ViewBuilder
    private func statsView(_ stats: AnalysisStats) -> some View {
        let total = max(1, Double(stats.codeBytes + stats.dataBytes + stats.unknownBytes))
        // The swatches are the strip's own colours, so the legend and the
        // numbers are one thing rather than two.
        swatch(kindCode: 1, "code", Double(stats.codeBytes) / total)
        swatch(kindCode: 2, "data", Double(stats.dataBytes) / total)
        swatch(kindCode: 0, "unknown", Double(stats.unknownBytes) / total)
        Spacer(minLength: 8)
        Text("\(stats.regions) regions")
            .foregroundStyle(.tertiary)
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
