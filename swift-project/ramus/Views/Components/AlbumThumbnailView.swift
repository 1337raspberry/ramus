import SwiftUI
import NukeUI

/// Reusable album art thumbnail used in queue rows, search results, etc.
struct AlbumThumbnailView: View {
    let url: URL?
    var size: CGFloat = 28
    var cornerRadius: CGFloat = 3
    var placeholderIcon: Bool = false

    var body: some View {
        Color.clear
            .frame(width: size, height: size)
            .overlay {
                LazyImage(url: url) { state in
                    if let image = state.image {
                        image.resizable().aspectRatio(contentMode: .fill)
                    } else if placeholderIcon {
                        Color.secondary.opacity(0.15)
                            .overlay {
                                Image(systemName: "music.note")
                                    .font(.system(size: size * 0.28))
                                    .foregroundStyle(.tertiary)
                            }
                    } else {
                        Color.secondary.opacity(0.15)
                    }
                }
            }
            .clipped()
            .clipShape(RoundedRectangle(cornerRadius: cornerRadius))
    }
}
