import Foundation
import Models
import os

private let log = Logger(subsystem: "com.raspsoft.ramus", category: "AudioPlayer")

/// mpv-based audio player for Plex audio playback.
/// mpv handles all formats natively (FLAC, Opus, OGG, etc.) with built-in
/// gapless playback and playlist management. Download-first caching retained
/// for lower latency and offline use; HLS transcode retained for bandwidth
/// savings on remote lossless.
@Observable @MainActor
public final class AudioPlayer {

    // MARK: - Observable State & Callbacks

    public private(set) var state: PlayerState = PlayerState()
    /// Playback position — updated by mpv's time-pos property observation.
    /// Separated from PlayerState so position ticks don't trigger re-renders
    /// of views that only observe queue/track/status.
    public private(set) var position: Duration = 0
    /// Duration of the current track — metadata initially, refined by mpv.
    public private(set) var duration: Duration = 0
    public private(set) var isLoading: Bool = false
    /// True when mpv is paused waiting for cache data.
    public private(set) var isBuffering: Bool = false
    public private(set) var waveformLevels: [Float]?
    /// Cache buffering progress (0...1). Derived from mpv's cache-buffering-state (0-100).
    public private(set) var bufferedFraction: Double = 0

    /// Session identifier for linking timeline reports to transcode streams.
    /// Regenerated on each loadQueue call.
    public private(set) var playSessionID: String = UUID().uuidString

    /// Closure to fetch loudness levels for a track ratingKey. Set by ViewModel.
    @ObservationIgnored public var levelsProvider: (@Sendable (String) async -> [Float]?)?
    /// Called when the current track changes (including auto-advance). Set by ViewModel.
    /// Fires after playback has started — safe for session reporting.
    @ObservationIgnored public var onTrackChange: (() -> Void)?
    /// Called when track info is set but playback hasn't started yet.
    /// Use for UI refresh (album info, accent color) without session side-effects.
    @ObservationIgnored public var onTrackInfoChange: (() -> Void)?
    /// Monotonic timestamp of the last position broadcast to prevent >30fps UI updates.
    @ObservationIgnored private var lastPositionTick: ContinuousClock.Instant = .now
    /// Called on pause/resume/stop/seek state changes. Set by ViewModel for session reporting.
    @ObservationIgnored public var onPlaybackStateChange: ((PlaybackStatus) -> Void)?
    /// Called when a track finishes naturally (auto-advance). Passes the ended track for scrobbling.
    @ObservationIgnored public var onTrackEnded: ((Track) -> Void)?

    // MARK: - Private State

    private let mpv = MPVController()

    // Server info
    private var serverURL: URL?
    private var token: String?
    private var clientIdentifier: String?
    private var config: PlaybackConfig = .default

    // Download cache
    private let downloadSession: URLSession
    private var downloadCache: [String: URL] = [:]  // track.id → local file URL
    private var cacheSizes: [String: Int64] = [:]    // track.id → file size bytes (avoid stat on evict)
    private var cacheAccessOrder: [String] = []      // oldest → newest for LRU eviction
    @ObservationIgnored private var prefetchTask: Task<Void, Never>?
    /// Raw mpv paused-for-cache state, tracked separately so isBuffering can
    /// combine download-loading and cache-stall states for the UI.
    @ObservationIgnored private var isMpvBuffering: Bool = false
    /// Delayed task that shows the buffering indicator only if loading takes > 300ms.
    /// Prevents flashing the indicator for near-instant loads (cached/LAN tracks).
    @ObservationIgnored private var bufferDelayTask: Task<Void, Never>?
    private let cacheDir: URL

    /// Lossless codecs that benefit from transcoding (bandwidth savings).
    private static let losslessCodecs: Set<String> = ["flac", "alac", "wav", "aiff", "aif", "pcm"]

    /// Tracks we're currently downloading (to avoid double-downloads).
    private var activeDownloads: Set<String> = []
    /// Handle to the current buildAndLoadPlaylist task — cancelled on new loadQueue.
    private var buildTask: Task<Void, Never>?
    /// When non-nil, `onPlaylistPosChange` ignores events until this index is reached.
    /// Prevents spurious scrobbles when starting mid-queue — `loadFile("replace")`
    /// briefly reports playlist-pos=0 before `playlistPlayIndex` seeks to the real start.
    private var expectedStartIndex: Int?

