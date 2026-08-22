//! `mapdif` / `stadif` map-diff files (ClassicUO `MapLoader.ApplyPatches`).
//!
//! Applied when the shard sends GeneralInfo `0xBF` sub `0x18`. Each facet has
//! a list-of-block-indices file (`mapdiflN.mul` / `stadiflN.mul`) and the
//! replacement payload (`mapdifN.mul` / `stadifN.mul` + `stadifiN.mul`).
//! Missing files mean that facet simply has no diffs — the base map stands.

use std::path::Path;

/// One facet's optional land + statics diff tables.
#[derive(Default)]
pub struct MapDiffs {
    /// `mapdiflN`: little-endian u32 block indices, in apply order.
    pub map_list: Vec<u32>,
    /// `mapdifN`: 196-byte map blocks (4-byte header + 64 × 3).
    pub map_data: Vec<u8>,
    pub sta_list: Vec<u32>,
    /// `stadifiN`: 12-byte staidx records (pos, len, extra).
    pub sta_idx: Vec<u8>,
    /// `stadifN`: concatenated 7-byte static records.
    pub sta_data: Vec<u8>,
}

impl MapDiffs {
    pub fn load(resource_dir: impl AsRef<Path>, facet: u8) -> Self {
        let dir = resource_dir.as_ref();
        let read_u32s = |name: &str| -> Vec<u32> {
            let Ok(buf) = std::fs::read(dir.join(name)) else {
                return Vec::new();
            };
            buf.chunks_exact(4)
                .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect()
        };
        let read = |name: &str| std::fs::read(dir.join(name)).unwrap_or_default();
        Self {
            map_list: read_u32s(&format!("mapdifl{facet}.mul")),
            map_data: read(&format!("mapdif{facet}.mul")),
            sta_list: read_u32s(&format!("stadifl{facet}.mul")),
            sta_idx: read(&format!("stadifi{facet}.mul")),
            sta_data: read(&format!("stadif{facet}.mul")),
        }
    }

    pub fn has_any(&self) -> bool {
        !self.map_list.is_empty() || !self.sta_list.is_empty()
    }

    /// Land block `i` in the diff file (ClassicUO `sizeof(MapBlock)` = 196).
    pub fn map_block(&self, i: usize) -> Option<&[u8]> {
        let off = i.checked_mul(196)?;
        self.map_data.get(off..off + 196)
    }

    /// Statics for diff-list entry `i`: `(offset_in_stadif, byte_len)`.
    pub fn sta_index(&self, i: usize) -> Option<(u32, u32)> {
        let off = i.checked_mul(12)?;
        let b = self.sta_idx.get(off..off + 12)?;
        let pos = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        let len = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
        if pos == 0xFFFF_FFFF || len == 0 {
            return Some((0xFFFF_FFFF, 0));
        }
        Some((pos, len))
    }
}

/// Counts from `0xBF/0x18`: ClassicUO reads `(mapPatches, staticPatches)` per
/// facet, up to 6 maps. ServUO's `MapPatches` writes four facets.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MapPatchCounts {
    /// `(land_patches, static_patches)` per facet index.
    pub facets: Vec<(u32, u32)>,
}

impl MapPatchCounts {
    /// Parse the 0xBF/0x18 payload after the subcommand word.
    /// Layout (ClassicUO `ApplyPatches`): `[count u32 BE]{ [map u32 BE][statics u32 BE] }×count`.
    pub fn parse_bf18(body: &[u8]) -> Self {
        if body.len() < 4 {
            return Self::default();
        }
        let n = u32::from_be_bytes([body[0], body[1], body[2], body[3]]) as usize;
        let n = n.min(6);
        let mut facets = Vec::with_capacity(n);
        let mut p = 4usize;
        for _ in 0..n {
            if p + 8 > body.len() {
                break;
            }
            let map = u32::from_be_bytes([body[p], body[p + 1], body[p + 2], body[p + 3]]);
            let sta = u32::from_be_bytes([body[p + 4], body[p + 5], body[p + 6], body[p + 7]]);
            facets.push((map, sta));
            p += 8;
        }
        Self { facets }
    }

    pub fn for_facet(&self, facet: u8) -> (u32, u32) {
        self.facets.get(facet as usize).copied().unwrap_or((0, 0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bf18_four_facets() {
        let mut b = Vec::new();
        b.extend_from_slice(&4u32.to_be_bytes());
        for i in 0..4u32 {
            b.extend_from_slice(&(i + 1).to_be_bytes()); // map
            b.extend_from_slice(&(i * 10).to_be_bytes()); // statics
        }
        let c = MapPatchCounts::parse_bf18(&b);
        assert_eq!(c.facets.len(), 4);
        assert_eq!(c.for_facet(0), (1, 0));
        assert_eq!(c.for_facet(2), (3, 20));
        assert_eq!(c.for_facet(9), (0, 0));
    }
}
