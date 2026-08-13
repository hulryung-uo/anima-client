// ---- buff / debuff bar ----
// Display-only chips under the minimap. Each scene.buff = { icon, name, dur }.
// `dur` is the duration (seconds) the server sent; we record when an icon first
// appeared and count down locally (mm:ss). dur 0 = permanent (no timer). The bar
// has pointer-events:none so it never blocks clicks.
const buffSeen = new Map(); // icon -> { firstSeen: ms, dur: seconds, name }
// Names hinting a debuff → red tint; everything else is a (green) buff.
// Debuff tint. A heuristic over the buff's NAME, which is now the server's own
// localized title rather than the 35-entry English table it used to be — so it
// matches more of them, and mis-tints anything a shard names unusually. UO
// itself carries no buff/debuff flag on 0xDF; ClassicUO hardcodes the split by
// icon id instead.
const DEBUFF_RE = /poison|curse|weaken|clumsy|feeble|strangle|bleed|mortal|corpse|pain|evil omen|paralyze|sleep|blood oath|dismount|death/i;

// Buff-icon gump graphics, ported from ClassicUO BuffTable._defaultTable: indexed
// by `buff.icon - 0x3E9` (BuffIconType base). 0 = no art for that slot. Lets the
// buff bar show the real UO icon (gump) instead of text alone.
const BUFF_ICON_GUMPS = [
  0x754C,0x754A,0x0000,0x0000,0x755E,0x7549,0x7551,0x7556,0x753A,0x754D,0x754E,0x7565,0x753B,0x7543,0x7544,0x7546,
  0x755C,0x755F,0x7566,0x7554,0x7540,0x7568,0x754F,0x7550,0x7553,0x753E,0x755D,0x7563,0x7562,0x753F,0x7559,0x7557,
  0x754B,0x753D,0x7561,0x7558,0x755B,0x7560,0x7541,0x7545,0x7552,0x7569,0x7548,0x755A,0x753C,0x7547,0x7567,0x7542,
  0x758A,0x758B,0x758C,0x758D,0x0000,0x758E,0x094B,0x094C,0x094D,0x094E,0x094F,0x0950,0x753E,0x5011,0x7590,0x7591,
  0x7592,0x7593,0x7594,0x7595,0x7596,0x7598,0x7599,0x759B,0x759C,0x759E,0x759F,0x75A0,0x75A1,0x75A3,0x75A4,0x75A5,
  0x75A6,0x75A7,0x75C0,0x75C1,0x75C2,0x75C3,0x75C4,0x75F2,0x75F3,0x75F4,0x75F5,0x75F6,0x75F7,0x75F8,0x75F9,0x75FA,
  0x75FB,0x75FC,0x75FD,0x75FE,0x75FF,0x7600,0x7601,0x7602,0x7603,0x7604,0x7605,0x7606,0x7607,0x7608,0x7609,0x760A,
  0x760B,0x760C,0x760D,0x760E,0x760F,0x7610,0x7611,0x7612,0x7613,0x7614,0x7615,0x75C5,0x75F6,0x761B,0x9BC9,0x9BB5,
  0x9BDD,0x9BC6,0x9BCC,0x9BBE,0x9BBD,0x9BCB,0x9BC8,0x9BBF,0x9BCD,0x9BC0,0x9BCE,0x9BC1,0x9BC7,0x9BC2,0x9BB7,0x9BCA,
  0x9BB6,0x9BB8,0x9BB9,0x9BBA,0x9BBB,0x9BBC,0x9BC3,0x9BC4,0x9BC5,0x9BD2,0x9BD3,0x9BD4,0x9BD5,0x9BD1,0x9BD6,0x9BD7,
  0x9BCF,0x9BD8,0x9BD9,0x9BDB,0x9BDC,0x9BDA,0x9BD0,0x9BDE,0x9BDF,0xC349,0xC34D,0xC34E,0xC34C,0xC34B,0xC34A,0xC343,
  0xC345,0xC346,0xC347,0xC348,0x9CDE,0x5DE1,0x5DDF,0x5DE3,0x5DE5,0x5DE4,0x5DE6,0x5D51,0x0951,
];

function refreshBuffs(s) {
  const bar = document.getElementById("buffs");
  if (!bar) return;
  const list = (s && s.buffs) || [];
  // The buff/debuff icon bar (0xDF) is an AOS/SA feature. A T2A shard normally has
  // no buffs, so keep the bar hidden while it's empty — but a T2A-era shard can
  // still run modern ServUO and send a real buff, so show it (with its icon)
  // rather than silently swallow the buff we already parse. Non-T2A always shows.
  if (T2A && !list.length) { bar.style.display = "none"; return; }
  bar.style.display = "";
  const live = new Set(list.map((b) => b.icon));
  // Forget icons that are gone.
  for (const icon of [...buffSeen.keys()]) if (!live.has(icon)) buffSeen.delete(icon);
  // Record first-seen for new icons (and refresh dur on re-add).
  for (const b of list) {
    const prev = buffSeen.get(b.icon);
    if (!prev || prev.dur !== b.dur) buffSeen.set(b.icon, { firstSeen: performance.now(), dur: b.dur, name: b.name });
    else prev.name = b.name;
  }
  // Rebuild chips (cheap: a handful of buffs at most).
  bar.textContent = "";
  for (const b of list) {
    const el = document.createElement("div");
    el.className = "buff" + (DEBUFF_RE.test(b.name) ? " debuff" : "");
    el.dataset.icon = b.icon;
    // Real UO buff-icon art (gump), like ClassicUO's BuffGump. Map the 0xDF icon id
    // to its gump graphic; 0/absent means no art → just the name+timer text below.
    const gid = BUFF_ICON_GUMPS[(b.icon >>> 0) - 0x3E9] | 0;
    if (gid) {
      const img = document.createElement("img");
      img.className = "bi"; img.src = `gump/${gid}.png`; img.alt = ""; img.draggable = false;
      img.onerror = () => { img.remove(); }; // missing art → fall back to text-only
      el.appendChild(img);
    }
    const name = document.createElement("span"); name.className = "bn"; name.textContent = b.name;
    // The server's own description (0xDF's description cliloc, resolved
    // server-side) as a hover tooltip. Absent on shards that send none.
    if (b.desc) el.title = b.desc.replace(/<[^>]*>/g, "");
    const time = document.createElement("span"); time.className = "bt"; time.textContent = buffTimeText(b.icon);
    el.append(name, time);
    bar.appendChild(el);
  }
}

// mm:ss remaining for an icon, or "" when permanent / expired.
function buffTimeText(icon) {
  const st = buffSeen.get(icon);
  if (!st || !st.dur) return ""; // dur 0 = no timer
  const left = Math.max(0, st.dur - (performance.now() - st.firstSeen) / 1000);
  const m = Math.floor(left / 60), sec = Math.floor(left % 60);
  return m + ":" + String(sec).padStart(2, "0");
}

// Update just the countdown text each second (no DOM rebuild).
function tickBuffTimers() {
  const bar = document.getElementById("buffs");
  if (!bar) return;
  for (const el of bar.children) {
    const t = el.querySelector(".bt");
    if (t) t.textContent = buffTimeText(Number(el.dataset.icon));
  }
}