    // MARK: - Init / Deinit

    public init() {
        let sessionConfig = URLSessionConfiguration.ephemeral
        sessionConfig.timeoutIntervalForResource = 120
        sessionConfig.waitsForConnectivity = true
        self.downloadSession = URLSession(configuration: sessionConfig)

        self.cacheDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("ramus_audio_cache", isDirectory: true)
        try? FileManager.default.createDirectory(at: cacheDir, withIntermediateDirectories: true)

        // Sweep orphaned files from prior sessions — downloadCache is empty at
        // init so every file on disk is an orphan. Capture the file list
        // synchronously so no download can race with the deletion.
        let orphans = (try? FileManager.default.contentsOfDirectory(
            at: cacheDir, includingPropertiesForKeys: nil,
            options: .skipsHiddenFiles
        )) ?? []
        if !orphans.isEmpty {
            Task.detached {
                for fileURL in orphans { try? FileManager.default.removeItem(at: fileURL) }
                log.info("startup sweep: removed \(orphans.count, privacy: .public) orphaned cache file(s)")
            }
        }

        setupMPVCallbacks()
    }

    // MARK: - Configuration

    public func configure(serverURL: URL, token: String, clientIdentifier: String) {
        self.serverURL = serverURL
        self.token = token
        self.clientIdentifier = clientIdentifier
    }

    /// Whether the current connection is remote (non-LAN).
    /// Set by PlaybackViewModel when the connection is established or changes.
    public var isRemote: Bool = false { didSet {
        log.info("isRemote = \(self.isRemote, privacy: .public)")
    }}

    /// Update server URL and token for future URL construction.
    /// Does not affect the currently playing item (its URL is already loaded in mpv).
    public func updateServerConnection(serverURL: URL, token: String, isRemote: Bool? = nil) {
        let oldHost = self.serverURL?.host() ?? "none"
        let newHost = serverURL.host() ?? "none"
        log.info("connection updated: \(oldHost, privacy: .public) -> \(newHost, privacy: .public)")
        self.serverURL = serverURL
        self.token = token
        if let isRemote { self.isRemote = isRemote }
        prefetchTask?.cancel()
    }

    /// Update playback config without re-connecting.
    public func updateConfig(_ config: PlaybackConfig) {
        self.config = config
    }

    // MARK: - Playback Controls

    /// Load a queue of tracks and start playing from the given index.
    public func loadQueue(_ tracks: [Track], startAt index: Int = 0) {
        buildTask?.cancel()
        prefetchTask?.cancel()
        bufferDelayTask?.cancel()
        onPlaybackStateChange?(.stopped)
        // Pause immediately so the old track stops audibly. Don't use mpv.stop()
        // — that triggers an idle event that races with the new playlist setup.
        // loadFile("replace") in buildAndLoadPlaylist will clear the old playlist;
        // setPause(false) there resumes with the new track.
        mpv.setPause(true)
        playSessionID = UUID().uuidString
        state.queue = tracks
        state.queueIndex = index

        guard !tracks.isEmpty, index < tracks.count else {
            state.status = .stopped
            state.currentTrack = nil
            return
        }

        // For mid-queue starts, mpv briefly reports playlist-pos=0 before
        // playlistPlayIndex seeks to the real start. Suppress spurious scrobbles.
        expectedStartIndex = index > 0 ? index : nil

        log.info("loadQueue: \(tracks.count, privacy: .public) tracks, starting at [\(index, privacy: .public)] \"\(tracks[index].title, privacy: .public)\" by \(tracks[index].artistName, privacy: .public)")

        state.currentTrack = tracks[index]
        duration = tracks[index].duration
        position = 0
        isLoading = true
        waveformLevels = nil
        bufferedFraction = 0

        // Show buffering indicator only if loading takes > 300ms.
        // Avoids flashing the indicator for cached/LAN tracks.
        bufferDelayTask?.cancel()
        bufferDelayTask = Task {
            try? await Task.sleep(for: .milliseconds(300))
            guard !Task.isCancelled, self.isLoading else { return }
            self.isBuffering = true
        }

        onTrackInfoChange?()
        fetchWaveform(for: tracks[index])

        // Build mpv playlist. For each track, resolve URL (cached local → remote).
        // Load first track with "replace", rest with "append".
        buildTask = Task { await buildAndLoadPlaylist(startAt: index) }
    }

