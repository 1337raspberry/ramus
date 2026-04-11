//! Per-track spectrogram analysis for the focus-mode visualiser.
//!
//! This module is pure DSP — it takes a slice of mono `f32` samples
//! (interleave your stereo upstream, or L+R average it) and produces a
//! time-indexed spectrogram the frontend can look up by playback position.
//!
//! **Why pre-compute instead of live?** libmpv does not expose real-time
//! PCM frames (see CLAUDE.md), and running a parallel live decoder would
//! fight mpv for Plex's concurrent-download slot. Pre-computing the whole
//! spectrogram at prefetch-time and indexing it by `time-pos` gives us
//! two wins for free: perfect sync with the audio output (mpv's reported
//! position is the ground truth) and zero cross-decoder latency guesses.
//!
//! The analyser runs a short-time FFT (STFT) with a Hann window, groups
//! the magnitude bins into 128 log-spaced bands, then compresses and
//! quantises each band to a `u8` so the on-wire/on-disk size stays small.
//! 5-minute track at 30 Hz × 128 bands × 1 byte ≈ 1.15 MB.

use serde::{Deserialize, Serialize};
use std::f32::consts::PI;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use realfft::num_complex::Complex;
use realfft::RealFftPlanner;

/// Magic bytes prepended to every `.spec` cache file. Just a sanity
/// check so `read_spec_file` can reject unrelated/corrupted files — no
/// versioning (this is pre-release; format changes require nuking
/// the dev cache manually). `RSPF` = "Ramus SPectrum File".
pub const SPEC_FILE_MAGIC: u32 = 0x52535046;

// --- Tunable DSP parameters ---

/// FFT window size in samples. 2048 @ 48 kHz → ~42.7 ms analysis window.
pub const DEFAULT_FFT_SIZE: usize = 2048;

/// Hop size in samples — the step between successive FFT windows. With
/// `DEFAULT_FFT_SIZE` = 2048 and a 48 kHz source, 1024 gives 50% overlap
/// and a native frame rate of ~46 Hz. We then downsample to `TARGET_HOP_MS`
/// when emitting so the on-wire payload stays small.
pub const DEFAULT_HOP_SIZE: usize = 1024;

/// Number of log-spaced output bands in the final spectrogram.
pub const DEFAULT_BAND_COUNT: usize = 128;

/// Target frame rate for emitted frames, in milliseconds per frame.
/// 33.33 ms → 30 Hz. The RAF loop interpolates within each frame so the
/// viz still looks smooth at 60 fps.
pub const DEFAULT_TARGET_HOP_MS: f64 = 1000.0 / 30.0;

/// Lowest frequency the band grid covers, in Hz.
///
/// 50 Hz puts band 0 at FFT bin 2 (43–64.5 Hz @ 44.1 kHz / 2048-pt FFT),
/// which is kick-drum and bass-guitar territory. A lower floor would
/// map band 0 to bin 0 (DC) or bin 1 (sub-bass rumble), both of which
/// are near-silent in typical music and would leave the mirror centre
/// as a permanent empty trough.
///
/// Log spacing at the low end is tighter than the FFT's linear
/// resolution, so bands 0–~4 all end up sharing bin 2 and the innermost
/// ~10 mirrored bars render as a uniform "bass plateau" during kicks.
/// That's an honest representation of what a 2048-pt FFT can resolve
/// down there; fixing it would mean doubling `DEFAULT_FFT_SIZE`.
pub const BAND_FREQ_LOW_HZ: f32 = 50.0;

/// Highest frequency we care about (above this is beyond most speakers).
pub const BAND_FREQ_HIGH_HZ: f32 = 20_000.0;

/// Absolute safety floor for amplitude → dB conversion. Anything this
/// quiet is "silence" to us. The *visual* floor is computed adaptively
/// per-track (see `DYNAMIC_RANGE_DB`) — this is just a sentinel to
/// keep math finite and give amp_to_db something to clamp into.
pub const DB_FLOOR: f32 = -90.0;

