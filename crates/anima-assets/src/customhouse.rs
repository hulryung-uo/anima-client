//! Custom-house building catalog: `walls.txt`, `floors.txt`, `doors.txt`,
//! `misc.txt`, `stairs.txt`, `teleprts.txt`, `roof.txt`, `suppinfo.txt`.
//!
//! These are plain tab-separated text files that ship with every UO client
//! install (not `.mul`/`.uop`) and enumerate every piece placeable in the
//! in-game custom-house designer, plus (`suppinfo.txt`) a per-tile
//! support/adjacency table used to validate where a piece may legally go. The
//! real client reads exactly these files — see ClassicUO
//! `Game/Managers/HouseCustomizationManager.cs` (`ParseFile` /
//! `ParseFileWithCategory`) and the row shapes in `Game/Data/CustomHouse.cs`
//! (`CustomHouseWall`, `CustomHouseFloor`, `CustomHouseRoof`,
//! `CustomHouseStair`, `CustomHouseMisc`, `CustomHouseDoor`,
//! `CustomHouseTeleport`, `CustomHousePlaceInfo`), which this module ports.
//!
//! ## Format
//! Row 1 declares each column's type (`int`/`string`), row 2 names the
//! columns, and data starts at row 3. A trailing `string` column is a
//! human-readable comment, not used by the client for anything but display.
//!
//! We don't rely on line position to skip the two header rows: like the real
//! client's loader, we just try to parse every line as data and drop whatever
//! doesn't fit — a header token (`int`, `Category`, …) never parses as an
//! integer, so both header rows (and any blank/separator line some files
//! insert between them, e.g. `doors.txt`) fall out for free. `suppinfo.txt`'s
//! rows additionally carry a leading column with no header name at all (real
//! rows start with an ASCII-art glyph like `\`, `v`, `./`, …); we simply never
//! read that column, exactly like `CustomHousePlaceInfo.Parse` (its first used
//! index is `scanf[1]`, not `scanf[0]`).
//!
//! ## Grouping
//! Walls, misc pieces, and roofs are grouped into **categories**: every row
//! sharing a `Category` id is one category (a wall material, say), and each
//! row within it is a *style* (standard/half/quarter wall, …) — this mirrors
//! the flip-page-of-styles-within-a-category gump the real client shows.
//! Floors, doors, stairs, and teleporters are flat lists instead (ClassicUO's
//! `HouseCustomizationManager` keeps them as plain `List<T>`, not
//! `List<TCategory>`).

use std::io;
use std::path::Path;

/// A group of styles sharing one `Category` id (ClassicUO
/// `CustomHouseObjectCategory<T>`). Only walls, misc pieces, and roofs are
/// grouped this way — see the module docs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CustomHouseCategory<T> {
    pub category: i32,
    pub items: Vec<T>,
}

/// One wall style row from `walls.txt` (ClassicUO `CustomHouseWall`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomHouseWall {
    pub category: i32,
    pub style: i32,
    pub tid: i32,
    pub south1: i32,
    pub south2: i32,
    pub south3: i32,
    pub corner: i32,
    pub east1: i32,
    pub east2: i32,
    pub east3: i32,
    pub post: i32,
    pub window_s: i32,
    pub alt_window_s: i32,
    pub window_e: i32,
    pub alt_window_e: i32,
    pub second_alt_window_s: i32,
    pub second_alt_window_e: i32,
    pub feature_mask: i32,
    pub comment: String,
    /// `[South1, South2, South3, Corner, East1, East2, East3, Post]` as tile
    /// graphics (ClassicUO `CustomHouseWall.Graphics`, `GRAPHICS_COUNT = 8`).
    pub graphics: Vec<u16>,
}

/// One floor style row from `floors.txt` (ClassicUO `CustomHouseFloor`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomHouseFloor {
    pub category: i32,
    pub f1: i32,
    pub f2: i32,
    pub f3: i32,
    pub f4: i32,
    pub f5: i32,
    pub f6: i32,
    pub f7: i32,
    pub f8: i32,
    pub f9: i32,
    pub f10: i32,
    pub f11: i32,
    pub f12: i32,
    pub f13: i32,
    pub f14: i32,
    pub f15: i32,
    pub f16: i32,
    pub feature_mask: i32,
    pub comment: String,
    /// `[F1..F16]` as tile graphics (`GRAPHICS_COUNT = 16`).
    pub graphics: Vec<u16>,
}

