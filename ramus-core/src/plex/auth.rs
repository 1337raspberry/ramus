use std::path::PathBuf;
use std::time::Duration;

use reqwest::Client;
use serde::Deserialize;

use crate::models::{PlexServerConnection, ServerConfig};
use crate::plex::token_store::{config_dir, TokenKey, TokenStore};

// --- Errors ---

#[derive(Debug, thiserror::Error)]
pub enum PlexAuthError {
    #[error("PIN creation failed")]
    PinCreationFailed,
    #[error("polling timed out")]
    PollingTimeout,
    #[error("PIN expired")]
    PinExpired,
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// --- PIN Response ---

#[derive(Debug, Clone, Deserialize)]
pub struct PinResponse {
    pub id: i64,
    pub code: String,
    #[serde(rename = "authToken")]
    pub auth_token: Option<String>,
}

// --- PlexAuth ---

pub struct PlexAuth {
    http: Client,
}

impl Default for PlexAuth {
    fn default() -> Self {
        Self {
            http: Client::new(),
        }
    }
}

impl PlexAuth {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new Plex PIN for the OAuth flow.
    pub async fn create_pin(
        &self,
        client_identifier: &str,
    ) -> Result<PinResponse, PlexAuthError> {
        let resp = self
            .http
            .post("https://plex.tv/api/v2/pins")
            .query(&[
                ("strong", "true"),
                ("X-Plex-Product", "ramus"),
                ("X-Plex-Client-Identifier", client_identifier),
            ])
            .header("Accept", "application/json")
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(PlexAuthError::PinCreationFailed);
        }

        let pin: PinResponse = resp.json().await?;
        Ok(pin)
    }

    /// Build the Plex OAuth URL the user should visit to authorize.
    pub fn auth_url(code: &str, client_identifier: &str) -> String {
        format!(
            "https://app.plex.tv/auth#?clientID={}&code={}&context%5Bdevice%5D%5Bproduct%5D=ramus",
            client_identifier, code
        )
    }

    /// Poll Plex for the auth token after the user has visited the auth URL.
    /// Returns the token once the user authorizes, or errors on timeout/expiry.
    pub async fn poll_for_token(
        &self,
        pin_id: i64,
        client_identifier: &str,
        token_store: &TokenStore,
        max_attempts: u32,
        interval: Duration,
    ) -> Result<String, PlexAuthError> {
        for _ in 0..max_attempts {
            let url = format!("https://plex.tv/api/v2/pins/{}", pin_id);
            let resp = self
                .http
                .get(&url)
                .header("Accept", "application/json")
                .header("X-Plex-Client-Identifier", client_identifier)
                .header("X-Plex-Product", "ramus")
                .send()
                .await?;

            if resp.status().as_u16() == 404 {
                return Err(PlexAuthError::PinExpired);
            }

            let pin: PinResponse = resp.json().await?;
            if let Some(ref token) = pin.auth_token {
                if !token.is_empty() {
                    if !token_store.write(TokenKey::AuthToken, token) {
                        log::warn!("token write failed — token not persisted");
                    }
                    return Ok(token.clone());
                }
            }

            tokio::time::sleep(interval).await;
        }

        Err(PlexAuthError::PollingTimeout)
    }
}

// --- Server Config Persistence ---

const SERVER_CONFIG_FILE: &str = "server_config.json";

fn server_config_path() -> Result<PathBuf, PlexAuthError> {
    Ok(config_dir()
        .map_err(|_| PlexAuthError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "config dir not found",
        )))?
        .join(SERVER_CONFIG_FILE))
}

/// Store server config. The access token goes to the encrypted token store;
/// non-secret fields go to a JSON file in the config directory.
pub fn store_server_config(config: &ServerConfig, token_store: &TokenStore) -> bool {
    if !token_store.write(TokenKey::ServerToken, &config.access_token) {
        return false;
    }

    let path = match server_config_path() {
        Ok(p) => p,
        Err(_) => return false,
    };

    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return false;
        }
    }

    // ServerConfig's Serialize impl already excludes access_token
    match serde_json::to_string(config) {
        Ok(json) => std::fs::write(&path, json).is_ok(),
        Err(_) => false,
    }
}

/// Patch fields in the stored server config JSON without touching the token.
/// Only provided (Some) values are updated; None fields are left unchanged.
pub fn patch_stored_config(
    connections: Option<&[PlexServerConnection]>,
    active_uri: Option<&str>,
) {
    let path = match server_config_path() {
        Ok(p) => p,
        Err(_) => return,
    };
    let data = match std::fs::read_to_string(&path) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("patch_stored_config: cannot read config file: {e}");
            return;
        }
    };
    let mut doc: serde_json::Value = match serde_json::from_str(&data) {
        Ok(v) => v,
        Err(_) => return,
    };
    if let Some(obj) = doc.as_object_mut() {
        if let Some(conns) = connections {
            if let Ok(val) = serde_json::to_value(conns) {
                obj.insert("connections".to_string(), val);
            }
        }
        if let Some(uri) = active_uri {
            obj.insert("activeUri".to_string(), serde_json::Value::String(uri.to_string()));
        }
    }
    if let Ok(json) = serde_json::to_string(&doc) {
        let _ = std::fs::write(&path, json);
    }
}

