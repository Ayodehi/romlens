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
