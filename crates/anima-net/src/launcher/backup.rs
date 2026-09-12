//! Portable, non-secret profiles and explicit recovery of an unreadable file.
use super::*;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Backup {
    format: String,
    version: u8,
    servers: Vec<World>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct World {
    name: String,
    host: String,
    port: u16,
    shard: u16,
    notes: String,
    // A browser backup can carry its relay. Native TCP uses host/port/shard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    relay: Option<String>,
    accounts: Vec<Account>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Account {
    label: String,
    username: String,
}
fn check_text(value: &str, max: usize) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > max {
        return Err("A profile field is empty or too long.".into());
    }
    Ok(())
}
fn check_username(username: &str) -> Result<(), String> {
    check_text(username, 30)?;
    if !username.is_ascii() || username.chars().any(char::is_control) {
        return Err("UO account names must use printable ASCII, up to 30 bytes.".into());
    }
    Ok(())
}
fn check_server(server: &ServerProfile) -> Result<(), String> {
    check_text(&server.name, 80)?;
    check_text(&server.host, 253)?;
    normalized_host(&server.host)?;
    if server.port == 0 || server.notes.len() > 2000 {
        return Err("A server has an invalid port or oversized notes.".into());
    }
    Ok(())
}
pub(super) fn validate_profiles(data: &Profiles) -> Result<(), String> {
    if data.servers.len() > 100 || data.accounts.len() > 500 {
        return Err("Profiles exceed the limit of 100 servers or 500 accounts.".into());
    }
    let mut servers = HashSet::new();
    for server in &data.servers {
        identifier(&json!({"id": server.id}), "id")?;
        check_server(server)?;
        if !servers.insert(server.id.as_str()) {
            return Err("The profile file contains duplicate server identifiers.".into());
        }
    }
    let mut ids = HashSet::new();
    let mut usernames = HashSet::new();
    for account in &data.accounts {
        identifier(&json!({"id": account.id}), "id")?;
        check_text(&account.label, 80)?;
        check_username(&account.username)?;
        if !servers.contains(account.server_id.as_str())
            || !ids.insert(account.id.as_str())
            || !usernames.insert((&account.server_id, &account.username))
        {
            return Err("The profile file contains missing servers or duplicate accounts.".into());
        }
    }
    Ok(())
}
fn unique() -> String {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}-{}",
        now(),
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}
fn next_id(data: &Profiles) -> String {
    loop {
        let id = format!("import-{}", unique());
        if !data.servers.iter().any(|s| s.id == id) && !data.accounts.iter().any(|a| a.id == id) {
            return id;
        }
    }
}
impl LauncherStore {
    pub(super) fn export_backup(&self) -> Result<Value, String> {
        self.access(false, |data| {
            let backup = Backup {
                format: "anima-worlds".into(),
                version: 1,
                servers: data
                    .servers
                    .iter()
                    .map(|s| World {
                        name: s.name.clone(),
                        host: s.host.clone(),
                        port: s.port,
                        shard: s.shard,
                        notes: s.notes.clone(),
                        relay: None,
                        accounts: data
                            .accounts
                            .iter()
                            .filter(|a| a.server_id == s.id)
                            .map(|a| Account {
                                label: a.label.clone(),
                                username: a.username.clone(),
                            })
                            .collect(),
                    })
                    .collect(),
            };
            serde_json::to_value(backup).map_err(|_| "Cannot encode the profile backup.".into())
        })
    }
    pub(super) fn import_backup(&self, value: &Value) -> Result<Value, String> {
        let backup: Backup = serde_json::from_value(value.clone())
            .map_err(|_| "Choose a valid Anima worlds backup. Password fields are not accepted.")?;
        if backup.format != "anima-worlds" || backup.version != 1 {
            return Err("This is not a supported Anima worlds backup.".into());
        }
        if backup.servers.len() > 100
            || backup
                .servers
                .iter()
                .map(|s| s.accounts.len())
                .sum::<usize>()
                > 500
        {
            return Err("A backup can contain up to 100 servers and 500 accounts.".into());
        }
        let (servers, accounts) = self.access(true, |data| {
            let before = (data.servers.len(), data.accounts.len());
            for world in &backup.servers {
                let mut server = ServerProfile {
                    id: next_id(data),
                    name: world.name.trim().into(),
                    host: normalized_host(&world.host)?,
                    port: world.port,
                    shard: world.shard,
                    notes: world.notes.clone(),
                    cache: None,
                };
                check_server(&server)?;
                // Keep the existing server's name, notes, cache and credentials.
                if let Some(existing) = data
                    .servers
                    .iter()
                    .find(|s| same_endpoint(s, &server) && s.name == server.name)
                {
                    server.id = existing.id.clone();
                } else {
                    data.servers.push(server.clone());
                }
                for entry in &world.accounts {
                    check_text(&entry.label, 80)?;
                    check_username(&entry.username)?;
                    let username = entry.username.trim();
                    if data
                        .accounts
                        .iter()
                        .any(|a| a.server_id == server.id && a.username == username)
                    {
                        continue;
                    }
                    data.accounts.push(AccountProfile {
                        id: next_id(data),
                        server_id: server.id.clone(),
                        label: entry.label.trim().into(),
                        username: username.into(),
                        ..Default::default()
                    });
                }
            }
            validate_profiles(data)?;
            Ok((
                data.servers.len() - before.0,
                data.accounts.len() - before.1,
            ))
        })?;
        let mut snapshot = self.snapshot()?;
        snapshot["imported"] = json!({"servers": servers, "accounts": accounts});
        Ok(snapshot)
    }
    pub fn can_recover(&self) -> bool {
        self.path
            .as_ref()
            .and_then(|path| File::open(path).ok())
            .and_then(|file| profile_bytes(file).ok())
            .is_some_and(|bytes| decode_profiles(&bytes).is_err())
    }
    pub(super) fn recover_profiles(&self) -> Result<Value, String> {
        let path = self
            .path
            .as_ref()
            .ok_or("There is no profile file to recover.")?;
        let mut memory = self
            .memory
            .lock()
            .map_err(|_| "Profile storage is unavailable.")?;
        let lock = private_file(&path.with_extension("lock"), false)?;
        lock.lock().map_err(|_| "Cannot lock the profile file.")?;
        let bytes =
            profile_bytes(File::open(path).map_err(|_| "Cannot read the original profile file.")?)?;
        if decode_profiles(&bytes).is_ok() {
            return Err(
                "Profiles are readable now. Retry loading them before making changes.".into(),
            );
        }
        let fresh = Profiles::default();
        let staged = StagedProfiles::new(
            path,
            &serde_json::to_vec_pretty(&fresh).map_err(|_| "Cannot encode profiles.")?,
        )?;
        let backup = path.with_file_name(format!("launcher.recovered-{}.json", unique()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&backup)
            .map_err(|_| "Cannot create the recovery copy; profiles have not changed.")?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "Cannot finish the recovery copy; profiles have not changed.")?;
        drop(file);
        staged.commit()?;
        *memory = fresh;
        drop(lock);
        drop(memory);
        let mut snapshot = self.snapshot()?;
        snapshot["recovery_copy"] = json!(backup.to_string_lossy());
        // Leave OS-vault entries intact; arbitrary corrupt bytes cannot safely
        // identify credentials. Imported accounts receive new identifiers.
        Ok(snapshot)
    }
}
