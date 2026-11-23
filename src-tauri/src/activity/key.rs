//! The SQLCipher key of `activity.sqlite3`: 32 random bytes, hex encoded,
//! stored only in the Keychain. The key never reaches logs, the frontend, or
//! the debug export.

use rand::RngCore;
use zeroize::Zeroizing;

/// The service name under `app_identity`'s prefix (`-dev` in debug builds, so
/// a development binary never reads or rotates the release key).
const KEYCHAIN_SERVICE_NAME: &str = "activity-db";
const KEYCHAIN_USER: &str = "sqlcipher-key";
pub const KEY_BYTES: usize = 32;

#[derive(Debug, thiserror::Error)]
#[error("keychain: {0}")]
pub struct KeyStoreError(pub String);

/// Where the database key lives. The Keychain in the app; memory in tests.
pub trait ActivityKeyStore: Send + Sync {
    fn load(&self) -> Result<Option<Zeroizing<String>>, KeyStoreError>;
    fn store(&self, key_hex: &str) -> Result<(), KeyStoreError>;
    fn delete(&self) -> Result<(), KeyStoreError>;
}

pub fn keychain_service_for_build(debug_assertions: bool) -> String {
    crate::app_identity::credential_service_for(
        crate::app_identity::APP_IDENTIFIER,
        debug_assertions,
        KEYCHAIN_SERVICE_NAME,
    )
}

/// A fresh random key, hex encoded (64 chars).
pub fn generate_key_hex() -> Zeroizing<String> {
    let mut bytes = Zeroizing::new([0_u8; KEY_BYTES]);
    rand::rngs::OsRng.fill_bytes(bytes.as_mut());
    let mut hex = Zeroizing::new(String::with_capacity(KEY_BYTES * 2));
    for byte in bytes.iter() {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

pub fn is_valid_key_hex(value: &str) -> bool {
    value.len() == KEY_BYTES * 2 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub struct KeychainKeyStore {
    service: String,
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
impl KeychainKeyStore {
    pub fn for_current_build() -> Self {
        Self {
            service: keychain_service_for_build(cfg!(debug_assertions)),
        }
    }

    fn entry(&self) -> Result<keyring::Entry, KeyStoreError> {
        keyring::Entry::new(&self.service, KEYCHAIN_USER)
            .map_err(|error| KeyStoreError(error.to_string()))
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
impl ActivityKeyStore for KeychainKeyStore {
    fn load(&self) -> Result<Option<Zeroizing<String>>, KeyStoreError> {
        match self.entry()?.get_password() {
            Ok(value) => Ok(Some(Zeroizing::new(value))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(KeyStoreError(error.to_string())),
        }
    }

    fn store(&self, key_hex: &str) -> Result<(), KeyStoreError> {
        self.entry()?
            .set_password(key_hex)
            .map_err(|error| KeyStoreError(error.to_string()))
    }

    fn delete(&self) -> Result<(), KeyStoreError> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(KeyStoreError(error.to_string())),
        }
    }
}

/// In-memory key store for tests and the engine harness.
#[cfg(test)]
#[derive(Default)]
pub struct MemoryKeyStore(std::sync::Mutex<Option<String>>);

#[cfg(test)]
impl MemoryKeyStore {
    pub fn current(&self) -> Option<String> {
        self.slot().clone()
    }

    fn slot(&self) -> std::sync::MutexGuard<'_, Option<String>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
impl ActivityKeyStore for MemoryKeyStore {
    fn load(&self) -> Result<Option<Zeroizing<String>>, KeyStoreError> {
        Ok(self.slot().clone().map(Zeroizing::new))
    }

    fn store(&self, key_hex: &str) -> Result<(), KeyStoreError> {
        *self.slot() = Some(key_hex.to_string());
        Ok(())
    }

    fn delete(&self) -> Result<(), KeyStoreError> {
        *self.slot() = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_keys_are_32_random_bytes_in_hex() {
        let first = generate_key_hex();
        let second = generate_key_hex();
        assert!(is_valid_key_hex(&first));
        assert!(is_valid_key_hex(&second));
        assert_ne!(*first, *second);
    }

    #[test]
    fn dev_builds_use_a_separate_keychain_service() {
        assert_ne!(
            keychain_service_for_build(true),
            keychain_service_for_build(false)
        );
    }
}
