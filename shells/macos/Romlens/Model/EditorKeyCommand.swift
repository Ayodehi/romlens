import AppKit

/// Keys both canvases understand; the view model interprets them per pane.
enum EditorKeyCommand: Equatable {
    case left, right, up, down, pageUp, pageDown, home, end, back
    case extendLeft, extendRight, extendUp, extendDown
    case follow, rename, comment, markCode, markData, markUnknown

    /// The command for a key event, or nil for keys the canvas does not own.
    static func from(event: NSEvent) -> EditorKeyCommand? {
        let shift = event.modifierFlags.contains(.shift)
        let plain = event.modifierFlags.intersection([.command, .control, .option]).isEmpty
        switch event.specialKey {
        case .leftArrow: return shift ? .extendLeft : .left
        case .rightArrow: return shift ? .extendRight : .right
        case .upArrow: return shift ? .extendUp : .up
        case .downArrow: return shift ? .extendDown : .down
        case .pageUp: return .pageUp
        case .pageDown: return .pageDown
        case .home: return .home
        case .end: return .end
        case .delete, .backspace: return .back
        case .carriageReturn, .enter: return .follow
        default: break
        }
        guard plain, let chars = event.charactersIgnoringModifiers else { return nil }
        switch chars {
        case "g": return .follow
        case "n": return .rename
        case ";": return .comment
        case "c": return .markCode
        case "d": return .markData
        case "u": return .markUnknown
        default: return nil
        }
    }
}

/// Which canvas a command came from; up/down mean rows in hex, lines in asm.
enum EditorSource {
    case hex, asm
}
