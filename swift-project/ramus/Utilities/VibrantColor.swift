import CoreGraphics

/// Extracts the most vibrant and distinct accent color from an image.
/// Uses MMCQ (Modified Median Cut Quantization) for palette extraction,
/// then scores candidates by saturation, luminance, and distinctiveness
/// from the dominant palette — similar to Plexamp / Android Palette API.
nonisolated enum VibrantColor: Sendable {

    /// Extract from a CGImage. Always returns a color (best available).
    static func extract(from image: CGImage) -> (r: Double, g: Double, b: Double) {
        let pixels = extractPixels(from: image)
        guard !pixels.isEmpty else { return (r: 0.5, g: 0.5, b: 0.5) }
        let boxes = medianCutBoxes(pixels: pixels, maxColors: 32)
        let palette = boxes.map { PaletteEntry(avg: $0.averageColor, vibrant: $0.mostVibrantPixel, population: $0.pixels.count) }
        return scorePalette(palette, totalPixels: pixels.count)
    }

    // MARK: - Pixel Extraction

    private static func extractPixels(from image: CGImage, targetSize: Int = 100) -> [RGB] {
        let width = targetSize
        let height = targetSize
        let bytesPerPixel = 4
        let bytesPerRow = width * bytesPerPixel

        guard let colorSpace = CGColorSpace(name: CGColorSpace.sRGB),
              let context = CGContext(
                  data: nil, width: width, height: height,
                  bitsPerComponent: 8, bytesPerRow: bytesPerRow,
                  space: colorSpace,
                  bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
              ) else { return [] }

        context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))

        guard let data = context.data else { return [] }
        let buffer = data.bindMemory(to: UInt8.self, capacity: width * height * bytesPerPixel)

        var pixels: [RGB] = []
        pixels.reserveCapacity(width * height)

        for i in 0..<(width * height) {
            let offset = i * bytesPerPixel
            let a = buffer[offset + 3]
            guard a > 128 else { continue }

            let r = Double(buffer[offset]) / 255.0
            let g = Double(buffer[offset + 1]) / 255.0
            let b = Double(buffer[offset + 2]) / 255.0

            // Skip near-black and near-white
            let lum = 0.299 * r + 0.587 * g + 0.114 * b
            if lum < 0.05 || lum > 0.95 { continue }

            pixels.append(RGB(r: r, g: g, b: b))
        }

        return pixels
    }

    // MARK: - MMCQ (Modified Median Cut Quantization)

    private struct RGB {
        let r: Double
        let g: Double
        let b: Double

        func blended(with other: RGB, ratio: Double) -> RGB {
            RGB(r: r * (1 - ratio) + other.r * ratio,
                g: g * (1 - ratio) + other.g * ratio,
                b: b * (1 - ratio) + other.b * ratio)
        }
    }

    private struct VoxelBox {
        var pixels: [RGB]
        var rMin: Double, rMax: Double
        var gMin: Double, gMax: Double
        var bMin: Double, bMax: Double

        init(pixels: [RGB]) {
            self.pixels = pixels
            rMin = pixels.first?.r ?? 0; rMax = rMin
            gMin = pixels.first?.g ?? 0; gMax = gMin
            bMin = pixels.first?.b ?? 0; bMax = bMin
            for p in pixels {
                if p.r < rMin { rMin = p.r }
                if p.r > rMax { rMax = p.r }
                if p.g < gMin { gMin = p.g }
                if p.g > gMax { gMax = p.g }
                if p.b < bMin { bMin = p.b }
                if p.b > bMax { bMax = p.b }
            }
        }

        var longestAxis: Int {
            let rR = rMax - rMin, gR = gMax - gMin, bR = bMax - bMin
            if rR >= gR && rR >= bR { return 0 }
            if gR >= rR && gR >= bR { return 1 }
            return 2
        }

        var volume: Double {
            max(rMax - rMin, 0.001) * max(gMax - gMin, 0.001) * max(bMax - bMin, 0.001)
        }

        var averageColor: RGB {
            guard !pixels.isEmpty else { return RGB(r: 0.5, g: 0.5, b: 0.5) }
            var rS = 0.0, gS = 0.0, bS = 0.0
            for p in pixels { rS += p.r; gS += p.g; bS += p.b }
            let n = Double(pixels.count)
            return RGB(r: rS / n, g: gS / n, b: bS / n)
        }

        /// Most vibrant pixel: highest combined saturation + mid-luminance score.
        var mostVibrantPixel: RGB {
            guard !pixels.isEmpty else { return averageColor }
            var best = pixels[0]
            var bestScore = -1.0
            for p in pixels {
                let maxC = max(p.r, p.g, p.b), minC = min(p.r, p.g, p.b)
                let l = (maxC + minC) / 2.0
                let delta = maxC - minC
                guard delta > 0.01 else { continue }
                let s = l < 0.5 ? delta / (maxC + minC) : delta / (2.0 - maxC - minC)
                let lumScore = 1.0 - abs(l - 0.5) * 2.0
                let score = s * 0.6 + lumScore * 0.4
                if score > bestScore { bestScore = score; best = p }
            }
            return best
        }

        func split() -> (VoxelBox, VoxelBox) {
            let axis = longestAxis
            var sorted = pixels
            switch axis {
            case 0: sorted.sort { $0.r < $1.r }
            case 1: sorted.sort { $0.g < $1.g }
            default: sorted.sort { $0.b < $1.b }
            }
            let mid = sorted.count / 2
            return (
                VoxelBox(pixels: Array(sorted[..<mid])),
                VoxelBox(pixels: Array(sorted[mid...]))
            )
        }
    }

    private static func medianCutBoxes(pixels: [RGB], maxColors: Int) -> [VoxelBox] {
        guard !pixels.isEmpty else { return [] }

        var boxes = [VoxelBox(pixels: pixels)]

        // Phase 1: Split by population until 75% of target
        let phase1Target = maxColors * 3 / 4
        while boxes.count < phase1Target {
            guard let idx = boxes.enumerated().filter({ $0.element.pixels.count > 1 })
                    .max(by: { $0.element.pixels.count < $1.element.pixels.count })?.offset else { break }
            let box = boxes.remove(at: idx)
            let (a, b) = box.split()
            boxes.append(a)
            boxes.append(b)
        }

        // Phase 2: Split by count * volume until target
        while boxes.count < maxColors {
            guard let idx = boxes.enumerated().filter({ $0.element.pixels.count > 1 })
                    .max(by: {
                        Double($0.element.pixels.count) * $0.element.volume <
                        Double($1.element.pixels.count) * $1.element.volume
                    })?.offset else { break }
            let box = boxes.remove(at: idx)
            let (a, b) = box.split()
            boxes.append(a)
            boxes.append(b)
        }

        return boxes
    }

    // MARK: - HSL Conversion + Scoring

    private struct HSL {
        let h: Double // 0...360
        let s: Double // 0...1
        let l: Double // 0...1
    }

    private static func rgbToHSL(_ c: RGB) -> HSL {
        let maxC = max(c.r, c.g, c.b)
        let minC = min(c.r, c.g, c.b)
        let l = (maxC + minC) / 2.0
        let delta = maxC - minC

        guard delta > 0.001 else { return HSL(h: 0, s: 0, l: l) }

        let s = l < 0.5 ? delta / (maxC + minC) : delta / (2.0 - maxC - minC)

        var h: Double
        if maxC == c.r {
            h = (c.g - c.b) / delta + (c.g < c.b ? 6 : 0)
        } else if maxC == c.g {
            h = (c.b - c.r) / delta + 2
        } else {
            h = (c.r - c.g) / delta + 4
        }
        h *= 60

        return HSL(h: h, s: s, l: l)
    }

    private static func hueDistance(_ h1: Double, _ h2: Double) -> Double {
        let diff = abs(h1 - h2)
        return min(diff, 360 - diff)
    }

    private static func colorDistance(_ a: HSL, _ b: HSL) -> Double {
        let hueDist = hueDistance(a.h, b.h) / 180.0
        let satDist = a.s - b.s
        let lumDist = a.l - b.l
        return sqrt(hueDist * hueDist + satDist * satDist + lumDist * lumDist)
    }

    private struct PaletteEntry {
        let avg: RGB
        let vibrant: RGB
        let population: Int
    }

    private static func scorePalette(_ palette: [PaletteEntry], totalPixels: Int) -> (r: Double, g: Double, b: Double) {
        // Find dominant colors (top 3 by population) to compute distinctiveness
        let sorted = palette.sorted { $0.population > $1.population }
        let dominantHSLs = sorted.prefix(3).map { rgbToHSL($0.avg) }

        let weightSat = 3.0
        let weightLum = 6.0
        let weightDistinct = 4.0

        struct Candidate {
            let avg: RGB
            let vibrant: RGB
            let score: Double
        }

        func scoreColor(_ color: RGB) -> Double {
            let hsl = rgbToHSL(color)
            let satScore = hsl.s
            let lumScore = 1.0 - abs(hsl.l - 0.5) * 2.0
            let minDist = dominantHSLs.map { colorDistance(hsl, $0) }.min() ?? 1.0
            let distinctScore = min(minDist * 1.5, 1.0)
            return (satScore * weightSat + lumScore * weightLum + distinctScore * weightDistinct) /
                   (weightSat + weightLum + weightDistinct)
        }

        var candidates: [Candidate] = []

        for entry in palette {
            let hsl = rgbToHSL(entry.avg)
            guard hsl.s >= 0.35, hsl.l >= 0.3, hsl.l <= 0.7 else { continue }
            let score = scoreColor(entry.avg)
            candidates.append(Candidate(avg: entry.avg, vibrant: entry.vibrant, score: score))
        }

        // If no candidates pass the vibrant filter, relax and score all
        if candidates.isEmpty {
            for entry in palette {
                let score = scoreColor(entry.avg)
                candidates.append(Candidate(avg: entry.avg, vibrant: entry.vibrant, score: score))
            }
        }

        guard let best = candidates.max(by: { $0.score < $1.score }) else {
            return (r: 0.5, g: 0.5, b: 0.5)
        }

        // Blend box average with most-vibrant pixel (50/50) for a punchy but natural result
        let blended = best.avg.blended(with: best.vibrant, ratio: 0.5)

        // Enforce a minimum lightness so the accent always reads against dark backgrounds
        let result = ensureLightness(blended, minimum: 0.55)
        return (r: result.r, g: result.g, b: result.b)
    }

    /// Push a color's HSL lightness up to a minimum, preserving hue and saturation.
    private static func ensureLightness(_ c: RGB, minimum: Double) -> RGB {
        let hsl = rgbToHSL(c)
        guard hsl.l < minimum else { return c }
        return hslToRGB(h: hsl.h, s: hsl.s, l: minimum)
    }

    private static func hslToRGB(h: Double, s: Double, l: Double) -> RGB {
        guard s > 0.001 else { return RGB(r: l, g: l, b: l) }

        let hNorm = h / 360.0

        func hue2rgb(_ p: Double, _ q: Double, _ t: Double) -> Double {
            var t = t
            if t < 0 { t += 1 }
            if t > 1 { t -= 1 }
            if t < 1/6.0 { return p + (q - p) * 6 * t }
            if t < 1/2.0 { return q }
            if t < 2/3.0 { return p + (q - p) * (2/3.0 - t) * 6 }
            return p
        }

        let q = l < 0.5 ? l * (1 + s) : l + s - l * s
        let p = 2 * l - q
        return RGB(
            r: hue2rgb(p, q, hNorm + 1/3.0),
            g: hue2rgb(p, q, hNorm),
            b: hue2rgb(p, q, hNorm - 1/3.0)
        )
    }
}