/// Width of the adaptive dynamic range window, in dB. Per track, we
/// find the peak dB across every frame and band, and map
/// `[peak - DYNAMIC_RANGE_DB, peak]` onto 0..255. 55 dB covers the
/// span between a pop song's peak and its quietest non-silent passage
/// comfortably, and still gives classical music room to show dynamics.
/// Shrink this if the bars look "too alive" during quiet sections;
/// widen it if quiet sections disappear entirely.
pub const DYNAMIC_RANGE_DB: f32 = 55.0;

/// Headroom added above the measured peak dB before becoming the
/// normalised ceiling. A small cushion prevents the single loudest
/// frame in the track from always hitting the exact top of the visual
/// range — leaves the bars with somewhere to *reach* on accents.
pub const PEAK_HEADROOM_DB: f32 = 2.0;

/// Compression curve exponent applied after dB→0..1 normalisation.
/// Values less than 1 lift quiet passages so they're visually readable.
/// 0.6 gives a clear distinction between loud and quiet within the
/// adaptive range; 0.4 was too aggressive and crushed everything into
/// the top of the range.
pub const QUANT_COMPRESSION: f32 = 0.6;

/// Knobs for a single analysis pass. Defaults match the constants above.
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

// --- Output types ---

/// A whole-track spectrogram, serialisable for the on-disk `.spec` cache
/// and the IPC bridge to the frontend.
///
/// `#[serde(rename_all = "camelCase")]` is mandatory here — Tauri's
/// command bridge uses serde_json, which emits Rust field names
/// verbatim. Without this, the frontend would see `hop_ms` / `band_count`
/// where its TypeScript types expect `hopMs` / `bandCount`, and every
/// lookup would fall back to `undefined` → NaN → empty bars.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpectrumFrames {
    /// Milliseconds between adjacent frames. The frontend looks up the
    /// current frame as `floor(position_ms / hop_ms)`.
    pub hop_ms: f64,
    /// Number of bands per frame (128 with defaults).
    pub band_count: u32,
    /// Size of the FFT window in samples — diagnostics only.
    pub fft_size: u32,
    /// Source sample rate the analyser saw — diagnostics only.
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

    /// Fetch a single frame by index. Returns an empty slice if the index
    /// is out of range (the frontend clamps to this, so calling with the
    /// last frame+1 is a normal end-of-track condition).
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
/// A successfully analysed track is `Ready`; a track currently being
/// analysed is `Analysing`; a track that can't be analysed (Plex is
/// transcoding it, symphonia refused the codec, etc.) is
/// `Unavailable { reason }`.
///
/// We use serde's default externally-tagged representation (not
/// `#[serde(tag = "kind")]`) because postcard — which we use for the
/// on-disk format — doesn't support internally-tagged enums. The JSON
/// shape across the Tauri IPC bridge is:
///
/// - `Analysing` → `"analysing"`
/// - `Ready(frames)` → `{"ready": { hop_ms, band_count, frames, ... }}`
/// - `Unavailable { reason }` → `{"unavailable": { "reason": "…" }}`
///
/// TypeScript types in the UI should mirror this exactly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpectrumState {
    Ready(SpectrumFrames),
    Analysing,
    Unavailable { reason: String },
}

// --- The analyser ---

