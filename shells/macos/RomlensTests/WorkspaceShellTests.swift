import AppKit
import RomlensKit
import Testing
@testable import Romlens

/// The tab groups in a real window (docs/29, W3): splitting, tab bars,
/// views kept alive across tab switches, and the menu commands.
@MainActor
@Suite struct WorkspaceShellTests {
    private func model() async throws -> RomViewModel {
        let rom = try Rom.fromBytes(bytes: makeRoutinesTestRom(), name: "r.sfc")
        return try await Fixture.analyzedModel(rom: rom)
    }

    private func open(_ m: RomViewModel, size: NSSize = NSSize(width: 1440, height: 900)) throws -> (RomWindowController, NSView) {
        let controller = RomWindowController(model: m)
        controller.window?.setContentSize(size)
        controller.window?.orderFront(nil)
        let content = try #require(controller.window?.contentView)
        settle(content)
        return (controller, content)
    }

    private func settle(_ content: NSView) {
        for _ in 0..<3 {
            content.layoutSubtreeIfNeeded()
            Fixture.spin(0.05)
        }
        content.display()
    }

    private func all<T: NSView>(_ view: NSView, _: T.Type) -> [T] {
        var out: [T] = []
        if let t = view as? T { out.append(t) }
        for sub in view.subviews { out += all(sub, T.self) }
        return out
    }

    /// With ROMLENS_SNAPSHOTS set, the window is saved as a PNG in the
    /// sandbox's temporary folder, to look at.
    static func snapshot(_ content: NSView, _ name: String) throws {
        guard ProcessInfo.processInfo.environment["ROMLENS_SNAPSHOTS"] != nil,
              let rep = content.bitmapImageRepForCachingDisplay(in: content.bounds) else { return }
        content.cacheDisplay(in: content.bounds, to: rep)
        guard let png = rep.representation(using: .png, properties: [:]) else { return }
        try png.write(to: FileManager.default.temporaryDirectory.appendingPathComponent("workspace-\(name).png"))
        // The sandbox's folder is not readable from outside it; with
        // ROMLENS_SNAPSHOTS=log the PNG also goes to the log, to decode.
        if ProcessInfo.processInfo.environment["ROMLENS_SNAPSHOTS"] == "log" {
            print("ROMLENS-SNAPSHOT workspace-\(name).png \(png.base64EncodedString())")
        }
    }

    @Test func oneGroupToStartWithItsTabBar() async throws {
        let m = try await model()
        let (controller, content) = try open(m)
        defer { controller.window?.close() }
        let groups = all(content, TabGroupView.self)
        #expect(groups.count == 1)
        #expect(groups.first?.tabBar.tabs.map(\.title) == ["Hex"])
        try Self.snapshot(content, "single")
    }

    @Test func splittingMakesASecondGroupWithItsOwnView() async throws {
        let m = try await model()
        m.editorTab = .disassembly
        try await Fixture.settle { !m.navigator.labels.isEmpty }
        m.select(offset: 0x44)
        let routine = try #require(m.routineName(at: 0x008044), "the labels name the routine at $00:8044")
        let (controller, content) = try open(m)
        defer { controller.window?.close() }
        m.splitFocused(.right)
        settle(content)
        let groups = all(content, TabGroupView.self).filter { $0.window != nil }
        #expect(groups.count == 2)
        #expect(all(content, AsmCanvasView.self).filter { $0.window != nil }.count == 2)
        // The groups sit side by side, each about half the editor's width.
        let frames = groups.map { $0.convert($0.bounds, to: content) }.sorted { $0.minX < $1.minX }
        #expect(frames.count == 2 && abs(frames[0].width - frames[1].width) < 4)
        #expect(frames[0].maxX <= frames[1].minX + 1)
        // A code tab is named by its routine.
        #expect(groups.flatMap { $0.tabBar.tabs.map(\.title) } == [routine, routine])
        try Self.snapshot(content, "two-columns")
    }

