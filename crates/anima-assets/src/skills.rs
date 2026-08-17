//! `Skills.idx` + `skills.mul` — skill names and the HasAction flag.
//!
//! ClassicUO `SkillsLoader`: each idx entry points at a mul record whose first
//! byte is HasAction (nonzero = the skill has a "use" verb) and the rest is a
//! NUL-trimmed ASCII name. Ids are the encounter order of valid records
//! (`count++` only when `Length > 0`), which is what the 0x3A packet uses.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct SkillInfo {
    pub id: u16,
    pub name: String,
    pub has_action: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Skills {
    pub entries: Vec<SkillInfo>,
}

impl Skills {
    pub fn open(resource_dir: impl AsRef<Path>) -> std::io::Result<Skills> {
        let dir = resource_dir.as_ref();
        let idx = std::fs::read(dir.join("Skills.idx"))
            .or_else(|_| std::fs::read(dir.join("skills.idx")))?;
        let mut mul =
            File::open(dir.join("skills.mul")).or_else(|_| File::open(dir.join("Skills.mul")))?;
        Ok(Skills {
            entries: parse_skills(&idx, &mut mul),
        })
    }

    pub fn get(&self, id: u16) -> Option<&SkillInfo> {
        self.entries.iter().find(|s| s.id == id)
    }

    /// Compact JSON array for `GET /skillinfo.json`.
    pub fn to_json(&self) -> String {
        let mut out = String::from("[");
        for (i, s) in self.entries.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            let name = s.name.replace('\\', "\\\\").replace('"', "\\\"");
            out.push_str(&format!(
                "{{\"id\":{},\"name\":\"{}\",\"use\":{}}}",
                s.id, name, s.has_action
            ));
        }
        out.push(']');
        out
    }
}

fn parse_skills(idx: &[u8], mul: &mut File) -> Vec<SkillInfo> {
    let mut entries = Vec::new();
    let mut id = 0u16;
    let mut i = 0usize;
    while i + 8 <= idx.len() {
        let pos = u32::from_le_bytes([idx[i], idx[i + 1], idx[i + 2], idx[i + 3]]);
        let len = u32::from_le_bytes([idx[i + 4], idx[i + 5], idx[i + 6], idx[i + 7]]);
        i += 12; // idx records are 12 bytes (pos, len, extra)
        if pos == 0xFFFF_FFFF || len == 0 || len == 0xFFFF_FFFF {
            // Hole: ClassicUO `count++` only runs for `Length > 0`, so we
            // skip without consuming an id. A 1-byte record is HasAction
            // with an empty name and still consumes an id.
            continue;
        }
        if mul.seek(SeekFrom::Start(pos as u64)).is_err() {
            continue;
        }
        let mut buf = vec![0u8; len as usize];
        if mul.read_exact(&mut buf).is_err() {
            continue;
        }
        let has_action = buf[0] != 0;
        let name = std::str::from_utf8(&buf[1..])
            .unwrap_or("")
            .trim_end_matches('\0')
            .trim()
            .to_string();
        entries.push(SkillInfo {
            id,
            name,
            has_action,
        });
        id += 1;
    }
    entries
}

/// Parse from in-memory idx + mul bytes (tests).
pub fn parse_skills_bytes(idx: &[u8], mul: &[u8]) -> Vec<SkillInfo> {
    let mut entries = Vec::new();
    let mut id = 0u16;
    let mut i = 0usize;
    while i + 8 <= idx.len() {
        let pos = u32::from_le_bytes([idx[i], idx[i + 1], idx[i + 2], idx[i + 3]]);
        let len = u32::from_le_bytes([idx[i + 4], idx[i + 5], idx[i + 6], idx[i + 7]]);
        i += 12;
        if pos == 0xFFFF_FFFF || len == 0 || len == 0xFFFF_FFFF {
            continue;
        }
        let start = pos as usize;
        let end = start + len as usize;
        if end > mul.len() {
            continue;
        }
        let buf = &mul[start..end];
        let has_action = buf[0] != 0;
        let name = std::str::from_utf8(&buf[1..])
            .unwrap_or("")
            .trim_end_matches('\0')
            .trim()
            .to_string();
        entries.push(SkillInfo {
            id,
            name,
            has_action,
        });
        id += 1;
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_skills_reads_has_action_and_name() {
        // Two idx records: Alchemy (no use), Anatomy (HasAction).
        let mut mul = Vec::new();
        mul.push(0);
        mul.extend_from_slice(b"Alchemy\0");
        let alchemy_len = mul.len() as u32;
        let anatomy_pos = mul.len() as u32;
        mul.push(1);
        mul.extend_from_slice(b"Anatomy");
        let anatomy_len = (mul.len() as u32) - anatomy_pos;

        let mut idx = Vec::new();
        idx.extend_from_slice(&0u32.to_le_bytes());
        idx.extend_from_slice(&alchemy_len.to_le_bytes());
        idx.extend_from_slice(&0u32.to_le_bytes());
        idx.extend_from_slice(&anatomy_pos.to_le_bytes());
        idx.extend_from_slice(&anatomy_len.to_le_bytes());
        idx.extend_from_slice(&0u32.to_le_bytes());

        let entries = parse_skills_bytes(&idx, &mul);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, 0);
        assert_eq!(entries[0].name, "Alchemy");
        assert!(!entries[0].has_action);
        assert_eq!(entries[1].id, 1);
        assert_eq!(entries[1].name, "Anatomy");
        assert!(entries[1].has_action);
        let json = Skills { entries }.to_json();
        assert!(json.contains("\"name\":\"Alchemy\""));
        assert!(json.contains("\"use\":true"));
    }

    #[test]
    fn parse_skills_empty_name_still_consumes_an_id() {
        // ClassicUO count++ for every Length > 0 record, empty name included.
        let mut mul = Vec::new();
        mul.push(0);
        mul.extend_from_slice(b"Alchemy\0");
        let alchemy_len = mul.len() as u32;
        let hole_pos = mul.len() as u32;
        mul.push(0); // HasAction + empty name
        let hole_len = 1u32;
        let anatomy_pos = mul.len() as u32;
        mul.push(1);
        mul.extend_from_slice(b"Anatomy");
        let anatomy_len = (mul.len() as u32) - anatomy_pos;

        let mut idx = Vec::new();
        for (pos, len) in [
            (0u32, alchemy_len),
            (hole_pos, hole_len),
            (anatomy_pos, anatomy_len),
        ] {
            idx.extend_from_slice(&pos.to_le_bytes());
            idx.extend_from_slice(&len.to_le_bytes());
            idx.extend_from_slice(&0u32.to_le_bytes());
        }
        let entries = parse_skills_bytes(&idx, &mul);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[1].id, 1);
        assert!(entries[1].name.is_empty());
        assert_eq!(entries[2].id, 2);
        assert_eq!(entries[2].name, "Anatomy");
    }

    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource
    fn real_skills_mul_starts_with_alchemy() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let s = Skills::open(&dir).expect("skills.mul");
        assert!(
            s.entries.len() >= 50,
            "expected a full skill table, got {}",
            s.entries.len()
        );
        assert_eq!(s.entries[0].id, 0);
        assert_eq!(s.entries[0].name, "Alchemy");
        assert_eq!(s.get(1).map(|e| e.name.as_str()), Some("Anatomy"));
        assert!(s.get(1).is_some_and(|e| e.has_action));
    }
}
