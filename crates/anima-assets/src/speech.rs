//! `speech.mul` keyword table — the ids ServUO puts on `e.Keywords`.
//!
//! Records are **big-endian** `id:u16`, `length:u16`, then `length` UTF-8 bytes
//! (ClassicUO `SpeechesLoader`). A keyword string is split on `*`; a leading
//! `*` means the match may start mid-phrase (`CheckStart`), a trailing `*`
//! means it may end mid-phrase (`CheckEnd`). Matching is case-insensitive and
//! word-bounded the way ClassicUO's `IsMatch` is: a letter on either side of
//! the hit rejects it, but punctuation (`!bank`, `bank!`) is allowed.
//!
//! The core stays file-free: this crate only loads and matches. The driver
//! hands the resulting ids to `build_unicode_say`, which packs them onto 0xAD.

use std::path::Path;
use std::sync::Arc;

/// One `speech.mul` entry after the `*`-split ClassicUO performs in
/// `SpeechEntry`'s constructor.
#[derive(Debug, Clone)]
pub struct SpeechEntry {
    pub id: u16,
    /// Non-empty fragments between `*` in the stored keyword.
    pub parts: Vec<String>,
    /// Stored keyword began with `*` — match need not be at the start.
    pub check_start: bool,
    /// Stored keyword ended with `*` — match need not be at the end.
    pub check_end: bool,
}

/// Parsed `speech.mul`. Cheap to clone (`Arc` of the table).
#[derive(Debug, Clone, Default)]
pub struct Speeches {
    entries: Arc<Vec<SpeechEntry>>,
}

impl Speeches {
    /// Load `speech.mul` from a UO data directory. Missing file → empty table
    /// (speech still works; keyword commands like "vendor buy" will not).
    pub fn open(resource_dir: impl AsRef<Path>) -> std::io::Result<Speeches> {
        let path = resource_dir.as_ref().join("speech.mul");
        let data = std::fs::read(&path)?;
        Ok(Speeches {
            entries: Arc::new(parse_speech_mul(&data)),
        })
    }

    pub fn from_bytes(data: &[u8]) -> Speeches {
        Speeches {
            entries: Arc::new(parse_speech_mul(data)),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Keyword ids that match `text`, sorted by id (ClassicUO `GetKeywords`).
    /// Empty when nothing matches — the caller then sends plain 0x03 / 0xAD.
    /// Capped at 50: ServUO's `UnicodeSpeech` drops the packet above that.
    pub fn keywords(&self, text: &str) -> Vec<u16> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Vec::new();
        }
        let mut ids: Vec<u16> = self
            .entries
            .iter()
            .filter(|e| is_match(trimmed, e))
            .map(|e| e.id)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids.truncate(50);
        ids
    }
}

/// Big-endian records: `[id u16][len u16][len bytes UTF-8]…`.
pub(crate) fn parse_speech_mul(data: &[u8]) -> Vec<SpeechEntry> {
    let mut entries = Vec::new();
    let mut p = 0usize;
    while p + 4 <= data.len() {
        let id = u16::from_be_bytes([data[p], data[p + 1]]);
        let len = u16::from_be_bytes([data[p + 2], data[p + 3]]) as usize;
        p += 4;
        if len == 0 {
            continue;
        }
        if p + len > data.len() {
            break;
        }
        let text = String::from_utf8_lossy(&data[p..p + len]).into_owned();
        p += len;
        entries.push(speech_entry(id, &text));
    }
    entries
}

pub(crate) fn speech_entry(id: u16, keyword: &str) -> SpeechEntry {
    let parts: Vec<String> = keyword
        .split('*')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    SpeechEntry {
        id,
        parts,
        check_start: keyword.starts_with('*'),
        check_end: keyword.ends_with('*'),
    }
}

