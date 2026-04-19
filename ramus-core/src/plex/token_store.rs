#[cfg(not(target_os = "ios"))]
use std::collections::HashMap;
#[cfg(not(target_os = "ios"))]
use std::fs;
use std::path::PathBuf;

#[cfg(not(target_os = "ios"))]
use aes_gcm::aead::{Aead, KeyInit};
#[cfg(not(target_os = "ios"))]
use aes_gcm::{Aes256Gcm, Nonce};
#[cfg(not(target_os = "ios"))]
use parking_lot::Mutex;
#[cfg(not(target_os = "ios"))]
use rand::RngCore;
#[cfg(not(target_os = "ios"))]
use sha2::{Digest, Sha256};

#[cfg(target_os = "ios")]
use std::sync::Arc;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKey {
    AuthToken,
    ServerToken,
}

impl TokenKey {
    fn as_str(&self) -> &'static str {
        match self {
            Self::AuthToken => "plexAuthToken",
            Self::ServerToken => "plexServerToken",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TokenStoreError {
    #[error("could not determine config directory")]
    NoConfigDir,
    #[error("could not read hardware UUID")]
    NoHardwareUUID,
    #[error("encryption error: {0}")]
    Encryption(String),
    #[error("decryption error: {0}")]
    Decryption(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("no keychain backend registered")]
    NoKeychainBackend,
}

/// iOS-only callback surface implemented by `ramus-tauri` at startup so
/// `ramus-core` can hit the Swift `KeychainBridge` without taking a reverse
/// dependency on the plugin crate. Methods return `true` for write/delete
/// success. `read` returns `None` on miss.
#[cfg(target_os = "ios")]
pub trait KeychainBackend: Send + Sync {
    fn read(&self, account: &str) -> Option<String>;
    fn write(&self, account: &str, value: &str) -> bool;
    fn delete(&self, account: &str) -> bool;
}

#[cfg(target_os = "ios")]
static KEYCHAIN_BACKEND: OnceLock<Arc<dyn KeychainBackend>> = OnceLock::new();

/// Register the iOS keychain backend. Must be called exactly once at app
/// startup, before any `TokenStore::new()` call. Subsequent calls are
/// ignored — `OnceLock` semantics.
#[cfg(target_os = "ios")]
pub fn set_keychain_backend(backend: Arc<dyn KeychainBackend>) {
    let _ = KEYCHAIN_BACKEND.set(backend);
}

/// Encrypted file-based token storage.
///
/// Tokens are AES-256-GCM encrypted with a key derived from the machine's
/// hardware UUID, rendering the file inert if copied elsewhere.
///
/// On iOS, tokens live in the system Keychain instead (see the iOS-specific
/// impl below). The public API is identical on both platforms.
#[cfg(not(target_os = "ios"))]
pub struct TokenStore {
    dir: PathBuf,
    lock: Mutex<()>,
    /// Test-only encryption key override.
    key_override: Option<[u8; 32]>,
}

#[cfg(not(target_os = "ios"))]
const NONCE_SIZE: usize = 12;
#[cfg(not(target_os = "ios"))]
const TOKEN_FILE: &str = "tokens.enc";

#[cfg(not(target_os = "ios"))]
impl TokenStore {
    /// Create a `TokenStore` using the platform config directory.
    pub fn new() -> Result<Self, TokenStoreError> {
        let dir = default_config_dir()?;
        Ok(Self {
            dir,
            lock: Mutex::new(()),
            key_override: None,
        })
    }

    /// Create a `TokenStore` with a custom directory. For tests.
    pub fn with_dir(dir: PathBuf) -> Self {
        Self {
            dir,
            lock: Mutex::new(()),
            key_override: None,
        }
    }

    /// Create a `TokenStore` with a custom directory and key. For tests.
    #[cfg(test)]
    pub(crate) fn with_dir_and_key(dir: PathBuf, key: [u8; 32]) -> Self {
        Self {
            dir,
            lock: Mutex::new(()),
            key_override: Some(key),
        }
    }

    fn token_file(&self) -> PathBuf {
        self.dir.join(TOKEN_FILE)
    }

    fn encryption_key(&self) -> Result<[u8; 32], TokenStoreError> {
        if let Some(key) = self.key_override {
            return Ok(key);
        }
        let uuid = hardware_uuid()?;
        let hash = Sha256::digest(uuid.as_bytes());
        Ok(hash.into())
    }

    /// Read a token.
    pub fn read(&self, key: TokenKey) -> Option<String> {
        let _guard = self.lock.lock();
        let tokens = self.load_all().ok()?;
        tokens.get(key.as_str()).cloned()
    }

    /// Write a token. Returns `true` on success.
    pub fn write(&self, key: TokenKey, value: &str) -> bool {
        let _guard = self.lock.lock();
        let mut tokens = self.load_all().unwrap_or_default();
        tokens.insert(key.as_str().to_string(), value.to_string());
        self.save_all(&tokens).is_ok()
    }

    /// Delete a token. Returns `true` on success.
    pub fn delete(&self, key: TokenKey) -> bool {
        let _guard = self.lock.lock();
        let mut tokens = match self.load_all() {
            Ok(t) => t,
            Err(_) => return true,
        };
        tokens.remove(key.as_str());
        self.save_all(&tokens).is_ok()
    }

    fn load_all(&self) -> Result<HashMap<String, String>, TokenStoreError> {
        let path = self.token_file();
        if !path.exists() {
            return Ok(HashMap::new());
        }

        let data = fs::read(&path)?;
        if data.len() < NONCE_SIZE + 16 {
            return Err(TokenStoreError::Decryption("data too short".into()));
        }

        let key_bytes = self.encryption_key()?;
        let cipher =
            Aes256Gcm::new_from_slice(&key_bytes).map_err(|e| TokenStoreError::Encryption(e.to_string()))?;

        let (nonce_bytes, ciphertext) = data.split_at(NONCE_SIZE);
        let nonce = Nonce::from_slice(nonce_bytes);

        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| TokenStoreError::Decryption(e.to_string()))?;

        let tokens: HashMap<String, String> = serde_json::from_slice(&plaintext)?;
        Ok(tokens)
    }

    fn save_all(&self, tokens: &HashMap<String, String>) -> Result<(), TokenStoreError> {
        let path = self.token_file();

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
            }
        }

