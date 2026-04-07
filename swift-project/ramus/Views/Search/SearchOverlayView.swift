import SwiftUI
import Search
import os

private let log = Logger(subsystem: "com.raspsoft.ramus", category: "SearchOverlay")

struct SearchOverlayView: View {

    @Bindable var viewModel: SearchViewModel
    var artURL: (String?) -> URL?
    var onSelect: (SearchResult) -> Void
    var onLoadGrid: ([SearchResult]) -> Void
    var onPlayNext: (SearchResult) -> Void = { _ in }
    var onAddToQueue: (SearchResult) -> Void = { _ in }

    @Environment(\.dynamicAccent) private var accentColor
    @FocusState private var isFieldFocused: Bool

    private var hasResults: Bool { !viewModel.results.isEmpty }

    var body: some View {
        VStack(spacing: 0) {
            searchField

            if hasResults {
                Divider()
                resultsList
            }
        }
        .frame(width: hasResults ? 500 : 350)
        .background(.white.opacity(0.15), in: RoundedRectangle(cornerRadius: 12))
        .compatGlassRegular(in: RoundedRectangle(cornerRadius: 12))
        .shadow(radius: hasResults ? 20 : 8, y: hasResults ? 10 : 4)
        .onAppear {
            let t = ContinuousClock.now
            let prefix = viewModel.query
            viewModel.query = ""
            isFieldFocused = true
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) {
                viewModel.query = prefix
            }
            log.info("onAppear — \(ContinuousClock.now - t, privacy: .public)")
        }
        .onChange(of: viewModel.isVisible) { _, visible in
            if visible {
                let prefix = viewModel.query
                viewModel.query = ""
                isFieldFocused = true
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) {
                    viewModel.query = prefix
                }
            }
        }
    }

    // MARK: - Search Field

    private var searchField: some View {
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass")
                .foregroundStyle(.secondary)
                .font(.caption)

            TextField("/genre @artist %album !track $year", text: $viewModel.query)
                .textFieldStyle(.plain)
                .font(.system(size: 13))
                .focused($isFieldFocused)
                .onChange(of: viewModel.query) { _, _ in
                    viewModel.queryChanged()
                }
                .onKeyPress(.upArrow) {
                    viewModel.moveUp()
                    return .handled
                }
                .onKeyPress(.downArrow) {
                    viewModel.moveDown()
                    return .handled
                }
                .onKeyPress(.return) {
                    if NSEvent.modifierFlags.contains(.shift) {
                        let albumResults = viewModel.results.filter { $0.kind == .album }
                        if !albumResults.isEmpty {
                            onLoadGrid(albumResults)
                        }
                    } else if let result = viewModel.confirm() {
                        onSelect(result)
                    }
                    return .handled
                }
                .onKeyPress(.escape) {
                    viewModel.dismiss()
                    return .handled
                }

            if viewModel.isSearching {
                ProgressView()
                    .controlSize(.mini)
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
    }

    // MARK: - Results List

    private var resultsList: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    let albums = viewModel.results.filter { $0.kind == .album }
                    let tracks = viewModel.results.filter { $0.kind == .track }

                    if !albums.isEmpty {
                        SectionHeaderLabel(title: "Albums")
                            .padding(.horizontal, 12)
                            .padding(.top, 8)
                            .padding(.bottom, 4)
                        ForEach(Array(albums.enumerated()), id: \.element.id) { index, result in
                            let globalIndex = index
                            albumRow(result, isSelected: globalIndex == viewModel.selectedIndex)
                                .id(result.id)
                                .onTapGesture {
                                    viewModel.selectedIndex = globalIndex
                                    onSelect(result)
                                }
                        }
                    }

                    if !tracks.isEmpty {
                        SectionHeaderLabel(title: "Tracks")
                            .padding(.horizontal, 12)
                            .padding(.top, 8)
                            .padding(.bottom, 4)
                        ForEach(Array(tracks.enumerated()), id: \.element.id) { index, result in
                            let globalIndex = albums.count + index
                            trackRow(result, isSelected: globalIndex == viewModel.selectedIndex)
                                .id(result.id)
                                .onTapGesture {
                                    viewModel.selectedIndex = globalIndex
                                    onSelect(result)
                                }
                        }
                    }
                }
            }
            .frame(maxHeight: 360)
            .onChange(of: viewModel.selectedIndex) { _, newIndex in
                if newIndex < viewModel.results.count {
                    proxy.scrollTo(viewModel.results[newIndex].id, anchor: .center)
                }
            }
        }
    }

    // MARK: - Row Views

    private func albumRow(_ result: SearchResult, isSelected: Bool) -> some View {
        HStack(spacing: 10) {
            AlbumThumbnailView(url: artURL(result.albumArtPath), size: 36, cornerRadius: 4, placeholderIcon: true)

            VStack(alignment: .leading, spacing: 2) {
                Text(result.albumTitle)
                    .font(.system(size: 13, weight: .semibold))
                HStack(spacing: 4) {
                    Text(result.artistName)
                    if let year = result.year {
                        Text("(\(String(year)))")
                    }
                }
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
            }

            Spacer()

            Button { onPlayNext(result) } label: {
                Image(systemName: "text.line.first.and.arrowtriangle.forward")
                    .font(.system(size: 10))
                    .frame(width: 24, height: 24)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .help("Play Next")

            Button { onAddToQueue(result) } label: {
                Image(systemName: "text.append")
                    .font(.system(size: 10))
                    .frame(width: 24, height: 24)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .help("Add to Queue")
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 12)
        .padding(.vertical, 5)
        .background(isSelected ? accentColor.opacity(0.2) : .clear)
        .contentShape(Rectangle())
    }

    private func trackRow(_ result: SearchResult, isSelected: Bool) -> some View {
        HStack(spacing: 10) {
            AlbumThumbnailView(url: artURL(result.albumArtPath), size: 36, cornerRadius: 4, placeholderIcon: true)

            VStack(alignment: .leading, spacing: 2) {
                Text(result.trackTitle ?? "")
                    .font(.system(size: 13))
                Text("\(result.trackArtist ?? result.artistName) — \(result.albumTitle)")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
            }

            Spacer()

            Button { onPlayNext(result) } label: {
                Image(systemName: "text.line.first.and.arrowtriangle.forward")
                    .font(.system(size: 10))
                    .frame(width: 24, height: 24)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .help("Play Next")

            Button { onAddToQueue(result) } label: {
                Image(systemName: "text.append")
                    .font(.system(size: 10))
                    .frame(width: 24, height: 24)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .help("Add to Queue")
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 12)
        .padding(.vertical, 5)
        .background(isSelected ? accentColor.opacity(0.2) : .clear)
        .contentShape(Rectangle())
    }
}
