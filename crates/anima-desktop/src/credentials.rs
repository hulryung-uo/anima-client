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

    struct ProfileFixture {
        directory: std::path::PathBuf,
        key: String,
        vault: Arc<dyn PasswordVault>,
    }
    impl Drop for ProfileFixture {
        fn drop(&mut self) {
            let _ = self.vault.delete(&self.key);
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    #[ignore = "Creates and removes one isolated profile and credential in the real OS vault"]
    fn native_profile_password_lifecycle() {
        use anima_net::launcher::LauncherStore;
        use serde_json::json;
        let id = format!(
            "anima-qa-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let vault = native_vault().unwrap();
        let key = json!(["localhost", 2594, 0, "anima-qa", id]).to_string();
        assert!(vault.get(&key).unwrap().is_none());
        let fixture = ProfileFixture {
            directory: std::env::temp_dir().join(&id),
            key,
            vault: vault.clone(),
        };
        std::fs::create_dir(&fixture.directory).unwrap();
        let path = fixture.directory.join("launcher.json");
        let store = LauncherStore::open(path.clone(), Some(vault.clone())).unwrap();
        for server_id in ["one", "two"] {
            store.command(&json!({"op":"save_server","id":server_id,"name":"QA only","host":"localhost","port":2594,"shard":0})).unwrap();
        }
        let mut account = json!({"op":"save_account","id":id,"server_id":"one","label":"QA only","username":"anima-qa","password":"Anima-QA-first","remember_password":true});
        store.command(&account).unwrap();
        assert_eq!(
            store.resolve_login(&id, "").unwrap().password,
            "Anima-QA-first"
        );
        let before = std::fs::read(&path).unwrap();
        let staging = path.with_extension(format!("{}.tmp", std::process::id()));
        std::fs::create_dir(&staging).unwrap();
        account["password"] = json!("Anima-QA-second");
        assert!(store.command(&account).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(
            store.resolve_login(&id, "").unwrap().password,
            "Anima-QA-first"
        );
        std::fs::remove_dir(staging).unwrap();
        account["server_id"] = json!("two");
        store.command(&account).unwrap();
        let restarted = LauncherStore::open(path.clone(), Some(vault.clone())).unwrap();
        assert_eq!(
            restarted.resolve_login(&id, "").unwrap().password,
            "Anima-QA-second"
        );
        let profile = std::fs::read_to_string(path).unwrap();
        assert!(!profile.contains("Anima-QA-first") && !profile.contains("Anima-QA-second"));
        restarted
            .command(&json!({"op":"delete_account","id":id}))
            .unwrap();
        assert!(vault.get(&fixture.key).unwrap().is_none());
    }
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
