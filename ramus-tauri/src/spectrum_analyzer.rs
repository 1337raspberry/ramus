//! File-to-spectrogram analyser for the focus-mode visualiser.
//!
//! I/O shell around `ramus_core::playback::spectrum`: opens a cached audio file
//! with symphonia, decodes it into mono f32 samples, feeds them to the DSP
//! module, and serialises the result to a sibling `<audio>.spec` file. The DSP
//! is pure math in ramus-core so it stays testable without real files.
//!
//! symphonia is a sync API — call `analyse_file` and the on-disk helpers from a
//! `tokio::task::spawn_blocking` block. A 5-minute FLAC takes ~1–2s on a modern
//! CPU, which would otherwise stall a tokio worker thread.
//!
//! On-disk format: a 4-byte `RSPF` magic followed by a postcard-encoded
//! `SpectrumState`. Only `Ready(frames)` and `Unavailable { reason }` are
//! written — `Analysing` is a runtime state only. A missing / corrupt /
//! magic-mismatch file is treated as "not analysed yet" so `read_spec_file`
//! returns `None` and the prefetch path regenerates it.

use std::fs::File;
use std::path::Path;

use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use ramus_core::playback::spectrum::{
    analyse_samples, write_spec_file, SpectrumConfig, SpectrumFrames, SpectrumState,
};

/// Errors the analyser surfaces to its caller. Variant names map directly to
/// the `reason` string the frontend displays.
#[derive(Debug)]
pub enum AnalyseError {
    /// File couldn't be opened (missing, permission, etc.).
    OpenFailed(std::io::Error),
    /// symphonia couldn't identify the format. Usually HLS manifest text, a
    /// partial download, or an unsupported codec.
    UnsupportedFormat(String),
    /// File parsed but the codec isn't in our feature flags (e.g. DSD, WMA).
    /// Surfaces as `unavailable: "unsupported_codec"`.
    UnsupportedCodec(String),
    /// Decoder ran but something went wrong mid-stream.
    DecodeFailed(String),
    /// File has no audio tracks.
    NoAudioTrack,
}

impl AnalyseError {
    /// Human-readable reason for the frontend placeholder. Kept short.
    pub fn reason(&self) -> String {
        match self {
            Self::OpenFailed(_) => "file_missing".to_string(),
            Self::UnsupportedFormat(_) => "transcoding".to_string(),
            Self::UnsupportedCodec(codec) => format!("unsupported_codec: {codec}"),
            Self::DecodeFailed(_) => "decode_failed".to_string(),
            Self::NoAudioTrack => "no_audio_track".to_string(),
        }
    }
}

impl std::fmt::Display for AnalyseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenFailed(e) => write!(f, "open failed: {e}"),
            Self::UnsupportedFormat(s) => write!(f, "unsupported format: {s}"),
            Self::UnsupportedCodec(s) => write!(f, "unsupported codec: {s}"),
            Self::DecodeFailed(s) => write!(f, "decode failed: {s}"),
            Self::NoAudioTrack => write!(f, "no audio track"),
        }
    }
}

impl std::error::Error for AnalyseError {}

