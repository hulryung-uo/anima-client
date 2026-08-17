// ---- persistent world pools ----
const tilePool = new Map();   // "x,y" -> {sp, g, z}
const staticPool = new Map(); // "x,y,g,z" -> sp
const animatedStatics = new Set(); // subset of staticPool sprites with _frames (flames/fountains)
const itemPool = new Map();   // serial -> {sp, g, x, y, z}  (dynamic world items: doors, furniture…)
// ---- hover tooltip (OPL — Object Property List) ----
// OPL = the full property list for an item/mobile: line 0 is the name, the rest
// are magical mods. The scene carries `scene.opl[serial]` = array of resolved
// lines (0xD6 MegaCliloc, resolved server-side via the Cliloc table). On hover we
// look it up; if absent we request it once (oplreq → 0xD6) and show "…" until it
// lands, then refresh.
const oplReq = new Set();     // serials we've already requested OPL for (this view)
let tipSerial = null;         // entity currently hovered (number)
// Render OPL lines into #tip: first line emphasized (the name), rest as mods.
// "no draw" is UO's placeholder name for invisible blocker/decoration objects
// (light sources, region markers, nodraw tiles). Never surface it as a label.
function isNoDraw(s) { return /^\s*no[\s_]?draw\s*$/i.test(String(s || "")); }
function showTipLines(lines) {
  const t = document.getElementById("tip");
  if (!t) return;
  // Drop "no draw" placeholder lines; if nothing meaningful remains, hide entirely.
  lines = (lines || []).filter((ln) => !isNoDraw(ln));
  if (!lines.length) { t.style.display = "none"; return; }
  t.innerHTML = "";
  lines.forEach((ln, i) => {
    const d = document.createElement("div");
    d.textContent = ln;
    d.className = i === 0 ? "tip-name" : "tip-mod";
    t.appendChild(d);
  });
  t.style.display = "block";
}
function showTip(txt) { showTipLines([txt]); }
function hideTip() { const t = document.getElementById("tip"); if (t) t.style.display = "none"; }
// Hover an entity (item OR mobile). Shows its OPL if we have it, else requests once.
function hoverEntity(serial) {
  if (!settings.tooltips) return;          // OPL tooltips disabled in Options
  serial = serial >>> 0;
  tipSerial = serial;
  const lines = scene && scene.opl ? scene.opl[serial] : null;
  if (lines && lines.length) { showTipLines(lines); return; }
  if (!oplReq.has(serial)) { oplReq.add(serial); sendInput("oplreq:" + serial); }
  showTip("…");
}
// Back-compat alias: world-item sprites hover through this.
function hoverItem(serial) { hoverEntity(serial); }
// Pointer left an entity: hide the tooltip. If OPL never arrived, forget the
// request so a later re-hover retries (the first one may have been dropped).
function hoverOut(serial) {
  serial = serial >>> 0;
  if (tipSerial === serial) { tipSerial = null; hideTip(); }
  const lines = scene && scene.opl ? scene.opl[serial] : null;
  if (!(lines && lines.length)) oplReq.delete(serial);
}
// Called each poll: if the hovered entity's OPL just arrived (or changed), refresh
// the visible tooltip in place.
function refreshTip() {
  if (pdTipEl) { renderEquipTip(); return; }   // paperdoll equip hover takes priority
  if (tipSerial == null) return;
  const lines = scene && scene.opl ? scene.opl[tipSerial] : null;
  if (lines && lines.length) showTipLines(lines);
}

