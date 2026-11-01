import Foundation

/// The folder inside the App Group container where the app writes the
/// widget's snapshot and cover. The group id comes from the bundle's
/// `RamusAppGroup` Info.plist key, so each build flavour uses its own.
public enum SharedContainer {
    public static let snapshotFile = "snapshot.json"
    public static let artFile = "art.jpg"

    public static func directory(bundle: Bundle = .main) -> URL? {
        guard let group = bundle.object(forInfoDictionaryKey: "RamusAppGroup") as? String,
              !group.isEmpty,
              let root = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group)
        else { return nil }
        return root.appendingPathComponent("widget", isDirectory: true)
    }
}