/// Decode a cached audio file and run the DSP analyser over it.
///
/// Called from `prefetch.rs` after a successful download. Returns `Err` if the
/// file can't be decoded for any reason; callers wrap the error in
/// `SpectrumState::Unavailable` via `AnalyseError::reason()` and persist it
/// with `write_spec_file`.
pub fn analyse_file(audio_path: &Path) -> Result<SpectrumFrames, AnalyseError> {
    let file = File::open(audio_path).map_err(AnalyseError::OpenFailed)?;

    // Extension hint lets symphonia pick the right demuxer first; it probes
    // and falls back if wrong.
    let mut hint = Hint::new();
    if let Some(ext) = audio_path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| match e {
            SymphoniaError::Unsupported(msg) => AnalyseError::UnsupportedFormat(msg.to_string()),
            other => AnalyseError::UnsupportedFormat(other.to_string()),
        })?;

    let mut format = probed.format;
    let track = format.default_track().ok_or(AnalyseError::NoAudioTrack)?;
    let track_id = track.id;
    let codec_name = track
        .codec_params
        .codec
        .to_string()
        .trim()
        .to_ascii_lowercase();

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|_| AnalyseError::UnsupportedCodec(codec_name.clone()))?;

    // Accumulate decoded samples as mono f32. A 5-min 48 kHz track is
    // 14.4M samples × 4 bytes = 57 MB peak RAM, acceptable for one-shot
    // analysis. If this ever becomes a problem, stream the STFT window
    // by window instead of buffering the full signal.
    let mut mono: Vec<f32> = Vec::new();
    let mut sample_rate: u32 = 0;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                // Clean EOF.
                break;
            }
            Err(SymphoniaError::ResetRequired) => {
                // New chained stream (e.g. Ogg chapter) — remake the decoder.
                let track = format.default_track().ok_or(AnalyseError::NoAudioTrack)?;
                decoder = symphonia::default::get_codecs()
                    .make(&track.codec_params, &DecoderOptions::default())
                    .map_err(|_| AnalyseError::UnsupportedCodec(codec_name.clone()))?;
                continue;
            }
            Err(e) => return Err(AnalyseError::DecodeFailed(e.to_string())),
        };

        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(audio_buf) => {
                // Capture sample rate from the first real packet.
                if sample_rate == 0 {
                    sample_rate = audio_buf.spec().rate;
                }
                mix_to_mono(&audio_buf, &mut mono);
            }
            Err(SymphoniaError::DecodeError(_)) => {
                // Recoverable: skip the bad packet.
                continue;
            }
            Err(e) => return Err(AnalyseError::DecodeFailed(e.to_string())),
        }
    }

    if sample_rate == 0 || mono.is_empty() {
        return Err(AnalyseError::DecodeFailed("empty decoded stream".into()));
    }

    Ok(analyse_samples(
        &mono,
        sample_rate,
        &SpectrumConfig::default(),
    ))
}

/// Average all channels of an `AudioBufferRef` into mono f32 samples in
/// [-1, 1] and append them to `out`. symphonia yields whichever integer or
/// float type the codec decodes into (FLAC: S16/S24/S32, MP3/AAC: F32, PCM
/// WAV: U8/S16/S24/S32, Ogg Opus: F32).
///
/// Conversion convention:
/// - signed `iN::MAX` maps to +1.0 (and `iN::MIN` to slightly below -1.0,
///   which the spectrum analyser clamps harmlessly)
/// - unsigned PCM is centre-shifted by its midpoint then divided by the
///   same midpoint so both extremes land at ±1.0
fn mix_to_mono(audio_buf: &AudioBufferRef<'_>, out: &mut Vec<f32>) {
    // One macro per sample type: expands to a typed loop over the concrete
    // `AudioBuffer<T>` where `T: Sample`. Factoring this behind a generic
    // helper hits `Signal` trait bound issues.
    macro_rules! mix_typed {
        ($buf:ident, $to_f32:expr) => {{
            let channels = $buf.spec().channels.count();
            let frames = $buf.frames();
            if channels == 0 || frames == 0 {
                return;
            }
            out.reserve(frames);
            let inv_channels = 1.0 / channels as f32;
            for frame_idx in 0..frames {
                let mut sum = 0.0_f32;
                for ch in 0..channels {
                    let sample = $buf.chan(ch)[frame_idx];
                    sum += ($to_f32)(sample);
                }
                out.push(sum * inv_channels);
            }
        }};
    }

    // Midpoints for unsigned PCM centre-shifts.
    const U8_MID: f32 = 128.0;
    const U16_MID: f32 = 32_768.0;
    const U24_MID: f32 = 8_388_608.0;
    const U32_MID: f32 = 2_147_483_648.0;
    // Signed 24-bit full-scale used by the i24 normaliser. Numerically equal
    // to U24_MID today, but kept distinct so refactoring one normaliser
    // convention doesn't silently break the other path.
    const S24_MID: f32 = 8_388_608.0;

    // Signed full-scale normalisers.
    const S8_SCALE: f32 = 1.0 / 127.0;
    const S16_SCALE: f32 = 1.0 / 32_767.0;
    const S32_SCALE: f32 = 1.0 / 2_147_483_647.0;

    match audio_buf {
        AudioBufferRef::F32(b) => mix_typed!(b, |s: f32| s),
        AudioBufferRef::F64(b) => mix_typed!(b, |s: f64| s as f32),
        AudioBufferRef::U8(b) => mix_typed!(b, |s: u8| (s as f32 - U8_MID) / U8_MID),
        AudioBufferRef::U16(b) => mix_typed!(b, |s: u16| (s as f32 - U16_MID) / U16_MID),
        AudioBufferRef::U24(b) => mix_typed!(b, |s: symphonia::core::sample::u24| (s.inner()
            as f32
            - U24_MID)
            / U24_MID),
        AudioBufferRef::U32(b) => mix_typed!(b, |s: u32| (s as f32 - U32_MID) / U32_MID),
        AudioBufferRef::S8(b) => mix_typed!(b, |s: i8| s as f32 * S8_SCALE),
        AudioBufferRef::S16(b) => mix_typed!(b, |s: i16| s as f32 * S16_SCALE),
        AudioBufferRef::S24(b) => mix_typed!(b, |s: symphonia::core::sample::i24| s.inner() as f32
            / S24_MID),
        AudioBufferRef::S32(b) => mix_typed!(b, |s: i32| s as f32 * S32_SCALE),
    }
}

