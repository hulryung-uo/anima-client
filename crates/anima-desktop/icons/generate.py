#!/usr/bin/env python3
"""Regenerates every icon asset in this directory from scratch.

Design: macOS-style rounded-square tile, deep indigo/obsidian background,
a gold ankh (Ultima Online's iconic symbol), one small cyan "AI accent"
node at the loop's apex.

Palette (documented so re-tuning stays easy):
    BG_TOP      #1a1d2e  background gradient, top
    BG_BOTTOM   #0d0e16  background gradient, bottom
    GOLD        #d4a843  ankh base fill (warm gold)
    GOLD_HI     #f0cf7a  ankh top-edge highlight (lighter gold)
    CYAN        #39c5cf  AI accent node (circuit-node dot)

Everything is drawn with plain PIL primitives (rounded rects, ellipses)
at 4x supersample and downsampled with LANCZOS for clean edges, so the
result is reproducible and easy to re-tune (see the constants below).

Usage:
    python3 generate.py            # (re)writes all icon files in this dir
    python3 generate.py --preview OUT.png [--size 128]
                                    # just render one PNG for quick review,
                                    # without touching the real icon files
"""
from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image, ImageDraw

ICONS_DIR = Path(__file__).resolve().parent

# ---------------------------------------------------------------------------
# Palette
# ---------------------------------------------------------------------------
BG_TOP = (0x1A, 0x1D, 0x2E, 255)
BG_BOTTOM = (0x0D, 0x0E, 0x16, 255)
GOLD = (0xD4, 0xA8, 0x43, 255)
GOLD_HI = (0xF0, 0xCF, 0x7A, 255)
CYAN = (0x39, 0xC5, 0xCF, 255)

# ---------------------------------------------------------------------------
# Geometry, expressed as fractions of the base canvas side (kept resolution
# independent — the master is rendered at SUPERSAMPLE x this and downscaled).
# ---------------------------------------------------------------------------
BASE = 1024
SUPERSAMPLE = 4

SQUIRCLE_MARGIN_FRAC = 0.05          # 5% margin each side -> squircle fills ~90%
SQUIRCLE_RADIUS_FRAC = 0.225         # corner radius, fraction of squircle side

STROKE_FRAC = 0.13                   # ankh stroke width (loop ring / arms / stem)
CX_FRAC = 0.5                        # ankh horizontal center

LOOP_OUTER_R_FRAC = 0.166            # loop outer radius
LOOP_CY_FRAC = 0.332                 # loop center, y

CROSSBAR_Y_FRAC = 0.547              # crossbar vertical center
CROSSBAR_HALF_W_FRAC = 0.225         # crossbar half-width (each arm)

STEM_BOTTOM_FRAC = 0.80              # bottom of the vertical stem

ACCENT_R_FRAC = 0.018                # cyan accent dot radius (~3.6% dia of canvas)

HIGHLIGHT_STROKE_SCALE = 0.34        # highlight ring thickness, relative to STROKE
HIGHLIGHT_INSET_PX_FRAC = 0.006      # highlight ring pulled in from the outer edge
CROSSBAR_HI_FRAC = 0.30              # highlight strip height, relative to STROKE


def _rounded_rect_mask(size: int, x0: float, y0: float, x1: float, y1: float, radius: float) -> Image.Image:
    mask = Image.new("L", (size, size), 0)
    d = ImageDraw.Draw(mask)
    d.rounded_rectangle([x0, y0, x1, y1], radius=radius, fill=255)
    return mask


