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
    /// The window is too narrow for the editor tabs as a segmented control.
    /// A toolbar item that does not fit goes to the overflow menu, and a
    /// segmented control shows there as blank checkmarks, so below this
    /// width the tabs are a menu instead, which always fits.
    @State private var compactToolbar = false
    static let compactWidth: CGFloat = 1180

    /// No segment is selected while a graphics view has the editor, so the
    /// control never claims a tab that is not showing.
    private var editorTab: Binding<RomViewModel.EditorTab?> {
        Binding(
            get: { model.graphicsTab == nil ? model.editorTab : nil },
            set: { if let tab = $0 { model.editorTab = tab } }
        )
    }

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
                NavigatorView(model: model)
                    .frame(minWidth: 200, idealWidth: 240, maxWidth: 360)
            }
            VStack(spacing: 0) {
                RomHeaderBand(model: model)
                Divider()
                EditorView(model: model)
                if model.isResultsVisible {
                    Divider()
                    switch model.resultsKind {
                    case .find: SearchResultsView(model: model)
                    case .references: ReferencesView(model: model)
                    }
                }
            }
            .frame(minWidth: 420, maxWidth: .infinity)
            if model.isInspectorVisible {
                RightPaneView(model: model)
                    .frame(minWidth: 280, idealWidth: 320, maxWidth: 420)
            }
        }
        .background {
            GeometryReader { g in
                Color.clear
                    .onAppear { compactToolbar = g.size.width < Self.compactWidth }
                    .onChange(of: g.size.width) { _, w in
                        compactToolbar = w < Self.compactWidth
                    }
            }
        }
        .toolbar {
            ToolbarItem(placement: .navigation) {
                Button {
                    withAnimation { model.isNavigatorVisible.toggle() }
                } label: {
                    Label("Navigator", systemImage: "sidebar.left")
                }
                .help("Show or hide the navigator (⌘0)")
            }
            ToolbarItem(placement: .navigation) {
                Button {
                    withAnimation { model.toggleFocus() }
                } label: {
                    Label(
                        model.isFocused ? "Show Panels" : "Focus on Code",
                        systemImage: model.isFocused
                            ? "arrow.down.right.and.arrow.up.left"
                            : "arrow.up.left.and.arrow.down.right"
                    )
                }
                .help("Hide or show the navigator, inspector and overview strip together (⌥⌘F)")
            }
            ToolbarItem(placement: .principal) {
                if compactToolbar {
                    Menu {
                        Picker("Editor", selection: editorTab) {
                            ForEach(RomViewModel.EditorTab.allCases) { tab in
                                Text(tab.title).tag(Optional(tab))
                            }
                        }
                        .pickerStyle(.inline)
                        .labelsHidden()
                    } label: {
                        Text(model.graphicsTab == nil ? model.editorTab.title : "Editor")
                            .padding(.horizontal, 8)
                    }
                    .menuStyle(.button)
                    .controlSize(.large)
                    .fixedSize()
                    .help("Hex (⌥⌘1), Disassembly (⌥⌘2), Both (⌥⌘3), C (⌥⌘8) or Graph (⌥⌘9)")
                } else {
                    Picker("Editor", selection: editorTab) {
                        ForEach(RomViewModel.EditorTab.allCases) { tab in
                            Text(tab.title).tag(Optional(tab))
                        }
                    }
                    .pickerStyle(.segmented)
                    .controlSize(.large)
                    .help("Hex (⌥⌘1), Disassembly (⌥⌘2), Both (⌥⌘3), C (⌥⌘8) or Graph (⌥⌘9)")
                }
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
                // A `Menu` rather than a `Picker(.menu)`: a menu-style picker
                // renders as an `NSPopUpButton`, whose title inset is much
                // tighter than a segmented control's, so the two sat in the
                // toolbar with visibly different padding. Building the label
                // by hand is what makes its insets ours to match.
                Menu {
                    Picker("Addresses", selection: $model.addressStyle) {
                        ForEach(AddressStyle.allCases) { style in
                            Text(style.label).tag(style)
                        }
                    }
                    .pickerStyle(.inline)
                    .labelsHidden()
                } label: {
                    Text(model.addressStyle.shortLabel)
                        .padding(.horizontal, 8)
                }
                .menuStyle(.button)
                .controlSize(.large)
                .fixedSize()
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
            case .variable: VariableSheet(model: model)
            }
        }
    }
}
