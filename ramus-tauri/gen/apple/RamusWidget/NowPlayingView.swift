import RamusWidgetCore
import SwiftUI
import WidgetKit

/// Timing of the track an entry shows.
struct TrackTiming {
    let duration: Double
    /// When the track started, given the snapshot's position reading.
    let start: Date
    let paused: Bool
    /// The fraction played at the entry's date, for a paused face.
    let fraction: Double

    init(snapshot s: WidgetSnapshot, date: Date, paused: Bool) {
        duration = max(s.track?.durationSec ?? 0, 0)
        start = Date(timeIntervalSince1970: s.positionAt - s.positionSec)
        self.paused = paused
        // A playing snapshot shown paused (the app stopped writing) has run
        // on from its reading; a real pause hasn't.
        let elapsed = s.state == .playing ? max(date.timeIntervalSince1970 - s.positionAt, 0) : 0
        fraction = duration > 0 ? min(max((s.positionSec + elapsed) / duration, 0), 1) : 0
    }

    var end: Date { start.addingTimeInterval(duration) }
    var live: Bool { !paused && duration > 0 }
}

struct NowPlayingView: View {
    let large: Bool
    let snapshot: WidgetSnapshot
    let colors: WidgetSnapshot.Colors?
    let art: UIImage?
    let timing: TrackTiming

    private var accent: Color { Color(hex: colors?.accent ?? "f16280") }
    private var levels: [Double] { snapshot.waveform ?? [] }

    var body: some View {
        if large { largeBody } else { mediumBody }
    }

    private var mediumBody: some View {
        HStack(spacing: 12) {
            cover.frame(width: 130, height: 130)
                .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                .shadow(color: .black.opacity(0.35), radius: 7, y: 4)
            VStack(alignment: .leading, spacing: 0) {
                titles(titleSize: 15, artistSize: 13)
                    .padding(.top, 6)
                Spacer(minLength: 6)
                WaveformView(levels: levels, accent: accent, timing: timing, snapshot: snapshot)
                    .frame(height: 30)
                Spacer(minLength: 6)
                transport(small: 20, main: 24)
                    .padding(.horizontal, 4)
                    .padding(.bottom, 4)
            }
        }
        .padding(14)
    }

    private var largeBody: some View {
        ZStack(alignment: .bottom) {
            // The cover fills the whole widget (cropped at the sides) under
            // a shade that dims all of it slightly and deepens towards the
            // bottom, where the text sits.
            Color.clear
                .overlay { cover }
                .clipped()
            LinearGradient(
                stops: [
                    .init(color: .black.opacity(0.13), location: 0),
                    .init(color: .black.opacity(0.24), location: 0.35),
                    .init(color: .black.opacity(0.55), location: 0.68),
                    .init(color: .black.opacity(0.79), location: 1),
                ],
                startPoint: .top, endPoint: .bottom
            )
            VStack(alignment: .leading, spacing: 0) {
                titles(titleSize: 17, artistSize: 14)
                    .padding(.bottom, 10)
                WaveformView(levels: levels, accent: accent, timing: timing, snapshot: snapshot)
                    .frame(height: 34)
                times.padding(.top, 2)
                transport(small: 22, main: 28)
                    .padding(.horizontal, 52)
                    .padding(.top, 2)
            }
            .padding(.horizontal, 18)
            .padding(.bottom, 20)
        }
    }

    @ViewBuilder
    private var cover: some View {
        if let art {
            Image(uiImage: art).resizable().aspectRatio(contentMode: .fill)
        } else {
            Rectangle().fill(.white.opacity(0.12))
                .overlay(Image(systemName: "music.note").font(.system(size: 34)).foregroundStyle(.white.opacity(0.5)))
        }
    }

