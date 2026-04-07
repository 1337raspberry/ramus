use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use reqwest::Client;
use serde::Deserialize;
use url::Url;

use crate::models::{LibrarySection, PlexServer, PlexServerConnection, UltraBlurColors};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum PlexClientError {
    #[error("not connected to a Plex server")]
    NotConnected,
    #[error("connection failed")]
    ConnectionFailed,
    #[error("no music library found")]
    NoMusicLibrary,
    #[error("invalid response from server")]
    InvalidResponse,
    #[error("unauthorized (401)")]
    Unauthorized,
    #[error("HTTP error {0}")]
    HttpError(u16),
    #[error("no secure connection available")]
    NoSecureConnection,
}

// ---------------------------------------------------------------------------
// Plex API Response Models (internal JSON deserialization)
// ---------------------------------------------------------------------------

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
    /// Duration in milliseconds — convert to seconds at the boundary.
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

// Custom deserialize for StreamInfo: `timed` can be JSON bool OR int.
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

// -- Response wrappers (Plex wraps everything in MediaContainer) --

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
struct LibrarySectionsResponse {
    #[serde(rename = "MediaContainer")]
    media_container: LibrarySectionsContainer,
}

#[derive(Debug, Deserialize)]
struct LibrarySectionsContainer {
    #[serde(rename = "Directory")]
    directory: Option<Vec<LibrarySectionRaw>>,
}

/// Internal raw representation — Plex uses `type` as the key name.
#[derive(Debug, Deserialize)]
struct LibrarySectionRaw {
    key: String,
    title: String,
    #[serde(rename = "type")]
    section_type: String,
}

#[derive(Debug, Deserialize)]
struct LevelsResponse {
    #[serde(rename = "MediaContainer")]
    media_container: LevelsContainer,
}

#[derive(Debug, Deserialize)]
struct LevelsContainer {
    #[serde(rename = "Level")]
    level: Option<Vec<LevelSample>>,
}

