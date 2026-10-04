import CoreGraphics
import Foundation
import Testing
@testable import Romlens

/// The editor area's tree of tab groups and the changes made to it
/// (docs/29, W1).
@Suite struct EditorLayoutTests {
    private func code(_ r: CodeRepresentation = .assembly) -> EditorItem { EditorItem(.code(r)) }

    /// The shape of the tree, for comparing: groups as their tab counts, a
    /// split as its axis and children.
    private func shape(_ n: LayoutNode) -> String {
        switch n {
        case .group(let g): "\(g.items.count)"
        case .split(let s): "\(s.axis == .horizontal ? "H" : "V")(\(s.children.map(shape).joined(separator: ",")))"
        }
    }

    @Test func openingPutsTheTabAfterTheShownOneAndShowsIt() {
        let a = code(), b = code()
        var l = EditorLayout.single([a, b])
        let g = l.groups[0].id
        let c = l.open(.code(.c), in: g)!
        #expect(l.groups[0].items.map(\.id) == [a.id, c, b.id])
        #expect(l.groups[0].selected == c)
    }

    @Test func aViewWithOneTabIsShownWhereItIsInsteadOfOpenedAgain() {
        var l = EditorLayout.single([code()])
        let g = l.groups[0].id
        let atlas = l.open(.atlas, in: g)!
        l.open(.code(.hex), in: g)
        #expect(l.open(.atlas, in: g) == atlas)
        #expect(l.items.filter { $0.content == .atlas }.count == 1)
        #expect(l.groups[0].selected == atlas)
        // Code is not limited: two assembly tabs.
        l.open(.code(.assembly), in: g)
        #expect(l.items.filter { $0.content == .code(.assembly) }.count == 2)
    }

    @Test func splittingPutsTheTabInANewGroupOnThatSide() {
        let a = code(), b = code()
        var l = EditorLayout.single([a, b])
        let g = l.groups[0].id
        let fresh = l.split(g, .right, with: b.id)!
        #expect(shape(l.root) == "H(1,1)")
        #expect(l.groups.map(\.id) == [g, fresh])
        #expect(l.group(fresh)?.items.map(\.id) == [b.id])
        // Moving the left group's only tab above the right one empties the
        // left group, which goes.
        l.split(fresh, .top, with: a.id)
        #expect(shape(l.root) == "V(1,1)")
        #expect(l.groups.last?.id == fresh)
    }

    @Test func eachEdgePlacesTheGroupOnItsSide() {
        for (edge, expected) in [(DropEdge.left, "H"), (.right, "H"), (.top, "V"), (.bottom, "V")] {
            let a = code(), b = code()
            var l = EditorLayout.single([a, b])
            let target = l.groups[0].id
            let fresh = l.split(target, edge, with: b.id)!
            #expect(shape(l.root) == "\(expected)(1,1)")
            let order = l.groups.map(\.id)
            #expect(order == ((edge == .left || edge == .top) ? [fresh, target] : [target, fresh]))
        }
    }

    @Test func splittingAGroupWithItsOnlyTabDoesNothing() {
        let a = code()
        var l = EditorLayout.single([a])
        #expect(l.split(l.groups[0].id, .right, with: a.id) == nil)
        #expect(shape(l.root) == "1")
    }

    @Test func aSplitAlongTheSameAxisAddsASiblingAndSharesTheFraction() {
        let a = code(), b = code(), c = code()
        var l = EditorLayout.single([a, b, c])
        let g = l.groups[0].id
        let right = l.split(g, .right, with: b.id)!
        l.split(right, .right, with: c.id)
        guard case .split(let s) = l.root else { Issue.record("not a split"); return }
        #expect(shape(l.root) == "H(1,1,1)")
        #expect(s.fractions == [0.5, 0.25, 0.25])
    }

    @Test func movingTheLastTabOutClosesItsGroupAndCollapsesTheSplit() {
        let a = code(), b = code()
        var l = EditorLayout.single([a, b])
        let left = l.groups[0].id
        let right = l.split(left, .right, with: b.id)!
        l.move(b.id, to: left)
        #expect(shape(l.root) == "2")
        #expect(l.group(right) == nil)
        #expect(l.groups[0].selected == b.id)
    }

