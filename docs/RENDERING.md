# Rendering Visibility & Multi-Floor — Design

How the client decides **what to draw based on the player's position**: hiding the
roof/ceiling when you step indoors, and showing the correct level of a multi-story
building. This is a faithful port of ClassicUO's algorithm; section refs are to
`~/dev/uo/classicuo`.

## 1. The problem

UO is a top-down isometric world where statics (walls, floors, roofs, upper-storey
floors) stack in Z over the same (x,y). If we drew everything, a roof would hide the
room under it and an upper floor would hide the one you're standing on. So the client
computes a **per-frame Z ceiling** from the player's position and culls anything above
it, plus hides roofs entirely while indoors.

Two values drive it (ClassicUO `GameSceneDrawingSorting.cs:UpdateMaxDrawZ`, recomputed
whenever the player's tile changes):

- **`maxZ`** — the Z ceiling. Statics/items/mobiles at/above it are not drawn.
- **`noDrawRoofs`** — when true, *every* roof-flagged static in view is hidden
  (not just those above `maxZ`), so the whole roof lifts off.

## 2. UpdateMaxDrawZ (the ceiling)  — `GameSceneDrawingSorting.cs:52`

Inputs: player `(px,py,pz)`. Thresholds `pz14 = pz+14`, `pz16 = pz+16`. Start
`maxZ = 127`, `maxGroundZ = 127`, `noDrawRoofs = !DrawRoofs` (default false).

**Scan A — the player's own tile `(px,py)`** (`:88`). Iterate every object on the cell:
- **Land**: if stretched use its `AverageZ`. If `pz16 <= landZ` → there's ground over
  our head (terrain overhang/cave): `maxZ = pz16`, **stop**. Otherwise skip.
- **Mobile**: skip.
- **Static**: if `tileZ > pz14 && tileZ < maxZ` and
  `(flags & 0x20004)==0 && (!IsRoof || IsSurface)` → `maxZ = tileZ; noDrawRoofs = true`.
  - `0x20004` = Transparent(0x4) | Foliage(0x20000) — those never block the view.
  - `(!IsRoof || IsSurface)` catches an **upper-floor surface** or a solid blocker
    over our head (this is what reveals the floor you stand on when a storey is above).

**Scan B — the facing tile `(px+1, py+1)`** (`:148`), roofs only:
- Non-Land, `tileZ > pz14 && tileZ < maxZ`, `(flags & 0x204)==0 && IsRoof`
  (`0x204` = Transparent | Surface) → `maxZ = tileZ`, then
  `maxGroundZ = CalculateNearZ(tileZ, px+1, py+1, tileZ)`, `noDrawRoofs = true`.

**Finalize** (`:198`): `maxZ = maxGroundZ`; then `if (tempZ < pz16) { maxZ = pz16 }`
where `tempZ` is the CalculateNearZ result — i.e. **the ceiling never drops below
`pz+16`** (you always see ~16 units above your head). `maxGroundZ` is then reset to 127
(it's only used for selection/interaction, not drawing).

### CalculateNearZ — `Map.cs:164` (flood fill)

Given a roof tile, find the **lowest connected roof Z** so the ceiling sits at the
roof's *eave/floor* level rather than a random tile of a pitched roof:

```
CalculateNearZ(defaultZ, x, y, z):
  if visited[(x&63) + ((y&63)<<6)]: return defaultZ     # 64×64 visited set
  mark visited
  obj = first Static/Multi at (x,y) that IsRoof and |z - obj.z| <= 6
  if none: return defaultZ
  defaultZ = min(defaultZ, obj.z)
  for each 4-neighbour: defaultZ = CalculateNearZ(defaultZ, nx, ny, obj.z)
  return defaultZ
```

So a whole connected roof within ±6 Z collapses to its minimum Z → `maxZ`.

## 3. The draw cull (per object)  — `AddTileToRenderList` / `ProcessAlpha:326`

For an object on screen, it is **hidden** when:

**The fade IS the cull — this table used to say the opposite.** `AddTileToRenderList`'s
`maxZ` parameter is the literal **`150`** passed by its only call site
(`GameScene.cs:629-635`), which also discards the `bool` it returns — so every
`if (maxObjectZ > maxZ)` branch inside it is dead twice over. The live ceiling rule is
`ProcessAlpha` easing `AlphaHue` to 0, and an object only stops being drawn once it
*reaches* 0 (`PushToRenderQueue` returns early `if (obj.AlphaHue == 0)`, `:566-571`).
ClassicUO never removes the object from the tile's chain — which is exactly why its own
`Pathfinder.CreateItemList` is untouched by the ceiling (`Pathfinder.cs` contains no
`_maxZ`/`AlphaHue`/`AllowedToDraw` at all).

| type | hidden if |
|------|-----------|
| **Land** | Not by `_maxZ`. But `_maxGroundZ` (set only by Scan A's Land branch, `:99`) gates **every** render list via `DrawRenderList`'s `if (obj.Z <= maxGroundZ)` (`RenderLists.cs:127-133`) — so land *is* hidden at `pz+16` in the terrain-overhang case, which is what `terrain.rs:227` ports. |
| **Static / Item** | `ProcessAlpha`: `obj.Z >= _maxZ` → ease to 0, **else if** `_noDrawRoofs && IsRoof` → ease to 0 (`:337`, `:355`; the chain is `else if`, so the Z rule wins). The old "tall wall pokes through" exception (`height != 0 && (z - maxZ) < height`) is part of the dead-against-`150` code and is **not** a rule to port. |
| **Mobile** | Same `ProcessAlpha`, called with an **empty** `StaticTiles` (`:840-850`) — so only the `obj.Z >= _maxZ` branch can fire for a mobile, never the roof or translucent ones. (`z + DEFAULT_CHARACTER_HEIGHT(16) > maxZ` is the dead branch.) |

`CalculateAlpha` steps ±25 per 20 ms tick, symmetric both ways — 255→0 in 11 steps,
a ~220 ms floor. Two user toggles we do not model: `UseObjectsFading` (default true;
off makes it snap) and `DrawRoofs`/"Hide roof tiles" (default true).

## 4. Why this yields correct multi-floor

- **Outdoors**: nothing over your head matches Scan A/B → `maxZ = 127`, draw all.
- **Walk under a roof**: Scan B finds the roof → `maxZ` = roof eave Z (via CalculateNearZ),
  `noDrawRoofs = true`. Every roof tile is hidden; statics `>= maxZ` are hidden; the room
  (walls/floor/furniture below the eave) shows. You see inside.
- **Stand on the ground floor of a 2-storey building**: Scan A finds the *2nd-floor
  surface* over your head (`!IsRoof || IsSurface` branch) → `maxZ` = that surface Z →
  the entire 2nd floor and roof are hidden, the ground floor shows.
- **Stand on the 2nd floor**: now the 2nd-floor surface is *below/at* your feet (not
  `> pz14`), so it doesn't lower `maxZ`; only the roof above does → roof hidden, the 2nd
  floor (its own floor tiles + walls + furniture, all `< maxZ`) shows. The 1st floor
  below is lower Z so it isn't culled by `maxZ`; **iso z-ordering** draws the 2nd-floor
  surface over it where they overlap, so you correctly see the level you're on.

The key insight: `maxZ` hides everything **above** the player's current level; floor
separation **below** is handled by normal depth (z) ordering, not by culling.

## 5. How we implement it (this project)

The map/tiledata live server-side, so we compute the cull in `anima-net::scene` and only
send the visible statics to the browser; the renderer just draws what it receives and
depth-sorts by `(x+y)` + Z.

- **`scene::max_draw_z(map, px, py, pz)`** → `maxZ`. Ports Scans A & B + CalculateNearZ +
  the `pz16` clamp. (`MapData` already exposes `land(x,y)` and `statics(x,y)` with
  tiledata flags incl. `Roof 0x10000000`, `Surface 0x200`.)
- **scene ceiling flag** (was a cull): a static with `z >= maxZ` **or**
  `(under_cover && IsRoof)` is still EMITTED, carrying `"hz":1` and **no** `h`/`pf`
  animation frames and no light — but WITH its normal `h`/`pf` pathing suffix, since
  an invisible object still blocks and still feeds the browser's `calculate_new_z`
  (withholding it shifted the browser's predicted Z by up to 20 units on
  server-walkable tiles; ClassicUO's own `Pathfinder` likewise never consults the
  draw ceiling). The renderer
  eases it to alpha 0 and sets `visible = false`. Dropping it was what made a roof pop:
  there was no sprite left to fade.  `hz` is purely a draw decision
  where `under_cover = maxZ < 127`. (Roof tiles flagged `0x10000000`.) The persistent
  tile pool in the renderer then drops the hidden statics automatically.
- **Renderer depth order** (ClassicUO `Chunk.AddGameObject` priority): a single sorted
  container keyed by `zIndex = (x+y)*8192 + (priorityZ+130)*16 + typeBias`, where
  `priorityZ = z` adjusted (**land z−2**, **background −1**, **height≠0 / wall +1**,
  mobile +1) and `typeBias` (land 0 < static 4 < mobile 8). Primary `(x+y)` = iso
  depth; secondary `priorityZ` so on the **same tile a floor (height 0) draws under the
  wall (height≠0)** and walls/upper-floor surfaces occlude correctly. The server sends
  each static's `pz` (it has the tiledata height/flags); the renderer just applies it.
- **Land ceiling**: also stop sending land tiles with `z > maxZ` (rare — terrain
  overhangs), matching the Land rule.

### Deferred / cosmetic (not needed for correct floors)
- Alpha fade-in/out instead of instant hide (ProcessAlpha smoothing).
- ~~Translucent (alpha 178)~~ — **done**; only the foliage circle-of-transparency
  half of this bullet remains (and our occlusion pass is its own rule, not
  ClassicUO's CoT, which ships disabled by default).
- The static "tall wall pokes through the ceiling" height exception (`(z-maxZ)<height`).
  We currently hard-cull at `z >= maxZ`; revisit if wall tops clip oddly at a ceiling.

## 6. Verification
- Stand under a roof → whole roof gone, room visible. Verified at Britain house
  (1585,1560,20): with CalculateNearZ the ceiling is the roof's **eave near-Z**
  (`maxZ=40`, clamped ≥ pz16=36 — vs `58` before the flood-fill, which only lifted
  the upper roof band), and all connected roof statics are hidden via `_noDrawRoofs`.
  Open ground (Britain inn 1602,1591) → `maxZ=127` (draw everything).
- Stand on a 2-storey ground floor → upper storey hidden.
- Stand on the upper floor → roof gone, that floor shown, ground floor beneath via z-order.
- `scene.json` `map.maxZ` and `map.dbg` (statics over the player + roof/surface flags)
  are exposed for diagnosing any building that doesn't behave.
