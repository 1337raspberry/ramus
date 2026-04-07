import SwiftUI
import GenreTree
import Cache

/// Three-mode sidebar: Genres, Starred, Artists.
/// Tree vs flat display is controlled by the global `useHierarchy` setting.
struct SidebarView: View {

    @Bindable var libraryVM: LibraryViewModel
    @Environment(\.dynamicAccent) private var accentColor

    var body: some View {
        VStack(spacing: 0) {
            SidebarModePicker(
                selectedMode: libraryVM.sidebarMode
            ) { mode in
                libraryVM.sidebarMode = mode
            }
            .padding(.horizontal, 4)
            .padding(.top, 4)

            Group {
                switch libraryVM.sidebarMode {
                case .genres:
                    if libraryVM.useHierarchy {
                        genresList
                    } else {
                        flatGenresList
                    }
                case .favourites:
                    favouritesList
                case .artists:
                    artistsList
                }
            }
        }
        .background(.white.opacity(0.05), in: RoundedRectangle(cornerRadius: 12))
        .padding(.leading, 8)
        .padding(.top, 8)
        .padding(.bottom, 8)
        .onChange(of: libraryVM.sidebarMode) { _, newMode in
            var t = Transaction()
            t.disablesAnimations = true
            withTransaction(t) {
                libraryVM.selectedGenre = nil
                libraryVM.selectedArtistSourceId = nil
                libraryVM.selectedAlbum = nil
                libraryVM.tracks = []
                libraryVM.albums = []

                switch newMode {
                case .genres:
                    libraryVM.selectGenre(LibraryViewModel.allNode())
                case .favourites:
                    libraryVM.refreshFavouriteGenreTree()
                    libraryVM.selectGenre(LibraryViewModel.allNode())
                case .artists:
                    libraryVM.loadAllArtists()
                    libraryVM.selectGenre(LibraryViewModel.allNode())
                }
            }
        }
    }

    // MARK: - Genres (Tree)

    private var genresList: some View {
        let allCount = libraryVM.totalAlbumCount
        return GenreTreeView(
            genres: [LibraryViewModel.allNode(count: allCount)] + libraryVM.genreTree,
            allExpandableIDs: libraryVM.allExpandableGenreIDs,
            verticalPadding: libraryVM.libraryPadding,
            selection: $libraryVM.selectedGenre,
            expandedIDs: $libraryVM.expandedGenreIDs,
            scrollToID: $libraryVM.scrollToGenreID
        )
        .onChange(of: libraryVM.selectedGenre) { _, newValue in
            libraryVM.selectGenre(newValue)
        }
    }

    // MARK: - Genres (Flat)

    private var flatGenresList: some View {
        let allCount = libraryVM.totalAlbumCount
        return ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                flatGenreRow(name: "All", count: allCount, isSelected: libraryVM.selectedGenre?.id == LibraryViewModel.allNode().id) {
                    libraryVM.selectGenre(LibraryViewModel.allNode())
                }

                ForEach(libraryVM.flatGenreList, id: \.name) { genre in
                    flatGenreRow(name: genre.name, count: genre.count, isSelected: libraryVM.selectedGenre?.name == genre.name) {
                        libraryVM.selectFlatGenre(named: genre.name)
                    }
                }
            }
            .padding(.horizontal, 4)
            .padding(.top, 4)
        }
    }

    private func flatGenreRow(name: String, count: Int, isSelected: Bool, action: @escaping () -> Void) -> some View {
        HStack(spacing: 0) {
            Text(name)
                .lineLimit(1)
            Spacer(minLength: 0)
            Text("\(count)")
                .font(.caption2)
                .foregroundStyle(.tertiary)
                .monospacedDigit()
        }
        .padding(.leading, 12)
        .padding(.trailing, 6)
        .padding(.vertical, libraryVM.libraryPadding)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(isSelected ? accentColor.opacity(0.3) : Color.clear)
        .contentShape(Rectangle())
        .onTapGesture(perform: action)
    }

    // MARK: - Favourites

    private var favouritesList: some View {
        let favCount = libraryVM.favouriteAlbumCount
        return VStack(spacing: 0) {
            if libraryVM.useHierarchy {
                GenreTreeView(
                    genres: [LibraryViewModel.allNode(count: favCount)] + libraryVM.favouriteGenreTree,
                    allExpandableIDs: libraryVM.allExpandableFavouriteIDs,
                    verticalPadding: libraryVM.libraryPadding,
                    selection: $libraryVM.selectedGenre,
                    expandedIDs: $libraryVM.expandedGenreIDs,
                    scrollToID: $libraryVM.scrollToGenreID,
                    shuffleVM: libraryVM
                )
                .onChange(of: libraryVM.selectedGenre) { _, newValue in
                    libraryVM.selectGenre(newValue)
                }
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        flatGenreRow(name: "All", count: favCount, isSelected: libraryVM.selectedGenre?.id == LibraryViewModel.allNode().id) {
                            libraryVM.selectGenre(LibraryViewModel.allNode())
                        }

                        ForEach(libraryVM.flatFavouriteGenreList, id: \.name) { genre in
                            flatGenreRow(name: genre.name, count: genre.count, isSelected: libraryVM.selectedGenre?.name == genre.name) {
                                libraryVM.selectFlatGenre(named: genre.name)
                            }
                        }
                    }
                    .padding(.horizontal, 4)
                    .padding(.top, 4)
                }
            }
        }
    }

    // MARK: - Artists

    private var artistsList: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                ForEach(libraryVM.artists, id: \.sourceId) { artist in
                    Text(artist.name)
                        .lineLimit(1)
                        .padding(.leading, 12)
                        .padding(.vertical, libraryVM.libraryPadding)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(
                            libraryVM.selectedArtistSourceId == artist.sourceId
                                ? accentColor.opacity(0.3)
                                : Color.clear
                        )
                        .contentShape(Rectangle())
                        .onTapGesture {
                            libraryVM.selectedArtistSourceId = artist.sourceId
                        }
                }
            }
            .padding(.horizontal, 4)
            .padding(.top, 2)
        }
        .onChange(of: libraryVM.selectedArtistSourceId) { _, newValue in
            if let sourceId = newValue {
                libraryVM.selectArtist(sourceId: sourceId)
            }
        }
    }
}

/// A single pill-shaped picker with a sliding glass indicator.
private struct SidebarModePicker: View {
    let selectedMode: SidebarMode
    let onTap: (SidebarMode) -> Void

    private let modes = SidebarMode.allCases

    private var selectedIndex: Int {
        modes.firstIndex(of: selectedMode) ?? 0
    }

    var body: some View {
        GeometryReader { geo in
            let segmentWidth = geo.size.width / CGFloat(modes.count)

            ZStack(alignment: .leading) {
                // Sliding glass capsule
                RoundedRectangle(cornerRadius: 12)
                    .fill(.clear)
                    .compatGlassClear(in: RoundedRectangle(cornerRadius: 12))
                    .frame(width: segmentWidth, height: geo.size.height)
                    .offset(x: CGFloat(selectedIndex) * segmentWidth)
                    .animation(.spring(duration: 0.28, bounce: 0.3), value: selectedIndex)

                // Text labels
                HStack(spacing: 0) {
                    ForEach(modes) { mode in
                        Text(mode.label)
                            .font(.subheadline)
                            .frame(maxWidth: .infinity)
                            .frame(height: geo.size.height)
                            .contentShape(Rectangle())
                            .onTapGesture {
                                onTap(mode)
                            }
                    }
                }
            }
        }
        .frame(height: 28)
    }
}
