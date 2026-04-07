import SwiftUI
import Models

/// Track listing for a selected album.
struct TrackListView: View {

    let album: Album
    let tracks: [Track]
    let currentTrackId: String?
    var onPlayTrack: (Int) -> Void
    var onBack: () -> Void
    var onToggleFavourite: (Track) -> Void

    var body: some View {
        VStack(spacing: 0) {
            // Album header
            HStack {
                Button(action: onBack) {
                    Label("Back", systemImage: "chevron.left")
                }
                .buttonStyle(.borderless)

                VStack(alignment: .leading, spacing: 2) {
                    Text(album.title)
                        .font(.title3)
                        .fontWeight(.semibold)
                    Text(album.artistName + (album.year.map { " (\($0))" } ?? ""))
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }

                Spacer()

                if !tracks.isEmpty {
                    Button("Play All") {
                        onPlayTrack(0)
                    }
                }
            }
            .padding()

            Divider()

            // Track list
            List(Array(tracks.enumerated()), id: \.element.id) { index, track in
                HStack(spacing: 12) {
                    if currentTrackId == track.id {
                        Image(systemName: "speaker.wave.2.fill")
                            .foregroundStyle(.tint)
                            .frame(width: 24)
                    } else {
                        Text(track.index.map { "\($0)" } ?? "-")
                            .foregroundStyle(.secondary)
                            .frame(width: 24, alignment: .trailing)
                    }

                    VStack(alignment: .leading, spacing: 1) {
                        Text(track.title)
                            .fontWeight(currentTrackId == track.id ? .semibold : .regular)
                        if track.hasTrackArtist {
                            Text(track.displayArtist)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }

                    Spacer()

                    Button(action: { onToggleFavourite(track) }) {
                        Image(systemName: track.isFavourite ? "star.fill" : "star")
                            .foregroundStyle(track.isFavourite ? Color.yellow : Color.secondary.opacity(0.3))
                            .font(.caption)
                    }
                    .buttonStyle(.plain)

                    Text(track.duration.formattedDuration)
                        .font(.caption)
                        .foregroundStyle(.tertiary)
                        .monospacedDigit()
                }
                .contentShape(Rectangle())
                .onTapGesture { onPlayTrack(index) }
                .padding(.vertical, 2)
            }
            .listStyle(.plain)
        }
    }
}