// ---- paperdoll equip-icon tooltip (OPL + hair/beard dye swatch) ----
let pdTipEl = null;                 // the .eq-icon currently hovered (or null)
const hueHexCache = new Map();      // hue id → "#rrggbb"  (from /hue/<id>.json)
function hueHex(hue) {
  const id = hue & 0x3FFF;
  if (id === 0) return null;
  if (hueHexCache.has(id)) return hueHexCache.get(id);
  if (!hueHexCache.has("r" + id)) {
    hueHexCache.set("r" + id, 1);
    fetch(`hue/${id}.json`).then((r) => r.json())
      .then((j) => {
        hueHexCache.set(id, j.rgb);
        renderEquipTip(); applyHueSwatches(); applyWizHueSwatches();
        // The journal paints each line with the server's hue, but its render is
        // signature-gated and this table resolves one hue at a time, well after
        // the line was drawn. Without this the line keeps the per-type fallback
        // colour for good — observed live: speech that should have been
        // #9c9c00 stayed white because the fetch landed after the only render.
        invalidateJournal();
      }).catch(() => {});
  }
  return null;
}
function showEquipTip(ic) {
  pdTipEl = ic;
  const serial = (+ic.dataset.serial) >>> 0, layer = +ic.dataset.layer | 0;
  // Real items have an OPL (name/weight/AR/mods); request it once. Hair (11) and
  // facial hair (16) have none — we show the slot name + colour instead.
  if (layer !== 11 && layer !== 16) {
    const lines = scene && scene.opl ? scene.opl[serial] : null;
    if (!(lines && lines.length) && !oplReq.has(serial)) { oplReq.add(serial); sendInput("oplreq:" + serial); }
  }
  renderEquipTip();
}
function renderEquipTip() {
  const ic = pdTipEl;
  if (!ic) return;
  const t = document.getElementById("tip"); if (!t) return;
  const serial = (+ic.dataset.serial) >>> 0, layer = +ic.dataset.layer | 0, hue = +ic.dataset.hue | 0;
  const lines = scene && scene.opl ? scene.opl[serial] : null;
  const name = (lines && lines[0]) || EQUIP_SLOTS[layer] || ("Layer " + layer);
  let html = `<div class="tip-name">${esc(name)}</div>`;
  if (hue) {
    const hx = hueHex(hue);
    html += `<div class="tip-mod"><span class="tip-sw" style="background:${hx || "#777"}"></span>Hue ${hue & 0x3FFF}</div>`;
  }
  if (lines && lines.length > 1) {
    for (let i = 1; i < lines.length; i++) html += `<div class="tip-mod">${esc(lines[i])}</div>`;
  } else if (layer === 11) html += '<div class="tip-mod">hairstyle</div>';
  else if (layer === 16) html += '<div class="tip-mod">facial hair</div>';
  t.innerHTML = html;
  t.style.display = "block";
}
// --- hovering the DOLL figure directly: per-pixel hit-test through the stacked
// gump layers (topmost opaque pixel wins) so the cursor resolves the real item.
const _ac = document.createElement("canvas");
const _actx = _ac.getContext("2d", { willReadFrequently: true });
// img.src → ImageData. Each entry is a full decoded RGBA buffer (a 60×80 gump is
// ~19KB, a big body gump can run past 200KB) and a distinct hue/item makes a
// distinct src, so an unbounded cache pins more memory every dress/undress cycle.
// Small LRU instead: bounded at ALPHA_CACHE_MAX, touched (moved to MRU) on hit.
const ALPHA_CACHE_MAX = 32;
const alphaCache = new Map();
function imgAlpha(img, x, y) {
  const w = img.naturalWidth, hh = img.naturalHeight;
  if (!w || !img.complete || x < 0 || y < 0 || x >= w || y >= hh) return 0;
  let data = alphaCache.get(img.src);
  if (data) {
    alphaCache.delete(img.src); alphaCache.set(img.src, data); // touch → most-recently-used
  } else {
    _ac.width = w; _ac.height = hh; _actx.clearRect(0, 0, w, hh);
    try { _actx.drawImage(img, 0, 0); data = _actx.getImageData(0, 0, w, hh); }
    catch { return 0; }
    alphaCache.set(img.src, data);
    if (alphaCache.size > ALPHA_CACHE_MAX) alphaCache.delete(alphaCache.keys().next().value); // evict LRU
  }
  return data.data[(y * w + x) * 4 + 3];
}
// Topmost worn-layer <img> whose opaque pixel sits under the cursor, or null. Used
// for both the hover tooltip and per-pixel drag (so you grab the item you point at,
// not just the topmost layer the way native HTML5 drag would).
function dollImgAt(e) {
  const doll = document.getElementById("pd-doll");
  if (!doll) return null;
  const r = doll.getBoundingClientRect();
  const ix = Math.round(e.clientX - r.left - 40); // layers are shifted +40px (centering)
  const iy = Math.round(e.clientY - r.top);
  const imgs = doll.querySelectorAll("img[data-serial]");
  for (let i = imgs.length - 1; i >= 0; i--) {     // topmost layer first
    if (imgAlpha(imgs[i], ix, iy) > 20) return imgs[i];
  }
  return null;
}
function dollHitTest(e) {
  const img = dollImgAt(e);
  if (img) { showEquipTip(img); return; }
  if (pdTipEl && pdTipEl.closest && pdTipEl.closest("#pd-doll")) { pdTipEl = null; hideTip(); }
}

