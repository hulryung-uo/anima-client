//! UO sound-effect reader: decodes entries from `soundLegacyMUL.uop` and serves
//! them as ready-to-play WAV files (PCM 16-bit signed, mono, 22050 Hz).
//!
//! Each UOP sound entry is a 40-byte ASCII name header followed by the raw PCM
//! samples (ported from ClassicUO `SoundsLoader.TryGetSound`). `Sound.def`
//! remaps sound ids that have no own data to another id's data.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use crate::uop::{uop_hash, LazyUopReader};

/// Bytes of fixed ASCII name that precede the PCM data in each sound entry.
const NAME_HEADER: usize = 40;
/// UO sound sample rate (Hz).
const SAMPLE_RATE: u32 = 22050;
const SOUND_CACHE_BYTES: usize = 32 * 1024 * 1024;
const SOUND_CACHE_ENTRIES: usize = 512;
// Includes the largest checked stock sound (~21 MB), while bounding malformed
// entry declarations/decompression. A WAV adds 44 bytes to the raw PCM.
const MAX_SOUND_WAV_BYTES: usize = 32 * 1024 * 1024;

struct CachedSound {
    wav: Option<Vec<u8>>,
    touched: u64,
}
struct SoundCache {
    entries: HashMap<u16, CachedSound>,
    bytes: usize,
    clock: u64,
    byte_limit: usize,
    entry_limit: usize,
}
impl SoundCache {
    fn new(byte_limit: usize, entry_limit: usize) -> Self {
        Self {
            entries: HashMap::new(),
            bytes: 0,
            clock: 0,
            byte_limit,
            entry_limit,
        }
    }
    fn get(&mut self, id: u16) -> Option<Option<Vec<u8>>> {
        let entry = self.entries.get_mut(&id)?;
        self.clock += 1;
        entry.touched = self.clock;
        Some(entry.wav.clone())
    }
    fn insert(&mut self, id: u16, wav: &Option<Vec<u8>>) {
        if let Some(old) = self.entries.remove(&id) {
            self.bytes -= old.wav.as_ref().map_or(0, Vec::len);
        }
        let bytes = wav.as_ref().map_or(0, Vec::len);
        if bytes > self.byte_limit || self.entry_limit == 0 {
            return;
        }
        // Entry bound also limits missing/silenced IDs, which cost no WAV bytes.
        while self.bytes + bytes > self.byte_limit || self.entries.len() >= self.entry_limit {
            let oldest = *self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.touched)
                .unwrap()
                .0;
            let old = self.entries.remove(&oldest).unwrap();
            self.bytes -= old.wav.as_ref().map_or(0, Vec::len);
        }
        self.clock += 1;
        self.entries.insert(
            id,
            CachedSound {
                wav: wav.clone(),
                touched: self.clock,
            },
        );
        self.bytes += bytes;
    }
}

/// Sound-effect asset reader over `soundLegacyMUL.uop`.
pub struct Sounds {
    uop: LazyUopReader,
    /// `Sound.def` replacements: id with no own data → the id whose data to use
    /// (`-1` = explicitly silent).
    replacements: HashMap<u16, i32>,
    /// Retained WAVs, including negative lookups, have byte and entry budgets.
    cache: Mutex<SoundCache>,
}

impl Sounds {
    /// Open the sound UOP (and `Sound.def`, if present) under `data_dir`.
    pub fn open(data_dir: impl AsRef<Path>) -> std::io::Result<Sounds> {
        let dir = data_dir.as_ref();
        let uop = LazyUopReader::open(&dir.join("soundLegacyMUL.uop"))?;
        let replacements = std::fs::read_to_string(dir.join("Sound.def"))
            .map(|s| parse_sound_def(&s))
            .unwrap_or_default();
        Ok(Sounds {
            uop,
            replacements,
            cache: Mutex::new(SoundCache::new(SOUND_CACHE_BYTES, SOUND_CACHE_ENTRIES)),
        })
    }

