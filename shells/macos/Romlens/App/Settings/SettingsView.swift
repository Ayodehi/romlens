import AppKit
import RomlensKit
import SwiftUI

/// Settings (⌘,): the tutor's providers and their keys, its defaults, and
/// what it sends (docs/24, "Settings").
struct SettingsView: View {
    @Bindable var settings: TutorSettings

    var body: some View {
        TabView {
            ProvidersPane(settings: settings)
                .tabItem { Label("Providers", systemImage: "network") }
            TutorPane(settings: settings)
                .tabItem { Label("Tutor", systemImage: "graduationcap") }
            ImagesPane(settings: settings)
                .tabItem { Label("Images", systemImage: "photo") }
            PrivacyPane()
                .tabItem { Label("Privacy", systemImage: "hand.raised") }
        }
        .frame(width: 620, height: 460)
    }
}

/// The endpoints, each with its key, a Test, and for the student's own,
/// where and how to reach it.
struct ProvidersPane: View {
    @Bindable var settings: TutorSettings
    @State private var selection: String? = "anthropic"
    @State private var adding = false

    var body: some View {
        HSplitView {
            VStack(spacing: 0) {
                List(selection: $selection) {
                    ForEach(settings.endpoints) { e in
                        HStack {
                            Text(e.name)
                            Spacer()
                            if settings.ready(e) {
                                Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
                                    .help(e.needsKey ? "A key is in the Keychain" : "Needs no key")
                            } else {
                                Image(systemName: "key").foregroundStyle(.secondary).help("No key yet")
                            }
                        }
                        .tag(e.id)
                    }
                }
                HStack {
                    Button { adding = true } label: { Image(systemName: "plus") }
                        .help("Add an endpoint on your own machine or network")
                    Button {
                        if let e = selected, !e.builtIn { settings.remove(e); selection = "anthropic" }
                    } label: { Image(systemName: "minus") }
                        .disabled(selected?.builtIn ?? true)
                        .help("Remove this endpoint and its key")
                    Spacer()
                }
                .buttonStyle(.borderless)
                .padding(6)
            }
            .frame(minWidth: 170, maxWidth: 200)
            Group {
                if let e = selected {
                    EndpointDetail(settings: settings, endpoint: e).id(e.id)
                } else {
                    Text("Choose a provider").foregroundStyle(.secondary)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
        .sheet(isPresented: $adding) {
            AddEndpointSheet(settings: settings) { selection = $0 }
        }
    }

    private var selected: TutorEndpoint? { selection.flatMap(settings.endpoint) }
}

struct EndpointDetail: View {
    let settings: TutorSettings
    @State var endpoint: TutorEndpoint
    @State private var key = ""
    @State private var status: String?
    @State private var models: [String] = []
    @State private var testing = false

    var body: some View {
        Form {
            if endpoint.builtIn {
                LabeledContent("Endpoint", value: endpoint.baseURL)
            } else {
                TextField("Name", text: $endpoint.name)
                TextField("Base URL", text: $endpoint.baseURL, prompt: Text("http://localhost:11434/v1"))
                Picker("Protocol", selection: $endpoint.kind) {
                    Text(TutorEndpoint.Kind.chat.title).tag(TutorEndpoint.Kind.chat)
                    Text(TutorEndpoint.Kind.responses.title).tag(TutorEndpoint.Kind.responses)
                }
                Toggle("Its models see pictures", isOn: $endpoint.vision)
                Toggle("It takes tool_choice", isOn: $endpoint.toolChoice)
                Toggle("It needs a key", isOn: $endpoint.needsKey)
            }
            if endpoint.needsKey || endpoint.builtIn {
                SecureField("API key", text: $key, prompt: Text(settings.hasKey(endpoint) ? "Kept in the Keychain" : "Paste your key"))
                HStack {
                    Button("Save Key") { saveKey() }.disabled(key.isEmpty)
                    Button("Remove Key") { removeKey() }.disabled(!settings.hasKey(endpoint))
                }
            }
            HStack {
                Button(testing ? "Testing…" : "Test") { test() }.disabled(testing)
                if let status { Text(status).font(.caption).foregroundStyle(.secondary).lineLimit(2) }
            }
            if !models.isEmpty {
                Text("\(models.count) models: \(models.prefix(8).joined(separator: ", "))\(models.count > 8 ? "…" : "")")
                    .font(.caption).foregroundStyle(.secondary).lineLimit(3)
            }
        }
        .formStyle(.grouped)
        .onChange(of: endpoint) { _, e in if !e.builtIn { settings.update(e) } }
    }

    private func saveKey() {
        do {
            try settings.keys.setKey(key, for: endpoint.id)
            key = ""
            status = "Saved in the Keychain."
        } catch { status = error.localizedDescription }
    }

    private func removeKey() {
        do {
            try settings.keys.setKey(nil, for: endpoint.id)
            status = "Removed."
        } catch { status = error.localizedDescription }
    }

    private func test() {
        testing = true
        status = nil
        let info = endpoint.info
        let key = settings.keys.key(for: endpoint.id)
        Task.detached {
            let result = Result { try tutorListModels(endpoint: info, key: key) }
            await MainActor.run {
                testing = false
                switch result {
                case .success(let ids):
                    models = ids
                    status = ids.isEmpty ? "Reached it, but it lists no models." : "It answers."
                case .failure(let e):
                    models = []
                    status = e.localizedDescription
                }
            }
        }
    }
}

struct AddEndpointSheet: View {
    let settings: TutorSettings
    let added: (String) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var name = "Ollama"
    @State private var url = "http://localhost:11434/v1"
    @State private var kind = TutorEndpoint.Kind.chat

    var body: some View {
        Form {
            TextField("Name", text: $name)
            TextField("Base URL", text: $url)
            Picker("Protocol", selection: $kind) {
                Text(TutorEndpoint.Kind.chat.title).tag(TutorEndpoint.Kind.chat)
                Text(TutorEndpoint.Kind.responses.title).tag(TutorEndpoint.Kind.responses)
            }
            Text("Ollama, LM Studio, vLLM and LiteLLM all speak Chat Completions; Ollama from 0.13.3, LM Studio and LiteLLM speak Responses too.")
                .font(.caption).foregroundStyle(.secondary)
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                Button("Add") {
                    let e = TutorEndpoint.local(name: name, baseURL: url, kind: kind, taken: Set(settings.endpoints.map(\.id)))
                    settings.add(e)
                    added(e.id)
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
                .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty || URL(string: url)?.scheme == nil)
            }
        }
        .formStyle(.grouped)
        .frame(width: 420)
    }
}

/// The tutor's defaults for a new conversation.
struct TutorPane: View {
    @Bindable var settings: TutorSettings

    var body: some View {
        let e = settings.defaultEndpoint
        Form {
            Picker("Provider", selection: Binding(get: { e.id }, set: { id in
                if let n = settings.endpoint(id) { settings.defaultEndpoint = n }
            })) {
                ForEach(settings.endpoints) { Text($0.name).tag($0.id) }
            }
            let table = settings.tableModels(for: e)
            if table.isEmpty {
                TextField("Model", text: Binding(get: { settings.model(for: e) ?? "" }, set: { settings.setModel($0.isEmpty ? nil : $0, for: e) }),
                          prompt: Text("as the server names it, e.g. qwen3:32b"))
            } else {
                Picker("Model", selection: Binding(get: { settings.model(for: e) ?? "" }, set: { settings.setModel($0, for: e) })) {
                    ForEach(table, id: \.id) { m in
                        Text("\(m.name)  $\(price(m.priceInput))/$\(price(m.priceOutput))").tag(m.id)
                    }
                }
            }
            let efforts = table.first { $0.id == settings.model(for: e) }?.efforts ?? []
            if !efforts.isEmpty {
                Picker("Effort", selection: Binding(get: { settings.effort(for: e) ?? "" }, set: { settings.setEffort($0.isEmpty ? nil : $0, for: e) })) {
                    Text("The model's default").tag("")
                    ForEach(efforts, id: \.self) { Text($0).tag($0) }
                }
            }
            Picker("Edits", selection: $settings.mode) {
                ForEach(TutorModePreference.allCases, id: \.self) { Text($0.title).tag($0) }
            }
            Text("Shift-Tab in the Tutor window changes it for a conversation.").font(.caption).foregroundStyle(.secondary)
            LabeledContent("Stop a conversation at") {
                HStack(spacing: 2) {
                    Text("$")
                    TextField("", value: $settings.costCap, format: .number.precision(.fractionLength(2)))
                        .labelsHidden()
                        .frame(width: 70)
                        .multilineTextAlignment(.trailing)
                }
                .help("Dollars; empty for no limit")
            }
            Toggle("Show thinking and tool calls", isOn: $settings.showWork)
            Text("Details in the Tutor window's status line, or /details, switches it there too.").font(.caption).foregroundStyle(.secondary)
            Toggle("Check each lesson in the background", isOn: Binding(
                get: { settings.checkLessons },
                set: { settings.checkLessons = $0; TutorModel.checkLessonsChanged($0) }))
            Text("Once a lesson ends, the same model checks it against the ROM and corrects its steps. What it costs is shown on the lesson.").font(.caption).foregroundStyle(.secondary)
        }
        .formStyle(.grouped)
    }

    private func price(_ p: Double) -> String { p < 1 ? String(format: "%.2f", p) : String(format: "%g", p) }
}

/// Where the tutor draws pictures, when asked for one.
struct ImagesPane: View {
    @Bindable var settings: TutorSettings

    var body: some View {
        Form {
            Picker("Draw with", selection: Binding(get: { settings.imageEndpoint?.id ?? "" }, set: { id in
                settings.imageEndpoint = id.isEmpty ? nil : settings.endpoint(id)
            })) {
                Text("No pictures").tag("")
                ForEach(settings.imageEndpoints) { Text($0.name).tag($0.id) }
            }
            TextField("Model", text: $settings.imageModel, prompt: Text("gpt-image-1"))
                .disabled(settings.imageEndpoint == nil)
            Text("The tutor sends the image model a description in words only, never a picture from the game, and says its pictures are generated. OpenAI's Images API, or an endpoint that speaks it (LiteLLM, a local server).")
                .font(.caption).foregroundStyle(.secondary)
        }
        .formStyle(.grouped)
    }
}

/// What leaves the machine (`12-content-policy.md` rule 8).
struct PrivacyPane: View {
    var body: some View {
        Form {
            Section("What the tutor sends") {
                Text("Only when you ask a question, and only to the provider you chose: your question, what you have selected in Romlens, the pictures you attach, and what the tutor's tools read for it (listings, bytes, C, pictures of tiles and frames). Never the whole ROM or a recording.")
                Text("An endpoint on your own machine sends nothing off it.")
                Text("Image generation is sent only a description in words, never a picture from the game.")
            }
            Section("What stays here") {
                Text("Your keys are in the Keychain. Conversations, with the pictures in them, are in Romlens's own folder, never in a project you share, and nothing offers them for export.")
            }
        }
        .formStyle(.grouped)
    }
}
