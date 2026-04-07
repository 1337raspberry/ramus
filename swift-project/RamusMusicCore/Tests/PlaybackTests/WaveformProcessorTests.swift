import Testing
@testable import Playback

@Suite("WaveformProcessor")
struct WaveformProcessorTests {

    @Test("normalize converts dB to 0...1 range")
    func testNormalize() {
        let db: [Float] = [-40.0, -20.0, -10.0, 0.0]
        let result = WaveformProcessor.normalize(db)

        #expect(result.count == 4)
        // 0 dB = loudest → should be 1.0
        #expect(result[3] == 1.0)
        // All values should be in 0...1
        for v in result {
            #expect(v >= 0 && v <= 1)
        }
        // Quieter values should be smaller
        #expect(result[0] < result[1])
        #expect(result[1] < result[2])
        #expect(result[2] < result[3])
    }

    @Test("normalize handles empty input")
    func testNormalizeEmpty() {
        let result = WaveformProcessor.normalize([])
        #expect(result.isEmpty)
    }

    @Test("normalize handles full waveform dB levels")
    func testNormalizeFullWaveform() {
        let db: [Float] = [-30, -20, -10, -5, -3, -5, -10, -20, -30]
        let result = WaveformProcessor.normalize(db)

        #expect(result.count == 9)
        for v in result {
            #expect(v >= 0 && v <= 1)
        }
        // Middle should be louder than edges
        #expect(result[4] > result[0])
        #expect(result[4] > result[8])
    }
}
