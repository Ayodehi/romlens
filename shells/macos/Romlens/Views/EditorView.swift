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
