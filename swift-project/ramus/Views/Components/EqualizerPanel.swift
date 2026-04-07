import SwiftUI
import AppKit
import Playback

// MARK: - Panel Controller

@Observable
@MainActor
final class EqualizerPanelController {

    private var panel: NSPanel?
    private var clickMonitor: Any?
    private var escMonitor: Any?
    private var globalMonitor: Any?
    /// Timestamp of last dismiss — used to detect the click-outside monitor
    /// racing with the toggle button (dismiss fires before button action).
    private var lastDismissTime: ContinuousClock.Instant = .now - .seconds(1)

    var isVisible: Bool { panel?.isVisible ?? false }

    func toggle(at screenPoint: NSPoint, playbackVM: PlaybackViewModel) {
        // If the panel was just dismissed by the click monitor (same mouseDown
        // that triggered this button), treat it as a close — don't reopen.
        let elapsed = ContinuousClock.now - lastDismissTime
        if isVisible || elapsed < .milliseconds(100) {
            dismiss()
        } else {
            show(at: screenPoint, playbackVM: playbackVM)
        }
    }

    func show(at screenPoint: NSPoint, playbackVM: PlaybackViewModel) {
        dismiss()

        let content = EqualizerPanelContent(playbackVM: playbackVM, controller: self)

        let hostingView = NSHostingView(rootView: content)

        let width: CGFloat = 380
        let height: CGFloat = 280

        let panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: width, height: height),
            styleMask: [.nonactivatingPanel, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        panel.isFloatingPanel = true
        panel.level = .floating
        panel.backgroundColor = .clear
        panel.isOpaque = false
        panel.hasShadow = true
        panel.contentView = hostingView
        panel.hidesOnDeactivate = true
        panel.isMovable = false

        // Position centered above the button
        var origin = screenPoint
        origin.x -= width / 2
        origin.y += 8

        if let screen = NSScreen.main {
            let frame = screen.visibleFrame
            if origin.x + width > frame.maxX { origin.x = frame.maxX - width }
            if origin.x < frame.minX { origin.x = frame.minX }
            if origin.y + height > frame.maxY { origin.y = frame.maxY - height }
            if origin.y < frame.minY { origin.y = frame.minY }
        }

        panel.setFrameOrigin(origin)
        panel.orderFront(nil)
        self.panel = panel

        installEventMonitors()
    }

    func dismiss() {
        panel?.close()
        panel = nil
        lastDismissTime = .now
        removeEventMonitors()
    }

    private func installEventMonitors() {
        clickMonitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown]) { [weak self] event in
            guard let self, let panel = self.panel else { return event }
            if event.window !== panel {
                self.dismiss()
            }
            return event
        }

        globalMonitor = NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown]) { [weak self] _ in
            self?.dismiss()
        }

        escMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            if event.keyCode == 53 {
                self?.dismiss()
                return nil
            }
            return event
        }
    }

    private func removeEventMonitors() {
        if let m = clickMonitor { NSEvent.removeMonitor(m); clickMonitor = nil }
        if let m = globalMonitor { NSEvent.removeMonitor(m); globalMonitor = nil }
        if let m = escMonitor { NSEvent.removeMonitor(m); escMonitor = nil }
    }
}

// MARK: - Panel Content

private struct EqualizerPanelContent: View {

    @Bindable var playbackVM: PlaybackViewModel
    let controller: EqualizerPanelController

    private static let labels = ["31", "62", "125", "250", "500", "1K", "2K", "4K", "8K", "16K"]
    private static let maxGain: Float = 12

    /// Derive accent color reactively from the observable playbackVM,
    /// so it updates live when tracks change while the panel is open.
    private var accentColor: Color {
        if let rgb = playbackVM.accentRGB {
            Color(red: rgb.r, green: rgb.g, blue: rgb.b)
        } else {
            Color(white: 0.65)
        }
    }

