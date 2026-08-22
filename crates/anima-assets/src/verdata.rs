//! `verdata.mul` patch table (ClassicUO `VerdataLoader` + `UOFileManager` apply).
//!
//! FileIDs: 0 map, 2 statics, 4 art, 12 gumps, 14 multi, 16 skills, 30 tiledata,
//! 31 animdata. Modern clients (CV ≥ 5.0.0a) ship without this file; `open`
//! then returns an empty table and every apply is a no-op.

use std::path::Path;

/// One `UOFileIndex5D` record: file, block, position, length, extra (gump size).
#[derive(Debug, Clone, Copy)]
pub struct VerdataPatch {
    pub file_id: u32,
    pub block_id: u32,
    pub position: u32,
    pub length: u32,
    pub extra: u32,
}

pub struct Verdata {
    data: Vec<u8>,
    patches: Vec<VerdataPatch>,
}

impl Verdata {
    /// Open `verdata.mul` if present; an absent file is an empty patch table,
    /// not an error — ClassicUO does the same (`VerdataLoader.Load`).
    pub fn open(resource_dir: impl AsRef<Path>) -> Self {
        let path = resource_dir.as_ref().join("verdata.mul");
        match std::fs::read(&path) {
            Ok(data) => Self::parse(data),
            Err(_) => Self {
                data: Vec::new(),
                patches: Vec::new(),
            },
        }
    }

    pub(crate) fn parse(data: Vec<u8>) -> Self {
        if data.len() < 4 {
            return Self {
                data,
                patches: Vec::new(),
            };
        }
        let n = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        let mut patches = Vec::with_capacity(n.min(4096));
        let mut p = 4usize;
        for _ in 0..n {
            if p + 20 > data.len() {
                break;
            }
            patches.push(VerdataPatch {
                file_id: u32::from_le_bytes([data[p], data[p + 1], data[p + 2], data[p + 3]]),
                block_id: u32::from_le_bytes([data[p + 4], data[p + 5], data[p + 6], data[p + 7]]),
                position: u32::from_le_bytes([
                    data[p + 8],
                    data[p + 9],
                    data[p + 10],
                    data[p + 11],
                ]),
                length: u32::from_le_bytes([
                    data[p + 12],
                    data[p + 13],
                    data[p + 14],
                    data[p + 15],
                ]),
                extra: u32::from_le_bytes([data[p + 16], data[p + 17], data[p + 18], data[p + 19]]),
            });
            p += 20;
        }
        Self { data, patches }
    }

    pub fn is_empty(&self) -> bool {
        self.patches.is_empty()
    }

    pub fn patches(&self) -> &[VerdataPatch] {
        &self.patches
    }

    /// Slice of the mul at `patch.position` of `patch.length` bytes.
    pub fn bytes(&self, patch: VerdataPatch) -> Option<&[u8]> {
        let start = patch.position as usize;
        let end = start.checked_add(patch.length as usize)?;
        self.data.get(start..end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_is_empty_table() {
        let v = Verdata::parse(vec![]);
        assert!(v.is_empty());
    }

    #[test]
    fn parses_a_zero_count_header() {
        let v = Verdata::parse(0u32.to_le_bytes().to_vec());
        assert!(v.is_empty());
    }

    #[test]
    fn parses_one_tiledata_patch() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&1u32.to_le_bytes());
        buf.extend_from_slice(&30u32.to_le_bytes()); // file_id tiledata
        buf.extend_from_slice(&7u32.to_le_bytes()); // block
        buf.extend_from_slice(&24u32.to_le_bytes()); // position (after header+record)
        buf.extend_from_slice(&4u32.to_le_bytes()); // length
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&[9, 8, 7, 6]); // payload at offset 24
        let v = Verdata::parse(buf);
        assert_eq!(v.patches().len(), 1);
        assert_eq!(v.patches()[0].file_id, 30);
        assert_eq!(v.patches()[0].block_id, 7);
        assert_eq!(v.bytes(v.patches()[0]), Some(&[9, 8, 7, 6][..]));
    }
}