def render_master(size: int = BASE) -> Image.Image:
    """Renders the icon at `size`x`size` (supersampled internally for crisp edges)."""
    s = size * SUPERSAMPLE
    canvas = Image.new("RGBA", (s, s), (0, 0, 0, 0))

    # --- background: gradient rounded-square (squircle-ish) ---------------
    margin = s * SQUIRCLE_MARGIN_FRAC
    radius = (s - 2 * margin) * SQUIRCLE_RADIUS_FRAC
    mask = _rounded_rect_mask(s, margin, margin, s - margin, s - margin, radius)

    gradient = Image.new("RGBA", (1, s), 0)
    gd = ImageDraw.Draw(gradient)
    for y in range(s):
        t = y / (s - 1)
        r = round(BG_TOP[0] + (BG_BOTTOM[0] - BG_TOP[0]) * t)
        g = round(BG_TOP[1] + (BG_BOTTOM[1] - BG_TOP[1]) * t)
        b = round(BG_TOP[2] + (BG_BOTTOM[2] - BG_TOP[2]) * t)
        gd.point((0, y), fill=(r, g, b, 255))
    gradient = gradient.resize((s, s))
    canvas.paste(gradient, (0, 0), mask)

    draw = ImageDraw.Draw(canvas)

    # --- ankh geometry ------------------------------------------------------
    cx = s * CX_FRAC
    stroke = s * STROKE_FRAC
    loop_r_outer = s * LOOP_OUTER_R_FRAC
    loop_r_inner = loop_r_outer - stroke
    loop_cy = s * LOOP_CY_FRAC
    crossbar_y = s * CROSSBAR_Y_FRAC
    crossbar_half_w = s * CROSSBAR_HALF_W_FRAC
    stem_bottom = s * STEM_BOTTOM_FRAC
    stem_top = loop_cy + loop_r_inner  # flows seamlessly into the ring's bottom arc

    # Stem (vertical bar), drawn first so the loop ring can overlap its top.
    draw.rounded_rectangle(
        [cx - stroke / 2, stem_top, cx + stroke / 2, stem_bottom],
        radius=stroke / 2,
        fill=GOLD,
    )

    # Crossbar (horizontal bar).
    draw.rounded_rectangle(
        [cx - crossbar_half_w, crossbar_y - stroke / 2, cx + crossbar_half_w, crossbar_y + stroke / 2],
        radius=stroke / 2,
        fill=GOLD,
    )

    # Loop: thick ring = outer ellipse minus inner ellipse (same fill, so the
    # "hole" must be cut using composite-over-background rather than a plain
    # ellipse(outline=...) call, to keep the ring perfectly circular/thick).
    ring = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    rd = ImageDraw.Draw(ring)
    rd.ellipse([cx - loop_r_outer, loop_cy - loop_r_outer, cx + loop_r_outer, loop_cy + loop_r_outer], fill=GOLD)
    rd.ellipse([cx - loop_r_inner, loop_cy - loop_r_inner, cx + loop_r_inner, loop_cy + loop_r_inner], fill=(0, 0, 0, 0))
    canvas.alpha_composite(ring)
    draw = ImageDraw.Draw(canvas)

    # --- top-edge highlight (simple flat-with-one-highlight shading) -------
    hi_stroke = stroke * HIGHLIGHT_STROKE_SCALE
    inset = s * HIGHLIGHT_INSET_PX_FRAC

    # Loop: a lighter partial ring, kept to the top half via a paste mask.
    hi_r_outer = loop_r_outer - inset
    hi_r_inner = hi_r_outer - hi_stroke
    hi_ring = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    hrd = ImageDraw.Draw(hi_ring)
    hrd.ellipse([cx - hi_r_outer, loop_cy - hi_r_outer, cx + hi_r_outer, loop_cy + hi_r_outer], fill=GOLD_HI)
    hrd.ellipse([cx - hi_r_inner, loop_cy - hi_r_inner, cx + hi_r_inner, loop_cy + hi_r_inner], fill=(0, 0, 0, 0))
    top_half_mask = Image.new("L", (s, s), 0)
    ImageDraw.Draw(top_half_mask).rectangle([0, 0, s, loop_cy], fill=255)
    hi_ring.putalpha(Image.composite(hi_ring.split()[3], Image.new("L", (s, s), 0), top_half_mask))
    canvas.alpha_composite(hi_ring)

    # Crossbar: a thin lighter strip along its top edge.
    cb_hi_h = stroke * CROSSBAR_HI_FRAC
    draw = ImageDraw.Draw(canvas)
    draw.rounded_rectangle(
        [cx - crossbar_half_w + inset, crossbar_y - stroke / 2 + inset,
         cx + crossbar_half_w - inset, crossbar_y - stroke / 2 + inset + cb_hi_h],
        radius=cb_hi_h / 2,
        fill=GOLD_HI,
    )

    # --- AI accent: small cyan node at the loop's apex ---------------------
    apex_y = loop_cy - loop_r_outer + stroke / 2
    accent_r = s * ACCENT_R_FRAC
    draw.ellipse([cx - accent_r, apex_y - accent_r, cx + accent_r, apex_y + accent_r], fill=CYAN)

    return canvas.resize((size, size), Image.LANCZOS)


# ---------------------------------------------------------------------------
# Output generation
# ---------------------------------------------------------------------------

def _resize(master: Image.Image, size: int) -> Image.Image:
    return master.resize((size, size), Image.LANCZOS)


def write_icons(master: Image.Image, out_dir: Path) -> None:
    _resize(master, 32).save(out_dir / "32x32.png")
    _resize(master, 128).save(out_dir / "128x128.png")
    _resize(master, 256).save(out_dir / "128x128@2x.png")

    # --- icon.icns via a proper .iconset + iconutil (macOS only) ----------
    iconutil = shutil.which("iconutil")
    if iconutil is None:
        print("warning: `iconutil` not found (non-macOS host?); skipping icon.icns", file=sys.stderr)
    else:
        with tempfile.TemporaryDirectory() as tmp:
            iconset = Path(tmp) / "icon.iconset"
            iconset.mkdir()
            # (filename, pixel size) pairs iconutil expects.
            variants = [
                ("icon_16x16.png", 16),
                ("icon_16x16@2x.png", 32),
                ("icon_32x32.png", 32),
                ("icon_32x32@2x.png", 64),
                ("icon_128x128.png", 128),
                ("icon_128x128@2x.png", 256),
                ("icon_256x256.png", 256),
                ("icon_256x256@2x.png", 512),
                ("icon_512x512.png", 512),
                ("icon_512x512@2x.png", 1024),
            ]
            for name, px in variants:
                _resize(master, px).save(iconset / name)
            subprocess.run(
                [iconutil, "-c", "icns", str(iconset), "-o", str(out_dir / "icon.icns")],
                check=True,
            )

    # --- icon.ico: multi-size (16/32/48/256), each explicitly LANCZOS-resized
    ico_sizes = [16, 32, 48, 256]
    ico_frames = [_resize(master, sz).convert("RGBA") for sz in ico_sizes]
    ico_frames[-1].save(
        out_dir / "icon.ico",
        format="ICO",
        sizes=[(f.width, f.height) for f in ico_frames],
        append_images=ico_frames[:-1],
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preview", type=Path, help="write a single preview PNG here instead of the real icons")
    parser.add_argument("--size", type=int, default=BASE, help="preview size in px (default %(default)s)")
    args = parser.parse_args()

    if args.preview:
        render_master(args.size).save(args.preview)
        print(f"wrote preview {args.preview} ({args.size}x{args.size})")
        return

    master = render_master(BASE)
    write_icons(master, ICONS_DIR)
    print(f"wrote icons to {ICONS_DIR}")


if __name__ == "__main__":
    main()
