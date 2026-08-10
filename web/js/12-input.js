// ---- input ----
const KEY_DIR = { ArrowUp: 0, KeyW: 0, ArrowRight: 2, KeyD: 2, ArrowDown: 4, KeyS: 4, ArrowLeft: 6, KeyA: 6, KeyE: 1, KeyC: 3, KeyZ: 5, KeyQ: 7 };
const held = new Set();
let chatting = false;
let shiftHeld = false;
let wasMoving = false;   // last frame sent a walk → send one "stop" on release
// Send "stop" the INSTANT movement ends (key/button up), not on the next 120ms
// tick — otherwise the server keeps pacing for up to a tick and takes one extra
// step (the "한 발자국 더" overshoot), worst at run cadence (200ms).
function stopNow() { if (wasMoving) { sendInput("stop"); wasMoving = false; } }
// Right-button "mouse move" (ClassicUO MoveCharacterByMouseInput): hold RMB and
// the character walks toward the cursor; far from center → run.
let rightDown = false, mouseX = 0, mouseY = 0;
// Right-clicking an entity opens its context menu instead of steering. The PIXI
// pointerdown fires before the canvas DOM mousedown, so it sets this timestamp;
// the button-2 mousedown handler sees it and skips starting RMB steering.
let suppressSteerUntil = 0;
// A right-button press that landed on an entity, pending a quick-tap (→ context
// menu) vs hold/drag (→ steer) decision: { serial, t, x, y, steering, timer } or null.
let rmbEntity = null;
const STEER_HOLD_MS = 180;    // hold RMB this long on an entity → start steering (not a menu)
// Promote a pending entity-RMB into steering (hold/drag detected). After this the
// release won't open a context menu — it was a move, not a tap.
function promoteRmbSteer() {
  if (!rmbEntity || rmbEntity.steering) return;
  rmbEntity.steering = true;
  if (rmbEntity.timer) { clearTimeout(rmbEntity.timer); rmbEntity.timer = null; }
  rightDown = true; // mouseX/mouseY are already tracked → steer toward the cursor
}
// Last cursor position (page coords) where a context menu should open.
let lastMenuX = 0, lastMenuY = 0;
const MOUSE_RUN_RANGE = 190;  // ClassicUO: run when cursor ≥190px from center
const MOUSE_DEADZONE = 18;    // too close to center → don't move (avoid jitter)
// The unified movement intent for this frame: mouse (RMB) wins, else held keys.
let moveIntent = null;        // { dir, run } or null

// ClassicUO GameCursor.GetMouseDirection: classify cursor offset into one of 8
// *screen* directions (0=N/up,1=NE,2=E,3=SE,4=S,5=SW,6=W,7=NW) by sign + ratio.
function screenDir(dx, dy) {
  const ax = Math.abs(dx), ay = Math.abs(dy);
  let cls; // 0 = horizontal cardinal, 1 = diagonal, 2 = vertical cardinal
  if (dx === 0) cls = 2;
  else if (dy === 0) cls = 0;
  else if (ay * 5 <= ax * 2) cls = 0;       // |dy/dx| ≤ 0.4
  else if (ay * 2 >= ax * 5) cls = 2;       // |dy/dx| ≥ 2.5
  else cls = 1;
  if (cls === 0) return dx < 0 ? 6 : 2;     // W : E
  if (cls === 2) return dy < 0 ? 0 : 4;     // N : S
  if (dx > 0) return dy < 0 ? 1 : 3;        // NE : SE
  return dy < 0 ? 7 : 5;                     // NW : SW
}