    private func titles(titleSize: CGFloat, artistSize: CGFloat) -> some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(snapshot.track?.title ?? "")
                .font(.system(size: titleSize, weight: .bold))
                .lineLimit(1)
            Text(snapshot.track?.artist ?? "")
                .font(.system(size: artistSize))
                .foregroundStyle(.white.opacity(0.78))
                .lineLimit(1)
            if let cast = snapshot.cast {
                Text("on \(cast)")
                    .font(.system(size: 11.5))
                    .foregroundStyle(.white.opacity(0.55))
                    .lineLimit(1)
            }
        }
        .foregroundStyle(.white)
    }

    @ViewBuilder
    private var times: some View {
        HStack {
            if timing.live {
                // Timer text claims the full width it is offered, so each
                // half gets its own and aligns within it.
                Text(timerInterval: timing.start...timing.end, countsDown: false)
                    .frame(maxWidth: .infinity, alignment: .leading)
                (Text("−") + Text(timerInterval: timing.start...timing.end, countsDown: true))
                    .multilineTextAlignment(.trailing)
                    .frame(maxWidth: .infinity, alignment: .trailing)
            } else {
                Text(Self.clock(timing.fraction * timing.duration))
                Spacer()
                Text("−" + Self.clock((1 - timing.fraction) * timing.duration))
            }
        }
        .font(.system(size: 11).monospacedDigit())
        .foregroundStyle(.white.opacity(0.6))
    }

    private func transport(small: CGFloat, main: CGFloat) -> some View {
        let key = snapshot.track?.ratingKey
        let rev = snapshot.revision
        return HStack {
            Button(intent: PreviousIntent(ratingKey: key, revision: rev)) {
                Image(systemName: "backward.end.fill").font(.system(size: small))
            }
            Spacer()
            Button(intent: PlayIntent(play: timing.paused, ratingKey: key, revision: rev)) {
                Image(systemName: timing.paused ? "play.fill" : "pause.fill").font(.system(size: main))
            }
            Spacer()
            Button(intent: NextIntent(ratingKey: key, revision: rev)) {
                Image(systemName: "forward.end.fill").font(.system(size: small))
            }
        }
        .buttonStyle(.plain)
        .foregroundStyle(.white)
    }

    static func clock(_ seconds: Double) -> String {
        let s = max(Int(seconds.rounded()), 0)
        return String(format: "%d:%02d", s / 60, s % 60)
    }
}

struct SilhouetteShape: Shape {
    let levels: [Double]
    func path(in rect: CGRect) -> Path {
        Path(WaveformPath.silhouette(levels: levels, in: rect))
    }
}

/// The app's waveform silhouette, with the played part in the album's
/// accent and tap-to-seek zones laid over it.
struct WaveformView: View {
    static let zones = 12

    let levels: [Double]
    let accent: Color
    let timing: TrackTiming
    let snapshot: WidgetSnapshot

    var body: some View {
        // Without levels the shape is a thin line along the middle.
        let shape = SilhouetteShape(levels: levels.isEmpty ? [0.05, 0.05] : levels)
        GeometryReader { geo in
            ZStack(alignment: .leading) {
                shape.fill(.white.opacity(0.25))
                if timing.live {
                    // iOS animates a timer-driven progress bar by itself;
                    // stretched to the box and cut to the silhouette, it
                    // fills the waveform as the track plays.
                    ProgressView(timerInterval: timing.start...timing.end, countsDown: false) {
                        EmptyView()
                    } currentValueLabel: {
                        EmptyView()
                    }
                    .progressViewStyle(.linear)
                    .tint(accent)
                    .scaleEffect(x: 1, y: max(geo.size.height / 3, 1), anchor: .center)
                    .frame(width: geo.size.width, height: geo.size.height)
                    .mask(shape)
                } else {
                    shape.fill(accent)
                        .mask(alignment: .leading) {
                            Rectangle().frame(width: geo.size.width * timing.fraction)
                        }
                }
                HStack(spacing: 0) {
                    ForEach(0..<Self.zones, id: \.self) { zone in
                        Button(intent: SeekIntent(
                            fraction: WaveformPath.seekFraction(zone: zone, zones: Self.zones),
                            ratingKey: snapshot.track?.ratingKey,
                            revision: snapshot.revision
                        )) {
                            Color.clear.contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
        }
    }
}
