//! Light-shape reader (`lightidx.mul` + `light.mul`) — the per-light bitmaps UO
//! uses for the glow a torch, lamp, campfire or spell casts at night. From
//! ClassicUO `LightsLoader`.
//!
//! A light is **not** a circle: `light.mul` holds ~100 hand-drawn greyscale
//! masks of different sizes and shapes, and each light-emitting graphic points
//! at one of them through its tiledata `Quality`/layer byte (ClassicUO
//! `AddLight`: `light.ID = data.Layer`). The index is the usual 12-byte
//! `(offset, length, extra)` triple, with the shape's width in the high half of
//! `extra` and its height in the low half.
//!
//! Each pixel is one byte of intensity. ClassicUO:
//!
//! ```text
//! if (val > 0x1F) val = ~val & 0x1F;   // negative lights are bit-inverted
//! rgb24 = (val << 19) | (val << 11) | (val << 3);
//! ```
//!
//! — i.e. `val * 8` in all three channels, a grey, and fully transparent where
//! `val == 0`. It draws those greys **additively** into a light buffer. This
//! renderer instead *subtracts* from a darkness overlay, so [`Lights::light`]
//! hands back white pixels with the intensity in the **alpha** channel: same
//! shape, same falloff, expressed for a compositor that erases rather than adds.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Mutex;

use crate::art::Image;

/// ClassicUO `LightsLoader.MAX_LIGHTS_DATA_INDEX_COUNT` — ids beyond this are
/// refused before the file is even consulted.
pub const MAX_LIGHTS: u32 = 100;

pub struct Lights {
    idx: Vec<u8>,
    mul: Mutex<File>,
}

impl Lights {
    pub fn open(resource_dir: impl AsRef<Path>) -> std::io::Result<Lights> {
        let dir = resource_dir.as_ref();
        Ok(Lights {
            idx: std::fs::read(dir.join("lightidx.mul"))?,
            mul: Mutex::new(File::open(dir.join("light.mul"))?),
        })
    }

    /// Decode light shape `id` to a white RGBA image whose alpha is the light's
    /// intensity. `None` for an out-of-range id, an empty index entry, or a
    /// zero-sized shape (`light.mul` has plenty of both).
    pub fn light(&self, id: u32) -> Option<Image> {
        if id >= MAX_LIGHTS {
            return None;
        }
        let o = id as usize * 12;
        if o + 12 > self.idx.len() {
            return None;
        }
        let u32_at = |i: usize| {
            u32::from_le_bytes([
                self.idx[i],
                self.idx[i + 1],
                self.idx[i + 2],
                self.idx[i + 3],
            ])
        };
        let pos = u32_at(o);
        let len = u32_at(o + 4);
        let extra = u32_at(o + 8);
        if pos == 0xFFFF_FFFF || len == 0 {
            return None;
        }
        // `extra` packs the shape's dimensions: width in the high 16 bits,
        // height in the low 16 (ClassicUO `UOFileMul.FillEntries`).
        let w = (extra >> 16) as usize;
        let h = (extra & 0xFFFF) as usize;
        if w == 0 || h == 0 || w * h > len as usize {
            return None;
        }

        let buf = {
            let mut f = self.mul.lock().ok()?;
            f.seek(SeekFrom::Start(pos as u64)).ok()?;
            let mut buf = vec![0u8; w * h];
            f.read_exact(&mut buf).ok()?;
            buf
        };

        let mut rgba = vec![0u8; w * h * 4];
        for (i, &raw) in buf.iter().enumerate() {
            // A light runs -31..31; the negative half arrives bit-inverted.
            let val = if raw > 0x1F { !raw & 0x1F } else { raw };
            if val == 0 {
                continue; // leave it fully transparent
            }
            let a = val << 3; // 0..31 -> 0..248, ClassicUO's `val << 3` per channel
            rgba[i * 4] = 0xFF;
            rgba[i * 4 + 1] = 0xFF;
            rgba[i * 4 + 2] = 0xFF;
            rgba[i * 4 + 3] = a;
        }
        Some(Image {
            width: w as u32,
            height: h as u32,
            rgba,
        })
    }
}

