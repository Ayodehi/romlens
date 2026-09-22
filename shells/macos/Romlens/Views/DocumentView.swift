import RomlensKit
import SwiftUI

/// Navigator on the left, the editor in the middle, the inspector on the
/// right.
///
/// The toolbar is in three groups, by what a thing *is* rather than where it
/// fits: navigation on the leading edge where macOS puts back and forward,
/// the editor tabs in the centre, and the view options and the analysis
/// status trailing. They were previously all trailing, which ran the status
/// text and the navigation buttons together into one pill that read as a
/// single control.
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
                    // Inset rather than edge to edge: the strip is a view of
                    // the whole ROM, not a continuation of the editor's own
                    // scroll, and butting it against the toolbar made it read
                    // as part of the chrome.
                    RegionStripView(model: model)
                        .frame(height: 22)
                        .clipShape(RoundedRectangle(cornerRadius: 4))
                        .overlay(
                            RoundedRectangle(cornerRadius: 4)
                                .strokeBorder(.separator, lineWidth: 1)
                        )
                        .padding(.horizontal, 12)
                        .padding(.top, 10)
                        .padding(.bottom, 8)
                        .help("The whole ROM, one column per pixel. Click to jump.")
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
            // Leading: back and forward, the pair macOS puts here, and only
            // that pair. The titlebar shares this space, so a third button
            // truncated the document's name — and "Jump to Address" was the
            // odd one out anyway: the others move through history, it opens a
            // sheet, and it is ⌘L and a Go menu item already.
            ToolbarItemGroup(placement: .navigation) {
                Button {
                    model.goBack()
                } label: {
                    Label("Back", systemImage: "chevron.backward")
                }
                .disabled(!model.canGoBack)
                .help("Back (⌘[)")
                Button {
                    model.goForward()
                } label: {
                    Label("Forward", systemImage: "chevron.forward")
                }
                .disabled(!model.canGoForward)
                .help("Forward (⌘])")
            }
            ToolbarItem(placement: .principal) {
                Picker("Editor", selection: $model.editorTab) {
                    ForEach(RomViewModel.EditorTab.allCases) { tab in
                        Text(tab.title).tag(tab)
                    }
                }
                .pickerStyle(.segmented)
                .help("Hex (⌥⌘1), Disassembly (⌥⌘2) or Both (⌥⌘3)")
            }
            // Trailing: what is shown, then what was found. The status is
            // informational and sits last, furthest from the controls.
            ToolbarItem {
                Picker(selection: $model.addressStyle) {
                    ForEach(AddressStyle.allCases) { style in
                        Text(style.label).tag(style)
                    }
                } label: {
                    Label("Addresses", systemImage: "number")
                }
                .pickerStyle(.menu)
                .help("Which address columns the editor shows")
            }
            ToolbarItem {
                AnalysisStatusItem(model: model)
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