    /// Raw PCM samples for a sound id (after the 40-byte name header), resolving
    /// `Sound.def` replacements when the id has no own data.
    fn pcm(&self, id: u16) -> Option<Vec<u8>> {
        if let Some(raw) = self.raw_sound(id as usize) {
            if raw.len() > NAME_HEADER {
                return Some(raw[NAME_HEADER..].to_vec());
            }
        }
        // No own data: follow the def remap (one hop; -1 means silent).
        match self.replacements.get(&id) {
            Some(&repl) if repl >= 0 && repl != id as i32 => {
                let raw = self.raw_sound(repl as usize)?;
                if raw.len() > NAME_HEADER {
                    Some(raw[NAME_HEADER..].to_vec())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn raw_sound(&self, id: usize) -> Option<Vec<u8>> {
        let path = format!("build/soundlegacymul/{id:08}.dat");
        self.uop
            .by_hash_bounded(uop_hash(&path), MAX_SOUND_WAV_BYTES - 44 + NAME_HEADER)
    }

    /// The sound as a complete WAV file (PCM 16-bit mono 22050 Hz), ready to
    /// serve to a browser. Cached per id.
    pub fn wav(&self, id: u16) -> Option<Vec<u8>> {
        if let Some(hit) = self.cache.lock().unwrap().get(id) {
            return hit;
        }
        let wav = self.pcm(id).map(|pcm| wrap_wav(&pcm));
        self.cache.lock().unwrap().insert(id, &wav);
        wav
    }
}

/// Wrap raw PCM 16-bit mono 22050 Hz samples in a 44-byte WAV header.
fn wrap_wav(pcm: &[u8]) -> Vec<u8> {
    const CHANNELS: u16 = 1;
    const BITS: u16 = 16;
    let byte_rate = SAMPLE_RATE * CHANNELS as u32 * (BITS as u32 / 8);
    let block_align = CHANNELS * (BITS / 8);
    let data_len = pcm.len() as u32;
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // audio format = PCM
    out.extend_from_slice(&CHANNELS.to_le_bytes());
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&BITS.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

/// Parse `Sound.def` lines like `654 {487} 0` into id → replacement id. The
/// braces hold a group; we take the last entry (ClassicUO copies each in turn,
/// so the last wins). `-1` means the id is explicitly silenced.
fn parse_sound_def(text: &str) -> HashMap<u16, i32> {
    let mut map = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // index is the first integer.
        let Some(index) = line
            .split_whitespace()
            .next()
            .and_then(|t| t.parse::<u16>().ok())
        else {
            continue;
        };
        // group is whatever sits inside the braces.
        let Some(open) = line.find('{') else { continue };
        let Some(close) = line[open..].find('}').map(|p| open + p) else {
            continue;
        };
        let group = &line[open + 1..close];
        let last = group
            .split([' ', ',', '\t'])
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.trim().parse::<i32>().ok())
            .next_back();
        if let Some(repl) = last {
            map.insert(index, repl);
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_evicts_least_recent_sound_without_changing_a_served_copy() {
        let mut cache = SoundCache::new(8, 3);
        cache.insert(1, &Some(vec![1; 4]));
        cache.insert(2, &Some(vec![2; 4]));
        let serving = cache.get(1).unwrap().unwrap();
        cache.insert(3, &Some(vec![3; 4]));
        assert!(cache.get(2).is_none());
        assert_eq!(cache.get(1), Some(Some(vec![1; 4])));
        assert_eq!(cache.bytes, 8);
        cache.insert(1, &Some(vec![9; 6]));
        assert!(cache.get(3).is_none());
        assert_eq!(cache.bytes, 6);
        assert_eq!(serving, vec![1; 4]);
        cache.insert(4, &Some(vec![4; 9])); // can be served without retaining it
        assert!(cache.get(4).is_none());
        assert_eq!(cache.get(1), Some(Some(vec![9; 6])));
    }

    #[test]
    fn missing_sounds_are_cached_without_unbounded_metadata() {
        let mut cache = SoundCache::new(8, 3);
        cache.insert(1, &Some(vec![1; 4]));
        for id in 2..1000 {
            assert!(cache.get(1).is_some()); // frequently requested sound stays warm
            cache.insert(id, &None);
            assert!(cache.entries.len() <= 3);
        }
        assert_eq!(cache.get(999), Some(None));
        assert!(cache.get(2).is_none());
        assert_eq!(cache.bytes, 4);
    }

    #[test]
    fn sounds_read_payload_on_demand_and_preserve_def_aliases_and_silence() {
        let dir = std::env::temp_dir().join(format!("anima_sound_lazy_{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let file = dir.join("soundLegacyMUL.uop");
        let mut raw = vec![0; NAME_HEADER];
        raw.extend_from_slice(&[1, 2, 3, 4]);
        let mut data =
            crate::uop::tests::build_single_entry_uop("build/soundlegacymul/00070000.dat", &raw);
        std::fs::write(&file, &data).unwrap();
        std::fs::write(
            dir.join("Sound.def"),
            "1 {70000} 0\n2 {70000} 0\n3 {-1} 0\n",
        )
        .unwrap();
        let sounds = Sounds::open(&dir).unwrap();
        // A lazy reader has only read the directory. Change this owned fixture's
        // payload before its first lookup to distinguish it from an eager copy.
        let last = data.len() - 1;
        data[last] = 9;
        std::fs::write(&file, data).unwrap();
        assert_eq!(&sounds.wav(1).unwrap()[44..], &[1, 2, 3, 9]);
        assert_eq!(sounds.wav(2), sounds.wav(1));
        assert_eq!(sounds.wav(3), None);
        assert_eq!(sounds.wav(4), None);
        drop(sounds);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn wav_header_is_valid_riff() {
        let pcm = vec![0u8, 1, 2, 3];
        let wav = wrap_wav(&pcm);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[36..40], b"data");
        // sample rate at offset 24 (LE)
        assert_eq!(
            u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]),
            22050
        );
        // data chunk length matches the PCM length
        assert_eq!(u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]), 4);
        assert_eq!(wav.len(), 44 + 4);
    }

    #[test]
    fn parses_sound_def_replacement() {
        let m = parse_sound_def("654 {487} 0\n655 {263} 0\n# comment\n");
        assert_eq!(m.get(&654), Some(&487));
        assert_eq!(m.get(&655), Some(&263));
    }

    /// Requires local UO data at ~/dev/uo/uo-resource. Ignored by default.
    #[test]
    #[ignore]
    fn opens_real_sounds_and_builds_wav() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        assert!(
            Path::new(&dir).join("soundLegacyMUL.uop").exists(),
            "real sound bank is required"
        );
        let sounds = Sounds::open(&dir).expect("open sounds");
        // Scan a range; expect at least one decodable sound with a valid header.
        let mut found = 0;
        let mut largest = 0;
        for id in 0u16..0x1000 {
            if let Some(wav) = sounds.wav(id) {
                assert_eq!(&wav[0..4], b"RIFF");
                assert!(wav.len() > 44);
                assert!(wav.len() <= MAX_SOUND_WAV_BYTES);
                largest = largest.max(wav.len());
                found += 1;
            }
            let cache = sounds.cache.lock().unwrap();
            assert!(cache.bytes <= SOUND_CACHE_BYTES);
            assert!(cache.entries.len() <= SOUND_CACHE_ENTRIES);
        }
        let cache = sounds.cache.lock().unwrap();
        println!("decoded {found} sounds in 0..0x1000; largest WAV {largest} bytes; cache {} bytes / {} entries", cache.bytes, cache.entries.len());
        assert!(found > 0, "expected some decodable sounds");
    }
}
