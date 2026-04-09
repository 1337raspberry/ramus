use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use parking_lot::Mutex;
use rand::RngCore;
use sha2::{Digest, Sha256};

// --- Token Keys ---

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

// --- Errors ---

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
}

// --- TokenStore ---

/// Encrypted file-based token storage.
///
/// Tokens are AES-256-GCM encrypted with a key derived from the machine's
/// hardware UUID, so the file is inert if copied to another machine.
pub struct TokenStore {
    dir: PathBuf,
    lock: Mutex<()>,
    /// Optional override for the encryption key (used in tests).
    key_override: Option<[u8; 32]>,
}

const NONCE_SIZE: usize = 12;
const TOKEN_FILE: &str = "tokens.enc";

impl TokenStore {
    /// Create a new TokenStore using the platform config directory.
    pub fn new() -> Result<Self, TokenStoreError> {
        let dir = default_config_dir()?;
        Ok(Self {
            dir,
            lock: Mutex::new(()),
            key_override: None,
        })
    }

    /// Create a TokenStore with a custom directory (for testing).
    pub fn with_dir(dir: PathBuf) -> Self {
        Self {
            dir,
            lock: Mutex::new(()),
            key_override: None,
        }
    }

    /// Create a TokenStore with a custom directory and key (for testing).
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

    /// Read a token by key.
    pub fn read(&self, key: TokenKey) -> Option<String> {
        let _guard = self.lock.lock();
        let tokens = self.load_all().ok()?;
        tokens.get(key.as_str()).cloned()
    }

    /// Write a token. Returns true on success.
    pub fn write(&self, key: TokenKey, value: &str) -> bool {
        let _guard = self.lock.lock();
        let mut tokens = self.load_all().unwrap_or_default();
        tokens.insert(key.as_str().to_string(), value.to_string());
        self.save_all(&tokens).is_ok()
    }

    /// Delete a token. Returns true on success.
    pub fn delete(&self, key: TokenKey) -> bool {
        let _guard = self.lock.lock();
        let mut tokens = match self.load_all() {
            Ok(t) => t,
            Err(_) => return true, // nothing to delete
        };
        tokens.remove(key.as_str());
        self.save_all(&tokens).is_ok()
    }

    // -- Internal --

    fn load_all(&self) -> Result<HashMap<String, String>, TokenStoreError> {
        let path = self.token_file();
        if !path.exists() {
            return Ok(HashMap::new());
        }

        let data = fs::read(&path)?;
        if data.len() < NONCE_SIZE + 16 {
            // Too short to contain nonce + tag
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

        // Ensure directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
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

        // nonce || ciphertext || tag (tag is appended by aes-gcm)
        let mut blob = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
        blob.extend_from_slice(&nonce_bytes);
        blob.extend_from_slice(&ciphertext);

        fs::write(&path, &blob)?;

        // Set permissions to 0o600 on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }

        Ok(())
    }
}

// --- Platform config directory ---

fn default_config_dir() -> Result<PathBuf, TokenStoreError> {
    directories::ProjectDirs::from("com", "raspsoft", "ramus")
        .map(|d| d.data_dir().to_path_buf())
        .ok_or(TokenStoreError::NoConfigDir)
}

/// Returns the platform config directory path (for use by auth module).
pub fn config_dir() -> Result<PathBuf, TokenStoreError> {
    default_config_dir()
}

// --- Hardware UUID (platform-specific) ---

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

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn hardware_uuid() -> Result<String, TokenStoreError> {
    Err(TokenStoreError::NoHardwareUUID)
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store(dir: &std::path::Path) -> TokenStore {
        // Use a fixed test key instead of hardware UUID
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

        // Files should differ (different keys + different random nonces)
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