// Direction + run from the cursor relative to the screen center (the avatar).
function mouseMove() {
  // The avatar is drawn at the canvas CENTRE. mouseX/mouseY are CSS-client px
  // (canvas-relative), so the centre must be the canvas's *client* size — NOT
  // app.screen, which is the capped ~1.1MP renderer buffer (see renderSize) and
  // is smaller than the CSS-stretched canvas. Using app.screen put the steer
  // centre up-and-left of the real avatar, skewing every direction. Client space
  // is also zoom-independent (the avatar stays centred at any camZoom).
  const dx = mouseX - app.canvas.clientWidth / 2, dy = mouseY - app.canvas.clientHeight / 2;
  const range = Math.hypot(dx, dy);
  if (range < MOUSE_DEADZONE) return null;
  // screen → world dir: our iso is rotated one step (ClassicUO `facing - 1`).
  const dir = (screenDir(dx, dy) + 7) % 8;
  return { dir, run: range >= MOUSE_RUN_RANGE };
}

// What the player wants to do this frame (single source for prediction + send).
function activeMove() {
  if (chatting || wmOn) return null;   // don't walk while typing or with the world map open
  if (rightDown) { const m = mouseMove(); if (m) { standUp(); return m; } }
  if (held.size) { standUp(); return { dir: [...held].pop(), run: shiftHeld }; }
  return null;
}

// ---- world-entity interaction (click / use / attack) + target cursor ----
const DBLCLICK_MS = 250;        // single-vs-double-click discrimination window
let clickPend = null;           // pending single-click awaiting a possible double-click
let targetConsumedAt = 0;       // perf-time an entity answered a target click (so the
                                // ground handler below skips the same physical click)
let entityClickedAt = 0;        // perf-time a left-click hit a mobile/item (so the
                                // ground click-to-walk handler skips that same click)
let prevTargetActive = false;   // edge-detect target.active 0→1 (re-show after Esc)
let targetUIHidden = false;     // user pressed Esc → hide crosshair/banner locally

// A left-click landed on a clickable mobile or ground item. Resolves a pending
// target cursor first; otherwise shift = attack, double = use, single = request name.
function onEntityPointerDown(serial, e, isItem) {
  if (e.button === 2) {               // right-button on an entity
    // RMB on an entity is ambiguous: a *quick tap* = open its context menu, a
    // *hold or drag* = steer the character. To stop the two from firing together we
    // DON'T steer immediately here (the canvas mousedown checks `rmbEntity` and skips
    // starting the steer). Steering is promoted only once the press is held past
    // STEER_HOLD_MS or the cursor drags > a few px (see the mousemove/timer below);
    // a release before that opens the menu and never moved the character.
    rmbEntity = { serial: serial >>> 0, t: performance.now(), x: e.clientX, y: e.clientY, steering: false };
    lastMenuX = e.clientX; lastMenuY = e.clientY; // anchor a possible menu at the cursor
    rmbEntity.timer = setTimeout(() => { promoteRmbSteer(); }, STEER_HOLD_MS);
    return;
  }
  if (e.button !== 0) return;   // left only — right button still steers movement
  entityClickedAt = performance.now(); // a mobile/item ate this click → no click-to-walk
  e.stopPropagation();          // don't let it bubble to other interaction
  // An armed ignore pick spends itself here, ahead of the server's target
  // cursor: the arm is local and was made two clicks ago, so it is the more
  // specific intent. ClassicUO cannot reach this state at all — its ignore pick
  // IS the target cursor, so arming one cancels the other.
  if (ignorePick) { ignoreMobile(serial); armIgnorePick(false); return; }
  if (inspectPick) { inspectPicked({ serial }); return; }
  if (scene && scene.target && scene.target.active === 1 && !targetUIHidden) {
    targetConsumedAt = performance.now();
    sendInput("target:" + serial);   // answer the object-target cursor
    endTargetUI();
    return;
  }
  if (clickPend && clickPend.serial === serial) {  // second click in time → double-click
    clearTimeout(clickPend.timer); clickPend = null;
    // Any double-click ends a sit (see trySit()) before deciding what this one does —
    // double-clicking the same chair again just re-resolves and re-sits below.
    standUp();
    // War mode: double-clicking another mobile attacks it (ClassicUO behaviour),
    // instead of "use" (which would open its paperdoll).
    if (!isItem && scene && scene.war && (serial >>> 0) !== ((scene.player && scene.player.serial) >>> 0)) {
      sendInput("attack:" + serial);
      return;
    }
    sendInput("use:" + serial);
    // Only world items flagged as CONTAINERS (corpses/chests/bags — scene `c:1`)
    // open a loot window; doors/levers/other double-clickables must not spawn an
    // empty window. Mobiles never do this.
    if (isItem) {
      const it = (scene.items || []).find((x) => (x.serial >>> 0) === (serial >>> 0));
      if (it && it.c) openContainer(serial);
      else if (it) trySit(it); // no-op unless `it.g` is a chair/bench/stool/throne we're next to
    } else {
      // Double-clicked a MOBILE → open its paperdoll (humanoid bodies only, like UO).
      const m = (scene.mobiles || []).find((x) => (x.serial >>> 0) === (serial >>> 0));
      if (m && (m.body | 0) >= 400 && (m.body | 0) <= 407) openMobilePaperdoll(serial);
    }
  } else {
    if (clickPend) clearTimeout(clickPend.timer);
    clickPend = { serial, timer: setTimeout(() => {
      sendInput("click:" + serial);   // ask the server (OPL / name for other mobiles)
      if (!isItem) showNameOverhead(serial); // float the name now, in its notoriety colour
      clickPend = null;
    }, DBLCLICK_MS) };
    // Arm a ground-item pointer-drag: a left-press on a world item may turn into a
    // drag once the cursor moves past DRAG_THRESHOLD (see setupItemDnD). Until then
    // this stays a normal click; starting a drag cancels the pending name-request.
    if (isItem) {
      const it = (scene && scene.items || []).find((x) => (x.serial >>> 0) === (serial >>> 0));
      groundDrag = { serial: serial >>> 0, g: it ? it.g : 0, amount: (it && (it.amount | 0)) || 1,
                     st: !!(it && it.st), hue: (it && it.hue) | 0,
                     sx: e.clientX, sy: e.clientY, started: false, t: performance.now() };
    }
  }
}