/// Run STFT + log-binning + quantisation over a slice of mono samples.
///
/// `samples` must be mono (average L+R upstream). `sample_rate` is used
/// to map FFT bins onto log-spaced frequency bands. The config knobs
/// control window size, hop, band count, and the target emit frame rate.
///
/// Returns an empty-frame `SpectrumFrames` if the input is too short to
/// fit a single FFT window — callers should treat that as "analysable
/// but silent" rather than a hard error.
pub fn analyse_samples(
    samples: &[f32],
    sample_rate: u32,
    config: &SpectrumConfig,
) -> SpectrumFrames {
    // Guard against pathological configs rather than panicking downstream.
    let bands = config.band_count.max(1);
    let fft_size = config.fft_size.max(2);
    let hop_size = config.hop_size.max(1);

    // Build the native hop spectrogram first — one frame per STFT window.
    // Then decimate to the target hop rate (e.g. 46 Hz native → 30 Hz emit)
    // by averaging across the group of native frames that falls inside
    // each emit slot. Averaging (not picking one) preserves short transients.
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
            // Window the block in-place into the FFT input buffer.
            for i in 0..fft_size {
                fft_in[i] = samples[start + i] * hann[i];
            }
            // realfft may return Err only on size mismatch; our buffers
            // are sized by the planner so this shouldn't happen.
            if r2c.process(&mut fft_in, &mut fft_out).is_err() {
                break;
            }

            // Per-band peak bin power. Using max-of-bins (rather than sum)
            // is visually cleaner for music: a single sine produces one
            // dominant band instead of a wide skirt, and dense chords still
            // light up multiple bands because each band picks its own peak.
            //
            // Normalisation: for a Hann-windowed sine of amplitude A on an
            // N-point real FFT, the peak bin's complex magnitude is A*N/4
            // (Hann has coherent gain 0.5, which halves the usual A*N/2).
            // Dividing by N/4 recovers `amp ≈ A` so amp_to_db lands in a
            // sensible dBFS range without per-band saturation.
            //
            // NOTE: the first ~47 log-spaced bands share a single FFT bin
            // each at our default settings (50 Hz floor + 2048-pt FFT +
            // 44.1 kHz SR), so they return identical values. That's the
            // "bass lockstep" effect — handled visually in
            // FocusVisualizer.tsx via per-band noise decorrelation in the
            // RAF loop, not here.
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

    // --- Adaptive per-track dynamic range ---
    //
    // Scan every band of every native frame to find the loudest dB value
    // the track ever reaches, then build a normalisation window of
    // `DYNAMIC_RANGE_DB` width just below it. Per-track adaptivity is
    // what makes the bars actually follow the music: a fixed -80..0 dB
    // range crushes 99% of real music into the top third of the u8 scale
    // because the useful variation in a given band sits between roughly
    // -50 and -10 dB, not -80 and 0.
    //
    // If the track is effectively silent (peak near `DB_FLOOR`), we still
    // produce a valid frame array — it just quantises to all zeros, which
    // is what the visualiser should show anyway.
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
    // takes per-band max across the native frames in its group — maxing
    // (rather than averaging) keeps transients like kick drums crisp.
    //
    // **Important**: `group_size` MUST be picked by rounding, and the
    // reported `emit_hop_ms` MUST be recomputed from it. If you instead
    // truncate (`as usize`) and report the target unchanged, you end up
    // claiming a frame spacing that doesn't match the frames you emit,
    // and the frontend's `floor(position_ms / hop_ms)` lookup drifts
    // linearly out of sync with the audio — the symptom is "visualiser
    // playing back the track at ~64% speed", where it falls further
    // behind the longer the song plays. `(1024 / 48000) * 1000 = 21.33
    // ms` vs a 33.33 ms target gives a ratio of 1.56, which truncates
    // to 1 (no decimation!) but *should* round to 2.
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

    // Detailed analysis log so we can cross-check timing against the UI
    // and mpv. `native_duration_s` is the analyser's sense of the track
    // length (samples / sample_rate), which should match mpv's reported
    // `duration` and Plex's `duration_ms` to within the encoder padding
    // of a handful of milliseconds. If you see these diverge, that's
    // when to suspect VBR / priming / demuxer bugs.
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

// --- On-disk cache format ---

/// Compute the sibling `.spec` path for a given audio file path.
/// `/cache/track.flac` → `/cache/track.flac.spec`. This is colocated with
/// the format constants so callers in different crates agree on naming.
pub fn spec_file_path(audio_path: &Path) -> PathBuf {
    let mut s = audio_path.as_os_str().to_os_string();
    s.push(".spec");
    PathBuf::from(s)
}

/// Persist a `SpectrumState` to the sibling `.spec` file for the given
/// audio path. Only `Ready` and `Unavailable` hit disk — `Analysing` is
/// a runtime-only state and is silently dropped here so callers can pass
/// whatever they have in hand without a special case.
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

/// Read the sibling `.spec` file for the given audio path. Returns
/// `Some(state)` on a valid file, `None` if the file is missing, the
/// magic bytes don't match, or the postcard body fails to deserialise.
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

// --- Helpers (private) ---

/// Hann (raised-cosine) window of the given length. Standard STFT window
/// choice — good sidelobe suppression, minimal spectral leakage for
/// broadband signals like music.
fn hann_window(n: usize) -> Vec<f32> {
    if n <= 1 {
        return vec![1.0; n];
    }
    let denom = (n - 1) as f32;
    (0..n)
        .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / denom).cos())
        .collect()
}

