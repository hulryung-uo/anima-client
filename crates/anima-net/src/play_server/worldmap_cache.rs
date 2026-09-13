//! A world-map cache belongs to one resource folder and one set of source files.
use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 8] = b"ANIMAP02";
const MAX_BYTES: u64 = 128 * 1024 * 1024;
const FILES: &[&str] = &[
    "map0LegacyMUL.uop",
    "staidx0.mul",
    "statics0.mul",
    "tiledata.mul",
    "radarcol.mul",
    "mapdifl0.mul",
    "mapdif0.mul",
    "stadifl0.mul",
    "stadifi0.mul",
    "stadif0.mul",
];

fn digest(value: &[u8]) -> u64 {
    let mut hash = DefaultHasher::new();
    value.hash(&mut hash);
    hash.finish()
}

pub(super) struct WorldmapCache {
    path: PathBuf,
    source: u64,
}
impl WorldmapCache {
    pub(super) fn for_resources(directory: &Path, step: u32) -> io::Result<Self> {
        Self::in_directory(directory, step, &std::env::temp_dir())
    }
    fn in_directory(directory: &Path, step: u32, cache_directory: &Path) -> io::Result<Self> {
        let directory = fs::canonicalize(directory)?;
        let mut location = DefaultHasher::new();
        directory.hash(&mut location);
        step.hash(&mut location);
        let path = cache_directory.join(format!(
            "anima-worldmap-v2-{:016x}.cache",
            location.finish()
        ));
        let mut source = DefaultHasher::new();
        directory.hash(&mut source);
        step.hash(&mut source);
        for (index, name) in FILES.iter().enumerate() {
            name.hash(&mut source);
            match fs::metadata(directory.join(name)) {
                Ok(metadata) if metadata.is_file() => {
                    true.hash(&mut source);
                    metadata.len().hash(&mut source);
                    metadata.modified()?.hash(&mut source);
                }
                Err(error) if index >= 5 && error.kind() == io::ErrorKind::NotFound => {
                    false.hash(&mut source)
                }
                Err(error) => return Err(error),
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "map source is not a file",
                    ))
                }
            }
        }
        Ok(Self {
            path,
            source: source.finish(),
        })
    }
    pub(super) fn read(&self) -> Option<Vec<u8>> {
        let mut file = File::open(&self.path).ok()?;
        let length = file.metadata().ok()?.len();
        if !(33..=MAX_BYTES + 32).contains(&length) {
            return None;
        }
        let mut header = [0; 32];
        file.read_exact(&mut header).ok()?;
        let number = |start| u64::from_le_bytes(header[start..start + 8].try_into().unwrap());
        if &header[..8] != MAGIC || number(8) != self.source || number(16) != length - 32 {
            return None;
        }
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1).read_to_end(&mut bytes).ok()?;
        if bytes.len() as u64 != number(16) || digest(&bytes) != number(24) {
            return None;
        }
        Some(bytes)
    }
    pub(super) fn write_if_current(
        &self,
        directory: &Path,
        step: u32,
        bytes: &[u8],
    ) -> io::Result<()> {
        if Self::for_resources(directory, step)?.source != self.source {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "map sources changed during rendering",
            ));
        }
        self.write(bytes)
    }
    fn write(&self, bytes: &[u8]) -> io::Result<()> {
        if bytes.is_empty() || bytes.len() as u64 > MAX_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "map cache size is invalid",
            ));
        }
        let staging = self
            .path
            .with_extension(format!("{}.part", crate::connection::fresh_context_id()));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&staging)?;
            file.write_all(MAGIC)?;
            file.write_all(&self.source.to_le_bytes())?;
            file.write_all(&(bytes.len() as u64).to_le_bytes())?;
            file.write_all(&digest(bytes).to_le_bytes())?;
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&staging, &self.path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(staging);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Folder(PathBuf);
    impl Folder {
        fn new() -> Self {
            let path = std::env::temp_dir().join(crate::connection::fresh_context_id());
            fs::create_dir(&path).unwrap();
            for name in &FILES[..5] {
                fs::write(path.join(name), b"fixture").unwrap();
            }
            Self(path)
        }
        fn cache(&self) -> WorldmapCache {
            WorldmapCache::in_directory(&self.0, 1, &self.0).unwrap()
        }
    }
    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn folders_and_renderer_steps_do_not_share_cache_paths() {
        let a = Folder::new();
        let b = Folder::new();
        let first = WorldmapCache::in_directory(&a.0, 1, &a.0).unwrap();
        let second = WorldmapCache::in_directory(&b.0, 1, &a.0).unwrap();
        let step = WorldmapCache::in_directory(&a.0, 2, &a.0).unwrap();
        assert_ne!(first.path, second.path);
        assert_ne!(first.path, step.path);
        first.write(b"first map").unwrap();
        assert_eq!(a.cache().read().unwrap(), b"first map");
        assert!(second.read().is_none());
        assert!(step.read().is_none());
    }
    #[test]
    fn changing_any_map_source_rejects_the_previous_image() {
        for name in FILES {
            let folder = Folder::new();
            let old = folder.cache();
            old.write(b"old map").unwrap();
            fs::write(folder.0.join(name), b"changed fixture length").unwrap();
            let new = folder.cache();
            assert_eq!(
                old.path, new.path,
                "updates reuse one slot per resource folder"
            );
            assert!(new.read().is_none(), "{name}");
            new.write(b"new map").unwrap();
            assert_eq!(new.read().unwrap(), b"new map");
        }
    }
    #[test]
    fn modified_time_invalidates_same_length_assets() {
        let folder = Folder::new();
        let old = folder.cache();
        old.write(b"map").unwrap();
        let file = File::options()
            .write(true)
            .open(folder.0.join(FILES[0]))
            .unwrap();
        file.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1))
            .unwrap();
        assert!(folder.cache().read().is_none());
    }
    #[test]
    fn corruption_truncation_and_oversized_cache_are_misses() {
        let folder = Folder::new();
        let cache = folder.cache();
        cache.write(b"map pixels").unwrap();
        let original = fs::read(&cache.path).unwrap();
        for bytes in [
            original[..20].to_vec(),
            {
                let mut b = original.clone();
                b[32] ^= 1;
                b
            },
            {
                let mut b = original.clone();
                b.push(0);
                b
            },
        ] {
            fs::write(&cache.path, bytes).unwrap();
            assert!(cache.read().is_none());
        }
        File::options()
            .write(true)
            .open(&cache.path)
            .unwrap()
            .set_len(MAX_BYTES + 33)
            .unwrap();
        assert!(cache.read().is_none());
        cache.write(b"rebuilt map").unwrap();
        assert_eq!(cache.read().unwrap(), b"rebuilt map");
    }
    #[test]
    fn missing_sources_and_changes_during_render_do_not_reuse_or_publish_cache() {
        let folder = Folder::new();
        let cache = folder.cache();
        cache.write(b"old map").unwrap();
        fs::write(folder.0.join(FILES[0]), b"changed map source").unwrap();
        assert!(cache
            .write_if_current(&folder.0, 1, b"stale render")
            .is_err());
        assert_eq!(cache.read().unwrap(), b"old map");
        assert!(folder.cache().read().is_none());
        fs::remove_file(folder.0.join(FILES[0])).unwrap();
        assert!(WorldmapCache::for_resources(&folder.0, 1).is_err());
    }
    #[test]
    fn concurrent_writers_publish_whole_records_without_staging_collisions() {
        let folder = Folder::new();
        std::thread::scope(|scope| {
            let workers: Vec<_> = (1..=8)
                .map(|value| {
                    let cache = folder.cache();
                    scope.spawn(move || cache.write(&vec![value; 8192]).unwrap())
                })
                .collect();
            for worker in workers {
                worker.join().unwrap();
            }
        });
        let bytes = folder.cache().read().unwrap();
        assert_eq!(bytes.len(), 8192);
        assert!((1..=8).contains(&bytes[0]));
        assert!(bytes.iter().all(|value| *value == bytes[0]));
        assert!(!fs::read_dir(&folder.0).unwrap().any(|entry| entry
            .unwrap()
            .path()
            .extension()
            .is_some_and(|extension| extension == "part")));
    }
}