// The resolved item name (OPL line 0), or "" if its OPL hasn't arrived yet.
function oplName(serial) {
  const l = scene && scene.opl ? scene.opl[serial >>> 0] : null;
  return (l && l[0]) || "";
}
// Double-clicking another mobile's backpack attempts to snoop it (a crime in UO).
function snoopBackpack(serial) {
  sendInput("use:" + (serial >>> 0));   // double-click their pack = snoop attempt
  openContainer(serial);                // show the loot window if the snoop succeeds
  // (id, text, type, hue, now) — type 9 = yell → red default colour; hue 0 uses it.
  // Passing performance.now() as `hue` (the old 4-arg signature) made every warning a
  // random colour AND left `born` undefined, so `age` was NaN and it never expired.
  addOverhead("self", "⚠ Snooping is a crime — you may be flagged criminal!", 9, 0, performance.now());
}
// ---- per-entity interp state ----
const anim = new Map();       // id -> {rx,ry,tx,ty,z,dir,body,fallback,moveUntil}

// ---- client-side prediction for the player: a ClassicUO-style STEP QUEUE ----
// `pred` is the committed base tile (advances as steps complete) plus a small
// queue of predicted steps. The rendered position interpolates through the queue
// front (like Mobile.ProcessSteps) — it never free-runs ahead and then snaps
// back, so turning/stopping has no "slide backward then forward" artifact.
let pred = null;
// pred = { x,y,z,dir,           committed base tile
//          steps:[{x,y,z,dir,run,turn}],  queue (≤ MAX_STEPS)
//          t0,                  ms the front step started interpolating (carries over)
//          lastEnq, enqGate,    enqueue cadence gate (Walker.LastStepRequestTime)
//          moving,              for the walk/run animation
//          rx,ry,rz,            interpolated render position
//          sx,sy,sz, psx,psy }  last server pos / previous-poll server pos
const DIR_DELTA = [[0, -1], [1, -1], [1, 0], [1, 1], [0, 1], [-1, 1], [-1, 0], [-1, -1]];
const TURN_DELAY = 100;       // ClassicUO Constants.TURN_DELAY
const MAX_STEPS = 5;          // ClassicUO Constants.MAX_STEP_COUNT
// ClassicUO MovementSpeed.TimeToCompleteMovement: mounted halves, run halves again.
const stepDelay = (run, mounted) => (mounted ? (run ? 100 : 200) : (run ? 200 : 400));
// Fraction of a step over which the player's render Z (rz) eases from the source
// tile's Z to the step target — see the doc at its use in `processSteps`. < 1 so Z
// fully resolves BEFORE the tile boundary (ClassicUO does it in the first ~4 of a
// step's frames), leaving no residual to carry/bounce on a staircase.
const ZEASE_FRAC = 0.6;
// Don't enqueue a step whose tile would sit more than this far ahead of the last
// known server position — bounds how far the queue can lead (and thus the worst-
// case correction) without stalling at tile boundaries between polls.
const LEAD_CAP = 3.5;         // headroom over the ~2-tile steady lead so poll/cadence
                             // jitter never trips the enqueue stall (→ no micro-pause)
const SNAP_DIST = 4.5;        // hard resync only on a real desync/teleport (denies snap
                             // immediately via the denied flag, regardless of distance)
let lastDenies = 0;           // server DenyWalk count → clear queue + snap (ClassicUO DenyWalk→Reset)
let lastWalkSentAt = 0;       // perf-time of the last walk we sent. The server's confirm of
                             // it can lag one poll, briefly making the *previous* tile look
                             // "settled" — soft reconcile must ignore that window or it yanks
                             // the base back a tile then forward again ("뒤로갔다 앞으로").
const RECONCILE_HOLDOFF = 500; // ms after a walk before a small at-rest offset is trusted
const mounted = () => !!(scene && scene.player && scene.player.mounted);
const cheby = (a, b) => Math.max(Math.abs(a), Math.abs(b));

