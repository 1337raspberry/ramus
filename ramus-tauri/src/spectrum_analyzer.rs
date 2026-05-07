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
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::OnceLock;

use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::{CodecRegistry, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia_adapter_libopus::OpusDecoder;

use ramus_core::playback::spectrum::{
    analyse_samples, write_spec_file, SpectrumConfig, SpectrumFrames, SpectrumState,
};

/// Process-wide codec registry seeded with symphonia's defaults plus the
/// libopus adapter. We can't mutate `symphonia::default::get_codecs()`
/// (it's a `OnceLock`), so we hand-roll a registry once and reuse it for
/// every `analyse_file` call.
fn codec_registry() -> &'static CodecRegistry {
    static REGISTRY: OnceLock<CodecRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut registry = CodecRegistry::new();
        symphonia::default::register_enabled_codecs(&mut registry);
        registry.register_all::<OpusDecoder>();
        registry
    })
}

/// Hard ceiling on the mono PCM buffer assembled during analysis. 300M
/// f32 samples ≈ 1.2 GB peak RAM — well above the legitimate music-track
/// envelope (90 min at 96 kHz mono fits in 518M; 30 min at 192 kHz in
/// 346M) but bounded so a hostile Plex server can't OOM the process by
/// serving a multi-hour audiobook or a header-spoofed stream. Tracks
/// past the cap return `DecodeFailed("track too long")` and persist as
/// `Unavailable` so we don't retry.
const MAX_MONO_SAMPLES: usize = 300_000_000;

/// Errors the analyser surfaces to its caller. Variant names map directly to
/// the `reason` string the frontend displays.
#[derive(Debug)]
pub enum AnalyseError {
    /// File couldn't be opened (missing, permission, etc.).
    OpenFailed(std::io::Error),
    /// symphonia couldn't identify the format. Usually a partial download,
    /// a corrupted file, or a container we don't have the demuxer feature
    /// flag for.
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
    let ext = audio_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase());
    if let Some(e) = ext.as_deref() {
        hint.with_extension(e);
    }

    // For Ogg files (transcoded current track via stream-record), clamp
    // the symphonia source at the last structurally complete page so a
    // torn trailing page can never trip the strict probe with
    // UnexpectedEof. mpv's recorder commits page headers + lacing
    // tables before the page data is fully written; a snapshot taken
    // mid-write has a header advertising N bytes of payload but only
    // K<N bytes on disk, and symphonia's reader bails when asked to
    // read the missing bytes. Empirically: track 100796 at 15,204,352
    // bytes had 383 complete pages plus a header at offset 15,166,809
    // promising 43,002 bytes of payload, only 37,316 of which were
    // present — probe failed. Truncated to 15,166,809 it probed
    // cleanly. The bounded source reports its clamped length via
    // `byte_len()` so symphonia's seek-to-end-and-rewind logic stops
    // at the safe boundary.
    let source: Box<dyn MediaSource> = if ext.as_deref() == Some("ogg") {
        match find_last_complete_ogg_page_end(audio_path) {
            Ok(Some(boundary)) => {
                log::debug!(
                    "spectrum: clamping ogg read at offset {boundary} (file size {})",
                    file.metadata().map(|m| m.len()).unwrap_or(0)
                );
                Box::new(BoundedFileSource::new(file, boundary))
            }
            Ok(None) => {
                log::debug!(
                    "spectrum: no complete ogg pages found in {audio_path:?}, falling through to default reader"
                );
                Box::new(file)
            }
            Err(e) => {
                log::debug!("spectrum: ogg page walk failed for {audio_path:?}: {e}");
                Box::new(file)
            }
        }
    } else {
        Box::new(file)
    };

    let mss = MediaSourceStream::new(source, Default::default());
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| {
            // Surface the actual symphonia failure mode at info-level so a
            // user-visible "Visualiser unavailable while transcoding"
            // placeholder is debuggable from the dev console without a
            // rebuild. The probe rejection reason is otherwise lost — we
            // would only see the human-friendly placeholder.
            log::warn!(
                "spectrum: probe failed for {audio_path:?}: {e:?}",
                audio_path = audio_path
            );
            match e {
                SymphoniaError::Unsupported(msg) => AnalyseError::UnsupportedFormat(msg.to_string()),
                other => AnalyseError::UnsupportedFormat(other.to_string()),
            }
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

    log::debug!(
        "spectrum: probe ok for {audio_path:?} codec={codec_name} sr={:?} channels={:?}",
        track.codec_params.sample_rate,
        track.codec_params.channels.map(|c| c.count()),
    );

    let mut decoder = codec_registry()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| {
            log::warn!("spectrum: decoder make failed for codec={codec_name}: {e:?}");
            AnalyseError::UnsupportedCodec(codec_name.clone())
        })?;

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
                decoder = codec_registry()
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
                if mono.len() > MAX_MONO_SAMPLES {
                    return Err(AnalyseError::DecodeFailed("track too long".into()));
                }
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

