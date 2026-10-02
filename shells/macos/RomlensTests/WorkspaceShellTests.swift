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
        m.showTab(.disassembly)
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
        // Two Disassembly tabs: each named by its view, and by the routine
        // to tell them apart.
        #expect(groups.flatMap { $0.tabBar.tabs.map(\.title) }.filter { $0.hasPrefix("Disassembly") } == ["Disassembly · \(routine)", "Disassembly · \(routine)"])
        try Self.snapshot(content, "two-columns")
    }

    @Test func aTabsViewSurvivesSwitchingTabs() async throws {
        let m = try await model()
        m.showTab(.disassembly)
        let (controller, content) = try open(m)
        defer { controller.window?.close() }
        let code = try #require(m.workspace.focusedItem?.id)
        let canvas = try #require(all(content, AsmCanvasView.self).first)
        m.show(.graphics(.palette))
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
        m.show(.graphics(.tiles))
        m.show(.audio(.voices))
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
        m.showTab(.disassembly)
        m.jump(to: 0x44)
        if let frame = controller.window?.contentView?.superview {
            settle(frame)
            try Self.snapshot(frame, "window")
        }
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
        m.show(.graphics(.palette))
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

    @Test func theStatusBarIsUnderTheGroupsAndFocusHidesIt() async throws {
        let m = try await model()
        let (controller, content) = try open(m)
        defer { controller.window?.close() }
        let grid = try #require(all(content, EditorGridNSView.self).first)
        let strip = try #require(all(content, RegionStripNSView.self).first)
        let g = grid.convert(grid.bounds, to: nil)
        let s = strip.convert(strip.bounds, to: nil)
        // Window coordinates: y grows upward, so under means lower.
        #expect(s.maxY <= g.minY + 1, "the strip is under the tab groups: \(s) \(g)")
        #expect(s.height < 14, "a slim strip")
        m.toggleFocus()
        settle(content)
        #expect(all(content, RegionStripNSView.self).allSatisfy { $0.window == nil })
        m.toggleFocus()
    }

    @Test func theTutorShowsInTheDrawer() async throws {
        let m = try await model()
        let (controller, content) = try open(m)
        defer { controller.window?.close() }
        m.isInspectorVisible = false
        controller.showTutor(nil)
        settle(content)
        #expect(m.rightPane == .tutor && m.isInspectorVisible && m.tutor != nil)
        if let frame = content.superview { try Self.snapshot(frame, "drawer-tutor") }
        m.rightPane = .inspector
        settle(content)
    }

    @Test func theTutorAsATabBesideTheCode() async throws {
        let m = try await model()
        let (controller, content) = try open(m)
        defer { controller.window?.close() }
        m.showTutorTab()
        _ = m.tutor?.follow(URL(string: "romlens://a/008040")!)
        settle(content)
        #expect(all(content, TabGroupView.self).filter { $0.window != nil }.count == 2)
        if let frame = content.superview { try Self.snapshot(frame, "tutor-tab") }
    }

    @Test func aNarrowWindowShowsTheFocusedGroupOnly() async throws {
        let m = try await model()
        let (controller, content) = try open(m)
        defer { controller.window?.close() }
        m.show(.graphics(.palette))
        m.splitFocused(.right)
        settle(content)
        #expect(all(content, TabGroupView.self).filter { $0.window != nil }.count == 2)
        controller.window?.setContentSize(NSSize(width: 1000, height: 700))
        settle(content)
        let shown = all(content, TabGroupView.self).filter { $0.window != nil }
        #expect(shown.count == 1)
        #expect(shown.first?.groupID == m.workspace.focusedGroup)
        #expect(m.workspace.layout.groups.count == 2, "the layout is kept")
        try Self.snapshot(content, "narrow")
        controller.window?.setContentSize(NSSize(width: 1440, height: 900))
        settle(content)
        #expect(all(content, TabGroupView.self).filter { $0.window != nil }.count == 2)
    }

    @Test func aNewWindowOpensAtItsOwnSize() async throws {
        let m = try await model()
        let controller = RomWindowController(model: m)
        controller.window?.orderFront(nil)
        defer { controller.window?.close() }
        let content = try #require(controller.window?.contentView)
        settle(content)
        let size = try #require(controller.window?.contentLayoutRect.size)
        print("ROMLENS-SIZE \(size)")
        #expect(size.width > 1200, "opened at \(size)")
    }

    /// Many tabs in one group: squeezed, then scrolled so the shown one is
    /// in view, with every tab in the overflow menu (the user's report, 2
    /// October 2026: the newest tab was drawn past the edge, so its view
    /// seemed to replace another's).
    @Test func aCrowdedTabBarKeepsTheShownTabInView() async throws {
        let m = try await model()
        let (controller, content) = try open(m)
        defer { controller.window?.close() }
        for t in GraphicsModel.Tab.allCases { m.openGraphics(t) }
        for t in AudioModel.Tab.allCases { m.openAudio(t) }
        settle(content)
        let bar = try #require(all(content, TabGroupView.self).first?.tabBar)
        #expect(bar.tabs.count == 14)
        #expect(bar.isOverflowing)
        let shown = try #require(bar.tabs.first { $0.item.id == m.workspace.focusedItem?.id })
        #expect(shown.rect.minX >= -0.5 && shown.rect.maxX <= bar.available + 0.5, "the shown tab is in view: \(shown.rect) of \(bar.available)")
        // Squeezed no narrower than a short tab's own width.
        #expect(bar.tabs.allSatisfy { $0.rect.width >= 83.5 })
        // Back to the first: the bar scrolls back.
        m.focus(item: bar.tabs[0].item.id)
        settle(content)
        #expect(abs(bar.tabs[0].rect.minX) < 0.5)
        try Self.snapshot(content, "crowded")
    }
}
