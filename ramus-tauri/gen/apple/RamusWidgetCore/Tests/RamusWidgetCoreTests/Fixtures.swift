import Foundation
@testable import RamusWidgetCore

func snapshotJSON(
    state: String = "playing",
    durationSec: Double = 200,
    positionSec: Double = 50,
    positionAt: Double = 1_000,
    lastActivityAt: Double = 1_000,
    resumable: Bool = true,
    schema: Int = 1
) -> Data {
    let track = state == "idle" || state == "signedOut" ? "null" : """
    {"ratingKey":"9","title":"Tessellate","artist":"alt-J","album":"An Awesome Wave","durationSec":\(durationSec)}
    """
    return Data("""
    {"schema":\(schema),"revision":4,"state":"\(state)","track":\(track),
     "positionSec":\(positionSec),"positionAt":\(positionAt),"lastActivityAt":\(lastActivityAt),
     "resumable":\(resumable),"resume":null,
     "colors":{"topLeft":"1a1e45","topRight":"2d572f","bottomRight":"4f6731","bottomLeft":"11323e","accent":"3ddb7a"},
     "waveform":[0.1,0.5,1.0],"cast":null}
    """.utf8)
}

func snapshot(_ data: Data) -> WidgetSnapshot {
    try! WidgetSnapshot.decode(data)
}

func at(_ seconds: Double) -> Date { Date(timeIntervalSince1970: seconds) }
