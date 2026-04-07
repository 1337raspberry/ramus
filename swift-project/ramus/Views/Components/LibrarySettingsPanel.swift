import SwiftUI
import UniformTypeIdentifiers
import PlexAPI
import Cache
import Models

/// Settings sheet: server info, library picker, sync, playback config, sign out.
struct LibrarySettingsPanel: View {

    @Bindable var playbackVM: PlaybackViewModel
    @Bindable var libraryVM: LibraryViewModel
    var onSyncComplete: () -> Void
    var onSignOut: () -> Void
    @Environment(\.dismiss) private var dismiss

    @State private var isSyncingIncremental = false
    @State private var isSyncingGenres = false
    @State private var isSyncingFull = false
    private var anySyncing: Bool { isSyncingIncremental || isSyncingGenres || isSyncingFull }
    @State private var showingFileImporter = false
    @State private var cacheLimitText: String = ""


    var body: some View {
        VStack(spacing: 16) {
            // Close button + title
            HStack {
                Spacer()
                Button {
                    dismiss()
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .font(.title2)
                        .foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
            }

            Text("Settings")
                .font(.title2)
                .fontWeight(.semibold)

            // MARK: - Playback

            sectionHeader("Playback")

            Picker("Playback Mode", selection: $playbackVM.playbackMode) {
                Text("Direct Play").tag(PlaybackConfig.PlaybackMode.directPlay)
                Text("Transcode Lossless if Remote").tag(PlaybackConfig.PlaybackMode.transcodeLosslessRemote)
                Text("Always Transcode Lossless").tag(PlaybackConfig.PlaybackMode.transcodeLossless)
            }

            Stepper(
                "Prefetch \(playbackVM.lookaheadDepth) track\(playbackVM.lookaheadDepth == 1 ? "" : "s") ahead",
                value: $playbackVM.lookaheadDepth,
                in: 1...20
            )
            .font(.subheadline)

            Toggle("Show greeting messages", isOn: $playbackVM.showTaglines)
                .font(.subheadline)

            HStack {
                let cacheMB = Double(playbackVM.audioCacheSizeBytes) / 1_048_576
                Text("Audio cache: \(String(format: "%.1f", cacheMB)) MB")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Spacer()
                Button("Clear") {
                    playbackVM.clearAudioCache()
                }
                .buttonStyle(.borderless)
                .font(.caption)
                .foregroundStyle(.red)
            }

            HStack {
                Text("Cache limit:")
                    .font(.subheadline)
                TextField("2.0", text: $cacheLimitText)
                    .frame(width: 60)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit { commitCacheLimit() }
                Text("GB")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                Spacer()
            }

            HStack {
                let imgMB = Double(playbackVM.imageCacheSizeBytes) / 1_048_576
                Text("Image cache: \(String(format: "%.1f", imgMB)) MB")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Spacer()
                Button("Clear") {
                    playbackVM.clearImageCache()
                }
                .buttonStyle(.borderless)
                .font(.caption)
                .foregroundStyle(.red)
            }

            Divider()

            // MARK: - Library

            sectionHeader("Library")

            // Library picker (always show if libraries available)
            if !playbackVM.musicLibraries.isEmpty {
                Picker("Library", selection: $playbackVM.selectedLibrary) {
                    Text("Select Library").tag(LibrarySection?.none)
                    ForEach(playbackVM.musicLibraries, id: \.key) { lib in
                        Text(lib.title).tag(LibrarySection?.some(lib))
                    }
                }
                .frame(maxWidth: 300)
            }

            Picker("Auto-sync", selection: $playbackVM.syncIntervalHours) {
                Text("Off").tag(0)
                Text("1 hour").tag(1)
                Text("2 hours").tag(2)
                Text("4 hours").tag(4)
                Text("6 hours").tag(6)
                Text("8 hours").tag(8)
                Text("12 hours").tag(12)
                Text("24 hours").tag(24)
            }

            // Sync controls
            HStack(spacing: 12) {
                Button {
                    isSyncingIncremental = true
                    Task {
                        await playbackVM.startIncrementalSync()
                        onSyncComplete()
                        isSyncingIncremental = false
                    }
                } label: {
                    HStack(spacing: 6) {
                        if isSyncingIncremental {
                            ProgressView()
                                .controlSize(.small)
                        }
                        Text("Mini Sync — New Items Only")
                    }
                }
                .disabled(playbackVM.selectedLibrary == nil || anySyncing)

                Button {
                    isSyncingGenres = true
                    Task {
                        await playbackVM.startGenreSync()
                        onSyncComplete()
                        isSyncingGenres = false
                    }
                } label: {
                    HStack(spacing: 6) {
                        if isSyncingGenres {
                            ProgressView()
                                .controlSize(.small)
                        }
                        Text("Genre Full Sync")
                    }
                }
                .disabled(playbackVM.selectedLibrary == nil || anySyncing)

                Button {
                    isSyncingFull = true
                    Task {
                        await playbackVM.startSync()
                        onSyncComplete()
                        isSyncingFull = false
                    }
                } label: {
                    HStack(spacing: 6) {
                        if isSyncingFull {
                            ProgressView()
                                .controlSize(.small)
                        }
                        Text("Full Library Re-Sync")
                    }
                }
                .disabled(playbackVM.selectedLibrary == nil || anySyncing)
            }
            .fixedSize(horizontal: true, vertical: false)

            // Sync progress
            if playbackVM.isSyncing, let progress = playbackVM.syncProgress {
                VStack(spacing: 4) {
                    ProgressView(value: progress.fraction)
                        .frame(maxWidth: 250)
                    Text(progress.detail)
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                }
            }

            // Last sync time
            if let lastSync = playbackVM.lastSyncTime {
                Text("Last synced \(lastSync, format: .relative(presentation: .named))")
                    .font(.caption)
                    .foregroundStyle(.tertiary)
            }

            if let stats = playbackVM.cacheStats, stats.trackCount > 0 {
                Text("\(stats.artistCount) artists, \(stats.albumCount) albums, \(stats.trackCount) tracks")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            if let error = playbackVM.errorMessage {
                Text(error)
                    .foregroundStyle(.red)
                    .font(.caption)
                    .lineLimit(2)
            }

            genreSourceSection

            Toggle("Show genre hierarchy", isOn: $libraryVM.useHierarchy)
                .font(.subheadline)

            HStack {
                Text("Less")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Slider(value: $libraryVM.libraryPadding, in: 4...10, step: 1)
                Text("More")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Divider()

            // MARK: - Security

            sectionHeader("Security")

            Toggle("Refuse HTTP connections", isOn: $playbackVM.refuseHTTP)
                .font(.subheadline)

            Text("When enabled, ramus will only connect over HTTPS. If no secure connection is available, the connection will fail rather than fall back to unencrypted HTTP.")
                .font(.caption2)
                .foregroundStyle(.tertiary)
                .fixedSize(horizontal: false, vertical: true)

            Divider()

            // MARK: - Account

            sectionHeader("Account")

            // Server info — clickable to change
            if let name = playbackVM.serverName {
                Button {
                    playbackVM.signOut()
                    onSignOut()
                    dismiss()
                } label: {
                    HStack {
                        Image(systemName: "server.rack")
                            .foregroundStyle(.secondary)
                        Text(name)
                            .font(.subheadline)
                        Spacer()
                        Text("Change")
                            .font(.caption)
                            .foregroundStyle(.tertiary)
                    }
                    .padding(8)
                    .background(Color.white.opacity(0.05), in: RoundedRectangle(cornerRadius: 8))
                }
                .buttonStyle(.plain)
            }

            Button("Sign Out", role: .destructive) {
                playbackVM.signOut()
                onSignOut()
                dismiss()
            }
            .buttonStyle(.borderless)

        }
        .padding(24)
        .frame(minWidth: 500)
        .onAppear {
            playbackVM.refreshAudioCacheSize()
            playbackVM.refreshImageCacheSize()
            cacheLimitText = String(format: "%.1f", playbackVM.audioCacheLimitGB)
        }
        .fileImporter(
            isPresented: $showingFileImporter,
            allowedContentTypes: [.plainText, .text, .data],
            allowsMultipleSelection: false
        ) { result in
            switch result {
            case .success(let urls):
                guard let url = urls.first else { return }
                let gotAccess = url.startAccessingSecurityScopedResource()
                defer { if gotAccess { url.stopAccessingSecurityScopedResource() } }
                do {
                    // Try UTF-8 first, fall back to macOS Roman
                    let text: String
                    if let utf8 = try? String(contentsOf: url, encoding: .utf8) {
                        text = utf8
                    } else if let roman = try? String(contentsOf: url, encoding: .macOSRoman) {
                        text = roman
                    } else {
                        libraryVM.customImportError = "Could not read the file. Please save it as UTF-8."
                        return
                    }
                    try libraryVM.importCustomGenres(text: text, fileName: url.lastPathComponent)
                } catch {
                    libraryVM.customImportError = error.localizedDescription
                }
            case .failure(let error):
                libraryVM.customImportError = error.localizedDescription
            }
        }
    }

    // MARK: - Genre Source Section

    @ViewBuilder
    private var genreSourceSection: some View {
        if libraryVM.genreSource != .custom {
            Text("Genre data from Wikidata, licensed under CC0.")
                .font(.caption2)
                .foregroundStyle(.tertiary)
        }

        // Custom genre import
        HStack(spacing: 8) {
            Button {
                showingFileImporter = true
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: "doc.badge.plus")
                    Text(libraryVM.hasCustomGenreFile ? "Re-import Custom Genres..." : "Import Custom Genres...")
                }
            }
            .buttonStyle(.borderless)
            .font(.subheadline)

            if libraryVM.hasCustomGenreFile {
                if libraryVM.genreSource != .custom {
                    Button("Use Custom") {
                        libraryVM.genreSource = .custom
                    }
                    .buttonStyle(.borderless)
                    .font(.caption)
                    .foregroundStyle(.tint)
                } else {
                    Text("Active")
                        .font(.caption)
                        .foregroundStyle(.tint)
                }
            }

            Spacer()

            if libraryVM.hasCustomGenreFile {
                Button {
                    libraryVM.removeCustomGenres()
                } label: {
                    Image(systemName: "trash")
                        .font(.caption)
                        .foregroundStyle(.red)
                }
                .buttonStyle(.borderless)
                .help("Remove custom genre file")
            }
        }

        // Import status
        if let fileName = libraryVM.customImportFileName, libraryVM.hasCustomGenreFile {
            Text("Imported from: \(fileName)")
                .font(.caption2)
                .foregroundStyle(.tertiary)
        }

        if let error = libraryVM.customImportError {
            Text(error)
                .font(.caption)
                .foregroundStyle(.red)
                .lineLimit(4)
        }

        if !libraryVM.customImportWarnings.isEmpty {
            VStack(alignment: .leading, spacing: 2) {
                Text("\(libraryVM.customImportWarnings.count) warning\(libraryVM.customImportWarnings.count == 1 ? "" : "s"):")
                    .font(.caption)
                    .foregroundStyle(.orange)
                ForEach(libraryVM.customImportWarnings.prefix(5), id: \.self) { warning in
                    Text(warning)
                        .font(.caption2)
                        .foregroundStyle(.orange.opacity(0.8))
                }
                if libraryVM.customImportWarnings.count > 5 {
                    Text("...and \(libraryVM.customImportWarnings.count - 5) more")
                        .font(.caption2)
                        .foregroundStyle(.orange.opacity(0.6))
                }
            }
        }
    }

    private func commitCacheLimit() {
        guard let value = Double(cacheLimitText) else {
            cacheLimitText = String(format: "%.1f", playbackVM.audioCacheLimitGB)
            return
        }
        let clamped = max(0.1, min(50.0, value))
        playbackVM.audioCacheLimitGB = clamped
        cacheLimitText = String(format: "%.1f", clamped)
    }

    private func sectionHeader(_ title: String) -> some View {
        Text(title.uppercased())
            .font(.caption)
            .fontWeight(.semibold)
            .foregroundStyle(.tertiary)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}