// ---- sitting (chairs/benches/stools/thrones) ----
// Real UO (and ClassicUO, which we verified against) never sends a packet for this:
// ServUO's chair items (Scripts/Items/Decorative/{Chairs,Stools,Benchs,Thrones}.cs)
// have no OnDoubleClick override at all, so double-clicking one server-side is a
// no-op. The classic 2D client instead recomputes, PURELY IN THE RENDERER every
// frame, whether the mobile it's drawing currently occupies the same map tile as an
// object whose GRAPHIC is one of a hardcoded set of "chair" ids (ClassicUO
// `ChairTable`/`Mobile.TryGetSittingInfo`) — if so it draws that mobile seated
// instead of standing, using a per-graphic table of allowed facings + pixel offsets.
// We port that table + its offset math faithfully, but trigger it from an explicit
// double-click-while-adjacent gesture instead of true same-tile occupancy: our
// walk predictor never actually steps the avatar onto the seat's tile (CLAUDE.md:
// the renderer never mutates World/prediction), so we fake the visual "step onto
// the chair" as a render-only overlay — `sitting` below — that's read *only* by
// drawMobs()/the camera/transparencyPass, never by the movement/prediction code.
// Ported from ClassicUO src/ClassicUO.Client/Game/Data/ChairTable.cs (_defaultTable), 171 entries.
// graphic -> [d1,d2,d3,d4,offsetY,mirrorOffsetY] (the 8th 'drawback' field — a rare
// cloak-behind-the-seat nuance for a handful of graphics — is not ported; skipping it
// only affects whether a worn cloak draws in front of or behind certain seats).
const CHAIR_TABLE = new Map([
  [0x0459, [0, -1, 4, -1, 2, 2]],
  [0x045A, [-1, 2, -1, 6, 2, 2]],
  [0x045B, [0, -1, 4, -1, 2, 2]],
  [0x045C, [-1, 2, -1, 6, 2, 2]],
  [0x0A2A, [0, 2, 4, 6, -4, -4]],
  [0x0A2B, [0, 2, 4, 6, -8, -8]],
  [0x0B2C, [-1, 2, -1, 6, 2, 2]],
  [0x0B2D, [0, -1, 4, -1, 2, 2]],
  [0x0B2E, [4, 4, 4, 4, 0, 0]],
  [0x0B2F, [2, 2, 2, 2, 6, 6]],
  [0x0B30, [6, 6, 6, 6, -8, 8]],
  [0x0B31, [0, 0, 0, 0, 0, 4]],
  [0x0B32, [4, 4, 4, 4, 0, 0]],
  [0x0B33, [2, 2, 2, 2, 0, 0]],
  [0x0B4E, [2, 2, 2, 2, 0, 0]],
  [0x0B4F, [4, 4, 4, 4, 0, 0]],
  [0x0B50, [0, 0, 0, 0, 0, 0]],
  [0x0B51, [6, 6, 6, 6, 0, 0]],
  [0x0B52, [2, 2, 2, 2, 0, 0]],
  [0x0B53, [4, 4, 4, 4, 0, 0]],
  [0x0B54, [0, 0, 0, 0, 0, 0]],
  [0x0B55, [6, 6, 6, 6, 0, 0]],
  [0x0B56, [2, 2, 2, 2, 4, 4]],
  [0x0B57, [4, 4, 4, 4, 4, 4]],
  [0x0B58, [6, 6, 6, 6, 0, 8]],
  [0x0B59, [0, 0, 0, 0, 0, 8]],
  [0x0B5A, [2, 2, 2, 2, 8, 8]],
  [0x0B5B, [4, 4, 4, 4, 8, 8]],
  [0x0B5C, [0, 0, 0, 0, 0, 8]],
  [0x0B5D, [6, 6, 6, 6, 0, 8]],
  [0x0B5E, [0, 2, 4, 6, -8, -8]],
  [0x0B5F, [-1, 2, -1, 6, 3, 14]],
  [0x0B60, [-1, 2, -1, 6, 3, 14]],
  [0x0B61, [-1, 2, -1, 6, 3, 14]],
  [0x0B62, [-1, 2, -1, 6, 3, 10]],
  [0x0B63, [-1, 2, -1, 6, 3, 10]],
  [0x0B64, [-1, 2, -1, 6, 3, 10]],
  [0x0B65, [0, -1, 4, -1, 3, 10]],
  [0x0B66, [0, -1, 4, -1, 3, 10]],
  [0x0B67, [0, -1, 4, -1, 3, 10]],
  [0x0B68, [0, -1, 4, -1, 3, 10]],
  [0x0B69, [0, -1, 4, -1, 3, 10]],
  [0x0B6A, [0, -1, 4, -1, 3, 10]],
  [0x0B91, [4, 4, 4, 4, 6, 6]],
  [0x0B92, [4, 4, 4, 4, 6, 6]],
  [0x0B93, [2, 2, 2, 2, 6, 6]],
  [0x0B94, [2, 2, 2, 2, 6, 6]],
  [0x0CF3, [-1, 2, -1, 6, 2, 8]],
  [0x0CF4, [-1, 2, -1, 6, 2, 8]],
  [0x0CF6, [0, -1, 4, -1, 2, 8]],
  [0x0CF7, [0, -1, 4, -1, 2, 8]],
  [0x0E50, [4, 4, 4, 4, 4, 4]],
  [0x0E51, [4, 4, 4, 4, 4, 4]],
  [0x0E52, [2, 2, 2, 2, 0, 0]],
  [0x0E53, [2, 2, 2, 2, 0, 0]],
  [0x1049, [-1, 2, -1, 6, 2, 2]],
  [0x104A, [0, -1, 4, -1, 2, 2]],
  [0x11FC, [0, 2, 4, 6, 2, 7]],
  [0x1207, [0, -1, 4, -1, 3, 10]],
  [0x1208, [0, -1, 4, -1, 3, 10]],
  [0x1209, [0, -1, 4, -1, 3, 10]],
  [0x120A, [0, -1, 4, -1, 3, 10]],
  [0x120B, [0, -1, 4, -1, 3, 10]],
  [0x120C, [0, -1, 4, -1, 3, 10]],
  [0x1218, [4, 4, 4, 4, 4, 4]],
  [0x1219, [2, 2, 2, 2, 4, 4]],
  [0x121A, [0, 0, 0, 0, 0, 8]],
  [0x121B, [6, 6, 6, 6, 0, 8]],
  [0x1527, [2, 2, 2, 2, 0, 0]],
  [0x1771, [0, 2, 4, 6, 0, 0]],
  [0x1776, [0, 2, 4, 6, 0, 0]],
  [0x1779, [0, 2, 4, 6, 0, 0]],
  [0x1DC7, [-1, 2, -1, 6, 3, 10]],
  [0x1DC8, [-1, 2, -1, 6, 3, 10]],
  [0x1DC9, [-1, 2, -1, 6, 3, 10]],
  [0x1DCA, [0, -1, 4, -1, 3, 10]],
  [0x1DCB, [0, -1, 4, -1, 3, 10]],
  [0x1DCC, [0, -1, 4, -1, 3, 10]],
  [0x1DCD, [-1, 2, -1, 6, 3, 10]],
  [0x1DCE, [-1, 2, -1, 6, 3, 10]],
  [0x1DCF, [-1, 2, -1, 6, 3, 10]],
  [0x1DD0, [0, -1, 4, -1, 3, 10]],
  [0x1DD1, [0, -1, 4, -1, 3, 10]],
  [0x1DD2, [-1, 2, -1, 6, 3, 10]],
  [0x2A58, [4, 4, 4, 4, 0, 0]],
  [0x2A59, [2, 2, 2, 2, 0, 0]],
  [0x2A5A, [0, 2, 4, 6, 0, 0]],
  [0x2A5B, [0, 2, 4, 6, 10, 10]],
  [0x2A7F, [0, 2, 4, 6, 0, 0]],
  [0x2A80, [0, 2, 4, 6, 0, 0]],
  [0x2DDF, [0, 2, 4, 6, 2, 2]],
  [0x2DE0, [0, 2, 4, 6, 2, 2]],
  [0x2DE3, [2, 2, 2, 2, 4, 4]],
  [0x2DE4, [4, 4, 4, 4, 4, 4]],
  [0x2DE5, [6, 6, 6, 6, 4, 4]],
  [0x2DE6, [0, 0, 0, 0, 4, 4]],
  [0x2DEB, [0, 0, 0, 0, 4, 4]],
  [0x2DEC, [4, 4, 4, 4, 4, 4]],
  [0x2DED, [2, 2, 2, 2, 4, 4]],
  [0x2DEE, [6, 6, 6, 6, 4, 4]],
  [0x2DF5, [0, 2, 4, 6, 4, 4]],
  [0x2DF6, [0, 2, 4, 6, 4, 4]],
  [0x3088, [0, 2, 4, 6, 4, 4]],
  [0x3089, [0, 2, 4, 6, 4, 4]],
  [0x308A, [0, 2, 4, 6, 4, 4]],
  [0x308B, [0, 2, 4, 6, 4, 4]],
  [0x319A, [-1, 2, -1, 6, 2, 2]],
  [0x319B, [0, -1, 4, -1, 2, 2]],
  [0x35ED, [0, 2, 4, 6, 0, 0]],
  [0x35EE, [0, 2, 4, 6, 0, 0]],
  [0x3DFF, [0, -1, 4, -1, 2, 2]],
  [0x3E00, [-1, 2, -1, 6, 2, 2]],
  [0x4023, [4, 4, 4, 4, 4, 4]],
  [0x4024, [2, 2, 2, 2, 0, 0]],
  [0x4027, [4, 4, 4, 4, 4, 4]],
  [0x4028, [4, 4, 4, 4, 4, 4]],
  [0x4029, [2, 2, 2, 2, 0, 0]],
  [0x402A, [2, 2, 2, 2, 0, 0]],
  [0x4BDC, [4, 4, 4, 4, 4, 4]],
  [0x4C1B, [4, 4, 4, 4, 4, 4]],
  [0x4C1E, [2, 2, 2, 2, 6, 6]],
  [0x4C80, [4, 4, 4, 4, 4, 4]],
  [0x4C81, [2, 2, 2, 2, 0, 0]],
  [0x4C82, [4, 4, 4, 4, 4, 4]],
  [0x4C83, [4, 4, 4, 4, 4, 4]],
  [0x4C84, [2, 2, 2, 2, 0, 0]],
  [0x4C85, [2, 2, 2, 2, 0, 0]],
  [0x4C86, [4, 4, 4, 4, 4, 4]],
  [0x4C87, [4, 4, 4, 4, 4, 4]],
  [0x4C88, [2, 2, 2, 2, 0, 0]],
  [0x4C89, [2, 2, 2, 2, 0, 0]],
  [0x4C8A, [2, 2, 2, 2, 0, 0]],
  [0x4C8B, [2, 2, 2, 2, 0, 0]],
  [0x4C8C, [2, 2, 2, 2, 0, 0]],
  [0x4C8D, [4, 4, 4, 4, 4, 4]],
  [0x4C8E, [4, 4, 4, 4, 4, 4]],
  [0x4C8F, [4, 4, 4, 4, 4, 4]],
  [0x4DE0, [2, 2, 2, 2, 0, 0]],
  [0x63BC, [0, -1, 4, -1, 3, 10]],
  [0x63BD, [0, -1, 4, -1, 3, 10]],
  [0x63C3, [-1, 2, -1, 6, 3, 14]],
  [0x63C4, [-1, 2, -1, 6, 3, 14]],
  [0x996C, [4, 4, 4, 4, 4, 4]],
  [0x9977, [2, 2, 2, 2, 0, 0]],
  [0x9C57, [6, 6, 6, 6, 6, 4]],
  [0x9C58, [6, 6, 6, 6, 6, 4]],
  [0x9C59, [0, 0, 0, 0, 4, 4]],
  [0x9C5A, [0, 0, 0, 0, 4, 4]],
  [0x9C5D, [6, 6, 6, 6, 6, 4]],
  [0x9C5E, [6, 6, 6, 6, 6, 4]],
  [0x9C5F, [6, 6, 6, 6, 6, 4]],
  [0x9C60, [0, 0, 0, 0, 4, 4]],
  [0x9C61, [0, 0, 0, 0, 4, 4]],
  [0x9C62, [0, 0, 0, 0, 4, 4]],
  [0x9E8E, [0, 0, 0, 0, 4, 4]],
  [0x9E8F, [6, 6, 6, 6, 6, 4]],
  [0x9E90, [2, 2, 2, 2, 0, 0]],
  [0x9E91, [4, 4, 4, 4, 4, 4]],
  [0x9E9F, [0, 0, 0, 0, 4, 4]],
  [0x9EA0, [6, 6, 6, 6, 6, 4]],
  [0x9EA1, [4, 4, 4, 4, 4, 4]],
  [0x9EA2, [2, 2, 2, 2, 0, 0]],
  [0xA05C, [6, 6, 6, 6, 6, 4]],
  [0xA05D, [4, 4, 4, 4, 4, 4]],
  [0xA05E, [0, 0, 0, 0, 4, 4]],
  [0xA05F, [2, 2, 2, 2, 0, 0]],
  [0xA211, [0, 2, 4, 6, -4, -4]],
  [0xA4EA, [4, 4, 4, 4, 4, 4]],
  [0xA4EB, [2, 2, 2, 2, 0, 0]],
  [0xA586, [4, 4, 4, 4, 4, 4]],
  [0xA587, [2, 2, 2, 2, 0, 0]],
]);