/// ClassicUO `SpeechesLoader.IsMatch`.
pub(crate) fn is_match(input: &str, entry: &SpeechEntry) -> bool {
    let input_l = input.to_lowercase();
    for part in &entry.parts {
        if part.is_empty() || part.len() > input.len() {
            continue;
        }
        let part_l = part.to_lowercase();
        if !entry.check_start && !input_l.starts_with(&part_l) {
            continue;
        }
        if !entry.check_end && !input_l.ends_with(&part_l) {
            continue;
        }
        let mut from = 0usize;
        while let Some(rel) = input_l[from..].find(&part_l) {
            let idx = from + rel;
            let before_ok = idx == 0
                || input_l[..idx]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_whitespace() || !c.is_alphabetic());
            let after = idx + part_l.len();
            let after_ok = after >= input_l.len()
                || input_l[after..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_whitespace() || !c.is_alphabetic());
            if before_ok && after_ok {
                return true;
            }
            // Advance one *character*, not one byte: ClassicUO's `idx + 1` is
            // a UTF-16 index. A byte step here panics on the next `find` when
            // `part` is multibyte (Korean keywords in speech.mul).
            from = input_l[idx..]
                .chars()
                .next()
                .map(|c| idx + c.len_utf8())
                .unwrap_or(input_l.len());
            if from >= input_l.len() {
                break;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_speech_mul_reads_be_id_and_utf8() {
        // Two records: id 0x0009 "*vendor*buy*", id 0x0034 "forward".
        let mut data = Vec::new();
        let a = "*vendor*buy*";
        data.extend_from_slice(&0x0009u16.to_be_bytes());
        data.extend_from_slice(&(a.len() as u16).to_be_bytes());
        data.extend_from_slice(a.as_bytes());
        let b = "forward";
        data.extend_from_slice(&0x0034u16.to_be_bytes());
        data.extend_from_slice(&(b.len() as u16).to_be_bytes());
        data.extend_from_slice(b.as_bytes());
        let entries = parse_speech_mul(&data);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, 0x0009);
        assert_eq!(entries[0].parts, ["vendor", "buy"]);
        assert!(entries[0].check_start && entries[0].check_end);
        assert_eq!(entries[1].id, 0x0034);
        assert_eq!(entries[1].parts, ["forward"]);
        assert!(!entries[1].check_start && !entries[1].check_end);
    }

    #[test]
    fn keywords_match_vendor_buy_and_sort() {
        let s = Speeches::from_bytes(&{
            let mut data = Vec::new();
            for (id, kw) in [
                (0x000C_u16, "*buy*"),
                (0x0009, "*vendor*buy*"),
                (0x0034, "forward"),
            ] {
                data.extend_from_slice(&id.to_be_bytes());
                data.extend_from_slice(&(kw.len() as u16).to_be_bytes());
                data.extend_from_slice(kw.as_bytes());
            }
            data
        });
        // Either fragment of `*vendor*buy*` matches; `*buy*` matches too.
        // Sorted by id — ClassicUO `GetKeywords` + `SpeechEntry.CompareTo`.
        assert_eq!(s.keywords("vendor buy"), vec![0x0009, 0x000C]);
        assert_eq!(s.keywords("Buy"), vec![0x0009, 0x000C]);
        // Exact (no wildcards) only matches the whole trimmed string.
        assert_eq!(s.keywords("forward"), vec![0x0034]);
        assert!(s.keywords("go forward please").is_empty());
        // Word boundary: a letter on either side rejects.
        assert!(s.keywords("buyout").is_empty());
        assert_eq!(s.keywords("!buy!"), vec![0x0009, 0x000C]);
    }

    #[test]
    fn is_match_advances_by_char_on_multibyte_keywords() {
        // "가안녕": a letter before "안" rejects the first hit; the retry
        // must not slice mid-syllable. Panic here was `from = idx + 1`.
        let e = speech_entry(1, "*안*");
        assert!(!is_match("가안녕", &e));
        assert!(is_match("안", &e));
        assert!(is_match("!안!", &e));
    }

    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource
    fn real_speech_mul_has_vendor_and_boat_words() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let s = Speeches::open(&dir).expect("speech.mul");
        assert!(
            s.len() > 100,
            "expected a real keyword table, got {}",
            s.len()
        );
        assert!(
            !s.keywords("vendor buy").is_empty(),
            "vendor buy should hit speech.mul"
        );
        assert!(
            !s.keywords("forward").is_empty(),
            "boat 'forward' should hit speech.mul"
        );
        assert!(
            !s.keywords("stop").is_empty(),
            "boat 'stop' should hit speech.mul"
        );
    }
}
