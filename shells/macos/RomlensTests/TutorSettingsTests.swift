import AppKit
import Foundation
import RomlensKit
import Testing
@testable import Romlens

/// The tutor's settings (docs/24, U7): kept in the defaults with the keys
/// apart, endpoints added and removed, the Settings window laid out.
@MainActor
@Suite(.serialized) struct TutorSettingsTests {
    private func fresh() -> (TutorSettings, UserDefaults, MemoryKeyStore) {
        let name = "romlens-tutor-settings-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        defaults.removePersistentDomain(forName: name)
        let keys = MemoryKeyStore()
        return (TutorSettings(defaults: defaults, keys: keys), defaults, keys)
    }

    @Test func theDefaultsStartOnClaudeAndAskBeforeEdits() {
        let (s, _, _) = fresh()
        #expect(s.endpoints.map(\.id) == ["anthropic", "openai"])
        #expect(s.defaultEndpoint.id == "anthropic")
        #expect(s.model(for: s.defaultEndpoint) == tutorDefaultModel(protocol: .anthropic))
        #expect(s.mode == .askBeforeEdits)
        #expect(!s.ready(s.defaultEndpoint), "no key yet")
    }

    @Test func settingsComeBackAndKeysStayOutOfTheDefaults() throws {
        let (s, defaults, keys) = fresh()
        let local = TutorEndpoint.local(name: "Ollama", baseURL: "http://localhost:11434/v1", kind: .chat, taken: Set(s.endpoints.map(\.id)))
        #expect(local.id == "ollama")
        s.add(local)
        s.defaultEndpoint = local
        s.setModel("qwen3:32b", for: local)
        s.mode = .acceptEdits
        s.costCap = 2.5
        try keys.setKey("sk-secret", for: "anthropic")
        let again = TutorSettings(defaults: defaults, keys: keys)
        #expect(again.defaultEndpoint.id == "ollama")
        #expect(again.model(for: local) == "qwen3:32b")
        #expect(again.mode == .acceptEdits && again.costCap == 2.5)
        #expect(again.ready(local), "a local endpoint needs no key")
        #expect(again.hasKey(again.endpoint("anthropic")!))
        let raw = String(decoding: defaults.data(forKey: TutorSettings.defaultsKey) ?? Data(), as: UTF8.self)
        #expect(!raw.contains("sk-secret"))
        let second = TutorEndpoint.local(name: "Ollama", baseURL: "http://other:11434/v1", kind: .chat, taken: Set(again.endpoints.map(\.id)))
        #expect(second.id == "ollama-2")
        again.remove(local)
        #expect(again.defaultEndpoint.id == "anthropic")
        #expect(again.endpoint("ollama") == nil)
    }

    @Test func theKeychainKeepsAKey() throws {
        let k = Keychain()
        let account = "romlens-test-\(UUID().uuidString)"
        defer { try? k.setKey(nil, for: account) }
        do {
            try k.setKey("sk-one", for: account)
        } catch {
            // A test host without a keychain it may write to.
            return
        }
        #expect(k.key(for: account) == "sk-one")
        try k.setKey("sk-two", for: account)
        #expect(k.key(for: account) == "sk-two")
        try k.setKey(nil, for: account)
        #expect(k.key(for: account) == nil)
        #expect(TutorCredentials(store: MemoryKeyStore()).key(endpoint: "anthropic") == nil)
    }

    @Test func theSettingsWindowLaysOut() throws {
        let (s, _, _) = fresh()
        let c = SettingsWindowController(settings: s)
        let w = try #require(c.window)
        w.orderFront(nil)
        defer { w.close() }
        w.contentView?.layoutSubtreeIfNeeded()
        Fixture.spin(0.2)
        #expect(w.frame.width >= 600)
        #expect(w.title == "Settings")
    }

    @Test func theMenuHasSettings() {
        let menu = MainMenu.build()
        let app = menu.items.first?.submenu
        let item = app?.items.first { $0.title == "Settings…" }
        #expect(item?.keyEquivalent == ",")
        #expect(item?.action == #selector(AppDelegate.showSettings(_:)))
    }
}

/// A file the core stopped writing does not come back on save.
@MainActor
@Suite struct OptionalFilesTests {
    @Test func aRemovedNoteLeavesThePackage() throws {
        let doc = ProjectDocument()
        try doc.read(from: makeTestRom(mapping: .loRom), ofType: Fixture.romType)
        let model = try #require(doc.model)
        model.session.cancelAnalysis()
        try model.session.workbench.executeBatch(commands: [.setRoutineNote(routine: 0x00_8000, text: "Boots.")], origin: .user)
        let first = try doc.fileWrapper(ofType: Fixture.projectType)
        #expect(first.fileWrappers?["c_notes.json"] != nil)
        try model.session.workbench.executeBatch(commands: [.setRoutineNote(routine: 0x00_8000, text: nil)], origin: .user)
        let second = try doc.fileWrapper(ofType: Fixture.projectType)
        #expect(second.fileWrappers?["c_notes.json"] == nil)
    }
}
