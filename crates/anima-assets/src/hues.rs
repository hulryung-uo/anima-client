//! `hues.mul` reader and sprite recoloring (UO "hues").
//!
//! UO recolors sprites by mapping a pixel's brightness to one of 32 gradient
//! colors held in a hue. The file is a flat array of groups (ClassicUO
//! `HuesLoader`): each group = `[header u32][8 × HuesBlock]`, and each block =
//! `[32 × u16 RGB1555 color][TableStart u16][TableEnd u16][name 20B]` (88 B).
//!
//! The hue index in packets is 1-based: `id = hue & 0x3FFF`, then `id - 1`
//! indexes the flat hue array (`group = (id-1) >> 3`, `entry = (id-1) % 8`).
//! Bit `0x8000` marks a *partial* hue (recolor only gray pixels).

use std::path::Path;

const BLOCK_COLORS: usize = 32;
const BLOCK: usize = BLOCK_COLORS * 2 + 2 + 2 + 20; // 88 bytes per hue entry
const GROUP: usize = 4 + 8 * BLOCK; // header + 8 entries = 708 bytes

/// All hues, each a 32-entry RGBA8 gradient (index 0 = hue 1).
pub struct Hues {
    /// `table[hue0][ramp]` = RGBA. `hue0` is the 0-based hue id.
    table: Vec<[[u8; 4]; BLOCK_COLORS]>,
}

/// RGB1555 (the 0x8000 "valid" bit is set for real colors) → RGBA8.
fn rgb1555(c: u16) -> [u8; 4] {
    let r = ((c >> 10) & 0x1F) as u8;
    let g = ((c >> 5) & 0x1F) as u8;
    let b = (c & 0x1F) as u8;
    // 5→8 bit expansion (replicate high bits into low), matching art.rs.
    [
        (r << 3) | (r >> 2),
        (g << 3) | (g >> 2),
        (b << 3) | (b >> 2),
        255,
    ]
}

impl Hues {
    pub fn open(data_dir: impl AsRef<Path>) -> std::io::Result<Hues> {
        let data = std::fs::read(data_dir.as_ref().join("hues.mul"))?;
        let groups = data.len() / GROUP;
        let mut table = Vec::with_capacity(groups * 8);
        for gi in 0..groups {
            let gbase = gi * GROUP + 4; // skip 4-byte group header
            for ei in 0..8 {
                let ebase = gbase + ei * BLOCK;
                let mut hue = [[0u8; 4]; BLOCK_COLORS];
                for (c, slot) in hue.iter_mut().enumerate() {
                    let o = ebase + c * 2;
                    let raw = u16::from_le_bytes([data[o], data[o + 1]]);
                    *slot = rgb1555(raw);
                }
                table.push(hue);
            }
        }
        Ok(Hues { table })
    }

    /// Number of hues loaded.
    pub fn count(&self) -> usize {
        self.table.len()
    }

    /// RGBA for a 1-based hue index at ramp position 0..31. The low 14 bits of
    /// `hue_index_1based` are the hue id; flags (e.g. 0x8000) are ignored here.
    /// Out-of-range hues/ramps return transparent black.
    pub fn color(&self, hue_index_1based: u16, ramp: u8) -> [u8; 4] {
        let id = (hue_index_1based & 0x3FFF) as usize;
        if id == 0 || id > self.table.len() {
            return [0, 0, 0, 0];
        }
        let ramp = (ramp as usize).min(BLOCK_COLORS - 1);
        self.table[id - 1][ramp]
    }
}

/// Recolor a decoded RGBA sprite frame in place with a UO hue.
///
/// `hue` is the packet-form value: `id = hue & 0x3FFF` (0 = no-op), and
/// `0x8000` flags a *partial* hue (only gray pixels — `r==g==b` — are
/// recolored). For each affected opaque pixel the ramp index is the pixel's
/// brightness (`max(r,g,b)` scaled to 0..31); RGB is replaced with the hue's
/// color while the original alpha is kept. Matches ClassicUO's hue translation.
pub fn apply_hue(img: &mut crate::art::Image, hues: &Hues, hue: u16) {
    let id = hue & 0x3FFF;
    if id == 0 {
        return;
    }
    let partial = hue & 0x8000 != 0;
    for px in img.rgba.chunks_exact_mut(4) {
        if px[3] == 0 {
            continue; // transparent
        }
        if partial && !(px[0] == px[1] && px[1] == px[2]) {
            continue; // partial hue: leave non-gray pixels untouched
        }
        let brightness = px[0].max(px[1]).max(px[2]) as u16;
        let ramp = (brightness * 31 / 255) as u8;
        let c = hues.color(id, ramp);
        px[0] = c[0];
        px[1] = c[1];
        px[2] = c[2];
        // keep original alpha (px[3])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource
    fn reads_real_hues() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let hues = Hues::open(&dir).expect("open hues");
        println!("hues loaded: {}", hues.count());
        assert!(hues.count() > 100, "expected many hues");
        // A known hue (1) should yield non-zero colors across its ramp.
        let any = (0..32).any(|r| {
            let c = hues.color(1, r);
            c[3] != 0 && (c[0] != 0 || c[1] != 0 || c[2] != 0)
        });
        assert!(any, "hue 1 should have visible colors");
    }
}