// Invert the iso projection at the player's z to get the world tile under a click
// (renderer-space global coords minus the camera offset). Matches the forward
// projection isoX/isoY with HALF/ZSTEP; z is assumed to be the player's z.
function groundTileAt(gx, gy) {
  const z = scene && scene.player ? (scene.player.z | 0) : 0;
  // app.stage is scaled by camZoom (mouse-wheel zoom), so undo that scale to get
  // back to unscaled world-iso pixels before inverting the projection.
  const sx = (gx - app.stage.position.x) / camZoom, sy = (gy - app.stage.position.y) / camZoom;
  const a = sx / HALF, b = (sy + z * ZSTEP) / HALF;
  return { x: Math.round((a + b) / 2), y: Math.round((b - a) / 2), z };
}
// Brief destination marker for click-to-walk: a fading diamond on the target tile.
// Added to `world` so it pans with the map; self-destroys after a short fade.
let walkMarker = null;
function showWalkMarker(x, y, z) {
  if (walkMarker) { world.removeChild(walkMarker); walkMarker.destroy(); walkMarker = null; }
  const g = new PIXI.Graphics();
  g.moveTo(0, -HALF / 2).lineTo(HALF, 0).lineTo(0, HALF / 2).lineTo(-HALF, 0).closePath();
  g.fill({ color: 0x66ddff, alpha: 0.35 });
  g.stroke({ color: 0xaaf0ff, width: 2, alpha: 0.9 });
  g.x = isoX(x, y); g.y = isoY(x, y, z); g.zIndex = depthZ(x, y, z, 9);
  world.addChild(g);
  walkMarker = g;
  const t0 = performance.now();
  const tick = () => {
    if (walkMarker !== g) return;            // replaced by a newer click
    const a = 1 - (performance.now() - t0) / 700;
    if (a <= 0) { world.removeChild(g); g.destroy(); walkMarker = null; markDirty(); return; }
    g.alpha = a; markDirty();
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
}
// Multi placement preview: draws the house/boat footprint under the cursor
// while the server has a pending placement target (`scene.placement` — absent
// otherwise, so this is a no-op against an older server that never sends it).
// Placing a large building blind, with only a plain target cursor, is the
// problem this solves. ClassicUO's GameScene derives the multi's origin from
// the hovered world tile (hx,hy) as originX = hx - xOff, originY = hy - yOff,
// then draws each footprint offset at (originX+dx, originY+dy) — mirrored
// below. Each footprint tile is its own diamond, a direct child of `world`
// (not one shared container) so its own zIndex sorts against terrain/statics
// individually — exactly like showWalkMarker's single diamond, just tiled.
// On top of the outline we also draw the actual house art (walls/roof/doors)
// from `scene.placement.parts` — the flat footprint alone tells you WHERE a
// house will sit but not what it looks like or how tall it is, which matters
// a lot when you're choosing a spot for a specific building. `parts`/`tiles`
// share one lifecycle (one list, one clear, one rebuild guard) since they're
// really the same preview, just two layers of it.
let placementHoverX = null, placementHoverY = null; // world tile under the cursor
let placementTiles = [];  // footprint diamonds + house-art sprites currently in `world`
let placementKey = null;  // last (placement, hovered tile) drawn — dedups rebuilds
let placementPartsPending = false; // true if the last rebuild skipped a part still awaiting its texture
function clearPlacementPreview() {
  for (const g of placementTiles) { world.removeChild(g); g.destroy(); }
  placementTiles = [];
  placementKey = null;
  placementPartsPending = false;
}
// Rebuilds the footprint only when the placement or the hovered TILE actually
// changed (mousemove fires far more often than the tile does, and a house
// footprint can be a few hundred tiles — see the mousemove handler in
// setupInput that maintains placementHoverX/Y). Called both from poll() (to
// pick up a fresh/cleared scene.placement) and from that mousemove handler.
function updatePlacementPreview() {
  const p = scene && scene.placement;
  if (!p || placementHoverX == null) {
    if (placementTiles.length) { clearPlacementPreview(); markDirty(); }
    return;
  }
  const key = p.multiId + "/" + p.xOff + "/" + p.yOff + "/" + p.zOff + "/" +
              (p.tiles ? p.tiles.length : 0) + "/" + (p.parts ? p.parts.length : 0) +
              "@" + placementHoverX + "," + placementHoverY;
  // Same (placement, hovered tile) normally means nothing changed — EXCEPT
  // when the previous pass skipped a house-art part because texFor() hadn't
  // loaded its texture yet. That load resolves in the background and calls
  // markDirty() when it does, but markDirty() alone just repaints the current
  // (incomplete) sprite list; without also busting the key here, the next
  // poll()'s call would see an unchanged key and bail before ever drawing the
  // now-ready part, silently losing it instead of catching up on a later pass.
  if (key === placementKey && !placementPartsPending) return;
  placementKey = key;
  for (const g of placementTiles) { world.removeChild(g); g.destroy(); }
  placementTiles = [];
  const originX = placementHoverX - (p.xOff | 0), originY = placementHoverY - (p.yOff | 0);
  const fallbackZ = (tileSZ(placementHoverX, placementHoverY) ?? (scene.player ? scene.player.z | 0 : 0)) + (p.zOff | 0);
  // Ground footprint diamonds are only a FALLBACK now: once the translucent
  // house itself is drawn (below) the grid under the cursor is just noise, so
  // it appears solely when there is no art to draw — an older server that sends
  // no `parts`, or a multi we couldn't resolve components for.
  const tiles = (p.parts && p.parts.length)
    ? []
    : (p.tiles || []).slice(0, 4096); // match the server's footprint cap
  for (const [dx, dy] of tiles) {
    const tx = originX + dx, ty = originY + dy;
    const z = tileSZ(tx, ty);
    const gz = z != null ? z : fallbackZ;
    const g = new PIXI.Graphics();
    g.moveTo(0, -HALF / 2).lineTo(HALF, 0).lineTo(0, HALF / 2).lineTo(-HALF, 0).closePath();
    g.fill({ color: 0x66ddff, alpha: 0.25 });
    g.stroke({ color: 0xaaf0ff, width: 2, alpha: 0.9 });
    g.x = isoX(tx, ty); g.y = isoY(tx, ty, gz); g.zIndex = depthZ(tx, ty, gz, 9);
    world.addChild(g);
    placementTiles.push(g);
  }
  // House art itself, drawn ABOVE the outline: one sprite per multi COMPONENT
  // (not deduped — a multi-story wall stack needs every floor drawn), offset
  // from the same origin as the tiles above. Every part shares one ground
  // Z (`baseZ`) rather than each looking up its own tileSZ, so the building
  // stays a rigid shape instead of shearing across sloped/uneven terrain —
  // ClassicUO's own multi-preview (GameScene) computes the same way:
  // z = groundZ - ZOff, then each component sits at baseZ + its own dz.
  const groundZ = tileSZ(placementHoverX, placementHoverY) ?? (scene.player ? scene.player.z | 0 : 0);
  const baseZ = groundZ - (p.zOff | 0);
  // Hued art is requested by appending ?hue=<hue> to the art URL (see e.g. the
  // container/paperdoll icons elsewhere in this file) — same convention here.
  const hueQ = (p.hue | 0) ? ("?hue=" + (p.hue | 0)) : "";
  const parts = (p.parts || []).slice(0, 2000); // cap, matching the server's own part cap
  placementPartsPending = false;
  for (const [dx, dy, dz, gph] of parts) {
    const tex = texFor(`art/static/${gph}.png${hueQ}`);
    // Not loaded yet: texFor() already kicked off the load and will markDirty()
    // when it resolves — skip this part for now rather than block, and flag
    // the rebuild-guard override above so a later poll() picks it up instead
    // of it being lost until the placement/hovered tile happens to change.
    if (!tex) { placementPartsPending = true; continue; }
    const tx = originX + dx, ty = originY + dy, tz = baseZ + dz;
    const sp = new PIXI.Sprite(tex);
    sp.anchor.set(0.5, 1.0); // foot-anchored, exactly like the static pool in syncWorld
    sp.x = isoX(tx, ty);
    sp.y = isoY(tx, ty, tz) + HALF;
    sp.zIndex = depthZ(tx, ty, tz, 4); // same bias as real statics — interleaves with terrain naturally
    sp.alpha = 0.6; // preview, not a real building
    world.addChild(sp);
    placementTiles.push(sp);
  }
  markDirty();
}
// House-design placement ghost: the same idea as the multi-placement preview
// just above, scaled down to one tile — while a design session is open
// (scene.houseDesign) AND a catalog piece is selected (hdesignPiece), show
// that piece translucent at the hovered tile so a click's result is never a
// surprise (a design click is otherwise a plain target cursor with no sense
// of where/what will land — the server also drops a bad placement silently,
// see HDESIGN_MAX_GRAPHIC's doc, so a confident preview matters even more
// here). No ghost while erasing (there's nothing being placed to preview) or
// once nothing is selected. Single sprite, not a list — a design piece is one
// tile, never a multi-tile footprint — but otherwise foot-anchored/depth-
// sorted/alpha exactly like the footprint sprites above, and rebuilt only
// when the (piece, hovered tile) pair actually changes, same guard shape.
let hdesignHoverX = null, hdesignHoverY = null; // world tile under the cursor (design mode only)
let hdesignGhost = null;     // the single translucent PIXI.Sprite currently shown, or null
let hdesignGhostKey = null;  // last (piece, tile) drawn — dedups rebuilds
let hdesignGhostPending = false; // true if the last attempt skipped for want of a loaded texture
function clearHouseDesignGhost() {
  if (hdesignGhost) { world.removeChild(hdesignGhost); hdesignGhost.destroy(); hdesignGhost = null; }
  hdesignGhostKey = null;
  hdesignGhostPending = false;
}
// Called from poll() (to react to a fresh session, a piece pick, or the
// session/panel ending) and from the mousemove handler below (to react to the
// hovered tile changing) — same split responsibility as updatePlacementPreview.
function updateHouseDesignGhost() {
  const hd = scene && scene.houseDesign;
  if (!hd || !hdesignPiece || hdesignEraser || hdesignHoverX == null) {
    if (hdesignGhost) clearHouseDesignGhost();
    return;
  }
  const key = hdesignPiece.graphic + "@" + hdesignHoverX + "," + hdesignHoverY;
  if (key === hdesignGhostKey && !hdesignGhostPending) return; // nothing changed
  const tex = texFor(`art/static/${hdesignPiece.graphic}.png`);
  if (!tex) { hdesignGhostPending = true; return; } // texFor() will markDirty() once it loads; caught on a later poll
  hdesignGhostKey = key;
  hdesignGhostPending = false;
  if (hdesignGhost) { world.removeChild(hdesignGhost); hdesignGhost.destroy(); hdesignGhost = null; }
  const z = tileSZ(hdesignHoverX, hdesignHoverY) ?? (scene.player ? scene.player.z | 0 : 0);
  const sp = new PIXI.Sprite(tex);
  sp.anchor.set(0.5, 1.0); // foot-anchored, exactly like the static pool / multi preview above
  sp.x = isoX(hdesignHoverX, hdesignHoverY);
  sp.y = isoY(hdesignHoverX, hdesignHoverY, z) + HALF;
  sp.zIndex = depthZ(hdesignHoverX, hdesignHoverY, z, 4);
  sp.alpha = 0.6; // preview, not a placed piece
  world.addChild(sp);
  hdesignGhost = sp;
  markDirty();
}
// CSS/client pixels → renderer (global) pixels: the canvas is CSS-stretched from a
// capped internal buffer, so screen px ≠ renderer px (PIXI events use renderer px).
function clientToGlobal(clientX, clientY) {
  const r = app.canvas.getBoundingClientRect();
  return {
    x: (clientX - r.left) / r.width * app.renderer.width,
    y: (clientY - r.top) / r.height * app.renderer.height,
  };
}
// Crosshair + banner while the server waits for a target; Esc/answer hide it.
function endTargetUI() { targetUIHidden = true; updateTargetUI(); }
// ---- UO-style mouse cursors ------------------------------------------------
// Drawn to an offscreen canvas → PNG data URI, so there's no dependency on cursor
// gump art the play server doesn't ship. PIXI owns the canvas cursor (it swaps to
// `cursorStyles[mode]` as you hover entities), so we drive it through cursorStyles
// rather than fighting it with a raw style.cursor that PIXI would overwrite.
let CURSOR_ARROW = "auto", CURSOR_TARGET = "crosshair", CURSOR_TARGET_WAR = "crosshair";
function cursorFromCanvas(size, hotx, hoty, paint) {
  const c = document.createElement("canvas"); c.width = c.height = size;
  const g = c.getContext("2d"); paint(g);
  return `url("${c.toDataURL("image/png")}") ${hotx} ${hoty}, auto`;
}
function buildGameCursors() {
  // Gold arrow pointer — hotspot at the tip (1,1); dark outline reads on any terrain.
  CURSOR_ARROW = cursorFromCanvas(28, 1, 1, (g) => {
    const pts = [[1,1],[1,20],[6,15],[10,23],[13,22],[9,14],[16,14]];
    g.beginPath(); g.moveTo(pts[0][0], pts[0][1]);
    for (let i = 1; i < pts.length; i++) g.lineTo(pts[i][0], pts[i][1]);
    g.closePath();
    g.lineJoin = "round";
    g.lineWidth = 3; g.strokeStyle = "#1a1206"; g.stroke();   // dark halo
    g.fillStyle = "#f0d27a"; g.fill();                        // gold body
    g.lineWidth = 1; g.strokeStyle = "#8a6a1e"; g.stroke();   // rim
  });
  const reticle = (color) => cursorFromCanvas(32, 16, 16, (g) => {
    g.translate(16, 16); g.lineCap = "round";
    for (const [w, col] of [[3.5, "#12100a"], [1.6, color]]) { // dark halo, then colour
      g.lineWidth = w; g.strokeStyle = col; g.fillStyle = col;
      g.beginPath(); g.arc(0, 0, 9, 0, Math.PI * 2); g.stroke();            // ring
      for (const [dx, dy] of [[0,-1],[0,1],[-1,0],[1,0]]) {                 // ticks + centre gap
        g.beginPath(); g.moveTo(dx*4, dy*4); g.lineTo(dx*13, dy*13); g.stroke();
      }
      g.beginPath(); g.arc(0, 0, 1.4, 0, Math.PI * 2); g.fill();            // centre dot
    }
  });
  CURSOR_TARGET = reticle("#ffd23f");     // amber reticle — neutral/beneficial target
  CURSOR_TARGET_WAR = reticle("#ff4d4d"); // red reticle — war / harmful target
  applyCursorMode();
}
// Point the canvas cursor at the arrow, or the target reticle while a target cursor
// is up (red in war mode). Updates PIXI's cursorStyles so it survives entity hovers,
// and sets the style directly for an immediate switch (PIXI only re-applies on move).
function applyCursorMode() {
  if (!app || !app.renderer) return;
  const targeting = !!(scene && scene.target && scene.target.active === 1 && !targetUIHidden);
  const base = targeting ? ((scene && scene.war) ? CURSOR_TARGET_WAR : CURSOR_TARGET) : CURSOR_ARROW;
  const cs = app.renderer.events && app.renderer.events.cursorStyles;
  if (cs) { cs.default = base; cs.pointer = targeting ? base : CURSOR_ARROW; }
  if (app.canvas) app.canvas.style.cursor = base;
}

function updateTargetUI() {
  const active = !!(scene && scene.target && scene.target.active === 1);
  if (active && !prevTargetActive) targetUIHidden = false; // fresh request → show again
  prevTargetActive = active;
  const show = active && !targetUIHidden;
  applyCursorMode();               // arrow ↔ target reticle (red in war mode)
  const hint = document.getElementById("targethint");
  if (hint) hint.style.display = show ? "block" : "none";
  if (!show) clearTargetHighlight();   // target resolved/cancelled → drop any highlight
}
// While a target cursor is active, the entity (mobile or world item) under the cursor
// is tinted gold so the player can see exactly what they're about to target. Only one
// at a time; restored on pointer-out or when targeting ends.
let targetHL = null;   // { sp, tint } of the currently-highlighted sprite
const TARGET_HL_TINT = 0xffd24a;
function targetingActive() {
  return !!(scene && scene.target && scene.target.active === 1 && !targetUIHidden);
}
function clearTargetHighlight() {
  if (targetHL && targetHL.sp) { try { targetHL.sp.tint = targetHL.tint; } catch (_) {} markDirty(); }
  targetHL = null;
}
function targetHighlightOn(sp) {
  if (!targetingActive() || !sp || (targetHL && targetHL.sp === sp)) return;
  clearTargetHighlight();
  targetHL = { sp, tint: sp.tint };
  sp.tint = TARGET_HL_TINT;
  markDirty();
}
function targetHighlightOff(sp) {
  if (targetHL && targetHL.sp === sp) clearTargetHighlight();
}