/// Run `analyse_file`, wrap the result as a `SpectrumState`, and persist it
/// to disk. Called by `prefetch.rs` from a spawn_blocking task.
pub fn analyse_and_persist(audio_path: &Path) -> SpectrumState {
    match analyse_file(audio_path) {
        Ok(frames) => {
            let state = SpectrumState::Ready(frames);
            if let Err(e) = write_spec_file(audio_path, &state) {
                log::warn!("spectrum: failed to persist .spec for {audio_path:?}: {e}");
            }
            state
        }
        Err(err) => {
            let state = SpectrumState::Unavailable {
                reason: err.reason(),
            };
            log::debug!("spectrum: analysis unavailable for {audio_path:?}: {err}");
            // Persist the unavailable marker so repeat plays don't retry a
            // decode that will always fail.
            let _ = write_spec_file(audio_path, &state);
            state
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ramus_core::playback::spectrum::{read_spec_file, spec_file_path};
    use std::io::Write;
    use tempfile::tempdir;

    /// Build a PCM 16-bit stereo 48 kHz WAV of the given duration filled
    /// with a sine wave. Enough for symphonia to probe and decode cleanly;
    /// shipping the generator keeps the test self-contained.
    fn write_sine_wav(path: &Path, freq_hz: f32, secs: f32, sample_rate: u32) {
        use std::f32::consts::PI;
        let channels: u16 = 2;
        let bits_per_sample: u16 = 16;
        let block_align = channels * bits_per_sample / 8;
        let byte_rate = sample_rate * block_align as u32;
        let num_samples = (sample_rate as f32 * secs) as u32;
        let data_size = num_samples * block_align as u32;
        let file_size = 36 + data_size;

        let mut f = File::create(path).unwrap();
        f.write_all(b"RIFF").unwrap();
        f.write_all(&file_size.to_le_bytes()).unwrap();
        f.write_all(b"WAVE").unwrap();
        f.write_all(b"fmt ").unwrap();
        f.write_all(&16u32.to_le_bytes()).unwrap();
        f.write_all(&1u16.to_le_bytes()).unwrap();
        f.write_all(&channels.to_le_bytes()).unwrap();
        f.write_all(&sample_rate.to_le_bytes()).unwrap();
        f.write_all(&byte_rate.to_le_bytes()).unwrap();
        f.write_all(&block_align.to_le_bytes()).unwrap();
        f.write_all(&bits_per_sample.to_le_bytes()).unwrap();
        f.write_all(b"data").unwrap();
        f.write_all(&data_size.to_le_bytes()).unwrap();
        for i in 0..num_samples {
            let t = i as f32 / sample_rate as f32;
            let sample = ((2.0 * PI * freq_hz * t).sin() * 0.5 * i16::MAX as f32) as i16;
            f.write_all(&sample.to_le_bytes()).unwrap();
            f.write_all(&sample.to_le_bytes()).unwrap();
        }
        f.sync_all().unwrap();
    }

    #[test]
    fn spec_file_path_appends_suffix() {
        let p = Path::new("/cache/abc123.flac");
        assert_eq!(spec_file_path(p), Path::new("/cache/abc123.flac.spec"));
    }

    #[test]
    fn analyse_file_decodes_sine_wav() {
        let dir = tempdir().unwrap();
        let wav = dir.path().join("sine.wav");
        write_sine_wav(&wav, 1000.0, 1.0, 48_000);

        let frames = analyse_file(&wav).expect("analyse");
        assert_eq!(frames.sample_rate, 48_000);
        assert_eq!(frames.band_count, 128);
        assert!(frames.frame_count() > 10);
    }

    #[test]
    fn analyse_file_fails_on_garbage_bytes() {
        let dir = tempdir().unwrap();
        let garbage = dir.path().join("trash.bin");
        std::fs::write(&garbage, b"this is not an audio file").unwrap();

        let err = analyse_file(&garbage).expect_err("should fail");
        match err {
            AnalyseError::UnsupportedFormat(_) => {}
            other => panic!("expected UnsupportedFormat, got {other:?}"),
        }
    }

    #[test]
    fn analyse_file_fails_on_hls_manifest() {
        // Simulates what Plex returns for a transcoded stream (HLS playlist).
        // symphonia can't decode this and should surface UnsupportedFormat →
        // "transcoding".
        let dir = tempdir().unwrap();
        let m3u8 = dir.path().join("stream.m3u8");
        std::fs::write(
            &m3u8,
            b"#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:10\n",
        )
        .unwrap();

        let err = analyse_file(&m3u8).expect_err("should fail");
        assert_eq!(err.reason(), "transcoding");
    }

    #[test]
    fn spec_file_roundtrip() {
        let dir = tempdir().unwrap();
        let wav = dir.path().join("sine.wav");
        write_sine_wav(&wav, 440.0, 0.5, 48_000);

        let original = analyse_file(&wav).unwrap();
        let state = SpectrumState::Ready(original.clone());
        write_spec_file(&wav, &state).unwrap();

        assert!(spec_file_path(&wav).exists());
        let loaded = read_spec_file(&wav).expect("read");
        match loaded {
            SpectrumState::Ready(frames) => {
                assert_eq!(frames, original);
            }
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[test]
    fn spec_file_unavailable_is_persisted() {
        let dir = tempdir().unwrap();
        let fake = dir.path().join("fake.flac");
        std::fs::write(&fake, b"not a real flac").unwrap();

        // analyse_and_persist must write an Unavailable marker even when
        // analysis fails, so subsequent plays don't retry.
        let state = analyse_and_persist(&fake);
        match state {
            SpectrumState::Unavailable { reason } => assert_eq!(reason, "transcoding"),
            other => panic!("expected Unavailable, got {other:?}"),
        }
        assert!(spec_file_path(&fake).exists());

        let loaded = read_spec_file(&fake).expect("read");
        assert!(matches!(loaded, SpectrumState::Unavailable { .. }));
    }

    #[test]
    fn spec_file_missing_returns_none() {
        let dir = tempdir().unwrap();
        let nowhere = dir.path().join("nothing.flac");
        assert!(read_spec_file(&nowhere).is_none());
    }

    #[test]
    fn spec_file_wrong_magic_returns_none() {
        let dir = tempdir().unwrap();
        let fake = dir.path().join("bad.flac");
        let spec = spec_file_path(&fake);
        std::fs::write(&spec, b"WRONGMAGICSTUFF").unwrap();
        assert!(read_spec_file(&fake).is_none());
    }

    #[test]
    fn write_spec_file_drops_analysing_state() {
        let dir = tempdir().unwrap();
        let fake = dir.path().join("track.flac");
        // Writing Analysing succeeds as a no-op; it must not create a file.
        write_spec_file(&fake, &SpectrumState::Analysing).unwrap();
        assert!(!spec_file_path(&fake).exists());
    }
}
