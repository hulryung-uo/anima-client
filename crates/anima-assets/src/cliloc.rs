//! `Cliloc.enu` reader: the UO localized-string table.
//!
//! UO refers to most server-side text (item names, context-menu entries, system
//! messages, gump labels) by a numeric "cliloc" id rather than literal text. The
//! table that maps id → English text lives in `Cliloc.enu` in the data dir.
//!
//! Format (see ClassicUO `ClilocLoader`): a 6-byte header (`[u32][u16]`), then a
//! flat stream of records until EOF, each:
//! `[number u32 LE][flag u8][length u16 LE][text: `length` bytes UTF-8]`.
//!
//! (Newer 7.0.10.4+ clients can ship a BWT-compressed cliloc whose 4th byte is
//! `0x8E`; we don't ship that decompressor — the `Cliloc.enu` in this project's
//! data dir is the plain form.)

use std::collections::HashMap;
use std::io;
use std::path::Path;

/// The localized-string table (English), `id → text`.
pub struct Cliloc {
    entries: HashMap<u32, String>,
}

impl Cliloc {
    /// Open `Cliloc.enu` from `data_dir` and parse every record into the map.
    pub fn open(data_dir: impl AsRef<Path>) -> io::Result<Cliloc> {
        let data = std::fs::read(data_dir.as_ref().join("Cliloc.enu"))?;
        Ok(Self::parse(&data))
    }

    /// Parse a raw (uncompressed) cliloc buffer.
    fn parse(data: &[u8]) -> Cliloc {
        let mut entries = HashMap::new();
        // 6-byte header: [u32][u16].
        let mut p = 6usize;
        while p + 7 <= data.len() {
            let number = u32::from_le_bytes([data[p], data[p + 1], data[p + 2], data[p + 3]]);
            // data[p + 4] = flag (unused: original/custom/etc.)
            let length = u16::from_le_bytes([data[p + 5], data[p + 6]]) as usize;
            p += 7;
            if p + length > data.len() {
                break; // truncated record — stop rather than panic.
            }
            let text = String::from_utf8_lossy(&data[p..p + length]).into_owned();
            p += length;
            entries.insert(number, text);
        }
        Cliloc { entries }
    }

    /// The text for a cliloc id, if present.
    pub fn get(&self, id: u32) -> Option<&str> {
        self.entries.get(&id).map(String::as_str)
    }

    /// Resolve a localized message: look up the template for `id` and fill its
    /// `~N_...~` placeholders from the tab-separated `args`. Returns `None` when
    /// the id isn't in the table (the caller falls back to e.g. `#<id>`).
    ///
    /// See [`substitute`] for the placeholder rules.
    pub fn format(&self, id: u32, args: &str) -> Option<String> {
        self.get(id).map(|template| substitute(template, args))
    }

    /// Number of loaded entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Fill a cliloc template's `~N_...~` placeholders with tab-separated `args`.
///
/// Mirrors ClassicUO `ClilocLoader.Translate`: `args` is split on TAB `\t`
/// (leading tabs ignored). Each placeholder `~N_LABEL~` names a **1-based**
/// argument index (the digits before the first `_`, or the whole inner text if
/// there is no `_`); it's replaced by that argument. A placeholder whose index
/// is out of range or unparseable (the sequential/unnumbered form) is dropped,
/// so no raw `~...~` markers leak into the output. Text outside placeholders is
/// kept verbatim, so a template with no placeholders returns unchanged.
fn substitute(template: &str, args: &str) -> String {
    let arg = args.trim_start_matches('\t');
    let parts: Vec<&str> = if arg.is_empty() { Vec::new() } else { arg.split('\t').collect() };
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('~') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('~') {
            Some(close) => {
                let inner = &after[..close];
                let num = inner.split('_').next().unwrap_or("");
                if let Ok(n) = num.parse::<usize>() {
                    if let Some(v) = n.checked_sub(1).and_then(|i| parts.get(i)) {
                        out.push_str(v);
                    }
                }
                // unparseable / out-of-range → drop the placeholder entirely.
                rest = &after[close + 1..];
            }
            None => {
                // Unmatched '~' — keep it verbatim and stop scanning.
                out.push('~');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cliloc_with(id: u32, text: &str) -> Cliloc {
        let mut buf = vec![0u8; 6];
        buf.extend_from_slice(&id.to_le_bytes());
        buf.push(0); // flag
        buf.extend_from_slice(&(text.len() as u16).to_le_bytes());
        buf.extend_from_slice(text.as_bytes());
        Cliloc::parse(&buf)
    }

    #[test]
    fn substitutes_numbered_placeholders() {
        // In-order numbered form (the common case).
        assert_eq!(substitute("~1_NAME~ : ~2_VAL~", "Hastin\t42"), "Hastin : 42");
        // Out-of-order indices still pick the right arg.
        assert_eq!(substitute("~2_b~/~1_a~", "first\tsecond"), "second/first");
        // Leading tab(s) on the arg string are ignored.
        assert_eq!(substitute("You see ~1_val~ damage", "\t10"), "You see 10 damage");
        // A template with no placeholders is unchanged.
        assert_eq!(substitute("You have been damaged!", ""), "You have been damaged!");
        // Missing args / unnumbered placeholders are dropped (no raw ~...~ leaks).
        assert_eq!(substitute("[~1_X~]", ""), "[]");
        assert_eq!(substitute("a~b~c", "z"), "ac");
    }

    #[test]
    fn format_resolves_known_id_only() {
        let c = cliloc_with(1042762, "~1_AMT~ damage to ~2_NAME~");
        assert_eq!(c.format(1042762, "8\tan orc"), Some("8 damage to an orc".to_string()));
        assert_eq!(c.format(999999, "x"), None); // unknown id → caller falls back
    }


    #[test]
    fn parses_a_synthetic_table() {
        // header (6) + one record: id=3000000, flag=0, len=5, "hello".
        let mut buf = vec![0u8; 6];
        buf.extend_from_slice(&3_000_000u32.to_le_bytes());
        buf.push(0); // flag
        buf.extend_from_slice(&5u16.to_le_bytes());
        buf.extend_from_slice(b"hello");
        let c = Cliloc::parse(&buf);
        assert_eq!(c.get(3_000_000), Some("hello"));
        assert_eq!(c.get(1), None);
    }

    /// Requires local UO data at ~/dev/uo/uo-resource. Ignored by default so the
    /// suite runs without game files; run with `--ignored` to validate.
    #[test]
    #[ignore]
    fn reads_real_cliloc() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let c = Cliloc::open(&dir).expect("open Cliloc.enu");
        assert!(!c.is_empty(), "cliloc should have entries");
        // 3000000 is a low, always-present id ("You see: ~1_NAME~" range start).
        let text = c.get(3_000_000);
        println!("cliloc 3000000 = {text:?} ({} entries)", c.len());
        assert!(text.is_some(), "id 3000000 should resolve");
    }
}
