//! The world-map window: radar colours rasterised to a PNG.

use super::*;

/// Tiles per output pixel when rendering the full-world map. 1 = full resolution
/// (one pixel per tile), so the client maps world tile (x, y) → image pixel 1:1.
/// Must match the JS `WORLDMAP_STEP` in `web/main.js`.
pub const WORLDMAP_STEP: u32 = 1;

/// Render the whole facet to a full-resolution RGBA PNG using ClassicUO's exact
/// world-map algorithm (`WorldMapGump.LoadMap`): per tile take the radar LAND
/// color, then overlay each STATIC top-most-by-Z with its radar STATIC color, then
/// a Z-relief shading pass that embosses slopes. This makes buildings, roads,
/// water and walls visible (the old land-average path showed only blurry terrain).
///
/// Traversal is block-by-block (8×8 cells) via [`MapData::block_cells`] so each
/// map/statics block is decoded exactly once — the per-pixel `land()`/`statics()`
/// path would be far too slow across the ~29M cells. `step` is accepted for API
/// symmetry but full resolution (1) is used. The caller renders this once and
/// caches the PNG.
pub fn render_worldmap(map: &mut MapData, radar: &RadarCol, _step: u32) -> Vec<u8> {
    let w = MAP_WIDTH as usize;
    let h = MAP_HEIGHT as usize;
    let mut rgba = vec![0u8; w * h * 4];
    // Parallel per-pixel Z buffer (ClassicUO `allZ`): land Z, raised by the
    // top-most static, then read by the relief pass.
    let mut allz = vec![0i8; w * h];

    let blocks_x = MAP_WIDTH / 8;
    let blocks_y = MAP_HEIGHT / 8;
    for bx in 0..blocks_x {
        let base_x = (bx * 8) as usize;
        for by in 0..blocks_y {
            let (land, statics) = map.block_cells(bx, by);
            let base_y = (by * 8) as usize;
            for cy in 0..8usize {
                for cx in 0..8usize {
                    let cell = cy * 8 + cx;
                    let (g, z) = land[cell];
                    let idx = (base_y + cy) * w + (base_x + cx);
                    let o = idx * 4;
                    let c = radar.land_color(g);
                    rgba[o] = c[0];
                    rgba[o + 1] = c[1];
                    rgba[o + 2] = c[2];
                    rgba[o + 3] = 255;
                    allz[idx] = z;
                    // Statics in file order; the top-most by Z wins (>= so a later
                    // equal-Z static overrides), giving roads/water/buildings.
                    for s in &statics[cell] {
                        if s.graphic == 0 || s.graphic == 0xFFFF {
                            continue;
                        }
                        if s.z >= allz[idx] {
                            let sc = radar.static_color(s.graphic);
                            rgba[o] = sc[0];
                            rgba[o + 1] = sc[1];
                            rgba[o + 2] = sc[2];
                            rgba[o + 3] = 255;
                            allz[idx] = s.z;
                        }
                    }
                }
            }
        }
    }

    // Z-relief shading (ClassicUO): compare each pixel's Z to the pixel one row
    // SOUTH. Lower-than-south → darken ×0.8; higher-than-south → brighten ×1.25
    // (clamped). Equal → unchanged. This is the embossed terrain look.
    const MAG_DARK: f32 = 80.0 / 100.0;
    const MAG_LIGHT: f32 = 100.0 / 80.0;
    for y in 0..h - 1 {
        let row = y * w;
        for x in 0..w {
            let idx = row + x;
            let z0 = allz[idx];
            let z1 = allz[idx + w];
            if z0 == z1 {
                continue;
            }
            let o = idx * 4;
            // Leave pure-black/empty pixels untouched (ClassicUO skips PackedValue 0).
            if rgba[o] == 0 && rgba[o + 1] == 0 && rgba[o + 2] == 0 {
                continue;
            }
            let mag = if z0 < z1 { MAG_DARK } else { MAG_LIGHT };
            for k in 0..3 {
                rgba[o + k] = (rgba[o + k] as f32 * mag).min(255.0) as u8;
            }
        }
    }

    Image {
        width: MAP_WIDTH,
        height: MAP_HEIGHT,
        rgba,
    }
    .to_png()
}