// ---- guard-zone (guard line) boundary overlay ----
// UO town "guard zones" are areas where NPC guards protect you — a crime just
// outside the line isn't. The client has no packet that carries a region's
// rectangle; the boundary is server-only data (anima-net's `regions.rs`,
// sourced from a local ServUO `Data/Regions.xml`), served at `/regions.json`
// already filtered to the CURRENT facet. `guardLineLayer` is a dedicated PIXI
// Graphics — a sibling of `world`/`entLayer`/`mobs`, added in `main()` right
// above `world` (terrain/statics/items) but below `entLayer`/`mobs`/`barLayer`
// /`overLayer` — so the lines read as ground markings and never cover a
// mobile, its name, or its HP bar. It's a plain child of `app.stage` like the
// others, so panning the camera (app.stage.position) moves it with everything
// else for free; we only ever rebuild its geometry, never reposition it.
let guardRects = [];        // [{x,y,w,h}, …] for the facet last successfully fetched
let guardRectsFacet = -1;   // facet guardRects was successfully fetched for (-1 = never fetched)
let guardRectsPending = -1; // facet currently in flight (-1 = no fetch in flight)
// Toggle: 'R' key (setupInput) or the Options panel checkbox both flip
// settings.guardZones — see renderOptions()/the opt-body "change" handler.
function toggleGuardZones() {
  settings.guardZones = !settings.guardZones;
  saveSettings();
  const cb = document.getElementById("opt-guardZones");
  if (cb) cb.checked = settings.guardZones;
  setStatus(settings.guardZones ? "guard-zone lines on" : "guard-zone lines off");
  updateGuardZones(scene);
}
// Called once per poll (~150ms — the same cadence drawMinimap/refreshBuffs use
// for their own per-tick redraws): (re)fetches `/regions.json` only when the
// facet changed since the last successful fetch (and isn't already in
// flight), then redraws the clipped-to-view lines so they track the player
// as they walk. `guardRectsFacet` is only committed once the fetch actually
// succeeds — committing it up front would mean a transient failure/empty
// response latches in a blank overlay for the rest of that facet's lifetime,
// since nothing would ever retry it. The in-flight response is also checked
// against the (possibly-since-changed) *current* `scene.facet` before being
// applied, so a rapid facet flip can't let a slow, stale response for the
// old facet overwrite the new facet's rects.
function updateGuardZones(s) {
  if (!settings.guardZones) { drawGuardZones(); return; } // off → drawGuardZones() clears the layer
  const facet = s && typeof s.facet === "number" ? s.facet : 0;
  if (facet === guardRectsFacet || facet === guardRectsPending) { drawGuardZones(); return; }
  guardRectsPending = facet;
  fetch("regions.json?" + Date.now())
    .then((r) => { if (!r.ok) throw new Error("regions.json " + r.status); return r.json(); })
    .then((rects) => {
      if (guardRectsPending === facet) guardRectsPending = -1;
      const curFacet = scene && typeof scene.facet === "number" ? scene.facet : 0;
      if (facet !== curFacet) return; // stale response for a facet we've since left — drop it
      guardRects = Array.isArray(rects) ? rects : [];
      guardRectsFacet = facet;
      drawGuardZones();
    })
    .catch(() => {
      if (guardRectsPending === facet) guardRectsPending = -1;
      // leave guardRectsFacet/guardRects untouched so the next poll retries
    });
}
// Rebuild the perimeter lines. Cheap even though a facet can carry ~90 guard
// rects (Felucca/Trammel; the rest far fewer): only rects whose bounding box
// overlaps the current visible tile window (scene.map.cx/cy ± radius, plus a
// small margin so an edge just off-screen still pokes in) are drawn — most of
// a facet's guard zones are nowhere near the player at any given moment.
function drawGuardZones() {
  if (!guardLineLayer) return;
  guardLineLayer.clear();
  if (settings.guardZones && scene && scene.map && guardRects.length) {
    const m = scene.map, margin = 4;
    const x0 = m.cx - m.radius - margin, x1 = m.cx + m.radius + margin;
    const y0 = m.cy - m.radius - margin, y1 = m.cy + m.radius + margin;
    for (const r of guardRects) {
      const rx1 = r.x + r.w, ry1 = r.y + r.h;
      if (rx1 < x0 || r.x > x1 || ry1 < y0 || r.y > y1) continue; // outside the view — skip
      const pts = [[r.x, r.y], [rx1, r.y], [rx1, ry1], [r.x, ry1]]
        .map(([tx, ty]) => [isoX(tx, ty), isoY(tx, ty, 0)]);
      const path = () => {
        guardLineLayer.moveTo(pts[0][0], pts[0][1]);
        for (let i = 1; i < pts.length; i++) guardLineLayer.lineTo(pts[i][0], pts[i][1]);
        guardLineLayer.closePath();
      };
      // A visible gold wash so standing INSIDE a guard zone reads at a glance,
      // then the boundary as a dark halo + bright gold core so the line stays
      // legible on both bright grass and dark roads (a thin single-colour stroke
      // vanished against the forest — the "not visible" report).
      path();
      guardLineLayer.fill({ color: 0xffcc33, alpha: 0.11 });
      path();
      guardLineLayer.stroke({ width: 5, color: 0x1a1205, alpha: 0.55 }); // dark halo
      path();
      guardLineLayer.stroke({ width: 2.5, color: 0xffd24a, alpha: 0.95 }); // bright core
    }
  }
  markDirty();
}

// ---- minimap / radar (top-down, north-up) ----
// Built from the scene's per-tile land colors (already sent for the iso view), so
// no extra server data. Player centered; mobiles/items as dots. Redrawn per poll.
let miniBuf = null;       // offscreen (2r+1)² color buffer, scaled onto the canvas
let miniOn = true;
function toggleMinimap() {
  miniOn = !miniOn;
  document.getElementById("minimap").style.display = miniOn ? "block" : "none";
  document.getElementById("minilabel").style.display = miniOn ? "block" : "none";
}
function drawMinimap(s) {
  const cv = document.getElementById("minimap");
  if (!miniOn || !cv) return;
  const m = s.map;
  if (!m || !m.tiles || !m.tiles.length) return;
  const n = 2 * m.radius + 1;
  if (!miniBuf || miniBuf.width !== n) {
    miniBuf = document.createElement("canvas");
    miniBuf.width = miniBuf.height = n;
  }
  const octx = miniBuf.getContext("2d");
  const img = octx.createImageData(n, n);
  for (let i = 0; i < n * n; i++) {
    const t = m.tiles[i];
    let r = 8, g = 9, b = 12;
    if (t && t.c) { r = t.c[0]; g = t.c[1]; b = t.c[2]; }
    if (t && t.h) { r >>= 2; g >>= 2; b >>= 2; }                 // hidden under cover → dim
    else if (t && t.w === 0 && t.g) { r = (r >> 1) + 60; g >>= 1; b >>= 1; } // blocked → reddish
    const o = i * 4;
    img.data[o] = r; img.data[o + 1] = g; img.data[o + 2] = b; img.data[o + 3] = 255;
  }
  octx.putImageData(img, 0, 0);
  const ctx = cv.getContext("2d");
  const w = cv.width, h = cv.height, cx = w / 2, cy = h / 2;
  ctx.clearRect(0, 0, w, h);
  // Draw the tile buffer in ISO orientation, matching the game's projection
  // (screen = ((x-y),(x+y))·iso). The transform maps buffer (a,b) → iso·(a-b,a+b);
  // drawImage at -radius puts the player's tile at the canvas center.
  // Scale so the square canvas is INSCRIBED in the iso diamond (its corners land
  // on the window's edge tiles) → the radar fills with map, no black corners.
  const iso = w / (2 * m.radius);
  ctx.save();
  ctx.translate(cx, cy);
  ctx.transform(iso, iso, -iso, iso, 0, 0);
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(miniBuf, -m.radius, -m.radius);
  ctx.restore();
  // Entities at the same iso position (dots drawn unrotated so they stay round).
  const isoDot = (wx, wy, color, size) => {
    const dx = wx - m.cx, dy = wy - m.cy;
    const px = cx + iso * (dx - dy), py = cy + iso * (dx + dy);
    if (px < -2 || py < -2 || px > w + 2 || py > h + 2) return;
    ctx.fillStyle = color; ctx.beginPath(); ctx.arc(px, py, size, 0, 7); ctx.fill();
  };
  // Skip what the world layer never draws: never-drawn pathing records (`nd` —
  // invisible tiles, ladder rungs, nodraw art) and season-culled foliage (`fh`).
  // The radar is a second draw path over the same array, so a record that is not
  // on screen must not be on the map either. (`fh` leaked here before this row.)
  for (const it of s.items || []) {
    if (it.nd || it.fh) continue;
    isoDot(it.x, it.y, "#e2b340", 1.4);
  }
  for (const mb of s.mobiles || []) isoDot(mb.x, mb.y, cssColor(notoColor(mb.noto)), 2);
  // player: white dot + facing tick (also iso-projected so it points where you face).
  ctx.fillStyle = "#fff"; ctx.beginPath(); ctx.arc(cx, cy, 2.6, 0, 7); ctx.fill();
  const dd = DIR_DELTA[(s.player && s.player.dir & 7) || 0];
  ctx.strokeStyle = "#fff"; ctx.lineWidth = 1.4; ctx.beginPath();
  ctx.moveTo(cx, cy); ctx.lineTo(cx + (dd[0] - dd[1]) * 5, cy + (dd[0] + dd[1]) * 5); ctx.stroke();
}

// ---- full world map (server-rendered facet PNG, shown ISO with pan/zoom) ----
const WORLDMAP_STEP = 1;     // must match scene::WORLDMAP_STEP (full-res world map)
let wmImg = null, wmLoading = false, wmOn = false;
let wmScale = 1.0;
let wmPan = { x: 0, y: 0 };
let wmOcean = "#0b1b2c";    // out-of-map fill (sampled from a deep-sea corner of the map)
let wmMouse = null;         // {x,y} cursor in canvas px → shown as a world coordinate
// Britannia (Felucca map0) landmarks → name labels drawn on the world map.
const PLACES = [
  [1424, 1696, "Britain"], [1832, 2768, "Trinsic"], [2899, 676, "Vesper"],
  [2477, 411, "Minoc"], [545, 992, "Yew"], [643, 2068, "Skara Brae"],
  [3714, 2237, "Magincia"], [4406, 1338, "Moonglow"], [1413, 3712, "Jhelom"],
  [2237, 1208, "Cove"], [3742, 1175, "Nujel'm"], [2976, 3438, "Serpent's Hold"],
  [2696, 2168, "Bucs Den"], [1496, 1628, "Castle Brit"], [3667, 2625, "Ocllo"],
  [5258, 3963, "Delucia"], [5680, 3120, "Papua"],
];
// User-placed markers, persisted in localStorage.
let wmMarkers = [];
try { wmMarkers = JSON.parse(localStorage.getItem("anima.markers") || "[]"); } catch (e) { wmMarkers = []; }
const saveMarkers = () => { try { localStorage.setItem("anima.markers", JSON.stringify(wmMarkers)); } catch (e) {} };

