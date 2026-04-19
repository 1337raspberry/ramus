//! Plex API response models. Pure data structs for JSON deserialization.

use serde::Deserialize;

use crate::models::UltraBlurColors;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaItem {
    pub rating_key: String,
    pub title: String,
    pub title_sort: Option<String>,
    pub original_title: Option<String>,
    pub summary: Option<String>,
    pub parent_title: Option<String>,
    pub grandparent_title: Option<String>,
    pub parent_rating_key: Option<String>,
    pub grandparent_rating_key: Option<String>,
    pub index: Option<i32>,
    pub parent_index: Option<i32>,
    pub year: Option<i32>,
    /// Duration in milliseconds. Convert to seconds at the boundary.
    pub duration: Option<i64>,
    pub updated_at: Option<i64>,
    pub added_at: Option<i64>,
    pub last_viewed_at: Option<i64>,
    pub thumb: Option<String>,
    pub parent_thumb: Option<String>,
    pub grandparent_thumb: Option<String>,
    pub art: Option<String>,
    pub user_rating: Option<f64>,
    pub studio: Option<String>,
    #[serde(rename = "Media")]
    pub media: Option<Vec<MediaInfo>>,
    #[serde(rename = "Genre")]
    pub genre: Option<Vec<PlexTag>>,
    #[serde(rename = "Collection")]
    pub collection: Option<Vec<PlexTag>>,
    #[serde(rename = "UltraBlurColors")]
    pub ultra_blur_colors: Option<UltraBlurColors>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaInfo {
    pub audio_codec: Option<String>,
    pub bitrate: Option<i32>,
    #[serde(rename = "Part")]
    pub parts: Option<Vec<PartInfo>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PartInfo {
    pub key: Option<String>,
    /// File size in bytes. Always present for music tracks; used for
    /// accurate download size estimates instead of `bitrate × duration`.
    pub size: Option<i64>,
    #[serde(rename = "Stream")]
    pub streams: Option<Vec<StreamInfo>>,
}

#[derive(Debug, Clone)]
pub struct StreamInfo {
    pub id: Option<i32>,
    pub stream_type: Option<i32>,
    pub codec: Option<String>,
    pub bitrate: Option<i32>,
    pub key: Option<String>,
    pub format: Option<String>,
    pub timed: Option<bool>,
    pub provider: Option<String>,
}

// `timed` may be a JSON bool or int.
impl<'de> Deserialize<'de> for StreamInfo {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Raw {
            id: Option<i32>,
            stream_type: Option<i32>,
            codec: Option<String>,
            bitrate: Option<i32>,
            key: Option<String>,
            format: Option<String>,
            timed: Option<serde_json::Value>,
            provider: Option<String>,
        }
        let raw = Raw::deserialize(deserializer)?;
        let timed = raw.timed.and_then(|v| match v {
            serde_json::Value::Bool(b) => Some(b),
            serde_json::Value::Number(n) => n.as_i64().map(|i| i != 0),
            _ => None,
        });
        Ok(StreamInfo {
            id: raw.id,
            stream_type: raw.stream_type,
            codec: raw.codec,
            bitrate: raw.bitrate,
            key: raw.key,
            format: raw.format,
            timed,
            provider: raw.provider,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlexTag {
    pub tag: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LevelSample {
    pub v: f32,
}

// Plex wraps all responses in a `MediaContainer`.

#[derive(Debug, Deserialize)]
pub struct MediaContainerResponse {
    #[serde(rename = "MediaContainer")]
    pub media_container: MediaContainerBody,
}

#[derive(Debug, Deserialize)]
pub struct MediaContainerBody {
    #[serde(rename = "Metadata")]
    pub metadata: Option<Vec<MediaItem>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct LibrarySectionsResponse {
    #[serde(rename = "MediaContainer")]
    pub media_container: LibrarySectionsContainer,
}

#[derive(Debug, Deserialize)]
pub(super) struct LibrarySectionsContainer {
    #[serde(rename = "Directory")]
    pub directory: Option<Vec<LibrarySectionRaw>>,
}

/// Raw representation. Plex uses `type` as the JSON field name.
#[derive(Debug, Deserialize)]
pub(super) struct LibrarySectionRaw {
    pub key: String,
    pub title: String,
    #[serde(rename = "type")]
    pub section_type: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct LevelsResponse {
    #[serde(rename = "MediaContainer")]
    pub media_container: LevelsContainer,
}

#[derive(Debug, Deserialize)]
pub(super) struct LevelsContainer {
    #[serde(rename = "Level")]
    pub level: Option<Vec<LevelSample>>,
}

// plex.tv server discovery responses.

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PlexResourceResponse {
    pub name: String,
    pub provides: String,
    pub client_identifier: String,
    pub access_token: Option<String>,
    pub owned: Option<bool>,
    pub connections: Option<Vec<PlexResourceConnectionResponse>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PlexResourceConnectionResponse {
    pub uri: String,
    pub local: Option<bool>,
    pub relay: Option<bool>,
    #[serde(rename = "protocol")]
    pub protocol: Option<String>,
}