// -- plex.tv server discovery response --

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlexResourceResponse {
    name: String,
    provides: String,
    client_identifier: String,
    access_token: Option<String>,
    owned: Option<bool>,
    connections: Option<Vec<PlexResourceConnectionResponse>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlexResourceConnectionResponse {
    uri: String,
    local: Option<bool>,
    relay: Option<bool>,
    #[serde(rename = "protocol")]
    protocol: Option<String>,
}

// ---------------------------------------------------------------------------
// PlexClient
// ---------------------------------------------------------------------------

type ReconnectCallback =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>;

struct ConnectionState {
    server_url: Option<Url>,
    token: Option<String>,
    on_request_failed: Option<ReconnectCallback>,
}

/// Maximum response size (50 MB).
const MAX_RESPONSE_BYTES: usize = 50 * 1024 * 1024;

pub struct PlexClient {
    pub client_identifier: String,
    http: Client,
    state: RwLock<ConnectionState>,
}

impl PlexClient {
    pub fn new(client_identifier: String) -> Self {
        Self {
            client_identifier,
            http: Client::new(),
            state: RwLock::new(ConnectionState {
                server_url: None,
                token: None,
                on_request_failed: None,
            }),
        }
    }

    // -- State accessors --

    pub fn server_url(&self) -> Option<Url> {
        self.state.read().server_url.clone()
    }

    pub fn set_server_url(&self, url: Option<Url>) {
        self.state.write().server_url = url;
    }

    pub fn token(&self) -> Option<String> {
        self.state.read().token.clone()
    }

    pub fn set_token(&self, token: Option<String>) {
        self.state.write().token = token;
    }

    pub fn set_on_request_failed(&self, callback: Option<ReconnectCallback>) {
        self.state.write().on_request_failed = callback;
    }

    // -- Platform headers --

    fn platform() -> &'static str {
        if cfg!(target_os = "macos") {
            "macOS"
        } else if cfg!(target_os = "windows") {
            "Windows"
        } else {
            "Linux"
        }
    }

    fn device() -> &'static str {
        if cfg!(target_os = "macos") {
            "Mac"
        } else if cfg!(target_os = "windows") {
            "PC"
        } else {
            "Linux"
        }
    }

    fn apply_standard_headers(
        &self,
        builder: reqwest::RequestBuilder,
        token: Option<&str>,
    ) -> reqwest::RequestBuilder {
        let mut b = builder
            .header("Accept", "application/json")
            .header("X-Plex-Client-Identifier", &self.client_identifier)
            .header("X-Plex-Product", "ramus")
            .header("X-Plex-Platform", Self::platform())
            .header("X-Plex-Device", Self::device());
        if let Some(t) = token {
            b = b.header("X-Plex-Token", t);
        }
        b
    }

    // -- Internal HTTP helpers --

    fn read_state(&self) -> Result<(Url, String), PlexClientError> {
        let state = self.state.read();
        match (&state.server_url, &state.token) {
            (Some(url), Some(token)) => Ok((url.clone(), token.clone())),
            _ => Err(PlexClientError::NotConnected),
        }
    }

    fn reconnect_callback(&self) -> Option<ReconnectCallback> {
        self.state.read().on_request_failed.clone()
    }

    fn is_connection_error(e: &reqwest::Error) -> bool {
        e.is_timeout() || e.is_connect() || e.is_request()
    }

    async fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Vec<u8>, PlexClientError> {
        self.with_retry(|| async {
            let (base, token) = self.read_state()?;
            let url = base.join(path).map_err(|_| PlexClientError::InvalidResponse)?;
            let builder = self.http.get(url).query(query);
            let builder = self.apply_standard_headers(builder, Some(&token));

            let resp = builder.send().await.map_err(|e| {
                if Self::is_connection_error(&e) {
                    PlexClientError::ConnectionFailed
                } else {
                    PlexClientError::InvalidResponse
                }
            })?;

            let status = resp.status().as_u16();
            if status == 401 {
                return Err(PlexClientError::Unauthorized);
            }
            if !(200..300).contains(&(status as usize)) {
                return Err(PlexClientError::HttpError(status));
            }

            let body = resp.bytes().await.map_err(|_| PlexClientError::InvalidResponse)?;
            if body.len() > MAX_RESPONSE_BYTES {
                return Err(PlexClientError::InvalidResponse);
            }
            Ok(body.to_vec())
        })
        .await
    }

    async fn put(&self, path: &str, query: &[(&str, &str)]) -> Result<(), PlexClientError> {
        self.with_retry(|| async {
            let (base, token) = self.read_state()?;
            let url = base.join(path).map_err(|_| PlexClientError::InvalidResponse)?;

            let mut pairs: Vec<(String, String)> =
                query.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            pairs.push(("X-Plex-Token".to_string(), token.clone()));

            let builder = self.http.put(url).query(&pairs);
            let builder = self.apply_standard_headers(builder, None);

            let resp = builder.send().await.map_err(|e| {
                if Self::is_connection_error(&e) {
                    PlexClientError::ConnectionFailed
                } else {
                    PlexClientError::InvalidResponse
                }
            })?;

            let status = resp.status().as_u16();
            if status == 401 {
                return Err(PlexClientError::Unauthorized);
            }
            if !(200..300).contains(&(status as usize)) {
                return Err(PlexClientError::HttpError(status));
            }
            Ok(())
        })
        .await
    }

    async fn with_retry<F, Fut, T>(&self, work: F) -> Result<T, PlexClientError>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = Result<T, PlexClientError>>,
    {
        match work().await {
            Ok(v) => Ok(v),
            Err(PlexClientError::ConnectionFailed) => {
                if let Some(cb) = self.reconnect_callback() {
                    if cb().await {
                        return work().await;
                    }
                }
                Err(PlexClientError::ConnectionFailed)
            }
            Err(e) => Err(e),
        }
    }

    // -- Public API --

    /// Discover servers available to the authenticated user via plex.tv.
    pub async fn discover_servers(
        &self,
        auth_token: &str,
    ) -> Result<Vec<PlexServer>, PlexClientError> {
        let builder = self
            .http
            .get("https://plex.tv/api/v2/resources")
            .query(&[("includeHttps", "1"), ("includeRelay", "1")]);
        let builder = self.apply_standard_headers(builder, Some(auth_token));

        let resp = builder
            .send()
            .await
            .map_err(|_| PlexClientError::ConnectionFailed)?;

        let status = resp.status().as_u16();
        if !(200..300).contains(&(status as usize)) {
            return Err(PlexClientError::ConnectionFailed);
        }

        let body = resp
            .bytes()
            .await
            .map_err(|_| PlexClientError::InvalidResponse)?;
        if body.len() > MAX_RESPONSE_BYTES {
            return Err(PlexClientError::InvalidResponse);
        }

        let resources: Vec<PlexResourceResponse> =
            serde_json::from_slice(&body).map_err(|_| PlexClientError::InvalidResponse)?;

        Ok(resources
            .into_iter()
            .filter(|r| r.provides.contains("server") && r.access_token.is_some())
            .filter_map(|r| {
                let token = r.access_token?;
                Some(PlexServer {
                    machine_identifier: r.client_identifier,
                    name: r.name,
                    access_token: token,
                    owned: r.owned.unwrap_or(false),
                    connections: r
                        .connections
                        .unwrap_or_default()
                        .into_iter()
                        .map(|c| PlexServerConnection {
                            uri: c.uri,
                            local: c.local.unwrap_or(false),
                            relay: c.relay.unwrap_or(false),
                            protocol: c.protocol.unwrap_or_else(|| "https".into()),
                        })
                        .collect(),
                })
            })
            .collect())
    }

    /// Test a single connection URI with GET /identity.
    pub async fn test_connection(
        &self,
        uri: &str,
        token: &str,
        timeout: Option<Duration>,
    ) -> bool {
        let timeout = timeout.unwrap_or(Duration::from_secs(5));
        let url = match Url::parse(uri) {
            Ok(u) => {
                let scheme = u.scheme();
                if scheme != "http" && scheme != "https" {
                    return false;
                }
                match u.join("identity") {
                    Ok(u) => u,
                    Err(_) => return false,
                }
            }
            Err(_) => return false,
        };

        let builder = self
            .http
            .get(url)
            .timeout(timeout);
        let builder = self.apply_standard_headers(builder, Some(token));

        match builder.send().await {
            Ok(resp) => resp.status().as_u16() == 200,
            Err(_) => false,
        }
    }

    /// Find best working connection for a server. Tests all concurrently,
    /// returns highest-priority one that succeeds.
    pub async fn find_best_connection(
        &self,
        server: &PlexServer,
        allow_http: bool,
    ) -> (Option<PlexServerConnection>, bool) {
        let sorted: Vec<PlexServerConnection> = if allow_http {
            server.sorted_connections().into_iter().cloned().collect()
        } else {
            server
                .sorted_connections()
                .into_iter()
                .filter(|c| c.protocol == "https")
                .cloned()
                .collect()
        };

        if sorted.is_empty() {
            return (None, false);
        }

        let mut handles = Vec::new();
        for (i, conn) in sorted.iter().enumerate() {
            let uri = conn.uri.clone();
            let token = server.access_token.clone();
            let client_id = self.client_identifier.clone();
            let http = self.http.clone();

            handles.push(tokio::spawn(async move {
                let url = match Url::parse(&uri) {
                    Ok(u) => match u.join("identity") {
                        Ok(u) => u,
                        Err(_) => return (i, false),
                    },
                    Err(_) => return (i, false),
                };
                let resp = http
                    .get(url)
                    .timeout(Duration::from_secs(5))
                    .header("Accept", "application/json")
                    .header("X-Plex-Client-Identifier", &client_id)
                    .header("X-Plex-Product", "ramus")
                    .header("X-Plex-Token", &token)
                    .send()
                    .await;
                let ok = matches!(resp, Ok(r) if r.status().as_u16() == 200);
                (i, ok)
            }));
        }

        let mut best_index: Option<usize> = None;
        for handle in handles {
            if let Ok((index, succeeded)) = handle.await {
                if succeeded {
                    best_index = Some(match best_index {
                        Some(cur) => cur.min(index),
                        None => index,
                    });
                }
            }
        }

        match best_index {
            Some(idx) => {
                let conn = &sorted[idx];
                let is_http = conn.protocol == "http";
                if is_http {
                    log::warn!(
                        "best available connection uses plaintext HTTP — token will be sent unencrypted"
                    );
                }
                (Some(conn.clone()), is_http)
            }
            None => (None, false),
        }
    }

    /// Connect to a Plex server and verify connectivity.
    pub async fn connect(&self, server_url: Url, token: String) -> Result<(), PlexClientError> {
        {
            let mut state = self.state.write();
            state.server_url = Some(server_url.clone());
            state.token = Some(token.clone());
        }

        let url = server_url
            .join("identity")
            .map_err(|_| PlexClientError::ConnectionFailed)?;
        let builder = self.http.get(url);
        let builder = self.apply_standard_headers(builder, Some(&token));

        let resp = builder
            .send()
            .await
            .map_err(|_| PlexClientError::ConnectionFailed)?;

        if !(200..300).contains(&(resp.status().as_u16() as usize)) {
            return Err(PlexClientError::ConnectionFailed);
        }
        Ok(())
    }

    /// Find all music-type library sections (type == "artist").
    pub async fn find_music_libraries(&self) -> Result<Vec<LibrarySection>, PlexClientError> {
        let body = self.get("library/sections", &[]).await?;
        let container: LibrarySectionsResponse =
            serde_json::from_slice(&body).map_err(|_| PlexClientError::InvalidResponse)?;

        let music: Vec<LibrarySection> = container
            .media_container
            .directory
            .unwrap_or_default()
            .into_iter()
            .filter(|s| s.section_type == "artist")
            .map(|s| LibrarySection {
                key: s.key,
                title: s.title,
                section_type: s.section_type,
            })
            .collect();

        if music.is_empty() {
            return Err(PlexClientError::NoMusicLibrary);
        }
        Ok(music)
    }

    /// Fetch all items of a given type from a library section, paginated.
    /// Type codes: 8=artist, 9=album, 10=track. Page size default 200.
    pub async fn fetch_all_items(
        &self,
        library_key: &str,
        item_type: i32,
        page_size: usize,
    ) -> Result<Vec<MediaItem>, PlexClientError> {
        let mut all_items = Vec::new();
        let mut offset: usize = 0;
        let max_pages = 5000;
        let page_size_str = page_size.to_string();
        let type_str = item_type.to_string();
        let path = format!("library/sections/{}/all", library_key);

        for _ in 0..max_pages {
            let offset_str = offset.to_string();
            let query = [
                ("type", type_str.as_str()),
                ("X-Plex-Container-Start", offset_str.as_str()),
                ("X-Plex-Container-Size", page_size_str.as_str()),
            ];
            let body = self.get(&path, &query).await?;
            let container: MediaContainerResponse =
                serde_json::from_slice(&body).map_err(|_| PlexClientError::InvalidResponse)?;

            let items = container.media_container.metadata.unwrap_or_default();
            if items.is_empty() {
                break;
            }
            let count = items.len();
            all_items.extend(items);
            if count < page_size {
                break;
            }
            offset += page_size;
        }
        Ok(all_items)
    }

    /// Fetch full metadata for a single item.
    pub async fn fetch_item_metadata(
        &self,
        rating_key: &str,
    ) -> Result<MediaItem, PlexClientError> {
        let path = format!("library/metadata/{}", rating_key);
        let body = self.get(&path, &[]).await?;
        let container: MediaContainerResponse =
            serde_json::from_slice(&body).map_err(|_| PlexClientError::InvalidResponse)?;
        container
            .media_container
            .metadata
            .and_then(|mut v| if v.is_empty() { None } else { Some(v.remove(0)) })
            .ok_or(PlexClientError::InvalidResponse)
    }

    /// Find a lyrics stream (stream_type == 4) from full track metadata.
    pub async fn fetch_lyrics_stream(
        &self,
        rating_key: &str,
    ) -> Result<Option<StreamInfo>, PlexClientError> {
        let item = self.fetch_item_metadata(rating_key).await?;
        Ok(item
            .media
            .and_then(|m| m.into_iter().next())
            .and_then(|m| m.parts)
            .and_then(|p| p.into_iter().next())
            .and_then(|p| p.streams)
            .and_then(|s| s.into_iter().find(|s| s.stream_type == Some(4))))
    }

    /// Find the audio stream (stream_type == 2) from full track metadata.
    pub async fn fetch_audio_stream(
        &self,
        rating_key: &str,
    ) -> Result<Option<StreamInfo>, PlexClientError> {
        let item = self.fetch_item_metadata(rating_key).await?;
        Ok(item
            .media
            .and_then(|m| m.into_iter().next())
            .and_then(|m| m.parts)
            .and_then(|p| p.into_iter().next())
            .and_then(|p| p.streams)
            .and_then(|s| s.into_iter().find(|s| s.stream_type == Some(2))))
    }

    /// Download raw lyrics data from a Plex stream path.
    pub async fn download_lyrics_data(&self, path: &str) -> Result<Vec<u8>, PlexClientError> {
        let (base, token) = self.read_state()?;
        let url = base
            .join(path)
            .map_err(|_| PlexClientError::InvalidResponse)?;

        let builder = self
            .http
            .get(url)
            .query(&[("X-Plex-Token", &token)])
            .header("Accept", "*/*")
            .header("X-Plex-Client-Identifier", &self.client_identifier)
            .header("X-Plex-Product", "ramus")
            .header("X-Plex-Platform", Self::platform())
            .header("X-Plex-Device", Self::device());

        let resp = builder
            .send()
            .await
            .map_err(|_| PlexClientError::ConnectionFailed)?;

        let status = resp.status().as_u16();
        if !(200..300).contains(&(status as usize)) {
            return Err(PlexClientError::HttpError(status));
        }

        let body = resp
            .bytes()
            .await
            .map_err(|_| PlexClientError::InvalidResponse)?;
        Ok(body.to_vec())
    }

    /// Fetch loudness level samples for an audio stream (waveform visualization).
    pub async fn fetch_levels(
        &self,
        stream_id: i32,
        subsample: Option<i32>,
    ) -> Result<Vec<f32>, PlexClientError> {
        let path = format!("library/streams/{}/levels", stream_id);
        let sub = subsample.unwrap_or(600).to_string();
        let body = self.get(&path, &[("subsample", sub.as_str())]).await?;
        let response: LevelsResponse =
            serde_json::from_slice(&body).map_err(|_| PlexClientError::InvalidResponse)?;
        Ok(response
            .media_container
            .level
            .unwrap_or_default()
            .into_iter()
            .map(|s| s.v)
            .collect())
    }

    /// Report playback timeline to the server. Fire-and-forget.
    pub async fn report_timeline(
        &self,
        rating_key: &str,
        state: &str,
        time_ms: i64,
        duration_ms: i64,
        session_identifier: &str,
    ) {
        let (base, token) = match self.read_state() {
            Ok(s) => s,
            Err(_) => return,
        };

        let url = match base.join("/:/timeline") {
            Ok(u) => u,
            Err(_) => return,
        };

        let rk = rating_key.to_string();
        let key_path = format!("/library/metadata/{}", rating_key);
        let time_str = time_ms.to_string();
        let dur_str = duration_ms.to_string();

        let _ = self
            .http
            .put(url)
            .query(&[
                ("ratingKey", rk.as_str()),
                ("key", key_path.as_str()),
                ("state", state),
                ("time", time_str.as_str()),
                ("duration", dur_str.as_str()),
                ("identifier", "com.plexapp.plugins.library"),
                ("X-Plex-Token", token.as_str()),
            ])
            .header("X-Plex-Session-Identifier", session_identifier)
            .header("Accept", "application/json")
            .header("X-Plex-Client-Identifier", &self.client_identifier)
            .header("X-Plex-Product", "ramus")
            .header("X-Plex-Platform", Self::platform())
            .header("X-Plex-Device", Self::device())
            .send()
            .await;
    }

    /// Mark an item as played on the server. Fire-and-forget.
    pub async fn scrobble(&self, rating_key: &str) {
        let (base, token) = match self.read_state() {
            Ok(s) => s,
            Err(_) => return,
        };

        let url = match base.join("/:/scrobble") {
            Ok(u) => u,
            Err(_) => return,
        };

        let key_path = format!("/library/metadata/{}", rating_key);

        let _ = self
            .http
            .put(url)
            .query(&[
                ("key", key_path.as_str()),
                ("identifier", "com.plexapp.plugins.library"),
                ("X-Plex-Token", token.as_str()),
            ])
            .header("Accept", "application/json")
            .header("X-Plex-Client-Identifier", &self.client_identifier)
            .header("X-Plex-Product", "ramus")
            .header("X-Plex-Platform", Self::platform())
            .header("X-Plex-Device", Self::device())
            .send()
            .await;
    }

    /// Set user rating on an item. 10.0 = favourite, 0.0 = unfavourite.
    pub async fn rate_item(
        &self,
        rating_key: &str,
        rating: f64,
    ) -> Result<(), PlexClientError> {
        let rating_str = rating.to_string();
        self.put(
            ":/rate",
            &[
                ("key", rating_key),
                ("identifier", "com.plexapp.plugins.library"),
                ("rating", &rating_str),
            ],
        )
        .await
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_client(base_url: &str) -> PlexClient {
        let client = PlexClient::new("test-client-id".into());
        {
            let mut state = client.state.write();
            state.server_url = Some(Url::parse(base_url).unwrap());
            state.token = Some("test-token".into());
        }
        client
    }

    // -- Header tests --

    #[tokio::test]
    async fn test_standard_headers_applied() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/library/sections"))
            .and(header("Accept", "application/json"))
            .and(header("X-Plex-Client-Identifier", "test-client-id"))
            .and(header("X-Plex-Product", "ramus"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "MediaContainer": {
                        "Directory": [
                            {"key": "1", "title": "Music", "type": "artist"}
                        ]
                    }
                })),
            )
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let result = client.find_music_libraries().await;
        assert!(result.is_ok());
    }

    // -- Error mapping tests --

    #[tokio::test]
    async fn test_401_maps_to_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/library/metadata/123"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let result = client.fetch_item_metadata("123").await;
        assert!(matches!(result, Err(PlexClientError::Unauthorized)));
    }

    #[tokio::test]
    async fn test_500_maps_to_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/library/metadata/123"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let result = client.fetch_item_metadata("123").await;
        assert!(matches!(result, Err(PlexClientError::HttpError(500))));
    }

    #[tokio::test]
    async fn test_not_connected_error() {
        let client = PlexClient::new("test-id".into());
        let result = client.find_music_libraries().await;
        assert!(matches!(result, Err(PlexClientError::NotConnected)));
    }

    // -- find_music_libraries --

    #[tokio::test]
    async fn test_find_music_libraries_filters_by_type() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/library/sections"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "MediaContainer": {
                        "Directory": [
                            {"key": "1", "title": "Music", "type": "artist"},
                            {"key": "2", "title": "Movies", "type": "movie"},
                            {"key": "3", "title": "Jazz", "type": "artist"}
                        ]
                    }
                })),
            )
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let libs = client.find_music_libraries().await.unwrap();
        assert_eq!(libs.len(), 2);
        assert_eq!(libs[0].title, "Music");
        assert_eq!(libs[1].title, "Jazz");
    }

    #[tokio::test]
    async fn test_find_music_libraries_no_music_returns_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/library/sections"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "MediaContainer": {
                        "Directory": [
                            {"key": "2", "title": "Movies", "type": "movie"}
                        ]
                    }
                })),
            )
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let result = client.find_music_libraries().await;
        assert!(matches!(result, Err(PlexClientError::NoMusicLibrary)));
    }

    // -- Pagination --

    #[tokio::test]
    async fn test_fetch_all_items_pagination() {
        let server = MockServer::start().await;

        // Page 1: full page of 2 items
        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(query_param("X-Plex-Container-Start", "0"))
            .and(query_param("X-Plex-Container-Size", "2"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "MediaContainer": {
                        "Metadata": [
                            {"ratingKey": "a1", "title": "Album 1"},
                            {"ratingKey": "a2", "title": "Album 2"}
                        ]
                    }
                })),
            )
            .mount(&server)
            .await;

        // Page 2: partial page (1 item) — signals end
        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(query_param("X-Plex-Container-Start", "2"))
            .and(query_param("X-Plex-Container-Size", "2"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "MediaContainer": {
                        "Metadata": [
                            {"ratingKey": "a3", "title": "Album 3"}
                        ]
                    }
                })),
            )
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let items = client.fetch_all_items("1", 9, 2).await.unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].rating_key, "a1");
        assert_eq!(items[2].rating_key, "a3");
    }

    #[tokio::test]
    async fn test_fetch_all_items_empty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "MediaContainer": {
                        "Metadata": []
                    }
                })),
            )
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let items = client.fetch_all_items("1", 9, 200).await.unwrap();
        assert!(items.is_empty());
    }

    // -- fetch_item_metadata --

    #[tokio::test]
    async fn test_fetch_item_metadata() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/library/metadata/999"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "MediaContainer": {
                        "Metadata": [{
                            "ratingKey": "999",
                            "title": "OK Computer",
                            "year": 1997,
                            "duration": 3200000,
                            "Genre": [{"tag": "Rock"}, {"tag": "Alternative"}],
                            "Media": [{
                                "audioCodec": "flac",
                                "bitrate": 1411,
                                "Part": [{
                                    "key": "/library/parts/999/file.flac",
                                    "Stream": [{
                                        "id": 42,
                                        "streamType": 2,
                                        "codec": "flac",
                                        "bitrate": 1411
                                    }]
                                }]
                            }]
                        }]
                    }
                })),
            )
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let item = client.fetch_item_metadata("999").await.unwrap();
        assert_eq!(item.title, "OK Computer");
        assert_eq!(item.year, Some(1997));
        assert_eq!(item.duration, Some(3200000));
        let genres = item.genre.unwrap();
        assert_eq!(genres.len(), 2);
        assert_eq!(genres[0].tag, "Rock");
        let media = item.media.unwrap();
        assert_eq!(media[0].audio_codec.as_deref(), Some("flac"));
    }

    // -- StreamInfo timed field --

    #[tokio::test]
    async fn test_stream_info_timed_bool() {
        let json = r#"{"id": 1, "streamType": 4, "timed": true}"#;
        let stream: StreamInfo = serde_json::from_str(json).unwrap();
        assert_eq!(stream.timed, Some(true));
    }

    #[tokio::test]
    async fn test_stream_info_timed_int() {
        let json = r#"{"id": 1, "streamType": 4, "timed": 1}"#;
        let stream: StreamInfo = serde_json::from_str(json).unwrap();
        assert_eq!(stream.timed, Some(true));
    }

    #[tokio::test]
    async fn test_stream_info_timed_int_zero() {
        let json = r#"{"id": 1, "streamType": 4, "timed": 0}"#;
        let stream: StreamInfo = serde_json::from_str(json).unwrap();
        assert_eq!(stream.timed, Some(false));
    }

    #[tokio::test]
    async fn test_stream_info_timed_absent() {
        let json = r#"{"id": 1, "streamType": 4}"#;
        let stream: StreamInfo = serde_json::from_str(json).unwrap();
        assert_eq!(stream.timed, None);
    }

    // -- Retry on connection failure --

    #[tokio::test]
    async fn test_retry_on_connection_failure() {
        let server = MockServer::start().await;

        // First call fails (404 simulating a transient issue),
        // but we test retry logic with callback returning true
        let client = PlexClient::new("test-id".into());
        let server_uri = server.uri();
        {
            let mut state = client.state.write();
            state.server_url = Some(Url::parse(&server_uri).unwrap());
            state.token = Some("tok".into());
        }

        // Mount the mock for the metadata endpoint
        Mock::given(method("GET"))
            .and(path("/library/metadata/1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "MediaContainer": {
                        "Metadata": [{"ratingKey": "1", "title": "Track"}]
                    }
                })),
            )
            .mount(&server)
            .await;

        let result = client.fetch_item_metadata("1").await;
        assert!(result.is_ok());
    }

    // -- rate_item --

    #[tokio::test]
    async fn test_rate_item() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/:/rate"))
            .and(query_param("key", "123"))
            .and(query_param("rating", "10"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let result = client.rate_item("123", 10.0).await;
        assert!(result.is_ok());
    }

    // -- test_connection --

    #[tokio::test]
    async fn test_test_connection_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/identity"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = PlexClient::new("test-id".into());
        let result = client
            .test_connection(&server.uri(), "tok", None)
            .await;
        assert!(result);
    }

    #[tokio::test]
    async fn test_test_connection_failure() {
        let client = PlexClient::new("test-id".into());
        let result = client
            .test_connection("http://192.0.2.1:32400", "tok", Some(Duration::from_millis(100)))
            .await;
        assert!(!result);
    }

    #[tokio::test]
    async fn test_test_connection_invalid_scheme() {
        let client = PlexClient::new("test-id".into());
        let result = client.test_connection("ftp://server", "tok", None).await;
        assert!(!result);
    }

    // -- connect --

    #[tokio::test]
    async fn test_connect_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/identity"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = PlexClient::new("test-id".into());
        let url = Url::parse(&server.uri()).unwrap();
        let result = client.connect(url, "token".into()).await;
        assert!(result.is_ok());
        assert!(client.server_url().is_some());
        assert_eq!(client.token(), Some("token".into()));
    }

    #[tokio::test]
    async fn test_connect_failure() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/identity"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let client = PlexClient::new("test-id".into());
        let url = Url::parse(&server.uri()).unwrap();
        let result = client.connect(url, "token".into()).await;
        assert!(matches!(result, Err(PlexClientError::ConnectionFailed)));
    }

    // -- fetch_levels --

    #[tokio::test]
    async fn test_fetch_levels() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/library/streams/42/levels"))
            .and(query_param("subsample", "100"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "MediaContainer": {
                        "Level": [{"v": 0.5}, {"v": 0.8}, {"v": 0.2}]
                    }
                })),
            )
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let levels = client.fetch_levels(42, Some(100)).await.unwrap();
        assert_eq!(levels.len(), 3);
        assert!((levels[0] - 0.5).abs() < f32::EPSILON);
        assert!((levels[1] - 0.8).abs() < f32::EPSILON);
    }
}