/// One door style row from `doors.txt` (ClassicUO `CustomHouseDoor`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomHouseDoor {
    pub category: i32,
    pub piece1: i32,
    pub piece2: i32,
    pub piece3: i32,
    pub piece4: i32,
    pub piece5: i32,
    pub piece6: i32,
    pub piece7: i32,
    pub piece8: i32,
    pub feature_mask: i32,
    pub comment: String,
    /// `[Piece1..Piece8]` as tile graphics (`GRAPHICS_COUNT = 8`).
    pub graphics: Vec<u16>,
}

/// One misc-fixture style row from `misc.txt` (ClassicUO `CustomHouseMisc`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomHouseMisc {
    pub category: i32,
    pub style: i32,
    pub tid: i32,
    pub piece1: i32,
    pub piece2: i32,
    pub piece3: i32,
    pub piece4: i32,
    pub piece5: i32,
    pub piece6: i32,
    pub piece7: i32,
    pub piece8: i32,
    pub feature_mask: i32,
    pub comment: String,
    /// `[Piece1..Piece8]` as tile graphics (`GRAPHICS_COUNT = 8`).
    pub graphics: Vec<u16>,
}

/// One stair style row from `stairs.txt` (ClassicUO `CustomHouseStair`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomHouseStair {
    pub category: i32,
    pub block: i32,
    pub north: i32,
    pub east: i32,
    pub south: i32,
    pub west: i32,
    pub squared1: i32,
    pub squared2: i32,
    pub rounded1: i32,
    pub rounded2: i32,
    pub multi_north: i32,
    pub multi_east: i32,
    pub multi_south: i32,
    pub multi_west: i32,
    pub feature_mask: i32,
    pub comment: String,
    /// `[MultiNorth!=0?Squared1:0, MultiEast!=0?Squared2:0, MultiSouth!=0?Rounded1:0,
    /// MultiWest!=0?Rounded2:0, Block, North, East, South, West]` — mirrors
    /// ClassicUO `CustomHouseStair.Parse`'s exact `Graphics` assembly
    /// (`GRAPHICS_COUNT = 9`), not just a straight column dump.
    pub graphics: Vec<u16>,
}

/// One teleporter-tile style row from `teleprts.txt` (ClassicUO
/// `CustomHouseTeleport`). Same column shape as [`CustomHouseFloor`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomHouseTeleport {
    pub category: i32,
    pub f1: i32,
    pub f2: i32,
    pub f3: i32,
    pub f4: i32,
    pub f5: i32,
    pub f6: i32,
    pub f7: i32,
    pub f8: i32,
    pub f9: i32,
    pub f10: i32,
    pub f11: i32,
    pub f12: i32,
    pub f13: i32,
    pub f14: i32,
    pub f15: i32,
    pub f16: i32,
    pub feature_mask: i32,
    pub comment: String,
    /// `[F1..F16]` as tile graphics (`GRAPHICS_COUNT = 16`).
    pub graphics: Vec<u16>,
}

/// One roof style row from `roof.txt` (ClassicUO `CustomHouseRoof`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomHouseRoof {
    pub category: i32,
    pub style: i32,
    pub tid: i32,
    pub north: i32,
    pub east: i32,
    pub south: i32,
    pub west: i32,
    pub ns_crosspiece: i32,
    pub ew_crosspiece: i32,
    pub n_dent: i32,
    pub e_dent: i32,
    pub s_dent: i32,
    pub w_dent: i32,
    pub n_t_piece: i32,
    pub e_t_piece: i32,
    pub s_t_piece: i32,
    pub w_t_piece: i32,
    pub x_piece: i32,
    pub extra_piece: i32,
    pub feature_mask: i32,
    pub comment: String,
    /// `[North, East, South, West, NSCrosspiece, EWCrosspiece, NDent, EDent,
    /// SDent, WDent, NTPiece, ETPiece, STPiece, WTPiece, XPiece, ExtraPiece]`
    /// as tile graphics (`GRAPHICS_COUNT = 16`).
    pub graphics: Vec<u16>,
}

/// One per-tile support/adjacency rule from `suppinfo.txt` (ClassicUO
/// `CustomHousePlaceInfo`), used to validate whether a piece may go at the
/// edge of the house plot (e.g. "can't place at the west/north border unless
/// this tile explicitly allows it"). Real rows start with an unnamed,
/// ASCII-art glyph column (`\`, `v`, `./`, `o`, …) that we — like the real
/// client — never read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupportInfo {
    pub tile_number: i32,
    pub top: i32,
    pub bottom: i32,
    pub adj_un: i32,
    pub adj_ln: i32,
    pub adj_ue: i32,
    pub adj_le: i32,
    pub adj_us: i32,
    pub adj_ls: i32,
    pub adj_uw: i32,
    pub adj_lw: i32,
    pub direct_supports: i32,
    pub cango_w: i32,
    pub cango_n: i32,
    pub cango_nwc: i32,
    pub comment: String,
}

