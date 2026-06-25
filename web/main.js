// anima-client renderer — isometric, real UO sprites, smooth (interpolated) camera.
//
// Tiles/statics live in ABSOLUTE world-iso coordinates in a persistent pool: as
// the player walks we only add/remove the edge tiles entering/leaving the view —
// never a full rebuild. The camera (stage offset) follows the player's *eased*
// position every frame, so movement scrolls smoothly. Entities are redrawn each
// frame at their interpolated positions with walk/idle animation frames.

const HALF = 22, ZSTEP = 4, VIEW = 600;
const STAND = 4, WALK = 0;
let app, world, entLayer, mobs;
let scene = null;

// absolute world iso (no centering); camera does the centering
const isoX = (x, y) => (x - y) * HALF;
const isoY = (x, y, z) => (x + y) * HALF - (z | 0) * ZSTEP;

// ---- texture + frame-count caches ----
const texCache = new Map(), loading = new Set();
function texFor(url) {
  if (texCache.has(url)) return texCache.get(url);
  if (!loading.has(url)) {
    loading.add(url);
    PIXI.Assets.load(url).then((t) => texCache.set(url, t)).catch(() => texCache.set(url, null));
  }
  return null;
}
const frameCount = new Map();
function framesFor(body, group, dir) {
  const k = `${body}/${group}/${dir}`;
  if (frameCount.has(k)) return Math.max(1, frameCount.get(k));
  if (!loading.has("i" + k)) {
    loading.add("i" + k);
    fetch(`animinfo/${body}/${group}/${dir}`).then((r) => r.json())
      .then((j) => frameCount.set(k, j.frames | 0)).catch(() => frameCount.set(k, 0));
  }
  return 5;
}

// ---- persistent world pools ----
const tilePool = new Map();   // "x,y" -> {sp, g, z}
const staticPool = new Map(); // "x,y,g,z" -> sp
// ---- per-entity interp state ----
const anim = new Map();       // id -> {rx,ry,tx,ty,z,dir,body,fallback,moveUntil}

// ---- diagnostics ----
const diag = { fps: 0, poll: 0, sync: 0, tiles: 0, ents: 0, frames: 0, acc: 0, worstFrame: 0 };

async function main() {
  app = new PIXI.Application();
  await app.init({ width: VIEW, height: VIEW, background: 0x05070a, antialias: false });
  document.getElementById("map").appendChild(app.canvas);
  world = new PIXI.Container(); world.sortableChildren = true;
  entLayer = new PIXI.Graphics();
  mobs = new PIXI.Container();
  app.stage.addChild(world, entLayer, mobs);

  poll();
  setInterval(poll, 250);
  app.ticker.add((t) => renderFrame(t.deltaMS));
  setupInput();
}

async function poll() {
  const t0 = performance.now();
  try {
    const r = await fetch("scene.json?" + Date.now());
    if (!r.ok) throw new Error(r.status);
    scene = await r.json();
    updateAnimStates(scene);
    const ts = performance.now();
    syncWorld(scene); // diffs only — no full rebuild
    diag.sync = performance.now() - ts;
    if (scene.player) hud(scene);
    setStatus("live · " + new Date().toLocaleTimeString());
  } catch (e) {
    setStatus("waiting for scene… (" + e + ")");
  }
  diag.poll = performance.now() - t0;
  if (diag.poll > 150) console.warn(`[diag] slow poll ${diag.poll.toFixed(0)}ms`);
}

function updateAnimStates(s) {
  const now = performance.now();
  const seen = new Set();
  const touch = (id, x, y, z, dir, body, fb) => {
    seen.add(id);
    let st = anim.get(id);
    if (!st) { st = { rx: x, ry: y }; anim.set(id, st); }
    if (st.tx !== x || st.ty !== y) st.moveUntil = now + 650;
    Object.assign(st, { tx: x, ty: y, z, dir, body, fallback: fb });
  };
  for (const m of s.mobiles || []) touch("m" + m.serial, m.x, m.y, m.z ?? 0, m.dir ?? 4, m.body, notoColor(m.noto));
  const p = s.player;
  if (p) touch("self", p.x, p.y, p.z ?? 0, p.dir ?? 4, p.body, 0xffffff);
  for (const id of [...anim.keys()]) if (!seen.has(id)) anim.delete(id);
}

// add/remove only the tiles/statics that entered/left the view
function syncWorld(s) {
  const m = s.map || { radius: 14, tiles: [], cx: 0, cy: 0 };
  const span = 2 * m.radius + 1;
  const seenT = new Set(), seenS = new Set();

  for (let row = 0; row < span; row++) {
    for (let col = 0; col < span; col++) {
      const t = m.tiles[row * span + col];
      if (!t || !t.g) continue;
      const x = m.cx + (col - m.radius), y = m.cy + (row - m.radius);
      const key = x + "," + y;
      seenT.add(key);
      const tex = texFor(`art/land/${t.g}.png`);
      let e = tilePool.get(key);
      if (!e) {
        if (!tex) continue; // retry next poll once the texture loads
        const sp = new PIXI.Sprite(tex);
        sp.anchor.set(0.5, 0.5);
        sp.x = isoX(x, y); sp.y = isoY(x, y, t.z); sp.zIndex = (x + y) * 100;
        world.addChild(sp);
        tilePool.set(key, { sp, g: t.g, z: t.z });
      } else if (e.g !== t.g && tex) {
        e.sp.texture = tex; e.g = t.g;
        e.sp.y = isoY(x, y, t.z); e.z = t.z;
      }
    }
  }
  for (const st of s.statics || []) {
    const key = `${st.x},${st.y},${st.g},${st.z}`;
    seenS.add(key);
    if (staticPool.has(key)) continue;
    const tex = texFor(`art/static/${st.g}.png`);
    if (!tex) continue;
    const sp = new PIXI.Sprite(tex);
    sp.anchor.set(0.5, 1.0);
    sp.x = isoX(st.x, st.y); sp.y = isoY(st.x, st.y, st.z) + HALF;
    sp.zIndex = (st.x + st.y) * 100 + 50;
    world.addChild(sp);
    staticPool.set(key, sp);
  }

  prune(tilePool, seenT, (e) => e.sp);
  prune(staticPool, seenS, (e) => e);
  diag.tiles = tilePool.size + staticPool.size;
}
function prune(pool, seen, getSp) {
  for (const [key, e] of pool) {
    if (!seen.has(key)) { world.removeChild(getSp(e)); getSp(e).destroy(); pool.delete(key); }
  }
}

