//! Saved server/account profiles. Only non-secret metadata reaches disk or HTTP.
//! The desktop injects its OS password vault; library/browser users need none.
mod probe;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

pub trait PasswordVault: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>, String>;
    fn set(&self, key: &str, password: &str) -> Result<(), String>;
    fn delete(&self, key: &str) -> Result<(), String>;
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerProfile {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub shard: u16,
    pub notes: String,
    pub cache: Option<ServerInfo>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerInfo {
    pub checked_at: u64,
    pub reachable: bool,
    pub latency_ms: Option<u64>,
    pub reported_name: Option<String>,
    pub clients: Option<u64>,
    pub uptime_hours: Option<u64>,
    pub details_at: Option<u64>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AccountProfile {
    pub id: String,
    pub server_id: String,
    pub label: String,
    pub username: String,
    pub remember_password: bool,
    pub characters: Vec<CachedCharacter>,
    pub last_used: Option<u64>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct CachedCharacter {
    pub index: u8,
    pub name: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Profiles {
    version: u8,
    servers: Vec<ServerProfile>,
    accounts: Vec<AccountProfile>,
}
impl Default for Profiles {
    fn default() -> Self {
        Self {
            version: 1,
            servers: vec![],
            accounts: vec![],
        }
    }
}

/// Never serialized or Debug-formatted: a password is resolved only for login.
pub struct SavedLogin {
    pub host: String,
    pub port: u16,
    pub shard: u16,
    pub username: String,
    pub password: String,
}

pub struct LauncherStore {
    path: Option<PathBuf>,
    memory: Mutex<Profiles>,
    vault: Option<Arc<dyn PasswordVault>>,
    probing: Mutex<()>,
}
impl LauncherStore {
    pub fn memory() -> Self {
        Self {
            path: None,
            memory: Mutex::new(Profiles::default()),
            vault: None,
            probing: Mutex::new(()),
        }
    }
    pub fn open(path: PathBuf, vault: Option<Arc<dyn PasswordVault>>) -> Result<Self, String> {
        let store = Self {
            path: Some(path),
            memory: Mutex::new(Profiles::default()),
            vault,
            probing: Mutex::new(()),
        };
        store.access(false, |_| Ok(()))?;
        Ok(store)
    }
    /// Lock/re-read on every operation, so two Anima windows cannot overwrite
    /// one another's accounts. The replacement file is written atomically.
    fn access<T>(
        &self,
        write: bool,
        f: impl FnOnce(&mut Profiles) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut memory = self
            .memory
            .lock()
            .map_err(|_| "Profile storage is unavailable.")?;
        let mut lock_file = None;
        let mut data = if let Some(path) = &self.path {
            let parent = path.parent().ok_or("Invalid profile location.")?;
            fs::create_dir_all(parent).map_err(|_| "Cannot create the profile folder.")?;
            let lock = private_file(&path.with_extension("lock"), false)?;
            lock.lock().map_err(|_| "Cannot lock the profile file.")?;
            lock_file = Some(lock);
            match File::open(path) {
                Ok(file) => {
                    if file.metadata().map_err(|_| "Cannot read profiles.")?.len() > 1024 * 1024 {
                        return Err("Profile file is too large; it has been left unchanged.".into());
                    }
                    let data: Profiles = serde_json::from_reader(file).map_err(|_| {
                        "Cannot read profiles; the existing file has been left unchanged."
                    })?;
                    if data.version != 1 {
                        return Err("This profile file needs a newer Anima client.".into());
                    }
                    data
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Profiles::default(),
                Err(_) => return Err("Cannot read the profile file.".into()),
            }
        } else {
            memory.clone()
        };
        let result = f(&mut data)?;
        if write {
            if let Some(path) = &self.path {
                let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
                let mut file = private_file(&tmp, true)?;
                let bytes =
                    serde_json::to_vec_pretty(&data).map_err(|_| "Cannot encode profiles.")?;
                file.write_all(&bytes)
                    .and_then(|_| file.sync_all())
                    .map_err(|_| "Could not save profiles. Please retry.")?;
                drop(file);
                fs::rename(&tmp, path)
                    .map_err(|_| "Could not replace the profile file. Please retry.")?;
            }
            *memory = data;
        }
        drop(lock_file);
        Ok(result)
    }
    pub fn snapshot(&self) -> Result<Value, String> {
        self.access(false, |data| {
            Ok(json!({
                "version": 1, "persistent": self.path.is_some(), "passwords": self.vault.is_some(),
                "servers": data.servers, "accounts": data.accounts,
            }))
        })
    }
    pub fn command(&self, body: &Value) -> Result<Value, String> {
        let op = body["op"].as_str().ok_or("Choose a profile action.")?;
        if op == "refresh" {
            self.refresh(required(body, "id", 64)?)?;
            return self.snapshot();
        }
        self.access(true, |data| {
            match op {
                "save_server" => {
                    let id = identifier(body, "id")?.to_string();
                    let name = required(body, "name", 80)?.trim().to_string();
                    let host = normalized_host(required(body, "host", 253)?)?;
                    let port = number(body, "port", 1, 65535)? as u16;
                    let shard = number(body, "shard", 0, 65535)? as u16;
                    let notes = body["notes"].as_str().unwrap_or("").trim().to_string();
                    if notes.len() > 2000 {
                        return Err("Keep server notes under 2,000 bytes.".into());
                    }
                    let mut server = ServerProfile {
                        id: id.clone(),
                        name,
                        host,
                        port,
                        shard,
                        notes,
                        cache: None,
                    };
                    if let Some(old) = data.servers.iter_mut().find(|s| s.id == id) {
                        if same_endpoint(old, &server) {
                            server.cache = old.cache.clone();
                        } else {
                            for account in data.accounts.iter_mut().filter(|a| a.server_id == id) {
                                self.forget(old, account)?;
                                account.characters.clear();
                                account.last_used = None;
                            }
                        }
                        *old = server;
                    } else {
                        if data.servers.len() >= 100 {
                            return Err("You can save up to 100 servers.".into());
                        }
                        data.servers.push(server);
                    }
                }
                "save_account" => {
                    let id = identifier(body, "id")?.to_string();
                    let server_id = identifier(body, "server_id")?.to_string();
                    let server = data
                        .servers
                        .iter()
                        .find(|s| s.id == server_id)
                        .ok_or("Save the server first.")?;
                    let username = required(body, "username", 30)?.trim().to_string();
                    if !username.is_ascii() || username.chars().any(char::is_control) {
                        return Err(
                            "UO account names must use printable ASCII, up to 30 bytes.".into()
                        );
                    }
                    let label = required(body, "label", 80)?.trim().to_string();
                    let remember = body["remember_password"].as_bool().unwrap_or(false);
                    let password = body["password"].as_str().unwrap_or("");
                    validate_password(password)?;
                    if data
                        .accounts
                        .iter()
                        .any(|a| a.id != id && a.server_id == server_id && a.username == username)
                    {
                        return Err("That account is already saved on this server.".into());
                    }
                    let mut account = AccountProfile {
                        id: id.clone(),
                        server_id,
                        label,
                        username,
                        remember_password: false,
                        ..Default::default()
                    };
                    let old = data.accounts.iter().find(|a| a.id == id);
                    if old.is_none() && data.accounts.len() >= 500 {
                        return Err("You can save up to 500 accounts.".into());
                    }
                    let same_account = old.is_some_and(|a| {
                        a.server_id == account.server_id && a.username == account.username
                    });
                    // Validate before changing the vault, so an invalid rename
                    // cannot remove the existing account's password.
                    if remember {
                        if self.vault.is_none() {
                            return Err("Password saving is available in the desktop app.".into());
                        }
                        if password.is_empty()
                            && !(same_account && old.is_some_and(|a| a.remember_password))
                        {
                            return Err("Enter the password you want to save.".into());
                        }
                    }
                    if let Some(old) = old {
                        if old.server_id == account.server_id && old.username == account.username {
                            account.characters = old.characters.clone();
                            account.last_used = old.last_used;
                            account.remember_password = old.remember_password;
                        }
                    }
                    if remember {
                        let vault = self
                            .vault
                            .as_ref()
                            .ok_or("Password saving is available in the desktop app.")?;
                        if !password.is_empty() {
                            vault.set(&credential_key(server, &account), password)?;
                        } else if !account.remember_password {
                            return Err("Enter the password you want to save.".into());
                        }
                        account.remember_password = true;
                    } else {
                        self.forget(server, &mut account)?;
                    }
                    // A failed save for a renamed account must leave the old
                    // credential usable. Remove it only after the new one exists.
                    if !same_account {
                        if let Some(old) = old {
                            if let Some(old_server) =
                                data.servers.iter().find(|s| s.id == old.server_id)
                            {
                                if let Err(error) = self.forget(old_server, &mut old.clone()) {
                                    let _ = self.forget(server, &mut account);
                                    return Err(error);
                                }
                            }
                        }
                    }
                    if let Some(old) = data.accounts.iter_mut().find(|a| a.id == id) {
                        *old = account;
                    } else {
                        data.accounts.push(account);
                    }
                }
                "delete_account" => {
                    let id = identifier(body, "id")?;
                    if let Some(account) = data.accounts.iter().find(|a| a.id == id) {
                        if let Some(server) =
                            data.servers.iter().find(|s| s.id == account.server_id)
                        {
                            self.forget(server, &mut account.clone())?;
                        }
                    }
                    data.accounts.retain(|a| a.id != id);
                }
                "delete_server" => {
                    let id = identifier(body, "id")?;
                    if let Some(server) = data.servers.iter().find(|s| s.id == id) {
                        for account in data.accounts.iter().filter(|a| a.server_id == id) {
                            self.forget(server, &mut account.clone())?;
                        }
                    }
                    data.accounts.retain(|a| a.server_id != id);
                    data.servers.retain(|s| s.id != id);
                }
                _ => return Err("Unknown profile action.".into()),
            }
            Ok(())
        })?;
        self.snapshot()
    }
    fn forget(&self, server: &ServerProfile, account: &mut AccountProfile) -> Result<(), String> {
        if account.remember_password {
            self.vault
                .as_ref()
                .ok_or("Open the desktop app to remove this saved password.")?
                .delete(&credential_key(server, account))?;
        }
        account.remember_password = false;
        Ok(())
    }
    pub fn resolve_login(&self, id: &str, typed_password: &str) -> Result<SavedLogin, String> {
        validate_password(typed_password)?;
        self.access(false, |data| {
            let account = data
                .accounts
                .iter()
                .find(|a| a.id == id)
                .ok_or("The saved account no longer exists.")?;
            let server = data
                .servers
                .iter()
                .find(|s| s.id == account.server_id)
                .ok_or("The saved server no longer exists.")?;
            let password = if typed_password.is_empty() && account.remember_password {
                self.vault
                    .as_ref()
                    .ok_or("Open the desktop app to use this saved password.")?
                    .get(&credential_key(server, account))?
                    .ok_or("Saved password is missing. Enter it again and save the account.")?
            } else {
                typed_password.to_string()
            };
            Ok(SavedLogin {
                host: server.host.clone(),
                port: server.port,
                shard: server.shard,
                username: account.username.clone(),
                password,
            })
        })
    }
    pub fn cache_characters(
        &self,
        id: &str,
        host: &str,
        port: u16,
        shard: u16,
        username: &str,
        slots: Vec<CachedCharacter>,
    ) -> Result<(), String> {
        self.access(true, |data| {
            if let Some(account) = data
                .accounts
                .iter_mut()
                .find(|a| a.id == id && a.username == username)
            {
                if data.servers.iter().any(|s| {
                    s.id == account.server_id
                        && s.host == host
                        && s.port == port
                        && s.shard == shard
                }) {
                    account.characters = slots;
                    account.last_used = Some(now());
                }
            }
            Ok(())
        })
    }
    fn refresh(&self, id: &str) -> Result<(), String> {
        let _guard = self
            .probing
            .try_lock()
            .map_err(|_| "A server check is already running.")?;
        let server = self.access(false, |data| {
            data.servers
                .iter()
                .find(|s| s.id == id)
                .cloned()
                .ok_or("Save this server before checking it.".into())
        })?;
        if server
            .cache
            .as_ref()
            .is_some_and(|c| now().saturating_sub(c.checked_at) < 5000)
        {
            return Ok(());
        }
        let mut info = probe::check(&server.host, server.port);
        // A failed/unsupported status query must not erase the last known details.
        if info.details_at.is_none() {
            if let Some(old) = &server.cache {
                info.reported_name = old.reported_name.clone();
                info.clients = old.clients;
                info.uptime_hours = old.uptime_hours;
                info.details_at = old.details_at;
            }
        }
        self.access(true, |data| {
            if let Some(current) = data
                .servers
                .iter_mut()
                .find(|s| s.id == id && same_endpoint(s, &server))
            {
                current.cache = Some(info);
            }
            Ok(())
        })
    }
}
fn private_file(path: &Path, truncate: bool) -> Result<File, String> {
    let mut opts = OpenOptions::new();
    opts.read(true).write(true).create(true).truncate(truncate);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)
        .map_err(|_| "Cannot open the profile file for writing.".into())
}
fn required<'a>(v: &'a Value, key: &str, max: usize) -> Result<&'a str, String> {
    v[key]
        .as_str()
        .filter(|s| !s.trim().is_empty() && s.len() <= max)
        .ok_or_else(|| format!("Enter a valid {key} (up to {max} bytes)."))
}
fn identifier<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    let id = required(v, key, 64)?;
    if !id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err("Invalid profile identifier.".into());
    }
    Ok(id)
}
fn number(v: &Value, key: &str, min: u64, max: u64) -> Result<u64, String> {
    v[key]
        .as_u64()
        .filter(|n| (min..=max).contains(n))
        .ok_or_else(|| format!("{key} must be between {min} and {max}."))
}
fn normalized_host(host: &str) -> Result<String, String> {
    let host = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    if host.is_empty()
        || !host
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-' | b':'))
    {
        return Err("Enter a hostname or IP address without a URL or port.".into());
    }
    Ok(host)
}
fn validate_password(password: &str) -> Result<(), String> {
    if password.len() > 30 || !password.is_ascii() || password.chars().any(char::is_control) {
        return Err("UO passwords must use printable ASCII, up to 30 bytes.".into());
    }
    Ok(())
}
fn credential_key(server: &ServerProfile, account: &AccountProfile) -> String {
    json!([
        server.host,
        server.port,
        server.shard,
        account.username,
        account.id
    ])
    .to_string()
}
fn same_endpoint(a: &ServerProfile, b: &ServerProfile) -> bool {
    a.host == b.host && a.port == b.port && a.shard == b.shard
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests;
