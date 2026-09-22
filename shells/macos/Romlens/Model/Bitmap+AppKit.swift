import CoreGraphics
import Foundation
import RomlensKit

extension BitmapInfo {
    /// The core's RGBA buffer as a `CGImage`, sharing the bytes rather than
    /// copying them pixel by pixel. `nil` for an empty image.
    var cgImage: CGImage? {
        guard width > 0, height > 0, rgba.count == Int(width * height * 4),
              let provider = CGDataProvider(data: rgba as CFData)
        else { return nil }
        return CGImage(
            width: Int(width),
            height: Int(height),
            bitsPerComponent: 8,
            bitsPerPixel: 32,
            bytesPerRow: Int(width) * 4,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.last.rawValue),
            provider: provider,
            decode: nil,
            shouldInterpolate: false,
            intent: .defaultIntent
        )
    }
}

extension TileFormat {
    var title: String {
        switch self {
        case .bpp2: "2 bpp"
        case .bpp4: "4 bpp"
        case .bpp8: "8 bpp"
        case .mode7: "Mode 7"
        }
    }

    /// Bits per pixel, which is also the number of planes the teaching view
    /// draws (Mode 7's "planes" are the eight bits of its one byte).
    var bitsPerPixel: Int {
        switch self {
        case .bpp2: 2
        case .bpp4: 4
        case .bpp8, .mode7: 8
        }
    }

    var colours: Int { 1 << bitsPerPixel }
}

extension ScreenSize {
    var title: String {
        switch self {
        case .s32x32: "32×32"
        case .s64x32: "64×32"
        case .s32x64: "32×64"
        case .s64x64: "64×64"
        }
    }

    var cells: (columns: Int, rows: Int) {
        switch self {
        case .s32x32: (32, 32)
        case .s64x32: (64, 32)
        case .s32x64: (32, 64)
        case .s64x64: (64, 64)
        }
    }
}
