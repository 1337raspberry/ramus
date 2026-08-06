//! Track URL resolution: local cache vs direct play vs transcode, the
//! resume mechanics for each, and the per-file `stream-record` option.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::models::Track;
use crate::playback::transcode;

use super::adaptive::effective_stream_policy;
use super::PlayerInner;

/// Allowed file extensions for cached audio files.
const ALLOWED_EXTENSIONS: &[&str] = &[
    "flac", "alac", "m4a", "mp3", "aac", "wav", "aiff", "ogg", "opus", "mp2", "bin",
];

/// Sanitize a string for use as a filename. Only `[a-zA-Z0-9_-]` are kept.
pub fn sanitize_filename(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect()
}

/// Whether a file extension is in the allowed set for audio caching.
pub fn is_allowed_extension(ext: &str) -> bool {
    ALLOWED_EXTENSIONS.contains(&ext.to_lowercase().as_str())
}

/// How a resume position should be realised after (re)loading a track.
/// The two seek mechanisms are kept distinct because a transcode stream
/// can't be byte-range sought.
pub(super) enum ResumePlan {
    /// No resume — play from the top.
    None,
    /// Seek via an mpv `start=<secs>` per-file option. Used for local
    /// files and direct-play URLs (HTTP-Range-seekable), so mpv's reported
    /// timeline stays absolute and no position remap is needed.
    MpvSeek(f64),
    /// The resume is baked into a transcode `offset=` URL. mpv sees a
    /// fresh stream starting at 0, so the player shifts reported positions
    /// by this many seconds (`position_base`) back onto the track timeline.
    StreamOffset(f64),
}

/// Resolve a track's playback URL for a normal (from-the-top) load.
pub(super) fn resolve_url(
    track: &Track,
    inner: &PlayerInner,
    persistent: &HashMap<String, PathBuf>,
) -> Option<String> {
    resolve_url_with_resume(track, inner, persistent, None).map(|(url, _)| url)
}

/// Resolve a track's playback URL, optionally resuming `resume` seconds in
/// (connection-failover reload / backward-seek of an offset stream).
/// Returns the URL and a [`ResumePlan`] telling the caller how to reach
/// the resume point. `resume` of `Some(v)` with `v <= 0` is treated as no
/// resume.
pub(super) fn resolve_url_with_resume(
    track: &Track,
    inner: &PlayerInner,
    persistent: &HashMap<String, PathBuf>,
    resume: Option<f64>,
) -> Option<(String, ResumePlan)> {
    let resume = resume.filter(|p| *p > 0.0);

    if let Some(path) = persistent.get(&track.rating_key) {
        let plan = resume.map_or(ResumePlan::None, ResumePlan::MpvSeek);
        return Some((format!("file://{}", path.display()), plan));
    }
    if let Some(path) = inner.cache.get(&track.rating_key) {
        let plan = resume.map_or(ResumePlan::None, ResumePlan::MpvSeek);
        return Some((format!("file://{}", path.display()), plan));
    }

    let server_url = inner.server_url.as_ref()?;
    let token = inner.token.as_ref()?;

    let (needs_transcode, bitrate) = effective_stream_policy(track, inner);
    if needs_transcode {
        // Single-file Opus instead of HLS. Plex enforces a per-client
        // concurrent-transcode cap of ~1, and a long-lived HLS session
        // (which lasts the full real-time duration of the song) gets
        // killed the moment the prefetch worker opens a second transcode
        // session for the next track. Single-file completes in seconds —
        // mpv slurps the whole 3-5 MB file into its forward buffer at
        // server-transcode speed, the session ends, and prefetch can run
        // without competition. Session shape mirrors the prefetch path:
        // `<client-id>-<rating-key>` — Plex tokenises on `-` for session
        // grouping, so extra suffixes risk it conflating two sessions
        // for the same client.
        let session = format!("{}-{}", inner.client_identifier, track.rating_key);
        // Resume is served by the server-side `offset=` (see
        // `build_transcode_download_url`) rather than an mpv seek: a
        // transcode stream is `Accept-Ranges: none`, so an mpv `start=`
        // would force a read-through from byte 0. Sub-second offsets are
        // dropped (meaningless, and Plex's offset is integer seconds).
        let offset = resume.map(|p| p as u64).filter(|s| *s > 0);
        let url = transcode::build_transcode_download_url(
            server_url,
            token,
            &track.rating_key,
            &inner.client_identifier,
            &session,
            bitrate,
            offset,
        )?;
        let plan = match offset {
            Some(secs) => ResumePlan::StreamOffset(secs as f64),
            None => ResumePlan::None,
        };
        Some((url.to_string(), plan))
    } else {
        let part_key = track.part_key.as_ref()?;
        let url = transcode::build_direct_play_url(server_url, part_key, token)?;
        let plan = resume.map_or(ResumePlan::None, ResumePlan::MpvSeek);
        Some((url.to_string(), plan))
    }
}

/// Build the per-file mpv `stream-record=<path>` option for a track being
/// loaded into the playlist, or `None` if recording isn't applicable.
///
/// Returns `None` for:
/// - Tracks without a configured `stream_record_dir` (feature off).
/// - URLs already pointing at a local file (no point recording a copy).
///
/// Forward slashes in the path are required because mpv's options parser
/// treats `\` as an escape character. The destination filename uses
/// `<rating_key>.<ext>` so the spectrum analyser's symphonia probe gets
/// a useful extension hint, and the file is unique per track.
pub(super) fn stream_record_option_for(
    track: &Track,
    url: &str,
    inner: &PlayerInner,
) -> Option<String> {
    let dir = inner.stream_record_dir.as_ref()?;
    if url.starts_with("file://") {
        return None;
    }
    let is_transcode = effective_stream_policy(track, inner).0;

    // Transcoded sources always come back as Ogg/Opus from Plex's
    // `/audio/:/transcode/universal/start` endpoint. For direct-play,
    // try the URL extension and fall back to the codec field — either
    // is good enough for symphonia's `Hint::with_extension`.
    let ext = if is_transcode {
        "ogg".to_string()
    } else {
        // Strip the query string before grabbing the extension —
        // rsplit was returning the query (everything after `?`) and
        // the codec field was always the de-facto fallback.
        url.split('?')
            .next()
            .and_then(|p| p.rsplit('.').next())
            .filter(|e| {
                !e.is_empty() && e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphanumeric())
            })
            .map(|s| s.to_ascii_lowercase())
            .or_else(|| track.codec.as_ref().map(|c| c.to_ascii_lowercase()))
            .unwrap_or_else(|| "audio".to_string())
    };

    let path = dir.join(format!("{}.{}", track.rating_key, ext));
    let path_str = path.to_string_lossy().replace('\\', "/");
    Some(format!("stream-record=\"{path_str}\""))
}
