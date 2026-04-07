import AppKit
import SwiftUI
import Playback

/// Displays lyrics with synced scrolling (if timestamped) or static text.
struct LyricsView: View {

    let lyrics: LyricsResult
    let position: TimeInterval
    @Binding var isPinned: Bool
    let onSeek: (TimeInterval) -> Void
    let onDismiss: () -> Void
    @Environment(\.dynamicAccent) private var accentColor
    @State private var flashLineId: Int?

    var body: some View {
        let activeIndex = lyrics.activeLineIndex(at: position)
        ScrollViewReader { proxy in
            ScrollView(.vertical, showsIndicators: false) {
                LazyVStack(spacing: 6) {
                    ForEach(Array(lyrics.lines.enumerated()), id: \.element.id) { index, line in
                        let isActive = activeIndex == index
                        let isSynced = line.timestamp != nil
                        Text(line.text)
                            .font(isActive ? .body.bold() : .body)
                            .foregroundStyle(isActive ? accentColor : .secondary)
                            .opacity(isActive ? 1.0 : 0.9)
                            .frame(maxWidth: .infinity, alignment: .center)
                            .multilineTextAlignment(.center)
                            .padding(.vertical, 2)
                            .background(
                                RoundedRectangle(cornerRadius: 4)
                                    .fill(accentColor.opacity(flashLineId == line.id ? 0.25 : 0))
                            )
                            .contentShape(Rectangle())
                            .onTapGesture {
                                guard let ts = line.timestamp else { return }
                                flashLineId = line.id
                                onSeek(ts)
                                let tappedId = line.id
                                Task { @MainActor in
                                    try? await Task.sleep(for: .milliseconds(150))
                                    if flashLineId == tappedId {
                                        withAnimation(.easeOut(duration: 0.3)) {
                                            flashLineId = nil
                                        }
                                    }
                                }
                            }
                            .onHover { hovering in
                                if isSynced {
                                    if hovering { NSCursor.pointingHand.push() } else { NSCursor.pop() }
                                }
                            }
                            .id(line.id)
                            .animation(.easeInOut(duration: 0.3), value: isActive)
                    }

                    Text(lyrics.source == .plex ? "via Plex" : "via LRCLIB")
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                        .padding(.top, 8)
                }
                .padding(.top, 32)
                .padding(.bottom, 16)
                .padding(.horizontal, 8)
            }
            .onChange(of: activeIndex) { _, newIndex in
                if let newIndex {
                    withAnimation(.easeInOut(duration: 0.3)) {
                        proxy.scrollTo(lyrics.lines[newIndex].id, anchor: .center)
                    }
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color.black.opacity(0.80), in: RoundedRectangle(cornerRadius: 16))
        .compatGlassClear(in: RoundedRectangle(cornerRadius: 16))
        .clipShape(RoundedRectangle(cornerRadius: 16))
        .contentShape(Rectangle())
        .onTapGesture { }
        .overlay(alignment: .topLeading) {
            Button {
                onDismiss()
            } label: {
                Image(systemName: "xmark")
                    .font(.caption.bold())
                    .foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
            .padding(8)
            .contentShape(Rectangle())
        }
        .overlay(alignment: .topTrailing) {
            Button {
                isPinned.toggle()
            } label: {
                Image(systemName: isPinned ? "pin.fill" : "pin")
                    .font(.caption)
                    .foregroundStyle(isPinned ? .primary : .secondary)
                    .rotationEffect(.degrees(45))
            }
            .buttonStyle(.plain)
            .padding(8)
            .contentShape(Rectangle())
        }
    }
}

