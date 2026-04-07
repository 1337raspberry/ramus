import SwiftUI
import NukeUI

/// Idle-screen view: random album art (tappable), funny tagline, genres.
struct SuggestedAlbumView: View {

    let data: SuggestedAlbumData
    var onPlay: () -> Void
    var onGenreTap: (String) -> Void = { _ in }

    var body: some View {
        VStack(spacing: 20) {
            Spacer()

            LazyImage(url: data.artURL) { phase in
                if let image = phase.image {
                    image
                        .resizable()
                        .interpolation(.high)
                        .aspectRatio(1, contentMode: .fit)
                } else {
                    Color.secondary.opacity(0.1)
                        .aspectRatio(1, contentMode: .fit)
                        .overlay {
                            Image(systemName: "music.note")
                                .font(.system(size: 64))
                                .foregroundStyle(.tertiary)
                        }
                }
            }
            .frame(maxWidth: 300, maxHeight: 300)
            .clipShape(RoundedRectangle(cornerRadius: 16))
            .contentShape(Rectangle())
            .hoverTilt(maxAngle: 4)
            .onTapGesture { onPlay() }
            .onHover { hovering in
                if hovering {
                    NSCursor.pointingHand.push()
                } else {
                    NSCursor.pop()
                }
            }

            if !data.tagline.segments.isEmpty {
                taglineText
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 24)
                    .padding(.bottom, 4)
            }

            if !data.genres.isEmpty {
                VStack(spacing: 4) {
                    ForEach(data.genres, id: \.self) { genre in
                        Button(action: { onGenreTap(genre) }) {
                            Text(genre)
                                .font(.subheadline.bold())
                                .foregroundStyle(.secondary)
                        }
                        .buttonStyle(.plain)
                        .onHover { hovering in
                            if hovering {
                                NSCursor.pointingHand.push()
                            } else {
                                NSCursor.pop()
                            }
                        }
                    }
                }
            }

            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding()
    }

    private var taglineText: Text {
        data.tagline.segments.reduce(Text("")) { result, segment in
            switch segment {
            case .text(let s): result + Text(s).font(.subheadline.italic())
            case .albumTitle(let s): result + Text(s).font(.subheadline.bold().italic())
            }
        }
    }
}