/// Walk Ogg page boundaries from the start of the file and return the
/// byte offset just past the last structurally complete page. A page
/// is "complete" when its 27-byte header, the lacing table it
/// announces, AND every payload byte its lacing table promises are all
/// present on disk. Returns `None` if the file isn't Ogg-shaped (no
/// `OggS` magic at offset 0) so the caller can fall back to default
/// reader behaviour.
///
/// Used to clamp symphonia's reads to a safe boundary when the source
/// file is being concurrently written by mpv's recorder. mpv's
/// libavformat muxer writes a page header + lacing table BEFORE all
/// the payload bytes hit disk, so a snapshot taken mid-write has a
/// torn trailing page that symphonia's strict probe rejects with
/// `UnexpectedEof`.
///
/// Takes a path rather than a File handle: on POSIX `try_clone()`
/// shares the file offset (dup(2) creates a new fd pointing at the
/// same open file description), so seeking the clone moves the
/// caller's File too. Opening a fresh file inside avoids that subtle
/// contamination.
fn find_last_complete_ogg_page_end(path: &Path) -> io::Result<Option<u64>> {
    let mut f = File::open(path)?;
    let file_len = f.metadata()?.len();

    let mut last_end: u64 = 0;
    let mut pos: u64 = 0;
    let mut header = [0u8; 27];

    loop {
        if pos + 27 > file_len {
            break;
        }
        f.seek(SeekFrom::Start(pos))?;
        if f.read_exact(&mut header).is_err() {
            break;
        }
        if &header[0..4] != b"OggS" {
            // Mid-file corruption / non-Ogg → bail. If we haven't found
            // any complete page yet, return None so the caller falls
            // through to the unbounded reader.
            return Ok(if last_end == 0 { None } else { Some(last_end) });
        }
        let n_segments = header[26] as u64;
        if pos + 27 + n_segments > file_len {
            break;
        }
        let mut lacing = vec![0u8; n_segments as usize];
        f.seek(SeekFrom::Start(pos + 27))?;
        if f.read_exact(&mut lacing).is_err() {
            break;
        }
        let data_len: u64 = lacing.iter().map(|&b| b as u64).sum();
        let page_end = pos + 27 + n_segments + data_len;
        if page_end > file_len {
            break;
        }
        last_end = page_end;
        pos = page_end;
    }

    Ok(if last_end == 0 { None } else { Some(last_end) })
}

/// `MediaSource` wrapper that clamps reads and seeks to a fixed maximum
/// byte length. Reports its clamped `byte_len` so symphonia's
/// seek-to-end probes (used to find the last granule for duration
/// estimation) hit our safe boundary instead of the live file's
/// torn-page tail.
struct BoundedFileSource {
    file: File,
    max_len: u64,
}

impl BoundedFileSource {
    fn new(file: File, max_len: u64) -> Self {
        Self { file, max_len }
    }
}

impl Read for BoundedFileSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let pos = self.file.stream_position()?;
        if pos >= self.max_len {
            return Ok(0);
        }
        let remaining = self.max_len - pos;
        let want = (buf.len() as u64).min(remaining) as usize;
        self.file.read(&mut buf[..want])
    }
}

impl Seek for BoundedFileSource {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(p) => p.min(self.max_len),
            SeekFrom::End(o) => {
                let base = self.max_len as i64;
                let target = base.saturating_add(o).max(0) as u64;
                target.min(self.max_len)
            }
            SeekFrom::Current(o) => {
                let cur = self.file.stream_position()? as i64;
                let target = cur.saturating_add(o).max(0) as u64;
                target.min(self.max_len)
            }
        };
        self.file.seek(SeekFrom::Start(new_pos))
    }
}

impl MediaSource for BoundedFileSource {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.max_len)
    }
}

