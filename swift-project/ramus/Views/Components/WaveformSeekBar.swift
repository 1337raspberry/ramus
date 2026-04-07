import SwiftUI

/// Plexamp-style waveform seek bar.
/// Shows a thin line when no data is available, fills in the waveform when levels arrive.
/// Played portion is tinted, unplayed portion is muted gray.
struct WaveformSeekBar: View {

    let levels: [Float]?
    let position: TimeInterval
    let duration: TimeInterval
    var bufferedFraction: Double = 0
    var isBuffering: Bool = false
    var onSeek: (TimeInterval) -> Void

    @Environment(\.dynamicAccent) private var accentColor
    @State private var isSeeking = false
    @State private var seekPosition: TimeInterval = 0
    @State private var scanOffset: CGFloat = 0

    private let barHeight: CGFloat = 40

    private var displayPosition: TimeInterval {
        isSeeking ? seekPosition : position
    }

    private var fraction: CGFloat {
        guard duration > 0 else { return 0 }
        return CGFloat(displayPosition / duration)
    }

    var body: some View {
        VStack(spacing: 2) {
            GeometryReader { geo in
                let width = geo.size.width

                Canvas { context, size in
                    if let levels, !levels.isEmpty {
                        drawWaveform(context: context, size: size, levels: levels)
                    } else {
                        drawThinLine(context: context, size: size)
                    }
                }
                .gesture(
                    DragGesture(minimumDistance: 0)
                        .onChanged { value in
                            isSeeking = true
                            let frac = max(0, min(1, value.location.x / width))
                            seekPosition = Double(frac) * duration
                        }
                        .onEnded { value in
                            let frac = max(0, min(1, value.location.x / width))
                            let target = Double(frac) * duration
                            onSeek(target)
                            isSeeking = false
                        }
                )
                .overlay {
                    if isBuffering {
                        scanningOverlay(width: width)
                    }
                }
            }
            .frame(height: barHeight)
            .contentShape(Rectangle())
            .onAppear { applyScanAnimation(isBuffering) }
            .onChange(of: isBuffering) { _, buffering in
                applyScanAnimation(buffering)
            }

            HStack {
                Text(displayPosition.formattedDuration)
                    .font(.system(size: 10, weight: .medium).monospacedDigit())
                    .foregroundStyle(.secondary)
                Spacer()
                Text(duration.formattedDuration)
                    .font(.system(size: 10, weight: .medium).monospacedDigit())
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 4)
        }
    }

    private func applyScanAnimation(_ buffering: Bool) {
        if buffering {
            scanOffset = 0
            withAnimation(.easeInOut(duration: 0.7).repeatForever(autoreverses: true)) {
                scanOffset = 1
            }
        } else {
            var t = Transaction()
            t.disablesAnimations = true
            withTransaction(t) { scanOffset = 0 }
        }
    }

    // MARK: - Drawing

    private func drawWaveform(context: GraphicsContext, size: CGSize, levels: [Float]) {
        let midY = size.height / 2
        let progressX = fraction * size.width
        let bufferedX = CGFloat(bufferedFraction) * size.width

        let path = waveformPath(levels: levels, size: size)

        // Unplayed portion (full waveform, muted)
        context.fill(path, with: .color(.secondary.opacity(0.2)))

        // Buffered portion (slightly brighter than unplayed)
        if bufferedX > progressX {
            var bufferedContext = context
            bufferedContext.clip(to: Path(CGRect(x: progressX, y: 0, width: bufferedX - progressX, height: size.height)))
            bufferedContext.fill(path, with: .color(.secondary.opacity(0.35)))
        }

        // Played portion (clipped to progress)
        var playedContext = context
        playedContext.clip(to: Path(CGRect(x: 0, y: 0, width: progressX, height: size.height)))
        playedContext.fill(path, with: .color(accentColor.opacity(0.85)))

        // Thin center line (always visible)
        let linePath = Path { p in
            p.move(to: CGPoint(x: 0, y: midY))
            p.addLine(to: CGPoint(x: size.width, y: midY))
        }
        context.stroke(linePath, with: .color(.secondary.opacity(0.3)), lineWidth: 1)

        // No playhead line — progress shown via color fill only
    }

    private func drawThinLine(context: GraphicsContext, size: CGSize) {
        let midY = size.height / 2
        let progressX = fraction * size.width
        let bufferedX = CGFloat(bufferedFraction) * size.width

        // Background line
        let bgPath = Path { p in
            p.move(to: CGPoint(x: 0, y: midY))
            p.addLine(to: CGPoint(x: size.width, y: midY))
        }
        context.stroke(bgPath, with: .color(.secondary.opacity(0.3)), lineWidth: 2)

        // Buffered line
        if bufferedX > progressX {
            let bufPath = Path { p in
                p.move(to: CGPoint(x: progressX, y: midY))
                p.addLine(to: CGPoint(x: bufferedX, y: midY))
            }
            context.stroke(bufPath, with: .color(.secondary.opacity(0.5)), lineWidth: 2)
        }

        // Progress line
        if progressX > 0 {
            let fgPath = Path { p in
                p.move(to: CGPoint(x: 0, y: midY))
                p.addLine(to: CGPoint(x: progressX, y: midY))
            }
            context.stroke(fgPath, with: .color(accentColor), lineWidth: 2)
        }
    }

    private func waveformPath(levels: [Float], size: CGSize) -> Path {
        let midY = size.height / 2
        let maxAmplitude = midY - 4 // leave padding top/bottom
        let count = levels.count
        guard count > 0 else { return Path() }

        let stepX = size.width / CGFloat(count)

        return Path { p in
            // Top edge (forward)
            p.move(to: CGPoint(x: 0, y: midY - CGFloat(levels[0]) * maxAmplitude))
            for i in 1..<count {
                let x = CGFloat(i) * stepX
                let y = midY - CGFloat(levels[i]) * maxAmplitude
                let prevX = CGFloat(i - 1) * stepX
                let cpX = (prevX + x) / 2
                p.addQuadCurve(to: CGPoint(x: x, y: y), control: CGPoint(x: cpX, y: y))
            }

            // Bottom edge (reverse, mirrored)
            for i in stride(from: count - 1, through: 0, by: -1) {
                let x = CGFloat(i) * stepX
                let y = midY + CGFloat(levels[i]) * maxAmplitude
                if i == count - 1 {
                    p.addLine(to: CGPoint(x: x, y: y))
                } else {
                    let nextX = CGFloat(i + 1) * stepX
                    let cpX = (nextX + x) / 2
                    p.addQuadCurve(to: CGPoint(x: x, y: y), control: CGPoint(x: cpX, y: y))
                }
            }
            p.closeSubpath()
        }
    }

    private func scanningOverlay(width: CGFloat) -> some View {
        let scanWidth = width * 0.18
        let travelRange = width - scanWidth
        return RoundedRectangle(cornerRadius: 2)
            .fill(
                LinearGradient(
                    colors: [.clear, accentColor.opacity(0.45), .clear],
                    startPoint: .leading,
                    endPoint: .trailing
                )
            )
            .frame(width: scanWidth, height: barHeight)
            .offset(x: -travelRange / 2 + scanOffset * travelRange)
            .allowsHitTesting(false)
    }

}
