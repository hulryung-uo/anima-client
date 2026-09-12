//! Small, locked desktop preferences. Malformed data is never overwritten by
//! a routine save; recovery first preserves the exact bytes in a new backup.
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
fn version() -> u32 {
    1
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DesktopConfig {
    #[serde(default = "version")]
    pub version: u32,
    #[serde(default)]
    pub data_dir: PathBuf,
    #[serde(default)]
    pub http_port: Option<u16>,
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}
impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            version: 1,
            data_dir: PathBuf::new(),
            http_port: None,
            extra: Default::default(),
        }
    }
}
#[derive(Clone)]
pub struct ConfigStore {
    pub path: PathBuf,
}
impl ConfigStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
    fn lock(&self) -> Result<File, String> {
        let parent = self.path.parent().ok_or("Invalid settings location.")?;
        fs::create_dir_all(parent)
            .map_err(|e| format!("Cannot create the settings folder: {e}"))?;
        let lock = private_file(&self.path.with_extension("lock"), false)
            .map_err(|e| format!("Cannot open the settings lock: {e}"))?;
        lock.lock()
            .map_err(|e| format!("Cannot lock app settings: {e}"))?;
        Ok(lock)
    }
    fn bytes(&self) -> Result<Option<Vec<u8>>, String> {
        match File::open(&self.path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(MAX_CONFIG_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| format!("Cannot read app settings: {e}"))?;
                if bytes.len() as u64 > MAX_CONFIG_BYTES {
                    return Err(
                        "App settings exceed 1 MB. The file has been left unchanged.".into(),
                    );
                }
                Ok(Some(bytes))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("Cannot read app settings: {e}")),
        }
    }
    fn decode(bytes: &[u8]) -> Result<DesktopConfig, String> {
        let cfg: DesktopConfig = serde_json::from_slice(bytes).map_err(|_| {
            "App settings are unreadable. Back up and reset them to continue.".to_string()
        })?;
        if cfg.version != 1 {
            return Err("These app settings use a different version. Use a compatible Anima build, or back up and reset them.".into());
        }
        Ok(cfg)
    }
    pub fn load(&self) -> Result<Option<DesktopConfig>, String> {
        let _lock = self.lock()?;
        self.bytes()?.map(|b| Self::decode(&b)).transpose()
    }
    pub fn recoverable(&self) -> bool {
        self.bytes()
            .ok()
            .flatten()
            .is_some_and(|b| Self::decode(&b).is_err())
    }
    fn write(&self, cfg: &DesktopConfig) -> Result<(), String> {
        let tmp = self
            .path
            .with_extension(format!("{}.{}.tmp", std::process::id(), unique()));
        let result = (|| -> io::Result<()> {
            let mut file = private_file(&tmp, true)?;
            file.write_all(&serde_json::to_vec_pretty(cfg)?)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&tmp, &self.path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result.map_err(|e| {
            format!("Could not save app settings. Your previous settings are unchanged: {e}")
        })
    }
    fn update(&self, change: impl FnOnce(&mut DesktopConfig)) -> Result<DesktopConfig, String> {
        let _lock = self.lock()?;
        let mut cfg = self
            .bytes()?
            .map(|b| Self::decode(&b))
            .transpose()?
            .unwrap_or_default();
        change(&mut cfg);
        self.write(&cfg)?;
        Ok(cfg)
    }
    pub fn save_data_dir(&self, dir: &Path) -> Result<(), String> {
        self.update(|cfg| cfg.data_dir = dir.to_path_buf())
            .map(|_| ())
    }
    /// The first window claims the origin; another concurrent window preserves
    /// it so the original browser preferences return on the next launch.
    pub fn claim_port(&self, port: u16) -> Result<u16, String> {
        self.update(|cfg| {
            if cfg.http_port.is_none_or(|p| p < 1024) {
                cfg.http_port = Some(port);
            }
        })
        .map(|cfg| cfg.http_port.unwrap_or(port))
    }
    pub fn recover(&self) -> Result<PathBuf, String> {
        let _lock = self.lock()?;
        let bytes = self
            .bytes()?
            .ok_or("The settings file no longer exists. Reload this screen.")?;
        if Self::decode(&bytes).is_ok() {
            return Err("App settings have changed and are readable now. Reload this screen before making changes.".into());
        }
        let backup = self
            .path
            .with_file_name(format!("config.recovered-{}.json", unique()));
        let mut file = private_file(&backup, true)
            .map_err(|e| format!("Cannot create a settings backup: {e}"))?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| format!("Cannot finish the settings backup: {e}"))?;
        drop(file);
        self.write(&DesktopConfig::default())?;
        Ok(backup)
    }
}
fn unique() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    t.wrapping_add(COUNTER.fetch_add(1, Ordering::Relaxed))
}
fn private_file(path: &Path, new: bool) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    if new {
        options.create_new(true);
    } else {
        options.create(true).truncate(false);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("anima-config-{}", unique()));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn store(&self) -> ConfigStore {
            ConfigStore::new(self.0.join("config.json"))
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn old_config_and_unknown_fields_survive_updates() {
        let dir = Temp::new();
        let store = dir.store();
        fs::write(&store.path, br#"{"data_dir":"old","custom":{"keep":true}}"#).unwrap();
        assert_eq!(store.load().unwrap().unwrap().http_port, None);
        store.claim_port(8192).unwrap();
        store.save_data_dir(Path::new("new")).unwrap();
        let cfg = store.load().unwrap().unwrap();
        assert_eq!(cfg.data_dir, Path::new("new"));
        assert_eq!(cfg.http_port, Some(8192));
        assert_eq!(cfg.extra["custom"]["keep"], true);
    }
    #[test]
    fn corrupt_config_requires_explicit_recovery_and_backup_is_exact() {
        let dir = Temp::new();
        let store = dir.store();
        let bytes = b"{interrupted save";
        fs::write(&store.path, bytes).unwrap();
        assert!(store.load().is_err());
        assert!(store.save_data_dir(Path::new("new")).is_err());
        assert_eq!(fs::read(&store.path).unwrap(), bytes);
        let backup = store.recover().unwrap();
        assert_eq!(fs::read(backup).unwrap(), bytes);
        store.save_data_dir(Path::new("new")).unwrap();
        assert!(store.load().is_ok());
        assert!(store.recover().is_err()); // a stale recovery click cannot reset valid settings
    }
    #[test]
    fn concurrent_windows_keep_the_first_origin_and_latest_directory() {
        let dir = Temp::new();
        let store = dir.store();
        let mut workers = Vec::new();
        for port in 8190..8198 {
            let store = store.clone();
            workers.push(std::thread::spawn(move || store.claim_port(port).unwrap()));
        }
        let values: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
        assert!(values.iter().all(|p| *p == values[0]));
        store.save_data_dir(Path::new("picked")).unwrap();
        assert_eq!(store.load().unwrap().unwrap().http_port, Some(values[0]));
    }
    #[test]
    fn newer_versions_and_oversized_files_are_preserved() {
        let dir = Temp::new();
        let store = dir.store();
        let bytes = br#"{"version":2,"data_dir":"future"}"#;
        fs::write(&store.path, bytes).unwrap();
        assert!(store.claim_port(8190).is_err());
        assert_eq!(fs::read(&store.path).unwrap(), bytes);
        fs::write(&store.path, vec![b' '; MAX_CONFIG_BYTES as usize + 1]).unwrap();
        assert!(store.load().is_err());
        assert!(!store.recoverable());
    }
}
