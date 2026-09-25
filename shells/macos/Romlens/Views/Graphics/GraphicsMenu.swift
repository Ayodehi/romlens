import SwiftUI

/// The toolbar's Graphics picker, beside the Hex / Disassembly / Both
/// segments rather than inside them: seven segments is too wide, and the
/// three Phase 0–1 tabs keep their muscle memory (`16-phase2-plan.md` 2B.8).
///
/// A `Menu` with a hand-made label rather than a `Picker(.menu)`: a
/// menu-style picker renders as an `NSPopUpButton`, whose title inset is
/// tighter than the segmented control's beside it.
struct GraphicsMenu: View {
    @Bindable var model: RomViewModel

    var body: some View {
        Menu {
            ForEach(GraphicsModel.Tab.allCases) { tab in
                Button {
                    model.openGraphics(tab)
                } label: {
                    Label(tab.title, systemImage: tab.systemImage)
                }
            }
        } label: {
            Text(model.graphicsTab?.title ?? "Graphics")
                .padding(.horizontal, 8)
        }
        .menuStyle(.button)
        .controlSize(.large)
        .fixedSize()
        .help("Frame and Layers (from a recording), Tile Decoder (⌥⌘4), Palette (⌥⌘5), OAM (⌥⌘6) or Tilemap (⌥⌘7)")
    }
}