/// The six intensity curves ClassicUO shapes a coloured light's ramp with
/// (`LightColors.CreateLightTextures`). Index by the light's 0..31 intensity to
/// get the scaled 0..31 the channel actually uses: Standard is the identity, A
/// is small, B very small and dim, C full and flat, D medium-dim, E a halo.
const LIGHT_CURVES: [[u8; 32]; 6] = [
    [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
        25, 26, 27, 28, 29, 30, 31,
    ],
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 6, 8, 10, 12, 14, 16, 18, 20,
        22, 24, 26, 28,
    ],
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6,
        7, 8,
    ],
    [
        0, 1, 2, 4, 6, 8, 11, 14, 17, 20, 23, 26, 29, 30, 31, 31, 31, 31, 31, 31, 31, 31, 31, 31,
        31, 31, 31, 31, 31, 31, 31, 31,
    ],
    [
        0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 15, 17, 19,
        21, 23, 25, 27,
    ],
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 5, 10, 15, 20, 25, 30, 30, 18, 18,
        18, 18, 18, 18, 18,
    ],
];

/// One entry of ClassicUO's light-shader table: a base colour and a curve per
/// channel (`MakeDefaultShaders`). Everything it does not name is white on the
/// Standard curve.
struct LightShader {
    rgb: u32,
    curves: [usize; 3], // red, green, blue
}

/// `LightColors.MakeDefaultShaders`, transcribed. Index 0 is unused — ClassicUO
/// stores the colour as `index + 1` on the wire side and subtracts it back.
fn light_shader(color: u16) -> LightShader {
    const STD: usize = 0;
    const A: usize = 1;
    const B: usize = 2;
    const C: usize = 3;
    const D: usize = 4;
    const E: usize = 5;
    match color {
        1 => LightShader {
            rgb: 0x00FF00,
            curves: [STD, A, STD],
        }, // green small
        2 => LightShader {
            rgb: 0x7F7FFF,
            curves: [STD, STD, STD],
        }, // light blue
        6 => LightShader {
            rgb: 0xFF00FF,
            curves: [B, STD, A],
        }, // dark blue
        10 => LightShader {
            rgb: 0x3F3FFF,
            curves: [STD, STD, STD],
        }, // blue
        20 => LightShader {
            rgb: 0x00FF00,
            curves: [STD, STD, STD],
        }, // green
        30 => LightShader {
            rgb: 0xFF7F00,
            curves: [C, C, STD],
        }, // orange
        31 => LightShader {
            rgb: 0xFF7F00,
            curves: [A, A, STD],
        }, // orange small
        32 => LightShader {
            rgb: 0xFF00FF,
            curves: [STD, STD, STD],
        }, // purple
        40 => LightShader {
            rgb: 0xFF0000,
            curves: [STD, STD, STD],
        }, // red
        50 => LightShader {
            rgb: 0xFFFF00,
            curves: [STD, STD, STD],
        }, // yellow
        60 => LightShader {
            rgb: 0xFFFF00,
            curves: [A, A, STD],
        }, // yellow small
        61 => LightShader {
            rgb: 0xFFFF00,
            curves: [D, D, STD],
        }, // yellow medium
        62 => LightShader {
            rgb: 0xFFFFFF,
            curves: [D, D, D],
        }, // white medium
        63 => LightShader {
            rgb: 0xFFFFFF,
            curves: [E, E, E],
        }, // white small full
        _ => LightShader {
            rgb: 0xFFFFFF,
            curves: [STD, STD, STD],
        },
    }
}