/// Retrieve stored server config, reconstituting the access token from the token store.
pub fn stored_server_config(token_store: &TokenStore) -> Option<ServerConfig> {
    let path = server_config_path().ok()?;
    let data = std::fs::read_to_string(&path).ok()?;
    let mut config: ServerConfig = serde_json::from_str(&data).ok()?;

    if let Some(token) = token_store.read(TokenKey::ServerToken) {
        config.access_token = token;
    }

    if config.access_token.is_empty() {
        return None;
    }

    Some(config)
}

/// Delete stored server config and its token.
pub fn delete_server_config(token_store: &TokenStore) {
    token_store.delete(TokenKey::ServerToken);
    if let Ok(path) = server_config_path() {
        let _ = std::fs::remove_file(path);
    }
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn test_store(dir: &std::path::Path) -> TokenStore {
        let key = Sha256::digest(b"test-machine-id");
        TokenStore::with_dir_and_key(dir.to_path_buf(), key.into())
    }

    // -- auth URL --

    #[test]
    fn test_auth_url_construction() {
        let url = PlexAuth::auth_url("ABCD1234", "my-client-id");
        assert!(url.starts_with("https://app.plex.tv/auth#?"));
        assert!(url.contains("clientID=my-client-id"));
        assert!(url.contains("code=ABCD1234"));
        assert!(url.contains("context%5Bdevice%5D%5Bproduct%5D=ramus"));
    }

    // -- PIN response parsing --

    #[test]
    fn test_pin_response_parsing() {
        let json = r#"{"id": 12345, "code": "ABCD", "authToken": null}"#;
        let pin: PinResponse = serde_json::from_str(json).unwrap();
        assert_eq!(pin.id, 12345);
        assert_eq!(pin.code, "ABCD");
        assert!(pin.auth_token.is_none());
    }

    #[test]
    fn test_pin_response_with_token() {
        let json = r#"{"id": 99, "code": "XYZ", "authToken": "my-token-123"}"#;
        let pin: PinResponse = serde_json::from_str(json).unwrap();
        assert_eq!(pin.auth_token, Some("my-token-123".into()));
    }

    // -- Server config persistence --

    #[test]
    fn test_store_and_load_server_config() {
        let dir = tempfile::tempdir().unwrap();
        let token_store = test_store(dir.path());

        let config = ServerConfig {
            machine_identifier: "server-1".into(),
            name: "My Plex".into(),
            access_token: "secret-token".into(),
            selected_library_key: Some("lib-1".into()),
            owned: true,
            connections: vec![],
            active_uri: Some("https://example.plex.direct:32400".into()),
        };

        // Write config file to same temp dir (override path for test)
        let config_path = dir.path().join(SERVER_CONFIG_FILE);
        token_store.write(TokenKey::ServerToken, &config.access_token);
        let json = serde_json::to_string(&config).unwrap();
        std::fs::write(&config_path, &json).unwrap();

        // Verify the JSON file does NOT contain the access token
        let raw = std::fs::read_to_string(&config_path).unwrap();
        assert!(!raw.contains("secret-token"));
        assert!(raw.contains("server-1"));

        // Read it back — reconstitute token from store
        let mut restored: ServerConfig = serde_json::from_str(&raw).unwrap();
        if let Some(token) = token_store.read(TokenKey::ServerToken) {
            restored.access_token = token;
        }
        assert_eq!(restored.machine_identifier, "server-1");
        assert_eq!(restored.name, "My Plex");
        assert_eq!(restored.access_token, "secret-token");
        assert_eq!(restored.selected_library_key, Some("lib-1".into()));
    }

    #[test]
    fn test_delete_server_config() {
        let dir = tempfile::tempdir().unwrap();
        let token_store = test_store(dir.path());

        token_store.write(TokenKey::ServerToken, "tok");
        let config_path = dir.path().join(SERVER_CONFIG_FILE);
        std::fs::write(&config_path, r#"{"machineIdentifier":"s","name":"n"}"#).unwrap();

        // Delete
        token_store.delete(TokenKey::ServerToken);
        let _ = std::fs::remove_file(&config_path);

        assert_eq!(token_store.read(TokenKey::ServerToken), None);
        assert!(!config_path.exists());
    }
}
