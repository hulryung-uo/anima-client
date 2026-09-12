use super::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[derive(Default)]
struct Vault {
    secrets: Mutex<HashMap<String, String>>,
    fail: AtomicBool,
}
impl PasswordVault for Vault {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        Ok(self.secrets.lock().unwrap().get(key).cloned())
    }
    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        if self.fail.load(Ordering::Relaxed) {
            return Err("vault locked".into());
        }
        self.secrets
            .lock()
            .unwrap()
            .insert(key.into(), value.into());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        self.secrets.lock().unwrap().remove(key);
        Ok(())
    }
}
struct Folder(PathBuf);
impl Folder {
    fn new() -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "anima-profiles-test-{}-{}-{}",
            std::process::id(),
            now(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn file(&self) -> PathBuf {
        self.0.join("launcher.json")
    }
}
impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn server(id: &str, host: &str) -> Value {
    json!({"op":"save_server","id":id,"name":"My shard","host":host,"port":2594,"shard":0,"notes":"Private test shard"})
}
fn account(id: &str, server: &str, password: &str) -> Value {
    json!({"op":"save_account","id":id,"server_id":server,"label":"Adventurer","username":"player","password":password,"remember_password":true})
}

#[test]
fn multiple_servers_accounts_restart_and_password_is_never_in_profiles() {
    let folder = Folder::new();
    let vault = Arc::new(Vault::default());
    let store = LauncherStore::open(folder.file(), Some(vault.clone())).unwrap();
    store.command(&server("one", "localhost")).unwrap();
    store.command(&server("two", "127.0.0.1")).unwrap();
    store.command(&account("a", "one", "private-one")).unwrap();
    store.command(&account("b", "two", "private-two")).unwrap();
    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot["servers"].as_array().unwrap().len(), 2);
    assert!(!snapshot.to_string().contains("private-one"));
    assert!(!fs::read_to_string(folder.file())
        .unwrap()
        .contains("private-two"));
    let restarted = LauncherStore::open(folder.file(), Some(vault)).unwrap();
    assert_eq!(
        restarted.resolve_login("a", "").unwrap().password,
        "private-one"
    );
    assert_eq!(
        restarted.resolve_login("b", "").unwrap().password,
        "private-two"
    );
    assert_eq!(
        restarted.resolve_login("a", "temporary").unwrap().password,
        "temporary"
    );
    assert_eq!(
        restarted.resolve_login("a", "").unwrap().password,
        "private-one"
    );
}
#[test]
fn invalid_account_rename_preserves_saved_password() {
    let folder = Folder::new();
    let vault = Arc::new(Vault::default());
    let store = LauncherStore::open(folder.file(), Some(vault.clone())).unwrap();
    store.command(&server("one", "localhost")).unwrap();
    store.command(&account("a", "one", "secret")).unwrap();
    let mut renamed = account("a", "one", "");
    renamed["username"] = json!("renamed");
    assert!(store.command(&renamed).is_err());
    assert_eq!(store.resolve_login("a", "").unwrap().password, "secret");
    vault.fail.store(true, Ordering::Relaxed);
    renamed["password"] = json!("replacement");
    assert!(store.command(&renamed).is_err());
    assert_eq!(store.resolve_login("a", "").unwrap().password, "secret");
}

#[test]
fn endpoint_change_forgets_password_and_character_cache() {
    let folder = Folder::new();
    let vault = Arc::new(Vault::default());
    let store = LauncherStore::open(folder.file(), Some(vault.clone())).unwrap();
    store.command(&server("one", "localhost")).unwrap();
    store.command(&account("a", "one", "secret")).unwrap();
    store
        .cache_characters(
            "a",
            "localhost",
            2594,
            0,
            "player",
            vec![CachedCharacter {
                index: 2,
                name: "Aria".into(),
            }],
        )
        .unwrap();
    store.command(&server("one", "example.com")).unwrap();
    assert!(vault.secrets.lock().unwrap().is_empty());
    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot["accounts"][0]["remember_password"], false);
    assert_eq!(snapshot["accounts"][0]["characters"], json!([]));
    assert!(store.resolve_login("a", "").unwrap().password.is_empty());
}
#[test]
fn delete_server_removes_only_its_accounts_and_passwords() {
    let folder = Folder::new();
    let vault = Arc::new(Vault::default());
    let store = LauncherStore::open(folder.file(), Some(vault.clone())).unwrap();
    for id in ["one", "two"] {
        store.command(&server(id, id)).unwrap();
        store.command(&account(id, id, id)).unwrap();
    }
    store
        .command(&json!({"op":"delete_server","id":"one"}))
        .unwrap();
    assert_eq!(
        store.snapshot().unwrap()["accounts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(vault.secrets.lock().unwrap().len(), 1);
    assert_eq!(store.resolve_login("two", "").unwrap().password, "two");
}
#[test]
fn vault_failure_does_not_claim_password_was_saved() {
    let folder = Folder::new();
    let vault = Arc::new(Vault::default());
    vault.fail.store(true, Ordering::Relaxed);
    let store = LauncherStore::open(folder.file(), Some(vault)).unwrap();
    store.command(&server("one", "localhost")).unwrap();
    assert!(store.command(&account("a", "one", "secret")).is_err());
    assert_eq!(store.snapshot().unwrap()["accounts"], json!([]));
}
#[test]
fn concurrent_windows_reread_profiles_before_writing() {
    let folder = Folder::new();
    let one = LauncherStore::open(folder.file(), None).unwrap();
    let two = LauncherStore::open(folder.file(), None).unwrap();
    one.command(&server("one", "localhost")).unwrap();
    two.command(&server("two", "localhost")).unwrap();
    assert_eq!(
        one.snapshot().unwrap()["servers"].as_array().unwrap().len(),
        2
    );
}
#[test]
fn corrupt_or_future_files_are_left_unchanged() {
    let folder = Folder::new();
    for text in ["broken", r#"{"version":99,"servers":[],"accounts":[]}"#] {
        fs::write(folder.file(), text).unwrap();
        assert!(LauncherStore::open(folder.file(), None).is_err());
        assert_eq!(fs::read_to_string(folder.file()).unwrap(), text);
    }
}
#[test]
fn metadata_updates_keep_password_and_unchecking_removes_it() {
    let folder = Folder::new();
    let vault = Arc::new(Vault::default());
    let store = LauncherStore::open(folder.file(), Some(vault.clone())).unwrap();
    store.command(&server("one", "localhost")).unwrap();
    store.command(&account("a", "one", "secret")).unwrap();
    let mut update = account("a", "one", "");
    update["label"] = json!("Crafter");
    store.command(&update).unwrap();
    assert_eq!(store.resolve_login("a", "").unwrap().password, "secret");
    update["remember_password"] = json!(false);
    store.command(&update).unwrap();
    assert!(vault.secrets.lock().unwrap().is_empty());
}
#[test]
fn unsupported_password_storage_and_invalid_fields_are_explicit() {
    let store = LauncherStore::memory();
    assert!(store
        .command(&server("one", "https://example.com/path"))
        .is_err());
    store.command(&server("one", "localhost")).unwrap();
    assert!(store.command(&account("a", "one", "secret")).is_err());
    let mut plain = account("a", "one", "secret");
    plain["remember_password"] = json!(false);
    store.command(&plain).unwrap();
    assert!(!store.snapshot().unwrap().to_string().contains("secret"));
    assert!(store.command(&plain).is_ok());
    plain["id"] = json!("duplicate");
    assert!(store.command(&plain).is_err());
}
