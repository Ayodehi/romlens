import AppKit
import RomlensKit
import SwiftUI
import Testing
@testable import Romlens

/// The navigator's tab picker fits the navigator however narrow it is:
/// the words, then smaller words, then icons, then a pop-up menu.
@MainActor
@Suite struct NavigatorWidthTests {
    /// What the picker is at `width`: its segments' labels, or "menu".
    private func picker(at width: CGFloat, _ m: RomViewModel) -> (labels: [String], images: Int, menu: Bool, fits: Bool) {
        let host = NSHostingView(rootView: NavigatorView(model: m).frame(width: width, height: 200))
        host.frame = NSRect(x: 0, y: 0, width: width, height: 200)
        let window = NSWindow(contentRect: host.frame, styleMask: [.borderless], backing: .buffered, defer: false)
        // Closed by the test and released by Swift: not by itself as well.
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.orderFront(nil)
        defer { window.close() }
        Fixture.spin(0.1)
        host.layoutSubtreeIfNeeded()
        if let s = Fixture.find(host, NSSegmentedControl.self) {
            let labels = (0..<s.segmentCount).compactMap { s.label(forSegment: $0) }.filter { !$0.isEmpty }
            let images = (0..<s.segmentCount).filter { s.image(forSegment: $0) != nil }.count
            let frame = s.convert(s.bounds, to: host)
            return (labels, images, false, frame.minX >= 0 && frame.maxX <= width)
        }
        let menu = Fixture.find(host, NSPopUpButton.self) != nil
        return ([], 0, menu, true)
    }

    @Test func thePickerStepsDownAsTheNavigatorNarrows() async throws {
        let rom = try Rom.fromBytes(bytes: makeGraphicsTestRom(), name: "g.sfc")
        let m = try await Fixture.analyzedModel(rom: rom)
        let wide = picker(at: 360, m)
        #expect(wide.labels == ["Labels", "Variables", "Regions", "Banks"] && wide.fits)
        for w in [300, 260, 240, 220, 200] as [CGFloat] {
            #expect(picker(at: w, m).fits, "at \(w) the picker runs past the navigator")
        }
        // At the navigator's narrowest, icons (each named in its help).
        #expect(picker(at: 200, m).images == 4)
    }
}