// ---- world-map points of interest (towns, banks, shops, dungeons, …) ----
// Server endpoint /pois.json → [{x,y,cat,name}, …]; fetched once, cached here.
let wmPois = null, wmPoisLoading = false;
// Map the ~73 raw `cat` strings into a handful of display groups. Any category
// not listed here falls into "Other" automatically (see buildPoiFilter()).
const POI_GROUPS = {
  Travel:   ["moongate", "gate", "teleporter", "docks", "shipwright", "bridge", "exit", "stairs", "customs", "stable", "gypsystable"],
  Services: ["bank", "gypsybank", "inn", "tavern", "healer", "mage", "library", "vet", "fortuneteller", "bard", "painter", "theater", "beekeeper"],
  Shops:    ["provisioner", "tailor", "blacksmith", "baker", "butcher", "jeweler", "carpenter", "tinker", "bowyer", "fletcher", "tanner", "arms", "reagents", "market"],
  Guilds:   ["guild", "warriors guild", "miners guild", "fishermans guild", "bardic guild", "armourers guild", "weapons guild", "thieves guild", "merchants guild", "tinkers guild", "chivalrykeeper", "rogues guild", "blacksmiths guild", "cavalry guild", "mages guild", "illusionists guild", "archers guild", "sorcerers guild", "holymage", "gypsymaiden"],
  Places:   ["town", "shrine", "dungeon", "champion", "landmark", "scenic", "ruins", "graveyard", "point of interest", "terrain", "body of water", "island", "marble patio", "minax's fortress"],
};
const POI_CAT_GROUP = {};   // cat → group name, derived from POI_GROUPS
for (const g in POI_GROUPS) for (const c of POI_GROUPS[g]) POI_CAT_GROUP[c] = g;
const POI_GROUP_ORDER = ["Travel", "Services", "Shops", "Guilds", "Places", "Other"];
// A sensible default-on set so the map isn't cluttered on first open.
const POI_DEFAULTS = ["moongate", "bank", "town", "shrine", "dungeon", "healer", "inn"];
let wmPoiCats = null;       // Set of enabled categories, persisted to localStorage
try { const s = JSON.parse(localStorage.getItem("anima.poiCats")); if (Array.isArray(s)) wmPoiCats = new Set(s); } catch (e) {}
if (!wmPoiCats) wmPoiCats = new Set(POI_DEFAULTS);
const savePoiCats = () => { try { localStorage.setItem("anima.poiCats", JSON.stringify([...wmPoiCats])); } catch (e) {} };
let wmPoiExpanded = new Set();   // which filter groups are expanded in the panel
// Distinct, readable colors for common categories; everything else gets a stable
// hash-based hue so each category is still visually separable.
const POI_COLORS = {
  moongate: "#b06aff", gate: "#9d7bff", teleporter: "#7c5cff", bank: "#ffd24a",
  gypsybank: "#e6b800", town: "#ffe08a", shrine: "#7fd8ff", dungeon: "#ff5c5c",
  champion: "#ff2e6b", healer: "#5cff8f", inn: "#ffb066", tavern: "#e0934a",
  mage: "#7aa7ff", provisioner: "#c9a06a", tailor: "#ff8fd0", blacksmith: "#9aa3ad",
  docks: "#4ad0c0", shipwright: "#3fb0c8", stable: "#c8a25a", library: "#9ad06a",
  graveyard: "#9a9aa8", landmark: "#d0c060", scenic: "#7fd09a", ruins: "#b08a6a",
};
function poiColor(cat) {
  if (POI_COLORS[cat]) return POI_COLORS[cat];
  let h = 0; for (let i = 0; i < cat.length; i++) h = (h * 31 + cat.charCodeAt(i)) >>> 0;
  return `hsl(${h % 360}, 62%, 62%)`;
}
function loadPois() {
  if (wmPois || wmPoisLoading) return;       // fetch only once; cache the result
  wmPoisLoading = true;
  fetch("pois.json").then(r => r.ok ? r.json() : Promise.reject()).then(d => {
    wmPois = Array.isArray(d) ? d : [];
    wmPoisLoading = false;
    buildPoiFilter();
    if (wmOn) drawWorldmap();
  }).catch(() => { wmPoisLoading = false; wmPois = []; });   // tolerate failure → just skip POIs
}
// Build the category-filter panel from the categories actually present, grouped.
function buildPoiFilter() {
  const host = document.getElementById("wmfilter");
  if (!host || !wmPois) return;
  const counts = {};
  for (const p of wmPois) { const c = (p.cat || "other"); counts[c] = (counts[c] || 0) + 1; }
  const groups = {};
  for (const c of Object.keys(counts)) {
    const g = POI_CAT_GROUP[c] || "Other";
    (groups[g] = groups[g] || []).push(c);
  }
  host.innerHTML = "";
  const title = document.createElement("div"); title.className = "wmf-title"; title.textContent = "POIs";
  host.appendChild(title);
  for (const g of POI_GROUP_ORDER) {
    const cats = groups[g]; if (!cats) continue;
    cats.sort();
    const on = cats.filter(c => wmPoiCats.has(c)).length;
    const grow = document.createElement("div"); grow.className = "wmf-grow";
    const gcb = document.createElement("input"); gcb.type = "checkbox";
    gcb.checked = on === cats.length; gcb.indeterminate = on > 0 && on < cats.length;
    gcb.title = "toggle all in group";
    gcb.addEventListener("change", () => {
      if (gcb.checked) cats.forEach(c => wmPoiCats.add(c)); else cats.forEach(c => wmPoiCats.delete(c));
      savePoiCats(); buildPoiFilter(); if (wmOn) drawWorldmap();
    });
    const head = document.createElement("div"); head.className = "wmf-ghead";
    const exp = document.createElement("span"); exp.className = "wmf-exp"; exp.textContent = wmPoiExpanded.has(g) ? "▾" : "▸";
    const lbl = document.createElement("span"); lbl.className = "wmf-glabel"; lbl.textContent = `${g} (${on}/${cats.length})`;
    head.appendChild(exp); head.appendChild(lbl);
    head.addEventListener("click", () => {
      if (wmPoiExpanded.has(g)) wmPoiExpanded.delete(g); else wmPoiExpanded.add(g);
      buildPoiFilter();
    });
    grow.appendChild(gcb); grow.appendChild(head);
    host.appendChild(grow);
    if (wmPoiExpanded.has(g)) {
      const body = document.createElement("div"); body.className = "wmf-body";
      for (const c of cats) {
        const row = document.createElement("label"); row.className = "wmf-crow";
        const cb = document.createElement("input"); cb.type = "checkbox"; cb.checked = wmPoiCats.has(c);
        cb.addEventListener("change", () => {
          if (cb.checked) wmPoiCats.add(c); else wmPoiCats.delete(c);
          savePoiCats(); buildPoiFilter(); if (wmOn) drawWorldmap();
        });
        const sw = document.createElement("span"); sw.className = "wmf-sw"; sw.style.background = poiColor(c);
        const nm = document.createElement("span"); nm.className = "wmf-cname"; nm.textContent = `${c} (${counts[c]})`;
        row.appendChild(cb); row.appendChild(sw); row.appendChild(nm);
        body.appendChild(row);
      }
      host.appendChild(body);
    }
  }
}
function loadWorldmap() {
  if (wmImg || wmLoading) return;
  wmLoading = true;
  const img = new Image();
  img.onload = () => {
    wmImg = img; wmLoading = false;
    // Sample a deep-ocean corner so the area outside the map blends with the sea
    // (ClassicUO shows the world surrounded by water — not black corners).
    try {
      const oc = document.createElement("canvas"); oc.width = img.width; oc.height = img.height;
      const octx = oc.getContext("2d"); octx.drawImage(img, 0, 0);
      const d = octx.getImageData(2, 2, 1, 1).data;
      if (d[0] + d[1] + d[2] > 0) wmOcean = `rgb(${d[0]},${d[1]},${d[2]})`;
    } catch (e) { /* tainted canvas etc. → keep default */ }
    if (wmOn) drawWorldmap();
  };
  img.onerror = () => { wmLoading = false; if (wmOn) setTimeout(loadWorldmap, 1500); }; // 503 while building → retry
  img.src = "worldmap.png?v=2";
}
function openWorldmap() {
  wmOn = true; wmPan = { x: 0, y: 0 };       // re-center on the player
  document.getElementById("worldmap").classList.add("on");
  held.clear();
  loadWorldmap(); loadPois(); buildPoiFilter(); drawWorldmap();
}
function closeWorldmap() { wmOn = false; document.getElementById("worldmap").classList.remove("on"); }
function toggleWorldmap() { wmOn ? closeWorldmap() : openWorldmap(); }
// Canvas px → world tile (x,y), inverting the iso transform used to draw the map.
function wmScreenToWorld(sx, sy, w, h) {
  const px = scene && scene.player ? scene.player.x : 0;
  const py = scene && scene.player ? scene.player.y : 0;
  const rx = sx - (w / 2 + wmPan.x), ry = sy - (h / 2 + wmPan.y), s = wmScale;
  const a = (rx + ry) / (2 * s), b = (ry - rx) / (2 * s); // image-pixel offset from player
  return [Math.round(px + a * WORLDMAP_STEP), Math.round(py + b * WORLDMAP_STEP)];
}
// World tile (x,y) → canvas px (forward iso transform; matches the drawn map).
function wmWorldToScreen(wx, wy, w, h) {
  const px = scene && scene.player ? scene.player.x : 0;
  const py = scene && scene.player ? scene.player.y : 0;
  const a = (wx - px) / WORLDMAP_STEP, b = (wy - py) / WORLDMAP_STEP, s = wmScale;
  return [w / 2 + wmPan.x + s * (a - b), h / 2 + wmPan.y + s * (a + b)];
}
function drawWorldmap() {
  if (!wmOn) return;
  const cv = document.getElementById("wmcanvas");
  const w = cv.clientWidth, h = cv.clientHeight;
  // Back the canvas at the device pixel ratio (retina) so labels/markers render at
  // native resolution instead of being CSS-upscaled (the source of the blur). The
  // context is scaled by dpr so all drawing below stays in CSS-pixel coordinates.
  const dpr = Math.min(window.devicePixelRatio || 1, 2);
  const bw = Math.round(w * dpr), bh = Math.round(h * dpr);
  if (cv.width !== bw || cv.height !== bh) { cv.width = bw; cv.height = bh; }
  const ctx = cv.getContext("2d");
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  // Fill out-of-map area with sea, not black, so the iso diamond's corners blend.
  ctx.fillStyle = wmOcean; ctx.fillRect(0, 0, w, h);
  if (!wmImg) { ctx.fillStyle = "#9aa0a6"; ctx.font = "14px monospace"; ctx.fillText("rendering world map…", 16, 26); return; }
  const px = scene && scene.player ? scene.player.x : 0;
  const py = scene && scene.player ? scene.player.y : 0;
  const ipx = px / WORLDMAP_STEP, ipy = py / WORLDMAP_STEP, s = wmScale;
  ctx.save();
  ctx.translate(w / 2 + wmPan.x, h / 2 + wmPan.y);
  ctx.transform(s, s, -s, s, 0, 0);          // iso: image pixel (a,b) → s·(a-b, a+b)
  ctx.imageSmoothingEnabled = true;          // bilinear → smooth, not blocky/low-res
  ctx.imageSmoothingQuality = "high";
  ctx.drawImage(wmImg, -ipx, -ipy);          // player's pixel at the origin
  ctx.restore();
  const wmLabelBoxes = []; // greedy de-collision: text labels skip drawing if they'd overlap an already-placed one (dots still draw). Places seed the list first (higher priority).
  // AABB via measureText (uses current ctx.font) + fixed line height; push+true if clear (or forced), false if it collides.
  const wmPlaceLabel = (x, y, align, str, force) => {
    const wd = ctx.measureText(str).width, pad = 2, lh = 12;
    const l = (align === "center" ? x - wd / 2 : x) - pad, r = l + wd + pad * 2;
    const t = y - lh / 2 - pad, b = y + lh / 2 + pad;
    if (!force) for (const q of wmLabelBoxes)
      if (l < q.r && r > q.l && t < q.b && b > q.t) return false;
    wmLabelBoxes.push({ l, r, t, b }); return true;
  };
  // place-name labels (cull off-canvas; fade names when zoomed far out).
  if (s >= 0.6) {
    ctx.textAlign = "center"; ctx.textBaseline = "middle";
    ctx.font = "11px ui-monospace, monospace"; ctx.lineWidth = 2.5;
    for (const [lx, ly, name] of PLACES) {
      const [sx, sy] = wmWorldToScreen(lx, ly, w, h);
      if (sx < 0 || sy < 0 || sx > w || sy > h) continue;
      wmPlaceLabel(sx, sy, "center", name, true);   // seed the box unconditionally; places always draw
      ctx.strokeStyle = "rgba(0,0,0,.85)"; ctx.strokeText(name, sx, sy);
      ctx.fillStyle = "#ffe08a"; ctx.fillText(name, sx, sy);
    }
  }
  // points of interest (filtered by enabled category); drawn UNDER the user
  // markers + player dot so those stay on top. Draw-only — never blocks clicks.
  if (wmPois && wmPois.length) {
    const showLabels = s >= 1.2;
    ctx.textAlign = "left"; ctx.textBaseline = "middle"; ctx.font = "10px ui-monospace, monospace";
    for (const p of wmPois) {
      const cat = p.cat || "other";
      if (!wmPoiCats.has(cat)) continue;
      const [sx, sy] = wmWorldToScreen(p.x, p.y, w, h);
      if (sx < -8 || sy < -8 || sx > w + 8 || sy > h + 8) continue;
      ctx.fillStyle = poiColor(cat); ctx.strokeStyle = "#0a0e12"; ctx.lineWidth = 1;
      ctx.beginPath(); ctx.arc(sx, sy, 3, 0, 7); ctx.fill(); ctx.stroke();
      // labels only when zoomed in, a name exists, and the box doesn't collide (first-come-wins).
      if (showLabels && p.name && wmPlaceLabel(sx + 5, sy, "left", p.name)) {
        ctx.lineWidth = 2.5; ctx.strokeStyle = "rgba(0,0,0,.85)";
        ctx.strokeText(p.name, sx + 5, sy); ctx.fillStyle = "#dfe6ee"; ctx.fillText(p.name, sx + 5, sy);
      }
    }
  }
  // user markers: a cyan pin + label, drawn over everything.
  ctx.textAlign = "left"; ctx.textBaseline = "middle"; ctx.font = "11px ui-monospace, monospace";
  for (const mk of wmMarkers) {
    const [sx, sy] = wmWorldToScreen(mk.x, mk.y, w, h);
    if (sx < -20 || sy < -20 || sx > w + 20 || sy > h + 20) continue;
    ctx.fillStyle = "#39d0ff"; ctx.strokeStyle = "#04121a"; ctx.lineWidth = 1.5;
    ctx.beginPath(); ctx.arc(sx, sy, 3.5, 0, 7); ctx.fill(); ctx.stroke();
    if (mk.name) {
      ctx.lineWidth = 2.5; ctx.strokeStyle = "rgba(0,0,0,.85)";
      ctx.strokeText(mk.name, sx + 6, sy); ctx.fillStyle = "#bfeeff"; ctx.fillText(mk.name, sx + 6, sy);
    }
  }
  // player marker sits where the origin lands (canvas center + pan).
  const mx = w / 2 + wmPan.x, my = h / 2 + wmPan.y;
  ctx.fillStyle = "#ff4d4d"; ctx.strokeStyle = "#fff"; ctx.lineWidth = 1.5;
  ctx.beginPath(); ctx.arc(mx, my, 4, 0, 7); ctx.fill(); ctx.stroke();
  // coordinate readouts (player + cursor), ClassicUO-style.
  ctx.textAlign = "left"; ctx.textBaseline = "top"; ctx.font = "12px ui-monospace, monospace";
  ctx.fillStyle = "rgba(8,11,16,.7)"; ctx.fillRect(8, 8, 168, wmMouse ? 36 : 20);
  ctx.fillStyle = "#e8ecf2"; ctx.fillText(`you  (${px}, ${py})`, 14, 12);
  if (wmMouse) {
    const [wx, wy] = wmScreenToWorld(wmMouse.x, wmMouse.y, w, h);
    ctx.fillStyle = "#9aa0a6"; ctx.fillText(`cur  (${wx}, ${wy})`, 14, 28);
  }
}
// Add / remove markers at a canvas point (double-click adds, shift-click removes
// the nearest one within ~10px).
function wmAddMarkerAt(sx, sy, w, h) {
  const [wx, wy] = wmScreenToWorld(sx, sy, w, h);
  const name = window.prompt(`Marker at (${wx}, ${wy}) — name:`, "");
  if (name === null) return;
  wmMarkers.push({ x: wx, y: wy, name: name.trim() });
  saveMarkers(); drawWorldmap();
}
function wmRemoveMarkerNear(sx, sy, w, h) {
  let best = -1, bestD = 12 * 12;
  for (let i = 0; i < wmMarkers.length; i++) {
    const [mx, my] = wmWorldToScreen(wmMarkers[i].x, wmMarkers[i].y, w, h);
    const d = (mx - sx) ** 2 + (my - sy) ** 2;
    if (d < bestD) { bestD = d; best = i; }
  }
  if (best >= 0) { wmMarkers.splice(best, 1); saveMarkers(); drawWorldmap(); }
}