/// Compute `(lo_bin, hi_bin)` pairs for each log-spaced output band.
///
/// The lowest band starts at the bin corresponding to `freq_low_hz` and
/// the highest ends at `freq_high_hz` (or Nyquist, whichever is lower).
/// Bands are spaced exponentially, so low-frequency bands span few FFT
/// bins each and high-frequency bands span many — matching human pitch
/// perception and giving bass plenty of resolution.
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

    // hz_per_bin: each FFT bin represents this many Hz.
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
        // Ensure every band covers at least one bin so we don't output
        // all-zeroes for bands tighter than the FFT resolution. This
        // causes adjacent low bands to share a bin, which is fine —
        // the viz just draws a smooth gradient across that range.
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
/// below zero return `DB_FLOOR` so downstream clamping has something
/// finite to work with.
fn amp_to_db(amp: f32) -> f32 {
    if amp <= 0.0 || !amp.is_finite() {
        return DB_FLOOR;
    }
    20.0 * amp.log10()
}

/// Quantise a dBFS value to 0..255 against an explicit `[floor, ceiling]`
/// window. `analyse_samples` picks the window adaptively per track so
/// the full 0..255 scale is used for the range the music actually
/// occupies — fixed [-80, 0] wasted most of the scale on silence.
///
/// Values at or below `floor` map to 0; values at or above `ceiling`
/// map to 255; in between we renormalise to 0..1 and apply
/// `QUANT_COMPRESSION` so quiet passages aren't flatlined. Degenerate
/// ranges (`ceiling <= floor`) return 0.
fn quantise_db_range(db: f32, floor: f32, ceiling: f32) -> u8 {
    if !db.is_finite() || ceiling <= floor || db <= floor {
        return 0;
    }
    let clamped = db.clamp(floor, ceiling);
    let t = (clamped - floor) / (ceiling - floor);
    let curved = t.powf(QUANT_COMPRESSION);
    (curved * 255.0).round().clamp(0.0, 255.0) as u8
}

