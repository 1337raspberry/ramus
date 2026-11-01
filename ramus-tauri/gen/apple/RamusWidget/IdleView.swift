import RamusWidgetCore
import SwiftUI
import WidgetKit

/// The face shown when nothing is playing: one-tap ways to start
/// something, plus Resume when an earlier queue can be picked up.
struct IdleView: View {
    let large: Bool
    let resume: Bool
    let signedOut: Bool
    let snapshot: WidgetSnapshot?
    let art: UIImage?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 10) {
                RamusLogo()
                if large || signedOut {
                    Text(signedOut ? "Open ramus to sign in" : "Nothing playing")
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(.white.opacity(0.72))
                }
            }
            .padding(.bottom, large ? 14 : 12)
            if large {
                VStack(spacing: 8) { ForEach(actions) { row($0) } }
            } else {
                HStack(spacing: 8) { ForEach(actions) { tile($0) } }
            }
        }
        .padding(14)
        .foregroundStyle(.white)
        .disabled(signedOut)
        .opacity(signedOut ? 0.55 : 1)
    }

    private var actions: [IdleAction] {
        var list: [IdleAction] = []
        if resume && !signedOut { list.append(.resume) }
        list += [.favouriteAlbum, .favouriteSongs, .unplayedAlbum]
        return list
    }

    private func tile(_ action: IdleAction) -> some View {
        button(action) {
            VStack(alignment: .leading) {
                icon(action, size: 22)
                Spacer(minLength: 4)
                Text(action.shortLabel)
                    .font(.system(size: 12, weight: .semibold))
                    .lineSpacing(1)
                    .lineLimit(2)
                    .minimumScaleFactor(0.85)
            }
            .padding(11)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
            .background { tileBackground(action) }
            .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
        }
    }

    private func row(_ action: IdleAction) -> some View {
        button(action) {
            HStack(spacing: 14) {
                if action == .resume, let art {
                    Image(uiImage: art).resizable().aspectRatio(contentMode: .fill)
                        .frame(width: 44, height: 44)
                        .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
                        .padding(.leading, -4)
                } else {
                    icon(action, size: 26).frame(width: 30)
                }
                VStack(alignment: .leading, spacing: 2) {
                    Text(action.title).font(.system(size: 15, weight: .semibold)).lineLimit(1)
                    Text(subtitle(action)).font(.system(size: 12)).foregroundStyle(.white.opacity(0.68)).lineLimit(1)
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 14)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background { TileBackground() }
        }
    }

    @ViewBuilder
    private func tileBackground(_ action: IdleAction) -> some View {
        if action == .resume, let art {
            ZStack {
                Image(uiImage: art).resizable().aspectRatio(contentMode: .fill)
                LinearGradient(colors: [.black.opacity(0.05), .black.opacity(0.55)], startPoint: .top, endPoint: .bottom)
            }
        } else {
            TileBackground()
        }
    }

    private func subtitle(_ action: IdleAction) -> String {
        if action == .resume, let r = snapshot?.resume { return "\(r.title) · \(r.artist)" }
        return action.subtitle
    }

    @ViewBuilder
    private func icon(_ action: IdleAction, size: CGFloat) -> some View {
        switch action {
        case .resume: Image(systemName: "play.fill").font(.system(size: size * 0.8))
        case .favouriteAlbum: Image(systemName: "star.square").font(.system(size: size, weight: .regular))
        case .favouriteSongs: Image(systemName: "shuffle").font(.system(size: size * 0.85, weight: .semibold))
        case .unplayedAlbum: UnplayedIcon().frame(width: size, height: size)
        }
    }

    @ViewBuilder
    private func button(_ action: IdleAction, @ViewBuilder label: () -> some View) -> some View {
        switch action {
        case .resume: Button(intent: ResumeIntent(), label: label).buttonStyle(.plain)
        case .favouriteAlbum: Button(intent: RandomFavouriteAlbumIntent(), label: label).buttonStyle(.plain)
        case .favouriteSongs: Button(intent: ShuffleFavouriteSongsIntent(), label: label).buttonStyle(.plain)
        case .unplayedAlbum: Button(intent: RandomUnplayedAlbumIntent(), label: label).buttonStyle(.plain)
        }
    }
}

enum IdleAction: Identifiable {
    case resume, favouriteAlbum, favouriteSongs, unplayedAlbum

    var id: Self { self }

    var shortLabel: String {
        switch self {
        case .resume: "Resume"
        case .favouriteAlbum: "Random\nfav album"
        case .favouriteSongs: "Shuffle\nfav songs"
        case .unplayedAlbum: "Random\nunplayed"
        }
    }

    var title: String {
        switch self {
        case .resume: "Resume"
        case .favouriteAlbum: "Random favourite album"
        case .favouriteSongs: "Shuffle favourite songs"
        case .unplayedAlbum: "Random unplayed album"
        }
    }

    var subtitle: String {
        switch self {
        case .resume: ""
        case .favouriteAlbum: "From your starred albums"
        case .favouriteSongs: "Every starred track, mixed by artist"
        case .unplayedAlbum: "Something you haven't heard yet"
        }
    }
}

/// A record with a small "new" dot.
struct UnplayedIcon: View {
    var body: some View {
        GeometryReader { geo in
            let s = min(geo.size.width, geo.size.height)
            ZStack {
                Circle().strokeBorder(lineWidth: s * 0.075)
                Circle().frame(width: s * 0.18, height: s * 0.18)
                Circle()
                    .frame(width: s * 0.27, height: s * 0.27)
                    .position(x: s * 0.8, y: s * 0.2)
            }
            .frame(width: s, height: s)
        }
    }
}