/// The full custom-house catalog: every placeable piece plus the
/// per-tile support table, as read from a UO data directory.
#[derive(Debug, Clone, Default)]
pub struct CustomHouseCatalog {
    pub walls: Vec<CustomHouseCategory<CustomHouseWall>>,
    pub floors: Vec<CustomHouseFloor>,
    pub doors: Vec<CustomHouseDoor>,
    pub misc: Vec<CustomHouseCategory<CustomHouseMisc>>,
    pub stairs: Vec<CustomHouseStair>,
    pub teleporters: Vec<CustomHouseTeleport>,
    pub roofs: Vec<CustomHouseCategory<CustomHouseRoof>>,
    pub support_info: Vec<SupportInfo>,
}

impl CustomHouseCatalog {
    /// Read whichever of the eight catalog files exist in `data_dir`. A
    /// missing file is a soft failure (that list/table is just empty) since
    /// not every install ships every one of these; only a genuinely
    /// unreadable file (exists but can't be read — permissions, I/O error)
    /// is a hard error.
    pub fn open(data_dir: impl AsRef<Path>) -> io::Result<Self> {
        let dir = data_dir.as_ref();
        let walls = group_by_category(
            read_optional(dir, "walls.txt")?
                .map(|t| parse_walls(&t))
                .unwrap_or_default(),
            |w: &CustomHouseWall| w.category,
        );
        let floors = read_optional(dir, "floors.txt")?
            .map(|t| parse_floors(&t))
            .unwrap_or_default();
        let doors = read_optional(dir, "doors.txt")?
            .map(|t| parse_doors(&t))
            .unwrap_or_default();
        let misc = group_by_category(
            read_optional(dir, "misc.txt")?
                .map(|t| parse_misc(&t))
                .unwrap_or_default(),
            |m: &CustomHouseMisc| m.category,
        );
        let stairs = read_optional(dir, "stairs.txt")?
            .map(|t| parse_stairs(&t))
            .unwrap_or_default();
        let teleporters = read_optional(dir, "teleprts.txt")?
            .map(|t| parse_teleporters(&t))
            .unwrap_or_default();
        let roofs = group_by_category(
            read_optional(dir, "roof.txt")?
                .map(|t| parse_roofs(&t))
                .unwrap_or_default(),
            |r: &CustomHouseRoof| r.category,
        );
        let support_info = read_optional(dir, "suppinfo.txt")?
            .map(|t| parse_support_info(&t))
            .unwrap_or_default();
        Ok(CustomHouseCatalog {
            walls,
            floors,
            doors,
            misc,
            stairs,
            teleporters,
            roofs,
            support_info,
        })
    }
}

