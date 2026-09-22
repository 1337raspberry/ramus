//! Live spectrum tap for the focus-mode visualiser.
//!
//! The tap is a libavfilter graph hosted in mpv's `--af` chain (the same
//! chain the equalizer lives in). It splits the decoded audio: the main
//! copy passes through `anull` untouched, and a side copy is brought to
//! stereo and split into its left and right channels, each of which
//! feeds an identical bank. A bank fans its channel through one
//! `bandpass` biquad per band and joins the results into a single
//! N-channel stream, squares it against itself (`amultiply`), low-passes
//! it into a power envelope and resamples it down to the frame rate, so
//! from there on the bank handles 60 samples a second per band. `aeval`
//! turns each sample into the band's level in dBFS, scaled into the range
//! a 16-bit sample can hold, and `ashowinfo` prints one Adler-32 checksum
//! per channel plane for every one-sample frame. A checksum over two
//! bytes is exactly invertible, so the printed line is the bank's half of
//! the frame: mpv forwards it to its client log stream, where the
//! platform layer intercepts it and this module decodes it and pairs the
//! two halves by timestamp.
//!
//! Two banks rather than one wide one because swresample, which the
//! decimator and the format conversions run on, refuses more than 64
//! channels; a bank per channel keeps every stage within that and gives
//! the visualiser real stereo rather than a mirrored mono spectrum.
//!
//! `astats` would be the obvious way to print per-band levels, but its
//! per-frame bookkeeping (an 8192-bin histogram walk per channel, whatever
//! measures were asked for) cost more than the rest of the graph put
//! together. The checksum route costs nothing measurable.
//!
//! Hosting the tap in `--af` (never `--lavfi-complex`) keeps the audio
//! output open across file boundaries, so gapless playback is unaffected,
//! and because it is post-decoder it works for direct play, transcodes
//! and cached files alike.
//!
//! This module is pure: graph generation, log-line parsing and the
//! dB → bar-height mapping. Nothing here touches mpv.

use serde::Serialize;

/// Number of log-spaced analysis bands per channel the desktop tap runs
/// by default.
///
/// Each band is one `bandpass` biquad plus one channel through a bank's
/// envelope chain, and there is a bank per stereo channel. The whole tap
/// measures around 11 % of one core at 64 bands per channel and 60 fps,
/// or 17 % where the main-path cut is needed (`tap_probe_cost`), only
/// while the visualiser is mounted. 64 is also `MAX_TAP_BANDS`: a bank's
/// resampler refuses more channels.
pub const DEFAULT_TAP_BANDS: usize = 64;

/// Channels the tap analyses: one bank each for left and right. The side
/// branch is forced to stereo first, so mono sources are duplicated into
/// both banks and multichannel sources are downmixed.
pub const TAP_CHANNELS: usize = 2;

/// Frames per second the tap emits. The envelope is resampled to this
/// rate, so it is also the rate the printer runs at.
pub const DEFAULT_TAP_FPS: u32 = 60;

/// Upper bound on the frame rate the graph is asked for: past this the
/// envelope resampler and the log transport only cost more.
pub const MAX_TAP_FPS: u32 = 120;

/// Frame size the main path is cut into before the tap splits off it,
/// on FFmpeg builds older than 8.0 (`TapConfig::cut_main_path`).
///
/// libavfilter advances the side branch only while it drains the graph
/// after each input frame, and before 8.0 that drain stops at the first
/// filter reporting an empty source, which leaves the side branch's
/// frame cutter (`asetnsamples`) one emitted frame per input frame. The
/// tap then cannot produce more frames per second than the decoder
/// hands mpv (43 for a 44.1 kHz source in 1024-sample frames, far fewer
/// for a FLAC block), falls behind at a fixed pace and never recovers.
/// FFmpeg 8.0's drain skips that condition and empties the branch. On
/// the older builds, cutting the main path into frames this small keeps
/// the input rate above `DEFAULT_TAP_FPS` for any source at 32 kHz or
/// more (below that the tap falls behind; the audio is fine). Smaller
/// frames would cover lower rates but every filter in both paths then
/// runs per frame: 256 costs the tap almost twice what 512 does. The
/// audio is untouched (frames are re-cut, not padded: `p=0`), so gapless
/// playback is unaffected, and the cut only exists while the tap is
/// installed.
pub const MAIN_FRAME_SAMPLES: usize = 512;

/// Sample rate the side branch is forced to. The main path is never
/// resampled; hi-res sources are analysed at 48 kHz, which is plenty for
/// bands that top out at 16 kHz.
pub const TAP_SAMPLE_RATE: u32 = 48_000;

/// Centre frequency of the lowest band, in Hz.
pub const TAP_FREQ_LOW_HZ: f32 = 50.0;

/// Centre frequency of the highest band, in Hz.
pub const TAP_FREQ_HIGH_HZ: f32 = 16_000.0;

/// Fewest bands a graph can be generated for (the octave width divides
/// by `bands - 1`).
pub const MIN_TAP_BANDS: usize = 2;

/// Most bands per channel a graph can be generated for: a bank's `join`
/// channel layout is spelled as a 64-bit channel mask, and its resampler
/// refuses more channels anyway.
pub const MAX_TAP_BANDS: usize = 64;

/// Corner frequency of the low-pass that turns each band's squared
/// signal into a power envelope, in Hz. Sets the bars' attack and release
/// (a time constant of roughly 8 ms) and keeps the envelope below the
/// frame rate's Nyquist so the decimating resampler has little to alias.
pub const ENVELOPE_LOWPASS_HZ: u32 = 20;

/// `filter_size` for the envelope's decimating resampler. The default
/// (32) builds a kernel of 32 × the decimation ratio taps per output
/// sample, measured at about 4 % of a core for 48 channels at an 800×
/// ratio. The envelope is already band-limited by the low-pass, so a
/// short kernel loses nothing visible.
pub const ENVELOPE_RESAMPLE_FILTER_SIZE: u32 = 2;

/// Lowest level the tap can express, in dBFS. The power envelope is
/// clamped to `TAP_POWER_FLOOR` before the logarithm, which lands this
/// value on the 16-bit sample's negative full scale. It sits below
/// `DB_FLOOR`, the mapper's own floor, so nothing visible is lost.
pub const TAP_DB_FLOOR: f32 = -100.0;

/// The linear power the envelope is clamped to, `10^(TAP_DB_FLOOR / 10)`,
/// spelled the way it appears in the graph.
pub const TAP_POWER_FLOOR: &str = "1e-10";

/// Divisor that maps dBFS onto the sample range: a level of
/// `-TAP_DB_SCALE` dBFS becomes -1.0, negative full scale in the 16-bit
/// format. One 16-bit step is therefore about 0.003 dB.
pub const TAP_DB_SCALE: f32 = 100.0;

// The floor lands exactly on negative full scale after scaling, and sits
// below anything the mapper can show.
const _: () = assert!(TAP_DB_FLOOR / TAP_DB_SCALE == -1.0);
const _: () = assert!(TAP_DB_FLOOR <= DB_FLOOR);

/// Prefix of every log line the `ashowinfo` printer emits. The numeric
/// suffix is the filter's index in the graph and changes every time the
/// chain is rebuilt, so callers must match on this prefix alone.
pub const TAP_LOG_PREFIX: &str = "Parsed_ashowinfo_";

/// Longest stitched log line held while its fragments arrive. A 64-band
/// line is under 1 KB; anything longer is not the printer's output.
const MAX_TAP_LINE: usize = 4096;

/// Most decoded half-frames held while waiting for their partner from the
/// other bank. The graph scheduler runs one bank's input chunk to
/// completion before the other's, so one printer normally leads by a
/// chunk (five or so frames); anything beyond this is a lost partner.
const MAX_PENDING_HALVES: usize = 32;

/// A held half-frame this much older than a newly decoded one is stale
/// (the stream was rebuilt or seeked forward) and is dropped rather than
/// left waiting for a partner that will never come. Only older halves
/// are judged: a straggler from before a seek must not evict the fresh
/// halves queued after it. Halves left over from a backwards seek are
/// newer than everything that follows and age out through
/// `MAX_PENDING_HALVES` instead.
const PENDING_MAX_SPREAD_S: f64 = 1.0;

/// Absolute floor for every dBFS value the mapper handles. Levels the tap
/// reports at or below this (silence, a band with no energy) clamp here
/// so the arithmetic downstream stays finite.
pub const DB_FLOOR: f32 = -90.0;

/// Width of the visible dynamic range window, in dB. Each frame maps
/// `[peak - DYNAMIC_RANGE_DB, peak + PEAK_HEADROOM_DB]` onto 0..255. 55 dB
/// covers a pop song's peak-to-quiet span comfortably and gives classical
/// music room to show dynamics.
pub const DYNAMIC_RANGE_DB: f32 = 55.0;

/// Headroom added above the running peak before it becomes the window's
/// ceiling, so the loudest frame doesn't saturate the top of the visual
/// range and accents still have somewhere to reach.
pub const PEAK_HEADROOM_DB: f32 = 2.0;

