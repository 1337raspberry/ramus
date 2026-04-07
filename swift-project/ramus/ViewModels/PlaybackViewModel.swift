import SwiftUI
import Nuke
import PlexAPI
import Playback
import Cache
import Models

/// RGB accent color extracted from album art.
struct AccentRGB: Equatable {
    let r: Double
    let g: Double
    let b: Double
}

/// Drives the playback UI: connects AudioPlayer state to views, handles transport actions.
@MainActor @Observable
final class PlaybackViewModel {

    let player = AudioPlayer()
    var plexClient = PlexClient()
    var cache: CacheDatabase?
    private var nowPlayingBridge: NowPlayingBridge?
    private var sessionReporter: SessionReporter?
    private var connectionMonitor: ConnectionMonitor?

    var isConnected = false
    var isReconnecting = PlexAuth.storedToken() != nil && PlexAuth.storedServerConfig() != nil
    var errorMessage: String?

    /// True when authenticated, connected, and library selected — ready for main UI.
    var isReady: Bool { isConnected && selectedLibrary != nil }

    // Library selection
    var musicLibraries: [LibrarySection] = []
    var selectedLibrary: LibrarySection?

    // Sync state
    var isSyncing = false
    var syncProgress: SyncEngine.SyncProgress?
    var cacheStats: CacheDatabase.CacheStats?

    /// Server config name for display in settings.
    var serverName: String? { PlexAuth.storedServerConfig()?.name }

    // MARK: - Playback Config (UserDefaults-backed)

    var playbackMode: PlaybackConfig.PlaybackMode = {
        let raw = UserDefaults.standard.string(forKey: UserDefaultsKeys.playbackMode) ?? ""
        return PlaybackConfig.PlaybackMode(rawValue: raw) ?? .directPlay
    }() {
        didSet {
            UserDefaults.standard.set(playbackMode.rawValue, forKey: UserDefaultsKeys.playbackMode)
            applyPlaybackConfig()
        }
    }

    var lookaheadDepth: Int = {
        let v = UserDefaults.standard.integer(forKey: UserDefaultsKeys.lookaheadDepth)
        return v > 0 ? v : 3
    }() {
        didSet {
            UserDefaults.standard.set(lookaheadDepth, forKey: UserDefaultsKeys.lookaheadDepth)
            applyPlaybackConfig()
        }
    }

    var showTaglines: Bool = {
        if UserDefaults.standard.object(forKey: UserDefaultsKeys.showTaglines) == nil { return true }
        return UserDefaults.standard.bool(forKey: UserDefaultsKeys.showTaglines)
    }() {
        didSet {
            UserDefaults.standard.set(showTaglines, forKey: UserDefaultsKeys.showTaglines)
        }
    }

    var audioCacheLimitGB: Double = {
        let v = UserDefaults.standard.double(forKey: UserDefaultsKeys.audioCacheLimitGB)
        return v > 0 ? max(0.1, min(50.0, v)) : 2.0
    }() {
        didSet {
            UserDefaults.standard.set(audioCacheLimitGB, forKey: UserDefaultsKeys.audioCacheLimitGB)
            applyPlaybackConfig()
        }
    }

    var audioCacheSizeBytes: Int64 = 0

    /// Automatic sync interval in hours (0 = disabled, manual only). Default 6.
    var syncIntervalHours: Int = {
        if UserDefaults.standard.object(forKey: UserDefaultsKeys.syncIntervalHours) == nil { return 6 }
        let v = UserDefaults.standard.integer(forKey: UserDefaultsKeys.syncIntervalHours)
        return max(0, min(24, v))
    }() {
        didSet {
            guard syncIntervalHours != oldValue else { return }
            UserDefaults.standard.set(syncIntervalHours, forKey: UserDefaultsKeys.syncIntervalHours)
        }
    }

    // MARK: - Security (UserDefaults-backed)

    var refuseHTTP: Bool = UserDefaults.standard.bool(forKey: UserDefaultsKeys.refuseHTTP) {
        didSet {
            guard refuseHTTP != oldValue else { return }
            UserDefaults.standard.set(refuseHTTP, forKey: UserDefaultsKeys.refuseHTTP)
            Task {
                await connectionMonitor?.setAllowHTTP(!refuseHTTP)
                if refuseHTTP {
                    await connectionMonitor?.evaluateConnection()
                }
            }
        }
    }