// ClassicUO Mobile.FixSittingDirection: snap the mobile's CURRENT facing to the
// nearest facing the chair actually supports (a chair often only allows 1 or 2 of
// the 4 cardinals — e.g. a bench against a wall only faces its own front/back).
// Then ClassicUO's GetSittingAnimDirection folds that resolved N/E/S/W onto one of
// only two real body-sprite directions people art has dedicated frames for (the
// other 6 of the 8 facings are mirrors) — N/W reuse the ONMOUNT_STAND group (a
// seated-leg pose that happens to read as "sitting" even off a horse); E/S have no
// good sit art at all, so CUO's fallback is "hold the plain Stand frame at the
// chair's pixel offset" plus `DrawCharacterSitted`'s three-band lean (upper
// 35% shifted ±8px, mid a trapezoid, lower unshifted). We port that deform.
// Offsets below fold in FixSittingDirection's SITTING_OFFSET_X=8 / SIT_OFFSET_Y=4
// constants, so the caller just adds {dx,dy} to the chair tile's screen position.
function chairSeatFor(rawDir, entry) {
  const [d1, d2, d3, d4, offsetY, mirrorOffsetY] = entry;
  let dir;
  switch (rawDir & 7) {
    case 7: case 0: dir = d1 !== -1 ? d1 : (rawDir === 7 ? d4 : d2); break;
    case 1: case 2: dir = d2 !== -1 ? d2 : (rawDir === 1 ? d1 : d3); break;
    case 3: case 4: dir = d3 !== -1 ? d3 : (rawDir === 3 ? d2 : d4); break;
    default:        dir = d4 !== -1 ? d4 : (rawDir === 5 ? d3 : d1); break; // 5, 6
  }
  switch (dir) {
    case 0: return { dir, group: 25, dx: 4, dy: 29 + mirrorOffsetY, flip: true };  // North
    case 6: return { dir, group: 25, dx: -3, dy: 27 + mirrorOffsetY, flip: false }; // West
    case 2: return { dir, group: 4, dx: 0, dy: 13 + offsetY, flip: true };         // East
    default: return { dir, group: 4, dx: -9, dy: 14 + offsetY, flip: false };       // South
  }
}

