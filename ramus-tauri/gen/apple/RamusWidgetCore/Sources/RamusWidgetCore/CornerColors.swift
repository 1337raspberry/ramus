import CoreGraphics
import Foundation

/// Four gradient corners as six-digit hex without a leading `#`.
public struct CornerHex: Equatable {
    public let topLeft: String
    public let topRight: String
    public let bottomRight: String
    public let bottomLeft: String
}

/// Corner-colour extraction from a cover, the same computation the app runs
/// on the decoded art for its blurred backdrop (`ui/src/lib/blurArt.ts`,
/// which documents each stage and constant). The cover is downscaled, split
/// into a grid of chroma-weighted cells, and each corner quadrant is pooled
/// to its most salient cell blended with the quadrant average, then held
/// inside brightness and chroma rails.
public enum CornerColors {
    public static let intermediateSize = 128
    static let gridSize = 4
    static let weightPower = 2.0
    static let weightFloor = 0.05
    static let winnerBlend = 0.65
    static let minVividLevel = 110.0
    static let vividChromaGate = 0.15
    static let paleCeiling = 120.0
    static let vividCeiling = 210.0
    static let ceilingChromaRef = 0.5
    static let lumaCap = 120.0
    static let lumaFloor = 40.0
    static let contextMix = 0.4
    static let contextHueRange = 90.0
    static let contextChromaGate = 0.07
    static let chromaCap = 0.45

    struct Rgb {
        var r: Double, g: Double, b: Double

        var chroma: Double { (max(r, g, b) - min(r, g, b)) / 255 }
        var luma: Double { 0.299 * r + 0.587 * g + 0.114 * b }

        /// Hue in degrees, 0..<360. Meaningless for achromatic colours.
        var hue: Double {
            let hi = max(r, g, b), lo = min(r, g, b), d = hi - lo
            if d == 0 { return 0 }
            var h: Double
            if hi == r {
                h = ((g - b) / d).truncatingRemainder(dividingBy: 6)
            } else if hi == g {
                h = (b - r) / d + 2
            } else {
                h = (r - g) / d + 4
            }
            return (h * 60 + 360).truncatingRemainder(dividingBy: 360)
        }

        func scaled(_ s: Double) -> Rgb { Rgb(r: r * s, g: g * s, b: b * s) }
        func mixed(_ o: Rgb, _ t: Double) -> Rgb {
            Rgb(r: r * (1 - t) + o.r * t, g: g * (1 - t) + o.g * t, b: b * (1 - t) + o.b * t)
        }

        var hex: String {
            func c(_ v: Double) -> Int { Int(max(0, min(255, v.rounded()))) }
            return String(format: "%02x%02x%02x", c(r), c(g), c(b))
        }
    }

    static func chromaWeight(_ r: Double, _ g: Double, _ b: Double) -> Double {
        weightFloor + pow((max(r, g, b) - min(r, g, b)) / 255, weightPower)
    }

    static func distance(_ a: Rgb, _ b: Rgb) -> Double {
        let dr = (a.r - b.r) / 255, dg = (a.g - b.g) / 255, db = (a.b - b.b) / 255
        return (dr * dr + dg * dg + db * db).squareRoot() / 3.0.squareRoot()
    }

    /// Chroma-weighted downscale of `intermediateSize`² RGBA pixels into
    /// `gridSize`² cells, row by row.
    static func weightedCells(_ px: [UInt8]) -> [Rgb] {
        let n = intermediateSize
        let cellPx = Double(n) / Double(gridSize)
        var out: [Rgb] = []
        for cy in 0..<gridSize {
            for cx in 0..<gridSize {
                var sr = 0.0, sg = 0.0, sb = 0.0, sw = 0.0
                let x0 = Int((Double(cx) * cellPx).rounded(.down))
                let x1 = Int((Double(cx + 1) * cellPx).rounded(.down))
                let y0 = Int((Double(cy) * cellPx).rounded(.down))
                let y1 = Int((Double(cy + 1) * cellPx).rounded(.down))
                for y in y0..<y1 {
                    for x in x0..<x1 {
                        let i = (y * n + x) * 4
                        let r = Double(px[i]), g = Double(px[i + 1]), b = Double(px[i + 2])
                        let w = chromaWeight(r, g, b)
                        sr += r * w; sg += g * w; sb += b * w; sw += w
                    }
                }
                out.append(Rgb(r: sr / sw, g: sg / sw, b: sb / sw))
            }
        }
        return out
    }

