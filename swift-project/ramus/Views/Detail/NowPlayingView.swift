import SwiftUI
import AppKit
import NukeUI
import Models
import Playback
import GenreTree
import os.log

#if DEBUG
private let renderLog = Logger(subsystem: "com.raspsoft.ramus", category: "RenderDebug")
#endif

/// Detail panel: album art, track info, transport, volume, genres.
///
/// Takes ViewModel references instead of closures so SwiftUI can diff
/// the struct by identity — parent re-evaluations skip body when refs
/// are unchanged. Re-renders on position ticks, track changes, and
/// metadata updates (genres, year, studio, lyrics, favourite state).
struct NowPlayingView: View {

    let playbackVM: PlaybackViewModel
    let libraryVM: LibraryViewModel
    let eqPanel: EqualizerPanelController
    var suggestedAlbum: SuggestedAlbumData?
    var suggestedAlbumSource: Album?

    @State private var showLyrics = false
    @State private var lyricsLoading = false
    @State private var lyricsFetchDone = false
    @State private var lyricsPinned = false


    private var player: AudioPlayer { playbackVM.player }
    private var state: PlayerState { player.state }

    private var hasUpcomingQueue: Bool {
        state.queue.count > state.queueIndex + 1
    }

    var body: some View {
        #if DEBUG
        let _ = renderLog.info("NowPlayingView.body")
        #endif
        if let track = state.currentTrack {
            VStack(spacing: 0) {
                nowPlayingContent(track: track)
                if hasUpcomingQueue {
                    queueChevronHint
                }
            }
        } else if let suggestion = suggestedAlbum {
            SuggestedAlbumView(
                data: suggestion,
                onPlay: { if let album = suggestedAlbumSource { libraryVM.playAlbumInPlace(album) } },
                onGenreTap: { libraryVM.navigateToGenre(named: $0) }
            )
        }
    }

    private var queueChevronHint: some View {
        Image(systemName: "chevron.down")
            .font(.caption2)
            .foregroundStyle(.tertiary)
            .frame(maxWidth: .infinity)
            .padding(.top, 4)
            .padding(.bottom, 12)
            .symbolEffect(.bounce, options: .speed(0.4), value: hasUpcomingQueue)
    }

    @ViewBuilder
    private var eqButton: some View {
        let enabled = playbackVM.equalizerEnabled
        Button {
            let mouse = NSEvent.mouseLocation
            eqPanel.toggle(at: mouse, playbackVM: playbackVM)
        } label: {
            Image(systemName: "slider.vertical.3")
                .font(.system(size: 15))
                .foregroundStyle(enabled ? AnyShapeStyle(.tint) : AnyShapeStyle(.secondary))
        }
        .buttonStyle(.plain)
        .help("Equalizer")
    }

    private func nowPlayingContent(track: Track) -> some View {
        GeometryReader { geo in
            let artMax = max(geo.size.height - 280, 100)
            nowPlayingStack(track: track, artMaxHeight: artMax)
        }
    }

