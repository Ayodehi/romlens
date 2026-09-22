import RomlensKit
import SwiftUI

/// Navigator on the left, the editor in the middle, the inspector on the
/// right; tabs, address style, analysis status and navigation in the toolbar.
struct DocumentView: View {
    @Bindable var model: RomViewModel

    /// The split view and the inspector re-apply their visibility through
    /// these bindings when the window becomes key (clicking into a second
    /// window), and the default transaction animates that as a slide of the
    /// whole content. Writing through a plain transaction keeps those
    /// re-applications silent; the View menu toggles animate explicitly in
    /// `RomWindowController`.
    private var columnVisibility: Binding<NavigationSplitViewVisibility> {
        Binding(
            get: { model.isNavigatorVisible ? .all : .detailOnly },
            set: { model.isNavigatorVisible = $0 != .detailOnly }
        )
        .transaction(Transaction())
    }

    private var inspectorPresented: Binding<Bool> {
        $model.isInspectorVisible.transaction(Transaction())
    }

    var body: some View {
        NavigationSplitView(columnVisibility: columnVisibility) {
            NavigatorView(model: model)
                .navigationSplitViewColumnWidth(min: 200, ideal: 240, max: 360)
        } detail: {
            VStack(spacing: 0) {
                if model.isStripVisible {
                    RegionStripView(model: model)
                        .frame(height: 28)
                        .help("The whole ROM: one column per pixel. Click to jump.")
                    Divider()
                }
                EditorView(model: model)
                if model.isResultsVisible {
                    Divider()
                    SearchResultsView(model: model)
                }
            }
        }
        .inspector(isPresented: inspectorPresented) {
            RightPaneView(model: model)
                .inspectorColumnWidth(min: 280, ideal: 320, max: 420)
        }
        .toolbar {
            ToolbarItem(placement: .principal) {
                Picker("Editor", selection: $model.editorTab) {
                    ForEach(RomViewModel.EditorTab.allCases) { tab in
                        Text(tab.title).tag(tab)
                    }
                }
                .pickerStyle(.segmented)
                .help("Hex (⌥⌘1), Disassembly (⌥⌘2) or Both (⌥⌘3)")
            }
            ToolbarItem {
                Picker("Address style", selection: $model.addressStyle) {
                    ForEach(AddressStyle.allCases) { style in
                        Text(style.label).tag(style)
                    }
                }
                .pickerStyle(.menu)
                .help("Show file offsets, SNES addresses, or both")
            }
            ToolbarItem {
                AnalysisStatusItem(model: model)
            }
            ToolbarItemGroup {
                Button {
                    model.goBack()
                } label: {
                    Label("Back", systemImage: "chevron.left")
                }
                .disabled(!model.canGoBack)
                .help("Back (⌘[)")
                Button {
                    model.goForward()
                } label: {
                    Label("Forward", systemImage: "chevron.right")
                }
                .disabled(!model.canGoForward)
                .help("Forward (⌘])")
                Button {
                    model.activeSheet = .jump
                } label: {
                    Label("Jump to Address", systemImage: "arrow.right.to.line")
                }
                .help("Jump to a file offset or SNES address (⌘L)")
            }
        }
        .sheet(item: $model.activeSheet) { sheet in
            switch sheet {
            case .jump: JumpToAddressSheet(model: model)
            case .renameLabel: RenameLabelSheet(model: model)
            case .comment: CommentSheet(model: model)
            case .flags: FlagOverrideSheet(model: model)
            case .find: FindSheet(model: model)
            case .dataType: DataTypeSheet(model: model)
            }
        }
    }
}
