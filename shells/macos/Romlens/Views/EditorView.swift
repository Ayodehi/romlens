import RomlensKit
import SwiftUI

/// The centre of the window: Hex, Disassembly, Both, C, Graph or Atlas, or one of
/// the graphics or sound views.
struct EditorView: View {
    @Bindable var model: RomViewModel
    /// What to show: a tab's content (docs/29), or the focused tab's when nil.
    var content: EditorContent? = nil

    var body: some View {
        switch content {
        case .graphics(let tab): GraphicsEditorView(model: model, tab: tab)
        case .audio(let tab): AudioEditorView(model: model, tab: tab)
        case .tutor: TutorTabPlaceholder()
        case .some(let c): textEditor(RomViewModel.EditorTab(content: c) ?? .hex)
        case nil:
            if model.graphicsTab != nil {
                GraphicsEditorView(model: model)
            } else if model.audioTab != nil {
                AudioEditorView(model: model)
            } else {
                textEditor(model.editorTab)
            }
        }
    }

    @ViewBuilder
    private func textEditor(_ tab: RomViewModel.EditorTab) -> some View {
        switch tab {
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
        case .c:
            if model.hasDisassembly {
                CSplitView(model: model)
            } else {
                analyzing
            }
        case .graph:
            if model.hasDisassembly {
                GraphView(model: model)
            } else {
                analyzing
            }
        case .source:
            if model.hasDisassembly {
                SourceSplitView(model: model)
            } else {
                analyzing
            }
        case .atlas:
            AtlasView(model: model)
        case .compare:
            CompareView(model: model)
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

/// Where the tutor shows as a tab until W9 (docs/29).
struct TutorTabPlaceholder: View {
    var body: some View {
        ContentUnavailableView("Tutor", systemImage: "graduationcap", description: Text("Show the tutor with View › Show Tutor."))
    }
}

/// The inspector's drawer (docs/29): Inspector and Tutor as two plain tabs,
/// and for the tutor a button that opens it as a tab in the editor.
struct RightPaneView: View {
    @Bindable var model: RomViewModel

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 16) {
                tab("Inspector", "info.circle", .inspector)
                tab("Tutor", "graduationcap", .tutor)
                Spacer()
                if model.rightPane == .tutor {
                    if let title = model.tutor?.title {
                        Text(title)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                    Button {
                        model.show(.tutor)
                    } label: {
                        Image(systemName: "arrow.up.forward.square")
                    }
                    .buttonStyle(.borderless)
                    .help("Open the tutor as a tab, with more room")
                    .accessibilityLabel("Open the tutor as a tab")
                }
            }
            .padding(.horizontal, 12)
            .frame(height: 32)
            Divider()
            switch model.rightPane {
            case .inspector:
                InspectorView(model: model)
            case .tutor:
                if let tutor = model.tutor {
                    TutorView(tutor: tutor)
                } else {
                    Color.clear.onAppear { model.ensureTutor() }
                }
            }
        }
        .background(Color(nsColor: .windowBackgroundColor))
    }

    private func tab(_ title: String, _ symbol: String, _ pane: RomViewModel.RightPane) -> some View {
        Button {
            if pane == .tutor { model.ensureTutor() }
            model.rightPane = pane
        } label: {
            Label(title, systemImage: symbol)
                .foregroundStyle(model.rightPane == pane ? .primary : .secondary)
                .padding(.vertical, 6)
                .overlay(alignment: .bottom) {
                    if model.rightPane == pane {
                        Rectangle().fill(Color.accentColor).frame(height: 2).offset(y: 3)
                    }
                }
        }
        .buttonStyle(.plain)
        .font(.callout)
    }
}
