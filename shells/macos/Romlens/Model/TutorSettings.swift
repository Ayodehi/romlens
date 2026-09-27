import Foundation
import Observation
import RomlensKit

/// An endpoint the tutor can talk to (docs/24): Anthropic's and OpenAI's,
/// which are built in, and any the student adds (Ollama, LM Studio,
/// LiteLLM…).
struct TutorEndpoint: Codable, Identifiable, Hashable, Sendable {
    enum Kind: String, Codable, CaseIterable, Sendable {
        case anthropic, responses, chat
        var title: String {
            switch self {
            case .anthropic: "Anthropic Messages"
            case .responses: "OpenAI Responses"
            case .chat: "Chat Completions"
            }
        }
    }

    var id: String
    var name: String
    var kind: Kind
    var baseURL: String
    var toolChoice: Bool
    var strict: Bool
    var vision: Bool
    /// Whether it needs a key (the two providers do; a local server may not).
    var needsKey: Bool
    var builtIn: Bool

    var info: TutorEndpointInfo {
        let p: TutorProtocol = switch kind {
        case .anthropic: .anthropic
        case .responses: .responses
        case .chat: .chat
        }
        return TutorEndpointInfo(id: id, protocol: p, baseUrl: baseURL, toolChoice: toolChoice, strict: strict, vision: vision)
    }

    static var builtIns: [TutorEndpoint] {
        tutorDefaultEndpoints().map { e in
            TutorEndpoint(
                id: e.id, name: e.id == "anthropic" ? "Anthropic" : "OpenAI",
                kind: e.protocol == .anthropic ? .anthropic : .responses,
                baseURL: e.baseUrl, toolChoice: e.toolChoice, strict: e.strict, vision: e.vision,
                needsKey: true, builtIn: true)
        }
    }

    /// A new endpoint of the student's, with an id nothing else has.
    static func local(name: String, baseURL: String, kind: Kind, taken: Set<String>) -> TutorEndpoint {
        var slug = name.lowercased().map { $0.isLetter || $0.isNumber ? $0 : "-" }.reduce(into: "") { $0.append($1) }
        slug = slug.trimmingCharacters(in: CharacterSet(charactersIn: "-"))
        if slug.isEmpty || slug == "anthropic" || slug == "openai" { slug = "local" }
        var id = slug
        var n = 2
        while taken.contains(id) {
            id = "\(slug)-\(n)"
            n += 1
        }
        return TutorEndpoint(
            id: id, name: name, kind: kind, baseURL: baseURL, toolChoice: false, strict: false,
            vision: false, needsKey: false, builtIn: false)
    }
}

enum TutorModePreference: String, Codable, CaseIterable, Sendable {
    case readOnly, askBeforeEdits, acceptEdits
    var title: String {
        switch self {
        case .readOnly: "Read-only"
        case .askBeforeEdits: "Ask before edits"
        case .acceptEdits: "Accept edits"
        }
    }
    var mode: TutorMode {
        switch self {
        case .readOnly: .readOnly
        case .askBeforeEdits: .askBeforeEdits
        case .acceptEdits: .acceptEdits
        }
    }
    init(_ m: TutorMode) {
        switch m {
        case .readOnly: self = .readOnly
        case .askBeforeEdits: self = .askBeforeEdits
        case .acceptEdits: self = .acceptEdits
        }
    }
}

/// The tutor's settings (docs/24, "Settings"). Everything but the keys is
/// in the user defaults; the keys are in the Keychain.
@MainActor @Observable
final class TutorSettings {
    struct Stored: Codable {
        var custom: [TutorEndpoint] = []
        var endpoint = "anthropic"
        /// The model last chosen for each endpoint.
        var models: [String: String] = [:]
        var efforts: [String: String] = [:]
        var mode = TutorModePreference.askBeforeEdits
        var costCap: Double? = 5
        var showThinking = true
        /// Where `generate_image` draws: an endpoint that speaks OpenAI's
        /// Images API, or none.
        var imageEndpoint: String?
        var imageModel = "gpt-image-1"

        init() {}

