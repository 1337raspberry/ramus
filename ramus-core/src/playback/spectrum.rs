//! Per-track spectrogram analysis for the focus-mode visualiser.
//!
//! Pure DSP: takes a slice of mono `f32` samples (average stereo upstream)
//! and produces a time-indexed spectrogram the frontend looks up by
//! playback position.
//!
//! libmpv does not expose real-time PCM frames, and running a parallel
//! live decoder would fight mpv for Plex's concurrent-download slot.
//! Pre-computing at prefetch-time and indexing by `time-pos` gives perfect
//! sync with audio output (mpv's reported position is ground truth) and
//! avoids cross-decoder latency guesses.
//!
//! Pipeline: STFT with a Hann window, group magnitude bins into 128
//! log-spaced bands, compress and quantise each band to a `u8`. A 5-minute
//! track at 30 Hz × 128 bands × 1 byte ≈ 1.15 MB.

use serde::{Deserialize, Serialize};
use std::f32::consts::PI;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use realfft::num_complex::Complex;
use realfft::RealFftPlanner;

/// Magic bytes prepended to every `.spec` cache file. Sanity check so
/// `read_spec_file` can reject unrelated/corrupted files — no versioning
/// (format changes require clearing the cache manually). `RSPF` =
/// "Ramus SPectrum File".
pub const SPEC_FILE_MAGIC: u32 = 0x52535046;

/// FFT window size in samples. 2048 @ 48 kHz → ~42.7 ms analysis window.
pub const DEFAULT_FFT_SIZE: usize = 2048;

/// Step between successive FFT windows. With `DEFAULT_FFT_SIZE` = 2048
/// and a 48 kHz source, 1024 gives 50% overlap and a native frame rate
/// of ~46 Hz. Frames are decimated to `DEFAULT_TARGET_HOP_MS` on emit so
/// the on-wire payload stays small.
pub const DEFAULT_HOP_SIZE: usize = 1024;

/// Number of log-spaced output bands in the final spectrogram.
pub const DEFAULT_BAND_COUNT: usize = 128;

/// Target frame rate for emitted frames, in milliseconds per frame.
/// 33.33 ms → 30 Hz; the RAF loop interpolates within each frame for
/// 60 fps smoothness.
pub const DEFAULT_TARGET_HOP_MS: f64 = 1000.0 / 30.0;

/// Lowest frequency the band grid covers, in Hz.
///
/// 50 Hz puts band 0 at FFT bin 2 (43–64.5 Hz @ 44.1 kHz / 2048-pt FFT),
/// which is kick-drum and bass-guitar territory. A lower floor would
/// map band 0 to bin 0 (DC) or bin 1 (sub-bass rumble), both near-silent
/// in typical music and leaving the mirror centre as a permanent trough.
///
/// Log spacing at the low end is tighter than the FFT's linear
/// resolution, so bands 0–~4 share bin 2 and the innermost ~10 mirrored
/// bars render as a uniform "bass plateau" during kicks. Fixing it would
/// mean doubling `DEFAULT_FFT_SIZE`.
pub const BAND_FREQ_LOW_HZ: f32 = 50.0;

/// Highest frequency the band grid covers, in Hz.
pub const BAND_FREQ_HIGH_HZ: f32 = 20_000.0;

/// Absolute safety floor for amplitude → dB conversion. The visual floor
/// is computed adaptively per-track (`DYNAMIC_RANGE_DB`); this is the
/// finite sentinel `amp_to_db` clamps into.
pub const DB_FLOOR: f32 = -90.0;

/// Width of the adaptive dynamic range window, in dB. Per track, the
/// peak dB across every frame and band is found, and `[peak - DYNAMIC_RANGE_DB,
/// peak]` is mapped onto 0..255. 55 dB covers a pop song's peak-to-quiet
/// span comfortably and gives classical music room to show dynamics.
pub const DYNAMIC_RANGE_DB: f32 = 55.0;

