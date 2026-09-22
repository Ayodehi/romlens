import RomlensKit
import SwiftUI

/// The centre of the window: Hex, Disassembly or Both.
struct EditorView: View {
    @Bindable var model: RomViewModel

    var body: some View {
        switch model.editorTab {
        case .hex:
            HexTableView(model: model)
        case .disassembly:
            if model.hasDisassembly {
                AsmTableView(model: model)
            } else {
                analyzing
            }
        case .both:
            if model.hasDisassembly {
                LockstepEditorView(model: model)
            } else {
                analyzing
            }
        }
    }

    private var analyzing: some View {
        ContentUnavailableView {
            Label("Analyzing…", systemImage: "cpu")
        } description: {
            Text("The disassembly appears when the first analysis finishes. The Hex tab works meanwhile.")
        } actions: {
            if case .failed(let message) = model.session.analysis {
                Text(message).foregroundStyle(.red)
                Button("Retry") { model.session.startAnalysis() }
            }
        }
    }
}

/// The one-case right pane; the tutor is one more case later.
struct RightPaneView: View {
    let model: RomViewModel

    var body: some View {
        switch model.rightPane {
        case .inspector:
            InspectorView(model: model)
        }
    }
}

/// Toolbar item: progress and phase while analyzing (with cancel), region
/// percentages when done; tap to re-run.
struct AnalysisStatusItem: View {
    let model: RomViewModel

    var body: some View {
        let session = model.session
        switch session.analysis {
        case .running(let fraction, let phase):
            HStack(spacing: 6) {
                ProgressView(value: fraction)
                    .progressViewStyle(.linear)
                    .frame(width: 80)
                Text(phase).font(.caption).foregroundStyle(.secondary)
                Button {
                    session.cancelAnalysis()
                } label: {
                    Image(systemName: "xmark.circle.fill")
                }
                .buttonStyle(.plain)
                .help("Cancel analysis (⌘.)")
                .keyboardShortcut(".", modifiers: .command)
            }
        case .failed(let message):
            Button {
                session.startAnalysis()
            } label: {
                Label("Analysis failed", systemImage: "exclamationmark.triangle")
            }
            .help(message)
        case .idle:
            Button {
                session.startAnalysis()
            } label: {
                if let stats = session.stats {
                    let total = max(1, Double(stats.codeBytes + stats.dataBytes + stats.unknownBytes))
                    Text(String(
                        format: "code %.1f%% · data %.1f%% · unknown %.1f%%",
                        Double(stats.codeBytes) / total * 100,
                        Double(stats.dataBytes) / total * 100,
                        Double(stats.unknownBytes) / total * 100
                    ))
                    .font(.caption.monospacedDigit())
                } else {
                    Label("Analyze", systemImage: "cpu")
                }
            }
            .buttonStyle(.plain)
            .help("Re-run the analysis")
        }
    }
}
