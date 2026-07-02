//! Legacy mobile animation reader (`anim.idx` + `anim.mul`).
//!
//! UO bodies animate via groups (Walk=0, Stand=4 for people; Walk=0, Stand=2 for
//! monsters) × 5 stored directions (the other 3 are mirrored). Each (group,dir)
//! is one idx entry → a palette + frames; each frame is RLE over a 256-color
//! palette. Ported from ClassicUO `AnimationsLoader` (legacy MUL path).
//!
//! Body coverage: people bodies (human 400/401, elf, gargoyle) use the high
//! formula; everything else uses the monster formula. `Body.def` remapping
//! ([`Anim::remap`]) redirects exotic body ids to a real animation body (+ a
//! fallback hue) so they resolve instead of falling back to a marker.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Mutex;

use crate::art::Image;

/// People animation groups (35 groups × 5 dirs = 175 entries/body): Stand=4.
pub const PEOPLE_WALK: u8 = 0;
pub const PEOPLE_STAND: u8 = 4;
/// Animal groups (13 groups × 5 = 65 entries/body): Walk=0, Run=1, Stand=2.
pub const ANIMAL_STAND: u8 = 2;
/// Monster/"high" groups (22 groups × 5 = 110 entries/body): Walk=0, Stand=1.
pub const MONSTER_STAND: u8 = 1;

/// One legacy animation file pair (`animN.idx` + `animN.mul`).
struct AnimFile {
    idx: Vec<u8>,
    mul: Mutex<File>,
}

pub struct Anim {
    /// Animation files indexed by ClassicUO file index: `[0]` = `anim.mul`, `[1]`
    /// = `anim2.mul`, … `[4]` = `anim5.mul`. `Bodyconv.def` redirects a body into
    /// one of `[1..]`; `None` when that expansion's file isn't installed.
    files: Vec<Option<AnimFile>>,
    /// `Body.def` remap: exotic body id → (real animation body, fallback hue).
    bodydef: HashMap<u16, (u16, u16)>,
    /// `Bodyconv.def` redirect: body id → (file index 1..=4, graphic in that file).
    bodyconv: HashMap<u16, (u8, u16)>,
    /// `mobtypes.txt` type: body id → group kind (0 = monster/high, 1 = animal/low,
    /// 2 = people). Authoritative over the graphic-range heuristic when present.
    mobtypes: HashMap<u16, u8>,
}

impl Anim {
    pub fn open(resource_dir: impl AsRef<Path>) -> std::io::Result<Anim> {
        let dir = resource_dir.as_ref();
        // File 0 (anim.mul) is mandatory; anim2..anim5 are optional expansion files.
        let open_file = |i: usize| -> Option<AnimFile> {
            let suffix = if i == 0 { String::new() } else { (i + 1).to_string() };
            let idx = std::fs::read(dir.join(format!("anim{suffix}.idx"))).ok()?;
            let mul = File::open(dir.join(format!("anim{suffix}.mul"))).ok()?;
            Some(AnimFile { idx, mul: Mutex::new(mul) })
        };
        if open_file(0).is_none() {
            // Force the same error surface as before when anim.mul/idx are missing.
            std::fs::read(dir.join("anim.idx"))?;
            File::open(dir.join("anim.mul"))?;
        }
        let files: Vec<Option<AnimFile>> = (0..5).map(open_file).collect();
        Ok(Anim {
            files,
            // Body.def is optional (older shards lack it); absent → empty map (no remap).
            bodydef: std::fs::read_to_string(dir.join("Body.def"))
                .map(|t| parse_body_def(&t))
                .unwrap_or_default(),
            bodyconv: std::fs::read_to_string(dir.join("Bodyconv.def"))
                .map(|t| parse_body_conv(&t))
                .unwrap_or_default(),
            mobtypes: std::fs::read_to_string(dir.join("mobtypes.txt"))
                .map(|t| parse_mob_types(&t))
                .unwrap_or_default(),
        })
    }