/// Compression curve exponent applied after dB → 0..1 normalisation.
/// Values below 1 lift quiet passages so they stay visually readable.
pub const QUANT_COMPRESSION: f32 = 0.6;

/// Running-peak seed for a freshly started level mapper, in dBFS. Chosen
/// so a track's quiet intro renders at a sensible height before the first
/// loud frame re-anchors the window.
pub const PEAK_SEED_DB: f32 = -20.0;

/// How fast the running peak relaxes toward the current frame's maximum,
/// in dB per second, once the music gets quieter than the last peak.
pub const PEAK_DECAY_DB_PER_SEC: f32 = 6.0;

/// Spectral tilt applied to every band before the running peak and the
/// window see it, in dB per octave: the spectrum is rotated about the
/// geometric centre of the band range, so the lowest band moves down and
/// the highest up by the same amount. Music's energy falls away above the
/// low-mids by several dB per octave, and against a single peak-anchored
/// window the treble bands then sit near the floor; a small positive tilt
/// lets them through without flattening the bass-led shape. The
/// `RAMUS_TAP_TILT` environment variable overrides it for tuning.
pub const TILT_DB_PER_OCTAVE: f32 = 1.0;

/// Lowest value the running peak is allowed to decay to. Keeps a fade-out
/// or a stretch of near-silence from being auto-gained up to full-height
/// bars: anything more than `DYNAMIC_RANGE_DB` below this floor renders as
/// zero.
pub const PEAK_FLOOR_DB: f32 = -40.0;

/// Shape of the tap graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TapConfig {
    /// Number of analysis bands per channel (`MIN_TAP_BANDS..=MAX_TAP_BANDS`).
    pub bands: usize,
    /// Frames per second (`1..=MAX_TAP_FPS`): the envelope is resampled
    /// to exactly this rate.
    pub fps: u32,
    /// Cut the main path into `MAIN_FRAME_SAMPLES` frames ahead of the
    /// split. Needed on FFmpeg builds older than 8.0 (see the constant);
    /// a needless cost everywhere else, so it is off by default and the
    /// player turns it on from what its libmpv reports.
    pub cut_main_path: bool,
}

impl Default for TapConfig {
    fn default() -> Self {
        Self {
            bands: DEFAULT_TAP_BANDS,
            fps: DEFAULT_TAP_FPS,
            cut_main_path: false,
        }
    }
}

impl TapConfig {
    /// Clamp the band count and frame rate into the ranges the graph
    /// generator supports.
    pub fn normalised(self) -> Self {
        Self {
            bands: self.bands.clamp(MIN_TAP_BANDS, MAX_TAP_BANDS),
            fps: self.fps.clamp(1, MAX_TAP_FPS),
            cut_main_path: self.cut_main_path,
        }
    }

    /// Values in one complete frame: every band of every channel.
    pub fn frame_width(&self) -> usize {
        self.normalised().bands * TAP_CHANNELS
    }
}

/// Width of each band in octaves: the log range from the lowest to the
/// highest centre frequency split evenly across `bands - 1` gaps.
pub fn band_octave_width(cfg: &TapConfig) -> f32 {
    let cfg = cfg.normalised();
    (TAP_FREQ_HIGH_HZ / TAP_FREQ_LOW_HZ).log2() / (cfg.bands - 1) as f32
}

/// Centre frequency of every band, log-spaced from `TAP_FREQ_LOW_HZ` to
/// `TAP_FREQ_HIGH_HZ` inclusive.
pub fn band_frequencies(cfg: &TapConfig) -> Vec<f32> {
    let cfg = cfg.normalised();
    let w = band_octave_width(&cfg);
    (0..cfg.bands)
        .map(|k| TAP_FREQ_LOW_HZ * 2f32.powf(k as f32 * w))
        .collect()
}

/// The `channel_layout` value handed to `join` for `n` mono inputs.
///
/// FFmpeg's `<N>c` shorthand only parses for channel counts that have a
/// default layout (it stops working above 24), so the layout is spelled
/// as a hex channel mask with the low `n` bits set. 24 uses the named
/// `22.2` layout, which the older channel-layout parser in FFmpeg 4.4
/// (the Ubuntu 22.04 libmpv) is known to accept.
pub fn channel_layout_spec(n: usize) -> String {
    let n = n.clamp(1, MAX_TAP_BANDS);
    if n == 24 {
        return "22.2".to_string();
    }
    let mask: u64 = if n >= 64 { u64::MAX } else { (1u64 << n) - 1 };
    format!("0x{mask:x}")
}

/// FFmpeg's name for the channel at mask bit `bit`. The first eighteen
/// are named in every FFmpeg release; the rest are spelled `USR<bit>`,
/// which the channel parser accepts from FFmpeg 5.1 on.
fn channel_name(bit: usize) -> String {
    const NAMED: [&str; 18] = [
        "FL", "FR", "FC", "LFE", "BL", "BR", "FLC", "FRC", "BC", "SL", "SR", "TC", "TFL", "TFC",
        "TFR", "TBL", "TBC", "TBR",
    ];
    NAMED
        .get(bit)
        .map_or_else(|| format!("USR{bit}"), |name| name.to_string())
}

/// The output channels of `channel_layout_spec(n)`, in channel order.
fn layout_channels(n: usize) -> Vec<String> {
    let n = n.clamp(1, MAX_TAP_BANDS);
    if n == 24 {
        // `22.2` is the first eighteen mask bits plus these six.
        let tail = ["LFE2", "TSL", "TSR", "BFC", "BFL", "BFR"].map(String::from);
        return (0..18).map(channel_name).chain(tail).collect();
    }
    (0..n).map(channel_name).collect()
}

/// `join`'s `map` option: input `k`'s only channel onto output channel
/// `k`. Left implicit, `join` guesses the mapping from the inputs'
/// channel names, and the guess depends on which named channels they
/// carry: the right bank's inputs are all `FR`, which lands input 0 on
/// output channel 1 (`FR`) and input 1 on channel 0, so that bank's two
/// lowest bands come out swapped while the left bank (all `FL`) happens
/// to be identity.
fn join_map(n: usize) -> String {
    layout_channels(n)
        .iter()
        .enumerate()
        .map(|(k, name)| format!("{k}.0-{name}"))
        .collect::<Vec<_>>()
        .join("|")
}

/// Generate the tap filter graph for `cfg`. The result is the inner graph
/// only; wrap it in `lavfi=[...]` for mpv's `af` property (see
/// `player::compose_af`).
///
/// Formatting goes through `format!`, which always writes `.` for the
/// decimal point regardless of locale.
///
/// The left bank is written before the right one, so its `ashowinfo`
/// gets the lower filter index in the log prefix; the parser relies on
/// that to tell the halves apart.
///
/// Each bank's printer stage, in order: `aeval` evaluates one expression
/// per sample on every channel (`val(ch)` is the current channel's
/// sample; FFmpeg's expression language has only the natural `log`,
/// hence the `10 / ln 10` factor; the comma inside `max()` is escaped
/// because a bare comma ends the filter's arguments at the graph level;
/// and `channel_layout=same` is required or `aeval` outputs as many
/// channels as it has expressions, one). `aformat` converts to planar
/// 16-bit, `asetnsamples` cuts the stream into one-sample frames so
/// `ashowinfo` prints a line per frame, and `anullsink` discards the
/// audio.
pub fn tap_graph(cfg: &TapConfig) -> String {
    let cfg = cfg.normalised();
    let n = cfg.bands;
    let w = band_octave_width(&cfg);
    let freqs = band_frequencies(&cfg);

    let mut g = String::with_capacity(256 + TAP_CHANNELS * n * 64);
    if cfg.cut_main_path {
        g.push_str(&format!("[in]asetnsamples=n={MAIN_FRAME_SAMPLES}:p=0,"));
        g.push_str("asplit=2[main][side];");
    } else {
        g.push_str("[in]asplit=2[main][side];");
    }
    g.push_str(&format!(
        "[side]aformat=channel_layouts=stereo:sample_fmts=fltp:sample_rates={TAP_SAMPLE_RATE},\
         channelsplit=channel_layout=stereo[l][r];"
    ));
    for tag in ["l", "r"] {
        push_bank(&mut g, tag, &cfg, w, &freqs);
    }
    g.push_str("[main]anull[out]");
    g
}

/// Append one channel's bank to the graph: the input pad `[tag]` in, the
/// printer at the end, every internal pad label prefixed with `tag`.
fn push_bank(g: &mut String, tag: &str, cfg: &TapConfig, w: f32, freqs: &[f32]) {
    let n = cfg.bands;
    g.push_str(&format!("[{tag}]asplit={n}"));
    for k in 0..n {
        g.push_str(&format!("[{tag}s{k}]"));
    }
    g.push(';');
    for (k, f) in freqs.iter().enumerate() {
        g.push_str(&format!(
            "[{tag}s{k}]bandpass=f={f:.1}:width_type=o:w={w:.3}[{tag}b{k}];"
        ));
    }
    for k in 0..n {
        g.push_str(&format!("[{tag}b{k}]"));
    }
    g.push_str(&format!(
        "join=inputs={n}:channel_layout={layout}:map={map},\
         asplit=2[{tag}p][{tag}q];[{tag}p][{tag}q]amultiply,lowpass=f={lp},\
         aresample={fps}:filter_size={taps},\
         aeval=exprs='{db_per_neper:.10}*log(max(val(ch)\\,{floor}))/{scale}':channel_layout=same,\
         aformat=sample_fmts=s16p,asetnsamples=n=1:p=0,ashowinfo,anullsink;",
        layout = channel_layout_spec(n),
        map = join_map(n),
        lp = ENVELOPE_LOWPASS_HZ,
        fps = cfg.fps,
        taps = ENVELOPE_RESAMPLE_FILTER_SIZE,
        db_per_neper = 10.0f64 / std::f64::consts::LN_10,
        floor = TAP_POWER_FLOOR,
        scale = TAP_DB_SCALE as u32,
    ));
}

