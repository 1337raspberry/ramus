use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Plex media identifier (ratingKey string).
pub type PlexID = String;

/// Duration in seconds.
pub type Duration = f64;

// --- Range Operators (used by Search + Cache) ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RangeOp {
    Equal,
    GreaterThan,
    LessThan,
    GreaterOrEqual,
    LessOrEqual,
}

impl RangeOp {
    /// SQL comparison operator literal. Closed set — no injection risk.
    pub fn sql_literal(&self) -> &'static str {
        match self {
            Self::Equal => "=",
            Self::GreaterThan => ">",
            Self::LessThan => "<",
            Self::GreaterOrEqual => ">=",
            Self::LessOrEqual => "<=",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RangeField {
    Year,
    Rating,
}

// --- Playback enums ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlaybackStatus {
    Playing,
    Paused,
    Stopped,
}

impl PlaybackStatus {
    pub fn as_plex_str(&self) -> &'static str {
        match self {
            Self::Playing => "playing",
            Self::Paused => "paused",
            Self::Stopped => "stopped",
        }
    }
}

/// Whether/when lossless audio should be sent through Plex's universal
/// transcoder before reaching the player. The two `*Cellular` variants
/// only make sense on iOS/Android where the player has a real cellular
/// signal — on desktop they're equivalent to `Never` / `Remote` because
/// `is_cellular` is always false there. The UI hides them on desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PlaybackMode {
    #[default]
    Never,
    Cellular,
    Remote,
    RemoteOrCellular,
    Always,
}

/// Bitrate (kbps) the universal transcoder will target when transcoding
/// lossless content. Modeled as an enum rather than a raw integer so the
/// URL builder can't be passed an unsupported value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TranscodeBitrate {
    Kbps320,
    Kbps256,
    #[default]
    Kbps192,
    Kbps128,
}

impl TranscodeBitrate {
    pub fn as_kbps(self) -> u16 {
        match self {
            Self::Kbps320 => 320,
            Self::Kbps256 => 256,
            Self::Kbps192 => 192,
            Self::Kbps128 => 128,
        }
    }
}

/// Quality preference for user-initiated downloads. `Original` direct-plays
/// the source file (always lossless's full quality). The `Kbps*` variants
/// transcode lossless sources to Ogg/Opus at the chosen bitrate, matching
/// Plex's universal-transcode endpoint. Lossy sources always direct-play
/// regardless of this setting (transcoding lossy → lossy throws away
/// quality with no bandwidth payoff).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum DownloadQuality {
    #[default]
    Original,
    Kbps320,
    Kbps256,
    Kbps192,
    Kbps128,
}

