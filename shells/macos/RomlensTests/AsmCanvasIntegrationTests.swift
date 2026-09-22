import AppKit
import Foundation
import RomlensKit
import Testing
@testable import Romlens

/// The disassembly canvas in a real window, mirroring the hex suite.
@MainActor
@Suite struct AsmCanvasIntegrationTests {
    @Test func windowShowsLinesAndHitTests() async throws {
        let model = try await Fixture.analyzedModel(rom: try Fixture.smallRom())
        let (controller, scrollView, canvas) = try Fixture.asmWindow(model)
        #expect(canvas.lineCount == Int(model.asmLineCount))
        #expect(canvas.bounds.height == CGFloat(model.asmLineCount) * model.asmLayout.rowHeight)
        #expect(canvas.bounds.width == max(model.asmLayout.totalWidth, scrollView.contentView.bounds.width))
        #expect(canvas.subviews.isEmpty)
        #expect(canvas.visibleLines.lowerBound == 0 && canvas.visibleLines.count > 10)
        // The SEI line: find it, hit test its mnemonic token.
        let seiLine = try #require(model.workbench.lineForOffset(fileOffset: 0))
        let layout = model.asmLayout
        let y = CGFloat(seiLine) * layout.rowHeight + 3
        let point = NSPoint(x: layout.x(ofChar: layout.textColumn + 1), y: y)
        #expect(canvas.line(at: point) == seiLine)
        let hit = try #require(canvas.token(at: point))
        #expect(hit.0.text.hasPrefix("SEI") && hit.1.kind == .mnemonic)
        #expect(canvas.token(at: NSPoint(x: 1, y: y)) == nil)
        // Selecting a line by offset invalidates and scrolls.
        model.jump(to: 14)
        Fixture.spin(0.05)
        let rtiLine = try #require(model.workbench.lineForOffset(fileOffset: 14))
        #expect(canvas.visibleLines.contains(Int(rtiLine)))
        // `g` on the BRA follows it (to itself here), Return does the same.
        model.select(offset: 10)
        canvas.onKeyCommand?(.follow)
        #expect(model.selectedOffset == 10)
        controller.window?.close()
    }

    @Test func drawingEveryLineStaysUnderBudget() async throws {
        let model = try await Fixture.analyzedModel(rom: try Fixture.rom(megabytes: 3))
        let (controller, _, canvas) = try Fixture.asmWindow(model)
        let lines = canvas.lineCount
        let stride = 40
        let frameHeight = CGFloat(stride) * canvas.rowHeight
        let bitmap = NSBitmapImageRep(
            bitmapDataPlanes: nil, pixelsWide: Int(canvas.bounds.width), pixelsHigh: Int(frameHeight),
            bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
            colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0
        )!
        var worstFrame: TimeInterval = 0
        let start = Date()
        var line = 0
        while line < lines {
            let frameStart = Date()
            NSGraphicsContext.saveGraphicsState()
            let context = NSGraphicsContext(bitmapImageRep: bitmap)!
            NSGraphicsContext.current = context
            context.cgContext.translateBy(x: 0, y: -CGFloat(line) * canvas.rowHeight)
            canvas.draw(NSRect(x: 0, y: CGFloat(line) * canvas.rowHeight, width: canvas.bounds.width, height: frameHeight))
            NSGraphicsContext.restoreGraphicsState()
            worstFrame = max(worstFrame, Date().timeIntervalSince(frameStart))
            line += stride
        }
        let total = Date().timeIntervalSince(start)
        print("asm canvas: \(lines) lines in \(String(format: "%.0f", total * 1000)) ms, \(String(format: "%.1f", total / Double(lines) * 1e6)) µs/line, worst 40-line frame \(String(format: "%.2f", worstFrame * 1000)) ms, \(model.asmCache.missCount) batch misses")
        #expect(worstFrame < 0.008, "worst frame \(worstFrame * 1000) ms")
        #expect(model.asmCache.missCount == (lines + 255) / 256)
        controller.window?.close()
    }

    @Test func liveScrollAndWheelHaveNoStalls() async throws {
        let model = try await Fixture.analyzedModel(rom: try Fixture.rom(megabytes: 3))
        let (controller, scrollView, canvas) = try Fixture.asmWindow(model)
        let clip = scrollView.contentView
        let pageHeight = clip.bounds.height
        var y: CGFloat = 0
        var worst: TimeInterval = 0
        var steps = 0
        while y < canvas.bounds.height - pageHeight {
            let t0 = Date()
            clip.scroll(to: NSPoint(x: 0, y: y))
            scrollView.reflectScrolledClipView(clip)
            canvas.displayIfNeeded()
            Fixture.spin(0)
            worst = max(worst, Date().timeIntervalSince(t0))
            steps += 1
            y += pageHeight
        }
        print("asm live scroll: \(steps) pages, worst step \(String(format: "%.1f", worst * 1000)) ms, subviews \(canvas.subviews.count)")
        #expect(worst < 0.1, "worst step \(worst * 1000) ms")
        #expect(canvas.subviews.isEmpty)
        clip.scroll(to: .zero)
        scrollView.reflectScrolledClipView(clip)
        for _ in 0..<500 {
            let cg = try #require(CGEvent(scrollWheelEvent2Source: nil, units: .line, wheelCount: 1, wheel1: -40, wheel2: 0, wheel3: 0))
            let ev = try #require(NSEvent(cgEvent: cg))
            scrollView.scrollWheel(with: ev)
            Fixture.spin(0)
        }
        #expect(scrollView.contentView.bounds.origin.y > 1000)
        #expect(canvas.subviews.isEmpty)
        controller.window?.close()
    }
}