/// One analysis frame: the mpv timeline position of the frame's sample
/// and the level of each band in dBFS, the left channel's bands followed
/// by the right channel's (`TapConfig::frame_width` values).
#[derive(Debug, Clone, PartialEq)]
pub struct TapFrame {
    pub pts: f64,
    pub db: Vec<f32>,
}

/// A frame after the level mapper: the track-timeline position and one
/// quantised bar height (0..255) per band, left channel then right.
/// Crosses the IPC boundary as is, hence the camelCase rename.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpectrumFrame {
    pub pos: f64,
    pub bands: Vec<u8>,
}

/// Outcome of feeding one log message to `TapLineParser`.
#[derive(Debug, Clone, PartialEq)]
pub enum TapFeed {
    /// The message was not part of the tap transport; forward it as usual.
    Ignored,
    /// The message belonged to the tap (a fragment of a line, a line that
    /// did not decode, or one channel's half of a frame still waiting for
    /// the other) and has been swallowed.
    Consumed,
    /// The message completed a frame.
    Frame(TapFrame),
}

/// One bank's decoded line, waiting for the other bank's line with the
/// same timestamp.
struct HalfFrame {
    /// The `ashowinfo` filter index from the log prefix: lower is the
    /// left bank.
    printer: u32,
    pts: f64,
    db: Vec<f32>,
}

/// Reassembles `ashowinfo` log messages into frames.
///
/// The printer writes one line per frame, in pieces: a header, the list
/// opener, one checksum per channel plane and the closer are each a
/// separate log call. mpv's message layer buffers partial lines and
/// delivers the line whole, with the filter-name prefix at its start:
///
/// ```text
/// Parsed_ashowinfo_76: n:30 pts:31 pts_time:0.516667 fmt:s16p channels:64 chlayout:64 channels (FL+FR+FC+…+USR rate:60 nb_samples:1 checksum:97BA516D plane_checksums: [ 020C0158 00D400BC … ]⏎
/// ```
///
/// The `chlayout` field is the layout description cut at a fixed width,
/// so it carries spaces and an unclosed parenthesis; fields are found by
/// name, never by position.
///
/// Should a build deliver the pieces separately instead, they are held
/// until the one carrying the newline arrives.
///
/// Each bank prints its own line per frame, and the graph scheduler runs
/// one bank's input chunk to completion before the other's, so lines
/// arrive in runs of one printer then the other. Decoded halves wait in
/// a small queue until the other printer's line with the same timestamp
/// arrives; the half from the lower filter index is the left channel.
///
/// The parser is defensive throughout: a line that does not decode to
/// exactly the configured band count (a message lost from mpv's bounded
/// log ring, another FFmpeg message interleaved mid-line) is dropped, a
/// half whose partner never comes ages out, and a rebuilt stream's old
/// halves are dropped as soon as the new timeline shows up.
pub struct TapLineParser {
    bands: usize,
    held: String,
    pending: Vec<HalfFrame>,
}

impl TapLineParser {
    /// `bands` is the count per channel: each printed line must carry
    /// exactly that many planes.
    pub fn new(bands: usize) -> Self {
        Self {
            bands: bands.max(1),
            held: String::new(),
            pending: Vec::with_capacity(MAX_PENDING_HALVES),
        }
    }

    /// Forget a half-stitched line and any halves still waiting for a
    /// partner. Called when the filter chain is rebuilt: the new graph's
    /// printers start over, and a leftover half must not pair with one
    /// of theirs.
    pub fn reset(&mut self) {
        self.held.clear();
        self.pending.clear();
    }

    /// Feed one mpv log message from the `ffmpeg` prefix, newline and all.
    pub fn feed(&mut self, fragment: &str) -> TapFeed {
        if !self.held.is_empty() {
            if fragment.starts_with(TAP_LOG_PREFIX) {
                // A new line has begun: the rest of the held one is lost.
                self.held.clear();
            } else {
                self.held.push_str(fragment);
                if fragment.ends_with('\n') {
                    let line = std::mem::take(&mut self.held);
                    return self.decode(&line);
                }
                if self.held.len() > MAX_TAP_LINE {
                    self.held.clear();
                }
                return TapFeed::Consumed;
            }
        }
        if !fragment.starts_with(TAP_LOG_PREFIX) {
            return TapFeed::Ignored;
        }
        if fragment.ends_with('\n') {
            return self.decode(fragment);
        }
        self.held.push_str(fragment);
        TapFeed::Consumed
    }

    fn decode(&mut self, line: &str) -> TapFeed {
        let Some(half) = parse_showinfo_line(line, self.bands) else {
            return TapFeed::Consumed;
        };
        // The stream was rebuilt or seeked forward: anything well before
        // this line will never be paired.
        self.pending
            .retain(|h| h.pts >= half.pts - PENDING_MAX_SPREAD_S);

        if let Some(i) = self
            .pending
            .iter()
            .position(|h| h.pts == half.pts && h.printer != half.printer)
        {
            let other = self.pending.remove(i);
            let (mut left, right) = if other.printer < half.printer {
                (other.db, half.db)
            } else {
                (half.db, other.db)
            };
            left.extend(right);
            return TapFeed::Frame(TapFrame {
                pts: half.pts,
                db: left,
            });
        }
        if let Some(dup) = self
            .pending
            .iter_mut()
            .find(|h| h.pts == half.pts && h.printer == half.printer)
        {
            dup.db = half.db;
            return TapFeed::Consumed;
        }
        if self.pending.len() >= MAX_PENDING_HALVES {
            self.pending.remove(0);
        }
        self.pending.push(half);
        TapFeed::Consumed
    }
}

/// Strip `Parsed_ashowinfo_<N>: ` and return the filter index and the
/// payload, or `None` if the line is not a tap line.
fn strip_tap_prefix(line: &str) -> Option<(u32, &str)> {
    let rest = line.strip_prefix(TAP_LOG_PREFIX)?;
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    let index = rest[..digits].parse::<u32>().ok()?;
    let rest = rest[digits..].strip_prefix(':')?;
    Some((index, rest.trim_start()))
}

/// Decode one complete `ashowinfo` line into one bank's half-frame of
/// `bands` levels.
///
/// The header must describe a one-sample planar 16-bit frame (anything
/// else means the graph is not the one this parser expects) and the
/// checksum list must hold exactly one entry per band.
fn parse_showinfo_line(line: &str, bands: usize) -> Option<HalfFrame> {
    let (printer, payload) = strip_tap_prefix(line)?;
    let (head, tail) = payload.split_once("plane_checksums: [")?;
    let (planes, _) = tail.split_once(']')?;

    let mut pts = None;
    let mut planar_s16 = false;
    let mut one_sample = false;
    for tok in head.split_whitespace() {
        if let Some(v) = tok.strip_prefix("pts_time:") {
            pts = v.parse::<f64>().ok().filter(|v| v.is_finite());
        } else if tok == "fmt:s16p" {
            planar_s16 = true;
        } else if tok == "nb_samples:1" {
            one_sample = true;
        }
    }
    if !(planar_s16 && one_sample) {
        return None;
    }
    let pts = pts?;

    let mut db = Vec::with_capacity(bands);
    for tok in planes.split_whitespace() {
        let checksum = u32::from_str_radix(tok, 16).ok()?;
        db.push(sample_to_db(adler32_to_s16(checksum)?));
    }
    if db.len() != bands {
        return None;
    }
    Some(HalfFrame { printer, pts, db })
}

/// Invert the Adler-32 of a two-byte plane back into its sample.
///
/// Adler-32 runs two sums over the bytes: `a` adds each byte, `b` adds
/// `a` after each byte. `ashowinfo` seeds both at 0 (zlib's convention
/// seeds `a` at 1), so for bytes `x, y` it prints `a = x + y` and
/// `b = 2x + y`, both far below the modulus, giving `x = b - a` and
/// `y = a - x`. The bytes are in memory order, so native-endian assembly
/// recovers the sample on any host.
///
/// Returns `None` when the arithmetic leaves the byte range, which
/// catches most checksums over more than two bytes; a checksum over
/// fewer bytes is indistinguishable from one with a zero byte, so the
/// caller must check the frame's format and sample count itself.
pub fn adler32_to_s16(checksum: u32) -> Option<i16> {
    let a = checksum & 0xffff;
    let b = checksum >> 16;
    let x = b.checked_sub(a)?;
    let y = a.checked_sub(x)?;
    if x > 0xff || y > 0xff {
        return None;
    }
    Some(i16::from_ne_bytes([x as u8, y as u8]))
}