impl DownloadQuality {
    /// Returns the transcode bitrate to use, or `None` for direct play.
    pub fn as_bitrate(self) -> Option<TranscodeBitrate> {
        match self {
            Self::Original => None,
            Self::Kbps320 => Some(TranscodeBitrate::Kbps320),
            Self::Kbps256 => Some(TranscodeBitrate::Kbps256),
            Self::Kbps192 => Some(TranscodeBitrate::Kbps192),
            Self::Kbps128 => Some(TranscodeBitrate::Kbps128),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchResultKind {
    Album,
    Track,
}

// --- UltraBlurColors ---

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UltraBlurColors {
    pub top_left: String,
    pub top_right: String,
    pub bottom_right: String,
    pub bottom_left: String,
}

/// 6-swatch palette from node-vibrant, cached in the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VibrantPalette {
    pub vibrant: Option<String>,
    pub dark_vibrant: Option<String>,
    pub light_vibrant: Option<String>,
    pub muted: Option<String>,
    pub dark_muted: Option<String>,
    pub light_muted: Option<String>,
}

/// UltraBlur colors + cached vibrant palette returned from the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumColorInfo {
    pub colors: Option<UltraBlurColors>,
    pub palette: Option<VibrantPalette>,
}

// --- AlbumFilterParams ---

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumFilterParams {
    #[serde(default)]
    pub unplayed: bool,
    /// Album-level favourite — `albums.rating >= 10.0`.
    #[serde(default)]
    pub favourite_albums: bool,
    /// Track-level favourite — at least one track on the album has
    /// `tracks.userRating >= 10.0`. Independent of `favourite_albums`; both
    /// can be active simultaneously and combine with AND.
    #[serde(default)]
    pub favourite_tracks: bool,
    pub year_min: Option<i32>,
    pub year_max: Option<i32>,
    /// OR semantics — match albums whose artist country contains any of these
    /// values (the column is comma-joined when an artist has multiple tags).
    #[serde(default)]
    pub countries: Vec<String>,
    /// AND semantics — album must be tagged with every selected genre, where
    /// each chip is expanded to its subtree via `GenreMapper::expand_genre`.
    #[serde(default)]
    pub genres: Vec<String>,
    pub collection: Option<String>,
}

impl AlbumFilterParams {
    pub fn is_empty(&self) -> bool {
        !self.unplayed
            && !self.favourite_albums
            && !self.favourite_tracks
            && self.year_min.is_none()
            && self.year_max.is_none()
            && self.countries.is_empty()
            && self.genres.is_empty()
            && self.collection.is_none()
    }
}

// --- Album ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    pub rating_key: PlexID,
    pub title: String,
    pub artist_name: String,
    pub year: Option<i32>,
    pub thumb: Option<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub collections: Vec<String>,
    #[serde(default)]
    pub is_favourite: bool,
    /// True iff at least one track on this album has `tracks.userRating >= 10.0`.
    /// Populated by `CacheDatabase::populate_album_favourite_tracks` alongside
    /// the genre/collection passes.
    #[serde(default)]
    pub has_favourite_track: bool,
    pub studio: Option<String>,
    pub added_at: Option<i64>,
    pub last_viewed_at: Option<i64>,
    pub view_count: Option<i64>,
    pub format: Option<String>,
    pub artist_country: Option<String>,
}

impl Album {
    pub fn id(&self) -> &str {
        &self.rating_key
    }
}

// --- Track ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub rating_key: PlexID,
    pub title: String,
    pub artist_name: String,
    pub track_artist: Option<String>,
    pub album_title: String,
    pub album_key: Option<PlexID>,
    pub index: Option<i32>,
    #[serde(default)]
    pub duration: Duration,
    pub codec: Option<String>,
    pub part_key: Option<String>,
    pub thumb: Option<String>,
    #[serde(default)]
    pub is_favourite: bool,
    pub bitrate: Option<i32>,
    pub disc_number: Option<i32>,
    /// File size in bytes. Populated at sync time from the Plex Part
    /// response. Missing on tracks that haven't been resynced since the
    /// column was added; downstream size estimates fall back to
    /// `bitrate × duration` in that case.
    pub file_size_bytes: Option<i64>,
    pub rating_count: Option<i64>,
}

impl Track {
    pub fn id(&self) -> &str {
        &self.rating_key
    }

    /// Track-level artist override if present, otherwise album artist.
    pub fn display_artist(&self) -> &str {
        match &self.track_artist {
            Some(ta) if !ta.is_empty() => ta,
            _ => &self.artist_name,
        }
    }

    /// True when this track has a different artist from the album artist.
    pub fn has_track_artist(&self) -> bool {
        match &self.track_artist {
            Some(ta) if !ta.is_empty() => {
                ta.to_lowercase() != self.artist_name.to_lowercase()
            }
            _ => false,
        }
    }

    /// Audio format display: "FLAC" for lossless, "MP3 320 kbps" for lossy.
    pub fn format_description(&self) -> Option<String> {
        let codec = self.codec.as_ref()?;
        if crate::util::is_lossless_codec(codec) {
            return Some(codec.to_uppercase());
        }
        if let Some(bitrate) = self.bitrate {
            Some(format!("{} {} kbps", codec.to_uppercase(), bitrate))
        } else {
            Some(codec.to_uppercase())
        }
    }
}

// --- PlexServerConnection ---

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlexServerConnection {
    pub uri: String,
    pub local: bool,
    pub relay: bool,
    pub protocol: String,
}

impl PlexServerConnection {
    /// Priority: lower = preferred.
    /// 0=local HTTPS, 1=remote HTTPS, 2=relay HTTPS,
    /// 3=local HTTP, 4=remote HTTP, 5=relay HTTP
    pub fn priority(&self) -> u8 {
        let https = self.protocol == "https";
        if self.local {
            return if https { 0 } else { 3 };
        }
        if !self.relay {
            return if https { 1 } else { 4 };
        }
        if https { 2 } else { 5 }
    }
}

