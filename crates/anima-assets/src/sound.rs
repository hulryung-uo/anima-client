//! UO sound-effect reader: decodes entries from `soundLegacyMUL.uop` and serves
//! them as ready-to-play WAV files (PCM 16-bit signed, mono, 22050 Hz).
//!
//! Each UOP sound entry is a 40-byte ASCII name header followed by the raw PCM
//! samples (ported from ClassicUO `SoundsLoader.TryGetSound`). `Sound.def`
//! remaps sound ids that have no own data to another id's data.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use crate::uop::UopReader;

/// Bytes of fixed ASCII name that precede the PCM data in each sound entry.
const NAME_HEADER: usize = 40;
/// UO sound sample rate (Hz).
const SAMPLE_RATE: u32 = 22050;

/// Sound-effect asset reader over `soundLegacyMUL.uop`.
pub struct Sounds {
    uop: UopReader,
    /// `Sound.def` replacements: id with no own data → the id whose data to use
    /// (`-1` = explicitly silent).
    replacements: HashMap<u16, i32>,
    /// Cache of fully-built WAV files by id (cheap to clone out for serving).
    cache: Mutex<HashMap<u16, Option<Vec<u8>>>>,
}

impl Sounds {
    /// Open the sound UOP (and `Sound.def`, if present) under `data_dir`.
    pub fn open(data_dir: impl AsRef<Path>) -> std::io::Result<Sounds> {
        let dir = data_dir.as_ref();
        let uop = UopReader::open(&dir.join("soundLegacyMUL.uop"))?;
        let replacements = std::fs::read_to_string(dir.join("Sound.def"))
            .map(|s| parse_sound_def(&s))
            .unwrap_or_default();
        Ok(Sounds {
            uop,
            replacements,
            cache: Mutex::new(HashMap::new()),
        })
    }

    /// Raw PCM samples for a sound id (after the 40-byte name header), resolving
    /// `Sound.def` replacements when the id has no own data.
    fn pcm(&self, id: u16) -> Option<Vec<u8>> {
        if let Some(raw) = self.uop.by_sound(id as usize) {
            if raw.len() > NAME_HEADER {
                return Some(raw[NAME_HEADER..].to_vec());
            }
        }
        // No own data: follow the def remap (one hop; -1 means silent).
        match self.replacements.get(&id) {
            Some(&repl) if repl >= 0 && repl != id as i32 => {
                let raw = self.uop.by_sound(repl as usize)?;
                if raw.len() > NAME_HEADER {
                    Some(raw[NAME_HEADER..].to_vec())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// The sound as a complete WAV file (PCM 16-bit mono 22050 Hz), ready to
    /// serve to a browser. Cached per id.
    pub fn wav(&self, id: u16) -> Option<Vec<u8>> {
        if let Some(hit) = self.cache.lock().unwrap().get(&id) {
            return hit.clone();
        }
        let wav = self.pcm(id).map(|pcm| wrap_wav(&pcm));
        self.cache.lock().unwrap().insert(id, wav.clone());
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
        let Some(index) = line.split_whitespace().next().and_then(|t| t.parse::<u16>().ok()) else {
            continue;
        };
        // group is whatever sits inside the braces.
        let Some(open) = line.find('{') else { continue };
        let Some(close) = line[open..].find('}').map(|p| open + p) else { continue };
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
    fn wav_header_is_valid_riff() {
        let pcm = vec![0u8, 1, 2, 3];
        let wav = wrap_wav(&pcm);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[36..40], b"data");
        // sample rate at offset 24 (LE)
        assert_eq!(u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]), 22050);
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
        if !Path::new(&dir).join("soundLegacyMUL.uop").exists() {
            return;
        }
        let sounds = Sounds::open(&dir).expect("open sounds");
        // Scan a range; expect at least one decodable sound with a valid header.
        let mut found = 0;
        for id in 1u16..0x200 {
            if let Some(wav) = sounds.wav(id) {
                assert_eq!(&wav[0..4], b"RIFF");
                assert!(wav.len() > 44);
                found += 1;
            }
        }
        println!("decoded {found} sounds in 1..0x200");
        assert!(found > 0, "expected some decodable sounds");
    }
}
