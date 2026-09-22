import RomlensKit
import SwiftUI

/// Navigator on the left, the editor in the middle, the inspector on the
/// right; tabs, address style, analysis status and navigation in the toolbar.
struct DocumentView: View {
    @Bindable var model: RomViewModel
    @State private var columns: NavigationSplitViewVisibility = .all

    var body: some View {
        NavigationSplitView(columnVisibility: $columns) {
            NavigatorView(model: model)
                .navigationSplitViewColumnWidth(min: 200, ideal: 240, max: 360)
        } detail: {
            EditorView(model: model)
        }
        .inspector(isPresented: $model.isInspectorVisible) {
            RightPaneView(model: model)
                .inspectorColumnWidth(min: 280, ideal: 320, max: 420)
        }
        .onChange(of: model.isNavigatorVisible, initial: true) { _, visible in
            columns = visible ? .all : .detailOnly
        }
        .onChange(of: columns) { _, value in
            model.isNavigatorVisible = value != .detailOnly
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
            }
        }
    }
}
