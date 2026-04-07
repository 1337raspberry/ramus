import SwiftUI
import NukeUI
import Models

/// Grid of album cards for the selected genre.
struct AlbumGridView: View {

    let albums: [Album]
    let selectedAlbum: Album?
    let sortOrder: AlbumSortOrder
    var artURL: (String?) -> URL?
    var onPlay: (Album) -> Void
    var onBrowse: (Album) -> Void
    var onToggleFavourite: (Album) -> Void
    var onSortChange: (AlbumSortOrder) -> Void

    private let columns = [GridItem(.adaptive(minimum: 125), spacing: 16)]


    var body: some View {
        if albums.isEmpty {
            ContentUnavailableView(
                "No Albums",
                systemImage: "music.note.list",
                description: Text("Select a genre to browse albums.")
            )
        } else {
            ScrollView {
                LazyVGrid(columns: columns, spacing: 16) {
                    ForEach(albums) { album in
                        AlbumCard(
                            album: album,
                            isSelected: selectedAlbum?.id == album.id,
                            artURL: artURL(album.thumb),
                            onPlay: { onPlay(album) },
                            onBrowse: { onBrowse(album) },
                            onToggleFavourite: { onToggleFavourite(album) }
                        )
                    }
                }
                .padding()
            }
            .mask {
                VStack(spacing: 0) {
                    LinearGradient(
                        stops: [
                            .init(color: .clear, location: 0),
                            .init(color: .white, location: 1)
                        ],
                        startPoint: .top,
                        endPoint: .bottom
                    )
                    .frame(height: 24)
                    Color.white
                }
            }
            .overlay(alignment: .topTrailing) {
                sortMenu
                    .padding(.top, -6)
            }
        }
    }

    private var sortMenu: some View {
        Menu {
            ForEach(AlbumSortOrder.allCases) { order in
                Button {
                    onSortChange(order)
                } label: {
                    if order == sortOrder {
                        Label(order.label, systemImage: "checkmark")
                    } else {
                        Text(order.label)
                    }
                }
            }
        } label: {
            Image(systemName: sortOrder.icon)
                .foregroundStyle(.secondary)
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .background(.white.opacity(0.05), in: Capsule())
        }
        .menuStyle(.borderlessButton)
        .fixedSize()
    }

}

/// Single album cell in the grid with album art via NukeUI.
private struct AlbumCard: View {

    let album: Album
    let isSelected: Bool
    let artURL: URL?
    var onPlay: () -> Void
    var onBrowse: () -> Void
    var onToggleFavourite: () -> Void

    @Environment(\.dynamicAccent) private var accentColor
    @State private var isHovered = false

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            ZStack {
                Color.clear
                    .aspectRatio(1, contentMode: .fit)
                    .overlay {
                        LazyImage(url: artURL) { state in
                            if let image = state.image {
                                image
                                    .resizable()
                                    .aspectRatio(1, contentMode: .fill)
                            } else {
                                Color.secondary.opacity(0.1)
                                    .overlay {
                                        Image(systemName: "music.note")
                                            .font(.largeTitle)
                                            .foregroundStyle(.tertiary)
                                    }
                            }
                        }
                    }
                    .clipped()
                    .clipShape(RoundedRectangle(cornerRadius: 8))
                    .onTapGesture(count: 2) { onPlay() }
                    .onForceTouch { onBrowse() }

                // Top-right: browse button on hover
                if isHovered {
                    VStack {
                        HStack {
                            Spacer()
                            Button(action: onBrowse) {
                                Image(systemName: "ellipsis")
                                    .font(.system(size: 12, weight: .bold))
                                    .foregroundStyle(.white)
                                    .padding(.horizontal, 8)
                                    .padding(.vertical, 5)
                                    .background(.ultraThinMaterial, in: Capsule())
                            }
                            .buttonStyle(.plain)
                            .padding(6)
                        }
                        Spacer()
                    }
                    .transition(.opacity)
                }

                // Bottom-left: favourite star
                if album.isFavourite || isHovered {
                    VStack {
                        Spacer()
                        HStack {
                            Button(action: onToggleFavourite) {
                                Image(systemName: album.isFavourite ? "star.fill" : "star")
                                    .font(.system(size: 14, weight: .bold))
                                    .foregroundStyle(album.isFavourite ? .yellow : .white)
                                    .shadow(color: .black.opacity(0.5), radius: 2)
                            }
                            .buttonStyle(.plain)
                            .padding(6)
                            Spacer()
                        }
                    }
                    .transition(.opacity)
                }
            }
            .onHover { isHovered = $0 }
            .animation(.easeInOut(duration: 0.15), value: isHovered)

            VStack(alignment: .leading, spacing: 2) {
                Text(album.title)
                    .font(.caption)
                    .fontWeight(isSelected ? .semibold : .regular)
                    .lineLimit(2)

                Text(album.artistName)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)

                if let year = album.year {
                    Text(String(year))
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                }
            }
            .frame(height: 52, alignment: .top)
        }
        .padding(8)
        .background(
            RoundedRectangle(cornerRadius: 10)
                .fill(isSelected ? accentColor.opacity(0.08) : .clear)
        )
    }
}
