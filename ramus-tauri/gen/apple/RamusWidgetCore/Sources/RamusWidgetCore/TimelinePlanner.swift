import Foundation

/// Which face a widget entry shows.
public enum Face: Equatable {
    case nowPlaying(paused: Bool)
    case idle(resume: Bool)
    case signedOut
}

public struct PlannedEntry: Equatable {
    public let date: Date
    public let face: Face

    public init(date: Date, face: Face) {
        self.date = date
        self.face = face
    }
}

/// Plans a widget's timeline from one snapshot, so the widget moves on by
/// itself if the app stops writing: a playing snapshot turns paused shortly
/// after its track should have ended (the app would have written the next
/// track by then if it were still running), and any pause turns into the
/// idle face an hour after the last playback activity.
public enum TimelinePlanner {
    public static let grace: TimeInterval = 15
    public static let staleAfter: TimeInterval = 3_600

    public static func plan(_ snapshot: WidgetSnapshot?, now: Date) -> [PlannedEntry] {
        guard let s = snapshot else {
            return [PlannedEntry(date: now, face: .idle(resume: false))]
        }
        let idle = Face.idle(resume: s.resumable)
        switch s.state {
        case .signedOut:
            return [PlannedEntry(date: now, face: .signedOut)]
        case .idle:
            return [PlannedEntry(date: now, face: idle)]
        case .paused:
            return pausedThenIdle(now: now, idleAt: date(s.lastActivityAt + staleAfter), idle: idle)
        case .playing:
            let duration = s.track?.durationSec ?? 0
            guard duration > 0 else {
                return pausedThenIdle(now: now, idleAt: now.addingTimeInterval(staleAfter), idle: idle)
            }
            let pausedAt = date(s.positionAt + (duration - s.positionSec) + grace)
            let idleAt = pausedAt.addingTimeInterval(staleAfter)
            guard pausedAt > now else {
                return pausedThenIdle(now: now, idleAt: idleAt, idle: idle)
            }
            return [
                PlannedEntry(date: now, face: .nowPlaying(paused: false)),
                PlannedEntry(date: pausedAt, face: .nowPlaying(paused: true)),
                PlannedEntry(date: idleAt, face: idle),
            ]
        }
    }

    private static func pausedThenIdle(now: Date, idleAt: Date, idle: Face) -> [PlannedEntry] {
        guard idleAt > now else { return [PlannedEntry(date: now, face: idle)] }
        return [
            PlannedEntry(date: now, face: .nowPlaying(paused: true)),
            PlannedEntry(date: idleAt, face: idle),
        ]
    }

    private static func date(_ seconds: Double) -> Date {
        Date(timeIntervalSince1970: seconds)
    }
}