    /// Apply `Body.def` remapping: return the real animation `(body, hue)` to draw
    /// for `body`. Faithful to ClassicUO `AnimationsLoader.ReplaceBody`: an exotic
    /// body is redirected to a base creature plus a fallback hue; the caller uses
    /// that hue only when the mobile has none of its own. Unmapped → `(body, 0)`.
    pub fn remap(&self, body: u16) -> (u16, u16) {
        self.bodydef.get(&body).copied().unwrap_or((body, 0))
    }

    /// Resolve `Bodyconv.def`: return the `(file index, graphic)` to read `body`
    /// from. Bodies in an expansion live in `anim2..anim5`; everything else stays
    /// in `anim.mul` as `(0, body)`. A redirect whose file isn't installed is
    /// ignored (falls back to the base file), matching ClassicUO's null-file skip.
    fn resolve(&self, body: u16) -> (usize, u16) {
        match self.bodyconv.get(&body) {
            Some(&(fi, g)) if self.files.get(fi as usize).is_some_and(Option::is_some) => {
                (fi as usize, g)
            }
            _ => (0, body),
        }
    }

    /// Byte-independent idx block for (graphic, group, animDir 0..4) given the group
    /// `kind` (0 = monster/high, 1 = animal/low, 2 = people). Each kind has a fixed
    /// group count and section offset (ClassicUO `Calculate*GroupOffset`):
    ///   monster/high: 22 groups × 5 = 110/body, base = g*110
    ///   animal/low:   13 groups × 5 =  65/body, base = 22000+(g-200)*65
    ///   people:       35 groups × 5 = 175/body, base = 35000+(g-400)*175
    /// Returns `None` if the computed block is negative (never valid).
    fn block(graphic: u16, group: u8, dir: u8, kind: u8) -> Option<usize> {
        let g = graphic as i64;
        let base = match kind {
            0 => g * 110,
            1 => (g - 200) * 65 + 22000,
            _ => (g - 400) * 175 + 35000,
        };
        let block = base + group as i64 * 5 + dir as i64;
        (block >= 0).then_some(block as usize)
    }

    /// Locate the idx entry for (body, group, animDir 0..4): resolve `Bodyconv.def`
    /// (file + graphic), pick the group `kind` (`mobtypes.txt` or graphic range),
    /// compute the block, and read `(file index, pos, size)` from that file's idx.
    fn entry(&self, body: u16, group: u8, dir: u8) -> Option<(usize, u32, u32)> {
        let (fi, graphic) = self.resolve(body);
        let kind = self.offset_kind(body, graphic, fi);
        let file = self.files.get(fi)?.as_ref()?;
        let o = Self::block(graphic, group, dir, kind)?.checked_mul(12)?;
        if o + 8 > file.idx.len() {
            return None;
        }
        let pos = u32le(&file.idx, o);
        let size = u32le(&file.idx, o + 4);
        if pos == 0xFFFF_FFFF || size == 0 || size == 0xFFFF_FFFF {
            return None;
        }
        Some((fi, pos, size))
    }

    /// Read `size` bytes at `pos` from animation file `fi`.
    fn read_block(&self, fi: usize, pos: u32, size: u32) -> Option<Vec<u8>> {
        let file = self.files.get(fi)?.as_ref()?;
        let mut f = file.mul.lock().ok()?;
        f.seek(SeekFrom::Start(pos as u64)).ok()?;
        let mut buf = vec![0u8; size as usize];
        f.read_exact(&mut buf).ok()?;
        Some(buf)
    }

    /// Animation group kind for `body`: 0 = monster (high), 1 = animal (low), 2 =
    /// people. Uses `mobtypes.txt` when it covers the body (authoritative), else the
    /// graphic-range heuristic of the file the body resolves into. This is the SAME
    /// kind the reader uses to pick the idx offset, so a renderer that fetches
    /// animations by group number should derive its group numbers from this value
    /// (not from the raw body range) to stay consistent with the file layout.
    pub fn anim_type(&self, body: u16) -> u8 {
        let (fi, graphic) = self.resolve(body);
        self.offset_kind(body, graphic, fi)
    }

