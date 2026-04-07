import Foundation

/// Data needed to render the idle-screen album suggestion.
struct SuggestedAlbumData: Equatable {
    let artURL: URL?
    let tagline: TaglineParts
    let genres: [String]
}

/// A tagline split into segments so the album title can be rendered bold.
struct TaglineParts: Equatable {
    let segments: [Segment]

    enum Segment: Equatable {
        case text(String)
        case albumTitle(String)
    }
}
