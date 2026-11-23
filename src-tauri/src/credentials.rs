//! The system credential store (login Keychain, Windows Credential Manager)
//! under this build's service names (`app_identity::credential_service`).

pub(crate) fn get_password(service: &str, user: &str) -> Result<Option<String>, keyring::Error> {
    match keyring::Entry::new(service, user).and_then(|entry| entry.get_password()) {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(error),
    }
}

pub(crate) fn set_password(service: &str, user: &str, value: &str) -> Result<(), keyring::Error> {
    keyring::Entry::new(service, user).and_then(|entry| entry.set_password(value))
}

pub(crate) fn delete_password(service: &str, user: &str) -> Result<(), keyring::Error> {
    match keyring::Entry::new(service, user).and_then(|entry| entry.delete_credential()) {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(error),
    }
}