/// Headroom added above the measured peak dB before becoming the
/// normalised ceiling. Prevents the loudest frame from saturating the
/// top of the visual range, leaving the bars somewhere to reach on accents.
pub const PEAK_HEADROOM_DB: f32 = 2.0;

/// Compression curve exponent applied after dB→0..1 normalisation. Values
/// less than 1 lift quiet passages so they're visually readable.
pub const QUANT_COMPRESSION: f32 = 0.6;

/// Parameters for a single analysis pass. Defaults match the constants above.
#[derive(Debug, Clone, Copy)]
pub struct SpectrumConfig {
    pub fft_size: usize,
    pub hop_size: usize,
    pub band_count: usize,
    pub target_hop_ms: f64,
    pub freq_low_hz: f32,
    pub freq_high_hz: f32,
}

impl Default for SpectrumConfig {
    fn default() -> Self {
        Self {
            fft_size: DEFAULT_FFT_SIZE,
            hop_size: DEFAULT_HOP_SIZE,
            band_count: DEFAULT_BAND_COUNT,
            target_hop_ms: DEFAULT_TARGET_HOP_MS,
            freq_low_hz: BAND_FREQ_LOW_HZ,
            freq_high_hz: BAND_FREQ_HIGH_HZ,
        }
    }
}

/// Whole-track spectrogram, serialisable for the on-disk `.spec` cache
/// and the IPC bridge to the frontend.
///
/// `#[serde(rename_all = "camelCase")]` is load-bearing: Tauri's command
/// bridge uses serde_json and emits Rust field names verbatim. Without
/// this the frontend would see `hop_ms` / `band_count` where TypeScript
/// expects `hopMs` / `bandCount`, and every lookup would be undefined.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpectrumFrames {
    /// Milliseconds between adjacent frames. The frontend looks up the
    /// current frame as `floor(position_ms / hop_ms)`.
    pub hop_ms: f64,
    /// Bands per frame (128 with defaults).
    pub band_count: u32,
    /// FFT window size in samples — diagnostics only.
    pub fft_size: u32,
    /// Source sample rate — diagnostics only.
    pub sample_rate: u32,
    /// `band_count * total_frames` bytes, row-major (frame 0 bands 0..N,
    /// then frame 1 bands 0..N, …). Each byte is a u8 amplitude quantised
    /// from dBFS via the `QUANT_COMPRESSION` curve.
    pub frames: Vec<u8>,
}

impl SpectrumFrames {
    /// Number of time frames in the spectrogram.
    pub fn frame_count(&self) -> usize {
        if self.band_count == 0 {
            return 0;
        }
        self.frames.len() / self.band_count as usize
    }

    /// Fetch a single frame by index. Returns an empty slice when out of
    /// range (the frontend treats `last_frame + 1` as a normal end-of-
    /// track condition).
    pub fn frame(&self, index: usize) -> &[u8] {
        let bands = self.band_count as usize;
        let start = index.saturating_mul(bands);
        let end = start.saturating_add(bands);
        if end > self.frames.len() {
            return &[];
        }
        &self.frames[start..end]
    }
}

/// What the frontend sees when it asks for a track's spectrum.
///
/// `Ready` for analysed tracks, `Analysing` while in progress,
/// `Unavailable { reason }` for tracks that can't be analysed (transcoding,
/// codec refused, …).
///
/// Uses serde's default externally-tagged representation because postcard
/// (the on-disk format) doesn't support internally-tagged enums. JSON
/// shape across the Tauri IPC bridge:
///
/// - `Analysing` → `"analysing"`
/// - `Ready(frames)` → `{"ready": { hop_ms, band_count, frames, ... }}`
/// - `Unavailable { reason }` → `{"unavailable": { "reason": "…" }}`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpectrumState {
    Ready(SpectrumFrames),
    Analysing,
    Unavailable { reason: String },
}