// { x, y, z, graphic, dir, group, dx, dy } of the chair we're seated on, or null.
// Render-overlay ONLY — see the comment above. Nothing outside drawMobs()/the
// camera math/transparencyPass ever reads this, and it never touches World state,
// `pred`, or the walk queue, so it can never desync movement.
let sitting = null;
function trySit(it) {
  if (!scene || !scene.player) return false;
  if (mounted()) return false; // ClassicUO: TryGetSittingInfo requires !IsMounted && !IsFlying
  const entry = CHAIR_TABLE.get(it.g | 0);
  if (!entry) return false;
  const ddx = (it.x | 0) - (scene.player.x | 0), ddy = (it.y | 0) - (scene.player.y | 0);
  if (cheby(ddx, ddy) > 1) return false; // must be standing on/adjacent to the chair
  if (Math.abs((it.z | 0) - (scene.player.z | 0)) > 2) return false; // ClassicUO gates same-tile sits at |Z diff| <= 1; a little slack since we sit from beside it, not on it
  const rawDir = ((pred ? pred.dir : scene.player.dir) | 0) & 7;
  const seat = chairSeatFor(rawDir, entry);
  sitting = { x: it.x | 0, y: it.y | 0, z: it.z | 0, graphic: it.g | 0, ...seat };
  markDirty();
  return true;
}
function standUp() {
  if (!sitting) return;
  sitting = null;
  markDirty();
}
// Leg-cycle fraction advanced per tile of ground covered. Walking = half a stride
// cycle per tile (one footstep → 80ms/frame for a 10-frame walk over a 400ms tile,
// matching ClassicUO). Running takes *bigger strides* (fewer cycles per tile), so
// its legs don't whirl: 0.32 → ~62ms/frame over a 200ms tile (between CUO's slow
// 80ms "skating" and a full-speed 40ms). Tune 0.32 up=faster legs / down=slower.
const cyclesPerTile = (run) => (run ? 0.32 : 0.5);