    public func pause() {
        mpv.setPause(true)
        state.status = .paused
        onPlaybackStateChange?(.paused)
    }

    public func resume() {
        mpv.setPause(false)
        state.status = .playing
        onPlaybackStateChange?(.playing)
    }

    public func togglePlayPause() {
        if state.status == .playing { pause() } else { resume() }
    }

    public func next() {
        let nextIndex = state.queueIndex + 1
        guard nextIndex < state.queue.count else { stop(); return }
        jumpToIndex(nextIndex)
    }

    public func previous() {
        if position > 3 {
            seek(to: 0)
            return
        }
        let prevIndex = state.queueIndex - 1
        guard prevIndex >= 0 else { seek(to: 0); return }
        jumpToIndex(prevIndex)
    }

    public func stop() {
        prefetchTask?.cancel()
        mpv.stop()
        state.status = .stopped
        state.currentTrack = nil
        position = 0
        duration = 0
        isLoading = false
        isBuffering = false
        isMpvBuffering = false
        bufferDelayTask?.cancel()
        expectedStartIndex = nil
        onPlaybackStateChange?(.stopped)
    }

    public func seek(to time: TimeInterval) {
        let itemDuration = duration
        let clampedTime = (itemDuration.isFinite && itemDuration > 0)
            ? min(time, max(0, itemDuration - 0.5))
            : time
        position = clampedTime
        mpv.seek(to: clampedTime)
    }

    public var volume: Float {
        get { Float(mpv.getVolume() / 100.0) }
        set {
            let clamped = max(0, min(1, newValue))
            mpv.setVolume(Double(clamped) * 100.0)
        }
    }

    // MARK: - Equalizer