    var body: some View {
        VStack(spacing: 12) {
            header
            bandSliders
            footer
        }
        .padding(16)
        .frame(width: 380)
        .tint(accentColor)
        .environment(\.dynamicAccent, accentColor)
        .background(.white.opacity(0.15), in: RoundedRectangle(cornerRadius: 12))
        .compatGlassRegular(in: RoundedRectangle(cornerRadius: 12))
    }

    private var header: some View {
        HStack {
            Text("Equalizer")
                .font(.headline)

            Spacer()

            Toggle("", isOn: $playbackVM.equalizerEnabled)
                .toggleStyle(.switch)
                .controlSize(.small)
                .labelsHidden()

            Button {
                controller.dismiss()
            } label: {
                Image(systemName: "xmark.circle.fill")
                    .foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
        }
    }

    private var bandSliders: some View {
        HStack(alignment: .bottom, spacing: 4) {
            // dB scale on the left
            VStack {
                Text("+12")
                    .font(.system(size: 8).monospacedDigit())
                    .foregroundStyle(.tertiary)
                Spacer()
                Text("0")
                    .font(.system(size: 8).monospacedDigit())
                    .foregroundStyle(.tertiary)
                Spacer()
                Text("-12")
                    .font(.system(size: 8).monospacedDigit())
                    .foregroundStyle(.tertiary)
            }
            .frame(width: 20, height: 140)
            .padding(.bottom, 18)

            ForEach(0..<10, id: \.self) { i in
                bandColumn(index: i)
            }
        }
        .opacity(playbackVM.equalizerEnabled ? 1 : 0.35)
        .allowsHitTesting(playbackVM.equalizerEnabled)
    }

    private func bandColumn(index: Int) -> some View {
        VStack(spacing: 4) {
            VerticalEQSlider(
                value: $playbackVM.equalizerBands[index],
                range: -Self.maxGain...Self.maxGain
            )
            .frame(height: 140)

            Text(Self.labels[index])
                .font(.system(size: 9))
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity)
    }

    private var footer: some View {
        HStack {
            Spacer()
            Button("Reset") {
                playbackVM.equalizerBands = [Float](repeating: 0, count: 10)
            }
            .buttonStyle(.borderless)
            .font(.caption)
            .foregroundStyle(.secondary)
        }
    }
}

// MARK: - Vertical EQ Slider

private struct VerticalEQSlider: View {
    @Binding var value: Float
    let range: ClosedRange<Float>

    @Environment(\.dynamicAccent) private var accentColor

    var body: some View {
        GeometryReader { geo in
            let height = geo.size.height
            let trackWidth: CGFloat = 3
            let thumbSize: CGFloat = 10
            let span = range.upperBound - range.lowerBound
            let fraction = CGFloat((value - range.lowerBound) / span)
            let thumbY = height - fraction * height

            ZStack {
                // Center line (0 dB reference)
                Rectangle()
                    .fill(Color.secondary.opacity(0.2))
                    .frame(width: geo.size.width, height: 1)
                    .position(x: geo.size.width / 2, y: height / 2)

                // Track
                RoundedRectangle(cornerRadius: 1.5)
                    .fill(Color.secondary.opacity(0.25))
                    .frame(width: trackWidth, height: height)
                    .position(x: geo.size.width / 2, y: height / 2)

                // Filled portion from center to thumb
                let centerY = height / 2
                let fillHeight = abs(thumbY - centerY)
                let fillY = min(thumbY, centerY) + fillHeight / 2
                RoundedRectangle(cornerRadius: 1.5)
                    .fill(accentColor.opacity(0.6))
                    .frame(width: trackWidth, height: fillHeight)
                    .position(x: geo.size.width / 2, y: fillY)

                // Thumb
                Circle()
                    .fill(accentColor)
                    .frame(width: thumbSize, height: thumbSize)
                    .position(x: geo.size.width / 2, y: thumbY)
            }
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { drag in
                        let frac = 1 - Float(drag.location.y / height)
                        let clamped = min(max(frac, 0), 1)
                        let raw = range.lowerBound + clamped * span
                        // Snap to 0 when close
                        value = abs(raw) < 0.8 ? 0 : (raw * 2).rounded() / 2
                    }
            )
        }
    }
}
