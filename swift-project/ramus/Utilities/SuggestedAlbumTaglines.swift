import Foundation

/// One-liners shown below the suggested album on the idle screen.
enum SuggestedAlbumTaglines {

    /// Marker used to delimit the album title in template strings.
    private static let marker = "\u{FFFF}"

    private static let taglines: [String] = [
        "\(marker), at this time of year, at this time of day? localised entirely within your plex server?",
        "survey says?... \(marker)!",
        "i'll take \(marker) for 800 please ken",
        "y'all got any more of that \(marker)",
        "fun fact: \(marker) was recorded in a single take \n(citation: i made it up)",
        "i checked todays weather and its lookin like a 90% chance of \(marker)",
        "listen to \(marker) or you will have to forward this message to 10 friends before midnight",
        "i listened to \(marker) way before it was cool",
        "@EVERYONE GET IN HERE WE'RE LISTENING TO \(marker)",
        "\(marker) is lame and for nerds",
        "if \(marker) is so good then why no \(marker) 2?\nhmmmm?",
        "they dont want you to know this but you can just listen to \(marker).\nany time you want, its free, go for it",
        "just fyi you can turn these off in the settings menu.\nin the mean time listen to \(marker) maybe?",
        "i didnt get my grade 10 just for you to ignore my \(marker) recommendation",
        "look nobody wants to admit they listened to \(marker) nine times\nbut i did and im ashamed of myself",
        "\(marker) will look great on your 3x3 collage this week.\ngo for it",
        "i dont have a bit for this one.\njust click if you wanna listen to \(marker)",
        "imagine listening to the suggestions of a random number generator. \n( \(marker) is good tho )",
        "tfw no \(marker)-enjoying gf",
        "\(marker)maxxing so hard rn",
        "vc tonight imo we're listening to \(marker) and reading poetry",
        "\(marker). just listen to it. i know what you are",
        "i would simply just listen to \(marker) imo but i'm built diff",
        ">still hasn't listened to \(marker)\n>ngmi",
        "the industrial revolution and it's consequences\nhave led to this silly app recommending you \(marker)",
        ">open app\n>ask for recommendation that isn't \(marker)\n>app insist that its a good recommendation\n>check recommendation\n>its \(marker)",
        "if it's exactly 21:12 right now you can ignore the suggestion of \(marker) and just listen to rush instead.",
        "i put on my robe and wizard hat and queue up \(marker)",
    ]

    /// Shuffle bag — cycles through all taglines before repeating any.
    private static var bag: [String] = []

    /// Returns a tagline split into segments so the album title can be rendered bold.
    static func pick(albumTitle: String) -> TaglineParts {
        if bag.isEmpty { bag = taglines.shuffled() }
        let template = bag.removeLast()
        let parts = template.components(separatedBy: marker)
        var segments: [TaglineParts.Segment] = []
        for (i, part) in parts.enumerated() {
            if !part.isEmpty { segments.append(.text(part)) }
            if i < parts.count - 1 { segments.append(.albumTitle(albumTitle)) }
        }
        if segments.isEmpty { segments.append(.text(template)) }
        return TaglineParts(segments: segments)
    }
}