        let key_bytes = self.encryption_key()?;
        let cipher =
            Aes256Gcm::new_from_slice(&key_bytes).map_err(|e| TokenStoreError::Encryption(e.to_string()))?;

        let plaintext = serde_json::to_vec(tokens)?;

        let mut nonce_bytes = [0u8; NONCE_SIZE];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext.as_ref())
            .map_err(|e| TokenStoreError::Encryption(e.to_string()))?;

        // Layout: nonce || ciphertext || tag (tag appended by aes-gcm).
        let mut blob = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
        blob.extend_from_slice(&nonce_bytes);
        blob.extend_from_slice(&ciphertext);

        fs::write(&path, &blob)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }

        Ok(())
    }
}

/// Runtime override for the config directory. Populated by the host app
/// on platforms where the `directories` crate can't resolve a sensible
/// path on its own (Android, where `ProjectDirs::from` returns `None`
/// because there's no XDG-equivalent — apps live in per-package sandboxes
/// reached through the Java `Context.getFilesDir()` API). The host reads
/// the right path via Tauri's `PathResolver` and calls `set_config_dir`
/// once at startup before any `TokenStore::new()` or `config_dir()` call.
static CONFIG_DIR_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

/// Install the config-dir override. Call exactly once, before any code
/// reads `config_dir()`. Subsequent calls are ignored (`OnceLock`).
pub fn set_config_dir(path: PathBuf) {
    let _ = CONFIG_DIR_OVERRIDE.set(path);
}

fn default_config_dir() -> Result<PathBuf, TokenStoreError> {
    if let Some(path) = CONFIG_DIR_OVERRIDE.get() {
        return Ok(path.clone());
    }
    directories::ProjectDirs::from("com", "raspsoft", "ramus")
        .map(|d| d.data_dir().to_path_buf())
        .ok_or(TokenStoreError::NoConfigDir)
}

/// Platform config directory path, shared with the auth module.
pub fn config_dir() -> Result<PathBuf, TokenStoreError> {
    default_config_dir()
}