function renderFrame(dt) {
  if (!scene) return;
  const k = 1 - Math.exp(-dt / 80); // interpolation factor
  for (const st of anim.values()) {
    st.rx += (st.tx - st.rx) * k;
    st.ry += (st.ty - st.ry) * k;
  }
  // camera follows the eased player so the avatar stays centered
  const self = anim.get("self");
  if (self) {
    app.stage.position.set(VIEW / 2 - isoX(self.rx, self.ry), VIEW / 2 - isoY(self.rx, self.ry, self.z));
  }
  drawMobs();

  // fps / worst-frame
  diag.frames++; diag.acc += dt; diag.worstFrame = Math.max(diag.worstFrame, dt);
  if (dt > 70) console.warn(`[diag] slow frame ${dt.toFixed(0)}ms`);
  if (diag.acc >= 500) {
    diag.fps = Math.round((1000 * diag.frames) / diag.acc);
    diag.frames = 0; diag.acc = 0;
    updateDiag();
  }
}

function drawMobs() {
  const now = performance.now();
  mobs.removeChildren();
  entLayer.clear();
  diag.ents = 0;

  for (const it of scene.items || []) {
    entLayer.rect(isoX(it.x, it.y) - 2, isoY(it.x, it.y, scene.player ? scene.player.z : 0) - 2, 4, 4).fill(0xe2b340);
  }
  for (const [, st] of anim) {
    diag.ents++;
    const moving = now < st.moveUntil;
    const d = st.dir & 7;
    const group = moving ? WALK : STAND;
    const frame = moving ? Math.floor(now / 100) % framesFor(st.body, WALK, d) : 0;
    const tex = st.body ? texFor(`anim/${st.body}/${group}/${d}/${frame}.png`) : null;
    const x = isoX(st.rx, st.ry), y = isoY(st.rx, st.ry, st.z);
    if (tex) {
      const sp = new PIXI.Sprite(tex);
      sp.anchor.set(0.5, 1.0); sp.x = x; sp.y = y + HALF;
      mobs.addChild(sp);
    } else {
      entLayer.circle(x, y - 6, 5).fill(st.fallback);
    }
  }
}

function notoColor(n) { return { 1: 0x4f8cf7, 2: 0x46a758, 3: 0x9aa0a6, 5: 0xd9a441, 6: 0xe5484d }[n] || 0xd6dae0; }

function hud(s) {
  const p = s.player;
  set("pname", p.name || "(unnamed)"); set("ppos", `(${p.x}, ${p.y}, ${p.z})`);
  bar("hp", p.hits, p.hitsMax); bar("mana", p.mana, p.manaMax); bar("stam", p.stam, p.stamMax);
  set("stats", `${p.str} / ${p.dex} / ${p.int}`); set("gold", p.gold);
  const j = document.getElementById("journal"); j.innerHTML = "";
  for (const line of s.journal || []) {
    const d = document.createElement("div"); d.textContent = `${line.name}: ${line.text}`; j.appendChild(d);
  }
}
function updateDiag() {
  set("diag", `fps ${diag.fps} · poll ${diag.poll.toFixed(0)}ms · sync ${diag.sync.toFixed(1)}ms · sprites ${diag.tiles} · ents ${diag.ents} · worst ${diag.worstFrame.toFixed(0)}ms`);
  diag.worstFrame = 0;
}
function bar(id, c, m) { document.getElementById(id).style.width = (m > 0 ? Math.round((c / m) * 100) : 0) + "%"; }
function set(id, v) { const el = document.getElementById(id); if (el) el.textContent = v; }
function setStatus(t) { set("status", t); }

// ---- input ----
const KEY_DIR = { ArrowUp: 0, KeyW: 0, ArrowRight: 2, KeyD: 2, ArrowDown: 4, KeyS: 4, ArrowLeft: 6, KeyA: 6, KeyE: 1, KeyC: 3, KeyZ: 5, KeyQ: 7 };
const held = new Set();
let chatting = false;
function setupInput() {
  window.addEventListener("keydown", (e) => {
    if (chatting) return;
    if (e.code === "KeyT" || e.code === "Enter") { e.preventDefault(); openChat(); return; }
    if (e.code in KEY_DIR) { held.add(KEY_DIR[e.code]); e.preventDefault(); }
  });
  window.addEventListener("keyup", (e) => { if (e.code in KEY_DIR) held.delete(KEY_DIR[e.code]); });
  setInterval(() => { if (!chatting && held.size) sendInput(`walk:${[...held].pop()}:0`); }, 200);
}
function openChat() { chatting = true; const t = window.prompt("Say:"); chatting = false; if (t && t.trim()) sendInput("say:" + t.trim()); }
function sendInput(cmd) { fetch("/input", { method: "POST", body: cmd }).catch(() => {}); }

main();
