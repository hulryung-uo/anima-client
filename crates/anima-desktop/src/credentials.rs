//! Native passwords only: the profile JSON contains usernames and references.
use anima_net::launcher::PasswordVault;
use std::sync::Arc;

pub fn native_vault() -> Option<Arc<dyn PasswordVault>> {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        Some(Arc::new(OsVault))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        None
    }
}
#[cfg(any(target_os = "macos", target_os = "windows"))]
struct OsVault;
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn entry(key: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new("dev.anima.client.accounts", key)
        .map_err(|_| "The system password store could not be opened.".into())
}
#[cfg(any(target_os = "macos", target_os = "windows"))]
impl PasswordVault for OsVault {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        match entry(key)?.get_password() {
            Ok(password) => Ok(Some(password)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err("The system password store is locked or unavailable. Unlock it, or enter your password manually.".into()),
        }
    }
    fn set(&self, key: &str, password: &str) -> Result<(), String> {
        entry(key)?.set_password(password).map_err(|_| "Password could not be saved in the system password store. Account changes have not been saved.".into())
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        match entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(
                "Could not remove the saved password. Unlock the system password store and retry."
                    .into(),
            ),
        }
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "windows")))]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Creates and removes one isolated test credential in the real OS vault"]
    fn native_vault_roundtrip() {
        let key = format!(
            "anima-qa-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let vault = native_vault().unwrap();
        assert!(vault.get(&key).unwrap().is_none());
        vault.set(&key, "Anima-QA-only").unwrap();
        let read = vault.get(&key);
        let removed = vault.delete(&key);
        assert_eq!(read.unwrap().as_deref(), Some("Anima-QA-only"));
        removed.unwrap();
        assert!(vault.get(&key).unwrap().is_none());
    }
}
