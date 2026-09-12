use super::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[derive(Default)]
struct Vault {
    secrets: Mutex<HashMap<String, String>>,
    fail: AtomicBool,
    fail_delete: Mutex<Option<String>>,
    remove_staging_on_set: Mutex<Option<PathBuf>>,
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
        if let Some(path) = self.remove_staging_on_set.lock().unwrap().take() {
            fs::remove_file(path).unwrap();
        }
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        if self.fail_delete.lock().unwrap().as_deref() == Some(key) {
            return Err("vault deletion failed".into());
        }
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

#[test]
fn moving_account_between_equivalent_servers_keeps_its_credential() {
    let folder = Folder::new();
    let vault = Arc::new(Vault::default());
    let store = LauncherStore::open(folder.file(), Some(vault)).unwrap();
    store.command(&server("one", "localhost")).unwrap();
    store.command(&server("two", "localhost")).unwrap();
    store.command(&account("a", "one", "original")).unwrap();
    store.command(&account("a", "two", "replacement")).unwrap();
    assert_eq!(
        store.resolve_login("a", "").unwrap().password,
        "replacement"
    );
    store.command(&account("a", "one", "")).unwrap();
    assert_eq!(
        store.resolve_login("a", "").unwrap().password,
        "replacement"
    );
}

#[test]
fn profile_write_failure_preserves_passwords_and_profile_file() {
    let folder = Folder::new();
    let vault = Arc::new(Vault::default());
    let store = LauncherStore::open(folder.file(), Some(vault.clone())).unwrap();
    store.command(&server("one", "localhost")).unwrap();
    store.command(&account("a", "one", "original")).unwrap();
    let before = fs::read(folder.file()).unwrap();
    // Deterministically prevent staging a file, without chmod/root assumptions.
    let staging = folder
        .file()
        .with_extension(format!("{}.tmp", std::process::id()));
    fs::create_dir(&staging).unwrap();
    let mut rename = account("a", "one", "replacement");
    rename["username"] = json!("renamed");
    for operation in [
        account("a", "one", "replacement"),
        rename,
        server("one", "changed.test"),
        json!({"op":"delete_server","id":"one"}),
    ] {
        assert!(store.command(&operation).is_err());
        assert_eq!(fs::read(folder.file()).unwrap(), before);
        assert_eq!(store.resolve_login("a", "").unwrap().password, "original");
        assert_eq!(vault.secrets.lock().unwrap().len(), 1);
    }
}

#[test]
fn failed_multi_account_delete_restores_passwords_already_removed() {
    let folder = Folder::new();
    let vault = Arc::new(Vault::default());
    let store = LauncherStore::open(folder.file(), Some(vault.clone())).unwrap();
    store.command(&server("one", "localhost")).unwrap();
    store.command(&account("a", "one", "first-secret")).unwrap();
    let mut second = account("b", "one", "second-secret");
    second["username"] = json!("crafter");
    store.command(&second).unwrap();
    *vault.fail_delete.lock().unwrap() = Some(
        vault
            .secrets
            .lock()
            .unwrap()
            .iter()
            .find(|(_, password)| *password == "second-secret")
            .unwrap()
            .0
            .clone(),
    );
    let before = fs::read(folder.file()).unwrap();
    assert!(store
        .command(&json!({"op":"delete_server","id":"one"}))
        .is_err());
    assert_eq!(fs::read(folder.file()).unwrap(), before);
    assert_eq!(
        store.resolve_login("a", "").unwrap().password,
        "first-secret"
    );
    assert_eq!(
        store.resolve_login("b", "").unwrap().password,
        "second-secret"
    );
}

#[test]
fn failed_final_file_replace_undoes_a_password_change() {
    let folder = Folder::new();
    let vault = Arc::new(Vault::default());
    let store = LauncherStore::open(folder.file(), Some(vault.clone())).unwrap();
    store.command(&server("one", "localhost")).unwrap();
    store.command(&account("a", "one", "original")).unwrap();
    let before = fs::read(folder.file()).unwrap();
    let staging = folder
        .file()
        .with_extension(format!("{}.tmp", std::process::id()));
    *vault.remove_staging_on_set.lock().unwrap() = Some(staging.clone());
    let error = store
        .command(&account("a", "one", "replacement"))
        .unwrap_err();
    assert!(error.contains("replace the profile file"));
    assert_eq!(fs::read(folder.file()).unwrap(), before);
    assert_eq!(store.resolve_login("a", "").unwrap().password, "original");
    assert!(!staging.exists());
}

#[test]
fn failed_password_rollback_reports_how_to_recover() {
    let folder = Folder::new();
    let vault = Arc::new(Vault::default());
    let store = LauncherStore::open(folder.file(), Some(vault.clone())).unwrap();
    store.command(&server("one", "localhost")).unwrap();
    store.command(&account("a", "one", "first-secret")).unwrap();
    let mut second = account("b", "one", "second-secret");
    second["username"] = json!("crafter");
    store.command(&second).unwrap();
    *vault.fail_delete.lock().unwrap() = Some(
        vault
            .secrets
            .lock()
            .unwrap()
            .iter()
            .find(|(_, password)| *password == "second-secret")
            .unwrap()
            .0
            .clone(),
    );
    vault.fail.store(true, Ordering::Relaxed);
    let before = fs::read(folder.file()).unwrap();
    let error = store
        .command(&json!({"op":"delete_server","id":"one"}))
        .unwrap_err();
    assert!(error.contains("Some saved passwords could not be restored"));
    assert!(error.contains("enter and save"));
    assert!(!error.contains("first-secret") && !error.contains("second-secret"));
    assert_eq!(fs::read(folder.file()).unwrap(), before);
    assert!(store.resolve_login("a", "").is_err());
    assert_eq!(
        store.resolve_login("b", "").unwrap().password,
        "second-secret"
    );
}

#[test]
fn oversized_character_cache_cannot_make_saved_profiles_unreadable() {
    let folder = Folder::new();
    let store = LauncherStore::open(folder.file(), None).unwrap();
    store.command(&server("one", "localhost")).unwrap();
    let mut plain = account("a", "one", "");
    plain["remember_password"] = json!(false);
    store.command(&plain).unwrap();
    let before = fs::read(folder.file()).unwrap();
    let error = store
        .cache_characters(
            "a",
            "localhost",
            2594,
            0,
            "player",
            vec![CachedCharacter {
                index: 0,
                name: "x".repeat(MAX_PROFILE_BYTES),
            }],
        )
        .unwrap_err();
    assert!(error.contains("Profile storage is full"));
    assert_eq!(fs::read(folder.file()).unwrap(), before);
    assert!(LauncherStore::open(folder.file(), None)
        .unwrap()
        .resolve_login("a", "")
        .is_ok());
}

#[test]
fn portable_backup_restores_worlds_and_aliases_without_passwords_or_stale_caches() {
    let folder = Folder::new();
    let vault = Arc::new(Vault::default());
    let original = LauncherStore::open(folder.file(), Some(vault.clone())).unwrap();
    original.command(&server("one", "localhost")).unwrap();
    let mut alias = server("two", "localhost");
    alias["name"] = json!("Another group");
    original.command(&alias).unwrap();
    original
        .command(&account("a", "one", "backup-secret"))
        .unwrap();
    original
        .command(&account("b", "two", "second-secret"))
        .unwrap();
    original
        .cache_characters(
            "a",
            "localhost",
            2594,
            0,
            "player",
            vec![CachedCharacter {
                index: 0,
                name: "Old character".into(),
            }],
        )
        .unwrap();
    let backup = original.command(&json!({"op":"export"})).unwrap();
    let text = backup.to_string();
    for excluded in [
        "backup-secret",
        "second-secret",
        "remember_password",
        "characters",
        "Old character",
        "last_used",
        "cache",
        "\"id\"",
    ] {
        assert!(
            !text.contains(excluded),
            "unexpected export field: {excluded}"
        );
    }
    let destination = Folder::new();
    let restored = LauncherStore::open(destination.file(), Some(vault.clone())).unwrap();
    let result = restored
        .command(&json!({"op":"import", "backup":backup}))
        .unwrap();
    assert_eq!(result["imported"], json!({"servers":2,"accounts":2}));
    let accounts = result["accounts"].as_array().unwrap();
    for entry in accounts {
        assert_eq!(entry["remember_password"], false);
        assert_eq!(entry["characters"], json!([]));
        assert_eq!(
            restored
                .resolve_login(entry["id"].as_str().unwrap(), "")
                .unwrap()
                .password,
            ""
        );
    }
    let again = restored.command(&json!({"op":"export"})).unwrap();
    assert_eq!(again, backup);
    assert_eq!(
        original.resolve_login("a", "").unwrap().password,
        "backup-secret"
    );
}

#[test]
fn importing_merges_latest_profiles_and_preserves_existing_credentials_and_notes() {
    let folder = Folder::new();
    let vault = Arc::new(Vault::default());
    let store = LauncherStore::open(folder.file(), Some(vault.clone())).unwrap();
    store.command(&server("one", "localhost")).unwrap();
    store.command(&account("a", "one", "keep-secret")).unwrap();
    let mut backup = store.command(&json!({"op":"export"})).unwrap();
    backup["servers"][0]["notes"] = json!("outdated note");
    backup["servers"][0]["accounts"][0]["label"] = json!("outdated label");
    backup["servers"][0]["accounts"]
        .as_array_mut()
        .unwrap()
        .push(json!({"username":"crafter", "label":"Crafter"}));
    let other_window = LauncherStore::open(folder.file(), Some(vault.clone())).unwrap();
    other_window
        .command(&server("elsewhere", "example.test"))
        .unwrap();
    vault.fail.store(true, Ordering::Relaxed);
    let result = store
        .command(&json!({"op":"import", "backup":backup}))
        .unwrap();
    assert_eq!(result["imported"], json!({"servers":0,"accounts":1}));
    assert_eq!(result["servers"].as_array().unwrap().len(), 2);
    assert_eq!(result["servers"][0]["notes"], "Private test shard");
    assert_eq!(result["accounts"][0]["label"], "Adventurer");
    assert_eq!(
        store.resolve_login("a", "").unwrap().password,
        "keep-secret"
    );
    let result = store
        .command(&json!({"op":"import", "backup":backup}))
        .unwrap();
    assert_eq!(result["imported"], json!({"servers":0,"accounts":0}));
}

#[test]
fn invalid_or_unwritable_import_is_atomic_and_does_not_change_credentials() {
    let folder = Folder::new();
    let vault = Arc::new(Vault::default());
    let store = LauncherStore::open(folder.file(), Some(vault.clone())).unwrap();
    store.command(&server("one", "localhost")).unwrap();
    store.command(&account("a", "one", "keep-secret")).unwrap();
    let backup = store.command(&json!({"op":"export"})).unwrap();
    let before = fs::read(folder.file()).unwrap();
    for (key, value) in [("version", json!(99)), ("password", json!("never-import"))] {
        let mut invalid = backup.clone();
        invalid[key] = value;
        assert!(store
            .command(&json!({"op":"import", "backup":invalid}))
            .is_err());
    }
    let mut invalid = backup.clone();
    invalid["servers"][0]["accounts"]
        .as_array_mut()
        .unwrap()
        .push(json!({"username":"valid-first", "label":"New"}));
    invalid["servers"][0]["accounts"]
        .as_array_mut()
        .unwrap()
        .push(json!({"username":"bad\nname", "label":"Broken"}));
    assert!(store
        .command(&json!({"op":"import", "backup":invalid}))
        .is_err());
    let staging = folder
        .file()
        .with_extension(format!("{}.tmp", std::process::id()));
    fs::create_dir(&staging).unwrap();
    assert!(store
        .command(&json!({"op":"import", "backup":backup}))
        .is_err());
    assert_eq!(fs::read(folder.file()).unwrap(), before);
    assert_eq!(
        store.resolve_login("a", "").unwrap().password,
        "keep-secret"
    );
}

#[test]
fn corrupt_profiles_can_be_recovered_without_preventing_startup_or_erasing_vault_entries() {
    let folder = Folder::new();
    let vault = Arc::new(Vault::default());
    let store = LauncherStore::open(folder.file(), Some(vault.clone())).unwrap();
    store.command(&server("one", "localhost")).unwrap();
    store
        .command(&account("a", "one", "keep-in-vault"))
        .unwrap();
    let secrets = vault.secrets.lock().unwrap().clone();
    for bytes in [
        b"{broken\n\xff".as_slice(),
        br#"{"version":99,"servers":[],"accounts":[]}"#,
    ] {
        fs::write(folder.file(), bytes).unwrap();
        let reopened = LauncherStore::recoverable(folder.file(), Some(vault.clone()));
        assert!(reopened.snapshot().is_err());
        assert!(reopened.can_recover());
        let result = reopened.command(&json!({"op":"recover"})).unwrap();
        let copy = Path::new(result["recovery_copy"].as_str().unwrap());
        assert_eq!(fs::read(copy).unwrap(), bytes);
        assert_eq!(result["servers"], json!([]));
        assert!(!reopened.can_recover());
        assert!(
            reopened.command(&json!({"op":"recover"})).is_err(),
            "a readable library must never be reset"
        );
        assert_eq!(*vault.secrets.lock().unwrap(), secrets);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(copy).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}

#[test]
fn failed_recovery_and_changed_readable_profiles_keep_the_original() {
    let folder = Folder::new();
    let bytes = b"unreadable original";
    fs::write(folder.file(), bytes).unwrap();
    let store = LauncherStore::recoverable(folder.file(), None);
    let staging = folder
        .file()
        .with_extension(format!("{}.tmp", std::process::id()));
    fs::create_dir(&staging).unwrap();
    assert!(store.command(&json!({"op":"recover"})).is_err());
    assert_eq!(fs::read(folder.file()).unwrap(), bytes);
    fs::remove_dir(staging).unwrap();
    let readable = br#"{"version":1,"servers":[],"accounts":[]}"#;
    fs::write(folder.file(), readable).unwrap();
    assert!(store.command(&json!({"op":"recover"})).is_err());
    assert_eq!(fs::read(folder.file()).unwrap(), readable);
}

#[test]
fn semantically_invalid_profiles_offer_recovery_instead_of_broken_login() {
    let folder = Folder::new();
    let bytes = br#"{"version":1,"servers":[],"accounts":[{"id":"a","server_id":"missing","label":"Main","username":"player"}]}"#;
    fs::write(folder.file(), bytes).unwrap();
    let store = LauncherStore::recoverable(folder.file(), None);
    assert!(store.snapshot().is_err());
    assert!(store.can_recover());
    assert_eq!(fs::read(folder.file()).unwrap(), bytes);
}