// --- Tests ---

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
        // Monotonic in lo bin (some adjacent bands may share bins at
        // the low end where log spacing is tighter than FFT resolution).
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
        // A typical adaptive window: peak -10 dB, floor -65 dB.
        let floor = -65.0;
        let ceiling = -10.0;

        // Floor and below → 0.
        assert_eq!(quantise_db_range(floor, floor, ceiling), 0);
        assert_eq!(quantise_db_range(floor - 10.0, floor, ceiling), 0);

        // Ceiling → 255.
        assert_eq!(quantise_db_range(ceiling, floor, ceiling), 255);

        // Degenerate / NaN → 0.
        assert_eq!(quantise_db_range(f32::NEG_INFINITY, floor, ceiling), 0);
        assert_eq!(quantise_db_range(f32::NAN, floor, ceiling), 0);
        assert_eq!(quantise_db_range(-20.0, ceiling, floor), 0);

        // A value in the middle should land clearly inside (0, 255).
        let mid = quantise_db_range(-35.0, floor, ceiling);
        assert!(mid > 0 && mid < 255);
    }

    #[test]
    fn adaptive_range_separates_loud_and_quiet_sines() {
        // Run the analyser on a loud sine and a quiet sine at the same
        // frequency. The loud one should quantise noticeably higher.
        fn analyse_amp(amp: f32) -> u8 {
            // 1 s of a 1 kHz sine at the given amplitude.
            let n = TEST_SR as usize;
            let samples: Vec<f32> = (0..n)
                .map(|i| {
                    let t = i as f32 / TEST_SR as f32;
                    (2.0 * PI * 1000.0 * t).sin() * amp
                })
                .collect();
            let frames = analyse_samples(&samples, TEST_SR, &SpectrumConfig::default());
            peak_band_of(&frames);
            // Max value across all frames + bands is what the visualiser
            // would peak at. Use that as the "loudness" figure.
            let mut max = 0u8;
            for b in &frames.frames {
                if *b > max {
                    max = *b;
                }
            }
            max
        }

        // Adaptive normalisation means BOTH tracks individually push
        // their own peak toward 255 — they land in roughly the same
        // place regardless of absolute amplitude, because the visualiser
        // shows relative dynamics within each track.
        let loud = analyse_amp(0.5);
        let quiet = analyse_amp(0.01);
        assert!(loud > 240, "loud sine should peak near 255, got {loud}");
        assert!(quiet > 240, "quiet sine should peak near 255, got {quiet}");
    }

    #[test]
    fn adaptive_range_preserves_within_track_dynamics() {
        // A 2-second track: first second at 0.005 amplitude, second
        // second at 0.5. The old fixed -80..0 + pow(0.4) pipeline
        // would crush both halves into the 180..220 range (≈15%
        // separation). With the adaptive range + pow(0.6), the quiet
        // half should read clearly lower than the loud half.
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

        // Find the MAX u8 the 1 kHz band reaches in each half. The
        // target band (~band 60) will be crisp and unambiguous. Use
        // band-peak-of-bucket, not bucket-peak-of-any-band, so spectral
        // leakage from adjacent bands can't muddy the comparison.
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
        // after the step transition (FFT windowing spans the boundary
        // for a few frames on each side).
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
        // 100 samples @ 48 kHz << fft_size of 2048
        let samples = vec![0.0_f32; 100];
        let frames = analyse_samples(&samples, TEST_SR, &SpectrumConfig::default());
        assert_eq!(frames.frame_count(), 0);
    }

    #[test]
    fn silence_produces_all_zero_bytes() {
        // 2 seconds of silence
        let samples = vec![0.0_f32; TEST_SR as usize * 2];
        let frames = analyse_samples(&samples, TEST_SR, &SpectrumConfig::default());
        assert!(frames.frame_count() > 0);
        for byte in &frames.frames {
            assert_eq!(*byte, 0, "silence must quantise to 0");
        }
    }

    /// Predict which log-spaced band a given frequency *should* peak in
    /// for the default config (20 Hz → 20 kHz, 128 bands). Rounds to the
    /// nearest integer band index.
    fn expected_band_for(freq_hz: f32, band_count: usize) -> usize {
        let lo = BAND_FREQ_LOW_HZ.ln();
        let hi = BAND_FREQ_HIGH_HZ.ln();
        let t = (freq_hz.ln() - lo) / (hi - lo);
        (t * band_count as f32).round() as usize
    }

    /// Return the band index with the highest total across all frames.
    /// Breaks ties by preferring the *earlier* band, so saturation tests
    /// don't flake on iterator behaviour.
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
        // 100 Hz sine — log-spaced from 20 Hz to 20 kHz, 128 bands, should
        // peak around band 29-30. Allow ±6 bands of tolerance for Hann
        // window leakage.
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
        // 8 kHz sine — should peak near the top of the band range.
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
        // Sanity: low and high sines must not peak in the same band.
        assert!(peak_band > 64, "8 kHz must be in the upper half of bands");
    }

    #[test]
    fn does_not_panic_on_edge_sample_rates() {
        // 8 kHz — phone-call-quality edge case
        let samples = sine(440.0, 0.5, 8_000);
        let _ = analyse_samples(&samples, 8_000, &SpectrumConfig::default());
        // 96 kHz — hi-res audio
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
        // Externally-tagged, snake_case variant names. The frontend
        // TypeScript types must mirror this shape.
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
        // All three variants survive a postcard round-trip.
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