    /// Standard 10-band graphic EQ center frequencies in Hz.
    public static let eqFrequencies: [Int] = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000]

    /// Apply a 10-band equalizer via mpv's lavfi audio filter chain.
    /// Each element in `bands` is gain in dB (expected range -12…+12).
    /// Pass `enabled: false` to clear all audio filters.
    public func applyEqualizer(enabled: Bool, bands: [Float]) {
        let safeBands = bands.map { $0.isFinite ? $0 : Float(0) }
        guard enabled, safeBands.contains(where: { $0 != 0 }) else {
            mpv.setAudioFilters("no")
            return
        }
        let posix = Locale(identifier: "en_US_POSIX")
        let filters = zip(Self.eqFrequencies, safeBands).map { freq, gain in
            "equalizer=f=\(freq):width_type=o:w=1:g=\(String(format: "%.1f", locale: posix, gain))"
        }
        let af = "lavfi=[" + filters.joined(separator: ",") + "]"
        mpv.setAudioFilters(af)
    }

    // MARK: - Queue Manipulation

    /// Append tracks to the end of the queue. Auto-starts if player is stopped.
    public func appendToQueue(_ tracks: [Track]) {
        let wasStopped = state.queue.isEmpty || state.status == .stopped
        let startIndex = state.queue.count
        state.queue.append(contentsOf: tracks)
        if wasStopped {
            loadQueue(state.queue, startAt: startIndex)
        } else {
            // Append to mpv's playlist too
            for track in tracks {
                let url = resolvePlaybackURL(for: track)
                mpv.loadFile(url, mode: "append")
            }
            prefetchTracks(after: state.queueIndex)
        }
    }

    /// Insert tracks immediately after the currently playing track. Auto-starts if player is stopped.
    public func insertNext(_ tracks: [Track]) {
        if state.queue.isEmpty || state.status == .stopped {
            let startIndex = state.queue.count
            state.queue.append(contentsOf: tracks)
            loadQueue(state.queue, startAt: startIndex)
            return
        }
        let insertIndex = min(state.queueIndex + 1, state.queue.count)
        state.queue.insert(contentsOf: tracks, at: insertIndex)
        // Insert into mpv's playlist at the same positions
        for (offset, track) in tracks.enumerated() {
            let url = resolvePlaybackURL(for: track)
            mpv.loadFileAt(url, index: insertIndex + offset)
        }
        prefetchTracks(after: state.queueIndex)
    }

    /// Remove a track at the given queue index. No-op if it's the currently playing track.
    public func removeFromQueue(at index: Int) {
        guard index >= 0, index < state.queue.count, index != state.queueIndex else { return }
        state.queue.remove(at: index)
        mpv.playlistRemove(index)
        if index < state.queueIndex { state.queueIndex -= 1 }
    }

    /// Jump to an arbitrary queue index and begin playing.
    public func jumpToIndex(_ index: Int) {
        guard index >= 0, index < state.queue.count else { return }
        prefetchTask?.cancel()
        state.queueIndex = index
        let track = state.queue[index]
        state.currentTrack = track
        duration = track.duration
        position = 0
        isLoading = true
        waveformLevels = nil

        mpv.playlistPlayIndex(index)
        mpv.setPause(false)
        state.status = .playing
        onTrackChange?()
        fetchWaveform(for: track)
        prefetchTracks(after: index)
    }

    // MARK: - Rating Updates

    /// Update the favourite status on a track in the current queue (and currentTrack if matching).
    public func updateTrackFavourite(ratingKey: String, isFavourite: Bool) {
        if let current = state.currentTrack, current.ratingKey == ratingKey {
            state.currentTrack = Track(
                ratingKey: current.ratingKey, title: current.title,
                artistName: current.artistName, trackArtist: current.trackArtist,
                albumTitle: current.albumTitle, albumKey: current.albumKey,
                index: current.index, duration: current.duration,
                codec: current.codec, partKey: current.partKey,
                thumb: current.thumb,
                isFavourite: isFavourite, bitrate: current.bitrate,
                discNumber: current.discNumber
            )
        }
        for i in state.queue.indices where state.queue[i].ratingKey == ratingKey {
            let t = state.queue[i]
            state.queue[i] = Track(
                ratingKey: t.ratingKey, title: t.title,
                artistName: t.artistName, trackArtist: t.trackArtist,
                albumTitle: t.albumTitle, albumKey: t.albumKey,
                index: t.index, duration: t.duration,
                codec: t.codec, partKey: t.partKey,
                thumb: t.thumb,
                isFavourite: isFavourite, bitrate: t.bitrate,
                discNumber: t.discNumber
            )
        }
    }

    // MARK: - Playlist Building

    /// Build the mpv playlist from our track queue. Uses cached files where available,
    /// remote URLs otherwise. Downloads happen in the background via prefetch.
    private func buildAndLoadPlaylist(startAt startIndex: Int) async {
        guard !state.queue.isEmpty else { return }

        let startTrack = state.queue[startIndex]

        // Download the starting track if it's a direct-play track (for instant start).
        // Transcode tracks stream directly — no download needed.
        let startURL = await resolveOrDownloadURL(for: startTrack)

        // If a new loadQueue was called while we were downloading, abort.
        guard !Task.isCancelled else { return }

        // Build playlist in queue order. Single pass, no stop-and-rebuild.
        // "replace" on first entry clears any previous playlist.
        for (i, track) in state.queue.enumerated() {
            let url: String
            if i == startIndex {
                url = startURL
            } else {
                url = resolvePlaybackURL(for: track)
            }
            mpv.loadFile(url, mode: i == 0 ? "replace" : "append")
        }

        // Start playing from the correct index
        if startIndex > 0 {
            mpv.playlistPlayIndex(startIndex)
        }
        mpv.setPause(false)
        state.status = .playing
        isLoading = false
        bufferDelayTask?.cancel()
        if !isMpvBuffering { isBuffering = false }
        onTrackChange?()

        // Start prefetching upcoming tracks
        prefetchTracks(after: startIndex)
    }

    /// Resolve a playback URL synchronously — cached local file or remote HTTP.
    private func resolvePlaybackURL(for track: Track) -> String {
        // Check cache first
        if let cachedURL = downloadCache[track.id],
           FileManager.default.fileExists(atPath: cachedURL.path) {
            touchCacheEntry(track.id)
            return cachedURL.absoluteString
        }

        // Build remote URL
        guard let url = buildPlaybackURL(for: track, session: playSessionID) else {
            log.warning("no playback URL for \"\(track.title, privacy: .public)\"")
            return ""
        }
        return url.absoluteString
    }

    /// Resolve URL, downloading first if beneficial (e.g. for the starting track).
    private func resolveOrDownloadURL(for track: Track) async -> String {
        // Check cache
        if let cachedURL = downloadCache[track.id],
           FileManager.default.fileExists(atPath: cachedURL.path) {
            touchCacheEntry(track.id)
            return cachedURL.absoluteString
        }

        // For transcode tracks, stream directly — no download
        if shouldTranscode(for: track) {
            guard let url = buildPlaybackURL(for: track, session: playSessionID) else { return "" }
            return url.absoluteString
        }

        // For direct-play tracks, try to download first for better experience
        guard let url = buildPlaybackURL(for: track, session: playSessionID) else { return "" }
        if let localURL = await downloadTrack(track: track, from: url) {
            return localURL.absoluteString
        }

        // Download failed — stream directly (mpv handles all formats)
        return url.absoluteString
    }

    // MARK: - URL Building

    /// Single source of truth for whether a track should be transcoded.
    /// With mpv, transcoding is ONLY for bandwidth savings — never for codec compatibility.
    private func shouldTranscode(for track: Track) -> Bool {
        let codec = (track.codec ?? "").lowercased()
        let isLossless = Self.losslessCodecs.contains(codec)

        switch config.playbackMode {
        case .directPlay:
            return false
        case .transcodeLosslessRemote:
            return isLossless && isRemote
        case .transcodeLossless:
            return isLossless
        }
    }

    /// Build a playback URL for a track (direct play or HLS transcode).
    private func buildPlaybackURL(for track: Track, session: String) -> URL? {
        guard let serverURL, let token, let clientIdentifier else { return nil }

        if !shouldTranscode(for: track) {
            guard let partKey = track.partKey,
                  let url = TranscodeHelper.buildDirectPlayURL(serverURL: serverURL, partKey: partKey, token: token)
            else {
                log.error("direct play track \"\(track.title, privacy: .public)\" has no partKey")
                return nil
            }
            return url
        }

        return TranscodeHelper.buildHLSURL(
            serverURL: serverURL, token: token,
            trackRatingKey: track.ratingKey, clientIdentifier: clientIdentifier,
            session: session
        )
    }

    // MARK: - MPV Callbacks

    private func setupMPVCallbacks() {
        mpv.onPositionChange = { [weak self] pos in
            guard let self else { return }
            // Throttle to ~30fps — boundary values (0 / near-end) always pass
            // to keep seek-to-start and end-of-track responsive.
            let now = ContinuousClock.now
            let isBoundary = pos <= 0 || (self.duration > 0 && pos >= self.duration - 0.5)
            if !isBoundary && now - self.lastPositionTick < .milliseconds(33) { return }
            self.lastPositionTick = now
            if pos != self.position {
                self.position = pos
            }
        }

        mpv.onDurationChange = { [weak self] dur in
            guard let self, dur > 0 else { return }
            if dur != self.duration {
                self.duration = dur
            }
        }

        mpv.onPlaylistPosChange = { [weak self] newPos in
            guard let self, newPos >= 0, newPos < self.state.queue.count else { return }

            // During initial playlist load for mid-queue starts, mpv briefly
            // reports playlist-pos=0 before playlistPlayIndex seeks to the
            // real start index. Ignore all position events until we arrive.
            if let expected = self.expectedStartIndex {
                if newPos == expected { self.expectedStartIndex = nil }
                return
            }

            let oldIndex = self.state.queueIndex
            guard newPos != oldIndex else { return }

            // Natural advance — scrobble the ended track
            if oldIndex < self.state.queue.count {
                let endedTrack = self.state.queue[oldIndex]
                self.onTrackEnded?(endedTrack)
            }

            // Update state
            self.state.queueIndex = newPos
            let track = self.state.queue[newPos]
            self.state.currentTrack = track
            self.duration = track.duration
            self.position = 0
            self.isLoading = false
            self.bufferDelayTask?.cancel()
            if !self.isMpvBuffering { self.isBuffering = false }
            self.waveformLevels = nil
            self.bufferedFraction = 0

            log.info("gapless advance to [\(newPos, privacy: .public)] \"\(track.title, privacy: .public)\"")

            self.onTrackChange?()
            self.fetchWaveform(for: track)
            self.prefetchTracks(after: newPos)
        }

        mpv.onPauseChange = { [weak self] paused in
            guard let self, !self.isLoading else { return }
            // Only update if it's mpv-initiated (not our explicit pause/resume calls)
            let expected: PlaybackStatus = paused ? .paused : .playing
            if self.state.status != expected && self.state.status != .stopped {
                self.state.status = expected
            }
        }

        mpv.onBufferingChange = { [weak self] buffering in
            guard let self else { return }
            self.isMpvBuffering = buffering
            let shouldBuffer = buffering || self.isLoading
            if shouldBuffer != self.isBuffering {
                self.isBuffering = shouldBuffer
            }
            if buffering {
                log.info("\"\(self.state.currentTrack?.title ?? "?", privacy: .public)\" — buffering")
            }
        }

        mpv.onCacheStateChange = { [weak self] percent in
            guard let self else { return }
            let fraction = min(Double(percent) / 100.0, 1.0)
            if fraction != self.bufferedFraction {
                self.bufferedFraction = fraction
            }
        }

        mpv.onFileLoaded = { [weak self] in
            guard let self else { return }
            self.isLoading = false
            self.bufferDelayTask?.cancel()
            if !self.isMpvBuffering { self.isBuffering = false }
        }

        mpv.onFileEnded = { [weak self] reason in
            guard let self, !self.isLoading else { return }
            switch reason {
            case .eof:
                // Natural track end. Scrobble is handled by onPlaylistPosChange
                // (which fires for all non-last tracks) and onIdleActive (which
                // handles queue completion for the last track). No action needed
                // here — avoids a race where queueIndex may already be advanced
                // by onPlaylistPosChange before this callback fires.
                break
            case .error(let err):
                log.error("playback error: \(err, privacy: .public)")
                let nextIndex = self.state.queueIndex + 1
                if nextIndex < self.state.queue.count {
                    self.jumpToIndex(nextIndex)
                } else {
                    self.stop()
                }
            default:
                break
            }
        }

        mpv.onIdleActive = { [weak self] in
            guard let self else { return }
            // Ignore idle events while loading a new queue — loadfile "replace"
            // briefly idles before the new playlist starts.
            guard !self.isLoading else { return }
            if self.state.status == .playing {
                // Scrobble the last track before stopping
                if self.state.queueIndex < self.state.queue.count {
                    let lastTrack = self.state.queue[self.state.queueIndex]
                    self.onTrackEnded?(lastTrack)
                }
                log.info("mpv idle — queue complete")
                self.state.status = .stopped
                self.state.currentTrack = nil
                self.onPlaybackStateChange?(.stopped)
            }
        }
    }

    // MARK: - Waveform

    private func fetchWaveform(for track: Track) {
        let ratingKey = track.ratingKey
        Task { [levelsProvider] in
            if let levels = await levelsProvider?(ratingKey) {
                guard self.state.currentTrack?.ratingKey == ratingKey else { return }
                self.waveformLevels = WaveformProcessor.normalize(levels)
            }
        }
    }

    // MARK: - Prefetch

    /// Prefetch upcoming direct-play tracks to cache. Once downloaded, update
    /// the mpv playlist entry to use the local file for better gapless performance.
    private func prefetchTracks(after index: Int) {
        prefetchTask?.cancel()
        let depth = config.lookaheadDepth
        guard depth > 0 else { return }
        prefetchTask = Task {
            for offset in 1...depth {
                let nextIndex = index + offset
                guard nextIndex < state.queue.count else { break }
                guard !Task.isCancelled else { break }

                let track = state.queue[nextIndex]

                // HLS transcode tracks don't need prefetch — mpv streams them on demand
                if shouldTranscode(for: track) {
                    log.info("prefetch [\(nextIndex, privacy: .public)] \"\(track.title, privacy: .public)\" — HLS path, skipping")
                    continue
                }

                if downloadCache[track.id] != nil {
                    log.info("prefetch [\(nextIndex, privacy: .public)] \"\(track.title, privacy: .public)\" — already cached")
                    touchCacheEntry(track.id)
                    continue
                }

                guard let url = buildPlaybackURL(for: track, session: UUID().uuidString) else {
                    log.warning("prefetch [\(nextIndex, privacy: .public)] \"\(track.title, privacy: .public)\" — no URL")
                    continue
                }

                log.info("prefetch [\(nextIndex, privacy: .public)] \"\(track.title, privacy: .public)\" starting")
                let result = await downloadTrack(track: track, from: url)
                if Task.isCancelled { break }
                if result != nil {
                    log.info("prefetch [\(nextIndex, privacy: .public)] \"\(track.title, privacy: .public)\" — cached, mpv will use via gapless")
                } else {
                    log.warning("prefetch [\(nextIndex, privacy: .public)] \"\(track.title, privacy: .public)\" — download failed")
                }
            }
        }
    }

    // MARK: - Download Engine

    /// Download a track to a local temp file. Retries with increasing backoff.
    /// Rebuilds the URL on each retry in case the server connection changed.
    private func downloadTrack(track: Track, from url: URL, maxRetries: Int = 3) async -> URL? {
        let id = track.id
        // Return cached file if we already have it
        if let cached = downloadCache[id],
           FileManager.default.fileExists(atPath: cached.path) {
            return cached
        }

        // Avoid double-downloads
        guard !activeDownloads.contains(id) else { return nil }
        activeDownloads.insert(id)
        defer { activeDownloads.remove(id) }

        let label = track.title
        var currentURL = url

        for attempt in 0...maxRetries {
            guard !Task.isCancelled else {
                log.info("download \"\(label, privacy: .public)\" cancelled — prefetch restarted")
                return nil
            }

            // Rebuild URL on retry — server may have changed via ConnectionMonitor
            if attempt > 0, let fresh = buildPlaybackURL(for: track, session: UUID().uuidString) {
                let oldHost = currentURL.host() ?? "?"
                let newHost = fresh.host() ?? "?"
                if oldHost != newHost {
                    log.info("download \"\(label, privacy: .public)\" attempt \(attempt + 1, privacy: .public): URL host changed \(oldHost, privacy: .public) -> \(newHost, privacy: .public)")
                }
                currentURL = fresh
            }

            let host = currentURL.host() ?? "unknown"
            let start = ContinuousClock.now
            log.info("download \"\(label, privacy: .public)\" attempt \(attempt + 1, privacy: .public)/\(maxRetries + 1, privacy: .public) from \(host, privacy: .public)")

            do {
                let (tempURL, response) = try await downloadSession.download(from: currentURL)
                let elapsed = ContinuousClock.now - start
                guard let http = response as? HTTPURLResponse,
                      (200...299).contains(http.statusCode) else {
                    let code = (response as? HTTPURLResponse)?.statusCode ?? -1
                    log.warning("download \"\(label, privacy: .public)\" attempt \(attempt + 1, privacy: .public): HTTP \(code, privacy: .public) after \(elapsed, privacy: .public)")
                    continue
                }

                let allowedExtensions: Set<String> = ["flac", "alac", "m4a", "mp3", "aac", "wav", "aiff", "ogg", "opus", "mp2", "bin"]
                let rawExt = currentURL.pathExtension.isEmpty
                    ? (track.codec?.lowercased() ?? "bin")
                    : currentURL.pathExtension.lowercased()
                let ext = allowedExtensions.contains(rawExt) ? rawExt : "bin"
                // Sanitize ID to prevent path traversal from malicious ratingKeys
                let safeID = id.filter { $0.isLetter || $0.isNumber || $0 == "-" || $0 == "_" }
                guard !safeID.isEmpty else { return nil }
                let dest = cacheDir.appendingPathComponent("\(safeID).\(ext)")
                try? FileManager.default.removeItem(at: dest)
                try FileManager.default.moveItem(at: tempURL, to: dest)

                let size = (try? FileManager.default.attributesOfItem(atPath: dest.path)[.size] as? Int64) ?? 0
                downloadCache[id] = dest
                cacheSizes[id] = size
                touchCacheEntry(id)

                log.info("download \"\(label, privacy: .public)\" complete: \(size / 1024, privacy: .public)KB in \(elapsed, privacy: .public)")
                return dest
            } catch {
                let elapsed = ContinuousClock.now - start
                if attempt < maxRetries {
                    let backoff = attempt + 1
                    log.warning("download \"\(label, privacy: .public)\" attempt \(attempt + 1, privacy: .public) failed after \(elapsed, privacy: .public): \(error.localizedDescription, privacy: .public). Retrying in \(backoff, privacy: .public)s...")
                    try? await Task.sleep(for: .seconds(backoff))
                } else {
                    log.error("download \"\(label, privacy: .public)\" FAILED after \(maxRetries + 1, privacy: .public) attempts (\(elapsed, privacy: .public)): \(error.localizedDescription, privacy: .public)")
                }
            }
        }
        return nil
    }

    // MARK: - LRU Cache Management

    private func touchCacheEntry(_ id: String) {
        cacheAccessOrder.removeAll { $0 == id }
        cacheAccessOrder.append(id)
        evictCacheIfNeeded()
    }

    private func evictCacheIfNeeded() {
        let limitBytes = config.audioCacheLimitBytes
        let currentTrackID = state.queue.indices.contains(state.queueIndex)
            ? state.queue[state.queueIndex].id : nil

        // Use stored sizes — no filesystem stat() calls needed.
        var sizedEntries: [(id: String, size: Int64)] = []
        var totalBytes: Int64 = 0
        for id in cacheAccessOrder {
            guard downloadCache[id] != nil else { continue }
            let size = cacheSizes[id] ?? 0
            sizedEntries.append((id, size))
            totalBytes += size
        }

        // Evict oldest entries (front of sizedEntries) until under limit.
        var i = 0
        while totalBytes > limitBytes && i < sizedEntries.count {
            let (id, size) = sizedEntries[i]
            if id == currentTrackID { i += 1; continue }
            sizedEntries.remove(at: i)
            cacheAccessOrder.removeAll { $0 == id }
            if let fileURL = downloadCache.removeValue(forKey: id) {
                cacheSizes.removeValue(forKey: id)
                totalBytes -= size
                try? FileManager.default.removeItem(at: fileURL)
                log.info("cache evict: \(id, privacy: .public) (\(size / 1024, privacy: .public) KB), total now \(totalBytes / 1_048_576, privacy: .public) MB")
            }
        }
    }

    /// Total size in bytes of all cached audio files.
    public func audioCacheSizeBytes() async -> Int64 {
        let dir = cacheDir
        return await Task.detached {
            let fm = FileManager.default
            let contents = (try? fm.contentsOfDirectory(
                at: dir, includingPropertiesForKeys: [.fileSizeKey],
                options: .skipsHiddenFiles
            )) ?? []
            var total: Int64 = 0
            for fileURL in contents {
                total += (try? fileURL.resourceValues(forKeys: [.fileSizeKey]).fileSize)
                    .map { Int64($0) } ?? 0
            }
            return total
        }.value
    }

    /// Delete all cached audio files and reset the cache index.
    public func clearAudioCache() {
        prefetchTask?.cancel()
        downloadCache.removeAll()
        cacheSizes.removeAll()
        cacheAccessOrder.removeAll()
        let dir = cacheDir
        Task.detached {
            try? FileManager.default.removeItem(at: dir)
            try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        }
    }
}
