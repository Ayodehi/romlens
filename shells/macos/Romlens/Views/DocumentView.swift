import RomlensKit
import SwiftUI

/// Navigator on the left, the editor in the middle, the inspector on the
/// right.
///
/// The toolbar holds only controls: the editor tabs in the centre, and
/// navigation and the address style trailing. Nothing is added on the leading
/// edge, because in a `NavigationSplitView` that space is the width of the
/// sidebar and the document's title has to share it — one extra button there
/// truncated "SuperMetroid.F8DF".
///
/// What the analyzer found is not a control, so it is not in the toolbar; it
/// is in `RomHeaderBand` with the overview strip, which is about the same
/// thing.
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

    /// No segment is selected while a graphics view has the editor, so the
    /// control never claims a tab that is not showing.
    private var editorTab: Binding<RomViewModel.EditorTab?> {
        Binding(
            get: { model.graphicsTab == nil ? model.editorTab : nil },
            set: { if let tab = $0 { model.editorTab = tab } }
        )
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
                RomHeaderBand(model: model)
                Divider()
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
                Picker("Editor", selection: editorTab) {
                    ForEach(RomViewModel.EditorTab.allCases) { tab in
                        Text(tab.title).tag(Optional(tab))
                    }
                }
                .pickerStyle(.segmented)
                .help("Hex (⌥⌘1), Disassembly (⌥⌘2) or Both (⌥⌘3)")
            }
            ToolbarItem(placement: .principal) {
                GraphicsMenu(model: model)
            }
            // Trailing: two bordered control groups, nothing else. Where
            // you are, then what you are looking at.
            ToolbarItemGroup {
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