// --- PlexServer ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlexServer {
    pub machine_identifier: String,
    pub name: String,
    /// Tokens stay server-side, never sent to the frontend.
    #[serde(skip_serializing, default)]
    pub access_token: String,
    pub owned: bool,
    pub connections: Vec<PlexServerConnection>,
}

impl PlexServer {
    pub fn id(&self) -> &str {
        &self.machine_identifier
    }

    /// Connections sorted by priority ascending (best first).
    pub fn sorted_connections(&self) -> Vec<&PlexServerConnection> {
        let mut conns: Vec<&PlexServerConnection> = self.connections.iter().collect();
        conns.sort_by_key(|c| c.priority());
        conns
    }
}

// --- ServerConfig ---

// `access_token` is excluded from serialization and stored in the encrypted token file.

#[derive(Debug, Clone, PartialEq)]
pub struct ServerConfig {
    pub machine_identifier: String,
    pub name: String,
    pub access_token: String,
    pub selected_library_key: Option<String>,
    pub owned: bool,
    pub connections: Vec<PlexServerConnection>,
    pub active_uri: Option<String>,
}

impl From<&ServerConfig> for PlexServer {
    fn from(config: &ServerConfig) -> Self {
        PlexServer {
            machine_identifier: config.machine_identifier.clone(),
            name: config.name.clone(),
            access_token: config.access_token.clone(),
            owned: config.owned,
            connections: config.connections.clone(),
        }
    }
}

impl Serialize for ServerConfig {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let field_count = 4
            + usize::from(self.selected_library_key.is_some())
            + usize::from(self.active_uri.is_some());
        let mut state = serializer.serialize_struct("ServerConfig", field_count)?;
        state.serialize_field("machineIdentifier", &self.machine_identifier)?;
        state.serialize_field("name", &self.name)?;
        state.serialize_field("owned", &self.owned)?;
        state.serialize_field("connections", &self.connections)?;
        if let Some(ref key) = self.selected_library_key {
            state.serialize_field("selectedLibraryKey", key)?;
        }
        if let Some(ref uri) = self.active_uri {
            state.serialize_field("activeUri", uri)?;
        }
        state.end()
    }
}

impl<'de> Deserialize<'de> for ServerConfig {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Raw {
            machine_identifier: String,
            name: String,
            #[serde(default)]
            access_token: String,
            selected_library_key: Option<String>,
            #[serde(default)]
            owned: bool,
            #[serde(default)]
            connections: Vec<PlexServerConnection>,
            active_uri: Option<String>,
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(ServerConfig {
            machine_identifier: raw.machine_identifier,
            name: raw.name,
            access_token: raw.access_token,
            selected_library_key: raw.selected_library_key,
            owned: raw.owned,
            connections: raw.connections,
            active_uri: raw.active_uri,
        })
    }
}

// --- PlayerState ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerState {
    pub status: PlaybackStatus,
    pub current_track: Option<Track>,
    #[serde(default)]
    pub queue: Vec<Track>,
    #[serde(default)]
    pub queue_index: usize,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self {
            status: PlaybackStatus::Stopped,
            current_track: None,
            queue: Vec::new(),
            queue_index: 0,
        }
    }
}

// --- PlaybackConfig ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackConfig {
    pub playback_mode: PlaybackMode,
    pub transcode_bitrate: TranscodeBitrate,
    pub lookahead_depth: u8,
    pub audio_cache_limit_bytes: i64,
}

impl PlaybackConfig {
    pub const DEFAULT_CACHE_LIMIT_BYTES: i64 = 2_147_483_648;

    pub fn new(
        playback_mode: PlaybackMode,
        transcode_bitrate: TranscodeBitrate,
        lookahead_depth: u8,
        audio_cache_limit_bytes: i64,
    ) -> Self {
        Self {
            playback_mode,
            transcode_bitrate,
            lookahead_depth: lookahead_depth.clamp(1, 20),
            audio_cache_limit_bytes,
        }
    }
}