    var showHTTPWarning = false

    // MARK: - Equalizer (UserDefaults-backed)

    var equalizerEnabled: Bool = UserDefaults.standard.bool(forKey: UserDefaultsKeys.equalizerEnabled) {
        didSet {
            guard equalizerEnabled != oldValue else { return }
            UserDefaults.standard.set(equalizerEnabled, forKey: UserDefaultsKeys.equalizerEnabled)
            applyEqualizer()
        }
    }

    var equalizerBands: [Float] = {
        if let arr = UserDefaults.standard.array(forKey: UserDefaultsKeys.equalizerBands) as? [Double],
           arr.count == 10 {
            return arr.map { b in Float(b).isFinite ? Float(b) : 0 }
        }
        return [Float](repeating: 0, count: 10)
    }() {
        didSet {
            guard equalizerBands != oldValue else { return }
            UserDefaults.standard.set(equalizerBands.map { Double($0) }, forKey: UserDefaultsKeys.equalizerBands)
            scheduleEQApply()
        }
    }

    @ObservationIgnored private var eqApplyTask: Task<Void, Never>?

    /// Debounce EQ filter rebuilds during slider drags (~50ms).
    private func scheduleEQApply() {
        eqApplyTask?.cancel()
        eqApplyTask = Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(50))
            guard !Task.isCancelled else { return }
            applyEqualizer()
        }
    }

    func applyEqualizer() {
        player.applyEqualizer(enabled: equalizerEnabled, bands: equalizerBands)
    }

    private func applyPlaybackConfig() {
        let limitBytes = Int64(audioCacheLimitGB * 1_073_741_824)
        player.updateConfig(PlaybackConfig(
            playbackMode: playbackMode,
            lookaheadDepth: lookaheadDepth,
            audioCacheLimitBytes: limitBytes
        ))
    }

    func refreshAudioCacheSize() {
        Task {
            audioCacheSizeBytes = await player.audioCacheSizeBytes()
        }
    }

    func clearAudioCache() {
        player.clearAudioCache()
        audioCacheSizeBytes = 0
    }

    // MARK: - Image Cache

    var imageCacheSizeBytes: Int64 = 0

    func refreshImageCacheSize() {
        Task.detached(priority: .utility) {
            let size = (ImagePipeline.shared.configuration.dataCache as? DataCache)?.totalSize ?? 0
            await MainActor.run { self.imageCacheSizeBytes = Int64(size) }
        }
    }

    func clearImageCache() {
        if let dataCache = ImagePipeline.shared.configuration.dataCache as? DataCache {
            dataCache.removeAll()
        }
        ImageCache.shared.removeAll()
        imageCacheSizeBytes = 0
    }

    /// Last sync time from UserDefaults.
    var lastSyncTime: Date? {
        let ti = UserDefaults.standard.double(forKey: UserDefaultsKeys.lastSyncTime)
        return ti > 0 ? Date(timeIntervalSince1970: ti) : nil
    }

    // MARK: - Auto-Connect (uses ServerConfig + discovery)

    func autoConnect() async {
        // Init cache
        do {
            cache = try CacheDatabase()
            cacheStats = try cache?.stats()
        } catch {
            // Non-fatal
        }

        guard let token = PlexAuth.storedToken(),
              let config = PlexAuth.storedServerConfig() else {
            isReconnecting = false
            return
        }

        do {
            // Re-discover servers to find best connection
            let servers = try await plexClient.discoverServers(authToken: token)
            guard let server = servers.first(where: { $0.machineIdentifier == config.machineIdentifier }) else {
                isReconnecting = false
                return
            }

            let (connection, isHTTP) = await plexClient.findBestConnection(server: server, allowHTTP: !refuseHTTP)
            guard let connection else {
                isReconnecting = false
                if refuseHTTP {
                    errorMessage = "No secure connection available. Disable \"Refuse HTTP connections\" in Settings or check your network."
                }
                return
            }
            if isHTTP { showHTTPWarning = true }

            guard let serverURL = URL(string: connection.uri) else {
                isReconnecting = false
                return
            }
            try await plexClient.connect(serverURL: serverURL, token: server.accessToken)
            player.configure(serverURL: serverURL, token: server.accessToken, clientIdentifier: plexClient.clientIdentifier)
            player.isRemote = !connection.local || connection.uri.contains("plex.direct")
            applyPlaybackConfig()
            applyEqualizer()
            setupLevelsProvider()
            setupNowPlaying()
            setupSessionReporter()
            await startConnectionMonitor(server: server, activeURI: connection.uri, authToken: token)
            isConnected = true

            // Restore library selection
            let libs = try await plexClient.findMusicLibraries()
            musicLibraries = libs
            if let libKey = config.selectedLibraryKey {
                selectedLibrary = libs.first { $0.key == libKey }
            }
            if selectedLibrary == nil, libs.count == 1 {
                selectedLibrary = libs.first
            }

            // Auto incremental sync if stale (silent — no progress UI)
            if cache != nil, selectedLibrary != nil, syncIntervalHours > 0 {
                let staleThreshold: TimeInterval = Double(syncIntervalHours) * 3600
                if let lastSync = lastSyncTime,
                   Date().timeIntervalSince(lastSync) > staleThreshold {
                    Task { await startSilentIncrementalSync() }
                } else if lastSyncTime == nil {
                    // Never synced — don't auto-sync, user will see empty state
                }
            }
        } catch {
            // Silently fail — user will see onboarding or empty state
        }

        isReconnecting = false
    }

    // MARK: - Onboarding Handoff

    /// Accept configured objects from the onboarding flow.
    func acceptOnboarding(client: PlexClient, cache: CacheDatabase?, library: LibrarySection, server: PlexServer? = nil) {
        self.plexClient = client
        self.cache = cache
        self.selectedLibrary = library
        self.cacheStats = try? cache?.stats()

        if let serverURL = client.serverURL, let token = client.token {
            player.configure(serverURL: serverURL, token: token, clientIdentifier: client.clientIdentifier)
            applyPlaybackConfig()
            applyEqualizer()
            setupLevelsProvider()
            setupNowPlaying()
            setupSessionReporter()
            isConnected = true

            if serverURL.scheme == "http" { showHTTPWarning = true }

            if let server, let authToken = PlexAuth.storedToken() {
                Task { await startConnectionMonitor(server: server, activeURI: serverURL.absoluteString, authToken: authToken) }
            }
        }

        // Fetch music libraries for settings panel
        Task {
            if let libs = try? await plexClient.findMusicLibraries() {
                musicLibraries = libs
            }
        }
    }

    // MARK: - Connection Monitor

    private func startConnectionMonitor(server: PlexServer, activeURI: String, authToken: String) async {
        await connectionMonitor?.stop()

        let monitor = ConnectionMonitor(client: plexClient)

        await monitor.setOnConnectionChanged { [weak self] newURL, newToken, isLocal, isHTTP in
            guard let self else { return }
            await MainActor.run { [self] in
                self.plexClient.serverURL = newURL
                self.plexClient.token = newToken
                let remote = !isLocal || (newURL.host()?.contains("plex.direct") == true)
                self.player.updateServerConnection(serverURL: newURL, token: newToken, isRemote: remote)
                if isHTTP { self.showHTTPWarning = true }
            }
        }

        await monitor.setOnConnectionLost { [weak self] in
            guard let self else { return }
            await MainActor.run { [self] in
                if self.refuseHTTP {
                    self.errorMessage = "No secure connection available. Only unencrypted HTTP connections were found."
                }
            }
        }

        let client = plexClient
        plexClient.onRequestFailed = { [weak monitor] in
            guard let monitor else { return false }
            let urlBefore = client.serverURL
            await monitor.evaluateConnection()
            return client.serverURL != nil && client.serverURL != urlBefore
        }

        await monitor.setAllowHTTP(!refuseHTTP)
        await monitor.start(server: server, activeConnectionURI: activeURI, authToken: authToken)
        self.connectionMonitor = monitor
    }

    // MARK: - Sign Out

    func signOut() {
        if let monitor = connectionMonitor {
            Task { await monitor.stop() }
        }
        connectionMonitor = nil
        PlexAuth.deleteToken()
        PlexAuth.deleteServerConfig()
        player.stop()
        nowPlayingBridge?.clear()
        sessionReporter?.playbackStopped()
        isConnected = false
        musicLibraries = []
        selectedLibrary = nil
    }

    // MARK: - Sync

    var needsSync: Bool {
        guard let stats = cacheStats else { return true }
        return stats.trackCount == 0
    }

    enum SyncMode { case incremental, genres, full }

    func startSync() async {
        await runSync(mode: .full, silent: false)
    }

    func startGenreSync() async {
        await runSync(mode: .genres, silent: false)
    }

    func startIncrementalSync() async {
        await runSync(mode: .incremental, silent: false)
    }

    /// Background auto-sync — no progress UI shown.
    private func startSilentIncrementalSync() async {
        await runSync(mode: .incremental, silent: true)
    }

    private func runSync(mode: SyncMode, silent: Bool) async {
        guard let cache, let library = selectedLibrary else {
            if !silent { errorMessage = "Select a library first" }
            return
        }

        let engine = SyncEngine(cache: cache, client: plexClient)
        if !silent {
            isSyncing = true
            if mode == .full { errorMessage = nil }
        }

        let progressHandler: @Sendable (SyncEngine.SyncProgress) -> Void
        if silent {
            progressHandler = { _ in }
        } else {
            progressHandler = { [weak self] progress in
                guard let self else { return }
                Task { @MainActor [self] in
                    self.syncProgress = progress
                }
            }
        }

        do {
            switch mode {
            case .full:
                try await engine.fullSync(libraryKey: library.key, onProgress: progressHandler)
            case .genres:
                try await engine.genreSync(libraryKey: library.key, onProgress: progressHandler)
            case .incremental:
                try await engine.incrementalSync(libraryKey: library.key, onProgress: progressHandler)
            }
            cacheStats = try? cache.stats()
        } catch {
            if !silent {
                errorMessage = "Sync failed: \(error.localizedDescription)"
            }
        }

        if !silent {
            isSyncing = false
            syncProgress = nil
        }
    }

    /// Play an arbitrary list of tracks (used by LibraryViewModel).
    func playTracks(_ tracks: [Track], startAt index: Int = 0) {
        player.loadQueue(tracks, startAt: index)
        nowPlayingBridge?.updateTrack()
    }

    // MARK: - Queue Manipulation

    func appendToQueue(_ tracks: [Track]) { player.appendToQueue(tracks) }
    func insertNext(_ tracks: [Track]) { player.insertNext(tracks) }
    func removeFromQueue(at index: Int) { player.removeFromQueue(at: index) }
    func jumpToQueueIndex(_ index: Int) {
        player.jumpToIndex(index)
        nowPlayingBridge?.updateTrack()
    }

    // MARK: - Transport

    func togglePlayPause() {
        player.togglePlayPause()
        nowPlayingBridge?.updatePlaybackState()
    }
    func next() {
        player.next()
        nowPlayingBridge?.updateTrack()
    }
    func previous() {
        player.previous()
        nowPlayingBridge?.updateTrack()
    }
    func seek(to time: TimeInterval) {
        player.seek(to: time)
        nowPlayingBridge?.updatePlaybackState()
        sessionReporter?.playbackSeeked()
    }

    // MARK: - Waveform

    private func setupLevelsProvider() {
        let client = plexClient
        player.levelsProvider = { ratingKey in
            guard let metadata = try? await client.fetchItemMetadata(ratingKey: ratingKey),
                  let streamID = metadata.media?.first?.parts?.first?.streams?
                      .first(where: { $0.streamType == 2 })?.id else {
                return nil
            }
            return try? await client.fetchLevels(streamID: streamID, subsample: 600)
        }
    }

    private func setupNowPlaying() {
        nowPlayingBridge = NowPlayingBridge(player: player) { [weak self] thumb in
            self?.artURL(for: thumb, size: 180)
        }
        // UI refresh — fires immediately when track info is set (before download/playback).
        player.onTrackInfoChange = { [weak self] in
            guard let self else { return }
            self.nowPlayingBridge?.updateTrack()
            self.refreshCurrentAlbumInfo()
            Task { await self.refreshAccentColor() }
            if self.player.state.currentTrack?.albumKey != nil {
                self.clearSuggestedAlbum()
            } else if self.suggestedAlbum == nil {
                self.pickSuggestedAlbum()
            }
        }
        // Session reporting — fires after playback has actually started.
        player.onTrackChange = { [weak self] in
            guard let self else { return }
            self.nowPlayingBridge?.updateTrack()
            self.sessionReporter?.trackStarted()
            self.refreshCurrentAlbumInfo()
            Task { await self.refreshAccentColor() }
            if self.player.state.currentTrack?.albumKey != nil {
                self.clearSuggestedAlbum()
            } else if self.suggestedAlbum == nil {
                self.pickSuggestedAlbum()
            }
        }
    }

    private func setupSessionReporter() {
        let reporter = SessionReporter(player: player, client: plexClient)
        sessionReporter = reporter

        player.onPlaybackStateChange = { [weak self] status in
            guard let reporter = self?.sessionReporter else { return }
            switch status {
            case .paused:
                reporter.playbackPaused()
            case .playing:
                reporter.playbackResumed()
            case .stopped:
                reporter.playbackStopped()
            }
        }

        player.onTrackEnded = { [weak self] track in
            self?.sessionReporter?.trackEnded(track)
        }
    }

    // MARK: - Current Album Info (cached, updated on track change)

    private(set) var currentAlbumYear: Int?
    private(set) var currentAlbumStudio: String?
    private(set) var currentAlbumIsFavourite: Bool = false
    private(set) var currentAlbumGenres: [String] = []
    private(set) var currentAlbumArtistSourceId: String?
    private(set) var currentAlbumColors: UltraBlurColors?

    func refreshCurrentAlbumInfo() {
        guard let cache, let albumKey = player.state.currentTrack?.albumKey else {
            currentAlbumYear = nil
            currentAlbumStudio = nil
            currentAlbumIsFavourite = false
            currentAlbumGenres = []
            currentAlbumArtistSourceId = nil
            currentAlbumColors = nil
            return
        }
        if let row = try? cache.albumForSourceId(albumKey) {
            currentAlbumYear = row.year
            currentAlbumStudio = row.studio
            currentAlbumIsFavourite = (row.rating ?? 0) >= 10
            currentAlbumArtistSourceId = row.artistSourceId
        }
        currentAlbumGenres = (try? cache.genresForAlbum(sourceId: albumKey)) ?? []
        currentAlbumColors = try? cache.albumColors(sourceId: albumKey)
    }

    func toggleCurrentAlbumFavourite(using libraryVM: LibraryViewModel) {
        guard let albumKey = player.state.currentTrack?.albumKey,
              let cache, let row = try? cache.albumForSourceId(albumKey) else { return }
        let album = LibraryViewModel.albumFromRow(row)
        libraryVM.toggleFavourite(album: album)
        refreshCurrentAlbumInfo()
    }

    func toggleCurrentTrackFavourite(using libraryVM: LibraryViewModel) {
        guard let track = player.state.currentTrack else { return }
        libraryVM.toggleFavourite(track: track)
        player.updateTrackFavourite(ratingKey: track.ratingKey, isFavourite: !track.isFavourite)
    }

    // MARK: - Lyrics

    private let lyricsProvider = LyricsProvider()
    private var lyricsCache: [String: LyricsResult] = [:]
    private(set) var currentLyrics: LyricsResult?

    /// Fetch lyrics for the currently playing track.
    func fetchLyricsForCurrentTrack() async {
        guard let track = player.state.currentTrack else {
            currentLyrics = nil
            return
        }
        // Return cached if available
        if let cached = lyricsCache[track.ratingKey] {
            currentLyrics = cached
            return
        }
        let albumTitle = track.albumTitle
        let result = await lyricsProvider.fetchLyrics(for: track, albumTitle: albumTitle, using: plexClient)
        if let result {
            lyricsCache[track.ratingKey] = result
        }
        // Only set if we're still on the same track
        if player.state.currentTrack?.ratingKey == track.ratingKey {
            currentLyrics = result
        }
    }

    /// Clear lyrics state (called on track change).
    func clearLyrics() {
        currentLyrics = nil
        // Keep cache small — only remember current track
        if lyricsCache.count > 5 {
            lyricsCache.removeAll()
        }
    }

    // MARK: - Accent Color

    /// Accent color extracted from album art. Nil means use default accent.
    var accentRGB: AccentRGB?
    private var accentColorCache: [String: AccentRGB] = [:]
    private var accentColorTask: Task<Void, Never>?

    /// Random background colors shown before any album is playing.
    let initialColors: UltraBlurColors = {
        let palette = [
            "7a3b3b", "8b5e5e", "7a5a3b", "8b7a5e", "6b6b3b",
            "3b6b4a", "4a7a5e", "3b6b6b", "3b5a6b", "3b4a7a",
            "5e5e8b", "6b3b6b", "7a5a7a", "5e3b4a", "4a5e6b",
            "6b5a4a", "5a6b5a", "4a5a5e", "7a6b5a", "5e4a5a",
        ]
        func pick() -> String { palette.randomElement()! }
        return UltraBlurColors(topLeft: pick(), topRight: pick(), bottomRight: pick(), bottomLeft: pick())
    }()

    func refreshAccentColor() async {
        accentColorTask?.cancel()

        guard let track = player.state.currentTrack,
              let albumKey = track.albumKey else { return }

        if let cached = accentColorCache[albumKey] {
            if accentRGB != cached {
                accentRGB = cached
            }
            return
        }

        guard let url = artURL(for: track.thumb, size: 300) else { return }

        let task = Task {
            do {
                let image = try await ImagePipeline.shared.image(for: url)
                guard !Task.isCancelled else { return }
                guard let cgImage = image.cgImage(forProposedRect: nil, context: nil, hints: nil) else { return }
                let extracted = await Task.detached(priority: .utility) {
                    VibrantColor.extract(from: cgImage)
                }.value
                guard !Task.isCancelled else { return }
                let rgb = AccentRGB(r: extracted.r, g: extracted.g, b: extracted.b)

                if self.player.state.currentTrack?.albumKey == albumKey {
                    self.accentColorCache[albumKey] = rgb
                    self.accentRGB = rgb
                }
            } catch {
                // Silent — keep current accent
            }
        }
        accentColorTask = task
    }

    // MARK: - Suggested Album (idle screen)

    struct SuggestedAlbum {
        let data: SuggestedAlbumData
        let source: Album
        var colors: UltraBlurColors?
    }

    private(set) var suggestedAlbum: SuggestedAlbum?

    /// Pick a random album to display on the idle screen.
    func pickSuggestedAlbum() {
        guard let cache,
              let row = try? cache.randomAlbum() else {
            suggestedAlbum = nil
            return
        }
        let album = LibraryViewModel.albumFromRow(row)
        let genres = (try? cache.genresForAlbum(sourceId: row.sourceId)) ?? []
        let colors = try? cache.albumColors(sourceId: row.sourceId)
        suggestedAlbum = SuggestedAlbum(
            data: SuggestedAlbumData(
                artURL: artURL(for: album.thumb, size: 600),
                tagline: showTaglines
                    ? SuggestedAlbumTaglines.pick(albumTitle: album.title)
                    : TaglineParts(segments: []),
                genres: genres
            ),
            source: album,
            colors: colors
        )
    }

    /// Clear suggested album state (e.g. when playback starts).
    func clearSuggestedAlbum() {
        suggestedAlbum = nil
    }

    // MARK: - Art URLs

    func artURL(for thumb: String?, size: Int? = nil) -> URL? {
        guard let thumb, let serverURL = plexClient.serverURL, let token = plexClient.token else { return nil }
        var components = URLComponents(url: serverURL.appendingPathComponent("photo/:/transcode"), resolvingAgainstBaseURL: false)
        let thumbURL = "\(thumb)?X-Plex-Token=\(token)"
        var queryItems = [
            URLQueryItem(name: "url", value: thumbURL),
            URLQueryItem(name: "X-Plex-Token", value: token),
            URLQueryItem(name: "minSize", value: "1"),
            URLQueryItem(name: "upscale", value: "1"),
        ]
        let dim = size ?? 600
        queryItems.append(URLQueryItem(name: "width", value: "\(dim)"))
        queryItems.append(URLQueryItem(name: "height", value: "\(dim)"))
        components?.queryItems = queryItems
        return components?.url
    }
}
