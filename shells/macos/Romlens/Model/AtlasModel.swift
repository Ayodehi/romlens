import Foundation
import Observation

/// The Atlas tab's state (docs/22, A2): which overlay it paints and the
/// zoom the menu asks for. Where the canvas is looking stays in the canvas,
/// which owns its geometry.
@MainActor
@Observable
final class AtlasModel {
    /// What a column's colour says.
    enum Overlay: String, CaseIterable, Identifiable {
        case kind, confidence, entropy, coverage
        var id: String { rawValue }
        var title: String {
            switch self {
            case .kind: "Kind"
            case .confidence: "Confidence"
            case .entropy: "Entropy"
            case .coverage: "Coverage"
            }
        }

        var help: String {
            switch self {
            case .kind: "What the analysis says each part is: code, graphics, a table… Faint where it is a guess, hatched where a column blends kinds."
            case .confidence: "How sure the analysis is, red for a guess to green for certain."
            case .entropy: "How random the bytes look, dark blue for repetitive to yellow for random: compressed data is the yellow."
            case .coverage: "What an imported execution log or trace saw run."
            }
        }
    }

    enum Zoom { case zoomIn, zoomOut, fit }

    var overlay: Overlay = .kind

    /// View › Zoom In, Zoom Out and Zoom to Fit, for the canvas to act on.
    private(set) var zoomRequest: (kind: Zoom, id: Int)?

    func requestZoom(_ kind: Zoom) {
        zoomRequest = (kind, (zoomRequest?.id ?? 0) + 1)
    }
}