impl Default for PlaybackConfig {
    fn default() -> Self {
        Self::new(
            PlaybackMode::default(),
            TranscodeBitrate::default(),
            6,
            Self::DEFAULT_CACHE_LIMIT_BYTES,
        )
    }
}

// --- SearchResult ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub id: String,
    pub kind: SearchResultKind,
    pub album_source_id: String,
    pub album_title: String,
    pub artist_name: String,
    pub year: Option<i32>,
    pub album_art_path: Option<String>,
    pub track_source_id: Option<String>,
    pub track_title: Option<String>,
    pub track_artist: Option<String>,
    pub is_favourite: bool,
    pub score: f64,
}

// --- ArtistInfo ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistInfo {
    pub id: i64,
    pub name: String,
    pub source_id: String,
    pub art_url: Option<String>,
    pub country: Option<String>,
}

// --- GenreSource ---

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GenreSource {
    #[default]
    Open,
    Custom,
}

// --- Settings ---

/// A named filter snapshot the user has saved for one-tap recall. Bookmarks
/// store the structured `AlbumFilterParams` rather than a query string so
/// they can be created from the chip-based filter UI and re-applied against
/// the live library every time (additions/removals reflect automatically).
///
/// The `filters` field persists verbatim; adding fields to `AlbumFilterParams`
/// implicitly extends the stored `Settings` schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bookmark {
    pub id: String,
    pub name: String,
    pub filters: AlbumFilterParams,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PopularityDisplay {
    Off,
    #[default]
    Hot,
    Chart,
}

/// Maximum number of bookmarks. Enforced in `update_settings`.
pub const MAX_BOOKMARKS: usize = 50;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub playback_mode: PlaybackMode,
    pub transcode_bitrate: TranscodeBitrate,
    pub lookahead_depth: u8,
    pub audio_cache_limit_bytes: i64,
    pub image_cache_limit_bytes: i64,
    pub sync_interval_hours: u32,
    pub genre_source: GenreSource,
    pub library_padding: i8,
    pub refuse_http: bool,
    pub last_sync_time_secs: i64,
    pub disable_spectrum: bool,
    pub flat_genres: bool,
    pub eq_enabled: bool,
    pub eq_bands: Vec<f32>,
    pub bookmarks: Vec<Bookmark>,
    /// User manual "Work Offline" toggle. When `true`, the app ignores
    /// live server reachability and shows only downloaded content.
    pub offline_mode: bool,
    pub popularity_display: PopularityDisplay,
    /// Merge Plex `Style` tags into the genre table during sync when true.
    pub include_plex_styles: bool,
    /// Show country-of-origin flags next to artist names. Defaults true.
    pub show_artist_flags: bool,
    /// Quality used for user-initiated downloads. `Original` direct-plays
    /// the source; `Kbps*` transcodes lossless sources to Ogg/Opus.
    pub download_quality: DownloadQuality,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            playback_mode: PlaybackMode::default(),
            transcode_bitrate: TranscodeBitrate::default(),
            lookahead_depth: 6,
            audio_cache_limit_bytes: PlaybackConfig::DEFAULT_CACHE_LIMIT_BYTES,
            image_cache_limit_bytes: 1_073_741_824,
            sync_interval_hours: 4,
            genre_source: GenreSource::default(),
            library_padding: 0,
            refuse_http: false,
            last_sync_time_secs: 0,
            disable_spectrum: true,
            flat_genres: false,
            eq_enabled: false,
            eq_bands: vec![0.0; 10],
            bookmarks: Vec::new(),
            offline_mode: false,
            popularity_display: PopularityDisplay::default(),
            include_plex_styles: true,
            show_artist_flags: true,
            download_quality: DownloadQuality::default(),
        }
    }
}

