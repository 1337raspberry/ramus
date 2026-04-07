import Foundation

/// Pure functions for processing Plex loudness data into waveform display values.
public enum WaveformProcessor: Sendable {

    /// Convert dB loudness levels to normalized 0...1 amplitudes.
    /// Input: array of negative dB values (e.g. -35.0 quiet, -3.0 loud).
    /// Output: array of 0...1 values suitable for drawing.
    public static func normalize(_ dbLevels: [Float]) -> [Float] {
        guard !dbLevels.isEmpty else { return [] }

        // Convert dB to linear amplitude
        var linear = dbLevels.map { pow(10.0, $0 / 20.0) }

        // Normalize to 0...1
        let maxVal = linear.max() ?? 1.0
        guard maxVal > 0 else { return linear }
        linear = linear.map { $0 / maxVal }

        return linear
    }
}