/// Read `data_dir/name` as UTF-8 text. `Ok(None)` when the file simply isn't
/// there (soft failure — the caller treats this as an empty list); any other
/// I/O error (permissions, not valid UTF-8 as `read_to_string`'s error, …) is
/// propagated since that means the file exists but is genuinely broken.
fn read_optional(dir: &Path, name: &str) -> io::Result<Option<String>> {
    match std::fs::read_to_string(dir.join(name)) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Group parsed rows into [`CustomHouseCategory`] buckets, in first-appearance
/// order — mirrors ClassicUO `ParseFileWithCategory`'s linear "find existing
/// category or start a new one" loop (row counts here are small enough that
/// this is plenty fast, and it keeps category order identical to the file's).
fn group_by_category<T>(
    rows: Vec<T>,
    category_of: impl Fn(&T) -> i32,
) -> Vec<CustomHouseCategory<T>> {
    let mut groups: Vec<CustomHouseCategory<T>> = Vec::new();
    for row in rows {
        let category = category_of(&row);
        match groups.iter_mut().find(|g| g.category == category) {
            Some(g) => g.items.push(row),
            None => groups.push(CustomHouseCategory {
                category,
                items: vec![row],
            }),
        }
    }
    groups
}

/// Tab-separated column at `i`, parsed as an integer. `None` on a missing
/// column or anything that doesn't parse (a header token like `int` or
/// `Category`, or an empty field from a blank/separator line) — the caller
/// turns that into "this line isn't a data row" and drops it.
fn col_i32(cols: &[&str], i: usize) -> Option<i32> {
    cols.get(i)?.trim().parse().ok()
}

/// Tab-separated column at `i` as a trimmed string; `""` if the column is
/// absent (the trailing `Comment` column is sometimes missing entirely).
fn col_comment(cols: &[&str], i: usize) -> String {
    cols.get(i)
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn parse_walls(text: &str) -> Vec<CustomHouseWall> {
    text.lines().filter_map(parse_wall_row).collect()
}

fn parse_wall_row(line: &str) -> Option<CustomHouseWall> {
    let c: Vec<&str> = line.split('\t').collect();
    let category = col_i32(&c, 0)?;
    let style = col_i32(&c, 1)?;
    let tid = col_i32(&c, 2)?;
    let south1 = col_i32(&c, 3)?;
    let south2 = col_i32(&c, 4)?;
    let south3 = col_i32(&c, 5)?;
    let corner = col_i32(&c, 6)?;
    let east1 = col_i32(&c, 7)?;
    let east2 = col_i32(&c, 8)?;
    let east3 = col_i32(&c, 9)?;
    let post = col_i32(&c, 10)?;
    let window_s = col_i32(&c, 11)?;
    let alt_window_s = col_i32(&c, 12)?;
    let window_e = col_i32(&c, 13)?;
    let alt_window_e = col_i32(&c, 14)?;
    let second_alt_window_s = col_i32(&c, 15)?;
    let second_alt_window_e = col_i32(&c, 16)?;
    let feature_mask = col_i32(&c, 17)?;
    let comment = col_comment(&c, 18);
    let graphics = [south1, south2, south3, corner, east1, east2, east3, post]
        .iter()
        .map(|&v| v as u16)
        .collect();
    Some(CustomHouseWall {
        category,
        style,
        tid,
        south1,
        south2,
        south3,
        corner,
        east1,
        east2,
        east3,
        post,
        window_s,
        alt_window_s,
        window_e,
        alt_window_e,
        second_alt_window_s,
        second_alt_window_e,
        feature_mask,
        comment,
        graphics,
    })
}

fn parse_floors(text: &str) -> Vec<CustomHouseFloor> {
    text.lines().filter_map(parse_floor_row).collect()
}

fn parse_floor_row(line: &str) -> Option<CustomHouseFloor> {
    let c: Vec<&str> = line.split('\t').collect();
    let category = col_i32(&c, 0)?;
    let f1 = col_i32(&c, 1)?;
    let f2 = col_i32(&c, 2)?;
    let f3 = col_i32(&c, 3)?;
    let f4 = col_i32(&c, 4)?;
    let f5 = col_i32(&c, 5)?;
    let f6 = col_i32(&c, 6)?;
    let f7 = col_i32(&c, 7)?;
    let f8 = col_i32(&c, 8)?;
    let f9 = col_i32(&c, 9)?;
    let f10 = col_i32(&c, 10)?;
    let f11 = col_i32(&c, 11)?;
    let f12 = col_i32(&c, 12)?;
    let f13 = col_i32(&c, 13)?;
    let f14 = col_i32(&c, 14)?;
    let f15 = col_i32(&c, 15)?;
    let f16 = col_i32(&c, 16)?;
    let feature_mask = col_i32(&c, 17)?;
    let comment = col_comment(&c, 18);
    let graphics = [
        f1, f2, f3, f4, f5, f6, f7, f8, f9, f10, f11, f12, f13, f14, f15, f16,
    ]
    .iter()
    .map(|&v| v as u16)
    .collect();
    Some(CustomHouseFloor {
        category,
        f1,
        f2,
        f3,
        f4,
        f5,
        f6,
        f7,
        f8,
        f9,
        f10,
        f11,
        f12,
        f13,
        f14,
        f15,
        f16,
        feature_mask,
        comment,
        graphics,
    })
}

fn parse_doors(text: &str) -> Vec<CustomHouseDoor> {
    text.lines().filter_map(parse_door_row).collect()
}

fn parse_door_row(line: &str) -> Option<CustomHouseDoor> {
    let c: Vec<&str> = line.split('\t').collect();
    let category = col_i32(&c, 0)?;
    let piece1 = col_i32(&c, 1)?;
    let piece2 = col_i32(&c, 2)?;
    let piece3 = col_i32(&c, 3)?;
    let piece4 = col_i32(&c, 4)?;
    let piece5 = col_i32(&c, 5)?;
    let piece6 = col_i32(&c, 6)?;
    let piece7 = col_i32(&c, 7)?;
    let piece8 = col_i32(&c, 8)?;
    let feature_mask = col_i32(&c, 9)?;
    let comment = col_comment(&c, 10);
    let graphics = [
        piece1, piece2, piece3, piece4, piece5, piece6, piece7, piece8,
    ]
    .iter()
    .map(|&v| v as u16)
    .collect();
    Some(CustomHouseDoor {
        category,
        piece1,
        piece2,
        piece3,
        piece4,
        piece5,
        piece6,
        piece7,
        piece8,
        feature_mask,
        comment,
        graphics,
    })
}

fn parse_misc(text: &str) -> Vec<CustomHouseMisc> {
    text.lines().filter_map(parse_misc_row).collect()
}

fn parse_misc_row(line: &str) -> Option<CustomHouseMisc> {
    let c: Vec<&str> = line.split('\t').collect();
    let category = col_i32(&c, 0)?;
    let style = col_i32(&c, 1)?;
    let tid = col_i32(&c, 2)?;
    let piece1 = col_i32(&c, 3)?;
    let piece2 = col_i32(&c, 4)?;
    let piece3 = col_i32(&c, 5)?;
    let piece4 = col_i32(&c, 6)?;
    let piece5 = col_i32(&c, 7)?;
    let piece6 = col_i32(&c, 8)?;
    let piece7 = col_i32(&c, 9)?;
    let piece8 = col_i32(&c, 10)?;
    let feature_mask = col_i32(&c, 11)?;
    let comment = col_comment(&c, 12);
    let graphics = [
        piece1, piece2, piece3, piece4, piece5, piece6, piece7, piece8,
    ]
    .iter()
    .map(|&v| v as u16)
    .collect();
    Some(CustomHouseMisc {
        category,
        style,
        tid,
        piece1,
        piece2,
        piece3,
        piece4,
        piece5,
        piece6,
        piece7,
        piece8,
        feature_mask,
        comment,
        graphics,
    })
}

fn parse_stairs(text: &str) -> Vec<CustomHouseStair> {
    text.lines().filter_map(parse_stair_row).collect()
}

fn parse_stair_row(line: &str) -> Option<CustomHouseStair> {
    let c: Vec<&str> = line.split('\t').collect();
    let category = col_i32(&c, 0)?;
    let block = col_i32(&c, 1)?;
    let north = col_i32(&c, 2)?;
    let east = col_i32(&c, 3)?;
    let south = col_i32(&c, 4)?;
    let west = col_i32(&c, 5)?;
    let squared1 = col_i32(&c, 6)?;
    let squared2 = col_i32(&c, 7)?;
    let rounded1 = col_i32(&c, 8)?;
    let rounded2 = col_i32(&c, 9)?;
    let multi_north = col_i32(&c, 10)?;
    let multi_east = col_i32(&c, 11)?;
    let multi_south = col_i32(&c, 12)?;
    let multi_west = col_i32(&c, 13)?;
    let feature_mask = col_i32(&c, 14)?;
    let comment = col_comment(&c, 15);
    // Mirrors ClassicUO `CustomHouseStair.Parse`'s `Graphics` assembly exactly
    // (not a plain column dump): the combined-staircase pieces only count when
    // their `Multi*` flag is set.
    let graphics = [
        if multi_north != 0 { squared1 } else { 0 },
        if multi_east != 0 { squared2 } else { 0 },
        if multi_south != 0 { rounded1 } else { 0 },
        if multi_west != 0 { rounded2 } else { 0 },
        block,
        north,
        east,
        south,
        west,
    ]
    .iter()
    .map(|&v| v as u16)
    .collect();
    Some(CustomHouseStair {
        category,
        block,
        north,
        east,
        south,
        west,
        squared1,
        squared2,
        rounded1,
        rounded2,
        multi_north,
        multi_east,
        multi_south,
        multi_west,
        feature_mask,
        comment,
        graphics,
    })
}

fn parse_teleporters(text: &str) -> Vec<CustomHouseTeleport> {
    text.lines().filter_map(parse_teleporter_row).collect()
}

fn parse_teleporter_row(line: &str) -> Option<CustomHouseTeleport> {
    let c: Vec<&str> = line.split('\t').collect();
    let category = col_i32(&c, 0)?;
    let f1 = col_i32(&c, 1)?;
    let f2 = col_i32(&c, 2)?;
    let f3 = col_i32(&c, 3)?;
    let f4 = col_i32(&c, 4)?;
    let f5 = col_i32(&c, 5)?;
    let f6 = col_i32(&c, 6)?;
    let f7 = col_i32(&c, 7)?;
    let f8 = col_i32(&c, 8)?;
    let f9 = col_i32(&c, 9)?;
    let f10 = col_i32(&c, 10)?;
    let f11 = col_i32(&c, 11)?;
    let f12 = col_i32(&c, 12)?;
    let f13 = col_i32(&c, 13)?;
    let f14 = col_i32(&c, 14)?;
    let f15 = col_i32(&c, 15)?;
    let f16 = col_i32(&c, 16)?;
    let feature_mask = col_i32(&c, 17)?;
    let comment = col_comment(&c, 18);
    let graphics = [
        f1, f2, f3, f4, f5, f6, f7, f8, f9, f10, f11, f12, f13, f14, f15, f16,
    ]
    .iter()
    .map(|&v| v as u16)
    .collect();
    Some(CustomHouseTeleport {
        category,
        f1,
        f2,
        f3,
        f4,
        f5,
        f6,
        f7,
        f8,
        f9,
        f10,
        f11,
        f12,
        f13,
        f14,
        f15,
        f16,
        feature_mask,
        comment,
        graphics,
    })
}

fn parse_roofs(text: &str) -> Vec<CustomHouseRoof> {
    text.lines().filter_map(parse_roof_row).collect()
}

fn parse_roof_row(line: &str) -> Option<CustomHouseRoof> {
    let c: Vec<&str> = line.split('\t').collect();
    let category = col_i32(&c, 0)?;
    let style = col_i32(&c, 1)?;
    let tid = col_i32(&c, 2)?;
    let north = col_i32(&c, 3)?;
    let east = col_i32(&c, 4)?;
    let south = col_i32(&c, 5)?;
    let west = col_i32(&c, 6)?;
    let ns_crosspiece = col_i32(&c, 7)?;
    let ew_crosspiece = col_i32(&c, 8)?;
    let n_dent = col_i32(&c, 9)?;
    let e_dent = col_i32(&c, 10)?;
    let s_dent = col_i32(&c, 11)?;
    let w_dent = col_i32(&c, 12)?;
    let n_t_piece = col_i32(&c, 13)?;
    let e_t_piece = col_i32(&c, 14)?;
    let s_t_piece = col_i32(&c, 15)?;
    let w_t_piece = col_i32(&c, 16)?;
    let x_piece = col_i32(&c, 17)?;
    let extra_piece = col_i32(&c, 18)?;
    let feature_mask = col_i32(&c, 19)?;
    let comment = col_comment(&c, 20);
    let graphics = [
        north,
        east,
        south,
        west,
        ns_crosspiece,
        ew_crosspiece,
        n_dent,
        e_dent,
        s_dent,
        w_dent,
        n_t_piece,
        e_t_piece,
        s_t_piece,
        w_t_piece,
        x_piece,
        extra_piece,
    ]
    .iter()
    .map(|&v| v as u16)
    .collect();
    Some(CustomHouseRoof {
        category,
        style,
        tid,
        north,
        east,
        south,
        west,
        ns_crosspiece,
        ew_crosspiece,
        n_dent,
        e_dent,
        s_dent,
        w_dent,
        n_t_piece,
        e_t_piece,
        s_t_piece,
        w_t_piece,
        x_piece,
        extra_piece,
        feature_mask,
        comment,
        graphics,
    })
}

fn parse_support_info(text: &str) -> Vec<SupportInfo> {
    text.lines().filter_map(parse_support_row).collect()
}

fn parse_support_row(line: &str) -> Option<SupportInfo> {
    let c: Vec<&str> = line.split('\t').collect();
    // c[0] is the unnamed leading glyph column — real data rows carry an
    // ASCII-art marker there (`\`, `v`, `./`, …); header rows have nothing
    // (an empty field from the leading tab). Either way we never read it,
    // matching ClassicUO `CustomHousePlaceInfo.Parse` (first used index is
    // `scanf[1]`).
    let tile_number = col_i32(&c, 1)?;
    let top = col_i32(&c, 2)?;
    let bottom = col_i32(&c, 3)?;
    let adj_un = col_i32(&c, 4)?;
    let adj_ln = col_i32(&c, 5)?;
    let adj_ue = col_i32(&c, 6)?;
    let adj_le = col_i32(&c, 7)?;
    let adj_us = col_i32(&c, 8)?;
    let adj_ls = col_i32(&c, 9)?;
    let adj_uw = col_i32(&c, 10)?;
    let adj_lw = col_i32(&c, 11)?;
    let direct_supports = col_i32(&c, 12)?;
    let cango_w = col_i32(&c, 13)?;
    let cango_n = col_i32(&c, 14)?;
    let cango_nwc = col_i32(&c, 15)?;
    let comment = col_comment(&c, 16);
    Some(SupportInfo {
        tile_number,
        top,
        bottom,
        adj_un,
        adj_ln,
        adj_ue,
        adj_le,
        adj_us,
        adj_ls,
        adj_uw,
        adj_lw,
        direct_supports,
        cango_w,
        cango_n,
        cango_nwc,
        comment,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `walls.txt` fixture: both header rows plus two category-0 styles and
    /// one category-1 style, matching the real file's shape.
    const WALLS_FIXTURE: &str = "int\tint\tint\tint\tint\tint\tint\tint\tint\tint\tint\tint\tint\tint\tint\tint\tint\tint\tstring\r\n\
Category\tStyle\tTID\tSouth1\tSouth2\tSouth3\tCorner\tEast1\tEast2\tEast3\tPost\tWindowS\tAltWindowS\tWindowE\tAltWindowE\tSecondAltWindowS\tSecondAltWindowE\tFeatureMask\tComment\r\n\
0\t0\t1060054\t10\t7\t12\t6\t13\t8\t11\t9\t0\t14\t15\t0\t0\t0\t0\tDark Wood Standard Walls\r\n\
0\t1\t0\t18\t18\t18\t16\t17\t17\t17\t19\t0\t0\t0\t0\t0\t0\t0\tDark Wood Half Walls\r\n\
1\t0\t1060055\t171\t168\t173\t166\t172\t167\t170\t169\t186\t9472\t9479\t9478\t9473\t185\t0\tLight Wood Standard Walls\r\n";

    #[test]
    fn parses_walls_and_groups_by_category() {
        let groups = group_by_category(parse_walls(WALLS_FIXTURE), |w| w.category);
        assert_eq!(groups.len(), 2, "two distinct Category ids");
        assert_eq!(groups[0].category, 0);
        assert_eq!(groups[0].items.len(), 2, "two styles in category 0");
        assert_eq!(groups[0].items[0].style, 0);
        assert_eq!(groups[0].items[0].south1, 10);
        assert_eq!(groups[0].items[0].comment, "Dark Wood Standard Walls");
        assert_eq!(
            groups[0].items[0].graphics,
            vec![10, 7, 12, 6, 13, 8, 11, 9]
        );
        assert_eq!(groups[1].category, 1);
        assert_eq!(groups[1].items.len(), 1, "one style in category 1");
    }

    /// `floors.txt` fixture: two rows, no grouping (flat list).
    const FLOORS_FIXTURE: &str = "int\tint\tint\tint\tint\tint\tint\tint\tint\tint\tint\tint\tint\tint\tint\tint\tint\tint\tstring\n\
Category\tF1\tF2\tF3\tF4\tF5\tF6\tF7\tF8\tF9\tF10\tF11\tF12\tF13\tF14\tF15\tF16\tFeatureMask\tComment\n\
0\t1305\t1306\t1307\t1308\t1309\t1310\t1311\t1312\t1313\t1314\t1315\t1316\t1181\t1182\t1183\t1184\t0\tGrey Pavers and Flagstones\n\
1\t1317\t1318\t1319\t1320\t1321\t1322\t1323\t1324\t1327\t1328\t1329\t1330\t1331\t1332\t1333\t1334\t0\tSandstone Bricks\n";

    #[test]
    fn parses_floors_as_flat_list() {
        let floors = parse_floors(FLOORS_FIXTURE);
        assert_eq!(floors.len(), 2);
        assert_eq!(floors[0].category, 0);
        assert_eq!(floors[0].f1, 1305);
        assert_eq!(floors[0].graphics[0], 1305);
        assert_eq!(floors[0].comment, "Grey Pavers and Flagstones");
        assert_eq!(floors[1].category, 1);
        assert_eq!(floors[1].f1, 1317);
    }

    /// `doors.txt` fixture: real files interleave blank, all-tab "separator"
    /// lines between the two header rows (and sometimes after) — must not
    /// break parsing or get mistaken for data.
    const DOORS_FIXTURE: &str = "int\tint\tint\tint\tint\tint\tint\tint\tint\tint\tstring\r\n\
\t\t\t\t\t\t\t\t\t\t\r\n\
Category\tPiece1\tPiece2\tPiece3\tPiece4\tPiece5\tPiece6\tPiece7\tPiece8\tFeatureMask\tComment\r\n\
\t\t\t\t\t\t\t\t\t\t\r\n\
0\t1657\t1659\t1653\t1655\t1661\t1663\t1665\t1667\t0\tMetal Door\r\n\
1\t8177\t8179\t8173\t8175\t8181\t8183\t8185\t8187\t0\tMetal Gate\r\n";

    #[test]
    fn skips_blank_separator_lines_in_doors() {
        let doors = parse_doors(DOORS_FIXTURE);
        assert_eq!(doors.len(), 2, "only the two real data rows");
        assert_eq!(doors[0].category, 0);
        assert_eq!(doors[0].piece1, 1657);
        assert_eq!(doors[0].comment, "Metal Door");
        assert_eq!(doors[1].comment, "Metal Gate");
    }

    /// `suppinfo.txt` fixture: header rows carry a leading *empty* column
    /// (just a tab), but real data rows carry a leading ASCII-art glyph
    /// (`\`, `v`, …) in that same column — never the empty string. Both must
    /// be safely ignored rather than assumed to be a fixed width.
    const SUPPINFO_FIXTURE: &str = "\tint\tint\tint\tint\tint\tint\tint\tint\tint\tint\tint\tint\tint\tint\tint\tstring\r\n\
\ttileNumber\ttop\tbottom\tadj_UN\tadj_LN\tadj_UE\tadj_LE\tadj_US\tadj_LS\tadj_UW\tadj_LW\tdirectSupports\tcango W\tcango N\tcango NWC\tComment\r\n\
\\\t10\t1\t1\t0\t0\t1\t1\t1\t1\t1\t1\t1\t0\t1\t0\tDark Wood Std\r\n\
v\t6\t1\t1\t1\t1\t1\t1\t1\t1\t1\t1\t1\t0\t0\t0\t\r\n";

    #[test]
    fn parses_suppinfo_ignoring_leading_glyph_column() {
        let rows = parse_support_info(SUPPINFO_FIXTURE);
        assert_eq!(rows.len(), 2, "header rows dropped, both data rows kept");
        assert_eq!(rows[0].tile_number, 10);
        assert_eq!(rows[0].direct_supports, 1);
        assert_eq!(rows[0].cango_n, 1);
        assert_eq!(rows[0].comment, "Dark Wood Std");
        assert_eq!(rows[1].tile_number, 6, "the 'v'-prefixed row still parses");
        assert_eq!(
            rows[1].comment, "",
            "a missing trailing Comment is just empty"
        );
    }

    #[test]
    fn open_is_soft_on_missing_files() {
        let dir = std::env::temp_dir().join(format!(
            "anima-customhouse-test-empty-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
        ));
        std::fs::create_dir_all(&dir).expect("create empty test dir");
        let catalog = CustomHouseCatalog::open(&dir).expect("missing files are not an error");
        assert!(catalog.walls.is_empty());
        assert!(catalog.floors.is_empty());
        assert!(catalog.doors.is_empty());
        assert!(catalog.misc.is_empty());
        assert!(catalog.stairs.is_empty());
        assert!(catalog.teleporters.is_empty());
        assert!(catalog.roofs.is_empty());
        assert!(catalog.support_info.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_reads_whichever_files_are_present() {
        let dir = std::env::temp_dir().join(format!(
            "anima-customhouse-test-partial-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
        ));
        std::fs::create_dir_all(&dir).expect("create test dir");
        std::fs::write(dir.join("floors.txt"), FLOORS_FIXTURE).expect("write floors.txt");
        let catalog = CustomHouseCatalog::open(&dir).expect("open with one file present");
        assert_eq!(catalog.floors.len(), 2);
        assert!(catalog.walls.is_empty(), "walls.txt wasn't written");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Requires local UO data at ~/dev/uo/uo-resource. Ignored by default so
    /// the suite runs without game files; run with `--ignored` to validate.
    #[test]
    #[ignore]
    fn reads_real_customhouse_catalog() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let catalog = CustomHouseCatalog::open(&dir).expect("open custom-house catalog");

        assert!(!catalog.walls.is_empty(), "walls should be non-empty");
        assert!(!catalog.floors.is_empty(), "floors should be non-empty");
        assert!(!catalog.roofs.is_empty(), "roofs should be non-empty");
        assert!(!catalog.stairs.is_empty(), "stairs should be non-empty");

        // floors.txt category 0's first row (F1) starts with graphic 1305.
        let floor0 = catalog
            .floors
            .iter()
            .find(|f| f.category == 0)
            .expect("floors category 0");
        assert_eq!(floor0.f1, 1305);
        assert_eq!(floor0.graphics[0], 1305);

        // walls.txt category 0, style 0 has South1 = 10.
        let wall_cat0 = catalog
            .walls
            .iter()
            .find(|c| c.category == 0)
            .expect("walls category 0");
        let wall_style0 = wall_cat0
            .items
            .iter()
            .find(|w| w.style == 0)
            .expect("walls category 0 style 0");
        assert_eq!(wall_style0.south1, 10);

        println!(
            "walls: {} categories / {} rows",
            catalog.walls.len(),
            catalog.walls.iter().map(|c| c.items.len()).sum::<usize>()
        );
        println!("floors: {} rows", catalog.floors.len());
        println!("doors: {} rows", catalog.doors.len());
        println!(
            "misc: {} categories / {} rows",
            catalog.misc.len(),
            catalog.misc.iter().map(|c| c.items.len()).sum::<usize>()
        );
        println!("stairs: {} rows", catalog.stairs.len());
        println!("teleporters: {} rows", catalog.teleporters.len());
        println!(
            "roofs: {} categories / {} rows",
            catalog.roofs.len(),
            catalog.roofs.iter().map(|c| c.items.len()).sum::<usize>()
        );
        println!("support_info: {} rows", catalog.support_info.len());
    }
}
