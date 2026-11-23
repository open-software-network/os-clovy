//! Keychain storage for endpoint API keys. Keys live only here (never in
//! `provider-settings.json`, never in a DTO, never in a log line). Debug builds
//! use a `-dev` service so a development build never reads or overwrites the
//! installed app's credentials. Test builds use an in-memory store so unit
//! tests never touch the real Keychain.

use std::{collections::HashMap, sync::Mutex};

pub trait SecretStore: Send + Sync {
    fn get(&self, account: &str) -> Result<Option<String>, String>;
    fn set(&self, account: &str, value: &str) -> Result<(), String>;
    fn delete(&self, account: &str) -> Result<(), String>;
}

pub fn store() -> &'static dyn SecretStore {
    #[cfg(not(test))]
    {
        static STORE: std::sync::LazyLock<KeychainSecretStore> =
            std::sync::LazyLock::new(KeychainSecretStore::default);
        &*STORE
    }
    #[cfg(test)]
    {
        static STORE: std::sync::LazyLock<MemorySecretStore> =
            std::sync::LazyLock::new(MemorySecretStore::default);
        &*STORE
    }
}

/// Keychain-backed store with a read-through cache, so the agent loop does not
/// hit the Keychain on every model call.
#[cfg(not(test))]
#[derive(Default)]
pub struct KeychainSecretStore {
    cache: Mutex<HashMap<String, Option<String>>>,
}

#[cfg(not(test))]
impl KeychainSecretStore {
    fn service() -> String {
        crate::app_identity::credential_service("llm-providers")
    }
}

#[cfg(not(test))]
impl SecretStore for KeychainSecretStore {
    fn get(&self, account: &str) -> Result<Option<String>, String> {
        if let Some(cached) = self
            .cache
            .lock()
            .ok()
            .and_then(|cache| cache.get(account).cloned())
        {
            return Ok(cached);
        }
        let value = match keyring::Entry::new(&Self::service(), account)
            .and_then(|entry| entry.get_password())
        {
            Ok(value) => Some(value),
            Err(keyring::Error::NoEntry) => None,
            Err(error) => return Err(error.to_string()),
        };
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(account.to_string(), value.clone());
        }
        Ok(value)
    }

    fn set(&self, account: &str, value: &str) -> Result<(), String> {
        keyring::Entry::new(&Self::service(), account)
            .and_then(|entry| entry.set_password(value))
            .map_err(|error| error.to_string())?;
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(account.to_string(), Some(value.to_string()));
        }
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        match keyring::Entry::new(&Self::service(), account)
            .and_then(|entry| entry.delete_credential())
        {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(error) => return Err(error.to_string()),
        }
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(account.to_string(), None);
        }
        Ok(())
    }
}

/// In-memory store for tests and for callers that need an isolated store.
#[derive(Default)]
pub struct MemorySecretStore {
    values: Mutex<HashMap<String, String>>,
    fail_writes: bool,
}

impl MemorySecretStore {
    /// A store whose writes fail, to exercise the "Keychain unavailable" path.
    #[cfg(test)]
    pub fn failing() -> Self {
        Self {
            values: Mutex::new(HashMap::new()),
            fail_writes: true,
        }
    }
}

impl SecretStore for MemorySecretStore {
    fn get(&self, account: &str) -> Result<Option<String>, String> {
        Ok(self
            .values
            .lock()
            .map_err(|_| "secret store lock failed".to_string())?
            .get(account)
            .cloned())
    }

    fn set(&self, account: &str, value: &str) -> Result<(), String> {
        if self.fail_writes {
            return Err("secret store unavailable".to_string());
        }
        self.values
            .lock()
            .map_err(|_| "secret store lock failed".to_string())?
            .insert(account.to_string(), value.to_string());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        self.values
            .lock()
            .map_err(|_| "secret store lock failed".to_string())?
            .remove(account);
        Ok(())
    }
}