impl Bookmark {
    /// Validate a batch of bookmarks: cap at `MAX_BOOKMARKS`, names must be
    /// non-empty (after trim) and case-insensitive unique, ids must be
    /// non-empty and unique. Returns an explanation string suitable for
    /// surfacing on `update_settings` rejection.
    pub fn validate_batch(items: &[Bookmark]) -> Result<(), String> {
        if items.len() > MAX_BOOKMARKS {
            let count = items.len();
            return Err(format!(
                "too many bookmarks: {count} (max {MAX_BOOKMARKS})"
            ));
        }
        let mut seen_names = std::collections::HashSet::new();
        let mut seen_ids: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for b in items {
            let id = b.id.trim();
            if id.is_empty() {
                return Err("bookmark id cannot be empty".into());
            }
            if !seen_ids.insert(id) {
                return Err(format!("duplicate bookmark id: {id}"));
            }
            let name = b.name.trim();
            if name.is_empty() {
                return Err("bookmark name cannot be empty".into());
            }
            if !seen_names.insert(name.to_lowercase()) {
                return Err(format!("duplicate bookmark name: {name}"));
            }
        }
        Ok(())
    }
}

impl Settings {
    pub fn to_playback_config(&self) -> PlaybackConfig {
        PlaybackConfig::new(
            self.playback_mode,
            self.transcode_bitrate,
            self.lookahead_depth,
            self.audio_cache_limit_bytes,
        )
    }
}