    static func poolCorner(_ cells: [Rgb], xs: [Int], ys: [Int], mean: Rgb) -> Rgb {
        var sr = 0.0, sg = 0.0, sb = 0.0, sw = 0.0
        var pr = 0.0, pg = 0.0, pb = 0.0, n = 0.0
        var winner: Rgb?
        var best = -1.0
        for y in ys {
            for x in xs {
                let c = cells[y * gridSize + x]
                let w = chromaWeight(c.r, c.g, c.b)
                sr += c.r * w; sg += c.g * w; sb += c.b * w; sw += w
                pr += c.r; pg += c.g; pb += c.b; n += 1
                let score = (0.1 + c.chroma) * (0.15 + distance(c, mean))
                if score > best {
                    best = score
                    winner = c
                }
            }
        }
        let avg = Rgb(r: sr / sw, g: sg / sw, b: sb / sw)
        let plain = Rgb(r: pr / n, g: pg / n, b: pb / n)
        var out = winner.map { avg.mixed($0, winnerBlend) } ?? avg

        // Hue-gated context mix.
        var ctx = contextMix
        if out.chroma >= contextChromaGate && plain.chroma >= contextChromaGate {
            var gap = abs(out.hue - plain.hue)
            gap = min(gap, 360 - gap)
            ctx = contextMix * max(0, 1 - gap / contextHueRange)
        }
        out = out.mixed(plain, ctx)

        // Brightness floor for vivid-but-dark colours, else a
        // chroma-scaled ceiling for pale ones.
        let hi = max(out.r, out.g, out.b)
        let chroma = out.chroma
        if chroma > vividChromaGate && hi > 0 && hi < minVividLevel {
            out = out.scaled(minVividLevel / hi)
        } else {
            let ceiling = paleCeiling + (vividCeiling - paleCeiling) * min(1, chroma / ceilingChromaRef)
            if hi > ceiling { out = out.scaled(ceiling / hi) }
        }

        // Perceptual brightness cap.
        if out.luma > lumaCap { out = out.scaled(lumaCap / out.luma) }

        // Chroma cap.
        let finalChroma = out.chroma
        if finalChroma > chromaCap {
            let grey = (out.r + out.g + out.b) / 3
            let t = chromaCap / finalChroma
            out = Rgb(r: grey + (out.r - grey) * t, g: grey + (out.g - grey) * t, b: grey + (out.b - grey) * t)
        }

        // Additive luma floor.
        let luma = out.luma
        if luma < lumaFloor {
            let add = lumaFloor - luma
            out = Rgb(r: out.r + add, g: out.g + add, b: out.b + add)
        }
        return out
    }

    /// Corners from `intermediateSize`² RGBA pixels.
    public static func fromPixels(_ px: [UInt8]) -> CornerHex {
        let cells = weightedCells(px)
        let count = Double(cells.count)
        let mean = Rgb(
            r: cells.reduce(0) { $0 + $1.r } / count,
            g: cells.reduce(0) { $0 + $1.g } / count,
            b: cells.reduce(0) { $0 + $1.b } / count
        )
        let lo = [0, 1], hi = [gridSize - 2, gridSize - 1]
        return CornerHex(
            topLeft: poolCorner(cells, xs: lo, ys: lo, mean: mean).hex,
            topRight: poolCorner(cells, xs: hi, ys: lo, mean: mean).hex,
            bottomRight: poolCorner(cells, xs: hi, ys: hi, mean: mean).hex,
            bottomLeft: poolCorner(cells, xs: lo, ys: hi, mean: mean).hex
        )
    }

    /// Corners from a decoded cover, downscaled into an sRGB bitmap with
    /// high-quality interpolation (what the app's canvas draw does).
    public static func fromImage(_ image: CGImage) -> CornerHex? {
        let n = intermediateSize
        guard image.width > 0, image.height > 0,
              let space = CGColorSpace(name: CGColorSpace.sRGB) else { return nil }
        var px = [UInt8](repeating: 0, count: n * n * 4)
        let drawn = px.withUnsafeMutableBytes { buf -> Bool in
            guard let ctx = CGContext(
                data: buf.baseAddress, width: n, height: n, bitsPerComponent: 8,
                bytesPerRow: n * 4, space: space,
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
            ) else { return false }
            ctx.interpolationQuality = .high
            ctx.draw(image, in: CGRect(x: 0, y: 0, width: n, height: n))
            return true
        }
        return drawn ? fromPixels(px) : nil
    }
}

/// The tone pass the app applies when it paints corner colours behind its
/// player on a high-dynamic-range iPhone screen: saturation, then
/// brightness, then the backdrop drawn at 80 % over the sheet's base.
public enum BackdropTone {
    static let saturation = 1.05
    static let brightness = 0.9
    static let opacity = 0.8
    static let base: (r: Double, g: Double, b: Double) = (10, 10, 12)

    public static func display(_ hex: String) -> String {
        let h = hex.hasPrefix("#") ? String(hex.dropFirst()) : hex
        let v = UInt32(h, radix: 16) ?? 0
        let r = Double((v >> 16) & 0xff), g = Double((v >> 8) & 0xff), b = Double(v & 0xff)
        let grey = (r + g + b) / 3
        func adjust(_ c: Double) -> Double {
            max(0, min(255, ((grey + (c - grey) * saturation) * brightness).rounded()))
        }
        return CornerColors.Rgb(
            r: adjust(r) * opacity + base.r * (1 - opacity),
            g: adjust(g) * opacity + base.g * (1 - opacity),
            b: adjust(b) * opacity + base.b * (1 - opacity)
        ).hex
    }
}
