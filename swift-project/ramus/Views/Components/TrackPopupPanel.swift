import SwiftUI
import AppKit
import Models
import NukeUI

// MARK: - Popup Controller

@Observable
@MainActor
final class TrackPopupController {

    private var panel: NSPanel?
    private var clickMonitor: Any?
    private var escMonitor: Any?
    private var globalMonitor: Any?

    var isVisible: Bool { panel?.isVisible ?? false }

    func show(
        album: Album,
        tracks: [Track],
        currentTrackId: String?,
        at screenPoint: NSPoint,
        artURL: URL?,
        onPlayTrack: @escaping (Int) -> Void,
        onToggleFavourite: @escaping (Track) -> Void,
        onPlayNext: @escaping () -> Void = {},
        onAddToQueue: @escaping () -> Void = {},
        onAddTrackToQueue: @escaping (Track) -> Void = { _ in }
    ) {
        dismiss()

        let content = TrackPopupContent(
            album: album,
            tracks: tracks,
            currentTrackId: currentTrackId,
            artURL: artURL,
            onPlayTrack: { [weak self] index in
                onPlayTrack(index)
                self?.dismiss()
            },
            onToggleFavourite: onToggleFavourite,
            onPlayNext: { [weak self] in
                onPlayNext()
                self?.dismiss()
            },
            onAddToQueue: { [weak self] in
                onAddToQueue()
                self?.dismiss()
            },
            onAddTrackToQueue: onAddTrackToQueue,
            onDismiss: { [weak self] in self?.dismiss() }
        )

        let hostingView = NSHostingView(rootView: content)

        let trackRowHeight: CGFloat = 44
        let separatorHeight: CGFloat = 24
        let headerHeight: CGFloat = 52
        let maxVisibleTracks: CGFloat = 10
        let discCount = Set(tracks.compactMap(\.discNumber)).count
        let separatorCount = max(discCount - 1, 0)
        let visibleTracks = CGFloat(min(tracks.count, Int(maxVisibleTracks)))
        let height = headerHeight + visibleTracks * trackRowHeight + CGFloat(separatorCount) * separatorHeight + 20
        let width: CGFloat = 340

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

        // Position with top-left at cursor, adjusted for screen edges
        var origin = screenPoint
        origin.y -= height

        if let screen = NSScreen.main {
            let frame = screen.visibleFrame
            if origin.x + width > frame.maxX { origin.x = frame.maxX - width }
            if origin.y < frame.minY { origin.y = frame.minY }
            if origin.x < frame.minX { origin.x = frame.minX }
        }

        panel.setFrameOrigin(origin)
        panel.orderFront(nil)
        self.panel = panel

        installEventMonitors()
    }

    func dismiss() {
        panel?.close()
        panel = nil
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

// MARK: - Popup Content View

private struct TrackPopupContent: View {

    let album: Album
    let tracks: [Track]
    let currentTrackId: String?
    let artURL: URL?
    var onPlayTrack: (Int) -> Void
    var onToggleFavourite: (Track) -> Void
    @State private var toggledTrackIDs: Set<String> = []
    var onPlayNext: () -> Void
    var onAddToQueue: () -> Void
    var onAddTrackToQueue: (Track) -> Void
    var onDismiss: () -> Void

    @Environment(\.dynamicAccent) private var accentColor

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            trackList
        }
        .frame(width: 340)
        .background(.white.opacity(0.15), in: RoundedRectangle(cornerRadius: 12))
        .compatGlassRegular(in: RoundedRectangle(cornerRadius: 12))
    }

    private var header: some View {
        HStack(spacing: 8) {
            LazyImage(url: artURL) { state in
                if let image = state.image {
                    image.resizable().aspectRatio(contentMode: .fill)
                } else {
                    Color.secondary.opacity(0.15)
                }
            }
            .frame(width: 36, height: 36)
            .clipShape(RoundedRectangle(cornerRadius: 4))

            VStack(alignment: .leading, spacing: 1) {
                Text(album.title)
                    .font(.caption)
                    .fontWeight(.semibold)
                    .lineLimit(1)
                Text(album.artistName + (album.year.map { " (\($0))" } ?? ""))
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer()

            Button(action: onPlayNext) {
                Image(systemName: "text.line.first.and.arrowtriangle.forward")
                    .font(.caption)
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .help("Play Next")

            Button(action: onAddToQueue) {
                Image(systemName: "text.append")
                    .font(.caption)
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .help("Add to Queue")
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }

    private var isMultiDisc: Bool {
        Set(tracks.compactMap(\.discNumber)).count > 1
    }

    private var trackList: some View {
        ScrollView {
            VStack(spacing: 0) {
                ForEach(Array(tracks.enumerated()), id: \.element.id) { index, track in
                    if isMultiDisc, index > 0,
                       let disc = track.discNumber,
                       let prevDisc = tracks[index - 1].discNumber,
                       disc != prevDisc {
                        discSeparator(disc: disc)
                    }
                    trackRow(index: index, track: track)
                }
            }
            .padding(.vertical, 4)
        }
    }

    private func discSeparator(disc: Int) -> some View {
        HStack(spacing: 6) {
            Rectangle().fill(Color.secondary.opacity(0.2)).frame(height: 0.5)
            Text("Disc \(disc)")
                .font(.caption2)
                .foregroundStyle(.secondary)
            Rectangle().fill(Color.secondary.opacity(0.2)).frame(height: 0.5)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 4)
    }

    private func trackRow(index: Int, track: Track) -> some View {
        HStack(spacing: 8) {
            if currentTrackId == track.id {
                Image(systemName: "speaker.wave.2.fill")
                    .foregroundStyle(.tint)
                    .frame(width: 22)
                    .font(.caption2)
            } else {
                Text(track.index.map { "\($0)" } ?? "-")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .frame(width: 22, alignment: .trailing)
            }

            VStack(alignment: .leading, spacing: 1) {
                Text(track.title)
                    .font(.caption)
                    .fontWeight(currentTrackId == track.id ? .semibold : .regular)
                    .lineLimit(1)
                if track.hasTrackArtist {
                    Text(track.displayArtist)
                        .font(.system(size: 10))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }

            Spacer()

            Button(action: { onAddTrackToQueue(track) }) {
                Image(systemName: "plus.circle")
                    .foregroundStyle(.secondary.opacity(0.5))
                    .font(.system(size: 9))
                    .frame(width: 20, height: 20)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help("Add to Queue")

            Button(action: {
                if toggledTrackIDs.contains(track.id) {
                    toggledTrackIDs.remove(track.id)
                } else {
                    toggledTrackIDs.insert(track.id)
                }
                onToggleFavourite(track)
            }) {
                let isFav = toggledTrackIDs.contains(track.id) ? !track.isFavourite : track.isFavourite
                Image(systemName: isFav ? "star.fill" : "star")
                    .foregroundStyle(isFav ? Color.yellow : Color.secondary.opacity(0.3))
                    .font(.system(size: 9))
                    .frame(width: 20, height: 20)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            Text(track.duration.formattedDuration)
                .font(.caption2)
                .foregroundStyle(.tertiary)
                .monospacedDigit()
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 4)
        .contentShape(Rectangle())
        .onTapGesture { onPlayTrack(index) }
        .background {
            if currentTrackId == track.id {
                RoundedRectangle(cornerRadius: 4)
                    .fill(accentColor.opacity(0.1))
            }
        }
    }

}
