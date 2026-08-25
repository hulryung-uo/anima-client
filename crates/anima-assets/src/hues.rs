//! `hues.mul` reader and sprite recoloring (UO "hues").
//!
//! UO recolors sprites by mapping a pixel's **5-bit red** field to one of 32
//! gradient colors held in a hue (ClassicUO `GetColor16` uses `(c >> 10) &
//! 0x1F` on the original 16-bit colour; after 5→8 expansion that is
//! `px[0] >> 3`). The file is a flat array of groups (ClassicUO `HuesLoader`):
//! each group = `[header u32][8 × HuesBlock]`, and each block =
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
/// **5-bit red** field — ClassicUO `GetColor16` uses `(c >> 10) & 0x1F` on the
/// original 16-bit colour. After our 5→8 expansion (`(r << 3) | (r >> 2)`)
/// that is `px[0] >> 3`, not a 0..255 rescale, which would map dark reds
/// (5-bit 1 → 8-bit 8) onto ramp 0.
pub fn apply_hue(img: &mut crate::art::Image, hues: &Hues, hue: u16) {
    apply_hue_channel(img, hues, hue, false)
}

/// As [`apply_hue`], but `effect` selects the GREEN channel as the ramp index.
///
/// ClassicUO's shader has two hued branches and they differ in exactly this:
/// `HUED`/`PARTIAL_HUED` do `get_rgb(color.r, hue)` while `EFFECT_HUED` does
/// `get_rgb(color.g, hue)` (`IsometricWorld.fx:119` vs `:161-164`), and effect
/// art goes through the second — `GameEffectView` passes `effect: true` to
/// `GetHueVector`. Effect art is mostly coloured rather than greyscale, so
/// `r != g` and the wrong channel picks a different step of the ramp: a hued
/// fireball comes out at the wrong brightness. On a grey source the two agree
/// exactly, which is why plenty of effects looked right anyway.
pub fn apply_hue_channel(img: &mut crate::art::Image, hues: &Hues, hue: u16, effect: bool) {
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
        let ramp = if effect { px[1] >> 3 } else { px[0] >> 3 };
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

    fn test_hues(ramp: [[u8; 4]; BLOCK_COLORS]) -> Hues {
        Hues { table: vec![ramp] }
    }

    #[test]
    fn apply_hue_indexes_ramp_by_red_channel() {
        // Ramp slot i is tagged in green so we can see which one was picked.
        let mut ramp = [[0u8; 4]; BLOCK_COLORS];
        for (i, slot) in ramp.iter_mut().enumerate() {
            *slot = [0, i as u8, 0, 255];
        }
        let hues = test_hues(ramp);
        let mut img = crate::art::Image {
            width: 1,
            height: 1,
            rgba: vec![255, 10, 10, 255], // red = 255 → ramp 31
        };
        apply_hue(&mut img, &hues, 1);
        assert_eq!(img.rgba[1], 31, "red 255 must pick the last ramp slot");
        // 5-bit red 1 expands to 8-bit 8 (`(r << 3) | (r >> 2)`); >> 3 recovers 1.
        // `* 31 / 255` would pick ramp 0.
        img.rgba = vec![8, 0, 0, 255];
        apply_hue(&mut img, &hues, 1);
        assert_eq!(img.rgba[1], 1, "5-bit red 1 must pick ramp 1, not 0");
        // Green-only pixel: red=0 → ramp 0, not brightness (which would be 31).
        img.rgba = vec![0, 255, 0, 255];
        apply_hue(&mut img, &hues, 1);
        assert_eq!(img.rgba[1], 0, "ramp follows red, not max(r,g,b)");
    }

    /// Effect art takes the ramp index from GREEN, not red — ClassicUO's shader
    /// has two hued branches and that is the whole difference between them
    /// (`IsometricWorld.fx:161-164` vs `:119`). On a grey pixel the two agree,
    /// which is why the wrong channel went unnoticed for so long.
    #[test]
    fn effect_hue_indexes_ramp_by_green_channel() {
        // Tag each ramp slot in BLUE this time, so the assertion cannot be
        // satisfied by the channel being read.
        let mut ramp = [[0u8; 4]; BLOCK_COLORS];
        for (i, slot) in ramp.iter_mut().enumerate() {
            *slot = [0, 0, i as u8, 255];
        }
        let hues = test_hues(ramp);

        // r and g deliberately disagree: red 255 (ramp 31) vs green 8 (ramp 1).
        let px = vec![255, 8, 0, 255];

        let mut sprite = crate::art::Image {
            width: 1,
            height: 1,
            rgba: px.clone(),
        };
        apply_hue_channel(&mut sprite, &hues, 1, false);
        assert_eq!(sprite.rgba[2], 31, "sprite art follows red");

        let mut effect = crate::art::Image {
            width: 1,
            height: 1,
            rgba: px,
        };
        apply_hue_channel(&mut effect, &hues, 1, true);
        assert_eq!(effect.rgba[2], 1, "effect art follows green");

        // A grey source is the case where both rules agree — the reason plenty
        // of hued effects looked correct under the wrong channel.
        for effect in [false, true] {
            let mut grey = crate::art::Image {
                width: 1,
                height: 1,
                rgba: vec![80, 80, 80, 255],
            };
            apply_hue_channel(&mut grey, &hues, 1, effect);
            assert_eq!(grey.rgba[2], 10, "grey agrees either way (effect={effect})");
        }
    }

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