    @Test func aTabsViewSurvivesSwitchingTabs() async throws {
        let m = try await model()
        m.editorTab = .disassembly
        let (controller, content) = try open(m)
        defer { controller.window?.close() }
        let code = try #require(m.workspace.focusedItem?.id)
        let canvas = try #require(all(content, AsmCanvasView.self).first)
        m.graphicsTab = .palette
        settle(content)
        #expect(canvas.isHiddenOrHasHiddenAncestor, "the code tab's view is kept, hidden")
        m.focus(item: code)
        settle(content)
        #expect(all(content, AsmCanvasView.self).first === canvas, "the same view, not a new one")
        #expect(!canvas.isHiddenOrHasHiddenAncestor)
    }

    @Test func theMenuCommands() async throws {
        let m = try await model()
        let (controller, content) = try open(m)
        defer { controller.window?.close() }
        m.graphicsTab = .tiles
        m.audioTab = .voices
        let group = m.workspace.focusedGroup
        #expect(m.workspace.layout.group(group)?.items.count == 3)
        controller.previousTab(nil)
        #expect(m.graphicsTab == .tiles)
        controller.nextTab(nil)
        controller.nextTab(nil)
        #expect(m.editorTab == .hex && m.showsTextEditor, "next wraps round to the first tab")
        controller.splitDown(nil)
        settle(content)
        #expect(m.workspace.layout.groups.count == 2)
        let tag0 = NSMenuItem(); tag0.tag = 0
        controller.focusGroupItem(tag0)
        #expect(m.workspace.focusedGroup == group)
        controller.closeTab(nil)
        #expect(m.workspace.layout.group(group)?.items.count == 2)
        let three = NSMenuItem(); three.tag = LayoutPreset.allCases.firstIndex(of: .three)!
        controller.applyLayout(three)
        settle(content)
        #expect(all(content, TabGroupView.self).filter { $0.window != nil }.count == 3)
        try Self.snapshot(content, "three")
    }

    @Test func theToolbarHoldsCommandsOnly() async throws {
        let m = try await model()
        let (controller, _) = try open(m)
        defer { controller.window?.close() }
        let toolbar = try #require(controller.window?.toolbar)
        let labels = toolbar.items.map(\.label).filter { !$0.isEmpty }
        for gone in ["Editor", "Graphics", "Audio", "Tutor", "Focus on Code"] {
            #expect(!labels.contains(gone), "\(gone) is still in the toolbar: \(labels)")
        }
    }

    @Test func whereADragWouldLand() async throws {
        let m = try await model()
        let (controller, content) = try open(m)
        defer { controller.window?.close() }
        m.graphicsTab = .palette
        settle(content)
        let grid = try #require(all(content, EditorGridNSView.self).first)
        let group = try #require(all(content, TabGroupView.self).first)
        let view = group.content.convert(group.content.bounds, to: grid)
        // The right edge: the right half previews.
        let right = try #require(grid.dropTarget(at: NSPoint(x: view.maxX - 10, y: view.midY)))
        #expect(right.target == .zone(.edge(.right)))
        #expect(abs(right.preview.minX - (view.midX + 4)) < 1 && abs(right.preview.maxX - (view.maxX - 4)) < 1)
        // The top: the top half (this view is flipped, so minY is the top).
        let top = try #require(grid.dropTarget(at: NSPoint(x: view.midX, y: view.minY + 10)))
        #expect(top.target == .zone(.edge(.top)) && top.preview.maxY < view.midY + 1)
        // The middle: the whole view.
        #expect(grid.dropTarget(at: NSPoint(x: view.midX, y: view.midY))?.target == .zone(.center))
        // The tab bar, right of both tabs: inserted at the end.
        let bar = group.tabBar.convert(group.tabBar.bounds, to: grid)
        #expect(grid.dropTarget(at: NSPoint(x: bar.maxX - 80, y: bar.midY))?.target == .tabBar(index: 2))
        #expect(grid.dropTarget(at: NSPoint(x: bar.minX + 2, y: bar.midY))?.target == .tabBar(index: 0))
    }
}