    /// The default standing group for a body (varies by kind: monster 1, animal 2,
    /// people 4). Walk is group 0 for every kind.
    pub fn stand_group(&self, body: u16) -> u8 {
        match self.anim_type(body) {
            0 => MONSTER_STAND,
            1 => ANIMAL_STAND,
            _ => PEOPLE_STAND,
        }
    }

    /// Group kind (0/1/2) used to compute the idx offset: `mobtypes.txt` if it
    /// covers the (Body.def-remapped) `body`, else the per-file graphic-range
    /// heuristic. `graphic`/`file_index` come from [`Self::resolve`] (Bodyconv).
    fn offset_kind(&self, body: u16, graphic: u16, file_index: usize) -> u8 {
        match self.mobtypes.get(&body) {
            Some(&kind) => kind,
            None => type_by_graphic(graphic, file_index),
        }
    }

    /// Number of frames in (body, group, UO-direction 0..7), or `None` if absent.
    pub fn frame_count(&self, body: u16, group: u8, dir8: u8) -> Option<usize> {
        let (dir, _) = map_dir(dir8);
        let (fi, pos, size) = self.entry(body, group, dir)?;
        if size < 516 {
            return None;
        }
        let file = self.files.get(fi)?.as_ref()?;
        let mut f = file.mul.lock().ok()?;
        f.seek(SeekFrom::Start(pos as u64 + 512)).ok()?; // skip palette
        let mut b = [0u8; 4];
        f.read_exact(&mut b).ok()?;
        Some(u32::from_le_bytes(b) as usize)
    }

    /// Draw-center `(cx, cy)` for every frame of (body, group, dir) — the cheap
    /// header-only read (no pixel decode) the renderer needs to *position* each
    /// part. Mirror-adjusted to match [`Self::frame`]'s already-flipped image.
    pub fn frame_centers(&self, body: u16, group: u8, dir8: u8) -> Option<Vec<(i16, i16)>> {
        let (dir, mirror) = map_dir(dir8);
        let (fi, pos, size) = self.entry(body, group, dir)?;
        if size < 516 {
            return None;
        }
        let buf = self.read_block(fi, pos, size)?;
        let frame_count = u32le(&buf, 512) as usize;
        let mut out = Vec::with_capacity(frame_count);
        for i in 0..frame_count {
            if 516 + i * 4 + 4 > buf.len() {
                break;
            }
            let foff = u32le(&buf, 516 + i * 4) as usize;
            let p = 512 + foff;
            if p + 8 > buf.len() {
                out.push((0, 0));
                continue;
            }
            let cx = i16le(&buf, p);
            let cy = i16le(&buf, p + 2);
            let w = i16le(&buf, p + 4);
            out.push((if mirror { w - cx } else { cx }, cy));
        }
        Some(out)
    }

