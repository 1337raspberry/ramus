import SwiftUI
import Models
import Playback
import PlexAPI
import Cache
import Search
import os.log

#if DEBUG
private let renderLog = Logger(subsystem: "com.raspsoft.ramus", category: "RenderDebug")
#endif

struct ContentView: View {

    @State private var playbackVM = PlaybackViewModel()
    @State private var libraryVM = LibraryViewModel()
    @State private var searchVM = SearchViewModel()
    @State private var showLibrarySettings = false
    @State private var showAbout = false
    @State private var trackPopup = TrackPopupController()
    @State private var eqPanel = EqualizerPanelController()
    @State private var spacebarMonitor: Any?
    @State private var accentColor: Color = Color(white: 0.65)

    var body: some View {
        @Bindable var playbackBinding = playbackVM
        #if DEBUG
        let _ = renderLog.info("ContentView.body")
        #endif
        VStack(spacing: 0) {
            if !playbackVM.isReady {
                if playbackVM.isReconnecting {
                    reconnectingView
                } else {
                    OnboardingView(onComplete: { client, cache, library, server in
                        playbackVM.acceptOnboarding(client: client, cache: cache, library: library, server: server)
                        libraryVM.setup(cache: playbackVM.cache, player: playbackVM)
                        searchVM.setup(cache: playbackVM.cache, genreMapper: libraryVM.genreMapper)
                        if !libraryVM.genreTree.isEmpty {
                            libraryVM.selectGenre(LibraryViewModel.allNode())
                        }
                        playbackVM.pickSuggestedAlbum()
                    })
                }
            } else {
                mainLayout
            }
        }
        .frame(minWidth: 800, minHeight: 500)
        .onNSWindow { window in
            window.isMovableByWindowBackground = false
            WindowDoubleClickHandler.attach(to: window)
        }
        .tint(accentColor)
        .environment(\.dynamicAccent, accentColor)
        .background {
            let colors = playbackVM.currentAlbumColors ?? playbackVM.suggestedAlbum?.colors ?? playbackVM.initialColors
            UltraBlurBackground(colors: colors)
                .animation(.easeInOut(duration: 0.8), value: colors)
        }
        .onChange(of: libraryVM.genreMapperVersion) { _, _ in
            searchVM.setup(cache: playbackVM.cache, genreMapper: libraryVM.genreMapper)
        }
        .onChange(of: playbackVM.accentRGB) { _, _ in updateAccentColor() }
        .task {
            await playbackVM.autoConnect()
            if playbackVM.isReady {
                libraryVM.setup(cache: playbackVM.cache, player: playbackVM)
                searchVM.setup(cache: playbackVM.cache, genreMapper: libraryVM.genreMapper)
                if !libraryVM.genreTree.isEmpty {
                    libraryVM.selectGenre(LibraryViewModel.allNode())
                }
                playbackVM.pickSuggestedAlbum()
            }
        }
        .sheet(isPresented: $showLibrarySettings) {
            LibrarySettingsPanel(
                playbackVM: playbackVM,
                libraryVM: libraryVM,
                onSyncComplete: { libraryVM.refreshGenreTree() },
                onSignOut: {
                    libraryVM = LibraryViewModel()
                    searchVM = SearchViewModel()
                }
            )
        }
        .onReceive(NotificationCenter.default.publisher(for: .openSettings)) { _ in
            showLibrarySettings = true
        }
        .sheet(isPresented: $showAbout) {
            AboutView()
        }
        .onReceive(NotificationCenter.default.publisher(for: .openAbout)) { _ in
            showAbout = true
        }
        .alert("Unencrypted Connection", isPresented: $playbackBinding.showHTTPWarning) {
            Button("Continue for now", role: .cancel) { }
        } message: {
            Text("ramus is connected to your Plex server over HTTP. Your auth token will be sent unencrypted.\n\nIf this is your home network and you're connected to your own Plex server, this is probably fine — but you should try to get HTTPS set up. You can enable \"Refuse HTTP connections\" in Settings to prevent this in the future.")
        }
    }

    // MARK: - Reconnecting View