    @Test func movingWithinAGroupReorders() {
        let a = code(), b = code(), c = code()
        var l = EditorLayout.single([a, b, c])
        let g = l.groups[0].id
        l.move(a.id, to: g, at: 3)
        #expect(l.groups[0].items.map(\.id) == [b.id, c.id, a.id])
        l.move(c.id, to: g, at: 0)
        #expect(l.groups[0].items.map(\.id) == [c.id, b.id, a.id])
    }

    @Test func closingShowsTheNeighbourAndKeepsTheLastGroup() {
        let a = code(), b = code(), c = code()
        var l = EditorLayout.single([a, b, c])
        l.select(b.id)
        l.close(b.id)
        #expect(l.groups[0].selected == c.id)
        l.close(c.id)
        #expect(l.groups[0].selected == a.id)
        l.close(a.id)
        #expect(shape(l.root) == "0")
        #expect(l.groups[0].selected == nil)
    }

    @Test func closingAnotherGroupsLastTabRemovesItAndMergesTheAxis() {
        let a = code(), b = code(), c = code()
        var l = EditorLayout.single([a, b, c])
        let g = l.groups[0].id
        let right = l.split(g, .right, with: b.id)!
        l.split(right, .bottom, with: c.id)
        #expect(shape(l.root) == "H(1,V(1,1))")
        l.close(b.id)
        #expect(shape(l.root) == "H(1,1)")
    }

    @Test func equalizeAndDividerFractions() {
        let a = code(), b = code(), c = code()
        var l = EditorLayout.single([a, b, c])
        let g = l.groups[0].id
        let r = l.split(g, .right, with: b.id)!
        l.split(r, .right, with: c.id)
        guard case .split(let s) = l.root else { Issue.record("not a split"); return }
        l.setFractions(s.id, [2, 1, 1])
        #expect(l.split(s.id)?.fractions == [0.5, 0.25, 0.25])
        l.setFractions(s.id, [1, 1])
        #expect(l.split(s.id)?.fractions == [0.5, 0.25, 0.25])
        l.equalize(s.id)
        #expect(l.split(s.id)?.fractions.map { ($0 * 300).rounded() } == [100, 100, 100])
    }

    @Test func presetsKeepEveryTab() {
        let items = (0..<4).map { _ in code() }
        var l = EditorLayout.single(items)
        l.apply(.three)
        #expect(shape(l.root) == "H(4,V(0,0))")
        l.apply(.twoRows)
        #expect(shape(l.root) == "V(4,0)")
        l.split(l.groups[0].id, .right, with: items[3].id)
        l.apply(.single)
        #expect(Set(l.items.map(\.id)) == Set(items.map(\.id)))
        #expect(shape(l.root) == "4")
    }

    @Test func layoutsRoundTripThroughJSON() throws {
        var l = EditorLayout.single([code(), EditorItem(.graphics(.tilemap)), EditorItem(.audio(.voices), followsSelection: false), EditorItem(.tutor)])
        l.split(l.groups[0].id, .bottom, with: l.items[1].id)
        let data = try JSONEncoder().encode(l)
        #expect(try JSONDecoder().decode(EditorLayout.self, from: data) == l)
    }

    @Test func dropZones() {
        let r = CGRect(x: 0, y: 0, width: 300, height: 600)
        #expect(dropZone(CGPoint(x: 150, y: 300), in: r) == .center)
        #expect(dropZone(CGPoint(x: 10, y: 300), in: r) == .edge(.left))
        #expect(dropZone(CGPoint(x: 290, y: 300), in: r) == .edge(.right))
        #expect(dropZone(CGPoint(x: 150, y: 10), in: r) == .edge(.top))
        #expect(dropZone(CGPoint(x: 150, y: 590), in: r) == .edge(.bottom))
        // The thresholds: just inside and just outside a third.
        #expect(dropZone(CGPoint(x: 99, y: 300), in: r) == .edge(.left))
        #expect(dropZone(CGPoint(x: 101, y: 300), in: r) == .center)
        #expect(dropZone(CGPoint(x: 150, y: 199), in: r) == .edge(.top))
        // Corners: the nearer edge as a share of the side wins. 15% of the
        // width from the left loses to 5% of the height from the top.
        #expect(dropZone(CGPoint(x: 45, y: 30), in: r) == .edge(.top))
        #expect(dropZone(CGPoint(x: 15, y: 120), in: r) == .edge(.left))
        #expect(dropZone(.zero, in: .zero) == .center)
    }
}