/// Run STFT + log-binning + quantisation over a slice of mono samples.
///
/// `samples` must be mono (average L+R upstream). `sample_rate` maps FFT
/// bins onto log-spaced frequency bands. Config controls window size,
/// hop, band count, and the target emit frame rate.
///
/// Returns an empty-frame `SpectrumFrames` if the input is too short to
/// fit a single FFT window — callers treat that as "analysable but
/// silent" rather than a hard error.
pub fn analyse_samples(
    samples: &[f32],
    sample_rate: u32,
    config: &SpectrumConfig,
) -> SpectrumFrames {
    // Guard against pathological configs rather than panicking downstream.
    let bands = config.band_count.max(1);
    let fft_size = config.fft_size.max(2);
    let hop_size = config.hop_size.max(1);

    // Build the native-hop spectrogram (one frame per STFT window), then
    // decimate to the emit rate by taking per-band max across each group
    // of native frames. Maxing (not averaging) preserves transients.
    let hann = hann_window(fft_size);
    let mut planner = RealFftPlanner::<f32>::new();
    let r2c = planner.plan_fft_forward(fft_size);
    let mut fft_in = r2c.make_input_vec();
    let mut fft_out: Vec<Complex<f32>> = r2c.make_output_vec();

    let band_edges = log_band_edges(bands, fft_size, sample_rate, config);
    let mut native_frames: Vec<Vec<f32>> = Vec::new();

    if samples.len() >= fft_size {
        let mut start = 0;
        while start + fft_size <= samples.len() {
            for i in 0..fft_size {
                fft_in[i] = samples[start + i] * hann[i];
            }
            // realfft only errors on size mismatch; planner-sized buffers
            // make that unreachable.
            if r2c.process(&mut fft_in, &mut fft_out).is_err() {
                break;
            }

            // Per-band peak bin power. Max-of-bins (rather than sum) is
            // visually cleaner for music: a single sine produces one
            // dominant band instead of a wide skirt, and dense chords
            // still light up multiple bands.
            //
            // Normalisation: for a Hann-windowed sine of amplitude A on
            // an N-point real FFT, the peak bin's complex magnitude is
            // A*N/4 (Hann coherent gain 0.5 halves the usual A*N/2).
            // Dividing by N/4 recovers `amp ≈ A` so amp_to_db lands in a
            // sensible dBFS range without per-band saturation.
            //
            // The first ~47 log-spaced bands share a single FFT bin each
            // at default settings, returning identical values. The "bass
            // lockstep" effect is handled visually in FocusVisualizer.tsx
            // via per-band noise decorrelation in the RAF loop.
            let amp_scale = 4.0 / fft_size as f32;
            let mut frame = vec![0.0_f32; bands];
            for (b, edges) in band_edges.iter().enumerate() {
                let (lo, hi) = (edges.0, edges.1);
                if lo >= hi {
                    frame[b] = DB_FLOOR;
                    continue;
                }
                let mut max_power = 0.0_f32;
                for bin in &fft_out[lo..hi] {
                    let p = bin.re * bin.re + bin.im * bin.im;
                    if p > max_power {
                        max_power = p;
                    }
                }
                let amp = max_power.sqrt() * amp_scale;
                frame[b] = amp_to_db(amp);
            }
            native_frames.push(frame);

            start += hop_size;
        }
    }

    // Adaptive per-track dynamic range: find the loudest dB value the
    // track ever reaches and build a normalisation window of
    // `DYNAMIC_RANGE_DB` width just below it. A fixed -80..0 dB range
    // crushes most real music into the top third of the u8 scale because
    // the useful variation in a given band sits between roughly -50 and
    // -10 dB. Effectively silent tracks (peak near `DB_FLOOR`) still
    // produce a valid frame array, quantised to all zeros.
    let mut peak_db = DB_FLOOR;
    for frame in &native_frames {
        for &db in frame {
            if db > peak_db {
                peak_db = db;
            }
        }
    }
    let ceiling = peak_db + PEAK_HEADROOM_DB;
    let floor = (ceiling - DYNAMIC_RANGE_DB).max(DB_FLOOR);

    // Decimate native frames to the target emit rate. Each emit slot
    // takes per-band max across its group of native frames.
    //
    // `group_size` MUST be picked by rounding, and `emit_hop_ms` MUST be
    // recomputed from it. Truncating (`as usize`) and reporting the
    // target unchanged claims a frame spacing that doesn't match the
    // frames emitted; the frontend's `floor(position_ms / hop_ms)` lookup
    // then drifts linearly out of sync with the audio. Example: native
    // hop `(1024 / 48000) * 1000 = 21.33` ms vs a 33.33 ms target gives
    // a ratio of 1.56 — truncates to 1 (no decimation) but rounds to 2.
    let native_hop_ms = (hop_size as f64 / sample_rate as f64) * 1000.0;
    let group_size_f = (config.target_hop_ms / native_hop_ms).max(1.0);
    let group_size = group_size_f.round() as usize;
    let emit_hop_ms = group_size as f64 * native_hop_ms;

    let mut emit_frames: Vec<u8> = Vec::new();
    let mut native_idx = 0;
    while native_idx < native_frames.len() {
        let end = (native_idx + group_size).min(native_frames.len());
        for band in 0..bands {
            let mut peak = f32::NEG_INFINITY;
            for frame in &native_frames[native_idx..end] {
                if frame[band] > peak {
                    peak = frame[band];
                }
            }
            emit_frames.push(quantise_db_range(peak, floor, ceiling));
        }
        native_idx += group_size;
    }

    // Detailed analysis log for cross-checking timing against the UI and
    // mpv. `native_duration_s` (samples / sample_rate) should match mpv's
    // reported `duration` and Plex's `duration_ms` within encoder padding;
    // divergence implicates VBR / priming / demuxer bugs.
    let native_duration_s = samples.len() as f64 / sample_rate as f64;
    let emit_frame_count = if bands > 0 {
        emit_frames.len() / bands
    } else {
        0
    };
    let emit_duration_s = emit_frame_count as f64 * emit_hop_ms / 1000.0;
    log::debug!(
        "spectrum: sr={}Hz, samples={}, native_duration={:.3}s, \
         native_frames={}, native_hop={:.3}ms, group_size={}, \
         emit_hop={:.3}ms, emit_frames={}, emit_duration={:.3}s, \
         peak={:.1}dB, range=[{:.1}, {:.1}]",
        sample_rate,
        samples.len(),
        native_duration_s,
        native_frames.len(),
        native_hop_ms,
        group_size,
        emit_hop_ms,
        emit_frame_count,
        emit_duration_s,
        peak_db,
        floor,
        ceiling,
    );

    SpectrumFrames {
        hop_ms: emit_hop_ms,
        band_count: bands as u32,
        fft_size: fft_size as u32,
        sample_rate,
        frames: emit_frames,
    }
}