// ---- info bar (ClassicUO's InfoBarManager, minus what the wire won't give us) ----
//
// A user-chosen set of readouts, as opposed to the fixed HUD block: pick the
// half-dozen numbers you actually watch and put them where you want them
// (the bar is an ordinary draggable window, so it remembers that).
//
// The field list is ClassicUO's `InfoBarVars`, restricted to what
// `scene.player` actually carries. The nine it is missing —
// LowerReagentCost, SpellDamageInc, FasterCasting, FasterCastRecovery,
// HitChanceInc, DefenseChanceInc, LowerManaCost, DamageChanceInc and
// SwingSpeedInc — are exactly the `0x11 type >= 6` block the core does not
// parse (its own open row in CLASSICUO_GAPS.md), so this is the whole of the
// set until that lands. The picker says so rather than leaving the absence to
// look like an oversight.
const INFOBAR_FIELDS = [
  { key: "hp",      label: "HP",      color: "#e5484d", get: (p) => `${p.hits}/${p.hitsMax}` },
  { key: "mana",    label: "Mana",    color: "#4f8cf7", get: (p) => `${p.mana}/${p.manaMax}` },
  { key: "stam",    label: "Stam",    color: "#46a758", get: (p) => `${p.stam}/${p.stamMax}` },
  { key: "weight",  label: "Weight",  color: "#d7cfa8",
    // Over capacity is the one field whose colour carries information: ServUO
    // refuses a pickup past `weightMax`, silently (see the grid-loot row).
    get: (p) => `${p.weight}/${p.weightMax}`,
    warn: (p) => (p.weight | 0) > (p.weightMax | 0) },
  { key: "follow",  label: "Pets",    color: "#c7d0dc", get: (p) => `${p.followers}/${p.followersMax}` },
  { key: "gold",    label: "Gold",    color: "#e3c34d", get: (p) => String(p.gold) },
  { key: "damage",  label: "Dmg",     color: "#e08a5a", get: (p) => `${p.damageMin}-${p.damageMax}` },
  { key: "armor",   label: "AR",      color: "#9aa0a6", get: (p) => String(p.armor) },
  { key: "luck",    label: "Luck",    color: "#b9a7ff", get: (p) => String(p.luck) },
  { key: "rfire",   label: "Fire",    color: "#ff7a45", get: (p) => String(p.resistFire) },
  { key: "rcold",   label: "Cold",    color: "#7fd4ff", get: (p) => String(p.resistCold) },
  { key: "rpois",   label: "Poison",  color: "#8fd14f", get: (p) => String(p.resistPoison) },
  { key: "renrg",   label: "Energy",  color: "#d98cff", get: (p) => String(p.resistEnergy) },
  { key: "statcap", label: "Cap",     color: "#c7d0dc", get: (p) => String(p.statsCap) },
  { key: "tithe",   label: "Tithe",   color: "#e3c34d", get: (p) => String(p.tithing) },
  { key: "noto",    label: "Noto",    color: "#c7d0dc", get: (p) => NOTO_NAMES[p.noto | 0] || String(p.noto | 0) },
  // The nine ClassicUO fields this bar could not offer until 0x11's `type >= 6`
  // tail was parsed. A shard that never sends that block reports 0 for all of
  // them, which is also what a character with no such bonuses looks like — the
  // packet has no way to say "absent", so neither has the bar.
  { key: "hci",     label: "HCI",     color: "#e08a5a", get: (p) => pct(p.hitChance) },
  { key: "dci",     label: "DCI",     color: "#9aa0a6", get: (p) => pct(p.defenseChance) },
  { key: "di",      label: "DI",      color: "#e08a5a", get: (p) => pct(p.damageChance) },
  { key: "ssi",     label: "SSI",     color: "#e3c34d", get: (p) => pct(p.swingSpeed) },
  { key: "lrc",     label: "LRC",     color: "#8fd14f", get: (p) => pct(p.lowerRegCost) },
  { key: "lmc",     label: "LMC",     color: "#4f8cf7", get: (p) => pct(p.lowerManaCost) },
  { key: "sdi",     label: "SDI",     color: "#d98cff", get: (p) => pct(p.spellDamage) },
  { key: "fc",      label: "FC",      color: "#b9a7ff", get: (p) => String(p.fasterCasting | 0) },
  { key: "fcr",     label: "FCR",     color: "#b9a7ff", get: (p) => String(p.fasterCastRecovery | 0) },
  // And the resistance CAPS, which arrive in the same block and are the other
  // half of reading a resistance number ("60 fire" means little without "of 70").
  { key: "capfire", label: "FireCap", color: "#ff7a45", get: (p) => String(p.maxResistFire | 0) },
  { key: "capcold", label: "ColdCap", color: "#7fd4ff", get: (p) => String(p.maxResistCold | 0) },
  { key: "cappois", label: "PoisCap", color: "#8fd14f", get: (p) => String(p.maxResistPoison | 0) },
  { key: "capenrg", label: "EnrgCap", color: "#d98cff", get: (p) => String(p.maxResistEnergy | 0) },
  { key: "capphys", label: "PhysCap", color: "#9aa0a6", get: (p) => String(p.maxResistPhysical | 0) },
];
const pct = (v) => `${v | 0}%`;
// ServUO `Notoriety`: 1 innocent, 2 friend, 3 grey/animal, 4 criminal,
// 5 enemy, 6 murderer, 7 invulnerable.
const NOTO_NAMES = { 1: "innocent", 2: "friend", 3: "grey", 4: "criminal", 5: "enemy", 6: "murderer", 7: "invul" };
const INFOBAR_DEFAULT = ["hp", "mana", "stam", "weight", "gold"];
let infoBarOn = localStorage.getItem("anima.infoBarOn") === "1";
let infoBarFields = INFOBAR_DEFAULT.slice();
try {
  const saved = JSON.parse(localStorage.getItem("anima.infoBarFields") || "null");
  if (Array.isArray(saved)) infoBarFields = saved;
} catch (e) {}
function saveInfoBar() {
  localStorage.setItem("anima.infoBarOn", infoBarOn ? "1" : "0");
  localStorage.setItem("anima.infoBarFields", JSON.stringify(infoBarFields));
}
function toggleInfoBar() {
  infoBarOn = !infoBarOn;
  document.getElementById("infobar").classList.toggle("on", infoBarOn);
  saveInfoBar();
  if (infoBarOn && scene) refreshInfoBar(scene);
}
function buildInfoBarPicker() {
  const p = document.getElementById("ib-picker");
  if (!p || p.childElementCount) return;
  for (const f of INFOBAR_FIELDS) {
    const l = document.createElement("label");
    const cb = document.createElement("input");
    cb.type = "checkbox"; cb.checked = infoBarFields.includes(f.key);
    cb.addEventListener("change", () => {
      // Keep the user's own order: a re-checked field goes back to the end
      // rather than jumping to wherever the table happens to list it.
      infoBarFields = cb.checked
        ? infoBarFields.concat([f.key])
        : infoBarFields.filter((k) => k !== f.key);
      saveInfoBar();
      if (scene) refreshInfoBar(scene, true);
    });
    l.appendChild(cb);
    l.appendChild(document.createTextNode(f.label));
    p.appendChild(l);
  }
  const note = document.createElement("div");
  note.className = "ib-note";
  note.textContent = "The combat modifiers and resistance caps ride in the 0x11 "
    + "status packet's `type >= 6` tail, which only an ML-or-later shard sends. "
    + "This one does not, so they all read 0 here.";
  p.appendChild(note);
}
// Rebuild only when a shown value changed — this runs on every poll.
function refreshInfoBar(s, force) {
  if (!infoBarOn) return;
  const host = document.getElementById("ib-fields");
  const p = s && s.player;
  if (!host || !p) return;
  const shown = infoBarFields
    .map((k) => INFOBAR_FIELDS.find((f) => f.key === k))
    .filter(Boolean);
  const sig = shown.map((f) => f.key + "=" + f.get(p) + (f.warn && f.warn(p) ? "!" : "")).join("|");
  if (!force && host._sig === sig) return;
  host._sig = sig;
  host.innerHTML = "";
  for (const f of shown) {
    const el = document.createElement("span");
    el.className = "ib-item";
    const lab = document.createElement("span");
    lab.className = "ib-l"; lab.textContent = f.label;
    const val = document.createElement("span");
    val.textContent = f.get(p);
    val.style.color = f.warn && f.warn(p) ? "#e5484d" : f.color;
    el.appendChild(lab); el.appendChild(val);
    host.appendChild(el);
  }
}

