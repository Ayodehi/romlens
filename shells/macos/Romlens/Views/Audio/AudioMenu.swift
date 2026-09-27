import SwiftUI

/// The toolbar's Audio picker, beside the Graphics one and built the same
/// way (docs/23).
struct AudioMenu: View {
    @Bindable var model: RomViewModel

    var body: some View {
        Menu {
            ForEach(AudioModel.Tab.allCases) { tab in
                Button {
                    model.openAudio(tab)
                } label: {
                    Label(tab.title, systemImage: tab.systemImage)
                }
            }
        } label: {
            Text(model.audioTab?.title ?? "Audio")
                .padding(.horizontal, 8)
        }
        .menuStyle(.button)
        .controlSize(.large)
        .fixedSize()
        .help("Voices, Samples or Audio RAM: the sound CPU from a recording, or from the ROM's upload run by Romlens")
    }
}