/// Compute the sibling `.spec` path for a given audio file path
/// (`/cache/track.flac` → `/cache/track.flac.spec`). Colocated with the
/// format constants so callers in different crates agree on naming.
pub fn spec_file_path(audio_path: &Path) -> PathBuf {
    let mut s = audio_path.as_os_str().to_os_string();
    s.push(".spec");
    PathBuf::from(s)
}

/// Persist a `SpectrumState` to the sibling `.spec` file. Only `Ready`
/// and `Unavailable` hit disk; `Analysing` is runtime-only and silently
/// dropped here.
pub fn write_spec_file(audio_path: &Path, state: &SpectrumState) -> std::io::Result<()> {
    if matches!(state, SpectrumState::Analysing) {
        return Ok(());
    }
    let spec_path = spec_file_path(audio_path);
    let mut file = File::create(&spec_path)?;
    file.write_all(&SPEC_FILE_MAGIC.to_le_bytes())?;
    let body = postcard::to_stdvec(state)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    file.write_all(&body)?;
    file.sync_all()?;
    Ok(())
}

/// Read the sibling `.spec` file. Returns `Some(state)` on a valid file,
/// `None` if missing, magic mismatched, or the postcard body fails to
/// deserialise.
pub fn read_spec_file(audio_path: &Path) -> Option<SpectrumState> {
    let spec_path = spec_file_path(audio_path);
    let mut file = File::open(&spec_path).ok()?;

    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).ok()?;
    if u32::from_le_bytes(magic) != SPEC_FILE_MAGIC {
        return None;
    }

    let mut body = Vec::new();
    file.read_to_end(&mut body).ok()?;
    postcard::from_bytes::<SpectrumState>(&body).ok()
}