        init(from d: Decoder) throws {
            let c = try d.container(keyedBy: CodingKeys.self)
            custom = try c.decodeIfPresent([TutorEndpoint].self, forKey: .custom) ?? []
            endpoint = try c.decodeIfPresent(String.self, forKey: .endpoint) ?? "anthropic"
            models = try c.decodeIfPresent([String: String].self, forKey: .models) ?? [:]
            efforts = try c.decodeIfPresent([String: String].self, forKey: .efforts) ?? [:]
            mode = try c.decodeIfPresent(TutorModePreference.self, forKey: .mode) ?? .askBeforeEdits
            costCap = try c.decodeIfPresent(Double.self, forKey: .costCap)
            showThinking = try c.decodeIfPresent(Bool.self, forKey: .showThinking) ?? true
            imageEndpoint = try c.decodeIfPresent(String.self, forKey: .imageEndpoint)
            imageModel = try c.decodeIfPresent(String.self, forKey: .imageModel) ?? "gpt-image-1"
        }
    }

    static let defaultsKey = "Tutor.settings"
    static let shared = TutorSettings(defaults: .standard, keys: Keychain())

    let keys: any KeyStore
    private let defaults: UserDefaults
    private(set) var stored: Stored

    init(defaults: UserDefaults, keys: any KeyStore) {
        self.defaults = defaults
        self.keys = keys
        if let data = defaults.data(forKey: Self.defaultsKey),
           let s = try? JSONDecoder().decode(Stored.self, from: data) {
            stored = s
        } else {
            stored = Stored()
        }
    }

    private func save() {
        if let data = try? JSONEncoder().encode(stored) { defaults.set(data, forKey: Self.defaultsKey) }
    }

    var endpoints: [TutorEndpoint] { TutorEndpoint.builtIns + stored.custom }

    func endpoint(_ id: String) -> TutorEndpoint? { endpoints.first { $0.id == id } }

    var defaultEndpoint: TutorEndpoint {
        get { endpoint(stored.endpoint) ?? TutorEndpoint.builtIns[0] }
        set { stored.endpoint = newValue.id; save() }
    }

    /// The model for an endpoint: the one last chosen, or the table's
    /// default for its protocol.
    func model(for e: TutorEndpoint) -> String? {
        stored.models[e.id] ?? tutorDefaultModel(protocol: e.info.protocol)
    }

    func setModel(_ model: String?, for e: TutorEndpoint) {
        stored.models[e.id] = model
        save()
    }

    func effort(for e: TutorEndpoint) -> String? { stored.efforts[e.id] }

    func setEffort(_ effort: String?, for e: TutorEndpoint) {
        stored.efforts[e.id] = effort
        save()
    }

    var mode: TutorModePreference {
        get { stored.mode }
        set { stored.mode = newValue; save() }
    }

    var costCap: Double? {
        get { stored.costCap }
        set { stored.costCap = newValue.map { max(0, $0) }; save() }
    }

    var showThinking: Bool {
        get { stored.showThinking }
        set { stored.showThinking = newValue; save() }
    }

    /// The endpoint pictures are drawn at, if one is chosen and still there.
    var imageEndpoint: TutorEndpoint? {
        get { stored.imageEndpoint.flatMap(endpoint) }
        set { stored.imageEndpoint = newValue?.id; save() }
    }

    var imageModel: String {
        get { stored.imageModel }
        set { stored.imageModel = newValue; save() }
    }

    /// Endpoints that can draw: OpenAI's, and the student's own that speak
    /// OpenAI's protocols (Anthropic's API makes no pictures).
    var imageEndpoints: [TutorEndpoint] { endpoints.filter { $0.kind != .anthropic } }

    func add(_ e: TutorEndpoint) {
        stored.custom.append(e)
        save()
    }

    func update(_ e: TutorEndpoint) {
        guard let i = stored.custom.firstIndex(where: { $0.id == e.id }) else { return }
        stored.custom[i] = e
        save()
    }

    func remove(_ e: TutorEndpoint) {
        stored.custom.removeAll { $0.id == e.id }
        if stored.endpoint == e.id { stored.endpoint = "anthropic" }
        try? keys.setKey(nil, for: e.id)
        save()
    }

    func hasKey(_ e: TutorEndpoint) -> Bool { !(keys.key(for: e.id) ?? "").isEmpty }

    /// Ready to ask: a key where one is needed.
    func ready(_ e: TutorEndpoint) -> Bool { !e.needsKey || hasKey(e) }

    /// The table's models for an endpoint's protocol; a local server's come
    /// from asking it.
    func tableModels(for e: TutorEndpoint) -> [TutorModelInfo] {
        tutorModels().filter { $0.protocol == e.info.protocol }
    }
}
