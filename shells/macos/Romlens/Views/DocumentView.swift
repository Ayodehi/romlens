import RomlensKit
import SwiftUI

/// Hex table on the left, inspector on the right, address style and jump in
/// the toolbar.
struct DocumentView: View {
    @Bindable var model: RomViewModel

    var body: some View {
        HSplitView {
            HexTableView(model: model)
                .frame(minWidth: 480, maxWidth: .infinity, maxHeight: .infinity)
            InspectorView(model: model)
                .frame(minWidth: 280, idealWidth: 300, maxWidth: 380, maxHeight: .infinity)
        }
        .toolbar {
            ToolbarItem(placement: .principal) {
                Picker("Address style", selection: $model.addressStyle) {
                    ForEach(AddressStyle.allCases) { style in
                        Text(style.label).tag(style)
                    }
                }
                .pickerStyle(.segmented)
                .help("Show file offsets, SNES addresses, or both")
            }
            ToolbarItem {
                Button {
                    model.goBack()
                } label: {
                    Label("Back", systemImage: "chevron.left")
                }
                .disabled(!model.canGoBack)
                .help("Back to the previous location (⌘[)")
            }
            ToolbarItem {
                Button {
                    model.isShowingJumpSheet = true
                } label: {
                    Label("Jump to Address", systemImage: "arrow.right.to.line")
                }
                .help("Jump to a file offset or SNES address (⌘L)")
            }
        }
        .sheet(isPresented: $model.isShowingJumpSheet) {
            JumpToAddressSheet(model: model)
        }
    }
}