/// The Both tab: the pure mapper converges, and the live window keeps the
/// panes aligned with anchors inside their frames.
@MainActor
@Suite struct LockstepTests {
    @Test func mapperConvergesInBothDirections() async throws {
        let model = try await Fixture.analyzedModel(rom: try Fixture.smallRom())
        let wb = model.workbench
        for row in [0, 1, 5, 100, 1000] {
            let line = try #require(wb.lineForOffset(fileOffset: UInt32(row) * 16))
            let back = try #require(wb.offsetForLine(line: line))
            #expect(back / 16 <= UInt32(row))
            let again = try #require(wb.lineForOffset(fileOffset: back))
            #expect(again == line, "row \(row): line \(line) → offset \(back) → line \(again)")
        }
    }

    /// Rubber-band overscroll hands the sync a negative clip origin; it must
    /// clamp instead of trapping on the UInt32 conversion.
    @Test func overscrollDoesNotTrap() async throws {
        let model = try await Fixture.analyzedModel(rom: try Fixture.smallRom())
        model.editorTab = .both
        let controller = RomWindowController(model: model)
        controller.window?.orderFront(nil)
        let content = try #require(controller.window?.contentView)
        content.layoutSubtreeIfNeeded()
        Fixture.spin(0.1)
        let pane = try #require(Fixture.find(content, LockstepPaneView.self))
        let lockstep = LockstepController(model: model, hex: pane.hex, asm: pane.asm, pane: pane)
        #expect(lockstep.asmLine(forTopRow: -1) == lockstep.asmLine(forTopRow: 0))
        #expect(lockstep.asmLine(forTopRow: 1_000_000) != nil)
        #expect(lockstep.hexRow(forTopLine: -5) == 0)
        #expect(lockstep.hexRow(forTopLine: 1_000_000) != nil)
        // Drive the real clip views past both ends.
        for scroll in [pane.hex.scrollView, pane.asm.scrollView] {
            scroll.contentView.scroll(to: NSPoint(x: 0, y: -40))
            scroll.reflectScrolledClipView(scroll.contentView)
            Fixture.spin(0.02)
            let height = scroll.documentView?.bounds.height ?? 0
            scroll.contentView.scroll(to: NSPoint(x: 0, y: height + 40))
            scroll.reflectScrolledClipView(scroll.contentView)
            Fixture.spin(0.02)
        }
        controller.window?.close()
    }

    @Test func bothTabScrollsInLockstepWithABracket() async throws {
        let model = try await Fixture.analyzedModel(rom: try Fixture.rom(megabytes: 1))
        model.editorTab = .both
        let controller = RomWindowController(model: model)
        controller.window?.orderFront(nil)
        let content = try #require(controller.window?.contentView)
        content.layoutSubtreeIfNeeded()
        Fixture.spin(0.1)
        content.layoutSubtreeIfNeeded()
        let pane = try #require(Fixture.find(content, LockstepPaneView.self))
        let hexScroll = pane.hex.scrollView
        let asmScroll = pane.asm.scrollView
        #expect(pane.overlay.hitTest(NSPoint(x: 10, y: 10)) == nil)
        // Scroll hex to row 100: the asm top line covers byte 1600.
        let rowHeight = pane.hex.canvas.rowHeight
        hexScroll.contentView.scroll(to: NSPoint(x: 0, y: 100 * rowHeight))
        hexScroll.reflectScrolledClipView(hexScroll.contentView)
        Fixture.spin(0.05)
        let topLine = Int(floor(asmScroll.contentView.bounds.origin.y / pane.asm.canvas.rowHeight))
        let topOffset = try #require(model.workbench.offsetForLine(line: UInt32(topLine)))
        let expected = try #require(model.workbench.lineForOffset(fileOffset: 1600))
        #expect(topLine == Int(expected), "asm top line \(topLine) covers \(topOffset)")
        #expect(topOffset <= 1600 && topOffset + 16 > 1600 - 16)
        // And back: scroll asm, hex follows.
        asmScroll.contentView.scroll(to: NSPoint(x: 0, y: 400 * pane.asm.canvas.rowHeight))
        asmScroll.reflectScrolledClipView(asmScroll.contentView)
        Fixture.spin(0.05)
        let hexTopRow = Int(floor(hexScroll.contentView.bounds.origin.y / rowHeight))
        let asmTopOffset = try #require(model.workbench.offsetForLine(line: 400))
        #expect(hexTopRow == Int(asmTopOffset / 16))
        // Select something visible: anchors land inside their pane frames.
        model.jump(to: 1600)
        Fixture.spin(0.05)
        let bracket = try #require(pane.overlay.bracket)
        // Anchors are clamped onto the pane frame when the row or column is
        // out of view, so edge points count as inside.
        #expect(bracket.hexFrame.insetBy(dx: -0.5, dy: -0.5).contains(bracket.hexTop), "\(bracket)")
        #expect(bracket.asmFrame.insetBy(dx: -0.5, dy: -0.5).contains(bracket.asmTop), "\(bracket)")
        #expect(bracket.hexTop.x < bracket.asmTop.x, "hex is left of asm")
        // The mapping call is cheap enough for every draw and scroll.
        let start = Date()
        for i in 0..<1000 { _ = model.workbench.lineForOffset(fileOffset: UInt32(i * 16)) }
        let perCall = Date().timeIntervalSince(start) / 1000
        print("lineForOffset: \(String(format: "%.1f", perCall * 1e6)) µs per call")
        #expect(perCall < 50e-6, "\(perCall * 1e6) µs")
        controller.window?.close()
    }
}