// --- LibrarySection ---

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySection {
    pub key: String,
    pub title: String,
    pub section_type: String,
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;

    // -- helpers --

    fn make_track(artist_name: &str, track_artist: Option<&str>) -> Track {
        Track {
            rating_key: "1".into(),
            title: "Song".into(),
            artist_name: artist_name.into(),
            track_artist: track_artist.map(String::from),
            album_title: "Album".into(),
            album_key: None,
            index: None,
            duration: 0.0,
            codec: None,
            part_key: None,
            thumb: None,
            is_favourite: false,
            bitrate: None,
            disc_number: None,
            file_size_bytes: None,
            rating_count: None,
        }
    }

    fn make_track_with_codec(codec: Option<&str>, bitrate: Option<i32>) -> Track {
        Track {
            codec: codec.map(String::from),
            bitrate,
            ..make_track("Artist", None)
        }
    }

    // -- has_track_artist --

    #[test]
    fn test_nil_track_artist_returns_false() {
        assert!(!make_track("Album Artist", None).has_track_artist());
    }

    #[test]
    fn test_empty_track_artist_returns_false() {
        assert!(!make_track("Album Artist", Some("")).has_track_artist());
    }

    #[test]
    fn test_same_as_album_artist_returns_false() {
        assert!(!make_track("Radiohead", Some("Radiohead")).has_track_artist());
    }

    #[test]
    fn test_same_as_album_artist_case_insensitive_returns_false() {
        assert!(!make_track("Radiohead", Some("radiohead")).has_track_artist());
    }

    #[test]
    fn test_different_track_artist_returns_true() {
        assert!(make_track("Various Artists", Some("Radiohead")).has_track_artist());
    }

    // -- display_artist --

    #[test]
    fn test_display_artist_nil_track_artist() {
        assert_eq!(make_track("Radiohead", None).display_artist(), "Radiohead");
    }

    #[test]
    fn test_display_artist_empty_track_artist() {
        assert_eq!(make_track("Radiohead", Some("")).display_artist(), "Radiohead");
    }

    #[test]
    fn test_display_artist_with_override() {
        assert_eq!(
            make_track("Various Artists", Some("Thom Yorke")).display_artist(),
            "Thom Yorke"
        );
    }

    // -- format_description --

    #[test]
    fn test_format_description_none_codec() {
        assert_eq!(make_track_with_codec(None, None).format_description(), None);
    }

    #[test]
    fn test_format_description_flac() {
        assert_eq!(
            make_track_with_codec(Some("flac"), None).format_description(),
            Some("FLAC".into())
        );
    }

    #[test]
    fn test_format_description_alac() {
        assert_eq!(
            make_track_with_codec(Some("alac"), None).format_description(),
            Some("ALAC".into())
        );
    }

    #[test]
    fn test_format_description_mp3_with_bitrate() {
        assert_eq!(
            make_track_with_codec(Some("mp3"), Some(320)).format_description(),
            Some("MP3 320 kbps".into())
        );
    }

    #[test]
    fn test_format_description_aac_no_bitrate() {
        assert_eq!(
            make_track_with_codec(Some("aac"), None).format_description(),
            Some("AAC".into())
        );
    }

    #[test]
    fn test_format_description_wav() {
        assert_eq!(
            make_track_with_codec(Some("wav"), None).format_description(),
            Some("WAV".into())
        );
    }

    // -- PlexServerConnection::priority --

    #[test]
    fn test_priority_ordering() {
        let cases = vec![
            (true, false, "https", 0u8),
            (false, false, "https", 1),
            (false, true, "https", 2),
            (true, false, "http", 3),
            (false, false, "http", 4),
            (false, true, "http", 5),
        ];
        for (local, relay, protocol, expected) in cases {
            let conn = PlexServerConnection {
                uri: "http://test".into(),
                local,
                relay,
                protocol: protocol.into(),
            };
            assert_eq!(conn.priority(), expected, "local={local}, relay={relay}, protocol={protocol}");
        }
    }

    #[test]
    fn test_sorted_connections() {
        let server = PlexServer {
            machine_identifier: "abc".into(),
            name: "Test".into(),
            access_token: "tok".into(),
            owned: true,
            connections: vec![
                PlexServerConnection {
                    uri: "http://relay".into(),
                    local: false,
                    relay: true,
                    protocol: "http".into(),
                },
                PlexServerConnection {
                    uri: "https://local".into(),
                    local: true,
                    relay: false,
                    protocol: "https".into(),
                },
                PlexServerConnection {
                    uri: "https://remote".into(),
                    local: false,
                    relay: false,
                    protocol: "https".into(),
                },
            ],
        };
        let sorted = server.sorted_connections();
        assert_eq!(sorted[0].uri, "https://local");
        assert_eq!(sorted[1].uri, "https://remote");
        assert_eq!(sorted[2].uri, "http://relay");
    }

    // -- PlaybackConfig clamping --

    #[test]
    fn test_playback_config_clamps_low() {
        let cfg = PlaybackConfig::new(
            PlaybackMode::Never,
            TranscodeBitrate::Kbps128,
            0,
            PlaybackConfig::DEFAULT_CACHE_LIMIT_BYTES,
        );
        assert_eq!(cfg.lookahead_depth, 1);
    }

    #[test]
    fn test_playback_config_clamps_high() {
        let cfg = PlaybackConfig::new(
            PlaybackMode::Never,
            TranscodeBitrate::Kbps128,
            50,
            PlaybackConfig::DEFAULT_CACHE_LIMIT_BYTES,
        );
        assert_eq!(cfg.lookahead_depth, 20);
    }

    #[test]
    fn test_playback_config_default() {
        let cfg = PlaybackConfig::default();
        assert_eq!(cfg.playback_mode, PlaybackMode::Never);
        assert_eq!(cfg.transcode_bitrate, TranscodeBitrate::Kbps192);
        assert_eq!(cfg.lookahead_depth, 6);
        assert_eq!(cfg.audio_cache_limit_bytes, 2_147_483_648);
    }

    // -- ServerConfig serialization --

    #[test]
    fn test_server_config_serialization_excludes_access_token() {
        let config = ServerConfig {
            machine_identifier: "m1".into(),
            name: "My Server".into(),
            access_token: "secret-token".into(),
            selected_library_key: Some("lib1".into()),
            owned: true,
            connections: vec![],
            active_uri: None,
        };
        let json = serde_json::to_string(&config).unwrap();
        assert!(!json.contains("secret-token"));
        assert!(json.contains("machineIdentifier"));
        assert!(json.contains("My Server"));
        assert!(json.contains("selectedLibraryKey"));
    }

    #[test]
    fn test_server_config_deserialization_without_access_token() {
        let json = r#"{"machineIdentifier":"m1","name":"Server"}"#;
        let config: ServerConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.machine_identifier, "m1");
        assert_eq!(config.name, "Server");
        assert_eq!(config.access_token, "");
        assert_eq!(config.selected_library_key, None);
        assert!(!config.owned);
        assert!(config.connections.is_empty());
        assert_eq!(config.active_uri, None);
    }

    #[test]
    fn test_server_config_round_trip() {
        let original = ServerConfig {
            machine_identifier: "m1".into(),
            name: "Server".into(),
            access_token: "secret".into(),
            selected_library_key: Some("lib".into()),
            owned: true,
            connections: vec![PlexServerConnection {
                uri: "https://192.168.1.1:32400".into(),
                local: true,
                relay: false,
                protocol: "https".into(),
            }],
            active_uri: Some("https://192.168.1.1:32400".into()),
        };
        let json = serde_json::to_string(&original).unwrap();
        let restored: ServerConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.machine_identifier, "m1");
        assert_eq!(restored.name, "Server");
        assert_eq!(restored.access_token, "");
        assert_eq!(restored.selected_library_key, Some("lib".into()));
        assert!(restored.owned);
        assert_eq!(restored.connections.len(), 1);
        assert_eq!(restored.connections[0].uri, "https://192.168.1.1:32400");
    }

    // -- PlayerState default --

    #[test]
    fn test_default_player_state() {
        let state = PlayerState::default();
        assert_eq!(state.status, PlaybackStatus::Stopped);
        assert!(state.current_track.is_none());
        assert!(state.queue.is_empty());
        assert_eq!(state.queue_index, 0);
    }

    // -- Track identity --

    #[test]
    fn test_track_id_is_rating_key() {
        let track = make_track("A", None);
        assert_eq!(track.id(), "1");
    }

    // -- RangeOp sql_literal --

    #[test]
    fn test_range_op_sql_literals() {
        assert_eq!(RangeOp::Equal.sql_literal(), "=");
        assert_eq!(RangeOp::GreaterThan.sql_literal(), ">");
        assert_eq!(RangeOp::LessThan.sql_literal(), "<");
        assert_eq!(RangeOp::GreaterOrEqual.sql_literal(), ">=");
        assert_eq!(RangeOp::LessOrEqual.sql_literal(), "<=");
    }

    fn bm(id: &str, name: &str) -> Bookmark {
        Bookmark {
            id: id.into(),
            name: name.into(),
            filters: AlbumFilterParams::default(),
        }
    }

    #[test]
    fn test_bookmark_validate_ok() {
        let batch = vec![bm("a", "Metal"), bm("b", "Chill")];
        assert!(Bookmark::validate_batch(&batch).is_ok());
    }

    #[test]
    fn test_bookmark_rejects_duplicate_names_case_insensitive() {
        let batch = vec![bm("a", "Metal"), bm("b", "metal")];
        assert!(Bookmark::validate_batch(&batch).is_err());
    }

    #[test]
    fn test_bookmark_rejects_blank_name() {
        assert!(Bookmark::validate_batch(&[bm("a", "")]).is_err());
        assert!(Bookmark::validate_batch(&[bm("a", "   ")]).is_err());
    }

    #[test]
    fn test_bookmark_rejects_over_cap() {
        let batch: Vec<Bookmark> = (0..(MAX_BOOKMARKS + 1))
            .map(|i| bm(&i.to_string(), &format!("name{i}")))
            .collect();
        assert!(Bookmark::validate_batch(&batch).is_err());
    }

    #[test]
    fn test_bookmark_rejects_duplicate_ids() {
        let batch = vec![bm("a", "Metal"), bm("a", "Chill")];
        assert!(Bookmark::validate_batch(&batch).is_err());
    }

    #[test]
    fn test_bookmark_rejects_empty_id() {
        assert!(Bookmark::validate_batch(&[bm("", "Metal")]).is_err());
        assert!(Bookmark::validate_batch(&[bm("   ", "Metal")]).is_err());
    }

    #[test]
    fn test_settings_ignores_legacy_saved_searches_field() {
        // Pre-bookmarks shape persisted a `savedSearches` array. After the
        // rename to `bookmarks` the legacy key is silently dropped (serde
        // ignores unknown fields by default) and the new field falls back
        // to an empty Vec via `#[serde(default)]` on `Settings`.
        let legacy = r#"{"savedSearches":[{"id":"a","name":"Metal","query":"/metal"}]}"#;
        let settings: Settings = serde_json::from_str(legacy).unwrap();
        assert!(settings.bookmarks.is_empty());
    }

    #[test]
    fn test_bookmark_round_trip_serde() {
        let original = Bookmark {
            id: "b1".into(),
            name: "algerian sleepytime favourites".into(),
            filters: AlbumFilterParams {
                favourite_albums: true,
                countries: vec!["Algeria".into()],
                ..Default::default()
            },
        };
        let json = serde_json::to_string(&original).unwrap();
        let parsed: Bookmark = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, original);
    }
}