    /// Decode one frame of (body, group, UO-direction 0..7, frame_idx).
    /// Returns the RGBA image (already horizontally mirrored when the direction
    /// requires it) plus the frame's draw-center `(cx, cy)`: ClassicUO draws the
    /// `width×height` bitmap with its top-left at `(screenX - cx, screenY - height
    /// - cy)`, so the caller MUST honor the center to align multi-part mobiles
    /// (body + equipment) and especially a rider onto a mount. `cx` is already
    /// adjusted for the mirror (`width - cx`). `None` if absent/undecodable.
    pub fn frame(&self, body: u16, group: u8, dir8: u8, frame_idx: usize) -> Option<(Image, i16, i16)> {
        let (dir, mirror) = map_dir(dir8);
        let (fi, pos, size) = self.entry(body, group, dir)?;

        let buf = self.read_block(fi, pos, size)?;
        if buf.len() < 516 {
            return None;
        }

        // palette: 256 × u16 ARGB1555
        let pal = |i: u8| u16le(&buf, i as usize * 2);
        let data_start = 512usize;
        let frame_count = u32le(&buf, 512) as usize;
        if frame_idx >= frame_count {
            return None;
        }
        let foff = u32le(&buf, 516 + frame_idx * 4) as usize;
        let mut p = data_start + foff;
        if p + 8 > buf.len() {
            return None;
        }

        let center_x = i16le(&buf, p);
        let center_y = i16le(&buf, p + 2);
        let width = i16le(&buf, p + 4) as i32;
        let height = i16le(&buf, p + 6) as i32;
        p += 8;
        if width <= 0 || height <= 0 || width > 512 || height > 512 {
            return None;
        }
        let (w, h) = (width as usize, height as usize);
        let mut rgba = vec![0u8; w * h * 4];

        loop {
            if p + 4 > buf.len() {
                break;
            }
            let header = u32le(&buf, p);
            p += 4;
            if header == 0x7FFF_7FFF {
                break;
            }
            let run = (header & 0x0FFF) as usize;
            let mut x = ((header >> 22) & 0x3FF) as i32;
            if x & 0x200 != 0 {
                x |= !0x3FF; // sign-extend 10-bit
            }
            let mut y = ((header >> 12) & 0x3FF) as i32;
            if y & 0x200 != 0 {
                y |= !0x3FF;
            }
            x += center_x as i32;
            y += center_y as i32 + height;

            for k in 0..run {
                if p >= buf.len() {
                    break;
                }
                let c = pal(buf[p]);
                p += 1;
                let px = x + k as i32;
                if px >= 0 && px < width && y >= 0 && y < height {
                    let o = (y as usize * w + px as usize) * 4;
                    let r = ((c >> 10) & 0x1F) as u8;
                    let g = ((c >> 5) & 0x1F) as u8;
                    let b = (c & 0x1F) as u8;
                    rgba[o] = (r << 3) | (r >> 2);
                    rgba[o + 1] = (g << 3) | (g >> 2);
                    rgba[o + 2] = (b << 3) | (b >> 2);
                    rgba[o + 3] = 255;
                }
            }
        }

        let img = Image {
            width: w as u32,
            height: h as u32,
            rgba,
        };
        // A mirrored image flips X, so the draw-center flips too: cx → width - cx.
        let cx = if mirror { width as i16 - center_x } else { center_x };
        Some((if mirror { flip_h(&img) } else { img }, cx, center_y))
    }
}

/// Parse `Body.def`: each data line is `index {a, b, c…} hue`. The real animation
/// body is `group[2]` when the group lists ≥3 ids, else `group[0]` (ClassicUO
/// `ProcessBodyDef`: "Yes, this is actually how this is supposed to work"). The
/// first entry for an index wins; `#` starts a comment. Malformed lines are skipped.
fn parse_body_def(text: &str) -> HashMap<u16, (u16, u16)> {
    let mut map = HashMap::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        // Split around the `{ … }` group list.
        let (Some(open), Some(close)) = (line.find('{'), line.find('}')) else { continue };
        if close < open {
            continue;
        }
        let Some(index) = parse_def_int(line[..open].trim()) else { continue };
        let group: Vec<i64> = line[open + 1..close]
            .split(',')
            .filter_map(|t| parse_def_int(t.trim()))
            .collect();
        if group.is_empty() {
            continue;
        }
        let Some(hue) = parse_def_int(line[close + 1..].trim()) else { continue };
        let check = if group.len() >= 3 { group[2] } else { group[0] };
        if !(0..=0xFFFF).contains(&check) || !(0..=0xFFFF).contains(&index) {
            continue;
        }
        // First entry for an index wins (ClassicUO keeps the already-present graphic).
        map.entry(index as u16).or_insert((check as u16, hue.clamp(0, 0xFFFF) as u16));
    }
    map
}

/// Parse `Bodyconv.def`: each line is `index c1 c2 c3 c4 …` where column `i`
/// (1-based) holds this body's graphic in `anim{i+1}.mul` (`-1` = not in that
/// file). ClassicUO writes `_bodyConvInfos[index]` once per non-negative column,
/// so the highest-numbered valid column wins. We store `(file index i, graphic)`;
/// the caller drops redirects whose file isn't installed. `#` starts a comment.
fn parse_body_conv(text: &str) -> HashMap<u16, (u8, u16)> {
    let mut map = HashMap::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut toks = line.split_whitespace();
        let Some(index) = toks.next().and_then(parse_def_int) else { continue };
        if !(0..=0xFFFF).contains(&index) {
            continue;
        }
        // Columns i = 1.. → anim{i+1}.mul. Later valid columns overwrite earlier
        // ones (ClassicUO order); only files 1..=4 (anim2..anim5) can ever exist.
        for (i, tok) in toks.enumerate() {
            let col = i + 1;
            if col > 4 {
                break;
            }
            let Some(graphic) = parse_def_int(tok) else { continue };
            if (0..=0xFFFF).contains(&graphic) {
                map.insert(index as u16, (col as u8, graphic as u16));
            }
        }
    }
    map
}