/// Hann (raised-cosine) window. Standard STFT choice: good sidelobe
/// suppression and minimal spectral leakage for broadband signals.
fn hann_window(n: usize) -> Vec<f32> {
    if n <= 1 {
        return vec![1.0; n];
    }
    let denom = (n - 1) as f32;
    (0..n)
        .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / denom).cos())
        .collect()
}

/// `(lo_bin, hi_bin)` pairs for each log-spaced output band. Lowest band
/// starts at `freq_low_hz`, highest ends at `min(freq_high_hz, nyquist)`.
/// Exponential spacing matches human pitch perception.
fn log_band_edges(
    band_count: usize,
    fft_size: usize,
    sample_rate: u32,
    config: &SpectrumConfig,
) -> Vec<(usize, usize)> {
    let nyquist_hz = sample_rate as f32 / 2.0;
    let fft_bins = fft_size / 2 + 1;
    let hi_hz = config.freq_high_hz.min(nyquist_hz);
    let lo_hz = config.freq_low_hz.max(1.0).min(hi_hz * 0.5);

    let hz_per_bin = sample_rate as f32 / fft_size as f32;

    let log_lo = lo_hz.ln();
    let log_hi = hi_hz.ln();

    let mut edges = Vec::with_capacity(band_count);
    for b in 0..band_count {
        let t_lo = b as f32 / band_count as f32;
        let t_hi = (b + 1) as f32 / band_count as f32;
        let f_lo = (log_lo + (log_hi - log_lo) * t_lo).exp();
        let f_hi = (log_lo + (log_hi - log_lo) * t_hi).exp();

        let mut bin_lo = (f_lo / hz_per_bin).floor() as usize;
        let mut bin_hi = (f_hi / hz_per_bin).ceil() as usize;
        // Every band must cover at least one bin so bands tighter than
        // FFT resolution don't output all-zeroes. Adjacent low bands then
        // share a bin, which the viz renders as a smooth gradient.
        if bin_hi <= bin_lo {
            bin_hi = bin_lo + 1;
        }
        bin_lo = bin_lo.min(fft_bins);
        bin_hi = bin_hi.min(fft_bins);
        edges.push((bin_lo, bin_hi));
    }
    edges
}

/// Convert linear amplitude (0..1) to dBFS (-∞..0). Amplitudes at or
/// below zero return `DB_FLOOR` to keep downstream clamping finite.
fn amp_to_db(amp: f32) -> f32 {
    if amp <= 0.0 || !amp.is_finite() {
        return DB_FLOOR;
    }
    20.0 * amp.log10()
}