/// iOS keychain-backed implementation. Tokens are stored directly as
/// generic-password items keyed by `TokenKey::as_str()`; no on-disk file,
/// no AES layer. The struct holds the registered backend via `Arc` so it
/// can be freely `Clone`d if ever needed.
#[cfg(target_os = "ios")]
pub struct TokenStore {
    backend: Arc<dyn KeychainBackend>,
}

#[cfg(target_os = "ios")]
impl TokenStore {
    pub fn new() -> Result<Self, TokenStoreError> {
        let backend = KEYCHAIN_BACKEND
            .get()
            .cloned()
            .ok_or(TokenStoreError::NoKeychainBackend)?;
        Ok(Self { backend })
    }

    pub fn read(&self, key: TokenKey) -> Option<String> {
        self.backend.read(key.as_str())
    }

    pub fn write(&self, key: TokenKey, value: &str) -> bool {
        self.backend.write(key.as_str(), value)
    }

    pub fn delete(&self, key: TokenKey) -> bool {
        self.backend.delete(key.as_str())
    }
}

#[cfg(target_os = "macos")]
fn hardware_uuid() -> Result<String, TokenStoreError> {
    use core_foundation::base::TCFType;
    use core_foundation::string::CFString;
    use std::ffi::c_void;

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOServiceMatching(name: *const std::ffi::c_char) -> *mut c_void;
        fn IOServiceGetMatchingService(
            mainPort: u32,
            matching: *mut c_void,
        ) -> u32;
        fn IORegistryEntryCreateCFProperty(
            entry: u32,
            key: *const c_void,
            allocator: *const c_void,
            options: u32,
        ) -> *const c_void;
        fn IOObjectRelease(object: u32) -> u32;
    }

    unsafe {
        let matching = IOServiceMatching(c"IOPlatformExpertDevice".as_ptr());
        if matching.is_null() {
            return Err(TokenStoreError::NoHardwareUUID);
        }

        let service = IOServiceGetMatchingService(0, matching);
        if service == 0 {
            return Err(TokenStoreError::NoHardwareUUID);
        }

        let key = CFString::new("IOPlatformUUID");
        let property = IORegistryEntryCreateCFProperty(
            service,
            key.as_concrete_TypeRef() as *const c_void,
            std::ptr::null(),
            0,
        );
        IOObjectRelease(service);

        if property.is_null() {
            return Err(TokenStoreError::NoHardwareUUID);
        }

        let cf_string: CFString = TCFType::wrap_under_create_rule(property as _);
        Ok(cf_string.to_string())
    }
}

#[cfg(target_os = "windows")]
fn hardware_uuid() -> Result<String, TokenStoreError> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let key = hklm
        .open_subkey("SOFTWARE\\Microsoft\\Cryptography")
        .map_err(|_| TokenStoreError::NoHardwareUUID)?;
    let guid: String = key
        .get_value("MachineGuid")
        .map_err(|_| TokenStoreError::NoHardwareUUID)?;
    Ok(guid)
}

#[cfg(target_os = "linux")]
fn hardware_uuid() -> Result<String, TokenStoreError> {
    fs::read_to_string("/etc/machine-id")
        .map(|s| s.trim().to_string())
        .map_err(|_| TokenStoreError::NoHardwareUUID)
}

// iOS doesn't get a `hardware_uuid` — the iOS TokenStore is keychain-backed
// and bypasses the AES layer entirely.

// Android doesn't expose a stable hardware UUID the way desktop OSes do
// (Settings.Secure.ANDROID_ID requires going through the Java SDK), so we
// persist a random UUID in the app's sandboxed config dir on first run and
// read it back thereafter. Same guarantee as Linux's `/etc/machine-id`:
// stable per install, invalidated if the app's data is wiped.
#[cfg(target_os = "android")]
fn hardware_uuid() -> Result<String, TokenStoreError> {
    let dir = default_config_dir()?;
    let path = dir.join("machine-id");
    if let Ok(existing) = fs::read_to_string(&path) {
        let s = existing.trim().to_string();
        if !s.is_empty() {
            return Ok(s);
        }
    }
    fs::create_dir_all(&dir)?;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    let id = bytes.iter().fold(String::with_capacity(32), |mut acc, b| {
        use std::fmt::Write;
        let _ = write!(acc, "{b:02x}");
        acc
    });
    fs::write(&path, &id)?;
    Ok(id)
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "windows",
    target_os = "linux",
    target_os = "ios",
    target_os = "android"
)))]
fn hardware_uuid() -> Result<String, TokenStoreError> {
    Err(TokenStoreError::NoHardwareUUID)
}