/// Graphic-range group kind (0 = monster/high, 1 = animal/low, 2 = people) for a
/// file — ClassicUO `CalculateTypeByGraphic`. `anim2` splits monster/animal at 200;
/// `anim3` is animal <300, monster 300..400, people ≥400; the base file and
/// `anim4`/`anim5` use monster <200, animal 200..400, people ≥400. Used only when
/// `mobtypes.txt` doesn't cover the body.
fn type_by_graphic(graphic: u16, file_index: usize) -> u8 {
    match file_index {
        1 => (graphic >= 200) as u8, // anim2: <200 monster, else animal
        2 => {
            if graphic < 300 {
                1
            } else if graphic < 400 {
                0
            } else {
                2
            }
        }
        _ => {
            if graphic < 200 {
                0
            } else if graphic < 400 {
                1
            } else {
                2
            }
        }
    }
}

/// Parse `mobtypes.txt`: each data line is `id TYPE flags`, where TYPE is one of
/// `monster`/`sea_monster`/`animal`/`human`/`equipment` and `flags` is hex. We map
/// TYPE → group kind (0 monster/high, 1 animal/low, 2 people); `sea_monster` uses
/// the high group like a monster, and `equipment` uses the people group like a
/// human. For a monster, the offset-override flags apply (ClassicUO `CalculateOffset`):
/// `CalculateOffsetByPeopleGroup` (0x400) → people, `CalculateOffsetByLowGroup`
/// (0x40) → animal. `#` starts a comment; lines not beginning with a digit are skipped.
fn parse_mob_types(text: &str) -> HashMap<u16, u8> {
    let mut map = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || !line.starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        let mut toks = line.split_whitespace();
        let Some(id) = toks.next().and_then(|t| t.parse::<u16>().ok()) else { continue };
        let Some(ty) = toks.next() else { continue };
        let mut kind = match ty.to_ascii_lowercase().as_str() {
            "monster" | "sea_monster" => 0u8,
            "animal" => 1,
            "human" | "equipment" => 2,
            _ => continue,
        };
        // Flags column: hex, possibly with a trailing `# comment`. Absent/`#` → 0.
        let flags = toks
            .next()
            .and_then(|t| t.split('#').next())
            .filter(|t| !t.is_empty())
            .and_then(|t| i64::from_str_radix(t, 16).ok())
            .unwrap_or(0);
        if kind == 0 {
            if flags & 0x400 != 0 {
                kind = 2; // CalculateOffsetByPeopleGroup
            } else if flags & 0x40 != 0 {
                kind = 1; // CalculateOffsetByLowGroup
            }
        }
        map.insert(id, kind);
    }
    map
}

/// Parse a `.def` integer token (decimal, or `0x`-prefixed hex). `None` if unparseable.
fn parse_def_int(t: &str) -> Option<i64> {
    let t = t.trim();
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16).ok()
    } else {
        t.parse().ok()
    }
}

/// 8-direction (UO) → (stored anim dir 0..4, mirror). From ClassicUO GetAnimDirection.
fn map_dir(dir8: u8) -> (u8, bool) {
    match dir8 & 7 {
        2 => (1, true),
        4 => (1, false),
        1 => (2, true),
        5 => (2, false),
        0 => (3, true),
        6 => (3, false),
        3 => (0, false),
        7 => (4, false),
        _ => (0, false),
    }
}

