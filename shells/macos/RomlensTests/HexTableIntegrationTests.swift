import AppKit
import Foundation
import RomlensKit
import Testing
@testable import Romlens

/// Drives the real window and hex canvas off screen. Uses the development
/// ROM when `ROMLENS_ROM_DIR` is set, else a padded fixture.
@MainActor
enum Fixture {
    static func rom(megabytes: Int = 1) throws -> Rom {
        if let dir = ProcessInfo.processInfo.environment["ROMLENS_ROM_DIR"] {
            let url = URL(fileURLWithPath: dir).appendingPathComponent("SuperMetroid.F8DF.sfc")
            if FileManager.default.fileExists(atPath: url.path) {
                return try Rom.open(path: url.path)
            }
        }
        var bytes = makeTestRom(mapping: .loRom)
        bytes.append(Data(repeating: 0xEA, count: (megabytes << 20) - bytes.count))
        return try Rom.fromBytes(bytes: bytes, name: "big.sfc")
    }

    static func window(_ model: RomViewModel) throws -> (RomWindowController, NSScrollView, HexCanvasView) {
        let controller = RomWindowController(model: model)
        controller.window?.orderFront(nil)
        let content = try #require(controller.window?.contentView)
        content.layoutSubtreeIfNeeded()
        func find<T: NSView>(_ view: NSView, _: T.Type) -> T? {
            if let t = view as? T { return t }
            for sub in view.subviews { if let t = find(sub, T.self) { return t } }
            return nil
        }
        return (controller, try #require(find(content, NSScrollView.self)), try #require(find(content, HexCanvasView.self)))
    }
}

@MainActor
@Suite struct HexTableIntegrationTests {
    @Test func windowShowsEveryRow() throws {
        let model = RomViewModel(rom: try Fixture.rom())
        let (controller, scrollView, canvas) = try Fixture.window(model)
        #expect(canvas.rowCount == Int(model.rowCount))
        #expect(canvas.bounds.height == CGFloat(model.rowCount) * model.layout.rowHeight)
        #expect(canvas.bounds.width >= model.layout.totalWidth - 1)
        #expect(canvas.subviews.isEmpty)
        #expect(canvas.visibleRows.count > 10)
        #expect(canvas.visibleRows.lowerBound == 0)

        // Hit testing: byte 3 of row 2, and the gap between hex and ASCII.
        let layout = model.layout
        let hit = NSPoint(x: layout.x(ofChar: layout.hexColumn(byte: 3)) + 1, y: 2 * layout.rowHeight + 3)
        #expect(canvas.byte(at: hit) == 2 * 16 + 3)
        #expect(canvas.byte(at: NSPoint(x: 2, y: 5)) == nil)

        // Jump lands the header row in view and selects it.
        model.jump(to: model.info.headerOffset)
        RunLoop.main.run(until: Date().addingTimeInterval(0.05))
        let headerRow = Int(model.info.headerOffset / 16)
        #expect(canvas.visibleRows.contains(headerRow))
        #expect(model.selectedOffset == model.info.headerOffset)
        #expect(scrollView.contentView.bounds.origin.y > 0)
        controller.window?.close()
    }

    @Test func drawingTheWholeImageStaysUnderBudget() throws {
        let model = RomViewModel(rom: try Fixture.rom(megabytes: 3))
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
        // A 120 Hz frame is 8.3 ms; drawing 40 fresh rows must fit with room
        // for AppKit's own work.
        #expect(worstFrame < 0.008, "worst frame \(worstFrame * 1000) ms")
        #expect(model.cache.missCount == (rows + 255) / 256)
        controller.window?.close()
    }

    /// Scrolls the live scroll view through the whole image, page by page,
    /// spinning the run loop after each step so AppKit draws as it would for
    /// a user scroll, and checks that nothing accumulates.
    @Test func liveScrollHasNoStalls() throws {
        let model = RomViewModel(rom: try Fixture.rom(megabytes: 3))
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
            RunLoop.main.run(until: Date())
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
        let model = RomViewModel(rom: try Fixture.rom(megabytes: 3))
        let (controller, scrollView, canvas) = try Fixture.window(model)
        var worst: TimeInterval = 0
        for _ in 0..<2000 {
            let cg = try #require(CGEvent(scrollWheelEvent2Source: nil, units: .line, wheelCount: 1, wheel1: -40, wheel2: 0, wheel3: 0))
            let ev = try #require(NSEvent(cgEvent: cg))
            let t0 = Date()
            scrollView.scrollWheel(with: ev)
            RunLoop.main.run(until: Date())
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