/// Undo the graph's scaling: a 16-bit sample back to dBFS, clamped to the
/// mapper's floor.
fn sample_to_db(sample: i16) -> f32 {
    (sample as f32 / 32768.0 * TAP_DB_SCALE).max(DB_FLOOR)
}

fn sanitise_db(db: f32) -> f32 {
    if db.is_finite() {
        db.max(DB_FLOOR)
    } else if db == f32::INFINITY {
        0.0
    } else {
        DB_FLOOR
    }
}

/// Quantise a dBFS value to 0..255 against an explicit `[floor, ceiling]`
/// window.
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

/// Maps per-band dBFS levels to bar heights against a running peak.
///
/// A whole-track analysis could find each track's peak up front; a live
/// feed cannot, so the peak is tracked instead: it jumps up instantly to
/// any louder frame and relaxes downward at `PEAK_DECAY_DB_PER_SEC`
/// toward the current frame's maximum, never below `PEAK_FLOOR_DB`. Each
/// frame then quantises `[peak - DYNAMIC_RANGE_DB, peak + PEAK_HEADROOM_DB]`
/// onto 0..255.
#[derive(Debug, Clone)]
pub struct LevelMapper {
    peak_db: f32,
    decay_per_frame: f32,
    tilt_db_per_octave: f32,
    /// Per-value tilt offsets for the frame width last mapped, rebuilt
    /// when the width changes.
    tilt: Vec<f32>,
    /// The tilted values of the frame being mapped; kept between frames
    /// so mapping allocates nothing but its output.
    tilted: Vec<f32>,
}

impl LevelMapper {
    /// `fps` is the tap's frame rate; the decay is applied per frame. The
    /// tilt is `TILT_DB_PER_OCTAVE`.
    pub fn new(fps: u32) -> Self {
        Self::with_tilt(fps, TILT_DB_PER_OCTAVE)
    }

    /// As `new`, with an explicit spectral tilt in dB per octave.
    pub fn with_tilt(fps: u32, tilt_db_per_octave: f32) -> Self {
        Self {
            peak_db: PEAK_SEED_DB,
            decay_per_frame: PEAK_DECAY_DB_PER_SEC / fps.max(1) as f32,
            tilt_db_per_octave: if tilt_db_per_octave.is_finite() {
                tilt_db_per_octave
            } else {
                0.0
            },
            tilt: Vec::new(),
            tilted: Vec::new(),
        }
    }

    /// Forget the running peak (fresh tap, new listening session).
    pub fn reset(&mut self) {
        self.peak_db = PEAK_SEED_DB;
    }

    /// Change the spectral tilt; the next frame is mapped with it. A
    /// non-finite value means no tilt.
    pub fn set_tilt(&mut self, db_per_octave: f32) {
        self.tilt_db_per_octave = if db_per_octave.is_finite() {
            db_per_octave
        } else {
            0.0
        };
        self.tilt.clear();
    }

    /// The spectral tilt in dB per octave.
    pub fn tilt(&self) -> f32 {
        self.tilt_db_per_octave
    }

    /// Current running peak in dBFS.
    pub fn peak_db(&self) -> f32 {
        self.peak_db
    }

    /// Quantise one frame of band levels to bar heights, advancing the
    /// running peak. The tilt is applied first, so the peak anchors to the
    /// tilted spectrum; a band at the floor is silence and stays there.
    pub fn map(&mut self, db: &[f32]) -> Vec<u8> {
        if self.tilt.len() != db.len() {
            self.tilt = tilt_offsets(db.len(), self.tilt_db_per_octave);
        }
        self.tilted.clear();
        self.tilted
            .extend(db.iter().zip(&self.tilt).map(|(&d, &t)| {
                let d = sanitise_db(d);
                if d > DB_FLOOR {
                    (d + t).max(DB_FLOOR)
                } else {
                    d
                }
            }));
        let db = &self.tilted;
        let frame_max = db.iter().copied().fold(DB_FLOOR, f32::max);
        if frame_max > self.peak_db {
            self.peak_db = frame_max;
        } else {
            self.peak_db = (self.peak_db - self.decay_per_frame)
                .max(frame_max)
                .max(PEAK_FLOOR_DB);
        }
        let ceiling = self.peak_db + PEAK_HEADROOM_DB;
        let floor = (ceiling - DYNAMIC_RANGE_DB).max(DB_FLOOR);
        db.iter()
            .map(|&d| quantise_db_range(d, floor, ceiling))
            .collect()
    }
}

/// The tilt offset for every value of a `len`-wide frame (`TAP_CHANNELS`
/// runs of equal-octave bands): `slope` dB per octave, zero at the centre
/// of the band range. A width that isn't a whole number of channels, or
/// a single band, gets no tilt.
fn tilt_offsets(len: usize, slope: f32) -> Vec<f32> {
    let n = len / TAP_CHANNELS;
    if slope == 0.0 || n < 2 || !len.is_multiple_of(TAP_CHANNELS) {
        return vec![0.0; len];
    }
    let span = (TAP_FREQ_HIGH_HZ / TAP_FREQ_LOW_HZ).log2();
    (0..len)
        .map(|i| {
            let k = (i % n) as f32 / (n - 1) as f32;
            slope * span * (k - 0.5)
        })
        .collect()
}

