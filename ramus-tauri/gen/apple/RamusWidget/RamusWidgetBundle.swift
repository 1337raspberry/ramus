import RamusWidgetCore
import SwiftUI
import WidgetKit

struct RamusWidgetView: View {
    @Environment(\.widgetFamily) private var family
    let entry: RamusEntry

    var body: some View {
        let large = family == .systemLarge
        Group {
            switch entry.face {
            case .nowPlaying(let paused):
                if let snapshot = entry.snapshot {
                    NowPlayingView(
                        large: large,
                        snapshot: snapshot,
                        colors: entry.colors,
                        art: entry.art,
                        timing: TrackTiming(snapshot: snapshot, date: entry.date, paused: paused)
                    )
                } else {
                    IdleView(large: large, resume: false, signedOut: false, snapshot: nil, art: nil)
                }
            case .idle(let resume):
                IdleView(large: large, resume: resume, signedOut: false, snapshot: entry.snapshot, art: entry.art)
            case .signedOut:
                IdleView(large: large, resume: false, signedOut: true, snapshot: nil, art: nil)
            }
        }
        .containerBackground(for: .widget) { background }
    }

    @ViewBuilder
    private var background: some View {
        if case .nowPlaying = entry.face, entry.snapshot != nil {
            CornerGradient(corners: Corners(entry.colors))
        } else {
            CornerGradient(corners: .pinkySunset)
        }
    }
}

struct RamusPlayerWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "RamusPlayer", provider: Provider()) { entry in
            RamusWidgetView(entry: entry)
        }
        .configurationDisplayName("ramus")
        .description("Control what's playing, or start something new.")
        .supportedFamilies([.systemMedium, .systemLarge])
        .contentMarginsDisabled()
    }
}

@main
struct RamusWidgetBundle: WidgetBundle {
    var body: some Widget { RamusPlayerWidget() }
}
