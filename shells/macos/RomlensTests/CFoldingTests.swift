import AppKit
import Foundation
import RomlensKit
import Testing
@testable import Romlens

/// Folding the C's blocks the way an IDE does: `for (…) {…}` on one line.
@MainActor
@Suite struct CFoldingTests {
    @Test func blocksAreTheBracesThatSpanLines() {
        let text = """
        void f(void)
        {
            /* { not a block } */
            if (a) {
                s = "}{";
                c = '{';
            } else {
                x = 1; // }
            }
            { y = 2; }
        }
        """
        let ns = text as NSString
        func line(_ i: Int) -> Int { ns.substring(to: i).filter { $0 == "\n" }.count }
        let folds = CFold.find(in: text)
        #expect(folds.map { [line($0.open), line($0.close)] } == [[1, 10], [3, 6], [6, 8]])
    }

    /// The lines as drawn: hidden characters left out, a fold's first as `…`.
    private func shown(_ tv: NSTextView, _ folder: CFolder) -> [String] {
        let lm = tv.layoutManager!
        lm.ensureLayout(for: tv.textContainer!)
        let ns = tv.string as NSString
        var out: [String] = []
        lm.enumerateLineFragments(forGlyphRange: NSRange(location: 0, length: lm.numberOfGlyphs)) { _, _, _, glyphs, _ in
            var line = ""
            for g in glyphs.location..<NSMaxRange(glyphs) where lm.propertyForGlyph(at: g) != .null {
                let c = lm.characterIndexForGlyph(at: g)
                if folder.hidden.contains(where: { $0.location == c }) {
                    line += "…"
                } else if ns.character(at: c) != 10 {
                    line += ns.substring(with: NSRange(location: c, length: 1))
                }
            }
            out.append(line)
        }
        return out
    }

    @Test func aFoldedBlockReadsAsOneLine() async throws {
        let rom = try Rom.fromBytes(bytes: makeRoutinesTestRom(), name: "r.sfc")
        let m = try await Fixture.analyzedModel(rom: rom)
        m.select(offset: 0x22)
        m.editorTab = .c
        try await Fixture.settle(until: { m.decompiler.result?.name == "SUB_008020" })
        let pane = CPaneController(model: m)
        pane.update()
        let text = pane.textView.string
        let ns = text as NSString
        let header = "for (x = 0x0F; (s8)x >= 0; x--) {"
        let at = ns.range(of: header).location + header.utf16.count - 1
        let loop = try #require(pane.folder.folds.first { $0.open == at })
        let before = shown(pane.textView, pane.folder)
        #expect(before.contains { $0.hasSuffix(header) })

        // Folded: the loop is one line, and the text underneath is the same.
        #expect(pane.textView.onClickCharacter?(loop.hidden.location) == false, "an open block has no …")
        #expect(pane.foldKey(.fold, at: at - 3))
        let after = shown(pane.textView, pane.folder)
        #expect(after.contains { $0.hasSuffix(String(header.dropLast()) + "{…}") }, "\(after)")
        let hiddenLines = ns.substring(with: loop.hidden).filter { $0 == "\n" }.count
        #expect(after.count == before.count - hiddenLines)
        #expect(pane.textView.string == text)

        // A click on the … opens it again.
        #expect(pane.textView.onClickCharacter?(loop.hidden.location) == true)
        #expect(shown(pane.textView, pane.folder) == before)

        // The same routine's C again after a rename: still folded.
        _ = pane.foldKey(.fold, at: at - 3)
        try m.session.setLabel(address: 0x00_8020, name: "ClearSlots")
        try await Fixture.settle(until: { m.decompiler.result?.name == "ClearSlots" })
        pane.update()
        #expect(pane.textView.string != text)
        let again = pane.textView.string as NSString
        let at2 = again.range(of: header).location + header.utf16.count - 1
        #expect(pane.folder.folded == [at2])

        // Selecting an instruction inside the loop opens it.
        let bodyLine = again.substring(to: at2).filter { $0 == "\n" }.count + 1
        let inside = try #require(m.decompiler.offsets(forLine: bodyLine).min())
        m.select(offset: 0x20)
        pane.update()
        #expect(pane.folder.folded == [at2], "the loop's own lines show while it is folded")
        m.select(offset: inside)
        pane.update()
        #expect(pane.folder.folded.isEmpty)

        // Fold All folds the blocks inside the routine, not its body.
        _ = pane.foldKey(.foldAll)
        let outline = shown(pane.textView, pane.folder)
        #expect(outline.contains { $0.hasSuffix("{…}") })
        #expect(outline.contains("{"), "the routine's own body stays open")
        _ = pane.foldKey(.unfoldAll)
        #expect(pane.folder.folded.isEmpty)
    }
}
