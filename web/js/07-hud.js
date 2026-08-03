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
  for (const it of s.items || []) isoDot(it.x, it.y, "#e2b340", 1.4);
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

