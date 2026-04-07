import Foundation

extension TimeInterval {
    /// Format as "m:ss" (e.g. "3:07").
    var formattedDuration: String {
        let mins = Int(self) / 60
        let secs = Int(self) % 60
        return String(format: "%d:%02d", mins, secs)
    }
}