    private func nowPlayingStack(track: Track, artMaxHeight: CGFloat) -> some View {
        let albumFav = playbackVM.currentAlbumIsFavourite
        let trackFav = track.isFavourite
        let genres = playbackVM.currentAlbumGenres
        let studio = playbackVM.currentAlbumStudio
        let artURL = playbackVM.artURL(for: track.thumb, size: Int(artMaxHeight * 2))
        let volumeBinding = Binding<Float>(
            get: { self.player.volume },
            set: { self.player.volume = $0 }
        )

        return VStack(spacing: 8) {
            // TOP: Artist, Album, Year
            VStack(spacing: 4) {
                Button {
                    if let artistId = playbackVM.currentAlbumArtistSourceId {
                        libraryVM.loadAlbumsByArtist(sourceId: artistId)
                    }
                } label: {
                    Text(track.hasTrackArtist
                        ? "\(track.artistName) (\(track.displayArtist))"
                        : track.artistName)
                        .font(.title.bold())
                        .lineLimit(2)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .buttonStyle(.plain)
                .onHover { hovering in
                    if hovering { NSCursor.pointingHand.push() } else { NSCursor.pop() }
                }

                HStack {
                    Text(track.albumTitle)
                        .font(.title2)
                    Spacer()
                    Button { playbackVM.toggleCurrentAlbumFavourite(using: libraryVM) } label: {
                        Image(systemName: albumFav ? "star.fill" : "star")
                            .foregroundStyle(albumFav ? Color.yellow : Color.secondary)
                    }
                    .buttonStyle(.plain)
                }

                if let year = playbackVM.currentAlbumYear {
                    Button {
                        libraryVM.loadAlbumsByYear(year)
                    } label: {
                        Text(String(year))
                            .font(.subheadline)
                            .opacity(0.5)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .buttonStyle(.plain)
                    .onHover { hovering in
                        if hovering { NSCursor.pointingHand.push() } else { NSCursor.pop() }
                    }
                }
            }

            // MIDDLE: Art (tap to flip to lyrics), Volume, Track, Seek, Controls
            VStack(spacing: 8) {
                LazyImage(url: artURL) { phase in
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
                .clipShape(RoundedRectangle(cornerRadius: 16))
                .overlay {
                    Group {
                        if let lyrics = playbackVM.currentLyrics {
                            LyricsView(
                                lyrics: lyrics,
                                position: player.position,
                                isPinned: $lyricsPinned,
                                onSeek: { playbackVM.seek(to: $0) },
                                onDismiss: {
                                    withAnimation(.easeIn(duration: 0.25)) {
                                        showLyrics = false
                                    }
                                }
                            )
                        } else {
                            Color.clear
                                .background(Color.black.opacity(0.80), in: RoundedRectangle(cornerRadius: 16))
                                .compatGlassClear(in: RoundedRectangle(cornerRadius: 16))
                                .overlay {
                                    if lyricsFetchDone {
                                        Text("No lyrics available")
                                            .font(.subheadline)
                                            .foregroundStyle(.secondary)
                                    } else if lyricsLoading {
                                        ProgressView()
                                    }
                                }
                        }
                    }
                    .clipShape(RoundedRectangle(cornerRadius: 16))
                    .padding(-1)
                    .scaleEffect(showLyrics ? 1 : 0.001)
                    .opacity(showLyrics ? 1 : 0)
                    .allowsHitTesting(showLyrics)
                }
                .contentShape(Rectangle())
                .onTapGesture {
                    if !showLyrics {
                        // Fetch lyrics if needed — start immediately, delay spinner
                        if playbackVM.currentLyrics == nil && !lyricsLoading {
                            lyricsFetchDone = false
                            Task {
                                await playbackVM.fetchLyricsForCurrentTrack()
                                lyricsLoading = false
                                if playbackVM.currentLyrics == nil {
                                    lyricsFetchDone = true
                                }
                            }
                            Task { @MainActor in
                                try? await Task.sleep(for: .milliseconds(500))
                                if playbackVM.currentLyrics == nil && !lyricsFetchDone {
                                    lyricsLoading = true
                                }
                            }
                        }
                    }
                    if showLyrics {
                        withAnimation(.easeIn(duration: 0.25)) {
                            showLyrics = false
                        }
                    } else {
                        withAnimation(.spring(duration: 0.45, bounce: 0.3)) {
                            showLyrics = true
                        }
                    }
                }
                .onChange(of: state.currentTrack?.ratingKey) { _, _ in
                    playbackVM.clearLyrics()
                    lyricsFetchDone = false
                    if lyricsPinned {
                        // Stay on lyrics, auto-fetch for new track
                        lyricsLoading = true
                        Task {
                            await playbackVM.fetchLyricsForCurrentTrack()
                            lyricsLoading = false
                            if playbackVM.currentLyrics == nil {
                                lyricsFetchDone = true
                            }
                        }
                    } else {
                        showLyrics = false
                        lyricsLoading = false
                    }
                }
                .onHover { hovering in
                    if hovering { NSCursor.pointingHand.push() } else { NSCursor.pop() }
                }
                .frame(maxHeight: artMaxHeight)

                SlimVolumeSlider(value: volumeBinding)
                    .frame(height: 12)

                HStack {
                    Text(track.title)
                        .font(.headline)
                        .lineLimit(1)
                    Spacer()
                    eqButton
                    Button { playbackVM.toggleCurrentTrackFavourite(using: libraryVM) } label: {
                        Image(systemName: trackFav ? "star.fill" : "star")
                            .foregroundStyle(trackFav ? Color.yellow : Color.secondary)
                    }
                    .buttonStyle(.plain)
                }

                if player.duration > 0 {
                    WaveformSeekBar(
                        levels: player.waveformLevels,
                        position: player.position,
                        duration: player.duration,
                        bufferedFraction: player.bufferedFraction,
                        isBuffering: player.isBuffering,
                        onSeek: { playbackVM.seek(to: $0) }
                    )
                }

                HStack(spacing: 20) {
                    Spacer()
                    Button { playbackVM.previous() } label: {
                        Image(systemName: "backward.fill")
                    }
                    Button { playbackVM.togglePlayPause() } label: {
                        Image(systemName: state.status == .playing ? "pause.fill" : "play.fill")
                            .font(.title)
                    }
                    Button { playbackVM.next() } label: {
                        Image(systemName: "forward.fill")
                    }
                    Spacer()
                }
                .buttonStyle(.borderless)
            }

            // BOTTOM: Record label, format, genres
            VStack(spacing: 6) {
                if !genres.isEmpty {
                    FlowLayout(spacing: 6) {
                        ForEach(genres, id: \.self) { genre in
                            Button { libraryVM.navigateToGenre(named: genre) } label: {
                                Text(genre)
                                    .font(.subheadline.bold())
                                    .foregroundStyle(.tint)
                            }
                            .buttonStyle(.plain)
                            .help(libraryVM.genreMapper?.matchGenre(genre)?.shortSummary ?? "")
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

                if studio != nil || track.formatDescription != nil {
                    HStack {
                        if let studio {
                            Label(studio, systemImage: "building.2")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        if let format = track.formatDescription {
                            Label(format, systemImage: "waveform")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
            }
        }
        .padding()
    }
}

// MARK: - Slim Volume Slider

/// A minimal volume slider: thin track line with small circular thumb.
private struct SlimVolumeSlider: View {
    @Binding var value: Float
    @Environment(\.dynamicAccent) private var accentColor

    var body: some View {
        GeometryReader { geo in
            let width = geo.size.width
            let thumbX = CGFloat(value) * width

            ZStack(alignment: .leading) {
                // Track
                Rectangle()
                    .fill(Color.secondary.opacity(0.3))
                    .frame(height: 2)

                // Filled portion
                Rectangle()
                    .fill(accentColor.opacity(0.7))
                    .frame(width: thumbX, height: 2)

                // Thumb
                Circle()
                    .fill(accentColor)
                    .frame(width: 8, height: 8)
                    .offset(x: thumbX - 4)
            }
            .frame(height: geo.size.height)
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { drag in
                        let fraction = Float(drag.location.x / width)
                        value = min(max(fraction, 0), 1)
                    }
            )
        }
    }
}

// MARK: - Flow Layout

/// Simple horizontal flow layout for genre text.
struct FlowLayout: Layout {
    var spacing: CGFloat = 6

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let result = arrange(proposal: proposal, subviews: subviews)
        return result.size
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let result = arrange(proposal: proposal, subviews: subviews)
        for (index, position) in result.positions.enumerated() {
            subviews[index].place(
                at: CGPoint(x: bounds.minX + position.x, y: bounds.minY + position.y),
                proposal: .unspecified
            )
        }
    }

    private func arrange(proposal: ProposedViewSize, subviews: Subviews) -> (size: CGSize, positions: [CGPoint]) {
        let maxWidth = proposal.width ?? .infinity
        var positions: [CGPoint] = []
        var x: CGFloat = 0
        var y: CGFloat = 0
        var rowHeight: CGFloat = 0
        var totalHeight: CGFloat = 0

        for subview in subviews {
            let size = subview.sizeThatFits(.unspecified)
            if x + size.width > maxWidth && x > 0 {
                x = 0
                y += rowHeight + spacing
                rowHeight = 0
            }
            positions.append(CGPoint(x: x, y: y))
            rowHeight = max(rowHeight, size.height)
            x += size.width + spacing
            totalHeight = y + rowHeight
        }

        return (CGSize(width: maxWidth, height: totalHeight), positions)
    }
}

