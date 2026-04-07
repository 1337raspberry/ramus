import SwiftUI
import Models
import os.log

#if DEBUG
private let renderLog = Logger(subsystem: "com.raspsoft.ramus", category: "RenderDebug")
#endif

/// Displays the playback queue below the now-playing view.
/// Only renders a visible window of tracks for performance with large queues.
///
/// Takes queue/currentIndex as value types (SwiftUI diffs them — skips body
/// when unchanged) and playbackVM as a stable reference for actions (no
/// closures that would force re-renders inside GeometryReader).
struct QueueView: View {

    let queue: [Track]
    let currentIndex: Int
    let playbackVM: PlaybackViewModel

    @State private var visibleCount = 30

    private var upcomingStart: Int { currentIndex + 1 }
    private var totalUpcoming: Int { max(queue.count - upcomingStart, 0) }

    private var visibleSlice: [(queueIndex: Int, track: Track)] {
        guard upcomingStart < queue.count else { return [] }
        let end = min(upcomingStart + visibleCount, queue.count)
        return (upcomingStart..<end).map { ($0, queue[$0]) }
    }

    var body: some View {
        #if DEBUG
        let _ = renderLog.info("QueueView.body")
        #endif
        VStack(alignment: .leading, spacing: 0) {
            header
            if totalUpcoming == 0 {
                emptyState
            } else {
                trackList
            }
        }
        .background(.white.opacity(0.1), in: RoundedRectangle(cornerRadius: 10))
    }

    private var header: some View {
        HStack {
            Text("Up Next")
                .font(.caption)
                .fontWeight(.semibold)
                .foregroundStyle(.secondary)
            Spacer()
            Text("\(totalUpcoming) tracks")
                .font(.caption2)
                .foregroundStyle(.tertiary)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }

    private var emptyState: some View {
        Text("No upcoming tracks")
            .font(.caption)
            .foregroundStyle(.tertiary)
            .frame(maxWidth: .infinity)
            .padding(.vertical, 12)
    }

    private var trackList: some View {
        LazyVStack(spacing: 0) {
            ForEach(visibleSlice, id: \.queueIndex) { queueIndex, track in
                trackRow(track: track, queueIndex: queueIndex)
            }

            if visibleCount < totalUpcoming {
                Button {
                    visibleCount += 50
                } label: {
                    Text("Show more (\(totalUpcoming - visibleCount) remaining)")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 8)
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.bottom, 4)
    }

    private func trackRow(track: Track, queueIndex: Int) -> some View {
        HStack(spacing: 8) {
            AlbumThumbnailView(url: playbackVM.artURL(for: track.thumb, size: 50), size: 28, cornerRadius: 3)

            VStack(alignment: .leading, spacing: 1) {
                Text(track.title)
                    .font(.caption)
                    .lineLimit(1)
                Text(track.displayArtist)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }

            Spacer()

            Text(track.duration.formattedDuration)
                .font(.caption2)
                .foregroundStyle(.tertiary)
                .monospacedDigit()

            Button {
                playbackVM.removeFromQueue(at: queueIndex)
            } label: {
                Image(systemName: "xmark")
                    .font(.system(size: 8, weight: .semibold))
                    .frame(width: 20, height: 20)
                    .contentShape(Rectangle())
                    .foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 4)
        .contentShape(Rectangle())
        .onTapGesture { playbackVM.jumpToQueueIndex(queueIndex) }
    }

}