impl Default for LevelMapper {
    fn default() -> Self {
        Self::new(DEFAULT_TAP_FPS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn cfg(bands: usize) -> TapConfig {
        TapConfig {
            bands,
            fps: 60,
            cut_main_path: false,
        }
    }

    // --- graph generation ---

    #[test]
    fn frequencies_are_log_spaced_from_low_to_high() {
        let f = band_frequencies(&cfg(48));
        assert_eq!(f.len(), 48);
        assert!((f[0] - TAP_FREQ_LOW_HZ).abs() < 0.01);
        assert!((f[47] - TAP_FREQ_HIGH_HZ).abs() < 0.5);
        for w in f.windows(2) {
            assert!(w[1] > w[0]);
        }
        // Constant ratio between neighbours.
        let r0 = f[1] / f[0];
        let r1 = f[40] / f[39];
        assert!((r0 - r1).abs() < 1e-3);
    }

    #[test]
    fn octave_width_covers_the_whole_range() {
        let w = band_octave_width(&cfg(48));
        let total = w * 47.0;
        assert!((total - (TAP_FREQ_HIGH_HZ / TAP_FREQ_LOW_HZ).log2()).abs() < 1e-4);
    }

    #[test]
    fn channel_layout_spelling() {
        assert_eq!(channel_layout_spec(24), "22.2");
        assert_eq!(channel_layout_spec(48), "0xffffffffffff");
        assert_eq!(channel_layout_spec(2), "0x3");
        assert_eq!(channel_layout_spec(1), "0x1");
        assert_eq!(channel_layout_spec(64), "0xffffffffffffffff");
        // Out of range clamps rather than overflowing the shift.
        assert_eq!(channel_layout_spec(200), "0xffffffffffffffff");
        assert_eq!(channel_layout_spec(0), "0x1");
    }

    #[test]
    fn frame_rate_sets_the_envelope_resample_rate() {
        assert!(tap_graph(&cfg(48)).contains("aresample=60:"));
        let c = TapConfig {
            bands: 48,
            fps: 30,
            cut_main_path: false,
        };
        assert!(tap_graph(&c).contains("aresample=30:"));
    }

    /// Prints the default graph so it can be pasted into an mpv CLI run
    /// for re-verification or profiling on another libmpv build:
    ///
    /// ```text
    /// cargo test -p ramus-core -- --ignored print_tap_graph --nocapture
    /// ```
    #[test]
    #[ignore]
    fn print_tap_graph() {
        println!("{}", tap_graph(&TapConfig::default()));
    }

    #[test]
    fn two_band_graph_exact() {
        let g = tap_graph(&cfg(2));
        let bank = |t: &str| {
            format!(
                "[{t}]asplit=2[{t}s0][{t}s1];\
                 [{t}s0]bandpass=f=50.0:width_type=o:w=8.322[{t}b0];\
                 [{t}s1]bandpass=f=16000.0:width_type=o:w=8.322[{t}b1];\
                 [{t}b0][{t}b1]join=inputs=2:channel_layout=0x3:map=0.0-FL|1.0-FR,\
                 asplit=2[{t}p][{t}q];[{t}p][{t}q]amultiply,lowpass=f=20,\
                 aresample=60:filter_size=2,\
                 aeval=exprs='4.3429448190*log(max(val(ch)\\,1e-10))/100':channel_layout=same,\
                 aformat=sample_fmts=s16p,asetnsamples=n=1:p=0,ashowinfo,anullsink;"
            )
        };
        assert_eq!(
            g,
            format!(
                "[in]asplit=2[main][side];\
                 [side]aformat=channel_layouts=stereo:sample_fmts=fltp:sample_rates=48000,\
                 channelsplit=channel_layout=stereo[l][r];{}{}[main]anull[out]",
                bank("l"),
                bank("r")
            )
        );
    }

    #[test]
    fn default_graph_shape() {
        let g = tap_graph(&TapConfig::default());
        assert_eq!(g.matches("bandpass=").count(), TAP_CHANNELS * DEFAULT_TAP_BANDS);
        assert_eq!(g.matches("ashowinfo").count(), TAP_CHANNELS);
        assert_eq!(g.matches(&format!("asplit={DEFAULT_TAP_BANDS}")).count(), TAP_CHANNELS);
        assert_eq!(
            g.matches(&format!(
                "join=inputs={DEFAULT_TAP_BANDS}:channel_layout={}",
                channel_layout_spec(DEFAULT_TAP_BANDS)
            ))
            .count(),
            TAP_CHANNELS
        );
        // The left bank is written first, so its printer gets the lower
        // filter index.
        assert!(g.find("[l]asplit").unwrap() < g.find("[r]asplit").unwrap());
        // Envelope per bank: square, low-pass, decimate to the frame
        // rate, then dB, 16-bit, one-sample frames and the checksum
        // printer.
        assert!(g.contains("[lp][lq]amultiply,lowpass=f=20,aresample=60:filter_size=2,aeval="));
        assert!(g.contains("[rp][rq]amultiply,lowpass=f=20,aresample=60:filter_size=2,aeval="));
        assert!(g.contains(":channel_layout=same,aformat=sample_fmts=s16p,asetnsamples=n=1:p=0,ashowinfo,anullsink;"));
        assert!(!g.contains("astats"));
        assert!(!g.contains("ametadata"));
        assert!(g.ends_with("[main]anull[out]"));
        // Exactly one unconnected input and output pad.
        assert_eq!(g.matches("[in]").count(), 1);
        assert_eq!(g.matches("[out]").count(), 1);
        // The main path is never resampled.
        assert!(!g.contains("[main]aformat"));
        assert!(!g.contains("[main]aresample"));
        assert_eq!(TapConfig::default().frame_width(), TAP_CHANNELS * DEFAULT_TAP_BANDS);
    }

    #[test]
    fn main_path_is_cut_only_when_asked() {
        // Off by default: the cut is a cost the player only pays on FFmpeg
        // builds whose graph drain leaves the side branch behind.
        let g = tap_graph(&TapConfig::default());
        assert!(g.starts_with("[in]asplit=2[main][side];"));
        assert!(!g.contains("asetnsamples=n=512"));

        let cut = TapConfig {
            cut_main_path: true,
            ..TapConfig::default()
        };
        let g = tap_graph(&cut);
        assert!(g.starts_with(&format!(
            "[in]asetnsamples=n={MAIN_FRAME_SAMPLES}:p=0,asplit=2[main][side];"
        )));
        // The cut keeps the input rate above the tap's frame rate for any
        // source at 32 kHz or more.
        assert!(MAIN_FRAME_SAMPLES as u32 * DEFAULT_TAP_FPS <= 32_000);
    }

    #[test]
    fn join_maps_every_input_to_its_own_output_channel() {
        // Without a map, `join` guesses from the inputs' channel names:
        // the right bank's inputs each carry `FR`, which the guess puts on
        // output channel 1 first, so that bank's two lowest bands swap.
        let g = tap_graph(&cfg(4));
        assert_eq!(g.matches(":map=0.0-FL|1.0-FR|2.0-FC|3.0-LFE,").count(), 2);

        // 24 bands ride the named `22.2` layout, whose channels past the
        // first eighteen are LFE2, TSL, TSR, BFC, BFL, BFR in that order.
        let g = tap_graph(&cfg(24));
        assert!(g.contains("|17.0-TBR|18.0-LFE2|19.0-TSL|20.0-TSR|21.0-BFC|22.0-BFL|23.0-BFR,"));

        let g = tap_graph(&TapConfig::default());
        let map = g.split(":map=").nth(1).unwrap().split(',').next().unwrap();
        let entries: Vec<&str> = map.split('|').collect();
        assert_eq!(entries.len(), DEFAULT_TAP_BANDS);
        assert_eq!(entries[17], "17.0-TBR");
        assert_eq!(entries[18], "18.0-USR18");
        assert_eq!(entries[63], "63.0-USR63");
        for (k, e) in entries.iter().enumerate() {
            assert!(e.starts_with(&format!("{k}.0-")), "entry {k}: {e}");
        }
    }

    #[test]
    fn graph_uses_decimal_points_only() {
        // A comma decimal separator (a non-POSIX locale) would still leave
        // the text "w=0" in place, so parse every width back instead.
        let g = tap_graph(&TapConfig::default());
        let mut seen = 0;
        for (i, _) in g.match_indices(":w=") {
            let value = g[i + 3..].split('[').next().unwrap();
            assert!(
                value.parse::<f32>().is_ok(),
                "width is not a plain decimal: {value}"
            );
            seen += 1;
        }
        assert_eq!(seen, 2 * DEFAULT_TAP_BANDS);
    }

    #[test]
    fn power_floor_in_the_graph_matches_the_db_floor() {
        let floor: f64 = TAP_POWER_FLOOR.parse().unwrap();
        assert!((10.0 * floor.log10() - TAP_DB_FLOOR as f64).abs() < 1e-9);
    }

    #[test]
    fn config_normalises_out_of_range() {
        let c = TapConfig {
            bands: 0,
            fps: 0,
            cut_main_path: false,
        }
        .normalised();
        assert_eq!(c.bands, MIN_TAP_BANDS);
        assert_eq!(c.fps, 1);
        let c = TapConfig {
            bands: 500,
            fps: 1_000_000,
            cut_main_path: true,
        }
        .normalised();
        assert!(c.cut_main_path);
        assert_eq!(c.bands, MAX_TAP_BANDS);
        assert_eq!(c.fps, MAX_TAP_FPS);
    }

    // --- a line exactly as mpv delivers it ---

    /// Captured from mpv 0.41 / FFmpeg 9.0.1 running the default graph on
    /// a 440 Hz tone: the text of one `ffmpeg`-prefixed client-API
    /// message, newline included, for each bank's printer at the same
    /// frame. Every other parser test builds its input from `fragments`,
    /// which is this module's own model of the format; this one pins the
    /// model to the real thing.
    const CAPTURED_LEFT: &str = "Parsed_ashowinfo_76: n:30 pts:31 pts_time:0.516667 fmt:s16p channels:64 chlayout:64 channels (FL+FR+FC+LFE+BL+BR+FLC+FRC+BC+SL+SR+TC+TFL+TFC+TFR+TBL+TBC+TBR+USR18+USR19+USR20+USR21+USR22+USR23+USR24+USR25+USR rate:60 nb_samples:1 checksum:48864DF9 plane_checksums: [ 01C20138 01DB0145 01F20151 02170164 023A0176 0263018B 029601A5 00CE00C2 011100E4 0160010C 01BB013A 022C0173 02AF01B5 01530108 021A016C 010C00E6 023F0180 01BD0140 01AF013A 0237017F 01AA013A 029B01B4 024C018F 01FE016C 02BF01CF 011C00F9 010400EA 00EE00DD 00D500CF 01E20154 01AE0139 01FA015E 02A601B3 0197012B 00BE00BE 020C0164 017B011B 010400DF 02A201AD 02530185 02100163 01D90147 01AC0130 0189011E 016C010F 01570104 014800FC 014100F8 013E00F6 014500F9 015200FF 01670109 01840117 01AD012B 01E40146 02290168 027C0191 00E800C7 016B0108 020C0158 00D400BC 01C70135 00F300CB 02620182 ]\n";
    const CAPTURED_RIGHT: &str = "Parsed_ashowinfo_151: n:30 pts:31 pts_time:0.516667 fmt:s16p channels:64 chlayout:64 channels (FL+FR+FC+LFE+BL+BR+FLC+FRC+BC+SL+SR+TC+TFL+TFC+TFR+TBL+TBC+TBR+USR18+USR19+USR20+USR21+USR22+USR23+USR24+USR25+USR rate:60 nb_samples:1 checksum:48864DF9 plane_checksums: [ 01C20138 01DB0145 01F20151 02170164 023A0176 0263018B 029601A5 00CE00C2 011100E4 0160010C 01BB013A 022C0173 02AF01B5 01530108 021A016C 010C00E6 023F0180 01BD0140 01AF013A 0237017F 01AA013A 029B01B4 024C018F 01FE016C 02BF01CF 011C00F9 010400EA 00EE00DD 00D500CF 01E20154 01AE0139 01FA015E 02A601B3 0197012B 00BE00BE 020C0164 017B011B 010400DF 02A201AD 02530185 02100163 01D90147 01AC0130 0189011E 016C010F 01570104 014800FC 014100F8 013E00F6 014500F9 015200FF 01670109 01840117 01AD012B 01E40146 02290168 027C0191 00E800C7 016B0108 020C0158 00D400BC 01C70135 00F300CB 02620182 ]\n";

    #[test]
    fn a_captured_line_decodes_to_the_bands_in_order() {
        let half = parse_showinfo_line(CAPTURED_LEFT, 64).expect("captured line decodes");
        assert_eq!(half.printer, 76);
        assert!((half.pts - 0.516667).abs() < 1e-9);
        assert_eq!(half.db.len(), 64);
        // The last plane's checksum, worked by hand: A = 0x0182 = b0 + b1,
        // B = 0x0262 = 2·b0 + b1, so the bytes are 224, 162 and the little-
        // endian sample is 0xA2E0 = -23840.
        assert_eq!(adler32_to_s16(0x0262_0182), Some(-23840));
        assert!((half.db[63] - sample_to_db(-23840)).abs() < 1e-6);
        assert!(half.db[63] < -70.0 && half.db[63] > -75.0);
        // Plane k is band k: the tone's energy peaks in the band nearest 440 Hz.
        let freqs = band_frequencies(&TapConfig::default());
        let loudest = (0..64)
            .max_by(|&a, &b| half.db[a].partial_cmp(&half.db[b]).unwrap())
            .unwrap();
        assert!(
            ((freqs[loudest] - 440.0) / 440.0).abs() < 0.15,
            "loudest band {loudest} at {:.0} Hz",
            freqs[loudest]
        );
    }

    #[test]
    fn captured_lines_from_both_printers_pair_into_one_frame() {
        let mut p = TapLineParser::new(64);
        assert_eq!(p.feed(CAPTURED_LEFT), TapFeed::Consumed);
        match p.feed(CAPTURED_RIGHT) {
            TapFeed::Frame(f) => {
                assert!((f.pts - 0.516667).abs() < 1e-9);
                assert_eq!(f.db.len(), 128);
                let left = parse_showinfo_line(CAPTURED_LEFT, 64).unwrap();
                assert_eq!(&f.db[..64], &left.db[..]);
            }
            other => panic!("expected a frame, got {other:?}"),
        }
    }

    // --- checksum decoding ---

    /// Reference Adler-32 seeded at 0, the way `ashowinfo` calls it.
    fn adler32(bytes: &[u8]) -> u32 {
        let (mut a, mut b) = (0u32, 0u32);
        for &x in bytes {
            a = (a + x as u32) % 65521;
            b = (b + a) % 65521;
        }
        (b << 16) | a
    }

    /// The checksum `ashowinfo` prints for a plane holding `db` after the
    /// graph's scaling and 16-bit conversion.
    fn encode_db(db: f32) -> u32 {
        let s = (db / TAP_DB_SCALE * 32768.0).round().clamp(-32768.0, 32767.0) as i16;
        adler32(&s.to_ne_bytes())
    }

    #[test]
    fn adler32_of_two_bytes_inverts_exactly() {
        for s in [0i16, 1, -1, 127, 128, 255, 256, -256, 12345, -9617, i16::MAX, i16::MIN] {
            assert_eq!(adler32_to_s16(adler32(&s.to_ne_bytes())), Some(s), "{s}");
        }
    }

    #[test]
    fn checksums_over_wider_planes_are_mostly_rejected() {
        // Four bytes with a large leading byte overflow the byte range.
        assert_eq!(adler32_to_s16(adler32(&[200, 2, 3, 4])), None);
        assert_eq!(adler32_to_s16(0xffff_ffff), None);
        assert_eq!(adler32_to_s16(0x0000_ffff), None);
        // But a short plane looks like a zero byte followed by the value,
        // which is why the parser insists on `fmt:s16p` and
        // `nb_samples:1` rather than trusting the arithmetic alone.
        assert_eq!(adler32_to_s16(adler32(&[7])), Some(i16::from_ne_bytes([0, 7])));
    }

    #[test]
    fn captured_checksums_decode_to_plausible_levels() {
        // Seen in live runs: 01BA014A → −29.35 dBFS, and 00DF00DF, whose
        // sample has a zero low byte (bytes [0, 223]) → −25.78 dBFS.
        let s = adler32_to_s16(0x01BA_014A).unwrap();
        assert_eq!(s, i16::from_ne_bytes([112, 218]));
        assert!((sample_to_db(s) - -29.35).abs() < 0.01);
        let s = adler32_to_s16(0x00DF_00DF).unwrap();
        assert_eq!(s, i16::from_ne_bytes([0, 223]));
        assert!((sample_to_db(s) - -25.78).abs() < 0.01);
    }

    #[test]
    fn sample_scaling_covers_the_floor_and_full_scale() {
        assert_eq!(sample_to_db(0), 0.0);
        // Negative full scale is the graph's floor, clamped to the
        // mapper's.
        assert_eq!(sample_to_db(i16::MIN), DB_FLOOR);
        let minus_sixty = (-60.0 / TAP_DB_SCALE * 32768.0) as i16;
        assert!((sample_to_db(minus_sixty) - -60.0).abs() < 0.01);
        // Round trip through the printer's encoding.
        let s = adler32_to_s16(encode_db(-42.5)).unwrap();
        assert!((sample_to_db(s) - -42.5).abs() < 0.01);
    }

    // --- log-line parsing ---

    /// Filter indices the two printers get in a real graph (left bank
    /// first, so the lower one).
    const LEFT_PRINTER: u32 = 60;
    const RIGHT_PRINTER: u32 = 119;

    /// The pieces FFmpeg writes for one bank's line: header with the
    /// filter prefix, list opener, one checksum per plane, closer.
    fn fragments(printer: u32, n: u64, pts: f64, db: &[f32]) -> Vec<String> {
        let mut v = vec![format!(
            "Parsed_ashowinfo_{printer}: n:{n} pts:{} pts_time:{pts} fmt:s16p channels:{c} chlayout:{c} channels rate:60 nb_samples:1 checksum:00000000 ",
            n * 800,
            c = db.len()
        )];
        v.push("plane_checksums: [ ".to_string());
        for &d in db {
            v.push(format!("{:08X} ", encode_db(d)));
        }
        v.push("]\n".to_string());
        v
    }

    /// One bank's line as mpv delivers it: whole.
    fn line(printer: u32, n: u64, pts: f64, db: &[f32]) -> String {
        fragments(printer, n, pts, db).concat()
    }

    /// Both banks' lines for one frame, left first.
    fn frame_lines(n: u64, pts: f64, left: &[f32], right: &[f32]) -> Vec<String> {
        vec![
            line(LEFT_PRINTER, n, pts, left),
            line(RIGHT_PRINTER, n, pts, right),
        ]
    }

    fn feed_all(p: &mut TapLineParser, lines: &[String]) -> Vec<TapFrame> {
        let mut out = Vec::new();
        for f in lines {
            if let TapFeed::Frame(frame) = p.feed(f) {
                out.push(frame);
            }
        }
        out
    }

    fn close(a: &[f32], b: &[f32]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01)
    }

    #[test]
    fn pairs_the_two_banks_into_one_frame_left_first() {
        let mut p = TapLineParser::new(3);
        let lines = frame_lines(12, 0.2, &[-31.4, -29.0, -72.8], &[-40.0, -41.0, -42.0]);
        assert_eq!(p.feed(&lines[0]), TapFeed::Consumed);
        match p.feed(&lines[1]) {
            TapFeed::Frame(frame) => {
                assert!((frame.pts - 0.2).abs() < 1e-9);
                assert!(
                    close(&frame.db, &[-31.4, -29.0, -72.8, -40.0, -41.0, -42.0]),
                    "{:?}",
                    frame.db
                );
            }
            other => panic!("{other:?}"),
        }
        assert!(p.pending.is_empty());
    }

    #[test]
    fn right_bank_arriving_first_still_lands_on_the_right() {
        let mut p = TapLineParser::new(2);
        let mut lines = frame_lines(0, 1.5, &[-10.0, -11.0], &[-20.0, -21.0]);
        lines.reverse();
        let frames = feed_all(&mut p, &lines);
        assert_eq!(frames.len(), 1);
        assert!(close(&frames[0].db, &[-10.0, -11.0, -20.0, -21.0]), "{:?}", frames[0].db);
    }

    #[test]
    fn banks_running_ahead_by_a_chunk_pair_up_in_order() {
        // The scheduler runs one bank's chunk to completion before the
        // other's: five left lines, then five right lines.
        let mut p = TapLineParser::new(1);
        let mut lines = Vec::new();
        for n in 0..5 {
            lines.push(line(LEFT_PRINTER, n, n as f64 / 60.0, &[-(n as f32)]));
        }
        for n in 0..5 {
            lines.push(line(RIGHT_PRINTER, n, n as f64 / 60.0, &[-10.0 - n as f32]));
        }
        let frames = feed_all(&mut p, &lines);
        assert_eq!(frames.len(), 5);
        for (n, f) in frames.iter().enumerate() {
            assert!((f.pts - n as f64 / 60.0).abs() < 1e-9);
            assert!(close(&f.db, &[-(n as f32), -10.0 - n as f32]), "{:?}", f.db);
        }
        assert!(p.pending.is_empty());
    }

    #[test]
    fn stitches_fragments_into_a_line() {
        let mut p = TapLineParser::new(3);
        let frags = fragments(LEFT_PRINTER, 12, 0.2, &[-31.4, -29.0, -72.8]);
        assert_eq!(frags.len(), 6);
        // Every fragment is swallowed; the completed line is a half
        // waiting for its partner.
        for f in &frags {
            assert_eq!(p.feed(f), TapFeed::Consumed);
        }
        assert!(p.held.is_empty());
        assert_eq!(p.pending.len(), 1);
        assert!(matches!(
            p.feed(&line(RIGHT_PRINTER, 12, 0.2, &[-1.0, -2.0, -3.0])),
            TapFeed::Frame(_)
        ));
    }

    #[test]
    fn unrelated_messages_are_ignored_and_the_fragments_pass_through() {
        let mut p = TapLineParser::new(2);
        assert_eq!(p.feed("http: HTTP/1.1 200 OK\n"), TapFeed::Ignored);
        assert_eq!(p.feed("Parsed_equalizer_3: something\n"), TapFeed::Ignored);
        assert_eq!(p.feed(""), TapFeed::Ignored);
        // A prefix-less fragment while nothing is held is not ours.
        assert_eq!(p.feed("0123ABCD "), TapFeed::Ignored);
    }

    #[test]
    fn printer_indices_only_need_to_be_distinct_and_ordered() {
        let mut p = TapLineParser::new(1);
        let lines = vec![
            line(9, 1, 0.0166667, &[-20.0]),
            line(4, 1, 0.0166667, &[-30.0]),
        ];
        let frames = feed_all(&mut p, &lines);
        assert_eq!(frames.len(), 1);
        // Lower index is the left channel regardless of arrival order.
        assert!(close(&frames[0].db, &[-30.0, -20.0]));
    }

    #[test]
    fn a_line_with_the_wrong_plane_count_is_dropped() {
        let mut p = TapLineParser::new(3);
        assert_eq!(p.feed(&line(LEFT_PRINTER, 0, 0.0, &[-20.0, -20.0])), TapFeed::Consumed);
        assert_eq!(p.feed(&line(LEFT_PRINTER, 0, 0.0, &[-20.0; 4])), TapFeed::Consumed);
        assert!(p.pending.is_empty());
        // The parser is clean afterwards.
        assert_eq!(
            feed_all(&mut p, &frame_lines(1, 0.1, &[-20.0; 3], &[-20.0; 3])).len(),
            1
        );
    }

    #[test]
    fn a_line_of_the_wrong_shape_is_dropped() {
        for (from, to) in [
            ("fmt:s16p", "fmt:fltp"),
            ("nb_samples:1", "nb_samples:4"),
            ("pts_time:0", "pts_time:nan"),
        ] {
            let mut p = TapLineParser::new(1);
            let bad = line(LEFT_PRINTER, 0, 0.0, &[-20.0]).replace(from, to);
            assert_eq!(p.feed(&bad), TapFeed::Consumed, "{from}");
            assert!(p.pending.is_empty(), "{from}");
        }
    }

    #[test]
    fn a_checksum_that_cannot_be_two_bytes_drops_the_line() {
        for junk in ["FFFFFFFF ", "zzzz "] {
            let mut p = TapLineParser::new(2);
            let mut frags = fragments(LEFT_PRINTER, 0, 0.0, &[-20.0, -30.0]);
            frags[3] = junk.to_string();
            for f in &frags {
                assert_eq!(p.feed(f), TapFeed::Consumed);
            }
            assert!(p.pending.is_empty(), "{junk}");
        }
    }

    #[test]
    fn an_interleaved_message_costs_one_line_then_recovers() {
        let mut p = TapLineParser::new(2);
        let frags = fragments(LEFT_PRINTER, 0, 0.0, &[-20.0, -30.0]);
        // Header and opener arrive, then another FFmpeg message lands
        // mid-line with its own newline.
        p.feed(&frags[0]);
        p.feed(&frags[1]);
        assert_eq!(p.feed("Parsed_equalizer_3: reconfigured\n"), TapFeed::Consumed);
        assert!(p.held.is_empty());
        // The rest of the line's fragments are now orphans (no prefix).
        for f in &frags[2..] {
            assert_eq!(p.feed(f), TapFeed::Ignored);
        }
        // The right bank's line for that frame waits alone, and the next
        // frame pairs normally.
        assert_eq!(p.feed(&line(RIGHT_PRINTER, 0, 0.0, &[-1.0, -2.0])), TapFeed::Consumed);
        let next = frame_lines(1, 0.0166667, &[-20.0, -30.0], &[-1.0, -2.0]);
        assert_eq!(feed_all(&mut p, &next).len(), 1);
    }

    #[test]
    fn reset_drops_a_held_fragment_and_pending_halves() {
        let mut p = TapLineParser::new(2);
        assert_eq!(
            p.feed(&line(LEFT_PRINTER, 0, 0.0, &[-1.0, -2.0])),
            TapFeed::Consumed
        );
        p.feed("Parsed_ashowinfo_9: n:1 ");
        assert!(!p.pending.is_empty() && !p.held.is_empty());
        p.reset();
        assert!(p.pending.is_empty() && p.held.is_empty());
        // The old left half is gone: a right half at the same pts waits alone.
        assert_eq!(
            p.feed(&line(RIGHT_PRINTER, 0, 0.0, &[-1.0, -2.0])),
            TapFeed::Consumed
        );
        assert_eq!(p.pending.len(), 1);
    }

    #[test]
    fn a_new_tap_line_mid_stitch_restarts_the_stitch() {
        let mut p = TapLineParser::new(2);
        let frags = fragments(LEFT_PRINTER, 0, 0.0, &[-20.0, -30.0]);
        p.feed(&frags[0]);
        // The rest of that line never arrives; a whole new line does.
        let whole = line(LEFT_PRINTER, 1, 0.0166667, &[-20.0, -30.0]);
        assert_eq!(p.feed(&whole), TapFeed::Consumed);
        assert!(p.held.is_empty());
        assert_eq!(p.pending.len(), 1);
        assert!((p.pending[0].pts - 0.0166667).abs() < 1e-5);
    }

    #[test]
    fn a_runaway_line_is_dropped_and_its_tail_forwarded() {
        let mut p = TapLineParser::new(1);
        p.feed("Parsed_ashowinfo_1: n:0 ");
        let mut results = Vec::new();
        for _ in 0..200 {
            results.push(p.feed(&"x".repeat(64)));
            assert!(p.held.len() <= MAX_TAP_LINE);
        }
        // Held while under the cap, then dropped; the remaining pieces are
        // ordinary log text again.
        assert_eq!(results[0], TapFeed::Consumed);
        assert_eq!(*results.last().unwrap(), TapFeed::Ignored);
        assert!(p.held.is_empty());
        assert_eq!(feed_all(&mut p, &frame_lines(1, 0.5, &[-20.0], &[-20.0])).len(), 1);
    }

    #[test]
    fn side_data_lines_are_consumed() {
        let mut p = TapLineParser::new(1);
        assert_eq!(
            p.feed("Parsed_ashowinfo_75:   side data - replaygain: track gain ...\n"),
            TapFeed::Consumed
        );
        assert!(p.pending.is_empty());
    }

    #[test]
    fn levels_below_the_mapper_floor_clamp() {
        let mut p = TapLineParser::new(2);
        let frames = feed_all(
            &mut p,
            &frame_lines(0, 0.0, &[TAP_DB_FLOOR, -200.0], &[-95.0, -100.0]),
        );
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].db, vec![DB_FLOOR, DB_FLOOR, DB_FLOOR, DB_FLOOR]);
    }

    #[test]
    fn seek_produces_a_new_timeline_immediately() {
        let mut p = TapLineParser::new(1);
        let mut lines = frame_lines(600, 10.0, &[-20.0], &[-20.0]);
        lines.extend(frame_lines(0, 60.0, &[-20.0], &[-20.0]));
        let frames = feed_all(&mut p, &lines);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[1].pts, 60.0);
    }

    #[test]
    fn a_forward_seek_drops_halves_left_waiting_from_the_old_timeline() {
        let mut p = TapLineParser::new(1);
        // Left printed frames at 10.0 and 10.0167 s; the right bank's
        // lines were lost; then the stream restarts at 60 s.
        p.feed(&line(LEFT_PRINTER, 600, 10.0, &[-20.0]));
        p.feed(&line(LEFT_PRINTER, 601, 10.0166667, &[-20.0]));
        assert_eq!(p.pending.len(), 2);
        assert_eq!(p.feed(&line(LEFT_PRINTER, 0, 60.0, &[-20.0])), TapFeed::Consumed);
        assert_eq!(p.pending.len(), 1);
        // A straggling right half for 10.0 is queued alone and must not
        // evict the fresh left half.
        assert_eq!(p.feed(&line(RIGHT_PRINTER, 600, 10.0, &[-20.0])), TapFeed::Consumed);
        assert_eq!(p.pending.len(), 2);
        assert!(matches!(
            p.feed(&line(RIGHT_PRINTER, 0, 60.0, &[-21.0])),
            TapFeed::Frame(f) if f.pts == 60.0
        ));
        // And the straggler went with the pairing.
        assert!(p.pending.is_empty());
    }

    #[test]
    fn a_backward_seek_still_pairs_the_new_timeline() {
        let mut p = TapLineParser::new(1);
        // Unpaired halves from 60 s linger; the stream restarts at 10 s.
        p.feed(&line(LEFT_PRINTER, 0, 60.0, &[-20.0]));
        p.feed(&line(LEFT_PRINTER, 1, 60.0166667, &[-20.0]));
        let frames = feed_all(&mut p, &frame_lines(0, 10.0, &[-1.0], &[-2.0]));
        assert_eq!(frames.len(), 1);
        assert!(close(&frames[0].db, &[-1.0, -2.0]));
        // The leftovers are newer than anything that follows, so they
        // only leave through the queue cap.
        assert_eq!(p.pending.len(), 2);
        for n in 1..=(MAX_PENDING_HALVES as u64) {
            p.feed(&line(LEFT_PRINTER, n, 10.0 + n as f64 / 60.0, &[-20.0]));
        }
        assert!(p.pending.iter().all(|h| h.pts < 60.0));
    }

    #[test]
    fn a_half_whose_partner_never_comes_ages_out() {
        let mut p = TapLineParser::new(1);
        for n in 0..(MAX_PENDING_HALVES as u64 + 10) {
            assert_eq!(
                p.feed(&line(LEFT_PRINTER, n, n as f64 / 60.0, &[-20.0])),
                TapFeed::Consumed
            );
            assert!(p.pending.len() <= MAX_PENDING_HALVES);
        }
        // The oldest were dropped; the newest still pairs.
        let newest = MAX_PENDING_HALVES as u64 + 9;
        assert!(matches!(
            p.feed(&line(RIGHT_PRINTER, newest, newest as f64 / 60.0, &[-30.0])),
            TapFeed::Frame(_)
        ));
        assert!(matches!(
            p.feed(&line(RIGHT_PRINTER, 0, 0.0, &[-30.0])),
            TapFeed::Consumed
        ));
    }

    #[test]
    fn a_duplicate_half_replaces_the_held_one() {
        let mut p = TapLineParser::new(1);
        p.feed(&line(LEFT_PRINTER, 0, 0.0, &[-20.0]));
        p.feed(&line(LEFT_PRINTER, 0, 0.0, &[-25.0]));
        assert_eq!(p.pending.len(), 1);
        let frames = feed_all(&mut p, &[line(RIGHT_PRINTER, 0, 0.0, &[-30.0])]);
        assert_eq!(frames.len(), 1);
        assert!(close(&frames[0].db, &[-25.0, -30.0]));
    }

    // --- level mapping ---

    #[test]
    fn silence_maps_to_zero() {
        let mut m = LevelMapper::new(60);
        let out = m.map(&[DB_FLOOR, f32::NEG_INFINITY, f32::NAN, -200.0]);
        assert_eq!(out, vec![0, 0, 0, 0]);
    }

    #[test]
    fn a_louder_frame_re_anchors_the_peak_instantly() {
        let mut m = LevelMapper::new(60);
        assert_eq!(m.peak_db(), PEAK_SEED_DB);
        let out = m.map(&[-5.0, -40.0]);
        assert_eq!(m.peak_db(), -5.0);
        // The peak itself sits PEAK_HEADROOM_DB below the ceiling, so it
        // is bright but not saturated; the quiet band is dim.
        assert!(out[0] > 240 && out[0] < 255, "{}", out[0]);
        assert!(out[1] > 0 && out[1] < out[0]);
    }

    #[test]
    fn at_or_above_ceiling_saturates() {
        let mut m = LevelMapper::new(60);
        m.map(&[-10.0]);
        // Anything at ceiling or above (impossible in practice: the peak
        // would move) maps to 255 via the clamp.
        let ceiling = m.peak_db() + PEAK_HEADROOM_DB;
        let floor = ceiling - DYNAMIC_RANGE_DB;
        assert_eq!(quantise_db_range(ceiling, floor, ceiling), 255);
        assert_eq!(quantise_db_range(floor, floor, ceiling), 0);
    }

    #[test]
    fn peak_decays_toward_the_frame_max_at_the_configured_rate() {
        let fps = 60;
        let mut m = LevelMapper::new(fps);
        m.map(&[-10.0]);
        // One second of a much quieter frame: peak should have fallen by
        // PEAK_DECAY_DB_PER_SEC, not snapped to the new frame.
        for _ in 0..fps {
            m.map(&[-50.0]);
        }
        let expected = -10.0 - PEAK_DECAY_DB_PER_SEC;
        assert!((m.peak_db() - expected).abs() < 1e-3, "{}", m.peak_db());
    }

    #[test]
    fn peak_never_decays_below_the_frame_max_or_the_floor() {
        let mut m = LevelMapper::new(60);
        m.map(&[-10.0]);
        for _ in 0..600 {
            m.map(&[-30.0]);
        }
        assert!((m.peak_db() - -30.0).abs() < 1e-3);
        for _ in 0..6000 {
            m.map(&[-100.0]);
        }
        assert!((m.peak_db() - PEAK_FLOOR_DB).abs() < 1e-3);
        // A band far below the floor stays dark even under maximal gain.
        let out = m.map(&[-100.0, PEAK_FLOOR_DB]);
        assert_eq!(out[0], 0);
        assert!(out[1] > 200);
    }

    #[test]
    fn quiet_intro_renders_at_a_moderate_height_from_the_seed() {
        let mut m = LevelMapper::new(60);
        let out = m.map(&[-50.0]);
        assert!(out[0] > 100 && out[0] < 200, "{}", out[0]);
    }

    #[test]
    fn reset_restores_the_seed() {
        let mut m = LevelMapper::new(60);
        m.map(&[-3.0]);
        m.reset();
        assert_eq!(m.peak_db(), PEAK_SEED_DB);
    }

    #[test]
    fn output_length_matches_input() {
        let mut m = LevelMapper::default();
        assert_eq!(m.map(&[]).len(), 0);
        assert_eq!(m.map(&[-20.0; 128]).len(), 128);
    }

    #[test]
    fn the_running_peak_is_shared_across_both_channels() {
        // A loud left channel sets the window for the right channel too,
        // so a panned instrument reads as louder on one side rather than
        // being scaled up to match. No tilt, so the peak is the raw level.
        let mut m = LevelMapper::with_tilt(60, 0.0);
        let out = m.map(&[-5.0, -5.0, -50.0, -50.0]);
        assert_eq!(m.peak_db(), -5.0);
        assert!(out[0] > 240);
        assert!(out[2] < out[0] / 2, "{:?}", out);
    }

    #[test]
    fn tilt_rotates_a_flat_spectrum_about_the_centre_band() {
        // Four bands per channel, evenly spaced in octaves, all at the same
        // level: the tilt lowers the low bands and raises the high ones by
        // the same amount either side of the centre, and both channels get
        // the same treatment. The running peak follows the tilted levels,
        // so the top band is the loudest.
        let mut m = LevelMapper::with_tilt(60, 6.0);
        let out = m.map(&[-20.0; 8]);
        assert!(out[0] < out[1] && out[1] < out[2] && out[2] < out[3], "{:?}", out);
        assert_eq!(&out[..4], &out[4..]);
        let span = (TAP_FREQ_HIGH_HZ / TAP_FREQ_LOW_HZ).log2();
        assert!((m.peak_db() - (-20.0 + 6.0 * span / 2.0)).abs() < 1e-3);
        let mut flat = LevelMapper::with_tilt(60, 0.0);
        let out = flat.map(&[-20.0; 8]);
        assert!(out.iter().all(|&v| v == out[0]), "{:?}", out);
    }

    #[test]
    fn tilt_leaves_silent_bands_silent() {
        // A band at the floor is silence, not a level to be boosted: with
        // one loud low band and the rest at the floor, the tilted top band
        // still renders as nothing.
        let mut m = LevelMapper::with_tilt(60, 6.0);
        let out = m.map(&[0.0, DB_FLOOR, DB_FLOOR, DB_FLOOR, 0.0, DB_FLOOR, DB_FLOOR, DB_FLOOR]);
        assert!(out[0] > 200, "{:?}", out);
        assert_eq!(out[3], 0);
    }

    #[test]
    fn changing_the_tilt_takes_effect_on_the_next_frame() {
        // A live tuning change: the same flat frame is tilted, then flat
        // again once the tilt is set to zero, with no new mapper needed.
        let mut m = LevelMapper::with_tilt(60, 6.0);
        let tilted = m.map(&[-20.0; 8]);
        assert!(tilted[0] < tilted[3], "{:?}", tilted);
        m.set_tilt(0.0);
        let flat = m.map(&[-20.0; 8]);
        assert!(flat.iter().all(|&v| v == flat[0]), "{:?}", flat);
    }

    #[test]
    fn spectrum_frame_serialises_camel_case() {
        let f = SpectrumFrame {
            pos: 1.5,
            bands: vec![1, 2],
        };
        let s = serde_json::to_string(&f).unwrap();
        assert_eq!(s, r#"{"pos":1.5,"bands":[1,2]}"#);
    }
}
