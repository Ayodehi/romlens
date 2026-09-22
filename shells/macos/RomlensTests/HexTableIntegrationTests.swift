import AppKit
import Foundation
import RomlensKit
import Testing
@testable import Romlens

/// Drives the real window and hex canvas off screen. Uses the development
/// ROM when `ROMLENS_ROM_DIR` is set, else a padded fixture (see `Fixture`).
@MainActor
@Suite struct HexTableIntegrationTests {
    @Test func windowShowsEveryRow() throws {
        let model = RomViewModel(rom: try Fixture.rom(), startAnalysis: false)
        let (controller, scrollView, canvas) = try Fixture.window(model)
        let content = try #require(controller.window?.contentView)
        #expect(canvas.rowCount == Int(model.rowCount))
        #expect(canvas.bounds.height == CGFloat(model.rowCount) * model.layout.rowHeight)
        #expect(canvas.bounds.width >= model.layout.totalWidth - 1)
        // The canvas is as wide as the text or the viewport, whichever is
        // larger, so sideways scrolling exists only when the text does not fit.
        let clipWidth = scrollView.contentView.bounds.width
        #expect(canvas.bounds.width == max(model.layout.totalWidth, clipWidth))
        #expect(scrollView.horizontalScrollElasticity == .none)
        // Widen the window: the canvas must follow the viewport exactly.
        controller.window?.setContentSize(NSSize(width: 2000, height: 720))
        content.layoutSubtreeIfNeeded()
        Fixture.spin(0.05)
        content.layoutSubtreeIfNeeded()
        #expect(scrollView.contentView.bounds.width > model.layout.totalWidth)
        #expect(canvas.bounds.width == scrollView.contentView.bounds.width)
        #expect(canvas.subviews.isEmpty)
        #expect(canvas.visibleRows.count > 10)
        #expect(canvas.visibleRows.lowerBound == 0)

        // Hit testing: byte 3 of row 2, and the gap between hex and ASCII.
        let layout = model.layout
        let hit = NSPoint(x: layout.x(ofChar: layout.hexColumn(byte: 3)) + 1, y: 2 * layout.rowHeight + 3)
        let hitByte: UInt32? = canvas.byte(at: hit)
        #expect(hitByte == UInt32(2 * 16 + 3))
        #expect(canvas.byte(at: NSPoint(x: 2, y: 5)) == nil)

        // Jump lands the header row in view and selects it.
        model.jump(to: model.info.headerOffset)
        Fixture.spin(0.05)
        let headerRow = Int(model.info.headerOffset / 16)
        #expect(canvas.visibleRows.contains(headerRow))
        #expect(model.selectedOffset == model.info.headerOffset)
        #expect(scrollView.contentView.bounds.origin.y > 0)
        controller.window?.close()
    }

    @Test func drawingTheWholeImageStaysUnderBudget() throws {
        let model = RomViewModel(rom: try Fixture.rom(megabytes: 3), startAnalysis: false)
        let (controller, _, canvas) = try Fixture.window(model)
        let rows = canvas.rowCount
        let stride = 40 // rows per "frame"
        let frameHeight = CGFloat(stride) * canvas.rowHeight
        let bitmap = NSBitmapImageRep(
            bitmapDataPlanes: nil, pixelsWide: Int(canvas.bounds.width), pixelsHigh: Int(frameHeight),
            bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
            colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0
        )!
        var worstFrame: TimeInterval = 0
        let start = Date()
        var row = 0
        while row < rows {
            let frameStart = Date()
            NSGraphicsContext.saveGraphicsState()
            let context = NSGraphicsContext(bitmapImageRep: bitmap)!
            NSGraphicsContext.current = context
            context.cgContext.translateBy(x: 0, y: -CGFloat(row) * canvas.rowHeight)
            canvas.draw(NSRect(x: 0, y: CGFloat(row) * canvas.rowHeight, width: canvas.bounds.width, height: frameHeight))
            NSGraphicsContext.restoreGraphicsState()
            worstFrame = max(worstFrame, Date().timeIntervalSince(frameStart))
            row += stride
        }
        let total = Date().timeIntervalSince(start)
        print("hex canvas: \(rows) rows in \(String(format: "%.0f", total * 1000)) ms, \(String(format: "%.1f", total / Double(rows) * 1e6)) µs/row, worst 40-row frame \(String(format: "%.2f", worstFrame * 1000)) ms, \(model.cache.missCount) batch misses")
        // The 8.3 ms frame budget is the number to care about, and the line
        // printed above is where it is checked — by reading it, and by the
        // Instruments pass docs/15 still lists. It cannot be *asserted* here:
        // Swift Testing runs suites in parallel, so a worst-frame measurement
        // competes with whatever else is analyzing a ROM at the time, and the
        // assertion would fail on a busy machine while passing on an idle one.
        // What the bound below catches is a structural regression — the
        // NSTableView row-view leak this test was written for cost hundreds of
        // times the budget, not six.
        #expect(worstFrame < 0.100, "worst frame \(worstFrame * 1000) ms")
        #expect(model.cache.missCount == (rows + 255) / 256)
        controller.window?.close()
    }

    /// Scrolls the live scroll view through the whole image, page by page,
    /// spinning the run loop after each step so AppKit draws as it would for
    /// a user scroll, and checks that nothing accumulates.
    @Test func liveScrollHasNoStalls() throws {
        let model = RomViewModel(rom: try Fixture.rom(megabytes: 3), startAnalysis: false)
        let (controller, scrollView, canvas) = try Fixture.window(model)
        let clip = scrollView.contentView
        let pageHeight = clip.bounds.height
        var y: CGFloat = 0
        var steps = 0
        var worst: TimeInterval = 0
        let start = Date()
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
        print("live scroll: \(steps) pages of \(Int(pageHeight)) pt in \(Int(Date().timeIntervalSince(start) * 1000)) ms, worst step \(String(format: "%.1f", worst * 1000)) ms, subviews \(canvas.subviews.count)")
        #expect(worst < 0.1, "worst step \(worst * 1000) ms")
        #expect(canvas.subviews.isEmpty)
        clip.scroll(to: NSPoint(x: 0, y: canvas.bounds.height - pageHeight))
        scrollView.reflectScrolledClipView(clip)
        canvas.displayIfNeeded()
        #expect(canvas.visibleRows.upperBound == canvas.rowCount)
        controller.window?.close()
    }

    /// The same through scroll-wheel events, the path a mouse wheel takes.
    @Test func wheelScrollWorks() throws {
        let model = RomViewModel(rom: try Fixture.rom(megabytes: 3), startAnalysis: false)
        let (controller, scrollView, canvas) = try Fixture.window(model)
        var worst: TimeInterval = 0
        for _ in 0..<2000 {
            let cg = try #require(CGEvent(scrollWheelEvent2Source: nil, units: .line, wheelCount: 1, wheel1: -40, wheel2: 0, wheel3: 0))
            let ev = try #require(NSEvent(cgEvent: cg))
            let t0 = Date()
            scrollView.scrollWheel(with: ev)
            Fixture.spin(0)
            worst = max(worst, Date().timeIntervalSince(t0))
        }
        let y = scrollView.contentView.bounds.origin.y
        print("wheel scroll: reached y=\(Int(y)) of \(Int(canvas.bounds.height)), worst event \(String(format: "%.1f", worst * 1000)) ms")
        #expect(y > 10_000)
        #expect(canvas.visibleRows.lowerBound > 0)
        #expect(worst < 0.1)
        controller.window?.close()
    }
}