/// Which colour a light-emitting graphic burns, from ClassicUO's `LightColors.
/// GetHue`. `None` = plain white, which is most of them.
///
/// The original is a switch followed by two nested range chains and a couple of
/// stragglers, evaluated **in that order** — a later group overwrites an earlier
/// one, which is why this is a sequence of assignments rather than one lookup.
/// (Its `ishue` flag is only ever set by a user-supplied `lightshaders.txt`
/// override file, which nothing here reads, so a colour is always a shader
/// index.)
pub fn light_color_for(graphic: u16) -> Option<u16> {
    let g = graphic;
    let mut color: Option<u16> = match g {
        0x088C => Some(31),
        0x0FAC => Some(30),
        0x0FB1 => Some(60),
        0x1647 => Some(61),
        0x19BB | 0x1F2B => Some(40),
        0x9F66 => Some(0),
        _ => None,
    };
    const CHAIN_A: &[(u16, u16, u16)] = &[
        (0x09FB, 0x0A14, 30),
        (0x0A15, 0x0A29, 0),
        (0x0B1A, 0x0B1F, 0),
        (0x0B20, 0x0B25, 0),
        (0x0B26, 0x0B28, 0),
        (0x0DE1, 0x0DEA, 31),
        (0x1849, 0x1850, 61),
        (0x1853, 0x185A, 61),
        (0x197A, 0x19A9, 60),
        (0x19AB, 0x19B6, 60),
        (0x1ECD, 0x1ED2, 1),
    ];
    for &(lo, hi, c) in CHAIN_A {
        if g >= lo && g <= hi {
            color = Some(c);
            break;
        }
    }
    if g == 0x1FD4 || g == 0x0F6C {
        color = Some(2);
    }
    const CHAIN_B: &[(u16, u16, u16)] = &[
        (0x0E2D, 0x0E30, 62),
        (0x0E31, 0x0E33, 40),
        (0x0E5C, 0x0E6A, 6),
        (0x12EE, 0x134D, 31),
        (0x306A, 0x329B, 31),
        (0x343B, 0x346C, 31),
        (0x3547, 0x354C, 31),
        (0x3914, 0x3929, 1),
        (0x3946, 0x3964, 6),
        (0x3967, 0x397A, 6),
        (0x398C, 0x399F, 31),
        (0x3E02, 0x3E0B, 1),
        (0x3E27, 0x3E3A, 31),
    ];
    let mut hit_b = false;
    for &(lo, hi, c) in CHAIN_B {
        if g >= lo && g <= hi {
            color = Some(c);
            hit_b = true;
            break;
        }
    }
    if !hit_b {
        color = match g {
            0x40FE => Some(40),
            0x40FF => Some(10),
            0x4100 => Some(20),
            0x4101 => Some(32),
            0x983B..=0x983D | 0x983F..=0x9841 => Some(30),
            _ => color,
        };
    }
    // ClassicUO returns false when nothing matched; colour 0 is a real answer
    // there (its sentinel is ushort::MAX), and means "white".
    color.filter(|&c| c != 0)
}