/// Average all channels of an `AudioBufferRef` into mono f32 samples in
/// `[-1, 1]` and append them to `out`. symphonia yields whichever integer
/// or float type the codec decoded into (FLAC: S16/S24/S32, MP3/AAC: F32,
/// PCM WAV: U8/S16/S24/S32, Ogg Opus: F32).
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

    /// Build a single Ogg page header + lacing table + payload.
    /// Lacing values describe payload sizes; sum is total payload bytes.
    /// `header_type`: 0x02=BOS, 0x04=EOS, 0x01=continuation.
    fn build_ogg_page(header_type: u8, sequence: u32, lacing: &[u8], payload: &[u8]) -> Vec<u8> {
        let n_segments = lacing.len() as u8;
        let mut page = Vec::with_capacity(27 + lacing.len() + payload.len());
        page.extend_from_slice(b"OggS");
        page.push(0); // stream version
        page.push(header_type);
        page.extend_from_slice(&0u64.to_le_bytes()); // granule position
        page.extend_from_slice(&0xc0ffee_u32.to_le_bytes()); // bitstream serial
        page.extend_from_slice(&sequence.to_le_bytes());
        page.extend_from_slice(&0u32.to_le_bytes()); // CRC (not checked by walker)
        page.push(n_segments);
        page.extend_from_slice(lacing);
        page.extend_from_slice(payload);
        page
    }

    #[test]
    fn find_last_complete_ogg_page_end_returns_none_for_non_ogg() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("not_ogg.bin");
        std::fs::write(&path, b"some random bytes that aren't an ogg file").unwrap();
        let result = find_last_complete_ogg_page_end(&path).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn find_last_complete_ogg_page_end_walks_clean_pages() {
        // Three complete pages back to back; no torn trailing page.
        let dir = tempdir().unwrap();
        let path = dir.path().join("clean.ogg");
        let mut bytes = Vec::new();
        bytes.extend(build_ogg_page(0x02, 0, &[100], &[0xAA; 100]));
        bytes.extend(build_ogg_page(0x00, 1, &[200], &[0xBB; 200]));
        bytes.extend(build_ogg_page(0x00, 2, &[50], &[0xCC; 50]));
        std::fs::write(&path, &bytes).unwrap();

        let result = find_last_complete_ogg_page_end(&path).unwrap();
        assert_eq!(result, Some(bytes.len() as u64));
    }

    #[test]
    fn find_last_complete_ogg_page_end_stops_before_torn_payload() {
        // Two complete pages, then a header advertising more payload
        // than is present on disk (mimics mpv's recorder caught
        // mid-page-write).
        let dir = tempdir().unwrap();
        let path = dir.path().join("torn.ogg");
        let page1 = build_ogg_page(0x02, 0, &[100], &[0xAA; 100]);
        let page2 = build_ogg_page(0x00, 1, &[50], &[0xBB; 50]);
        // Torn page: lacing claims 200 bytes of payload but we only
        // write 80 of them.
        let torn = build_ogg_page(0x00, 2, &[200], &[0xCC; 80]);
        let clean_end = (page1.len() + page2.len()) as u64;
        let mut bytes = Vec::new();
        bytes.extend(&page1);
        bytes.extend(&page2);
        bytes.extend(&torn);
        std::fs::write(&path, &bytes).unwrap();

        let result = find_last_complete_ogg_page_end(&path).unwrap();
        assert_eq!(result, Some(clean_end));
    }

    #[test]
    fn find_last_complete_ogg_page_end_stops_before_torn_lacing() {
        // One complete page, then a header whose declared n_segments
        // would extend past EOF (we only wrote part of the lacing
        // table).
        let dir = tempdir().unwrap();
        let path = dir.path().join("torn_lacing.ogg");
        let page1 = build_ogg_page(0x02, 0, &[100], &[0xAA; 100]);
        let clean_end = page1.len() as u64;
        let mut bytes = page1.clone();
        // Synthetic torn page header: claims 50 lacing entries but we
        // only write 5 of them and no payload.
        bytes.extend_from_slice(b"OggS");
        bytes.push(0);
        bytes.push(0);
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&0xc0ffee_u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.push(50); // n_segments = 50
        bytes.extend_from_slice(&[10, 20, 30, 40, 50]); // only 5 lacing bytes written
        std::fs::write(&path, &bytes).unwrap();

        let result = find_last_complete_ogg_page_end(&path).unwrap();
        assert_eq!(result, Some(clean_end));
    }

    #[test]
    fn bounded_file_source_clamps_reads_and_seeks() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("clamp.bin");
        let data = (0u8..=255).cycle().take(1024).collect::<Vec<u8>>();
        std::fs::write(&path, &data).unwrap();

        let f = File::open(&path).unwrap();
        let mut bounded = BoundedFileSource::new(f, 100);

        // byte_len reports the clamp, not the underlying file size.
        assert_eq!(bounded.byte_len(), Some(100));

        // Read past the clamp returns clamped count.
        let mut buf = [0u8; 200];
        let n = bounded.read(&mut buf).unwrap();
        assert_eq!(n, 100);
        assert_eq!(&buf[..100], &data[..100]);
        // Subsequent read returns 0 (EOF).
        let n2 = bounded.read(&mut buf).unwrap();
        assert_eq!(n2, 0);

        // Seek-from-end is relative to the clamp.
        let pos = bounded.seek(SeekFrom::End(0)).unwrap();
        assert_eq!(pos, 100);
        // Seek-from-start past the clamp gets pulled back.
        let pos = bounded.seek(SeekFrom::Start(500)).unwrap();
        assert_eq!(pos, 100);
        let pos = bounded.seek(SeekFrom::Start(50)).unwrap();
        assert_eq!(pos, 50);
        let mut small = [0u8; 10];
        let n = bounded.read(&mut small).unwrap();
        assert_eq!(n, 10);
        assert_eq!(&small[..], &data[50..60]);
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
    fn spec_file_oversized_returns_none() {
        // A planted file larger than MAX_SPEC_FILE_SIZE must short-circuit
        // without read_to_end allocating the whole thing.
        use ramus_core::playback::spectrum::MAX_SPEC_FILE_SIZE;
        let dir = tempdir().unwrap();
        let fake = dir.path().join("huge.flac");
        let spec = spec_file_path(&fake);
        let f = File::create(&spec).unwrap();
        f.set_len(MAX_SPEC_FILE_SIZE + 1).unwrap();
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
