import ImageIO
import RamusWidgetCore
import UIKit
import WidgetKit

struct RamusEntry: TimelineEntry {
    let date: Date
    let face: Face
    let snapshot: WidgetSnapshot?
    let art: UIImage?
    /// The snapshot's colours as painted (see `WidgetSnapshot.Colors.resolved`).
    let colors: WidgetSnapshot.Colors?
}

/// Builds the widget's timeline from the snapshot the app last wrote. The
/// app reloads the widget whenever it writes a new one, so the timeline
/// itself never asks to be refreshed.
struct Provider: TimelineProvider {
    func placeholder(in context: Context) -> RamusEntry {
        RamusEntry(date: .now, face: .idle(resume: false), snapshot: nil, art: nil, colors: nil)
    }

    func getSnapshot(in context: Context, completion: @escaping (RamusEntry) -> Void) {
        completion(entries(now: .now).first ?? placeholder(in: context))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<RamusEntry>) -> Void) {
        completion(Timeline(entries: entries(now: .now), policy: .never))
    }

    private func entries(now: Date) -> [RamusEntry] {
        let dir = SharedContainer.directory()
        let snapshot = dir.flatMap { WidgetSnapshot.load(from: $0) }
        let art = dir.flatMap { Self.loadArt($0.appendingPathComponent(SharedContainer.artFile)) }
        let colors = snapshot?.colors?.resolved(cover: art?.cgImage)
        return TimelinePlanner.plan(snapshot, now: now).map {
            RamusEntry(date: $0.date, face: $0.face, snapshot: snapshot, art: art, colors: colors)
        }
    }

    /// The cover, decoded straight to a bounded size: widgets run under a
    /// tight memory limit, so a full-size original is never decoded.
    static func loadArt(_ url: URL) -> UIImage? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: 1000,
        ]
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { return nil }
        return UIImage(cgImage: image)
    }
}
