//! `tileart.uop` — Enhanced Client tile metadata (ClassicUO `TileArtLoader`).
//!
//! Version-4 records carry extra flags, stack-amount aliases, and a body→
//! appearance map used by paperdoll art (`PaperDollInteractable.GetAnimID`).
//! Loaded on demand per graphic; missing file → every lookup is `None`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use crate::uop::UopReader;

/// One tileart record, enough for stack aliases and paperdoll appearance.
#[derive(Debug, Clone, Default)]
pub struct TileArtInfo {
    pub tile_id: u32,
    pub flags: u64,
    /// `(min_amount, graphic)` pairs, sorted by amount ascending.
    pub stack_aliases: Vec<(u32, u32)>,
    /// Appearance type 0: wearer body → paperdoll anim id.
    pub appearance: HashMap<u32, u32>,
    pub appearance_types: usize,
}

impl TileArtInfo {
    /// ClassicUO `TryGetAppearance`: only type 0, and only when more than one
    /// appearance type was present.
    pub fn appearance_for(&self, mob_graphic: u32) -> Option<u32> {
        if self.appearance_types > 1 {
            self.appearance.get(&mob_graphic).copied()
        } else {
            None
        }
    }

    /// Highest alias whose `amount` is ≤ `qty`, else the original graphic.
    pub fn stack_graphic(&self, qty: u32) -> u32 {
        let mut g = self.tile_id;
        for &(n, id) in &self.stack_aliases {
            if qty >= n {
                g = id;
            }
        }
        g
    }
}

pub struct TileArt {
    uop: Option<UopReader>,
    cache: Mutex<HashMap<u32, Option<TileArtInfo>>>,
}

impl TileArt {
    pub fn open(resource_dir: impl AsRef<Path>) -> Self {
        let path = resource_dir.as_ref().join("tileart.uop");
        Self {
            uop: UopReader::open(&path).ok(),
            cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn get(&self, graphic: u32) -> Option<TileArtInfo> {
        {
            let cache = self.cache.lock().unwrap();
            if let Some(hit) = cache.get(&graphic) {
                return hit.clone();
            }
        }
        let info = self.load(graphic);
        self.cache.lock().unwrap().insert(graphic, info.clone());
        info
    }

    fn load(&self, graphic: u32) -> Option<TileArtInfo> {
        let uop = self.uop.as_ref()?;
        let path = format!("build/tileart/{graphic:08}.bin");
        let data = uop.by_path(&path)?;
        parse_tileart(&data)
    }
}

fn parse_tileart(data: &[u8]) -> Option<TileArtInfo> {
    if data.len() < 2 {
        return None;
    }
    let version = u16::from_le_bytes([data[0], data[1]]);
    if version != 4 {
        return None;
    }
    let mut p = 2usize;
    let _rd16 = |p: &mut usize| -> Option<u16> {
        let o = *p;
        *p += 2;
        data.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
    };
    let rd32 = |p: &mut usize| -> Option<u32> {
        let o = *p;
        *p += 4;
        data.get(o..o + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let rd64 = |p: &mut usize| -> Option<u64> {
        let o = *p;
        *p += 8;
        data.get(o..o + 8)
            .map(|b| u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    };
    let rd8 = |p: &mut usize| -> Option<u8> {
        let o = *p;
        *p += 1;
        data.get(o).copied()
    };
    let _string_dict = rd32(&mut p)?;
    let tile_id = rd32(&mut p)?;
    let _ = rd8(&mut p)?;
    let _ = rd8(&mut p)?;
    let _ = rd32(&mut p)?;
    let _ = rd32(&mut p)?;
    let _ = rd32(&mut p)?;
    let _ = rd32(&mut p)?;
    let _ = rd32(&mut p)?;
    let _body = rd32(&mut p)?;
    let _ = rd8(&mut p)?;
    let _ = rd32(&mut p)?;
    let _ = rd32(&mut p)?;
    let _ = rd32(&mut p)?;
    let _ = rd32(&mut p)?;
    let _ = rd32(&mut p)?;
    let flags = rd64(&mut p)?;
    let _flags2 = rd64(&mut p)?;
    let _facing = rd32(&mut p)?;
    for _ in 0..12 {
        let _ = rd32(&mut p)?; // two 6-int bounding boxes
    }
    let prop_count = rd8(&mut p)? as usize;
    for _ in 0..prop_count {
        let _ = rd8(&mut p)?;
        let _ = rd32(&mut p)?;
    }
    let prop_count2 = rd8(&mut p)? as usize;
    for _ in 0..prop_count2 {
        let _ = rd8(&mut p)?;
        let _ = rd32(&mut p)?;
    }
    let stack_n = rd32(&mut p)? as usize;
    let mut stack_aliases = Vec::with_capacity(stack_n.min(32));
    for _ in 0..stack_n {
        let amount = rd32(&mut p)?;
        let id = rd32(&mut p)?;
        stack_aliases.push((amount, id));
    }
    stack_aliases.sort_by_key(|(n, _)| *n);
    let appearance_n = rd32(&mut p)? as usize;
    let mut appearance = HashMap::new();
    let mut appearance_types = 0usize;
    for _ in 0..appearance_n {
        let sub_type = rd8(&mut p)?;
        if sub_type == 1 {
            let _ = rd8(&mut p)?;
            let _ = rd32(&mut p)?;
        } else {
            appearance_types += 1;
            let sub_count = rd32(&mut p)? as usize;
            for _ in 0..sub_count {
                let val = rd32(&mut p)?;
                let anim = rd32(&mut p)?;
                let offset = val / 1000;
                let body = val % 1000;
                if sub_type == 0 {
                    appearance.insert(body, anim + offset);
                }
            }
        }
    }
    Some(TileArtInfo {
        tile_id,
        flags,
        stack_aliases,
        appearance,
        appearance_types,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsupported_version() {
        let mut data = vec![0u8; 8];
        data[0] = 3; // version 3
        assert!(parse_tileart(&data).is_none());
    }

    #[test]
    fn stack_graphic_picks_highest_eligible_alias() {
        let info = TileArtInfo {
            tile_id: 0xEED,
            stack_aliases: vec![(2, 0xEEE), (6, 0xEEF)],
            ..Default::default()
        };
        assert_eq!(info.stack_graphic(1), 0xEED);
        assert_eq!(info.stack_graphic(2), 0xEEE);
        assert_eq!(info.stack_graphic(5), 0xEEE);
        assert_eq!(info.stack_graphic(6), 0xEEF);
    }

    #[test]
    fn appearance_requires_more_than_one_type() {
        let mut info = TileArtInfo {
            appearance_types: 1,
            appearance: HashMap::from([(400, 99)]),
            ..Default::default()
        };
        assert_eq!(info.appearance_for(400), None);
        info.appearance_types = 2;
        assert_eq!(info.appearance_for(400), Some(99));
    }
}
