import CoreGraphics

/// Geometry for the widget's waveform, drawn the same way as the app's seek
/// bar: a smooth curve through each level above the midline, mirrored below
/// it, as one closed shape.
public enum WaveformPath {
    public static func silhouette(levels: [Double], in rect: CGRect) -> CGPath {
        let path = CGMutablePath()
        guard !levels.isEmpty else { return path }
        // A single level still spans the full width.
        let levels = (levels.count == 1 ? [levels[0], levels[0]] : levels).map { min(max($0, 0), 1) }
        let midY = rect.midY
        let maxAmp = rect.height / 2 - 2
        let step = rect.width / CGFloat(levels.count - 1)
        let x = { (i: Int) in rect.minX + CGFloat(i) * step }
        let amp = { (i: Int) in CGFloat(levels[i]) * maxAmp }
        let last = levels.count - 1

        path.move(to: CGPoint(x: x(0), y: midY - amp(0)))
        for i in 1...last {
            let cp = (x(i - 1) + x(i)) / 2
            path.addQuadCurve(to: CGPoint(x: x(i), y: midY - amp(i)), control: CGPoint(x: cp, y: midY - amp(i)))
        }
        path.addLine(to: CGPoint(x: x(last), y: midY + amp(last)))
        for i in stride(from: last - 1, through: 0, by: -1) {
            let cp = (x(i + 1) + x(i)) / 2
            path.addQuadCurve(to: CGPoint(x: x(i), y: midY + amp(i)), control: CGPoint(x: cp, y: midY + amp(i)))
        }
        path.closeSubpath()
        return path
    }

    /// The point of a track a tap on seek zone `zone` of `zones` jumps to:
    /// the zone's centre.
    public static func seekFraction(zone: Int, zones: Int) -> Double {
        (Double(zone) + 0.5) / Double(zones)
    }
}