// ---- diagnostics ----
const diag = { fps: 0, poll: 0, sync: 0, tiles: 0, ents: 0, frames: 0, acc: 0, worstFrame: 0 };

// Cap the canvas's *internal* resolution and CSS-stretch it to fill the window.
// The profiler showed the JS thread ~94% idle — the cost is GPU pixel fill of a
// full-window (retina) canvas. Rendering a fixed ~1.1MP buffer and letting CSS
// upscale it (pixelated, so UO art stays crisp/blocky) bounds the fill cost
// regardless of monitor size, instead of rendering millions of pixels per frame.
const MAX_RENDER_PIX = 1_100_000;
function renderSize() {
  const w = window.innerWidth, h = window.innerHeight;
  const s = Math.min(1, Math.sqrt(MAX_RENDER_PIX / (w * h)));
  return { w: Math.max(320, Math.round(w * s)), h: Math.max(240, Math.round(h * s)) };
}


// ---- static filters (ClassicUO's StaticFilters) -----------------------------
//
// The player-facing half of UO's oldest visibility complaint: trees and bushes
// stand between you and everything. ClassicUO answers with two toggles — draw
// trees as stumps, and hide vegetation outright — plus the classification that
// makes tree/rock shadows possible at all.
//
// The tables come from `/staticfilters.json` rather than being copied here,
// because the tree/vegetation split is not a fixed list: ClassicUO files a
// "tree" seed under vegetation when tiledata says it is *not* impassable, and
// tiledata is the server's to read. Rocks and spell fields are ranges, not
// tables, so they stay here as the formulas they are.
const TREE_REPLACE_GRAPHIC = 0x0E59;   // ClassicUO Constants.TREE_REPLACE_GRAPHIC (a stump)
let staticFilters = null;              // { cave:Set, vegetation:Set, trees:Map }
let staticFiltersAsked = false;
function loadStaticFilters() {
  if (staticFiltersAsked) return;
  staticFiltersAsked = true;
  fetch("staticfilters.json").then((r) => r.json()).then((j) => {
    staticFilters = {
      cave: new Set(j.cave || []),
      vegetation: new Set(j.vegetation || []),
      trees: new Map(Object.entries(j.trees || {}).map(([g, h]) => [+g, h | 0])),
    };
    // The filters change which statics exist, so everything drawn under the old
    // (empty) tables has to be rebuilt once.
    rebuildStatics();
  }).catch(() => { staticFiltersAsked = false; });
}
const isTreeGraphic = (g) => !!staticFilters && staticFilters.trees.has(g | 0);
const isVegetationGraphic = (g) => !!staticFilters && staticFilters.vegetation.has(g | 0);
// ClassicUO `StaticFilters.IsRock` — a short switch plus one range.
const ROCK_GRAPHICS = new Set([4945, 4948, 4950, 4953, 4955, 4958, 4959, 4960, 4962]);
const isRockGraphic = (g) => ROCK_GRAPHICS.has(g | 0) || ((g | 0) >= 6001 && (g | 0) <= 6012);
// Should this static be drawn at all, and as what? Returns null to skip it.
// ClassicUO applies both rules in `GameSceneDrawingSorting`: foliage vanishes
// under TreeToStumps (that is what removes the canopy, leaving the trunk to be
// replaced), and vegetation vanishes under HideVegetation.
function filterStatic(g, foliage) {
  if (settings.treeStumps && foliage) return null;
  if (settings.hideVegetation && isVegetationGraphic(g)) return null;
  if (settings.treeStumps && isTreeGraphic(g)) return TREE_REPLACE_GRAPHIC;
  return g | 0;
}
// Trees, foliage and rocks cast one, exactly as mobiles do — the other half of
// ClassicUO's shadow support, which needed this classification to exist.
function staticCastsShadow(g, foliage) {
  return settings.shadows && settings.shadowsStatics
    && (isTreeGraphic(g) || !!foliage || isRockGraphic(g));
}