/// Quantise a dBFS value to 0..255 against an explicit `[floor, ceiling]`
/// window. `analyse_samples` picks the window adaptively per track so the
/// full 0..255 scale covers the range the music actually occupies.
///
/// Values at or below `floor` map to 0; values at or above `ceiling` map
/// to 255; in between renormalises to 0..1 and applies `QUANT_COMPRESSION`
/// so quiet passages aren't flatlined. Degenerate ranges return 0.
fn quantise_db_range(db: f32, floor: f32, ceiling: f32) -> u8 {
    if !db.is_finite() || ceiling <= floor || db <= floor {
        return 0;
    }
    let clamped = db.clamp(floor, ceiling);
    let t = (clamped - floor) / (ceiling - floor);
    let curved = t.powf(QUANT_COMPRESSION);
    (curved * 255.0).round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SR: u32 = 48_000;

    /// Generate `secs` seconds of a pure sine at `freq_hz`.
    fn sine(freq_hz: f32, secs: f32, sample_rate: u32) -> Vec<f32> {
        let n = (secs * sample_rate as f32) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / sample_rate as f32;
                (2.0 * PI * freq_hz * t).sin() * 0.5
            })
            .collect()
    }

    #[test]
    fn hann_window_is_symmetric_and_peaks_at_centre() {
        let w = hann_window(8);
        assert_eq!(w.len(), 8);
        assert!((w[0] - 0.0).abs() < 1e-4);
        assert!((w[7] - 0.0).abs() < 1e-4);
        // Centre should be ~1.0
        assert!(w[3] > 0.9);
        assert!(w[4] > 0.9);
        // Symmetry
        for i in 0..4 {
            assert!((w[i] - w[7 - i]).abs() < 1e-5);
        }
    }

    #[test]
    fn hann_window_handles_degenerate_sizes() {
        assert_eq!(hann_window(0).len(), 0);
        assert_eq!(hann_window(1), vec![1.0]);
    }

    #[test]
    fn log_band_edges_are_monotonic_and_cover_bins() {
        let config = SpectrumConfig::default();
        let edges = log_band_edges(128, 2048, TEST_SR, &config);
        assert_eq!(edges.len(), 128);
        for (lo, hi) in &edges {
            assert!(hi > lo, "band must cover ≥1 bin (lo={lo}, hi={hi})");
        }
        // Adjacent bands may share bins at the low end where log spacing
        // is tighter than FFT resolution.
        for window in edges.windows(2) {
            assert!(window[1].0 >= window[0].0);
        }
    }

    #[test]
    fn amp_to_db_sanity() {
        assert!((amp_to_db(1.0) - 0.0).abs() < 1e-4);
        assert!((amp_to_db(0.5) - (-6.0)).abs() < 0.1);
        assert_eq!(amp_to_db(0.0), DB_FLOOR);
        assert_eq!(amp_to_db(-1.0), DB_FLOOR);
    }

    #[test]
    fn quantise_db_range_edges_and_midpoint() {
        let floor = -65.0;
        let ceiling = -10.0;

        assert_eq!(quantise_db_range(floor, floor, ceiling), 0);
        assert_eq!(quantise_db_range(floor - 10.0, floor, ceiling), 0);

        assert_eq!(quantise_db_range(ceiling, floor, ceiling), 255);

        assert_eq!(quantise_db_range(f32::NEG_INFINITY, floor, ceiling), 0);
        assert_eq!(quantise_db_range(f32::NAN, floor, ceiling), 0);
        assert_eq!(quantise_db_range(-20.0, ceiling, floor), 0);

        let mid = quantise_db_range(-35.0, floor, ceiling);
        assert!(mid > 0 && mid < 255);
    }

    #[test]
    fn adaptive_range_separates_loud_and_quiet_sines() {
        fn analyse_amp(amp: f32) -> u8 {
            let n = TEST_SR as usize;
            let samples: Vec<f32> = (0..n)
                .map(|i| {
                    let t = i as f32 / TEST_SR as f32;
                    (2.0 * PI * 1000.0 * t).sin() * amp
                })
                .collect();
            let frames = analyse_samples(&samples, TEST_SR, &SpectrumConfig::default());
            peak_band_of(&frames);
            let mut max = 0u8;
            for b in &frames.frames {
                if *b > max {
                    max = *b;
                }
            }
            max
        }

        // Adaptive normalisation pushes both tracks' peaks toward 255
        // regardless of absolute amplitude — the visualiser shows
        // relative dynamics within each track.
        let loud = analyse_amp(0.5);
        let quiet = analyse_amp(0.01);
        assert!(loud > 240, "loud sine should peak near 255, got {loud}");
        assert!(quiet > 240, "quiet sine should peak near 255, got {quiet}");
    }

    #[test]
    fn adaptive_range_preserves_within_track_dynamics() {
        // 2-second track: first second at 0.005 amplitude, second second
        // at 0.5. With the adaptive range + pow(0.6), the quiet half
        // should read clearly lower than the loud half.
        let sr = TEST_SR as usize;
        let mut samples: Vec<f32> = Vec::with_capacity(sr * 2);
        for i in 0..sr {
            let t = i as f32 / TEST_SR as f32;
            samples.push((2.0 * PI * 1000.0 * t).sin() * 0.005);
        }
        for i in 0..sr {
            let t = i as f32 / TEST_SR as f32;
            samples.push((2.0 * PI * 1000.0 * t).sin() * 0.5);
        }
        let frames = analyse_samples(&samples, TEST_SR, &SpectrumConfig::default());
        assert!(frames.frame_count() > 8);

        let bands = frames.band_count as usize;
        let total = frames.frame_count();
        let half = total / 2;

        // Use band-peak-of-bucket (not bucket-peak-of-any-band) so
        // spectral leakage from adjacent bands can't muddy the comparison.
        let target = expected_band_for(1000.0, bands);
        let band_max_in_range = |start: usize, end: usize| -> u8 {
            let mut m = 0u8;
            for f in start..end {
                let v = frames.frame(f)[target];
                if v > m {
                    m = v;
                }
            }
            m
        };

        // Skip the first few frames of each half so the level settles
        // after the step transition.
        let skip = 4;
        let quiet_peak = band_max_in_range(skip, half - skip);
        let loud_peak = band_max_in_range(half + skip, total - skip);

        assert!(
            loud_peak > quiet_peak + 40,
            "expected clear dynamic separation, got quiet={quiet_peak}, loud={loud_peak}"
        );
    }

    #[test]
    fn empty_input_returns_empty_frames() {
        let frames = analyse_samples(&[], TEST_SR, &SpectrumConfig::default());
        assert_eq!(frames.band_count, DEFAULT_BAND_COUNT as u32);
        assert_eq!(frames.sample_rate, TEST_SR);
        assert_eq!(frames.frame_count(), 0);
        assert!(frames.frames.is_empty());
    }

    #[test]
    fn shorter_than_fft_returns_empty_frames() {
        let samples = vec![0.0_f32; 100];
        let frames = analyse_samples(&samples, TEST_SR, &SpectrumConfig::default());
        assert_eq!(frames.frame_count(), 0);
    }

    #[test]
    fn silence_produces_all_zero_bytes() {
        let samples = vec![0.0_f32; TEST_SR as usize * 2];
        let frames = analyse_samples(&samples, TEST_SR, &SpectrumConfig::default());
        assert!(frames.frame_count() > 0);
        for byte in &frames.frames {
            assert_eq!(*byte, 0, "silence must quantise to 0");
        }
    }

    /// Predict which log-spaced band a given frequency should peak in
    /// for the default config. Rounds to the nearest integer band index.
    fn expected_band_for(freq_hz: f32, band_count: usize) -> usize {
        let lo = BAND_FREQ_LOW_HZ.ln();
        let hi = BAND_FREQ_HIGH_HZ.ln();
        let t = (freq_hz.ln() - lo) / (hi - lo);
        (t * band_count as f32).round() as usize
    }

    /// Band index with the highest total across all frames. Ties prefer
    /// the earlier band so saturation tests don't flake on iterator order.
    fn peak_band_of(frames: &SpectrumFrames) -> usize {
        let bands = frames.band_count as usize;
        let mut totals = vec![0u64; bands];
        for frame_idx in 0..frames.frame_count() {
            for (b, &val) in frames.frame(frame_idx).iter().enumerate() {
                totals[b] += val as u64;
            }
        }
        let mut peak = 0;
        let mut best = 0u64;
        for (i, &t) in totals.iter().enumerate() {
            if t > best {
                best = t;
                peak = i;
            }
        }
        peak
    }

    #[test]
    fn sine_peak_lands_in_correct_band_low_range() {
        // 100 Hz sine should peak around band 29-30. Allow ±6 bands of
        // tolerance for Hann window leakage.
        let samples = sine(100.0, 1.0, TEST_SR);
        let frames = analyse_samples(&samples, TEST_SR, &SpectrumConfig::default());
        assert!(frames.frame_count() > 0);

        let peak_band = peak_band_of(&frames);
        let expected = expected_band_for(100.0, 128);
        let diff = peak_band.abs_diff(expected);
        assert!(
            diff <= 6,
            "100 Hz should peak near band {expected}, got {peak_band} (diff {diff})"
        );
    }

    #[test]
    fn sine_peak_lands_in_correct_band_high_range() {
        let samples = sine(8000.0, 1.0, TEST_SR);
        let frames = analyse_samples(&samples, TEST_SR, &SpectrumConfig::default());
        assert!(frames.frame_count() > 0);

        let peak_band = peak_band_of(&frames);
        let expected = expected_band_for(8000.0, 128);
        let diff = peak_band.abs_diff(expected);
        assert!(
            diff <= 6,
            "8 kHz should peak near band {expected}, got {peak_band} (diff {diff})"
        );
        assert!(peak_band > 64, "8 kHz must be in the upper half of bands");
    }

    #[test]
    fn does_not_panic_on_edge_sample_rates() {
        let samples = sine(440.0, 0.5, 8_000);
        let _ = analyse_samples(&samples, 8_000, &SpectrumConfig::default());
        let samples = sine(440.0, 0.5, 96_000);
        let _ = analyse_samples(&samples, 96_000, &SpectrumConfig::default());
    }

    #[test]
    fn frame_bounds_are_safe_past_end() {
        let samples = sine(440.0, 0.5, TEST_SR);
        let frames = analyse_samples(&samples, TEST_SR, &SpectrumConfig::default());
        let count = frames.frame_count();
        assert!(!frames.frame(count).iter().any(|_| true));
        assert!(!frames.frame(count + 10).iter().any(|_| true));
        assert_eq!(frames.frame(0).len(), frames.band_count as usize);
    }

    #[test]
    fn spectrum_frames_roundtrip_via_postcard() {
        let samples = sine(1000.0, 0.5, TEST_SR);
        let original = analyse_samples(&samples, TEST_SR, &SpectrumConfig::default());
        let encoded = postcard::to_stdvec(&original).expect("encode");
        let decoded: SpectrumFrames = postcard::from_bytes(&encoded).expect("decode");
        assert_eq!(original, decoded);
    }

    #[test]
    fn spectrum_state_serialises_externally_tagged() {
        let json = serde_json::to_string(&SpectrumState::Analysing).unwrap();
        assert_eq!(json, r#""analysing""#);

        let json = serde_json::to_string(&SpectrumState::Unavailable {
            reason: "transcoding".into(),
        })
        .unwrap();
        assert_eq!(json, r#"{"unavailable":{"reason":"transcoding"}}"#);
    }

    #[test]
    fn spectrum_state_roundtrips_via_postcard() {
        let a = SpectrumState::Analysing;
        let a_bytes = postcard::to_stdvec(&a).unwrap();
        let a_back: SpectrumState = postcard::from_bytes(&a_bytes).unwrap();
        assert_eq!(a, a_back);

        let u = SpectrumState::Unavailable {
            reason: "transcoding".into(),
        };
        let u_bytes = postcard::to_stdvec(&u).unwrap();
        let u_back: SpectrumState = postcard::from_bytes(&u_bytes).unwrap();
        assert_eq!(u, u_back);

        let samples = sine(1000.0, 0.2, TEST_SR);
        let frames = analyse_samples(&samples, TEST_SR, &SpectrumConfig::default());
        let r = SpectrumState::Ready(frames);
        let r_bytes = postcard::to_stdvec(&r).unwrap();
        let r_back: SpectrumState = postcard::from_bytes(&r_bytes).unwrap();
        assert_eq!(r, r_back);
    }

    #[test]
    fn custom_config_is_respected() {
        let config = SpectrumConfig {
            band_count: 32,
            ..SpectrumConfig::default()
        };
        let samples = sine(440.0, 0.5, TEST_SR);
        let frames = analyse_samples(&samples, TEST_SR, &config);
        assert_eq!(frames.band_count, 32);
        if frames.frame_count() > 0 {
            assert_eq!(frames.frame(0).len(), 32);
        }
    }
}