impl Lights {
    /// Light shape `id` with ClassicUO's colour `color` baked in: each pixel's
    /// intensity picks a column of that colour's ramp
    /// (`LightColors.CreateLightTextures`, `channel = curve[intensity] * base /
    /// 31`), and the intensity itself stays in the alpha channel.
    ///
    /// The renderer draws this additively over the night overlay, which is what
    /// ClassicUO's light buffer does with the same numbers.
    pub fn light_colored(&self, id: u32, color: u16) -> Option<Image> {
        let mut img = self.light(id)?;
        let shader = light_shader(color);
        let base = [
            (shader.rgb >> 16) & 0xFF,
            (shader.rgb >> 8) & 0xFF,
            shader.rgb & 0xFF,
        ];
        for px in img.rgba.chunks_exact_mut(4) {
            if px[3] == 0 {
                continue;
            }
            let val = (px[3] >> 3) as usize; // alpha 0..248 -> intensity 0..31
            for ch in 0..3 {
                let scaled = LIGHT_CURVES[shader.curves[ch]][val.min(31)] as u32;
                px[ch] = ((scaled * base[ch]) / 31) as u8;
            }
        }
        Some(img)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_past_the_table_are_refused_without_touching_the_file() {
        // The id bound comes before any file access, which is what keeps a
        // bogus tiledata layer byte from seeking into light.mul. Proved by
        // asking a Lights whose files do not exist: it never gets that far.
        let lights = Lights {
            idx: Vec::new(),
            mul: Mutex::new(File::open("/dev/null").unwrap()),
        };
        assert!(lights.light(MAX_LIGHTS).is_none());
        assert!(lights.light(u32::MAX).is_none());
        // And an in-range id with no index behind it is simply absent.
        assert!(lights.light(0).is_none());
    }

    #[test]
    fn light_colors_follow_classicuos_evaluation_order() {
        // The plain switch at the top.
        assert_eq!(light_color_for(0x088C), Some(31));
        assert_eq!(light_color_for(0x1647), Some(61));
        // Ranges from each of the two chains.
        assert_eq!(light_color_for(0x09FB), Some(30));
        assert_eq!(light_color_for(0x0A14), Some(30));
        assert_eq!(light_color_for(0x19A0), Some(60));
        assert_eq!(light_color_for(0x0E2E), Some(62)); // second chain
        assert_eq!(light_color_for(0x3920), Some(1)); // poison field
        assert_eq!(light_color_for(0x40FF), Some(10)); // tail switch
        assert_eq!(light_color_for(0x9840), Some(30));
        // Order matters: 0x0F6C is set to 2 *after* the first chain has run,
        // and the second chain does not contain it, so 2 must survive.
        assert_eq!(light_color_for(0x0F6C), Some(2));
        // Colour 0 is ClassicUO's "white" answer, not a colour.
        assert_eq!(light_color_for(0x9F66), None);
        assert_eq!(light_color_for(0x0A15), None);
        // And the overwhelming majority of graphics have no entry at all.
        assert_eq!(light_color_for(0x0001), None);
    }

    #[test]
    fn colored_ramp_scales_each_channel_by_its_own_curve() {
        // Colour 31 is orange (0xFF7F00) on curve A for red and green, Standard
        // for blue — and its blue base is 0, so blue stays 0 at every
        // intensity while red and green follow curve A.
        let shader = light_shader(31);
        assert_eq!(shader.rgb, 0xFF7F00);
        assert_eq!(LIGHT_CURVES[shader.curves[0]][31], 28); // curve A tops out at 28
        assert_eq!(LIGHT_CURVES[shader.curves[0]][8], 0); // …and is dark below half
                                                          // Standard is the identity, which is what an unnamed colour gets.
        let plain = light_shader(999);
        assert_eq!(plain.rgb, 0xFFFFFF);
        assert_eq!(LIGHT_CURVES[plain.curves[0]][17], 17);
    }

    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource/light.mul
    fn real_light_mul_colors_a_shape_without_touching_its_alpha() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let lights = Lights::open(dir).expect("open light.mul");
        let plain = lights.light(1).expect("light 1");
        let red = lights.light_colored(1, 40).expect("light 1 in red");
        assert_eq!(plain.width, red.width);
        // The mask is untouched; only the colour under it changed.
        for (p, r) in plain.rgba.chunks_exact(4).zip(red.rgba.chunks_exact(4)) {
            assert_eq!(p[3], r[3], "alpha (the shape) must be identical");
            if r[3] != 0 {
                assert_eq!([r[1], r[2]], [0, 0], "colour 40 is pure red");
                assert!(r[0] > 0);
            }
        }
    }

    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource/light.mul
    fn real_light_mul_decodes_shapes_with_graded_alpha() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let lights = Lights::open(dir).expect("open light.mul");
        let mut found = 0;
        for id in 0..MAX_LIGHTS {
            let Some(img) = lights.light(id) else {
                continue;
            };
            found += 1;
            assert!(img.width > 0 && img.height > 0);
            assert_eq!(img.rgba.len(), (img.width * img.height * 4) as usize);
            // Whatever is lit is white; the shape lives entirely in alpha.
            for px in img.rgba.chunks_exact(4) {
                if px[3] != 0 {
                    assert_eq!([px[0], px[1], px[2]], [0xFF, 0xFF, 0xFF]);
                }
            }
        }
        assert!(found > 20, "expected a table of light shapes, got {found}");
        // A real light is a gradient, not a disc: some pixel must be partly lit.
        let img = lights.light(1).expect("light 1");
        assert!(img.rgba.chunks_exact(4).any(|p| p[3] > 0 && p[3] < 248));
    }
}