// ---- counter bar (ClassicUO's CounterBarGump) ------------------------------
//
// A row of cells, each pinned to an item GRAPHIC (optionally to one specific
// hue), showing how many of that item you are carrying and using one on
// double-click. It exists for reagents, potions and bandages: the count falls as
// you spend them without any bag being open.
//
// The count is a port of ClassicUO's `PlayerMobile.GetTotalAmountOfItem` +
// `Item.GetTotalAmount`: walk the worn items on layers 1..0x17 (which is where
// the backpack, 0x15, sits); if a worn item holds anything, recurse and count
// matches anywhere below it including nested bags; otherwise test the worn item
// itself. That last branch is why an *empty* worn pouch counts as one pouch —
// ClassicUO's `IsContainer && !IsEmpty` falls through the same way. "Holds
// nothing we have been told about" is the only form of that test available to a
// client, which raises the obvious worry: a bar reading zero until you have
// opened every bag. It doesn't. Instrumenting the 0x25 handler shows ServUO
// pushing one add-to-container per carried item at login, at every depth — 16
// for the worn pack, and the stack of 20 inside a nested bag nobody had ever
// opened. Nothing here has to ask; this bar sends no packet at all except when
// you use a slot.
const CB_LAYER_MIN = 1, CB_LAYER_MAX = 0x17;
const CB_FLASH_MS = 5000;      // ClassicUO HIGHLIGHT_AMOUNT_CHANGED_DURATION
const CB_EMPTY_SLOTS = 5;      // a fresh bar has somewhere to drag onto
let counterBarOn = localStorage.getItem("anima.counterBarOn") === "1";
let counterSlots = null;       // [{ g, hue: number|null, cmp }] — hue null = any hue
try {
  const saved = JSON.parse(localStorage.getItem("anima.counterSlots") || "null");
  if (Array.isArray(saved)) counterSlots = saved;
} catch (e) {}
if (!counterSlots) counterSlots = Array.from({ length: CB_EMPTY_SLOTS }, () => ({ g: 0, hue: null, cmp: 0 }));
let cbSel = -1;                // slot the options strip is editing; -1 = none
// ClassicUO's `CounterBarHighlightOnAmount` / `CounterBarHighlightAmount`, and
// its defaults (off, 5).
let cbWarnOn = localStorage.getItem("anima.cbWarnOn") === "1";
let cbWarnAt = parseInt(localStorage.getItem("anima.cbWarnAt") || "5", 10) || 5;
function saveCounterBar() {
  localStorage.setItem("anima.counterBarOn", counterBarOn ? "1" : "0");
  localStorage.setItem("anima.counterSlots", JSON.stringify(counterSlots));
  localStorage.setItem("anima.cbWarnOn", cbWarnOn ? "1" : "0");
  localStorage.setItem("anima.cbWarnAt", String(cbWarnAt));
}
// contItems is a flat list of "item X is inside container Y"; index it by
// container once per pass rather than re-scanning it per slot.
function cbIndex() {
  const byCont = new Map();
  for (const it of (scene && scene.contItems) || []) {
    const c = it.cont >>> 0;
    let a = byCont.get(c);
    if (!a) byCont.set(c, (a = []));
    a.push(it);
  }
  return byCont;
}
// Depth-first through everything below a container. `seen` guards a malformed
// cont-chain from looping forever; the server should never send one.
function cbWalk(byCont, serial, visit, seen) {
  seen = seen || new Set();
  serial = serial >>> 0;
  if (seen.has(serial)) return;
  seen.add(serial);
  for (const it of byCont.get(serial) || []) {
    cbWalk(byCont, it.serial >>> 0, visit, seen);
    visit(it);
  }
}
// Every item we are carrying, by ClassicUO's counting rule above. (Its own
// `FindItem` walks a fractionally different tree — it also considers a worn
// container itself, which the count does not. We use one rule for both, so a
// slot bound to a bag graphic finds a nested bag rather than the worn one.)
function cbEachCarried(visit) {
  const p = scene && scene.player;
  if (!p || !p.equip) return;
  const byCont = cbIndex();
  for (const e of p.equip) {
    const layer = e.layer | 0;
    if (layer < CB_LAYER_MIN || layer > CB_LAYER_MAX) continue;
    const kids = byCont.get(e.serial >>> 0);
    if (kids && kids.length) cbWalk(byCont, e.serial, visit);
    else visit(e);
  }
}
function cbMatches(it, g, hue) {
  return (it.g | 0) === (g | 0) && (hue == null || (it.hue | 0) === (hue | 0));
}
function cbCount(g, hue) {
  let total = 0;
  // A worn item has no `amount` field — it is one of itself.
  cbEachCarried((it) => { if (cbMatches(it, g, hue)) total += (it.amount | 0) || 1; });
  return total;
}
function cbFind(g, hue) {
  let best = null;
  cbEachCarried((it) => {
    if (!cbMatches(it, g, hue)) return;
    // Hue-agnostic slot: ClassicUO's FindItem keeps the LOWEST hue among the
    // matches (`minColor`) — the plainest one, so a stack of undyed reagents
    // goes before a dyed keepsake of the same graphic.
    if (!best || hue == null && (it.hue | 0) < (best.hue | 0)) best = it;
  });
  return best;
}
// ClassicUO `CalculateDisplayAmountText`: a lone item shows no number at all;
// with a compare-to set the label becomes the signed distance from it, and ±
// marks sitting exactly on target.
function cbText(amount, cmp) {
  const d = amount - (cmp | 0);
  if (!cmp) return d === 1 ? "" : String(d);
  return (d === 0 ? "±" : d > 0 ? "+" : "") + d;
}
function toggleCounterBar() {
  counterBarOn = !counterBarOn;
  document.getElementById("counterbar").classList.toggle("on", counterBarOn);
  saveCounterBar();
  if (counterBarOn) renderCounterSlots();
}
function renderCounterSlots() {
  const host = document.getElementById("cb-slots");
  if (!host) return;
  host.innerHTML = "";
  counterSlots.forEach((slot, i) => {
    const cell = document.createElement("div");
    cell.className = "cb-slot" + (i === cbSel ? " sel" : "");
    cell.dataset.i = String(i);
    if (slot.g) {
      const img = document.createElement("img");
      img.src = `art/static/${slot.g}.png${hueQuery(slot.hue || 0)}`;
      img.onerror = () => { img.style.visibility = "hidden"; };
      cell.appendChild(img);
      const amt = document.createElement("span");
      amt.className = "cb-amt";
      cell.appendChild(amt);
    }
    const x = document.createElement("span");
    x.className = "cb-x"; x.textContent = "×"; x.title = "remove this slot";
    cell.appendChild(x);
    host.appendChild(cell);
  });
  renderCounterOpts();
  refreshCounterBar(true);
}
// Bind a slot to what was just dropped on it (ClassicUO CounterItem.OnMouseUp).
function assignCounterSlot(i, g, hue) {
  if (!counterSlots[i]) return;
  counterSlots[i] = { g: g | 0, hue: hue | 0, cmp: counterSlots[i].cmp | 0 };
  saveCounterBar();
  renderCounterSlots();
}
// ClassicUO reaches the per-slot settings through a right-click context menu,
// which this client has no equivalent of; a strip under the bar showing the
// selected slot's settings puts the same three entries (ignore hue, compare to,
// remove) somewhere visible instead.
function renderCounterOpts() {
  const box = document.getElementById("cb-opts");
  if (!box) return;
  const slot = counterSlots[cbSel];
  box.classList.toggle("on", !!slot);
  box.innerHTML = "";
  if (!slot) return;
  const hueRow = document.createElement("label");
  const hueCb = document.createElement("input");
  hueCb.type = "checkbox"; hueCb.checked = slot.hue == null; hueCb.disabled = !slot.g;
  hueCb.addEventListener("change", () => {
    slot.hue = hueCb.checked ? null : (cbFind(slot.g, null) || { hue: 0 }).hue | 0;
    saveCounterBar();
    renderCounterSlots();
  });
  hueRow.appendChild(hueCb);
  hueRow.appendChild(document.createTextNode("Ignore hue"));
  box.appendChild(hueRow);
  const cmpRow = document.createElement("label");
  cmpRow.appendChild(document.createTextNode("Compare to"));
  const cmp = document.createElement("input");
  cmp.type = "number"; cmp.value = String(slot.cmp | 0);
  cmp.addEventListener("change", () => {
    slot.cmp = parseInt(cmp.value, 10) || 0;
    saveCounterBar();
    refreshCounterBar(true);
  });
  cmpRow.appendChild(cmp);
  box.appendChild(cmpRow);
  const warnRow = document.createElement("label");
  const warnCb = document.createElement("input");
  warnCb.type = "checkbox"; warnCb.checked = cbWarnOn;
  warnCb.addEventListener("change", () => { cbWarnOn = warnCb.checked; saveCounterBar(); refreshCounterBar(true); });
  const warnAt = document.createElement("input");
  warnAt.type = "number"; warnAt.value = String(cbWarnAt);
  warnAt.addEventListener("change", () => {
    cbWarnAt = parseInt(warnAt.value, 10) || 0;
    saveCounterBar();
    refreshCounterBar(true);
  });
  warnRow.appendChild(warnCb);
  warnRow.appendChild(document.createTextNode("Warn below"));
  warnRow.appendChild(warnAt);
  box.appendChild(warnRow);
}
function cbSelect(i) {
  cbSel = cbSel === i ? -1 : i;
  const host = document.getElementById("cb-slots");
  if (host) for (const cell of host.children) cell.classList.toggle("sel", (+cell.dataset.i) === cbSel);
  renderCounterOpts();
}
function removeCounterSlot(i) {
  counterSlots.splice(i, 1);
  if (cbSel === i) cbSel = -1; else if (cbSel > i) cbSel--;
  saveCounterBar();
  renderCounterSlots();
}
function addCounterSlot() {
  counterSlots.push({ g: 0, hue: null, cmp: 0 });
  saveCounterBar();
  renderCounterSlots();
}
function useCounterSlot(i) {
  const slot = counterSlots[i];
  if (!slot || !slot.g) return;
  const it = cbFind(slot.g, slot.hue);
  // ClassicUO does nothing at all here; say why instead, since a slot that
  // silently ignores a double-click reads as a broken slot.
  if (!it) { addSysMessage("You are not carrying one of those."); return; }
  sendInput("use:" + (it.serial >>> 0));
}
// Amounts only — the cells themselves are rebuilt by renderCounterSlots. Runs on
// every poll, so each cell keeps its last count and only touches the DOM when
// that changed.
function refreshCounterBar(force) {
  if (!counterBarOn) return;
  const host = document.getElementById("cb-slots");
  if (!host) return;
  for (const cell of host.children) {
    const slot = counterSlots[+cell.dataset.i];
    if (!slot || !slot.g) continue;
    const amount = cbCount(slot.g, slot.hue);
    const shown = cbText(amount, slot.cmp);
    if (!force && cell._amt === amount && cell._shown === shown) continue;
    const had = cell._amt;
    cell._amt = amount; cell._shown = shown;
    const label = cell.querySelector(".cb-amt");
    if (label) label.textContent = shown;
    // ClassicUO tints the cell on a change and fades it over 5s — icelight for a
    // gain, firelight for a loss. Worth keeping: it is what tells you a reagent
    // was just spent when the bar is at the edge of your eye.
    if (had != null && amount !== had && !force) {
      cell.style.transition = "none";
      cell.style.background = amount > had ? "rgba(70, 167, 88, .35)" : "rgba(229, 72, 77, .35)";
      void cell.offsetWidth;                       // commit the jump before transitioning off it
      cell.style.transition = `background ${CB_FLASH_MS}ms linear`;
      cell.style.background = "";
    }
    cell.classList.toggle("low", cbWarnOn && amount - (slot.cmp | 0) < cbWarnAt);
    // Hovering a cell shows the real item's OPL, exactly as hovering it in a bag
    // does (ClassicUO's CounterItem.OnMouseOver does the same lookup).
    const it = cbFind(slot.g, slot.hue);
    if (it) cell.dataset.serial = String(it.serial >>> 0); else delete cell.dataset.serial;
  }
}