fn flip_h(img: &Image) -> Image {
    let (w, h) = (img.width as usize, img.height as usize);
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let s = (y * w + x) * 4;
            let d = (y * w + (w - 1 - x)) * 4;
            rgba[d..d + 4].copy_from_slice(&img.rgba[s..s + 4]);
        }
    }
    Image {
        width: img.width,
        height: img.height,
        rgba,
    }
}

fn u32le(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}
fn u16le(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([d[o], d[o + 1]])
}
fn i16le(d: &[u8], o: usize) -> i16 {
    i16::from_le_bytes([d[o], d[o + 1]])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource
    fn decodes_human_stand_frame() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let anim = Anim::open(&dir).expect("open anim");
        // Human male (0x190 = 400), standing, facing south (dir 4), frame 0.
        let (img, _cx, _cy) = anim.frame(400, PEOPLE_STAND, 4, 0).expect("human stand frame");
        println!("human stand frame: {}x{}", img.width, img.height);
        assert!(img.width > 0 && img.height > 0);
        assert!(!img.is_empty(), "frame should have opaque pixels");
    }

    #[test]
    fn body_def_remaps_and_picks_third_or_first() {
        let def = "\
# comment line
11 {28} 1401
46 {12, 59} 1106
100 {7, 8, 9} 0     # 3+ ids → use the third (9)
11 {99} 42         # duplicate index: first entry wins
bad line without braces
";
        let map = parse_body_def(def);
        // 2-id (and 1-id) groups use group[0]; hue carried through.
        assert_eq!(map.get(&11), Some(&(28, 1401)));
        assert_eq!(map.get(&46), Some(&(12, 1106)));
        // 3+ ids → group[2].
        assert_eq!(map.get(&100), Some(&(9, 0)));
        // Duplicate index 11 kept its first mapping (28), not the later 99.
        assert_eq!(map.get(&11).unwrap().0, 28);
    }

    #[test]
    fn body_conv_parses_columns_and_last_valid_wins() {
        let def = "\
# object type → file index table
157\t1\t-1\t-1\t-1\t-1
11\t3\t-1\t-1\t-1\t-1
28\t-1\t-1\t-1\t-1\t-1
300\t-1\t5\t-1\t-1\t-1
900\t-1\t-1\t2\t7\t-1
bad
";
        let map = parse_body_conv(def);
        // Column 1 → anim2 (file index 1).
        assert_eq!(map.get(&157), Some(&(1, 1)));
        assert_eq!(map.get(&11), Some(&(1, 3)));
        // All -1 → no redirect.
        assert_eq!(map.get(&28), None);
        // Column 2 → anim3 (file index 2).
        assert_eq!(map.get(&300), Some(&(2, 5)));
        // Columns 3 and 4 both valid → the later (higher) column wins: anim5 (4).
        assert_eq!(map.get(&900), Some(&(4, 7)));
    }

    #[test]
    fn type_by_graphic_matches_classicuo_per_file() {
        // Base file (0) / anim4 / anim5: monster<200, animal 200..400, people ≥400.
        assert_eq!(type_by_graphic(100, 0), 0);
        assert_eq!(type_by_graphic(250, 0), 1);
        assert_eq!(type_by_graphic(400, 0), 2);
        assert_eq!(type_by_graphic(250, 3), 1);
        // anim2: monster<200, else animal.
        assert_eq!(type_by_graphic(150, 1), 0);
        assert_eq!(type_by_graphic(250, 1), 1);
        // anim3: animal <300, monster 300..400, people ≥400.
        assert_eq!(type_by_graphic(250, 2), 1);
        assert_eq!(type_by_graphic(350, 2), 0);
        assert_eq!(type_by_graphic(400, 2), 2);
    }

    #[test]
    fn block_offset_by_kind() {
        // Section base by kind, plus group*5 + dir.
        assert_eq!(Anim::block(100, 0, 0, 0), Some(100 * 110)); // monster/high
        assert_eq!(Anim::block(250, 0, 0, 1), Some((250 - 200) * 65 + 22000)); // animal/low
        assert_eq!(Anim::block(400, 0, 0, 2), Some(35000)); // people
        assert_eq!(Anim::block(100, 4, 3, 0), Some(100 * 110 + 4 * 5 + 3));
    }

    #[test]
    fn mob_types_parses_and_applies_offset_flags() {
        let def = "\
# id  type  flags
5\tANIMAL\t2A
200\tMONSTER\t0
400\tHUMAN\t0
9\tMONSTER\t1008
50\tMONSTER\t440    # ByPeopleGroup (0x400) → people
60\tMONSTER\t48     # ByLowGroup (0x40) → animal
70\tSEA_MONSTER\t0
not a data line
";
        let map = parse_mob_types(def);
        assert_eq!(map.get(&5), Some(&1)); // animal
        assert_eq!(map.get(&200), Some(&0)); // monster (range would say animal — mobtypes wins)
        assert_eq!(map.get(&400), Some(&2)); // human → people
        assert_eq!(map.get(&9), Some(&0)); // monster, unrelated flags
        assert_eq!(map.get(&50), Some(&2)); // monster + 0x400 → people
        assert_eq!(map.get(&60), Some(&1)); // monster + 0x40 → animal
        assert_eq!(map.get(&70), Some(&0)); // sea_monster → high/monster
    }

    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource (real Body.def + anim.mul)
    fn body_def_remap_adds_real_coverage() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let anim = Anim::open(&dir).expect("open anim");
        assert!(!anim.bodydef.is_empty(), "Body.def should have loaded entries");
        // For every remap, the *target* body's stand frame should resolve, and count
        // how many exotic bodies gain a sprite they lacked at their original id.
        let mut gained = 0;
        for (&orig, &(target, _)) in &anim.bodydef {
            if orig == target {
                continue;
            }
            let orig_ok = anim.frame_count(orig, anim.stand_group(orig), 4).unwrap_or(0) > 0;
            let target_ok = anim.frame_count(target, anim.stand_group(target), 4).unwrap_or(0) > 0;
            if !orig_ok && target_ok {
                gained += 1;
            }
        }
        println!("Body.def: {} entries, {} bodies gained a sprite via remap", anim.bodydef.len(), gained);
        assert!(gained > 0, "remap should resolve sprites that the raw body id could not");
    }

    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource (real mobtypes.txt)
    fn mob_types_overrides_range_heuristic() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let anim = Anim::open(&dir).expect("open anim");
        assert!(!anim.mobtypes.is_empty(), "mobtypes.txt should have loaded entries");
        let mut overridden = 0;
        for (&body, &kind) in &anim.mobtypes {
            let range = if body < 200 {
                0
            } else if body < 400 {
                1
            } else {
                2
            };
            if kind != range {
                overridden += 1;
            }
        }
        println!("mobtypes: {} entries, {overridden} override the range heuristic", anim.mobtypes.len());
        assert!(overridden > 0, "mobtypes should correct some bodies the range heuristic gets wrong");
    }

    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource (real Bodyconv.def + anim2..anim5.mul)
    fn body_conv_resolves_expansion_sprites() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let anim = Anim::open(&dir).expect("open anim");
        assert!(!anim.bodyconv.is_empty(), "Bodyconv.def should have loaded entries");
        let installed = anim.files.iter().skip(1).filter(|f| f.is_some()).count();
        println!("bodyconv: {} entries, {} expansion files (anim2..anim5) installed", anim.bodyconv.len(), installed);
        // Count bodies whose stand frame resolves ONLY through a bodyconv redirect
        // into an expansion file (the raw id has no entry in the base anim.mul).
        let mut via_conv = 0;
        for (&body, &(fi, _)) in &anim.bodyconv {
            if fi == 0 || anim.files.get(fi as usize).and_then(Option::as_ref).is_none() {
                continue;
            }
            let (rfi, _, _) = match anim.entry(body, anim.stand_group(body), 4) {
                Some(e) => e,
                None => continue,
            };
            if rfi != 0 && anim.frame_count(body, anim.stand_group(body), 4).unwrap_or(0) > 0 {
                via_conv += 1;
            }
        }
        println!("bodyconv: {via_conv} bodies render from an expansion file");
        if installed > 0 {
            assert!(via_conv > 0, "expansion redirects should resolve real sprites");
        }
    }
}