// Tests exercise the file+AES backend; on iOS the backend is keychain-only
// so the test helpers (`with_dir_and_key`, `Sha256`, etc.) aren't compiled.
#[cfg(all(test, not(target_os = "ios")))]
mod tests {
    use super::*;

    fn test_store(dir: &std::path::Path) -> TokenStore {
        let key = Sha256::digest(b"test-machine-id");
        TokenStore::with_dir_and_key(dir.to_path_buf(), key.into())
    }

    #[test]
    fn test_write_read_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());

        assert!(store.write(TokenKey::AuthToken, "my-auth-token"));
        assert_eq!(store.read(TokenKey::AuthToken), Some("my-auth-token".into()));
    }

    #[test]
    fn test_read_missing_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());

        assert_eq!(store.read(TokenKey::AuthToken), None);
    }

    #[test]
    fn test_write_multiple_keys() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());

        assert!(store.write(TokenKey::AuthToken, "auth-tok"));
        assert!(store.write(TokenKey::ServerToken, "server-tok"));

        assert_eq!(store.read(TokenKey::AuthToken), Some("auth-tok".into()));
        assert_eq!(store.read(TokenKey::ServerToken), Some("server-tok".into()));
    }

    #[test]
    fn test_overwrite_key() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());

        assert!(store.write(TokenKey::AuthToken, "first"));
        assert!(store.write(TokenKey::AuthToken, "second"));
        assert_eq!(store.read(TokenKey::AuthToken), Some("second".into()));
    }

    #[test]
    fn test_delete_key() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());

        store.write(TokenKey::AuthToken, "tok");
        assert!(store.delete(TokenKey::AuthToken));
        assert_eq!(store.read(TokenKey::AuthToken), None);
    }

    #[test]
    fn test_delete_preserves_other_keys() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());

        store.write(TokenKey::AuthToken, "auth");
        store.write(TokenKey::ServerToken, "server");
        store.delete(TokenKey::AuthToken);

        assert_eq!(store.read(TokenKey::AuthToken), None);
        assert_eq!(store.read(TokenKey::ServerToken), Some("server".into()));
    }

    #[test]
    fn test_delete_nonexistent_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());

        assert!(store.delete(TokenKey::AuthToken));
    }

    #[test]
    fn test_different_keys_produce_different_ciphertext() {
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();

        let key1 = Sha256::digest(b"machine-a");
        let key2 = Sha256::digest(b"machine-b");

        let store1 = TokenStore::with_dir_and_key(dir1.path().to_path_buf(), key1.into());
        let store2 = TokenStore::with_dir_and_key(dir2.path().to_path_buf(), key2.into());

        store1.write(TokenKey::AuthToken, "same-token");
        store2.write(TokenKey::AuthToken, "same-token");

        let file1 = fs::read(dir1.path().join(TOKEN_FILE)).unwrap();
        let file2 = fs::read(dir2.path().join(TOKEN_FILE)).unwrap();

        assert_ne!(file1, file2);
    }

    #[test]
    fn test_wrong_key_cannot_decrypt() {
        let dir = tempfile::tempdir().unwrap();

        let key1 = Sha256::digest(b"correct-key");
        let key2 = Sha256::digest(b"wrong-key");

        let store1 = TokenStore::with_dir_and_key(dir.path().to_path_buf(), key1.into());
        store1.write(TokenKey::AuthToken, "secret");

        let store2 = TokenStore::with_dir_and_key(dir.path().to_path_buf(), key2.into());
        assert_eq!(store2.read(TokenKey::AuthToken), None);
    }

    #[cfg(unix)]
    #[test]
    fn test_file_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        store.write(TokenKey::AuthToken, "tok");

        let metadata = fs::metadata(dir.path().join(TOKEN_FILE)).unwrap();
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn test_token_key_strings() {
        assert_eq!(TokenKey::AuthToken.as_str(), "plexAuthToken");
        assert_eq!(TokenKey::ServerToken.as_str(), "plexServerToken");
    }
}