// ---- network stats (ClassicUO's NetworkStatsGump) --------------------------
//
// ClassicUO shows one link because it has one: client ⇄ game server. This
// client has two — browser ⇄ play server ⇄ game server — and a lag reading
// that silently blamed the wrong hop would be worse than none, so both are
// shown.
//
// The UO half is measured in the driver that owns the socket (`NetStats`): a
// real 0x73 round trip, ServUO echoing the sequence byte back, averaged over
// the last five as ClassicUO does. The browser half is the poll this page
// already times for the diag line.
//
// Rates are differenced HERE rather than server-side: ClassicUO recomputes its
// deltas on a 500ms timer, but the scene arrives on this page's own cadence, so
// the honest denominator is the time between the two samples we actually saw.
// ClassicUO's own thresholds, in milliseconds: green under 150, yellow under
// 200, orange under 300, red beyond.
const NET_PING_COLORS = [[150, "#46a758"], [200, "#e3c34d"], [300, "#e08a5a"]];
const NET_BAD_COLOR = "#e5484d";
let netStatsOn = localStorage.getItem("anima.netStatsOn") === "1";
let netPrev = null;      // counters as of the last rate tick
let netRate = { in: 0, out: 0 };
function toggleNetStats() {
  netStatsOn = !netStatsOn;
  document.getElementById("netstats").classList.toggle("on", netStatsOn);
  localStorage.setItem("anima.netStatsOn", netStatsOn ? "1" : "0");
  netPrev = null; netRate = { in: 0, out: 0 };   // a fresh window starts fresh
  if (netStatsOn) refreshNetStats();
}
// ClassicUO's `NetStatistics.GetSizeAdaptive`, including its two decimals.
function netSize(bytes) {
  let n = bytes, unit = "B";
  for (const next of ["KB", "MB", "GB"]) {
    if (n < 1024) break;
    n /= 1024; unit = next;
  }
  return `${Math.round(n * 100) / 100} ${unit}`;
}
function netPingColor(ms) {
  for (const [limit, color] of NET_PING_COLORS) if (ms < limit) return color;
  return NET_BAD_COLOR;
}
function refreshNetStats() {
  if (!netStatsOn) return;
  const host = document.getElementById("ns-body");
  const n = scene && scene.net;
  if (!host || !n) return;
  // Rates over a full second, recomputed once a second. The poll is 150ms, and
  // differencing per poll made the number flicker to 0 the instant traffic
  // paused — true, and useless. ClassicUO's 500ms delta timer exists for the
  // same reason.
  const now = performance.now();
  if (!netPrev) netPrev = { t: now, in: n.in, out: n.out };
  const dt = (now - netPrev.t) / 1000;
  if (dt >= 1) {
    netRate = { in: (n.in - netPrev.in) / dt, out: (n.out - netPrev.out) / dt };
    netPrev = { t: now, in: n.in, out: n.out };
  }
  // Microseconds on the wire, null until a ping has come back. A shard on this
  // machine answers in a few hundred µs, so print sub-millisecond times as
  // such instead of rounding them to the "0 ms" ClassicUO shows there.
  const us = n.pingUs;
  const ping = us == null ? null : us / 1000;
  const pingText = ping == null ? "no reply yet"
    : ping < 1 ? `${ping.toFixed(2)} ms` : `${Math.round(ping)} ms`;
  const sig = [us, n.in, n.out, n.pin, n.pout,
    Math.round(netRate.in), Math.round(netRate.out), Math.round(diag.poll)].join(":");
  if (host._sig === sig) return;
  host._sig = sig;
  host.innerHTML =
    `<div class="ns-hdr">game server (0x73 round trip)</div>`
    + `<div class="ns-row"><span>Ping</span><span style="color:${ping == null ? "#6b7280" : netPingColor(ping)}">${pingText}</span></div>`
    + `<div class="ns-row"><span>In</span><span>${netSize(netRate.in)}/s</span></div>`
    + `<div class="ns-row"><span>Out</span><span>${netSize(netRate.out)}/s</span></div>`
    + `<div class="ns-row ns-tot"><span>Total</span><span>${netSize(n.in)} in · ${netSize(n.out)} out</span></div>`
    + `<div class="ns-row ns-tot"><span>Packets</span><span>${n.pin} in · ${n.pout} out</span></div>`
    + `<div class="ns-hdr">this page (HTTP poll)</div>`
    + `<div class="ns-row"><span>Scene fetch</span><span>${diag.poll.toFixed(0)} ms</span></div>`;
}
// ---- object inspector (ClassicUO's InspectorGump) ---------------------------
//
// Arm it, click a thing, and read back everything this client believes about
// it. ClassicUO's version is a fixed key/value dump per object type — graphic,
// hue, position, then a hand-written list per Mobile/Item/Land/Static — and
// clicking a value copies it. Same here, with the same field names where the
// two clients have the same field.
//
// One addition, which is the reason this window is worth having in *this*
// project: the raw scene record underneath. ClassicUO's list silently omits
// anything nobody hardcoded, and the interesting question here is usually
// "what did the server actually send", not "what did someone remember to
// print". The table is the readable view; the JSON is the complete one.
let inspectOn = localStorage.getItem("anima.inspectOn") === "1";
let inspectPick = false;
let inspectData = null;   // { kind, title, rows: [[k, v], …], raw }
function toggleInspector() {
  inspectOn = !inspectOn;
  document.getElementById("inspector").classList.toggle("on", inspectOn);
  localStorage.setItem("anima.inspectOn", inspectOn ? "1" : "0");
  if (!inspectOn) armInspect(false);
  renderInspector();
}
function armInspect(on) {
  inspectPick = !!on;
  const b = document.getElementById("insp-pick");
  if (b) {
    b.classList.toggle("arm", inspectPick);
    b.textContent = inspectPick ? "click anything…" : "Inspect…";
  }
}
const hex4 = (n) => "0x" + ((n | 0) & 0xFFFF).toString(16).toUpperCase().padStart(4, "0");
const hex8 = (n) => "0x" + ((n >>> 0).toString(16).toUpperCase().padStart(8, "0"));
// Chebyshev distance, which is UO's own notion of range (`GameObject.Distance`).
function tileDistance(x, y) {
  const p = scene && scene.player;
  if (!p) return "";
  return String(Math.max(Math.abs((x | 0) - p.x), Math.abs((y | 0) - p.y)));
}
function inspectMobile(serial) {
  const p = scene && scene.player;
  const self = p && (p.serial >>> 0) === (serial >>> 0);
  const m = self ? p : ((scene && scene.mobiles) || []).find((x) => (x.serial >>> 0) === (serial >>> 0));
  if (!m) return null;
  const rows = [
    ["Serial", hex8(m.serial)], ["Name", m.name || ""], ["Graphics", hex4(m.body)],
    ["Hue", hex4(m.hue)], ["Position", `X=${m.x}, Y=${m.y}, Z=${m.z}`],
    ["Distance", tileDistance(m.x, m.y)], ["Direction", String(m.dir | 0)],
    ["HP", `${m.hits | 0}/${m.hitsMax | 0}`],
    ["Notoriety", NOTO_NAMES[m.noto | 0] || String(m.noto | 0)],
    ["IsMounted", String(!!m.mounted)], ["IsHidden", String(!!m.hidden)],
    ["IsPoisoned", String(!!m.poisoned)], ["IsInvulnerable", String(!!m.yellow)],
    ["AnimType", String(m.at | 0)], ["Equipment", String((m.equip || []).length)],
  ];
  if (self) {
    rows.push(["Mana", `${p.mana | 0}/${p.manaMax | 0}`], ["Stamina", `${p.stam | 0}/${p.stamMax | 0}`],
      ["Race", String(p.race | 0)], ["IsDead", String(!!p.dead)]);
  }
  const opl = (scene && scene.opl && scene.opl[m.serial >>> 0]) || null;
  if (opl) rows.push(["OPL", opl.join(" · ")]);
  return { kind: "Mobile", title: m.name || hex8(m.serial), rows, raw: m };
}
function inspectItem(serial) {
  serial = serial >>> 0;
  const world = ((scene && scene.items) || []).find((x) => (x.serial >>> 0) === serial);
  const held = ((scene && scene.contItems) || []).find((x) => (x.serial >>> 0) === serial);
  const it = world || held;
  if (!it) return null;
  const rows = [
    ["Serial", hex8(it.serial)], ["Graphics", hex4(it.g)], ["Hue", hex4(it.hue)],
    ["Amount", String((it.amount | 0) || 1)], ["IsContainer", String(!!it.c)],
    ["IsStackable", String(!!it.st)],
  ];
  if (world) {
    rows.push(["Position", `X=${it.x}, Y=${it.y}, Z=${it.z}`], ["Distance", tileDistance(it.x, it.y)]);
  } else {
    // A held item's x/y are the container gump's own pixel space, not tiles —
    // labelling them "Position" like a world item's would be a lie.
    rows.push(["Container", hex8(it.cont)], ["Slot", `X=${it.x}, Y=${it.y} (gump px)`]);
  }
  const opl = (scene && scene.opl && scene.opl[serial]) || null;
  if (opl) rows.push(["OPL", opl.join(" · ")]);
  return { kind: world ? "Item" : "Item (in container)", title: (opl && opl[0]) || hex8(serial), rows, raw: it };
}
// A land tile plus whatever statics stand on it. ClassicUO inspects one object,
// but our picking resolves a click to a tile, and "what is on this square" is
// the question a tile click is actually asking.
function inspectTile(x, y) {
  const map = scene && scene.map;
  if (!map) return null;
  const r = map.radius | 0, w = r * 2 + 1;
  const dx = (x | 0) - (map.cx | 0) + r, dy = (y | 0) - (map.cy | 0) + r;
  if (dx < 0 || dy < 0 || dx >= w || dy >= w) return null;
  const t = map.tiles[dy * w + dx];
  if (!t) return null;
  const rows = [
    ["Position", `X=${x}, Y=${y}, Z=${t.z}`], ["Graphics", hex4(t.g)],
    ["Texture", t.tx ? hex4(t.tx) : "none (flat draw)"],
    ["Distance", tileDistance(x, y)],
    ["Walkable", String(!!t.w)], ["StandZ", String(t.sz | 0)],
    // The four corner lights, in vertex order — the cheap oracle for the
    // ClassicUO port: a level corner must read exactly 0.854 at any terrain
    // shading level, and nothing may fall below 0.323 at the default 15.
    ["Light T/R/B/L", typeof cornerLight === "function"
      ? [cornerLight(x, y), cornerLight(x + 1, y), cornerLight(x + 1, y + 1), cornerLight(x, y + 1)]
          .map((v) => v.toFixed(3)).join(" ")
      : "—"],
    ["Impassable", String(!!t.i)], ["UnderRoof", String(!!t.h)],
  ];
  const on = (typeof staticsAt === "function" && staticsAt(x | 0, y | 0)) || [];
  rows.push(["Statics", String(on.length)]);
  on.slice(0, 12).forEach((s, i) => {
    // `hidden` = ceiling-hidden (`hz`): drawn at alpha 0, and deliberately
    // shipped without `h`/`pf`, so the inspector says so rather than letting a
    // reader conclude the tiledata is missing. Before this row these statics
    // were absent from the stream entirely, so the inspector under-reported what
    // was on a tile whenever the player was indoors.
    rows.push([`Static ${i}`, `${hex4(s.g)} z=${s.z}${s.h != null ? ` h=${s.h}` : ""}${s.ms ? " (multi)" : ""}${s.hz ? " (ceiling-hidden)" : ""}${s.nd ? " (never drawn)" : ""}`]);
  });
  return { kind: "Land tile", title: `${x}, ${y}`, rows, raw: { tile: t, statics: on } };
}
// Called from the click paths. Returns true when the armed pick consumed it.
function inspectPicked(what) {
  if (!inspectPick) return false;
  armInspect(false);
  const data = what.serial != null
    ? (inspectMobile(what.serial) || inspectItem(what.serial))
    : inspectTile(what.x, what.y);
  if (!data) { addSysMessage("Nothing to inspect there."); return true; }
  inspectData = data;
  if (!inspectOn) toggleInspector(); else renderInspector();
  return true;
}
function renderInspector() {
  if (!inspectOn) return;
  const host = document.getElementById("insp-body");
  if (!host) return;
  if (!inspectData) {
    host.innerHTML = '<div class="insp-none">Press Inspect, then click a mobile, an item or the ground.</div>';
    return;
  }
  const d = inspectData;
  // Sorted by key, as ClassicUO does (`dict.OrderBy(s => s.Key)`), except the
  // statics list, which is only meaningful in its own order.
  const named = d.rows.filter(([k]) => !k.startsWith("Static "));
  const statics = d.rows.filter(([k]) => k.startsWith("Static "));
  named.sort((a, b) => a[0].localeCompare(b[0]));
  const row = ([k, v]) => `<div class="insp-row"><span class="insp-k">${k}</span>`
    + `<span class="insp-v" title="click to copy">${String(v).replace(/[<&]/g, (c) => c === "<" ? "&lt;" : "&amp;")}</span></div>`;
  host.innerHTML = `<div class="insp-hdr">${d.kind} · ${d.title}</div>`
    + named.map(row).join("") + statics.map(row).join("")
    + `<div class="insp-raw-hdr">scene record</div>`
    + `<pre class="insp-raw">${JSON.stringify(d.raw, null, 1).replace(/[<&]/g, (c) => c === "<" ? "&lt;" : "&amp;")}</pre>`;
  // ClassicUO copies a clicked value to the clipboard; its Dump button writes a
  // file, which a browser tab cannot do unasked — copying the whole dump is the
  // same act with the same result in hand.
  for (const el of host.querySelectorAll(".insp-v"))
    el.addEventListener("click", () => inspectCopy(el.textContent, el));
}
// ClassicUO copies a clicked value with one SDL call. A browser cannot count on
// that: the async Clipboard API needs a permission the page may not have (it
// refuses outright under automation, and headless Chrome has no clipboard at
// all), and `execCommand` needs a trusted gesture. So there are three tiers, and
// the last one always works — if we cannot put the text on the clipboard, we
// select it on the page so ⌘C/Ctrl-C can.
function inspectCopy(text, el) {
  const ok = () => addSysMessage("Copied to clipboard.");
  const selectInstead = () => {
    if (!el) { addSysMessage("Could not copy — no clipboard available."); return; }
    const sel = window.getSelection();
    const range = document.createRange();
    range.selectNodeContents(el);
    sel.removeAllRanges();
    sel.addRange(range);
    addSysMessage("Selected — press ⌘C / Ctrl-C to copy.");
  };
  const viaCommand = () => {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.cssText = "position:fixed;left:-9999px;top:0";
    document.body.appendChild(ta);
    ta.select();
    let done = false;
    try { done = document.execCommand("copy"); } catch (e) {}
    ta.remove();
    if (done) ok(); else selectInstead();
  };
  if (navigator.clipboard && navigator.clipboard.writeText) {
    navigator.clipboard.writeText(text).then(ok, viaCommand);
  } else {
    viaCommand();
  }
}
function inspectDump() {
  if (!inspectData) return;
  const d = inspectData;
  inspectCopy(`${d.kind}: ${d.title}\n`
    + d.rows.map(([k, v]) => `${k} = ${v}`).join("\n")
    + `\n\nscene record:\n${JSON.stringify(d.raw, null, 1)}`);
}
