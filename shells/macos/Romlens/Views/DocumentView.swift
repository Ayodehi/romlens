import RomlensKit
import SwiftUI

/// The sidebar on the left, the tab groups in the middle, the inspector on
/// the right (docs/29).
///
/// The toolbar holds only commands: the sidebar, Back and Forward. Which
/// view shows is chosen in the editor area, from the tab bars, the
/// representation strip and the View menu, not from the toolbar; the four
/// capsules that used to choose it there are gone.
///
/// What the analyzer found is not a control, so it is not in the toolbar; it
/// is in `RomHeaderBand` with the overview strip, which is about the same
/// thing.
struct DocumentView: View {
    @Bindable var model: RomViewModel

    var body: some View {
        // Three plain panes rather than a `NavigationSplitView` with an
        // inspector. Under the macOS 26+ toolbar a split view controller
        // gives each column its own toolbar section and its own scroll-edge
        // blur, sized to the titlebar and drawn over the top of the column
        // whether or not anything scrolls beneath it. That blur sat on the
        // header band, and the sections broke the toolbar into pieces that
        // read as belonging to the panes below. A plain split keeps the
        // toolbar one cohesive bar across the window, with the panes under it.
        HSplitView {
            if model.isNavigatorVisible {
                SidebarView(model: model)
                    .frame(minWidth: 200, idealWidth: 240, maxWidth: 360, maxHeight: .infinity)
            }
            VStack(spacing: 0) {
                RomHeaderBand(model: model)
                Divider()
                EditorGridView(model: model)
                if model.isResultsVisible {
                    Divider()
                    switch model.resultsKind {
                    case .find: SearchResultsView(model: model)
                    case .references: ReferencesView(model: model)
                    }
                }
            }
            .frame(minWidth: 420, maxWidth: .infinity, maxHeight: .infinity)
            if model.isInspectorVisible {
                RightPaneView(model: model)
                    .frame(minWidth: 280, idealWidth: 320, maxWidth: 420, maxHeight: .infinity)
            }
        }
        // The window is sized by the person, not the content
        // (`RomWindowController`), so the panes fill it whatever the
        // editor shows.
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .toolbar {
            ToolbarItem(placement: .navigation) {
                Button {
                    withAnimation { model.isNavigatorVisible.toggle() }
                } label: {
                    Label("Sidebar", systemImage: "sidebar.left")
                }
                .help("Show or hide the sidebar (⌘0)")
            }
            ToolbarItem(placement: .principal) {
                JumpBar(model: model)
            }
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
                Button {
                    model.activeSheet = .openQuickly
                } label: {
                    Label("Open Quickly", systemImage: "magnifyingglass")
                }
                .help("Open Quickly: a label, variable, address or view (⇧⌘O)")
            }
            ToolbarItem {
                Button {
                    withAnimation { model.isInspectorVisible.toggle() }
                } label: {
                    Label("Inspector", systemImage: "sidebar.right")
                }
                .help("Show or hide the inspector (⌥⌘0)")
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
            case .variable: VariableSheet(model: model)
            case .cEdit: CEditSheet(model: model)
            case .openQuickly: OpenQuicklySheet(model: model)
            }
        }
    }
}