    private var reconnectingView: some View {
        VStack(spacing: 16) {
            Spacer()
            ProgressView()
                .controlSize(.large)
            Text("Connecting to server...")
                .font(.subheadline)
                .foregroundStyle(.secondary)
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    // MARK: - Main Three-Column Layout

    private var mainLayout: some View {
        ZStack(alignment: .top) {
            VStack(spacing: 0) {
                if playbackVM.isSyncing, let progress = playbackVM.syncProgress {
                    syncProgressView(progress)
                }

                if libraryVM.genreTree.isEmpty && !playbackVM.isSyncing {
                    emptyState
                } else if !playbackVM.isSyncing {
                    ThreeColumnLayout(
                        sidebar: {
                            SidebarView(libraryVM: libraryVM)
                        },
                        content: {
                            contentColumn
                        },
                        detail: {
                            nowPlayingDetail
                        }
                    )
                }
            }

            if searchVM.isVisible {
                SearchOverlayView(
                    viewModel: searchVM,
                    artURL: { playbackVM.artURL(for: $0, size: 50) },
                    onSelect: { result in
                        switch result.kind {
                        case .album:
                            libraryVM.playAlbumBySourceId(result.albumSourceId)
                        case .track:
                            if let trackId = result.trackSourceId {
                                libraryVM.playTrackFromSearch(
                                    albumSourceId: result.albumSourceId,
                                    trackSourceId: trackId
                                )
                            }
                        }
                        searchVM.dismiss()
                    },
                    onLoadGrid: { albumResults in
                        libraryVM.loadAlbumsFromSearch(albumResults)
                        searchVM.dismiss()
                    },
                    onPlayNext: { result in
                        let tracks = libraryVM.tracksForSearchResult(result)
                        playbackVM.insertNext(tracks)
                        searchVM.dismiss()
                    },
                    onAddToQueue: { result in
                        let tracks = libraryVM.tracksForSearchResult(result)
                        playbackVM.appendToQueue(tracks)
                        searchVM.dismiss()
                    }
                )
                .padding(.top, 8)
                .transition(.move(edge: .top).combined(with: .opacity))
            }
        }
        .onAppear { installSpacebarMonitor() }
        .onDisappear { removeSpacebarMonitor() }
        .background {
            Group {
                Button("") { searchVM.show() }
                    .keyboardShortcut("f", modifiers: .command)
                Button("") { showLibrarySettings = true }
                    .keyboardShortcut(",", modifiers: .command)
                Button("") { searchVM.show(withPrefix: "/") }
                    .keyboardShortcut("/", modifiers: [])
                Button("") { searchVM.show(withPrefix: "@") }
                    .keyboardShortcut("@", modifiers: .shift)
                Button("") { searchVM.show(withPrefix: "!") }
                    .keyboardShortcut("!", modifiers: .shift)
                Button("") { searchVM.show(withPrefix: "%") }
                    .keyboardShortcut("%", modifiers: .shift)
                Button("") { searchVM.show(withPrefix: "$") }
                    .keyboardShortcut("$", modifiers: .shift)
                Button("") {
                    NSApp.keyWindow?.zoom(nil)
                } .keyboardShortcut("z", modifiers: [.command, .shift])
                Button("") {
                    if playbackVM.player.state.currentTrack == nil {
                        playbackVM.pickSuggestedAlbum()
                    }
                }
                .keyboardShortcut("r", modifiers: [])
            }
            .hidden()
        }
    }

    // MARK: - Now Playing Detail

    private var hasUpcomingQueue: Bool {
        let s = playbackVM.player.state
        return s.queue.count > s.queueIndex + 1
    }

    private var nowPlayingDetail: some View {
        #if DEBUG
        let _ = renderLog.info("nowPlayingDetail evaluated")
        #endif
        return GeometryReader { geo in
            ScrollView(.vertical, showsIndicators: false) {
                NowPlayingView(
                    playbackVM: playbackVM,
                    libraryVM: libraryVM,
                    eqPanel: eqPanel,
                    suggestedAlbum: playbackVM.suggestedAlbum?.data,
                    suggestedAlbumSource: playbackVM.suggestedAlbum?.source
                )
                .frame(minHeight: geo.size.height)

                if hasUpcomingQueue {
                    QueueView(
                        queue: playbackVM.player.state.queue,
                        currentIndex: playbackVM.player.state.queueIndex,
                        playbackVM: playbackVM
                    )
                    .padding(.horizontal)
                    .padding(.bottom, 16)
                }
            }
        }
    }

    // MARK: - Content Column

    @ViewBuilder
    private var contentColumn: some View {
        if let album = libraryVM.selectedAlbum {
            TrackListView(
                album: album,
                tracks: libraryVM.tracks,
                currentTrackId: playbackVM.player.state.currentTrack?.id,
                onPlayTrack: { libraryVM.playTrack(at: $0) },
                onBack: { libraryVM.selectAlbum(nil) },
                onToggleFavourite: { libraryVM.toggleFavourite(track: $0) }
            )
        } else {
            let playingAlbumKey = playbackVM.player.state.currentTrack?.albumKey
            let playingAlbum = playingAlbumKey.flatMap { key in
                libraryVM.albums.first { $0.ratingKey == key }
            }
            AlbumGridView(
                albums: libraryVM.albums,
                selectedAlbum: playingAlbum,
                sortOrder: libraryVM.albumSortOrder,
                artURL: { playbackVM.artURL(for: $0, size: 180) },
                onPlay: { album in
                    libraryVM.playAlbumInPlace(album)
                },
                onBrowse: { album in
                    let mouseLocation = NSEvent.mouseLocation
                    let tracks = libraryVM.fetchTracks(for: album)
                    trackPopup.show(
                        album: album,
                        tracks: tracks,
                        currentTrackId: playbackVM.player.state.currentTrack?.id,
                        at: mouseLocation,
                        artURL: playbackVM.artURL(for: album.thumb, size: 50),
                        onPlayTrack: { index in
                            libraryVM.playAlbumInPlace(album, startAt: index)
                        },
                        onToggleFavourite: { track in
                            libraryVM.toggleFavourite(track: track)
                        },
                        onPlayNext: {
                            let albumTracks = libraryVM.fetchTracks(for: album)
                            playbackVM.insertNext(albumTracks)
                        },
                        onAddToQueue: {
                            let albumTracks = libraryVM.fetchTracks(for: album)
                            playbackVM.appendToQueue(albumTracks)
                        },
                        onAddTrackToQueue: { track in
                            playbackVM.appendToQueue([track])
                        }
                    )
                },
                onToggleFavourite: { libraryVM.toggleFavourite(album: $0) },
                onSortChange: { order in
                    libraryVM.albumSortOrder = order
                    libraryVM.sortAlbums()
                }
            )
        }
    }

    // MARK: - Accent Color

    private func updateAccentColor() {
        let color: Color
        if let accent = playbackVM.accentRGB {
            color = Color(red: accent.r, green: accent.g, blue: accent.b)
        } else {
            color = Color(white: 0.65)
        }
        withAnimation(.easeInOut(duration: 0.6)) { accentColor = color }
    }

    // MARK: - Empty State

    private var emptyState: some View {
        VStack(spacing: 0) {
            Spacer()
            Spacer()

            VStack(spacing: 16) {
                Image(systemName: playbackVM.needsSync ? "arrow.triangle.2.circlepath" : "music.note.list")
                    .font(.system(size: 36, weight: .light))
                    .foregroundStyle(.tertiary)

                if playbackVM.needsSync {
                    Text("No library data")
                        .font(.title3)
                        .fontWeight(.medium)
                    Text("Sync your Plex library to browse by genre.")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                } else {
                    Text("No genres found")
                        .font(.title3)
                        .fontWeight(.medium)
                    Text("Your library's genres may not match the genre hierarchy.")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }

                VStack(spacing: 8) {
                    if playbackVM.needsSync, playbackVM.selectedLibrary != nil {
                        Button("Sync Now") {
                            Task {
                                await playbackVM.startSync()
                                libraryVM.refreshGenreTree()
                            }
                        }
                        .buttonStyle(.borderedProminent)
                        .controlSize(.large)
                    }

                    Button("Settings...") {
                        showLibrarySettings = true
                    }
                    .buttonStyle(.borderless)
                    .foregroundStyle(.secondary)
                    .font(.subheadline)
                }
                .padding(.top, 4)
            }
            .multilineTextAlignment(.center)
            .frame(maxWidth: 320)

            Spacer()
            Spacer()
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    // MARK: - Sync Progress

    private func syncProgressView(_ progress: SyncEngine.SyncProgress) -> some View {
        VStack(spacing: 0) {
            Spacer()
            Spacer()

            VStack(spacing: 16) {
                Image(systemName: "arrow.triangle.2.circlepath")
                    .font(.system(size: 36, weight: .light))
                    .foregroundStyle(.tertiary)

                Text(phaseName(progress.phase))
                    .font(.title3)
                    .fontWeight(.medium)

                VStack(spacing: 6) {
                    ProgressView(value: progress.fraction)
                    Text(progress.detail)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: 320)
            }

            Spacer()
            Spacer()
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func phaseName(_ phase: SyncEngine.SyncProgress.Phase) -> String {
        switch phase {
        case .artists: "Syncing artists..."
        case .albums: "Syncing albums..."
        case .tracks: "Syncing tracks..."
        case .deepGenres: "Fetching full genre data..."
        case .done: "Done!"
        }
    }

    // MARK: - Spacebar Play/Pause

    private func installSpacebarMonitor() {
        guard spacebarMonitor == nil else { return }
        spacebarMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            guard event.keyCode == 49 else { return event } // 49 = space
            // Don't intercept if a text field has focus
            if let responder = event.window?.firstResponder,
               responder is NSTextView || responder is NSTextField {
                return event
            }
            guard !playbackVM.player.state.queue.isEmpty else { return event }
            playbackVM.togglePlayPause()
            return nil
        }
    }

    private func removeSpacebarMonitor() {
        if let monitor = spacebarMonitor {
            NSEvent.removeMonitor(monitor)
            spacebarMonitor = nil
        }
    }
}

#Preview {
    ContentView()
}
