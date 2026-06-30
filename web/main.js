// anima-client renderer — isometric, real UO sprites, smooth (interpolated) camera.
//
// Tiles/statics live in ABSOLUTE world-iso coordinates in a persistent pool: as
// the player walks we only add/remove the edge tiles entering/leaving the view —
// never a full rebuild. The camera (stage offset) follows the player's *eased*
// position every frame, so movement scrolls smoothly. Entities are redrawn each
// frame at their interpolated positions with walk/idle animation frames.

const HALF = 22, ZSTEP = 4;
// ClassicUO people animation groups
const WALK = 0, RUN_UNARMED = 2, STAND = 4;
// War-mode idle stance: PAG_STAND_ONEHANDED_ATTACK (the combat-ready pose a person
// holds while standing in war mode). ClassicUO swaps the plain Stand (4) for this.
const PEOPLE_COMBAT_STAND = 7;
const ONMOUNT_WALK = 23, ONMOUNT_RUN = 24, ONMOUNT_STAND = 25;
const CHAR_ANIM_DELAY = 80; // ClassicUO Constants.CHARACTER_ANIMATION_DELAY (ms/frame)
// Animation GROUP NUMBERS differ by body type (ClassicUO): monster (body<200)
// Walk=0/Stand=1, animal (200..399) Walk=0/Run=1/Stand=2, people (>=400)
// Walk=0/Run=2/Stand=4. Using the people stand (4) for an animal showed an
// attack pose (the "cat → alligator when idle" bug).
function animGroup(moving, running, mounted, body, war) {
  if (mounted) return moving ? (running ? ONMOUNT_RUN : ONMOUNT_WALK) : ONMOUNT_STAND;
  const t = body < 200 ? 0 : body < 400 ? 1 : 2;
  if (t === 0) return moving ? 0 : 1;                  // monster: no separate run
  if (t === 1) return moving ? (running ? 1 : 0) : 2;  // animal
  // people: standing in war mode → combat-ready stance instead of the idle stand.
  if (!moving) return war ? PEOPLE_COMBAT_STAND : STAND;
  return running ? RUN_UNARMED : WALK;
}

// People animation groups (ClassicUO PeopleAnimationGroup): 16 = CastDirected,
// 17 = CastArea. UO spell casts are sent (0x6E) with the SpellInfo.Action code,
// not a direct group — those live in the ~200+ range and must fold onto the cast
// gesture. Map a 0x6E `action` to the body's real animation group.
const PEOPLE_CAST_DIRECTED = 16;
function resolveActionGroup(action, body) {
  action = action | 0;
  if (body >= 400) {                 // people / humanoid
    if (action >= 200) return PEOPLE_CAST_DIRECTED; // spell cast gesture
    if (action >= 35) return action % 35;           // other out-of-range → fold in
    return action;                                  // direct people group (combat swings, etc.)
  }
  return action;                     // monsters/animals: action indexes their own group set
}
let app, world, entLayer, mobs, overLayer, barLayer, itemLayer;
let scene = null;
// Render-on-demand: only re-draw the canvas when the scene changed. markDirty()
// requests one redraw; movement/animation/polls set it. Starts true (first draw).
let dirty = true;
const markDirty = () => { dirty = true; };

// ---- overhead speech (ClassicUO-style floating text above the speaker) ----
const overheads = [];          // { id, text, color, born, ttl, sprite }
let lastJournalSeq = 0;

// ---- floating combat damage numbers (0x0B) ----
const damageFloaters = [];     // { id, sprite, born, ttl }
let lastDamageSeq = 0;         // highest damage event seq we've already floated
let lastAnimSeq = 0;           // highest character-animation (0x6E) event we've played
// Map a 0x6E `action` to the animation group for this body. For people (>=400) the
// action IS the people group (9=1H attack, 12-14=2H, 18/19=bow, 20=get-hit, …); for
// monsters/animals the action indexes their own group set — pass it through either way.
const CHAR_ANIM_FRAME_MS = 110; // per-frame time of a one-shot action (≈ClassicUO)
const DAMAGE_TTL = 1000;       // ms a number lives (rises + fades over this)
const DAMAGE_RISE = 20;        // px it rises over its life

// ---- graphical effects (0x70/0xC0/0xC7): spell bolts, hit sparkles, explosions,
// fields. Each scene effect spawns a short-lived animated sprite on overLayer
// (above the world; no world re-sort). Frames + interval are baked server-side
// from animdata.mul; we just cycle them, tint by hue, and position per kind. ----
const fxEffects = [];          // { seq, kind, frames[], fm, born, totalMs, sprite, hue, anchors… }
let lastEffectSeq = 0;         // highest effect event seq we've already spawned

// ---- user settings (persisted to localStorage; edited via the Options panel) ----
const SETTINGS_KEY = "anima.settings";
const SETTINGS_DEFAULTS = {
  sfx: true, sfxVol: 0.4,        // sound effects on/off + volume
  music: true, musicVol: 0.3,    // background music on/off + volume
  tooltips: true,                // OPL hover tooltips
  bars: true,                    // overhead HP bars
  damage: true,                  // floating damage numbers
  names: true,                   // overhead name labels
  abilities: true,               // weapon special-ability bar (also needs server AOS)
};
let settings = Object.assign({}, SETTINGS_DEFAULTS);
try { Object.assign(settings, JSON.parse(localStorage.getItem(SETTINGS_KEY) || "{}")); } catch (e) {}
function saveSettings() { try { localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings)); } catch (e) {} }
// Build the Options panel body from the current settings (checkboxes + sliders).
function renderOptions() {
  const body = document.getElementById("opt-body");
  if (!body) return;
  const cb = (key, label) => `<div class="opt-row"><label for="opt-${key}">${label}</label>`
    + `<input type="checkbox" id="opt-${key}" data-k="${key}"${settings[key] ? " checked" : ""}></div>`;
  const sl = (key, label) => `<div class="opt-row"><label for="opt-${key}">${label}</label>`
    + `<input type="range" id="opt-${key}" data-k="${key}" min="0" max="100" value="${Math.round(settings[key] * 100)}">`
    + `<span class="opt-val" id="optv-${key}">${Math.round(settings[key] * 100)}</span></div>`;
  body.innerHTML =
    '<div class="opt-sect">Audio</div>'
    + cb("sfx", "Sound effects") + sl("sfxVol", "SFX volume")
    + cb("music", "Music") + sl("musicVol", "Music volume")
    + '<div class="opt-sect">Display</div>'
    + cb("tooltips", "Item tooltips (OPL)")
    + cb("names", "Overhead names")
    + cb("bars", "HP bars")
    + cb("damage", "Damage numbers")
    + cb("abilities", "Weapon abilities");
}
// Show/hide the Options panel (force=true open, false close, omitted = toggle).
function toggleOptions(force) {
  const el = document.getElementById("options");
  if (!el) return;
  const on = force != null ? force : !el.classList.contains("on");
  if (on) { renderOptions(); el.classList.add("on"); } else el.classList.remove("on");
}

// ---- audio: sound effects (0x54) + background music (0x6D) ----
// Browsers block autoplay until the first user gesture, so sounds/music only
// start after the player first clicks or presses a key — that's expected.
let audioMuted = false;          // global master mute (N key / on-screen button)
let lastSoundSeq = 0;            // highest sound event seq we've already played
let curMusicId = null;           // music id currently loaded into bgMusic
const MAX_CONCURRENT_SFX = 8;
const bgMusic = new Audio();     // single looping background track (HTMLAudio is fine for a long loop)
bgMusic.loop = true;
bgMusic.volume = settings.musicVol;

// SFX use the Web Audio API instead of a fresh `new Audio(url)` per hit. The old
// path re-fetched AND re-decoded the WAV every single time before it could start —
// the main cause of "late" sound. Here each id is decoded ONCE into an AudioBuffer,
// cached, and replayed through a throwaway BufferSource → near-zero latency on any
// repeat (and the network/decode only ever happens on a sound's very first play).
let audioCtx = null, sfxGain = null;
const sfxBuffers = new Map();    // id -> AudioBuffer (ready) | Promise (in-flight)
const activeSfx = new Set();     // live BufferSource nodes (concurrency cap + mute-stop)
function ensureAudioCtx() {
  if (audioCtx) return audioCtx;
  try {
    audioCtx = new (window.AudioContext || window.webkitAudioContext)();
    sfxGain = audioCtx.createGain();
    sfxGain.gain.value = settings.sfxVol;
    sfxGain.connect(audioCtx.destination);
  } catch (_) { audioCtx = null; }
  return audioCtx;
}
// Browsers start the context "suspended" until a user gesture — resume it (and kick
// pending music) on the first click/keypress. Idempotent, so it can fire repeatedly.
function unlockAudio() {
  const ctx = ensureAudioCtx();
  if (ctx && ctx.state === "suspended") ctx.resume().catch(() => {});
  if (!audioMuted && settings.music && curMusicId != null) bgMusic.play().catch(() => {});
}
window.addEventListener("pointerdown", unlockAudio);
window.addEventListener("keydown", unlockAudio);
// Fetch + decode a sound id once; cache the AudioBuffer. Returns Promise<AudioBuffer|null>.
function loadSfx(id) {
  const c = sfxBuffers.get(id);
  if (c instanceof AudioBuffer) return Promise.resolve(c);
  if (c) return c;                       // decode already in flight
  const ctx = ensureAudioCtx();
  if (!ctx) return Promise.resolve(null);
  const p = fetch("sound/" + id + ".wav")
    .then((r) => r.arrayBuffer())
    .then((buf) => ctx.decodeAudioData(buf))
    .then((b) => { sfxBuffers.set(id, b); return b; })
    .catch(() => { sfxBuffers.delete(id); return null; });
  sfxBuffers.set(id, p);
  return p;
}
// Max tile distance a sound carries; at/over this it's silent (ClassicUO-like).
const SFX_MAX_DIST = 22;
const SFX_PAN_RANGE = 16;   // iso screen-x spread that maps to a hard L/R pan
// Play a decoded buffer, attenuated + panned by the sound's world position (x,y).
// A sound at (0,0) or with no player is treated as non-positional (center, full).
function playBuffer(b, x, y) {
  if (!audioCtx || !b || activeSfx.size >= MAX_CONCURRENT_SFX) return;
  const p = scene && scene.player;
  let vol = 1, pan = 0;
  const positional = !!(p && (x || y));
  if (positional) {
    const dx = (x | 0) - (p.x | 0), dy = (y | 0) - (p.y | 0);
    const dist = Math.max(Math.abs(dx), Math.abs(dy));       // chebyshev tiles
    vol = 1 - dist / SFX_MAX_DIST;
    if (vol <= 0.02) return;                                  // out of earshot
    vol = vol * vol;                                          // perceptual falloff
    // iso screen-x ∝ (dx − dy): left of the avatar pans left, right pans right.
    pan = Math.max(-1, Math.min(1, (dx - dy) / SFX_PAN_RANGE));
  }
  const src = audioCtx.createBufferSource();
  src.buffer = b;
  let out = src;
  if (positional && audioCtx.createStereoPanner) {
    const pn = audioCtx.createStereoPanner(); pn.pan.value = pan;
    out.connect(pn); out = pn;
  }
  if (positional) {
    const g = audioCtx.createGain(); g.gain.value = vol;
    out.connect(g); out = g;
  }
  out.connect(sfxGain);
  activeSfx.add(src);
  src.onended = () => activeSfx.delete(src);
  try { src.start(); } catch (_) { activeSfx.delete(src); }
}
function playSfx(id, x, y) {
  const ctx = ensureAudioCtx();
  if (!ctx) return;
  if (ctx.state === "suspended") ctx.resume().catch(() => {});
  const c = sfxBuffers.get(id);
  if (c instanceof AudioBuffer) { playBuffer(c, x, y); return; }  // cached → instant
  loadSfx(id).then((b) => { if (b) playBuffer(b, x, y); });        // first time → decode then play
}
// Apply the current audio settings to the live audio nodes/elements.
function applyAudioSettings() {
  bgMusic.volume = settings.musicVol;
  if (sfxGain) sfxGain.gain.value = settings.sfxVol;
  if (audioMuted || !settings.music) { bgMusic.pause(); }
  else if (curMusicId != null) { bgMusic.play().catch(() => {}); }
}

// Play any sound events newer than the last we played (mirrors journal_seq).
function playSounds(s) {
  if (!s || !s.sounds) return;
  for (const ev of s.sounds) {
    const seq = ev.seq | 0;
    if (seq <= lastSoundSeq) continue;
    lastSoundSeq = seq;
    if (audioMuted || !settings.sfx) continue;
    playSfx(ev.id | 0, ev.x | 0, ev.y | 0);
  }
}
// Sound push channel: the server streams each sound the instant it fires (SSE) so a
// hit no longer waits for the next 150ms poll. EventSource auto-reconnects; the
// poll's playSounds() covers any frame missed during a reconnect. Both dedupe on
// `lastSoundSeq`, so whichever delivers a seq first wins and the other skips it.
function connectSoundStream() {
  if (typeof EventSource === "undefined") return; // no SSE → poll fallback handles sound
  const es = new EventSource("sounds");
  es.onmessage = (e) => {
    let ev; try { ev = JSON.parse(e.data); } catch (_) { return; }
    const seq = ev.seq | 0;
    if (seq <= lastSoundSeq) return;
    lastSoundSeq = seq;
    if (audioMuted || !settings.sfx) return;
    playSfx(ev.id | 0, ev.x | 0, ev.y | 0);
  };
}

// Sync the looping background track to scene.music (id or null = stop).
function updateMusic(s) {
  const id = s && s.music != null ? (s.music | 0) : null;
  if (id !== curMusicId) {
    curMusicId = id;
    if (id == null) {
      bgMusic.pause();
      bgMusic.removeAttribute("src");
    } else {
      bgMusic.src = "music/" + id + ".mp3";
      if (!audioMuted && settings.music) bgMusic.play().catch(() => {});
    }
  }
}

function toggleMute() {
  audioMuted = !audioMuted;
  if (audioMuted) {
    for (const src of activeSfx) { try { src.stop(); } catch (_) {} }
    activeSfx.clear();
  }
  applyAudioSettings();   // pause/resume music respecting both mute + settings.music
  const btn = document.getElementById("mutebtn");
  if (btn) btn.textContent = audioMuted ? "muted" : "sound";
  setStatus(audioMuted ? "audio muted" : "audio on");
}
// UO message types (ClassicUO MessageType.cs): 0 Regular, 1 System, 2 Emote,
// 6 Label, 7 Focus, 8 Whisper, 9 Yell, 10 Spell (power words). ClassicUO colors
// overhead text by the server-sent HUE; these per-type colors are only the
// fallback used when the server sends hue 0. See msgColor()/MSG_CLASS.
const MSG_DEFAULT_COLOR = {
  0: 0xffffff,  // regular speech — white
  2: 0xffd27f,  // emote — soft amber
  6: 0xc8c8c8,  // label
  7: 0xffffff,  // focus
  8: 0x9aa0a6,  // whisper — dim gray
  9: 0xff5a4d,  // yell — red
  10: 0xb9a7ff, // spell / power words — soft violet
};
// Per-type font styling (weight/size/style), applied as an extra CSS class so
// yells read loud, whispers quiet, emotes italic and power words distinct.
const MSG_CLASS = { 2: "oh-emote", 8: "oh-whisper", 9: "oh-yell", 10: "oh-spell" };
// Overhead text color: ClassicUO-style — the server hue wins (resolved through the
// hue table); fall back to the per-type default when the server sent hue 0.
function msgColor(type, hue) {
  if (hue) { const hx = hueHex(hue); if (hx) return hx; }
  return cssColor(MSG_DEFAULT_COLOR[type] ?? 0xffffff);
}

// absolute world iso (no centering); camera does the centering
const isoX = (x, y) => (x - y) * HALF;
const isoY = (x, y, z) => (x + y) * HALF - (z | 0) * ZSTEP;
// Iso draw order (ClassicUO Chunk.AddGameObject): primary = (x+y) screen depth,
// secondary = priorityZ (z adjusted: land z-2, wall/height +1), tertiary = type
// bias (land 0 < surface/static 4 < mobile 8) so floors draw under walls etc.
const depthZ = (x, y, pz, bias) => (x + y) * 8192 + ((pz | 0) + 130) * 16 + bias;

// ---- texture + frame-count caches ----
const texCache = new Map(), loading = new Set();
function texFor(url) {
  if (texCache.has(url)) return texCache.get(url);
  if (!loading.has(url)) {
    loading.add(url);
    // markDirty() in the .then so a body/clothing frame that streams in gets
    // painted even while the character stands still (render-on-demand otherwise
    // wouldn't repaint an idle scene when a late texture arrives).
    PIXI.Assets.load(url).then((t) => { texCache.set(url, t); markDirty(); }).catch(() => texCache.set(url, null));
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
      .then((j) => { frameCount.set(k, j.frames | 0); frameCtr.set(k, j.c || []); })
      .catch(() => frameCount.set(k, 0));
  }
  return 5;
}
// Per-frame draw-center [cx, cy] (from animinfo). The renderer positions a part's
// sprite at (screenX - cx, screenY - height - cy) — ClassicUO's draw math — so the
// body, worn equipment, held items and a rider on a mount all align instead of
// being foot-anchored at the same point. null until the animinfo load lands.
const frameCtr = new Map(); // "body/group/dir" -> [[cx,cy],...]
function centerFor(body, group, dir, frame) {
  const c = frameCtr.get(`${body}/${group}/${dir}`);
  return c && c[frame] ? c[frame] : null;
}

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
      .then((j) => { hueHexCache.set(id, j.rgb); renderEquipTip(); applyHueSwatches(); }).catch(() => {});
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
const alphaCache = new Map(); // img.src → ImageData
function imgAlpha(img, x, y) {
  const w = img.naturalWidth, hh = img.naturalHeight;
  if (!w || !img.complete || x < 0 || y < 0 || x >= w || y >= hh) return 0;
  let data = alphaCache.get(img.src);
  if (!data) {
    _ac.width = w; _ac.height = hh; _actx.clearRect(0, 0, w, hh);
    try { _actx.drawImage(img, 0, 0); data = _actx.getImageData(0, 0, w, hh); }
    catch { return 0; }
    alphaCache.set(img.src, data);
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
  addOverhead("self", "⚠ Snooping is a crime — you may be flagged criminal!", 0xff5555, performance.now());
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

async function main() {
  app = new PIXI.Application();
  const rs = renderSize();
  // autoStart:false → no automatic per-frame render. We render ON DEMAND (only when
  // something visibly changed) and cap the redraw rate — see the loop below.
  await app.init({ width: rs.w, height: rs.h, background: 0x05070a, antialias: false, resolution: 1, autoDensity: false, autoStart: false });
  app.canvas.style.width = "100%";
  app.canvas.style.height = "100%";
  app.canvas.style.imageRendering = "pixelated"; // crisp nearest-neighbor upscale
  window.addEventListener("resize", () => { const r = renderSize(); app.renderer.resize(r.w, r.h); markDirty(); });
  document.getElementById("map").appendChild(app.canvas);
  world = new PIXI.Container(); world.sortableChildren = true;
  entLayer = new PIXI.Graphics();
  mobs = new PIXI.Container();
  overLayer = new PIXI.Container(); // floating speech, always on top of the world
  // Names + HP bars (drawn over the world, non-interactive so they never eat clicks).
  barLayer = new PIXI.Container(); barLayer.eventMode = "none";
  // Invisible per-item click targets — kept BELOW `world` so a mobile sharing a
  // tile with an item wins the hit-test (mobiles are the priority).
  itemLayer = new PIXI.Container();
  app.stage.addChild(itemLayer, world, entLayer, mobs, barLayer, overLayer);

  poll();
  setInterval(poll, 150);
  connectSoundStream(); // SSE: play sounds the instant they fire (no poll wait)
  setInterval(tickBuffTimers, 1000); // count down the buff-bar timers once a second
  // Render-on-demand loop: renderFrame() advances the prediction/glide every
  // animation frame (so motion stays smooth), but app.render() — the expensive GPU
  // pass — only runs when the scene actually changed (movement, animation, a poll
  // update) and at most ~RENDER_MS apart. Standing still ⇒ ~0 GPU work; walking ⇒
  // a capped redraw rate. This is what kills the "600% CPU" (it was re-drawing the
  // whole world 60×/s unconditionally).
  let lastT = performance.now(), lastDraw = 0;
  const RENDER_MS = 22; // ~45fps redraw ceiling
  function frame(now) {
    renderFrame(now - lastT); lastT = now;
    if (dirty && now - lastDraw >= RENDER_MS) { app.render(); dirty = false; lastDraw = now; }
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);
  setupInput();
  setupItemDnD();
  initFx();
}

// ---- Day/night tint + weather (separate 2D-canvas rAF loop) ----------------
// Drawn on its own <canvas id="fx"> over the game world but UNDER all HUD/UI.
// It runs an INDEPENDENT requestAnimationFrame loop and never calls app.render()
// or markDirty(), so the expensive PIXI world stays render-on-demand even while
// rain/snow animate continuously. Reads only the latest polled `scene`.
let fxCanvas = null, fxCtx = null, fxParticles = [], fxTint = 0, fxKind = -1, fxLastT = 0;
const FX_MAX_PARTICLES = 60; // hard cap regardless of weatherN
const FX_MAX_NIGHT = 0.6;    // tint opacity at the darkest light level

function initFx() {
  fxCanvas = document.getElementById("fx");
  if (!fxCanvas) return;
  fxCtx = fxCanvas.getContext("2d");
  const resize = () => { fxCanvas.width = window.innerWidth; fxCanvas.height = window.innerHeight; };
  resize();
  window.addEventListener("resize", resize);
  requestAnimationFrame(fxFrame);
}

function fxSpawn(W, H, snow) {
  if (snow) {
    return { x: Math.random() * W, y: Math.random() * H,
             vx: (Math.random() - 0.5) * 0.6, vy: 0.6 + Math.random() * 1.3,
             r: 1 + Math.random() * 1.8, phase: Math.random() * 100 };
  }
  // rain: fast, near-vertical light-blue streaks (slight lean)
  return { x: Math.random() * W, y: Math.random() * H,
           vx: -1.0 - Math.random(), vy: 9 + Math.random() * 6, r: 0, phase: 0 };
}

function fxFrame(now) {
  const dt = fxLastT ? Math.min(50, now - fxLastT) : 16;
  fxLastT = now;
  const ctx = fxCtx;
  if (!ctx) { requestAnimationFrame(fxFrame); return; }
  const W = fxCanvas.width, H = fxCanvas.height;
  ctx.clearRect(0, 0, W, H);

  // --- night tint: light 0 = full day (no tint) → ~0x1F darkest ---
  const light = scene ? (scene.light || 0) : 0;
  const target = Math.min(FX_MAX_NIGHT, (light / 0x1F) * FX_MAX_NIGHT);
  fxTint += (target - fxTint) * Math.min(1, dt / 400); // smooth dawn/dusk glide
  if (fxTint > 0.003) {
    ctx.fillStyle = "rgba(8, 14, 40, " + fxTint.toFixed(3) + ")";
    ctx.fillRect(0, 0, W, H);
    // Per-object light sources: once it's dark enough to notice, erase soft holes
    // in the night overlay at each light (torches/lamps + the player), so the
    // bright game world shows through as a glow. destination-out subtracts the
    // gradient's alpha from the darkness; a radial falloff makes a soft circle.
    if (fxTint > 0.05 && scene && scene.lights && scene.lights.length && app && app.renderer) {
      const cssX = app.canvas.clientWidth / app.renderer.width;   // renderer→CSS px (x)
      const cssY = app.canvas.clientHeight / app.renderer.height; // renderer→CSS px (y)
      const ox = app.stage.position.x, oy = app.stage.position.y;
      const center = 0.9 * fxTint; // erase strength at the glow's core
      ctx.save();
      ctx.globalCompositeOperation = "destination-out";
      for (const L of scene.lights) {
        const sx = (ox + isoX(L.x, L.y)) * cssX;
        const sy = (oy + isoY(L.x, L.y, L.z)) * cssY;
        const rad = (L.r || 3) * 44 * cssX;
        if (sx < -rad || sy < -rad || sx > W + rad || sy > H + rad) continue; // off-screen
        const g = ctx.createRadialGradient(sx, sy, 0, sx, sy, rad);
        g.addColorStop(0, "rgba(0,0,0," + center.toFixed(3) + ")");
        g.addColorStop(1, "rgba(0,0,0,0)");
        ctx.fillStyle = g;
        ctx.beginPath();
        ctx.arc(sx, sy, rad, 0, 6.283);
        ctx.fill();
      }
      ctx.restore();
    }
  }

  // --- season tint (0xBC): a very faint color wash so the world feels seasonal.
  // Fall = warm amber, Winter = cool blue, Desolation = desaturated grey. Spring(0)
  // and Summer(1) get no wash. We do NOT remap tree/foliage graphics (much larger). ---
  const seasonWash = { 2: "rgba(150,90,20,0.07)", 3: "rgba(120,150,200,0.07)", 4: "rgba(90,90,90,0.10)" };
  const sw = scene && seasonWash[scene.season];
  if (sw) { ctx.fillStyle = sw; ctx.fillRect(0, 0, W, H); }

  // --- weather: only rain(0) and snow(2) are animated; anything else clears ---
  const kind = scene ? scene.weather : 0xFF;
  const snow = kind === 2, rain = kind === 0;
  const wantN = (rain || snow) ? Math.min(FX_MAX_PARTICLES, scene.weatherN || 0) : 0;
  if (kind !== fxKind) { fxParticles.length = 0; fxKind = kind; } // re-seed on type change
  while (fxParticles.length < wantN) fxParticles.push(fxSpawn(W, H, snow));
  if (fxParticles.length > wantN) fxParticles.length = wantN;

  if (wantN > 0) {
    const f = dt / 16; // normalize step to ~60fps
    if (rain) {
      ctx.strokeStyle = "rgba(170, 200, 235, 0.5)";
      ctx.lineWidth = 1;
      ctx.beginPath();
      for (const p of fxParticles) {
        p.x += p.vx * f; p.y += p.vy * f;
        if (p.y > H) { p.y = -10; p.x = Math.random() * W; }
        if (p.x < 0) p.x += W; else if (p.x > W) p.x -= W;
        ctx.moveTo(p.x, p.y);
        ctx.lineTo(p.x - p.vx * 1.6, p.y - p.vy * 1.6);
      }
      ctx.stroke();
    } else {
      ctx.fillStyle = "rgba(255, 255, 255, 0.85)";
      for (const p of fxParticles) {
        p.x += (p.vx + Math.sin((p.y + p.phase) * 0.02) * 0.5) * f;
        p.y += p.vy * f;
        if (p.y > H) { p.y = -6; p.x = Math.random() * W; }
        if (p.x < 0) p.x += W; else if (p.x > W) p.x -= W;
        ctx.beginPath();
        ctx.arc(p.x, p.y, p.r, 0, 6.283);
        ctx.fill();
      }
    }
  }

  // --- quest arrow (0xBA): point from screen center toward the target tile.
  // On-screen → the arrow sits on the tile; off-screen → it clamps to the screen
  // edge in that direction so it always tells you which way to go. ---
  const qa = scene && scene.questArrow;
  if (qa && app && app.renderer) {
    const cssX = app.canvas.clientWidth / app.renderer.width;
    const cssY = app.canvas.clientHeight / app.renderer.height;
    const ox = app.stage.position.x, oy = app.stage.position.y;
    const tx = (ox + isoX(qa.x, qa.y)) * cssX;
    const ty = (oy + isoY(qa.x, qa.y, 0)) * cssY;
    const cx = W / 2, cy = H / 2;
    const margin = 44;
    let ax = tx, ay = ty;
    const onScreen = tx >= 0 && tx <= W && ty >= 0 && ty <= H;
    if (!onScreen) {
      // Clamp along the center→target ray to the screen rect (inset by margin).
      const dx = tx - cx, dy = ty - cy;
      let t = Infinity;
      if (dx > 0) t = Math.min(t, (W - margin - cx) / dx);
      else if (dx < 0) t = Math.min(t, (margin - cx) / dx);
      if (dy > 0) t = Math.min(t, (H - margin - cy) / dy);
      else if (dy < 0) t = Math.min(t, (margin - cy) / dy);
      if (!isFinite(t) || t < 0) t = 0;
      ax = cx + dx * t; ay = cy + dy * t;
    }
    const ang = Math.atan2(ty - cy, tx - cx);
    drawQuestArrow(ctx, ax, ay, ang, now);
  }

  requestAnimationFrame(fxFrame);
}

// Draw a glowing amber quest arrow at (x, y) pointing along `ang` (radians). A
// gentle pulse keeps it noticeable without being distracting.
function drawQuestArrow(ctx, x, y, ang, now) {
  const pulse = 1 + 0.12 * Math.sin(now * 0.006);
  ctx.save();
  ctx.translate(x, y);
  ctx.rotate(ang);
  ctx.scale(pulse, pulse);
  ctx.shadowColor = "rgba(255, 200, 90, 0.9)";
  ctx.shadowBlur = 12;
  ctx.fillStyle = "#ffcf5a";
  ctx.strokeStyle = "rgba(80, 50, 0, 0.9)";
  ctx.lineWidth = 1.5;
  // Arrow: a shaft + a head, drawn pointing toward +x (the rotation aims it).
  ctx.beginPath();
  ctx.moveTo(18, 0);    // tip
  ctx.lineTo(2, -11);
  ctx.lineTo(2, -4);
  ctx.lineTo(-16, -4);
  ctx.lineTo(-16, 4);
  ctx.lineTo(2, 4);
  ctx.lineTo(2, 11);
  ctx.closePath();
  ctx.fill();
  ctx.stroke();
  ctx.restore();
}

// ---- login page (shown when the play-server is in ANIMA_LOGIN mode and not yet
// in world; scene carries {auth:"login"|"connecting"|"error", msg}) ----
let loginWired = false;
function wireLogin() {
  if (loginWired) return; loginWired = true;
  const go = document.getElementById("lg-go");
  const submit = () => {
    const host = (document.getElementById("lg-host").value || "127.0.0.1").trim();
    const port = (document.getElementById("lg-port").value || "2593").trim();
    const user = (document.getElementById("lg-user").value || "").trim();
    const pass = document.getElementById("lg-pass").value || "";
    if (!user) { document.getElementById("lg-msg").textContent = "Enter an account name."; return; }
    document.getElementById("lg-msg").textContent = "Connecting…";
    fetch("login", { method: "POST", body: `${host}:${port}:${user}:${pass}` }).catch(() => {});
  };
  go.addEventListener("click", submit);
  for (const id of ["lg-host", "lg-port", "lg-user", "lg-pass"]) {
    document.getElementById(id).addEventListener("keydown", (e) => { if (e.code === "Enter") submit(); });
  }
}
function showLogin(auth, msg) {
  wireLogin();
  const el = document.getElementById("login");
  if (el) el.classList.add("on");
  const m = document.getElementById("lg-msg");
  const go = document.getElementById("lg-go");
  if (auth === "connecting") { if (m) m.textContent = "Connecting…"; if (go) go.disabled = true; }
  else if (auth === "error") { if (m) m.textContent = "Login failed: " + (msg || "unknown error"); if (go) go.disabled = false; }
  else { if (go) go.disabled = false; }
}
function hideLogin() {
  const el = document.getElementById("login");
  if (el && el.classList.contains("on")) el.classList.remove("on");
}

async function poll() {
  const t0 = performance.now();
  try {
    const r = await fetch("scene.json?" + Date.now());
    if (!r.ok) throw new Error(r.status);
    scene = await r.json();
    // Not in world yet (login-page mode): show the login form instead of rendering.
    if (scene && scene.auth) { showLogin(scene.auth, scene.msg); return; }
    hideLogin();
    updateAnimStates(scene);
    const ts = performance.now();
    syncWorld(scene); // diffs only — no full rebuild
    diag.sync = performance.now() - ts;
    markDirty(); // a fresh poll may change tiles/entities → redraw once
    ingestSpeech(scene); // float new speech above its speaker
    ingestAnims(scene); // play new character animations (0x6E: combat swings, bows…)
    ingestDamage(scene); // float new combat damage numbers (0x0B)
    ingestEffects(scene); // spawn new graphical effects (0x70/0xC0/0xC7)
    refreshTip(); // update the hover tooltip if its OPL just arrived/changed
    drawMinimap(scene);
    refreshBuffs(scene); // reconcile the buff/debuff bar with scene.buffs
    refreshAbilities(); // keep the weapon special-ability bar in sync with the equipped weapon
    if (wmOn) drawWorldmap();  // keep the open world map tracking the player
    if (scene.player) hud(scene);
    refreshPaperdoll();   // keep the paperdoll live (equip/stats change)
    if (spellbookOn) refreshSpellMana(); // keep the spellbook's mana readout live
    if (skillsOn) refreshSkills();  // keep the skills window live (values/locks change)
    checkSkillGains(scene);  // announce skill base changes as journal system messages
    refreshParty();   // keep the party panel live + surface incoming invites (0xBF/0x06)
    refreshContainers();  // keep open container windows live (items move/disappear)
    refreshShop(scene);   // vendor buy/sell window (auto-opens on scene.shop)
    refreshGumps(scene);  // server-sent generic gumps/dialogs (0xB0/0xDD)
    refreshPopup(scene);  // right-click context menu (0xBF/0x14)
    refreshBook(scene);   // open book reader (0x93/0xD4 + 0x66)
    updateTargetUI(); // reflect the server's target-cursor state (crosshair + banner)
    updateDeathUI(scene); // grayscale + "You are dead" banner while the player is a ghost
    playSounds(scene);   // play new sound effects (0x54)
    updateMusic(scene);  // sync background music (0x6D)
    setStatus("live · " + new Date().toLocaleTimeString());
  } catch (e) {
    setStatus("waiting for scene… (" + e + ")");
  }
  diag.poll = performance.now() - t0;
  if (diag.poll > 150) console.warn(`[diag] slow poll ${diag.poll.toFixed(0)}ms`);
}

// The player is dead while their body is a ghost id (402/403). Desaturate the whole
// scene (CSS class on #map) and show a "You are dead" banner; both clear on res
// (the body reverts to a living id). Resurrection arrives via the existing gump system.
function updateDeathUI(s) {
  const dead = !!(s && s.player && isGhostBody(s.player.body));
  const map = document.getElementById("map");
  if (map) map.classList.toggle("dead", dead);
  const banner = document.getElementById("deadbanner");
  if (banner) banner.style.display = dead ? "block" : "none";
}

function updateAnimStates(s) {
  const now = performance.now();
  const seen = new Set();
  const touch = (id, x, y, z, dir, body, fb) => {
    seen.add(id);
    let st = anim.get(id);
    if (!st) { st = { rx: x, ry: y, stepDur: 300 }; anim.set(id, st); }
    if (st.tx !== x || st.ty !== y) {
      st.moveUntil = now + 650;
      // Measure this entity's real step cadence so we can glide one tile over
      // exactly that time → continuous motion (no walk-one-tile-then-pause).
      if (st.prevMoveT) st.stepDur = Math.min(600, Math.max(120, now - st.prevMoveT));
      st.prevMoveT = now;
    }
    Object.assign(st, { tx: x, ty: y, z, dir, body, fallback: fb });
  };
  for (const m of s.mobiles || []) touch("m" + m.serial, m.x, m.y, m.z ?? 0, m.dir ?? 4, m.body, notoColor(m.noto));
  // The player is rendered from the predicted position (set in renderFrame); here
  // we just seed/reconcile prediction against the authoritative server position.
  const p = s.player;
  if (p) {
    if (!pred) {
      pred = {
        x: p.x, y: p.y, z: p.z ?? 0, dir: p.dir ?? 4,
        steps: [], t0: 0, lastEnq: 0, enqGate: 0, moving: false,
        rx: p.x, ry: p.y, rz: p.z ?? 0,
        sx: p.x, sy: p.y, sz: p.z ?? 0, psx: p.x, psy: p.y,
      };
    }
    // Has the authoritative position settled (unchanged since the previous poll)?
    const serverStable = pred.psx === p.x && pred.psy === p.y;
    pred.psx = p.x; pred.psy = p.y;
    pred.sx = p.x; pred.sy = p.y; pred.sz = p.z ?? pred.sz; // server pos (lags ~poll)
    const denies = s.stats?.denies ?? 0;
    const denied = denies > lastDenies;
    lastDenies = denies;
    const off = cheby(pred.x - p.x, pred.y - p.y); // base vs authoritative
    if (off > SNAP_DIST) {
      // Real desync/teleport (gate, recall, big shove): jump everything instantly.
      pred.steps.length = 0; pred.t0 = 0;
      pred.x = p.x; pred.y = p.y; pred.z = p.z ?? pred.z; pred.dir = p.dir ?? pred.dir;
      pred.rx = p.x; pred.ry = p.y; pred.rz = p.z ?? pred.rz;
    } else if (denied) {
      // ClassicUO DenyWalk→Reset: drop the queue, set base to the server position;
      // the render *eases* there (processSteps), so a 1-tile correction glides in.
      pred.steps.length = 0; pred.t0 = 0;
      pred.x = p.x; pred.y = p.y; pred.z = p.z ?? pred.z; pred.dir = p.dir ?? pred.dir;
    } else if (!moveIntent && pred.steps.length === 0 && serverStable && off > 0
               && performance.now() - lastWalkSentAt > RECONCILE_HOLDOFF) {
      // Idle, queue drained, and the server has been settled at a DIFFERENT tile for
      // long enough that it isn't just the last walk's confirm still in flight — a
      // genuine divergence (shove, short teleport, drift). Converge the base. The
      // holdoff is what kills the "뒤로갔다 앞으로" yank: right after you stop, the
      // server momentarily looks settled one tile back (its confirm lagging a poll),
      // and snapping to it then re-snapping forward is exactly the artifact CUO never
      // shows. Prediction is 1:1 with the server, so inside the window we just trust it.
      pred.x = p.x; pred.y = p.y; pred.z = p.z ?? pred.z; pred.dir = p.dir ?? pred.dir;
    } else if (!moveIntent && pred.steps.length === 0 && serverStable) {
      pred.z = p.z ?? pred.z; // keep Z authoritative at rest (forced Z changes)
    }
    if (!anim.has("self")) anim.set("self", { rx: pred.rx, ry: pred.ry, rz: pred.rz, stepDur: 400, fallback: 0xffffff });
    seen.add("self");
  }
  for (const id of [...anim.keys()]) if (!seen.has(id)) anim.delete(id);
}

// add/remove only the tiles/statics that entered/left the view
function syncWorld(s) {
  const m = s.map || { radius: 14, tiles: [], cx: 0, cy: 0 };
  const span = 2 * m.radius + 1;
  const seenT = new Set(), seenS = new Set();

  // height of a tile in the window (for corner slopes); null if outside
  const zAt = (x, y) => {
    const col = x - m.cx + m.radius, row = y - m.cy + m.radius;
    if (col < 0 || col >= span || row < 0 || row >= span) return null;
    const t = m.tiles[row * span + col];
    return t ? (t.z | 0) : null;
  };
  for (let row = 0; row < span; row++) {
    for (let col = 0; col < span; col++) {
      const t = m.tiles[row * span + col];
      // Land graphics 0–2 are UO's "no draw"/void tiles (their art is a literal
      // "NO DRAW" placeholder bitmap). They appear under building floors / off-map;
      // ClassicUO never draws them, so neither do we (drop any stale sprite too).
      const x = m.cx + (col - m.radius), y = m.cy + (row - m.radius);
      const key = x + "," + y;
      if (!t || !t.g || t.g <= 2) {
        const e = tilePool.get(key);
        if (e) { world.removeChild(e.sp); e.sp.destroy(); tilePool.delete(key); }
        continue;
      }
      // Hidden by the Z ceiling (e.g. surface terrain over a basement): drop any
      // existing sprite and don't draw, so the floor below is revealed.
      if (t.h) {
        const e = tilePool.get(key);
        if (e) { world.removeChild(e.sp); e.sp.destroy(); tilePool.delete(key); }
        continue;
      }
      seenT.add(key);
      const z0 = t.z | 0;
      const e = tilePool.get(key);
      if (e && e.g === t.g && e.z === z0) continue; // unchanged
      // corner heights (ClassicUO: top=this, right=(x+1,y), bottom=(x+1,y+1), left=(x,y+1))
      const z1 = zAt(x + 1, y), z2 = zAt(x + 1, y + 1), z3 = zAt(x, y + 1);
      if (z1 === null || z2 === null || z3 === null) continue; // await neighbours
      const sloped = !(z0 === z1 && z1 === z2 && z2 === z3);

      let sp;
      if (!sloped) {
        const tex = texFor(`art/land/${t.g}.png`);
        if (!tex) continue;
        sp = makeFlatTile(x, y, z0, tex);
      } else if (t.tx > 0) {
        const tex = texFor(`texmap/${t.tx}.png`); // seamless texture for slopes
        if (!tex) continue;
        sp = makeStretchedTile(x, y, z0, z1, z2, z3, tex, true);
      } else {
        const tex = texFor(`art/land/${t.g}.png`); // no texmap → stretch the art
        if (!tex) continue;
        sp = makeStretchedTile(x, y, z0, z1, z2, z3, tex, false);
      }
      if (e) { world.removeChild(e.sp); e.sp.destroy(); }
      world.addChild(sp);
      tilePool.set(key, { sp, g: t.g, z: z0 });
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
    sp.zIndex = depthZ(st.x, st.y, st.pz ?? st.z, 4);
    // Tile + foliage flag for the transparency pass (circle-of-transparency / foliage fade).
    sp._tx = st.x; sp._ty = st.y; sp._foliage = !!st.f;
    // Animated static (flames/fountains/water wheels): the server baked the ART
    // tile-id frame sequence (`a`) + per-frame interval ms (`ai`). Prefetch each
    // frame's texture and store them so the animation pass can swap sp.texture.
    if (Array.isArray(st.a) && st.a.length > 1) {
      sp._frames = st.a.map((id) => texFor(`art/static/${id}.png`));
      sp._afids = st.a;            // keep ids so late-loading frames can be resolved
      sp._ai = st.ai || 200;
      sp._fbase = performance.now();
      sp._fidx = -1;
      animatedStatics.add(sp);
    }
    world.addChild(sp);
    staticPool.set(key, sp);
  }

  // Dynamic world items (doors, furniture, signs, corpses…): draw their REAL art
  // like statics, depth-sorted, instead of as dots. Persistent pool keyed by serial
  // (items move/open/disappear → re-create when graphic or position changes).
  const seenI = new Set();
  for (const it of s.items || []) {
    if (it.serial === undefined || !it.g) continue;
    const key = it.serial;
    seenI.add(key);
    const iz = it.z | 0;
    const e = itemPool.get(key);
    if (e && e.g === it.g && e.x === it.x && e.y === it.y && e.z === iz) continue; // unchanged
    const tex = texFor(`art/static/${it.g}.png`);
    if (!tex) continue; // await art, retry next poll
    if (e) { world.removeChild(e.sp); e.sp.destroy(); }
    const sp = new PIXI.Sprite(tex);
    sp.anchor.set(0.5, 1.0);
    sp.x = isoX(it.x, it.y); sp.y = isoY(it.x, it.y, iz) + HALF;
    sp.zIndex = depthZ(it.x, it.y, it.pz ?? iz, 5); // bias 5: just above same-tile statics
    // Tile + foliage flag for the transparency pass (circle-of-transparency / foliage fade).
    sp._tx = it.x; sp._ty = it.y; sp._foliage = !!it.f;
    sp.eventMode = "static"; sp.cursor = "pointer";
    const serial = it.serial;
    sp.on("pointerdown", (ev) => onEntityPointerDown(serial, ev, true)); // world item → loot on dbl-click
    sp.on("pointerover", () => { hoverEntity(serial); targetHighlightOn(sp); });
    sp.on("pointerout", () => { hoverOut(serial); targetHighlightOff(sp); });
    world.addChild(sp);
    itemPool.set(key, { sp, g: it.g, x: it.x, y: it.y, z: iz });
    markDirty();
  }
  for (const [k, e] of itemPool) {
    if (!seenI.has(k)) { world.removeChild(e.sp); e.sp.destroy(); itemPool.delete(k); markDirty(); }
  }

  // Tiles: keep once drawn — only drop them when they're well outside the
  // window (hysteresis), so sliding the camera never re-creates visible tiles.
  pruneFar(tilePool, m.cx, m.cy, m.radius + 4);
  // Statics: seen-based — a roof the server stops sending (player under cover)
  // must be removed so the interior shows.
  prune(staticPool, seenS, (e) => e);
  diag.tiles = tilePool.size + staticPool.size;
}
function prune(pool, seen, getSp) {
  for (const [key, e] of pool) {
    if (!seen.has(key)) {
      const sp = getSp(e);
      animatedStatics.delete(sp); // drop from the animation set before destroy
      world.removeChild(sp); sp.destroy(); pool.delete(key);
    }
  }
}
function pruneFar(pool, cx, cy, maxDist) {
  for (const [key, e] of pool) {
    const i = key.indexOf(",");
    const x = +key.slice(0, i), y = +key.slice(i + 1);
    if (Math.abs(x - cx) > maxDist || Math.abs(y - cy) > maxDist) {
      world.removeChild(e.sp); e.sp.destroy(); pool.delete(key);
    }
  }
}

// A flat land tile: the 44×44 diamond art as a centered sprite.
function makeFlatTile(x, y, z, tex) {
  const sp = new PIXI.Sprite(tex);
  sp.anchor.set(0.5, 0.5);
  sp.x = isoX(x, y); sp.y = isoY(x, y, z); sp.zIndex = depthZ(x, y, z - 2, 0);
  return sp;
}

// A sloped land tile: a 4-corner quad whose vertices follow the corner heights
// (top=this, right=(x+1,y), bottom=(x+1,y+1), left=(x,y+1)), per ClassicUO.
// `square` UVs map a seamless texmap; otherwise the diamond art's edge points.
function makeStretchedTile(x, y, z0, z1, z2, z3, tex, square) {
  const Bx = (x - y) * HALF, By = (x + y) * HALF;
  const aPosition = [
    Bx,        By - HALF - z0 * ZSTEP, // top
    Bx + HALF, By        - z1 * ZSTEP, // right
    Bx,        By + HALF - z2 * ZSTEP, // bottom
    Bx - HALF, By        - z3 * ZSTEP, // left
  ];
  const aUV = square ? [0, 0, 1, 0, 1, 1, 0, 1] : [0.5, 0, 1, 0.5, 0.5, 1, 0, 0.5];
  const geometry = new PIXI.Geometry({ attributes: { aPosition, aUV }, indexBuffer: [0, 1, 2, 0, 2, 3] });
  const mesh = new PIXI.Mesh({ geometry, texture: tex });
  // Sort a sloped tile by its AverageZ (ClassicUO Land.CalculateAverageZ: the mean
  // of whichever diagonal corner-pair differs less), NOT the top corner — otherwise
  // a slope whose top corner is high sorts in front of taller statics (e.g. stairs)
  // that sit behind it and wrongly covers them. (top=z0, right=z1, bottom=z2, left=z3)
  const avgZ = Math.abs(z0 - z2) <= Math.abs(z3 - z1) ? (z0 + z2) >> 1 : (z3 + z1) >> 1;
  mesh.zIndex = depthZ(x, y, avgZ - 2, 0);
  return mesh;
}

// Is world tile (x,y) walkable per the latest scene? (outside the window → assume yes)
function tileWalkable(x, y) {
  const m = scene && scene.map;
  if (!m) return true;
  const span = 2 * m.radius + 1;
  const col = x - m.cx + m.radius, row = y - m.cy + m.radius;
  if (col < 0 || col >= span || row < 0 || row >= span) return true;
  const t = m.tiles[row * span + col];
  return t ? t.w === 1 : true;
}

// Predicted standing Z for stepping onto (x,y) — the server's per-tile `sz`
// (CalculateNewZ). null when outside the window. Lets prediction raise/lower Z
// together with the step so stairs glide instead of popping.
function tileSZ(x, y) {
  const m = scene && scene.map;
  if (!m) return null;
  const span = 2 * m.radius + 1;
  const col = x - m.cx + m.radius, row = y - m.cy + m.radius;
  if (col < 0 || col >= span || row < 0 || row >= span) return null;
  const t = m.tiles[row * span + col];
  if (!t) return null;
  return t.sz !== undefined ? (t.sz | 0) : (t.z | 0);
}

// ClassicUO Pathfinder.CanWalk: resolve a step from (x,y) facing `dir`. Returns
// {dir,x,y} (possibly redirected) or null if blocked. A diagonal forbids corner-
// cutting (both flanking cardinals must be open) and, if blocked, redirects to the
// first open flanking cardinal — so you slide along a wall. A cardinal just fails.
function canWalk(x, y, dir) {
  let nx = x + DIR_DELTA[dir][0], ny = y + DIR_DELTA[dir][1], ndir = dir;
  let passed = tileWalkable(nx, ny);
  if (dir % 2 === 1) {
    if (passed) {
      for (const off of [1, -1]) {
        const cd = (dir + off + 8) % 8;
        if (!tileWalkable(x + DIR_DELTA[cd][0], y + DIR_DELTA[cd][1])) { passed = false; break; }
      }
    }
    if (!passed) {
      for (const off of [1, -1]) {
        const cd = (dir + off + 8) % 8;
        if (tileWalkable(x + DIR_DELTA[cd][0], y + DIR_DELTA[cd][1])) {
          ndir = cd; nx = x + DIR_DELTA[cd][0]; ny = y + DIR_DELTA[cd][1]; passed = true; break;
        }
      }
    }
  }
  return passed ? { dir: ndir, x: nx, y: ny } : null;
}

// Append predicted steps to the queue while a direction is held (ClassicUO
// PlayerMobile.Walk + CanWalk + Mobile.EnqueueStep). A turn is its own step (same
// tile, new facing); a move is the next tile (diagonals slide along walls).
// Faithful port of ClassicUO PlayerMobile.Walk: called every frame while a key is
// held, but SELF-GATED by `LastStepRequestTime` — it queues at most ONE step per
// walkTime (a turn costs TURN_DELAY=100ms, a move costs the step cadence). So a
// quick tap queues exactly one step (→ one tile, no "한 발자국 더"), a held key
// queues one per cadence, and the move right after a turn fires only 100ms later
// (snappy direction changes). processSteps renders the queue and sends one walk
// per committed step (we are the pacer).
function enqueueSteps(now) {
  if (!pred) return;
  if (!moveIntent) {
    // Released: finish the in-progress front step (it commits → one walk = the tile
    // you were already walking into) and drop any BUFFERED step. ClassicUO-faithful:
    // queued steps complete, no new one starts.
    pred.intentDir = null;
    if (pred.steps.length > 1) pred.steps.length = 1;
    return;
  }
  const req = moveIntent.dir, run = moveIntent.run;
  pred.intentDir = req;
  // Walk gate (ClassicUO PlayerMobile.Walk: LastStepRequestTime > now → return) +
  // queue cap. Exactly one step per walkTime — no look-ahead pre-queue (that queued
  // the next tile early, which then committed after release → "한 발자국 더").
  if (pred.steps.length >= MAX_STEPS) return;
  if (now < (pred.lastStepReq || 0)) return;
  const tail = pred.steps.length ? pred.steps[pred.steps.length - 1] : pred;
  const res = canWalk(tail.x, tail.y, req);
  let walkTime = TURN_DELAY;
  const pushTurn = (d) => { pred.steps.push({ x: tail.x, y: tail.y, z: tail.z, dir: d, run, turn: true }); walkTime = TURN_DELAY; trace(`ENQ turn dir=${d} q=${pred.steps.length}`); };
  const pushMove = (d, nx, ny) => {
    const sz = tileSZ(nx, ny);
    pred.steps.push({ x: nx, y: ny, z: sz !== null ? sz : tail.z, dir: d, run, turn: false });
    walkTime = stepDelay(run, mounted());
    trace(`ENQ move dir=${d} q=${pred.steps.length}`);
  };
  if (tail.dir === req) {
    // Facing the requested dir → move (or, if CanWalk redirected a blocked diagonal,
    // turn to the cardinal first). Fully blocked → stand, but still gate so we don't
    // spin the CanWalk check every frame.
    if (!res) { pred.lastStepReq = now + stepDelay(run, mounted()); return; }
    if (res.dir !== req) pushTurn(res.dir); else pushMove(res.dir, res.x, res.y);
  } else if (res && res.dir === tail.dir) {
    pushMove(res.dir, res.x, res.y);            // redirect equals current facing → move
  } else {
    pushTurn(res ? res.dir : req);              // turn toward the resolved dir (or into a wall)
  }
  // Anchor the gate to the rigid step schedule, not jittery wall-clock. processSteps
  // commits on a fixed grid (`t0 += dur`); if the gate were `now + walkTime` it would
  // creep forward each step (each enqueue fires at `now >= prev gate`, only ever
  // later) until it lagged behind the commit grid — then enqueue is blocked on the
  // very frame the step commits, the queue drains for a frame, and the walk micro-
  // stutters. While movement is continuous (this enqueue is within one cadence of the
  // last) we advance from the PREVIOUS gate so it stays locked to the grid; after an
  // idle/release gap we restart from `now` (so taps and resume behave unchanged).
  const cont = pred.lastStepReq && now < pred.lastStepReq + walkTime;
  pred.lastStepReq = (cont ? pred.lastStepReq : now) + walkTime;
}

// Interpolate the rendered position through the queue front (ClassicUO
// Mobile.ProcessSteps): X/Y/Z advance together by the step's time fraction; a
// completed step commits to the base and the next begins (carrying the time
// remainder for continuous motion). Turns are consumed instantly (facing only).
function processSteps(now) {
  if (!pred) return;
  let guard = 0;
  while (pred.steps.length && guard++ < MAX_STEPS + 2) {
    const s = pred.steps[0];
    if (!pred.t0) pred.t0 = now;
    // The single pacer: a move interpolates over its UO cadence; a turn HOLDS for
    // TURN_DELAY (facing change, no position move) — this is the turn-then-move
    // timing. (Enqueue no longer paces; it just keeps the buffer full.)
    const dur = s.turn ? TURN_DELAY : stepDelay(s.run, mounted());
    const prog = Math.min(1, (now - pred.t0) / dur);
    if (s.turn) {
      pred.dir = s.dir; pred.rx = pred.x; pred.ry = pred.y; pred.rz = pred.z; pred.moving = true;
    } else {
      pred.rx = pred.x + (s.x - pred.x) * prog;
      pred.ry = pred.y + (s.y - pred.y) * prog;
      pred.rz = pred.z + (s.z - pred.z) * prog;
      pred.dir = s.dir; pred.moving = true;
    }
    if (prog >= 1) {                       // step complete → commit base, carry remainder
      if (!s.turn) { pred.x = s.x; pred.y = s.y; pred.z = s.z; }
      pred.dir = s.dir;
      // ClassicUO model: WE are the pacer. Each committed step (the prediction
      // paced it at the UO cadence) is sent to the server as ONE walk — so the
      // server does exactly the steps we did. A tap commits one step → one walk →
      // one server tile (no overshoot); release stops committing → server stops.
      sendInput(`walk:${s.dir}:${s.run ? 1 : 0}`);
      lastWalkSentAt = now;
      trace(`CMT ${s.turn ? "turn" : "move"} dir=${s.dir} -> walk`);
      pred.steps.shift();
      pred.t0 += dur;
      continue;
    }
    return;
  }
  // Queue empty → *ease* the render onto the base tile rather than snapping. At a
  // stop the server may settle ~1 tile from where we predicted (intent-based pacing
  // can't align the stop boundary exactly); easing that in over ~120ms reads as a
  // gentle final step instead of a teleport. Genuine desyncs/teleports are snapped
  // hard in reconcile (which also sets rx/ry), so this only smooths small offsets.
  pred.t0 = 0;
  const k = 0.25; // ~ease over ~6 frames (~100ms at 60fps)
  pred.rx += (pred.x - pred.rx) * k;
  pred.ry += (pred.y - pred.ry) * k;
  pred.rz += (pred.z - pred.rz) * k;
  const settled = Math.abs(pred.x - pred.rx) < 0.04 && Math.abs(pred.y - pred.ry) < 0.04;
  if (settled) { pred.rx = pred.x; pred.ry = pred.y; pred.rz = pred.z; }
  pred.moving = !settled; // keep the walk cycle while easing the last bit
}

function renderFrame(dt) {
  if (!scene) return;
  const now = performance.now();
  moveIntent = activeMove();   // mouse (RMB) or held keys → drives prediction
  // Player: append predicted steps while a key is held, then interpolate the queue.
  const me = anim.get("self");
  if (me && pred) {
    enqueueSteps(now);
    processSteps(now);
    me.rx = pred.rx; me.ry = pred.ry; me.rz = pred.rz; me.z = pred.z; me.dir = pred.dir;
    me.animMoving = pred.moving;
    me.stepDur = stepDelay(!!(moveIntent && moveIntent.run), mounted());
    // Leg cadence tied to GROUND COVERED (cyclesPerTile): walking unchanged
    // (80ms/frame); running takes bigger strides so the legs don't whirl. Phase
    // is a 0..1 cycle fraction.
    me.animPhase = pred.moving
      ? ((me.animPhase || 0) + cyclesPerTile(!!(moveIntent && moveIntent.run)) * dt / (me.stepDur || 300)) % 1
      : 0;
    if (scene.player) me.body = scene.player.body;
  }
  // Glide the OTHER entities (mobiles) toward their target tile at constant
  // velocity, timed to their measured step cadence (×1.12 margin so they're still
  // moving when the next tile arrives). The player ("self") is driven by the queue
  // above, not this glide. Snap on big jumps.
  for (const st of anim.values()) {
    if (st === me) continue;
    // Ease vertical position (z) too, so stairs/ramps glide instead of popping.
    const tz = st.z | 0;
    if (st.rz === undefined) st.rz = tz;
    if (st.rz !== tz) {
      st.rz += (tz - st.rz) * Math.min(1, (dt / (st.stepDur || 300)) * 2.5);
      if (Math.abs(tz - st.rz) < 0.08) st.rz = tz;
    }
    const dx = st.tx - st.rx, dy = st.ty - st.ry;
    const dist = Math.hypot(dx, dy);
    const mv = dist > 0.06 || now < (st.moveUntil || 0);
    st.animMoving = mv;
    // Leg cadence tied to ground covered. We don't get other mobiles' run flag, so
    // infer it from their measured step cadence (a fast ~≤280ms step = running).
    const stepDur = st.stepDur || 300;
    st.animPhase = mv ? ((st.animPhase || 0) + cyclesPerTile(stepDur <= 280) * dt / stepDur) % 1 : 0;
    if (dist < 1e-3) continue;
    if (dist > 3) { st.rx = st.tx; st.ry = st.ty; st.rz = tz; continue; }
    const dur = (st.stepDur || 300) * 1.12;
    const step = dt / dur; // tiles this frame
    if (dist <= step) { st.rx = st.tx; st.ry = st.ty; }
    else { st.rx += (dx / dist) * step; st.ry += (dy / dist) * step; }
  }
  // camera follows the eased player so the avatar stays centered (eased z too)
  const self = anim.get("self");
  if (self) {
    app.stage.position.set(app.screen.width / 2 - isoX(self.rx, self.ry), app.screen.height / 2 - isoY(self.rx, self.ry, self.rz ?? self.z));
  }
  // Cycle animated statics (flames/fountains/water wheels) to their current frame.
  tickAnimatedStatics(now);
  // Fade statics/foliage that would hide the avatar (circle-of-transparency).
  transparencyPass();
  // Request a redraw only when something is actually animating: self moving (camera
  // scrolls), a gliding mobile, or floating speech. Idle ⇒ no redraw ⇒ ~0 GPU.
  if (overLayer.children.length) markDirty();
  else for (const st of anim.values()) if (st.animMoving || st.act) { markDirty(); break; }
  drawMobs();
  drawOverheads(now);
  drawDamage(now);
  drawEffects(now);
  drawBars(now);

  // fps / worst-frame
  diag.frames++; diag.acc += dt; diag.worstFrame = Math.max(diag.worstFrame, dt);
  if (dt > 70) console.warn(`[diag] slow frame ${dt.toFixed(0)}ms`);
  if (diag.acc >= 500) {
    diag.fps = Math.round((1000 * diag.frames) / diag.acc);
    diag.frames = 0; diag.acc = 0;
    updateDiag();
  }
}

// Advance animated statics (flames/fountains/water wheels). The server baked each
// one's ART tile-id frame sequence (`_frames`) + interval (`_ai`); we just pick the
// current frame by wall-clock time and swap sp.texture when the index changes (only
// then). Frames that are still streaming in are re-resolved from cache via `_afids`;
// until a frame's texture is ready we keep the current one. markDirty() repaints the
// on-demand renderer whenever a texture actually changed. Cheap: iterates only the
// animated-statics set, and most frames are no-ops between swaps.
function tickAnimatedStatics(now) {
  if (!animatedStatics.size) return;
  let changed = false;
  for (const sp of animatedStatics) {
    if (sp.destroyed) { animatedStatics.delete(sp); continue; }
    const frames = sp._frames, n = frames.length;
    const idx = Math.floor(now / (sp._ai || 200)) % n;
    if (idx === sp._fidx) continue;
    let tex = frames[idx];
    if (!tex) { tex = texFor(`art/static/${sp._afids[idx]}.png`); frames[idx] = tex; }
    if (!tex) continue; // frame not loaded yet → keep the current texture
    sp.texture = tex; sp._fidx = idx; changed = true;
  }
  if (changed) markDirty();
}

// Circle of transparency + foliage fade. Statics/items that draw IN FRONT of the
// player (higher zIndex → they'd occlude the avatar) and sit within a small radius
// fade to semi-transparent, so you can always see yourself — like ClassicUO. Sprites
// are POOLED/persistent, so we track which we faded last pass and restore any that
// dropped out (otherwise statics would stay stuck-transparent). Cheap: a distance
// compare per pooled sprite; far ones are already alpha 1 and skipped.
const fadedSprites = new Set();
const R_COT = 2, R_FOL = 3;            // tile radius: circle-of-transparency / foliage
const A_COT = 0.55, A_FOL = 0.45;      // faded alpha: statics / foliage
function transparencyPass() {
  let ptx, pty, pz;
  if (pred) { ptx = Math.round(pred.rx); pty = Math.round(pred.ry); pz = pred.z; }
  else if (scene && scene.player) { ptx = scene.player.x; pty = scene.player.y; pz = scene.player.z; }
  else return;
  // The avatar's current draw depth (same formula drawMobs uses for "self").
  const playerZi = depthZ(ptx, pty, (pz | 0) + 1, 8);
  const newFaded = new Set();
  let changed = false;
  const consider = (sp) => {
    const r = sp._foliage ? R_FOL : R_COT;
    let target = 1;
    // Within radius (Chebyshev) AND drawn in front of the avatar → would occlude.
    if (Math.abs(sp._tx - ptx) <= r && Math.abs(sp._ty - pty) <= r && sp.zIndex > playerZi) {
      target = sp._foliage ? A_FOL : A_COT;
    }
    if (target !== 1) {
      newFaded.add(sp);
      if (sp.alpha !== target) { sp.alpha = target; changed = true; }
    }
  };
  for (const sp of staticPool.values()) consider(sp);
  for (const e of itemPool.values()) consider(e.sp);
  // Restore sprites that were faded last pass but aren't in the fade set now.
  for (const sp of fadedSprites) {
    if (!newFaded.has(sp) && !sp.destroyed && sp.alpha !== 1) { sp.alpha = 1; changed = true; }
  }
  fadedSprites.clear();
  for (const sp of newFaded) fadedSprites.add(sp);
  if (changed) markDirty();
}

// Dressed humans composite as a STACK of hued sprites (body + worn equipment),
// all sharing the body's screen position / anchor / depth. mobSprites is keyed by
// "<id>#<slot>" so each stack layer is a persistent sprite (no per-frame re-create
// → no full world re-sort), reused/pruned exactly like the single body was before.
const mobSprites = new Map(); // "<id>#<slot>" -> persistent layer sprite in the sorted world layer
const itemHits = new Map();   // "i"+serial -> invisible click target over a ground-item dot

// UO equipment draw order (back → front). Lower index = drawn earlier/behind, so
// clothes sit over the body and hair (11) / beard (16) / weapons composite on top.
// Per-direction draw order — a faithful port of ClassicUO `LayerOrder.UsedLayers`
// (UO layer numbers). The cloak (20) moves front/back with facing; the backpack (21)
// and mount (25) are NOT in any list → never drawn as a worn body layer (ClassicUO
// skips them). A layer not in the facing's list is not drawn (rank -1).
const _LO_DEF = [5, 4, 3, 24, 13, 8, 9, 14, 15, 19, 7, 23, 17, 22, 10, 11, 12, 16, 18, 1, 20, 6, 2];
const _LO_0 = [5, 4, 3, 24, 13, 8, 9, 14, 15, 19, 7, 23, 17, 22, 10, 11, 12, 16, 18, 1, 6, 2, 20]; // facing away → cloak in front
const _LO_3 = [20, 5, 4, 3, 24, 13, 8, 9, 14, 15, 19, 7, 23, 17, 22, 12, 10, 11, 16, 18, 6, 1, 2]; // facing viewer → cloak behind
const LAYER_ORDER_DIR = [_LO_0, _LO_DEF, _LO_DEF, _LO_3, _LO_DEF, _LO_DEF, _LO_DEF, _LO_DEF];
const layerRank = (l, dir) => LAYER_ORDER_DIR[dir & 7].indexOf(l | 0); // -1 = not drawn

// Ghost body ids: a dead human renders as a translucent ghost (402=male, 403=female).
const GHOST_BODIES = new Set([402, 403]);
const isGhostBody = (b) => GHOST_BODIES.has(b | 0);

// Is a worn layer hidden by something over it? Faithful port of ClassicUO
// MobileView.IsCovered: a robe (and a few special items) hides the inner clothes
// it fully covers, so they don't peek through. `byLayer` maps layer → equip entry
// ({ g: graphic, ... }). UO layers: Shoes 3, Pants 4, Hair 11, Torso 13, Tunic 17,
// Arms 19, Robe 22, Skirt 23, Legs 24, Helmet 6.
function isCovered(byLayer, layer) {
  const g = (l) => (byLayer[l] ? (byLayer[l].g | 0) : null);
  const has = (l) => byLayer[l] != null;
  const robe = g(22);
  switch (layer | 0) {
    case 3: { // Shoes
      const pants = g(4);
      if (has(24) || pants === 0x1411) return true;
      if (pants === 0x0513 || pants === 0x0514 || robe === 0x0504) return true;
      break;
    }
    case 4: { // Pants
      if (has(24) || robe === 0x0504) return true;
      const pants = g(4);
      if (pants === 0x01EB || pants === 0x03E5 || pants === 0x03EB) {
        const skirt = g(23);
        if (skirt != null && skirt !== 0x01C7 && skirt !== 0x01E4) return true;
        if (robe != null && robe !== 0x0229 && (robe <= 0x04E7 || robe > 0x04EB)) return true;
      }
      break;
    }
    case 17: { // Tunic
      if (g(17) === 0x0238) return robe != null && robe !== 0x9985 && robe !== 0x9986 && robe !== 0xA412;
      break;
    }
    case 13: { // Torso
      if (robe != null && robe !== 0 && robe !== 0x9985 && robe !== 0x9986 && robe !== 0xA412 && robe !== 0xA2CA) return true;
      const tunic = g(17);
      if (tunic != null && tunic !== 0x1541 && tunic !== 0x1542) {
        const torso = g(13);
        if (torso === 0x782A || torso === 0x782B) return true;
      }
      break;
    }
    case 19: // Arms
      return robe != null && robe !== 0 && robe !== 0x9985 && robe !== 0x9986 && robe !== 0xA412;
    case 6:   // Helmet
    case 11: { // Hair
      if (robe != null) {
        if (robe > 0x3173) {
          if (robe === 0x4B9D || robe === 0x7816) return true;
        } else if (robe <= 0x2687) {
          if (robe < 0x2683) return robe >= 0x204E && robe <= 0x204F;
          return true;
        } else if (robe === 0x2FB9 || robe === 0x3173) {
          return true;
        }
      }
      break;
    }
  }
  return false;
}

function drawMobs() {
  // Mobiles live *inside* the depth-sorted `world` container (not a top layer) so
  // statics in front occlude them. Sprites are PERSISTENT and updated in place —
  // recreating them every frame marked the (huge) world container's child list
  // dirty, forcing a full re-sort of ~2800 tiles every frame (the CPU hog). Now we
  // only touch a sprite's zIndex when it actually crosses a tile (rarely), so the
  // expensive re-sort happens per-tile, not per-frame.
  entLayer.clear();
  diag.ents = 0;
  const seen = new Set();

  // (Dynamic world items are now drawn as real art sprites in syncWorld's itemPool,
  // not dots here.)
  // Resolve each rendered entity's skin hue + worn equipment from the scene.
  const mobById = new Map();
  for (const m of scene.mobiles || []) mobById.set("m" + m.serial, m);
  for (const [id, st] of anim) {
    diag.ents++;
    const d = st.dir & 7;
    // We only know run/mount state for our own player; other mobiles walk/stand.
    const isSelf = id === "self";
    const moving = !!st.animMoving; // set in renderFrame (glide + held/mouse)
    const running = isSelf && !!(moveIntent && moveIntent.run);
    // Look up this entity's scene record (self → player; else mobile) for skin hue,
    // worn equipment, and mount state. Mount is per-entity: self uses player.mounted,
    // others use their own `mounted`/`mountAnim` fields.
    const ent = isSelf ? scene.player : mobById.get(id);
    const mounted = !!(ent && ent.mounted);
    const mountAnim = (ent && (ent.mountAnim | 0)) || 0;
    // A dead human is a ghost (body 402/403). Those ghost-body ids carry no animation
    // frames in the muls, so animate the ghost with the LIVING human body (402→400,
    // 403→401) rendered translucent, equipment hidden (UO shows ghosts bare).
    const ghost = isGhostBody(st.body);
    const bodyAnim = ghost ? (st.body === 403 ? 401 : 400) : (st.body | 0);
    // War-mode combat stance applies to our own avatar (the only mobile whose war
    // state the server tells us); others fall back to the normal idle stand.
    const inWar = isSelf && !!(scene && scene.war);
    // A one-shot 0x6E action (combat swing, bow, get-hit) takes over the pose while
    // it plays, then expires → revert to walk/stand/war. We only retire it once the
    // group's real frame count has loaded (so a placeholder count can't cut it short).
    let group, frames, frame;
    const act = st.act;
    // The raw 0x6E `action` isn't always a direct animation group: spell casts send
    // high "action" codes (UO SpellInfo.Action, ~200+) that map to the cast gesture.
    // resolveActionGroup() folds those onto the body's real group set.
    const ag = (act && !ghost) ? resolveActionGroup(act.group, bodyAnim) : 0;
    if (act && !ghost) {
      framesFor(bodyAnim, ag, d); // kick the frame-count/centers load
      const fk = `${bodyAnim}/${ag}/${d}`;
      const loaded = frameCount.has(fk) ? Math.max(1, frameCount.get(fk)) : 0;
      const fi = Math.floor((performance.now() - act.startMs) / act.frameMs);
      if (loaded > 0 && fi >= loaded) st.act = null; // played every frame → done
    }
    if (st.act && !ghost) {
      group = ag;
      frames = framesFor(bodyAnim, group, d);
      const fi = Math.max(0, Math.min(frames - 1, Math.floor((performance.now() - act.startMs) / act.frameMs)));
      frame = act.fwd ? fi : (frames - 1 - fi);
      if (st.prevFrameKey !== `${group}/${d}`) {
        for (let f = 0; f < frames; f++) texFor(`anim/${bodyAnim}/${group}/${d}/${f}.png`);
        st.prevFrameKey = `${group}/${d}`;
      }
    } else {
      group = animGroup(moving, running, mounted, bodyAnim, inWar);
      frames = framesFor(bodyAnim, group, d);
      // animPhase is a 0..1 cycle fraction (advanced per ground covered); map it to
      // the real frame count. Prefetch the whole cycle so frames don't pop in.
      frame = moving ? Math.floor((st.animPhase || 0) * frames) % frames : 0;
      if (moving && bodyAnim && st.prevFrameKey !== `${group}/${d}`) {
        // Prefetch the cycle once per (group,dir) change, not every frame.
        for (let f = 0; f < frames; f++) texFor(`anim/${bodyAnim}/${group}/${d}/${f}.png`);
        st.prevFrameKey = `${group}/${d}`;
      }
    }
    const skinHue = ent && ent.hue ? ent.hue : 0;
    // Compose the character from stable PARTS (mount, body, each worn layer). Two
    // fixes for the walk/run "naked↔dressed" flicker and the layer-swap bug:
    //  • PER-PART last-good texture (`st.partTex`): when a part's texture for the
    //    current frame is still loading, reuse its previous frame instead of dropping
    //    it — so no layer (or the body) ever vanishes for a frame mid-walk.
    //  • STABLE per-part keys + rank-based z (not a shifting array index): a layer
    //    that's momentarily missing no longer shoves the others into different slots
    //    and swaps their textures.
    if (!st.partTex) st.partTex = new Map();
    const entries = [];
    // bodyId/grp/frm identify the source frame so we can fetch its draw-center and
    // position the part correctly (ClassicUO math) rather than foot-anchoring it.
    const part = (key, url, rank, interactive, bodyId, grp, frm) => {
      let t = url ? texFor(url) : null;
      if (t) st.partTex.set(key, t); else t = st.partTex.get(key);
      if (t) {
        const c = bodyId != null ? centerFor(bodyId, grp, d, frm) : null;
        entries.push({ key, tex: t, rank, interactive, cx: c ? c[0] : null, cy: c ? c[1] : null });
      }
    };
    // MOUNT (behind the rider): the layer-25 item's AnimID animated as an animal
    // (walk=0/run=1/stand=2) driven by the rider's movement; rider uses ONMOUNT groups.
    if (mountAnim > 0 && !ghost) {
      const mg = moving ? (running ? 1 : 0) : 2;
      const mFrames = framesFor(mountAnim, mg, d);
      const mFrame = moving ? Math.floor((st.animPhase || 0) * mFrames) % mFrames : 0;
      if (moving && st.prevMountKey !== `${mg}/${d}`) {
        for (let f = 0; f < mFrames; f++) texFor(`anim/${mountAnim}/${mg}/${d}/${f}.png`);
        st.prevMountKey = `${mg}/${d}`;
      }
      part("mount", `anim/${mountAnim}/${mg}/${d}/${mFrame}.png`, -1, false, mountAnim, mg, mFrame);
    }
    // BODY (hued by skin).
    part("body", bodyAnim ? `anim/${bodyAnim}/${group}/${d}/${frame}.png${skinHue ? `?hue=${skinHue}` : ""}` : null, 0, true, bodyAnim, group, frame);
    // WORN LAYERS (clothes/hair/beard), each hued, over the body in the facing's UO
    // draw order. Layers not in that order (backpack 21, mount 25) are skipped —
    // the mount is drawn separately as the animal above.
    const byLayer = {};
    if (ent && ent.equip) for (const e of ent.equip) byLayer[e.layer] = e;
    // A ghost wears only the (translucent) death robe — layer 22 OuterTorso. Living
    // mobiles show every worn layer. The robe's anim aligns because we drew the body
    // with the living human anim (bodyAnim) above.
    const worn = st.body && ent && ent.equip
      ? ent.equip.filter((e) => (e.anim | 0) > 0 && layerRank(e.layer, d) >= 0 && !isCovered(byLayer, e.layer)
          && (!ghost || e.layer === 22)) : null;
    if (worn && worn.length) {
      // Prefetch every layer's WHOLE frame cycle once per (group,dir) change, so the
      // full dressed frame is decoded before it's shown (kills per-frame layer lag).
      if (moving && st.prevEquipKey !== `${group}/${d}`) {
        for (const e of worn) for (let f = 0; f < frames; f++) {
          texFor(`anim/${e.anim}/${group}/${d}/${f}.png${e.hue ? `?hue=${e.hue}` : ""}`);
        }
        st.prevEquipKey = `${group}/${d}`;
      }
      for (const e of worn) {
        // Trigger the layer's animinfo load (frame count + per-frame draw-centers)
        // so centerFor(e.anim,…) resolves. Without this the body positions by its
        // real draw-center while clothes fall back to the foot anchor → the worn
        // layers appear shifted down off the body.
        framesFor(e.anim, group, d);
        part("L" + e.layer, `anim/${e.anim}/${group}/${d}/${frame}.png${e.hue ? `?hue=${e.hue}` : ""}`,
          1 + layerRank(e.layer, d), false, e.anim, group, frame);
      }
    }
    const x = isoX(st.rx, st.ry), y = isoY(st.rx, st.ry, st.rz ?? st.z);
    if (entries.length) {
      entries.sort((a, b) => a.rank - b.rank);
      // zIndex only changes when the mobile crosses a tile (assigning it forces a
      // re-sort). All parts share the body's depth; a rank epsilon (≪ the per-z step
      // of 16) keeps them back→front regardless of which parts are present this frame.
      const zi = depthZ(Math.round(st.rx), Math.round(st.ry), st.z + 1, 8);
      for (const e of entries) {
        const key = id + "#" + e.key;
        let sp = mobSprites.get(key);
        if (!sp) {
          sp = new PIXI.Sprite(e.tex);
          sp.anchor.set(0.5, 1.0);
          // Only the body is the click target; mount/clothing/hair never eat clicks.
          if (id !== "self" && e.interactive) {
            sp.eventMode = "static";
            sp.cursor = "pointer";
            const serial = id.slice(1);
            sp.on("pointerdown", (ev) => onEntityPointerDown(serial, ev));
            // OPL tooltip on hover (same flow as world items) + target highlight.
            sp.on("pointerover", () => { hoverEntity(serial); targetHighlightOn(sp); });
            sp.on("pointerout", () => { hoverOut(serial); targetHighlightOff(sp); });
          } else {
            sp.eventMode = "none";
          }
          world.addChild(sp);
          mobSprites.set(key, sp);
        }
        if (sp.texture !== e.tex) sp.texture = e.tex;
        // Position by the frame's draw-center (ClassicUO: top-left at screenX - cx,
        // screenY - height - cy). This is what seats a rider on a mount and aligns
        // held items / armor / hair instead of stacking everything at the feet.
        // Until the center loads, fall back to the foot anchor.
        if (e.cx != null) {
          sp.anchor.set(0, 0);
          sp.x = x - e.cx;
          sp.y = (y - 3) - e.tex.height - e.cy;
        } else {
          sp.anchor.set(0.5, 1.0);
          sp.x = x; sp.y = y - 3;
        }
        sp.visible = true;
        // Dead humans render as translucent ghosts. Sprites are pooled/persistent, so
        // we must reset alpha to 1 for non-ghost bodies (else a former ghost stays faint).
        sp.alpha = isGhostBody(st.body) ? 0.45 : 1;
        const z = zi + e.rank / 256;
        if (sp.zIndex !== z) sp.zIndex = z;
        seen.add(key);
      }
    } else if (st.body) {
      // Nothing loaded yet → a small fallback dot until textures arrive.
      entLayer.circle(x, y - 3, 3).fill(st.fallback || 0xffffff);
    }
  }
  // Drop layer sprites for entities/slots that left view (or shed equipment).
  for (const [key, sp] of mobSprites) {
    if (!seen.has(key)) { world.removeChild(sp); sp.destroy(); mobSprites.delete(key); }
  }
}

// Notoriety → name color (ClassicUO NotorietyFlag): 1 Innocent=blue, 2 Ally=green,
// 3 Gray(attackable)=gray, 4 Criminal=gray, 5 Enemy=orange, 6 Murderer=red,
// 7 Invulnerable=yellow. 0/unknown → neutral off-white.
function notoColor(n) {
  return { 1: 0x4f8cf7, 2: 0x46a758, 3: 0x9aa0a6, 4: 0x9aa0a6, 5: 0xd98a2b, 6: 0xe5484d, 7: 0xf5d442 }[n] || 0xd6dae0;
}
const cssColor = (n) => "#" + (n >>> 0).toString(16).padStart(6, "0");

// ---- buff / debuff bar ----
// Display-only chips under the minimap. Each scene.buff = { icon, name, dur }.
// `dur` is the duration (seconds) the server sent; we record when an icon first
// appeared and count down locally (mm:ss). dur 0 = permanent (no timer). The bar
// has pointer-events:none so it never blocks clicks.
const buffSeen = new Map(); // icon -> { firstSeen: ms, dur: seconds, name }
// Names hinting a debuff → red tint; everything else is a (green) buff.
const DEBUFF_RE = /poison|curse|weaken|clumsy|feeble|strangle|bleed|mortal|corpse|pain|evil omen|paralyze|sleep|blood oath|dismount|death/i;

function refreshBuffs(s) {
  const bar = document.getElementById("buffs");
  if (!bar) return;
  // The buff/debuff icon bar (0xDF) is an AOS/SA feature — hide it entirely in T2A.
  if (T2A) { bar.style.display = "none"; return; }
  const list = (s && s.buffs) || [];
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
    const name = document.createElement("span"); name.className = "bn"; name.textContent = b.name;
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
  // place-name labels (cull off-canvas; fade names when zoomed far out).
  if (s >= 0.6) {
    ctx.textAlign = "center"; ctx.textBaseline = "middle";
    ctx.font = "11px ui-monospace, monospace"; ctx.lineWidth = 2.5;
    for (const [lx, ly, name] of PLACES) {
      const [sx, sy] = wmWorldToScreen(lx, ly, w, h);
      if (sx < 0 || sy < 0 || sx > w || sy > h) continue;
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
      if (showLabels && p.name) {   // labels only when zoomed in + a name exists
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

// ---- overhead speech (ClassicUO MessageManager / overhead text) ----
const OVERHEAD_HEAD = 68;   // px above the feet anchor — clears the head (incl. hats/hair)

// Client-side system messages (skill gains, etc.) that aren't in the server journal.
// They render after the server's journal lines in `hud()`. Capped FIFO so old notices
// don't pile up forever at the bottom.
const localJournal = [];           // { text } — newest last
const LOCAL_JOURNAL_MAX = 40;
function addSysMessage(text) {
  localJournal.push({ text });
  while (localJournal.length > LOCAL_JOURNAL_MAX) localJournal.shift();
}
// Scan the journal for new lines and float each above its speaker once.
let journalPrimed = false;
function ingestSpeech(s) {
  const now = performance.now();
  const pserial = s.player ? (s.player.serial >>> 0) : 0;
  // On the FIRST scene after a (re)load, don't replay the journal backlog as floating
  // overheads — just advance the seq cursor so only genuinely NEW lines float after.
  if (!journalPrimed) {
    journalPrimed = true;
    for (const j of s.journal || []) lastJournalSeq = Math.max(lastJournalSeq, j.seq | 0);
    return;
  }
  for (const j of s.journal || []) {
    const seq = j.seq | 0;
    if (seq <= lastJournalSeq) continue;
    lastJournalSeq = seq;
    const text = (j.text || "").trim();
    // Cliloc lines now arrive pre-resolved to real text (play.rs), so float them
    // too — only truly-empty lines are skipped.
    if (!text) continue;
    const serial = (j.serial >>> 0);
    if (!serial || serial === 0xffffffff) continue;     // system message → not overhead
    const id = serial === pserial ? "self" : "m" + serial;
    if (!anim.has(id)) continue;                         // speaker not in view
    addOverhead(id, text, j.type | 0, j.hue | 0, now);
  }
}

function addOverhead(id, text, type, hue, now) {
  // DOM label (crisp), not PIXI text — see the note in drawBars about the canvas
  // being pixel-upscaled. On T2A even single-click names arrive as overhead text.
  const el = document.createElement("div");
  el.className = "oh-label" + (MSG_CLASS[type] ? " " + MSG_CLASS[type] : "");
  el.textContent = text;
  namesEl().appendChild(el);
  // Linger longer for longer lines (ClassicUO scales with length), then fade.
  const ttl = Math.min(8000, 3000 + text.length * 70);
  const o = { id, text, type, hue, born: now, ttl, el, _c: null };
  applyOverheadColor(o); // server-hue → type-default; re-applied each frame if late
  overheads.push(o);
  while (overheads.length > 40) { const x = overheads.shift(); if (x.el) x.el.remove(); }
}
// Set the overhead's color (only writes the DOM when it actually changes, so a
// late-arriving hue from the async hue table recolors it on a later frame).
function applyOverheadColor(o) {
  const c = msgColor(o.type, o.hue);
  if (o._c !== c) { o.el.style.color = c; o._c = c; }
}

// Position each floating line above its speaker (screen coords = camera + canvas→CSS
// stretch), stack multiples upward, fade out near end of life, reap expired ones.
function drawOverheads(now) {
  const stack = new Map();   // entity id → accumulated CSS height already placed
  const fx = window.innerWidth / app.renderer.width, fy = window.innerHeight / app.renderer.height;
  for (let i = overheads.length - 1; i >= 0; i--) {
    const o = overheads[i];
    const age = now - o.born;
    if (age >= o.ttl) { if (o.el) o.el.remove(); overheads.splice(i, 1); continue; }
    const st = anim.get(o.id);
    if (!st) { if (o.el) o.el.style.display = "none"; continue; } // speaker left view
    o.el.style.display = "block";
    applyOverheadColor(o);   // pick up a server hue that resolved after creation
    const up = stack.get(o.id) || 0;
    o.el.style.left = ((app.stage.x + isoX(st.rx, st.ry)) * fx) + "px";
    o.el.style.top = ((app.stage.y + isoY(st.rx, st.ry, st.rz ?? st.z) - OVERHEAD_HEAD) * fy - up) + "px";
    stack.set(o.id, up + (o.el.offsetHeight || 16) + 2);
    const left = o.ttl - age;
    o.el.style.opacity = left < 700 ? Math.max(0, left / 700) : 1;
  }
}

// Float a red (orange when it's us) damage number over each newly-hit entity.
// Play new character-animation events (0x6E): a transient action (combat swing, bow,
// get-hit) on a mobile. We stash it on the entity's anim state; drawMobs plays group
// `act` once over its frames, then reverts to the idle/walk pose.
function ingestAnims(s) {
  if (!s || !s.anims) return;
  const now = performance.now();
  const pserial = s.player ? (s.player.serial >>> 0) : 0;
  for (const ev of s.anims) {
    const seq = ev.seq | 0;
    if (seq <= lastAnimSeq) continue;
    lastAnimSeq = seq;
    const serial = ev.serial >>> 0;
    const id = serial === pserial ? "self" : "m" + serial;
    const st = anim.get(id);
    if (!st) continue;                               // actor not in view
    st.act = { group: ev.act | 0, fwd: ev.fwd !== false, startMs: now,
               frameMs: CHAR_ANIM_FRAME_MS + (ev.delay | 0) * 10 };
    markDirty();
  }
}

function ingestDamage(s) {
  if (!s || !s.damage) return;
  const now = performance.now();
  const pserial = s.player ? (s.player.serial >>> 0) : 0;
  for (const ev of s.damage) {
    const seq = ev.seq | 0;
    if (seq <= lastDamageSeq) continue;
    lastDamageSeq = seq;
    const serial = ev.serial >>> 0;
    const isSelf = serial === pserial;
    const id = isSelf ? "self" : "m" + serial;
    if (!anim.has(id)) continue;                     // target not in view
    addDamageFloater(id, ev.amt | 0, isSelf, now);
  }
}

function addDamageFloater(id, amt, isSelf, now) {
  if (!settings.damage) return;            // damage numbers disabled in Options
  const el = document.createElement("div");
  el.className = "dmg-label" + (isSelf ? " self" : "");
  el.textContent = "-" + amt;
  namesEl().appendChild(el);
  damageFloaters.push({ id, el, born: now, ttl: DAMAGE_TTL });
  while (damageFloaters.length > 40) { const o = damageFloaters.shift(); if (o.el) o.el.remove(); }
}

// Position each damage number over its target, rising and fading over its life.
function drawDamage(now) {
  const fx = window.innerWidth / app.renderer.width, fy = window.innerHeight / app.renderer.height;
  for (let i = damageFloaters.length - 1; i >= 0; i--) {
    const o = damageFloaters[i];
    const age = now - o.born;
    if (age >= o.ttl) { if (o.el) o.el.remove(); damageFloaters.splice(i, 1); continue; }
    const st = anim.get(o.id);
    if (!st) { if (o.el) o.el.style.display = "none"; continue; } // target left view
    o.el.style.display = "block";
    const t = age / o.ttl;                            // 0..1 through its life
    o.el.style.left = ((app.stage.x + isoX(st.rx, st.ry)) * fx) + "px";
    o.el.style.top = ((app.stage.y + isoY(st.rx, st.ry, st.rz ?? st.z) - OVERHEAD_HEAD - 18 - t * DAMAGE_RISE) * fy) + "px";
    o.el.style.opacity = t > 0.5 ? Math.max(0, 1 - (t - 0.5) * 2) : 1; // fade over the back half
  }
}

// ---- graphical effects (0x70/0xC0/0xC7) ----------------------------------
// Resolve a world-tile position for an effect endpoint: prefer a live entity (so
// a fixed/target effect tracks it as it moves), else null (caller falls back to
// the packet's tile coords).
function fxEntityPos(serial, pserial) {
  serial = serial >>> 0;
  if (!serial) return null;
  const id = serial === pserial ? "self" : "m" + serial;
  const st = anim.get(id);
  if (!st) return null;
  return { x: st.rx, y: st.ry, z: st.rz ?? st.z ?? 0 };
}

// Spawn an animated sprite for each effect newer than the last we saw.
function ingestEffects(s) {
  if (!s || !s.effects) return;
  const now = performance.now();
  for (const ev of s.effects) {
    const seq = ev.seq | 0;
    if (seq <= lastEffectSeq) continue;
    lastEffectSeq = seq;
    spawnEffect(ev, now);
  }
}

function spawnEffect(ev, now) {
  const frames = (ev.frames && ev.frames.length) ? ev.frames : [ev.g | 0];
  const hue = ev.hue | 0;
  // animdata interval is a small tick count; clamp to a lively per-frame range.
  const fm = (ev.interval | 0) > 0 ? Math.min(150, Math.max(50, (ev.interval | 0) * 50)) : 80;
  const cycleMs = frames.length * fm;
  const pserial = (scene && scene.player) ? (scene.player.serial >>> 0) : 0;

  // Endpoints: a live entity if we can see it, else the packet's tile coords.
  const srcPos = fxEntityPos(ev.src, pserial) || { x: ev.sx, y: ev.sy, z: ev.sz | 0 };
  const tgtPos = fxEntityPos(ev.tgt, pserial) || { x: ev.tx, y: ev.ty, z: ev.tz | 0 };

  let totalMs;
  if (ev.kind === 0) {
    // Moving projectile: lifetime = travel time, scaled by distance + speed
    // (an approximation of ClassicUO's MovingEffect pacing).
    const dist = Math.hypot(tgtPos.x - srcPos.x, tgtPos.y - srcPos.y);
    totalMs = Math.min(2000, Math.max(150, dist * (40 + (ev.speed | 0) * 8)));
  } else if (ev.kind === 1) {
    totalMs = Math.max(250, cycleMs); // lightning: one quick flash at the target
  } else {
    // Fixed (2 FixedXYZ / 3 FixedFrom): loop for `dur` repeats, bounded so it
    // always cleans up.
    const reps = (ev.dur | 0) > 0 ? (ev.dur | 0) : 1;
    totalMs = Math.min(2500, Math.max(cycleMs, reps * fm));
  }

  const sprite = new PIXI.Sprite();
  sprite.anchor.set(0.5, 1.0); // foot-anchored like statics; hue baked via ?hue=
  overLayer.addChild(sprite);
  fxEffects.push({ kind: ev.kind | 0, src: ev.src >>> 0, tgt: ev.tgt >>> 0,
    frames, fm, hue, born: now, totalMs, sprite, srcPos, tgtPos, pserial });
  // Bound the pool so a burst of effects can't leak sprites.
  while (fxEffects.length > 48) { const o = fxEffects.shift(); overLayer.removeChild(o.sprite); o.sprite.destroy(); }
  markDirty();
}

// Animate + position each active effect; expire (and free) when its life ends.
function drawEffects(now) {
  for (let i = fxEffects.length - 1; i >= 0; i--) {
    const o = fxEffects[i];
    const age = now - o.born;
    if (age >= o.totalMs) { overLayer.removeChild(o.sprite); o.sprite.destroy(); fxEffects.splice(i, 1); continue; }

    let px, py, pz;
    if (o.kind === 0) {
      // Moving: interpolate source → target over the travel time.
      const t = Math.min(1, age / o.totalMs);
      px = o.srcPos.x + (o.tgtPos.x - o.srcPos.x) * t;
      py = o.srcPos.y + (o.tgtPos.y - o.srcPos.y) * t;
      pz = o.srcPos.z + (o.tgtPos.z - o.srcPos.z) * t;
    } else if (o.kind === 3) {
      // FixedFrom: follow the target entity, else the source, else its tile.
      const p = fxEntityPos(o.tgt, o.pserial) || fxEntityPos(o.src, o.pserial) || o.tgtPos;
      px = p.x; py = p.y; pz = p.z;
    } else if (o.kind === 1) {
      px = o.tgtPos.x; py = o.tgtPos.y; pz = o.tgtPos.z; // lightning at target
    } else {
      px = o.srcPos.x; py = o.srcPos.y; pz = o.srcPos.z; // FixedXYZ at source
    }

    // Cycle the resolved ART frame list (hue baked server-side via ?hue=).
    const g = o.frames[Math.floor(age / o.fm) % o.frames.length] | 0;
    const tex = texFor(`art/static/${g}.png` + (o.hue ? `?hue=${o.hue}` : ""));
    if (tex && o.sprite.texture !== tex) o.sprite.texture = tex;
    o.sprite.visible = !!o.sprite.texture && o.sprite.texture !== PIXI.Texture.EMPTY;

    o.sprite.x = isoX(px, py);
    o.sprite.y = isoY(px, py, pz) + HALF;
    const t = age / o.totalMs;
    o.sprite.alpha = t > 0.66 ? Math.max(0, 1 - (t - 0.66) * 3) : 1; // fade out the tail
  }
}

// ---- overhead name + HP bar (ClassicUO health-bar-over-head) ----
const nameDivs = new Map(); // id -> DOM div (crisp name label, pruned on leave)
function namesEl() {
  let el = document.getElementById("names");
  if (!el) { el = document.createElement("div"); el.id = "names"; document.body.appendChild(el); }
  return el;
}
const hpBars = new Map();    // id -> PIXI.Graphics
const tgtMarkers = new Map(); // id -> PIXI.Graphics (current attack-target marker)
const BAR_W = 30, BAR_H = 4; // health bar size
const BAR_HEAD = 40;         // px above the feet anchor for the bar (below the speech)
const BAR_FONT = 'ui-monospace, Menlo, "Apple SD Gothic Neo", "Malgun Gothic", sans-serif';
// Fill color by remaining health fraction (green → yellow → red), ClassicUO-style.
function hpColor(f) { return f > 0.5 ? 0x46a758 : f > 0.25 ? 0xd9a441 : 0xe5484d; }

// Draw a name + HP bar above each OTHER mobile, anchored to its interpolated iso
// position (like the overhead speech). Objects are cached per serial and only
// redrawn when their value/notoriety changes; pruned when the mobile leaves view.
function drawBars(now) {
  if (!scene) return;
  const seen = new Set();
  let changed = false;
  const lastAttack = (scene.lastAttack | 0) >>> 0; // current auto-attack target (0 = none)
  for (const m of scene.mobiles || []) {
    const id = "m" + m.serial;
    const st = anim.get(id);
    if (!st) continue;                       // not yet interpolated / left view
    const x = isoX(st.rx, st.ry);
    const feetY = isoY(st.rx, st.ry, st.rz ?? st.z);
    const topY = feetY - BAR_HEAD;       // name + target marker: above the head
    const barY = feetY + 2;              // HP bar: down at the feet (ClassicUO-style)
    // Is this the current attack target? Highlight its bar + draw a marker.
    const tgt = lastAttack !== 0 && (m.serial >>> 0) === lastAttack;
    // --- HP bar (only when the server gave us hits/hitsMax) ---
    if (settings.bars && (m.hitsMax | 0) > 0) {
      seen.add(id);
      let g = hpBars.get(id);
      if (!g) { g = new PIXI.Graphics(); barLayer.addChild(g); hpBars.set(id, g); changed = true; }
      const frac = Math.max(0, Math.min(1, m.hits / m.hitsMax));
      if (g._frac !== frac || g._noto !== m.noto || g._tgt !== tgt) {
        g.clear();
        // dark backing + notoriety-tinted border, then the health fill. The current
        // target gets a thicker bright-red border so it stands out.
        g.rect(-BAR_W / 2 - 1, -1, BAR_W + 2, BAR_H + 2).fill({ color: 0x000000, alpha: 0.6 })
         .stroke({ color: tgt ? 0xff2d2d : notoColor(m.noto), width: tgt ? 2 : 1 });
        if (frac > 0) g.rect(-BAR_W / 2, 0, BAR_W * frac, BAR_H).fill(hpColor(frac));
        g._frac = frac; g._noto = m.noto; g._tgt = tgt;
        changed = true;
      }
      g.x = x; g.y = barY; g.visible = true;
    } else {
      const g = hpBars.get(id); if (g) g.visible = false;
    }
    // --- target marker (red diamond above the target; works even with no HP bar) ---
    if (tgt) {
      seen.add(id);
      let mk = tgtMarkers.get(id);
      if (!mk) {
        mk = new PIXI.Graphics();
        mk.poly([0, -5, 5, 0, 0, 5, -5, 0]).fill(0xff2d2d).stroke({ color: 0x000000, width: 1 });
        barLayer.addChild(mk); tgtMarkers.set(id, mk); changed = true;
      }
      mk.x = x; mk.y = topY - 14; mk.visible = true;
    } else {
      const mk = tgtMarkers.get(id); if (mk) mk.visible = false;
    }
    // --- name: a DOM label, NOT PIXI text. The game canvas renders at a capped
    // internal resolution and is nearest-neighbour upscaled to fill the window, so
    // any in-canvas text comes out enlarged/blocky. A DOM overlay is always crisp at
    // the native display resolution. We place it at the entity's *screen* position
    // (camera transform + the canvas→CSS stretch). "no draw" placeholders are skipped.
    const nm = (m.name || "").trim();
    if (settings.names && nm && !/^no\s*draw$/i.test(nm)) {
      seen.add(id);
      let d = nameDivs.get(id);
      if (!d) { d = document.createElement("div"); d.className = "nm-label"; namesEl().appendChild(d); nameDivs.set(id, d); }
      if (d._t !== nm) { d.textContent = nm; d._t = nm; }
      if (d._noto !== m.noto) { d.style.color = cssColor(notoColor(m.noto)); d._noto = m.noto; }
      const fx = window.innerWidth / app.renderer.width, fy = window.innerHeight / app.renderer.height;
      d.style.left = ((app.stage.x + x) * fx) + "px";
      d.style.top = ((app.stage.y + topY - 2) * fy) + "px";
      d.style.display = "block";
    } else {
      const d = nameDivs.get(id); if (d) d.style.display = "none";
    }
  }
  // Prune name/bar objects whose mobile left view (don't leak PIXI objects / DOM).
  for (const [id, g] of hpBars) if (!seen.has(id)) { barLayer.removeChild(g); g.destroy(); hpBars.delete(id); changed = true; }
  for (const [id, d] of nameDivs) if (!seen.has(id)) { d.remove(); nameDivs.delete(id); }
  for (const [id, mk] of tgtMarkers) if (!seen.has(id)) { barLayer.removeChild(mk); mk.destroy(); tgtMarkers.delete(id); changed = true; }
  if (changed) markDirty(); // first appearance / value change → repaint once
}

function hud(s) {
  const p = s.player;
  // Show the *predicted* tile (what the avatar is visually standing on) so the
  // coordinate readout matches the on-screen position instead of the server pos,
  // which lags ~poll+confirm behind during movement.
  const lx = pred ? Math.round(pred.rx) : p.x, ly = pred ? Math.round(pred.ry) : p.y;
  set("pname", p.name || "(unnamed)"); set("ppos", `(${lx}, ${ly}, ${p.z})`);
  bar("hp", p.hits, p.hitsMax); bar("mana", p.mana, p.manaMax); bar("stam", p.stam, p.stamMax);
  set("stats", `${p.str} / ${p.dex} / ${p.int}`); set("gold", p.gold);
  // War-mode indicator (Tab toggles): reflect the server's authoritative flag.
  const wi = document.getElementById("warind");
  if (wi) {
    const war = !!s.war;
    wi.textContent = war ? "WAR" : "PEACE";
    wi.className = war ? "war" : "peace";
  }
  const j = document.getElementById("journal");
  // Keep following the newest line only if already scrolled to the bottom (don't
  // yank the view while the user is reading back).
  const atBottom = j.scrollHeight - j.scrollTop - j.clientHeight < 24;
  j.innerHTML = "";
  for (const line of s.journal || []) {
    if (!line.text) continue;
    const d = document.createElement("div");
    d.textContent = line.name ? `${line.name}: ${line.text}` : line.text;
    j.appendChild(d);
  }
  // Client-side system notices (skill gains/losses) after the server lines.
  for (const line of localJournal) {
    const d = document.createElement("div");
    d.className = "jrnl-sys";
    d.textContent = line.text;
    j.appendChild(d);
  }
  if (atBottom) j.scrollTop = j.scrollHeight;
  refreshStatus(s);   // keep the pull-out status bar live (if open)
}
function updateDiag() {
  set("diag", `fps ${diag.fps} · poll ${diag.poll.toFixed(0)}ms · sync ${diag.sync.toFixed(1)}ms · sprites ${diag.tiles} · ents ${diag.ents} · worst ${diag.worstFrame.toFixed(0)}ms`);
  diag.worstFrame = 0;
}
function bar(id, c, m) { document.getElementById(id).style.width = (m > 0 ? Math.round((c / m) * 100) : 0) + "%"; }
function set(id, v) { const el = document.getElementById(id); if (el) el.textContent = v; }
function setStatus(t) { set("status", t); }

// ---- paperdoll + container windows (HTML "gump" overlays over the canvas) ----
// These are plain DOM panels (divs over the PixiJS canvas), not sprites — gump-style
// chrome is far simpler/safer in HTML than in the render loop. They READ `scene`
// each poll (refresh*) and only act via sendInput(); they never touch movement/render.
const EQUIP_SLOTS = {
  1: "Right Hand", 2: "Left Hand", 3: "Shoes", 4: "Pants", 5: "Shirt", 6: "Head",
  7: "Gloves", 8: "Ring", 9: "Talisman", 10: "Neck", 11: "Hair", 12: "Waist",
  13: "Torso", 14: "Bracelet", 15: "Face", 16: "Beard", 17: "Tunic", 18: "Earrings", 19: "Arms",
  20: "Cloak", 21: "Backpack", 22: "Robe", 23: "OuterLegs", 24: "InnerLegs",
};
const BACKPACK_LAYER = 21;
// Paperdoll draw order (back → front), by UO layer number — ClassicUO
// PaperDollInteractable._layerOrder. The worn item's paperdoll gump is its
// AnimID + a gender offset; each gump is a full doll-canvas image stacked at the
// same origin. Weapons (1/2) are included so held items show on the doll.
const PAPERDOLL_ORDER = [20, 5, 4, 3, 24, 19, 13, 17, 8, 14, 15, 7, 23, 22, 12, 10, 11, 16, 18, 6, 1, 2, 9];
const MALE_GUMP_OFFSET = 50000, FEMALE_GUMP_OFFSET = 60000;

// Bring a gump window to the top by moving it to the end of <body> (all gumps share
// the same z-index, so DOM order decides paint order). Keeps them below the modal
// world map (z 20) and above the HUD.
function bringToFront(el) { document.body.appendChild(el); }

// Drag a window by its title bar; clamp so it never fully leaves the viewport.
function makeDraggable(win, handle) {
  handle.addEventListener("mousedown", (e) => {
    if (e.target.classList.contains("gump-close")) return; // let the ✕ click through
    e.preventDefault();
    bringToFront(win);
    const r = win.getBoundingClientRect();
    const dx = e.clientX - r.left, dy = e.clientY - r.top;
    const move = (ev) => {
      const x = Math.max(0, Math.min(window.innerWidth - 40, ev.clientX - dx));
      const y = Math.max(0, Math.min(window.innerHeight - 24, ev.clientY - dy));
      win.style.left = x + "px"; win.style.top = y + "px"; win.style.right = "auto";
    };
    const up = () => { window.removeEventListener("mousemove", move); window.removeEventListener("mouseup", up); };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  });
}

// --- paperdoll (toggled by the 'P' key; ✕/Esc close) ---
// pdTarget: null = our own doll; a serial = another mobile's doll (double-click an
// NPC/player to inspect their equipment, ClassicUO-style).
let paperdollOn = false;
let pdTarget = null;
function togglePaperdoll() {
  // 'P' always shows OUR doll (switch back from any inspected mobile).
  if (paperdollOn && pdTarget == null) { closePaperdoll(); return; }
  pdTarget = null;
  paperdollOn = true;
  const pd = document.getElementById("paperdoll");
  pd.classList.add("on"); pd._sig = null;
  refreshPaperdoll();
}
// Open another mobile's paperdoll (double-clicked in the world).
function openMobilePaperdoll(serial) {
  pdTarget = serial >>> 0;
  paperdollOn = true;
  const pd = document.getElementById("paperdoll");
  pd.classList.add("on"); pd._sig = null;
  refreshPaperdoll();
}
function closePaperdoll() {
  paperdollOn = false;
  pdTarget = null;
  document.getElementById("paperdoll").classList.remove("on");
}
// --- weapon special-ability bar (bottom-left; click to arm/disarm) ---
// Two buttons = the equipped weapon's primary/secondary moves. Clicking sends
// 0xD7 UseCombatAbility (Action::UseAbility → `sendInput("ability:"+id)`) with the
// move's `Ability` enum id; clicking the armed one again disarms (sends 0). The
// server actually arms/disarms the next swing — we just mirror the highlight.
// Names + weapon→ability table ported from ClassicUO Game/Data/Ability.cs.
const ABILITY_NAMES = {
  1: "Armor Ignore", 2: "Bleed Attack", 3: "Concussion Blow", 4: "Crushing Blow",
  5: "Disarm", 6: "Dismount", 7: "Double Strike", 8: "Infectious Strike",
  9: "Mortal Strike", 10: "Moving Shot", 11: "Paralyzing Blow", 12: "Shadow Strike",
  13: "Whirlwind Attack", 14: "Riding Swipe", 15: "Frenzied Whirlwind", 16: "Block",
  17: "Defense Mastery", 18: "Nerve Strike", 19: "Talon Strike", 20: "Feint",
  21: "Dual Wield", 22: "Double Shot", 23: "Armor Pierce", 24: "Bladeweave",
  25: "Force Arrow", 26: "Lightning Arrow", 27: "Psychic Attack", 28: "Serpent Arrow",
  29: "Force of Nature", 30: "Infused Throw", 31: "Mystic Arc",
};
// weapon graphic → [primaryAbilityId, secondaryAbilityId]
const WEAPON_ABILITIES = {
  0x0901:[10,30], 0x0902:[8,12], 0x0905:[7,9], 0x0906:[4,6], 0x090C:[2,9], 0x0DF0:[13,11],
  0x0DF1:[13,11], 0x0DF2:[6,5], 0x0DF3:[6,5], 0x0DF4:[6,5], 0x0DF5:[6,5], 0x0E81:[4,5],
  0x0E82:[4,5], 0x0E85:[7,5], 0x0E86:[7,5], 0x0E87:[2,6], 0x0E88:[2,6], 0x0E89:[7,3],
  0x0E8A:[7,3], 0x0EC2:[2,8], 0x0EC3:[2,8], 0x0EC4:[12,2], 0x0EC5:[12,2], 0x0F43:[1,5],
  0x0F44:[1,5], 0x0F45:[2,9], 0x0F46:[2,9], 0x0F47:[2,3], 0x0F48:[2,3], 0x0F49:[4,6],
  0x0F4A:[4,6], 0x0F4B:[7,13], 0x0F4C:[7,13], 0x0F4D:[11,6], 0x0F4E:[11,6], 0x0F4F:[3,9],
  0x0F50:[3,9], 0x0F51:[8,12], 0x0F52:[8,12], 0x0F5C:[3,5], 0x0F5D:[3,5], 0x0F5E:[4,1],
  0x0F5F:[4,1], 0x0F60:[1,3], 0x0F61:[1,3], 0x0F62:[1,11], 0x0F63:[1,11], 0x0FB5:[4,12],
  0x13AF:[1,2], 0x13B0:[1,2], 0x13B1:[11,9], 0x13B2:[11,9], 0x13B3:[12,6], 0x13B4:[12,6],
  0x13B6:[7,11], 0x13B7:[7,11], 0x13B8:[7,11], 0x13B9:[11,4], 0x13BA:[11,4], 0x13FD:[10,6],
  0x13E3:[4,12], 0x13F6:[8,5], 0x13F8:[3,29], 0x13FB:[13,2], 0x13FF:[7,1], 0x1401:[1,8],
  0x1402:[12,9], 0x1403:[12,9], 0x1404:[2,5], 0x1405:[2,5], 0x1406:[4,9], 0x1407:[4,9],
  0x1438:[13,4], 0x1439:[13,4], 0x143A:[7,3], 0x143B:[7,3], 0x143C:[1,9], 0x143D:[1,9],
  0x143E:[13,3], 0x143F:[13,3], 0x1440:[2,12], 0x1441:[2,12], 0x1442:[7,12], 0x1443:[7,12],
  0x26BA:[2,11], 0x26BB:[11,9], 0x26BC:[4,9], 0x26BD:[1,6], 0x26BE:[11,8], 0x26BF:[7,8],
  0x26C0:[6,3], 0x26C1:[7,9], 0x26C2:[1,10], 0x26C3:[7,10], 0x26C4:[2,11], 0x26C5:[11,9],
  0x26C6:[4,9], 0x26C7:[1,6], 0x26C8:[11,8], 0x26C9:[7,8], 0x26CA:[6,3], 0x26CB:[7,9],
  0x26CC:[1,10], 0x26CD:[7,10], 0x26CE:[13,5], 0x26CF:[13,5], 0x27A2:[4,14], 0x27A3:[20,16],
  0x27A4:[15,7], 0x27A5:[23,22], 0x27A6:[15,4], 0x27A7:[17,15], 0x27A8:[20,18], 0x27A9:[20,7],
  0x27AA:[5,11], 0x27AB:[21,19], 0x27AD:[13,17], 0x27AE:[16,20], 0x27AF:[16,23], 0x27ED:[4,14],
  0x27EE:[20,16], 0x27EF:[15,7], 0x27F0:[23,22], 0x27F1:[15,4], 0x27F2:[17,15], 0x27F3:[20,18],
  0x27F4:[20,7], 0x27F5:[5,11], 0x27F6:[21,19], 0x27F8:[13,17], 0x27F9:[16,20], 0x27FA:[16,23],
  0x2D1E:[25,28], 0x2D1F:[26,27], 0x2D20:[27,2], 0x2D21:[8,12], 0x2D22:[20,1], 0x2D23:[5,24],
  0x2D24:[3,4], 0x2D25:[16,29], 0x2D26:[5,24], 0x2D27:[13,24], 0x2D28:[5,4], 0x2D29:[17,24],
  0x2D2A:[25,28], 0x2D2B:[26,27], 0x2D2C:[27,2], 0x2D2D:[8,12], 0x2D2E:[20,1], 0x2D2F:[5,24],
  0x2D30:[3,4], 0x2D31:[16,29], 0x2D32:[5,24], 0x2D33:[13,24], 0x2D34:[5,4], 0x2D35:[17,24],
  0x4067:[31,3], 0x08FD:[7,8], 0x4068:[7,8], 0x406B:[1,9], 0x406C:[10,30], 0x0904:[7,5],
  0x406D:[7,5], 0x0903:[1,5], 0x406E:[1,5], 0x08FE:[2,11], 0x4072:[2,11], 0x090B:[4,3],
  0x4074:[4,3], 0x0908:[13,6], 0x4075:[13,6], 0x4076:[1,9], 0x48AE:[2,8], 0x48B0:[2,3],
  0x48B3:[4,6], 0x48B2:[4,6], 0x48B5:[11,6], 0x48B4:[11,6], 0x48B7:[8,5], 0x48B6:[8,5],
  0x48B9:[3,11], 0x48B8:[3,11], 0x48BB:[7,1], 0x48BA:[7,1], 0x48BD:[1,8], 0x48BC:[1,8],
  0x48BF:[2,5], 0x48BE:[2,5], 0x48CB:[6,3], 0x48CA:[6,3], 0x0481:[13,4], 0x48C0:[13,4],
  0x48C3:[7,3], 0x48C2:[7,3], 0x48C5:[2,11], 0x48C4:[2,11], 0x48C7:[11,9], 0x48C6:[11,9],
  0x48C9:[11,8], 0x48C8:[11,8], 0x48CC:[20,16], 0x48CD:[20,16], 0x48CF:[21,19], 0x48CE:[21,19],
  0x48D1:[20,7], 0x48D0:[20,7], 0xA289:[3,13], 0xA291:[3,13], 0xA28A:[23,13], 0xA292:[23,13],
  0xA28B:[2,13], 0xA293:[2,13], 0x08FF:[31,3], 0x0900:[1,11], 0x090A:[1,9], 0xAEA5:[7,1],
  0xAEB4:[7,1], 0xAEC3:[7,1], 0xAED2:[7,1], 0xAEA4:[7,13], 0xAEB3:[7,13], 0xAEC2:[7,13],
  0xAED1:[7,13],
};
const WEAPON_LAYERS = [1, 2]; // right hand / left hand (two-handed weapons sit on 1)
let armedAbility = 0; // the locally-armed ability id (0 = none); mirrors the server arm
// Find the equipped weapon's [primaryId, secondaryId], or null if none/unmapped.
function equippedWeaponAbilities() {
  const p = scene && scene.player;
  if (!p || !p.equip) return null;
  for (const layer of WEAPON_LAYERS) {
    const w = p.equip.find((e) => (e.layer | 0) === layer);
    if (w) return WEAPON_ABILITIES[w.g & 0xFFFF] || "unknown";
  }
  return null;
}
function clickAbility(id) {
  if (!id) return;
  // ClassicUO toggle: clicking the armed move disarms (send 0), else arm it.
  if (armedAbility === id) { armedAbility = 0; sendInput("ability:0"); }
  else { armedAbility = id; sendInput("ability:" + id); }
  refreshAbilities(true);
}
// Rebuild the two-button bar from the equipped weapon (called each poll).
function refreshAbilities(force) {
  const bar = document.getElementById("abilities");
  if (!bar) return;
  // Weapon special abilities are an AOS feature. Show the bar only when the server
  // advertised AOS in SupportedFeatures (0xB9) AND the user hasn't hidden it in
  // Options — a T2A/pre-AOS shard has no such moves. (Note: a shard that enables
  // Core.AOS server-side, e.g. for OPL tooltips, will advertise AOS=true here.)
  // T2A explicitly hides the bar regardless of the AOS flag (see `T2A` const).
  if (T2A || !(scene && scene.aos) || !settings.abilities) { bar.style.display = "none"; return; }
  bar.style.display = "flex";
  const ab = equippedWeaponAbilities();
  // ab: null = no weapon → generic ids 0/1 ; "unknown" = weapon not in table ;
  // [p,s] = real moves. (ids 0/1 let the server pick based on the equipped weapon.)
  let prim, sec, primName, secName;
  if (Array.isArray(ab)) {
    [prim, sec] = ab;
    primName = ABILITY_NAMES[prim] || "Primary";
    secName = ABILITY_NAMES[sec] || "Secondary";
  } else {
    prim = 0; sec = 1; primName = "Primary"; secName = "Secondary";
  }
  const sig = `${prim}:${sec}:${armedAbility}`;
  if (!force && bar._sig === sig) return;
  bar._sig = sig;
  bar.innerHTML =
    `<div class="abil-hdr">Weapon Abilities</div>` +
    `<div class="abil-btn${armedAbility && armedAbility === prim ? " armed" : ""}" data-id="${prim}">` +
    `<span>${primName}</span><span class="ak">PRI</span></div>` +
    `<div class="abil-btn${armedAbility && armedAbility === sec ? " armed" : ""}" data-id="${sec}">` +
    `<span>${secName}</span><span class="ak">SEC</span></div>`;
  for (const el of bar.querySelectorAll(".abil-btn"))
    el.addEventListener("click", () => clickAbility(Number(el.dataset.id)));
}

// --- spellbook (all schools, toggled by the 'K' key; ✕/Esc close) ---
// The cast packet (0xBF/0x1C, Action::CastSpell) takes a GLOBAL spell id, so every
// school is the same mechanism — `sendInput("cast:" + id)`. Ids/names ported from
// ClassicUO src/ClassicUO.Client/Game/Data/Spells*.cs.
// 64 Magery spells, ids 1..64 = (circle-1)*8 + position. Names in id order.
const MAGERY_SPELLS = [
  "Clumsy", "Create Food", "Feeblemind", "Heal", "Magic Arrow", "Night Sight",
  "Reactive Armor", "Weaken", "Agility", "Cunning", "Cure", "Harm", "Magic Trap",
  "Magic Untrap", "Protection", "Strength", "Bless", "Fireball", "Magic Lock",
  "Poison", "Telekinesis", "Teleport", "Unlock", "Wall of Stone", "Arch Cure",
  "Arch Protection", "Curse", "Fire Field", "Greater Heal", "Lightning",
  "Mana Drain", "Recall", "Blade Spirits", "Dispel Field", "Incognito",
  "Magic Reflection", "Mind Blast", "Paralyze", "Poison Field", "Summon Creature",
  "Dispel", "Energy Bolt", "Explosion", "Invisibility", "Mark", "Mass Curse",
  "Paralyze Field", "Reveal", "Chain Lightning", "Energy Field", "Flamestrike",
  "Gate Travel", "Mana Vampire", "Mass Dispel", "Meteor Swarm", "Polymorph",
  "Earthquake", "Energy Vortex", "Resurrection", "Air Elemental", "Summon Daemon",
  "Earth Elemental", "Fire Elemental", "Water Elemental",
];
// Other schools as [globalId, name] (ported from Spells*.cs).
const NECROMANCY_SPELLS = [
  [101, "Animate Dead"], [102, "Blood Oath"], [103, "Corpse Skin"], [104, "Curse Weapon"],
  [105, "Evil Omen"], [106, "Horrific Beast"], [107, "Lich Form"], [108, "Mind Rot"],
  [109, "Pain Spike"], [110, "Poison Strike"], [111, "Strangle"], [112, "Summon Familiar"],
  [113, "Vampiric Embrace"], [114, "Vengeful Spirit"], [115, "Wither"], [116, "Wraith Form"],
  [117, "Exorcism"],
];
const CHIVALRY_SPELLS = [
  [201, "Cleanse by Fire"], [202, "Close Wounds"], [203, "Consecrate Weapon"],
  [204, "Dispel Evil"], [205, "Divine Fury"], [206, "Enemy of One"], [207, "Holy Light"],
  [208, "Noble Sacrifice"], [209, "Remove Curse"], [210, "Sacred Journey"],
];
const BUSHIDO_SPELLS = [
  [401, "Honorable Execution"], [402, "Confidence"], [403, "Evasion"],
  [404, "Counter Attack"], [405, "Lightning Strike"], [406, "Momentum Strike"],
];
const NINJITSU_SPELLS = [
  [501, "Focus Attack"], [502, "Death Strike"], [503, "Animal Form"], [504, "Ki Attack"],
  [505, "Surprise Attack"], [506, "Backstab"], [507, "Shadowjump"], [508, "Mirror Image"],
];
const SPELLWEAVING_SPELLS = [
  [601, "Arcane Circle"], [602, "Gift of Renewal"], [603, "Immolating Weapon"],
  [604, "Attunement"], [605, "Thunderstorm"], [606, "Nature's Fury"], [607, "Summon Fey"],
  [608, "Summon Fiend"], [609, "Reaper Form"], [610, "Wildfire"], [611, "Essence of Wind"],
  [612, "Dryad Allure"], [613, "Ethereal Voyage"], [614, "Word of Death"],
  [615, "Gift of Life"], [616, "Arcane Empowerment"],
];
const MYSTICISM_SPELLS = [
  [678, "Nether Bolt"], [679, "Healing Stone"], [680, "Purge Magic"], [681, "Enchant"],
  [682, "Sleep"], [683, "Eagle Strike"], [684, "Animated Weapon"], [685, "Stone Form"],
  [686, "Spell Trigger"], [687, "Mass Sleep"], [688, "Cleansing Winds"], [689, "Bombard"],
  [690, "Spell Plague"], [691, "Hail Storm"], [692, "Nether Cyclone"], [693, "Rising Colossus"],
];
const MASTERY_SPELLS = [
  [701, "Inspire"], [702, "Invigorate"], [703, "Resilience"], [704, "Perseverance"],
  [705, "Tribulation"], [706, "Despair"], [707, "Death Ray"], [708, "Ethereal Burst"],
  [709, "Nether Blast"], [710, "Mystic Weapon"], [711, "Command Undead"], [712, "Conduit"],
  [713, "Mana Shield"], [714, "Summon Reaper"], [715, "Enchanted Summoning"],
  [716, "Anticipate Hit"], [717, "Warcry"], [718, "Intuition"], [719, "Rejuvenate"],
  [720, "Holy Fist"], [721, "Shadow"], [722, "White Tiger Form"], [723, "Flaming Shot"],
  [724, "Playing The Odds"], [725, "Thrust"], [726, "Pierce"], [727, "Stagger"],
  [728, "Toughness"], [729, "Onslaught"], [730, "Focused Eye"], [731, "Elemental Fury"],
  [732, "Called Shot"], [733, "Warrior's Gifts"], [734, "Shield Bash"], [735, "Bodyguard"],
  [736, "Heighten Senses"], [737, "Tolerance"], [738, "Injected Strike"], [739, "Potency"],
  [740, "Rampage"], [741, "Fists of Fury"], [742, "Knockout"], [743, "Whispering"],
  [744, "Combat Training"], [745, "Boarding"],
];
// Each school renders as its own native ClassicUO book gump. `book` = the book
// background gump id; `iconStart` = the gump id of the school's first spell icon
// (icon for the k-th spell, k 0-based, = iconStart + k). Ids decimal (the gump
// endpoint /gump/<id>.png wants decimal). Source: SpellbookGump.cs GetBookInfo.
// `spells` is normalised to [globalId, name] for every school (Magery built below).
const MAGERY_PAIRS = MAGERY_SPELLS.map((name, i) => [i + 1, name]);
// Power words (mantra) + reagents per Magery spell, keyed by name. Source: ServUO
// Scripts/Spells/{First..Eighth}/*.cs SpellInfo (name, mantra, Reagent.*).
const MAGERY_INFO = {
  "Clumsy": ["Uus Jux", "Bloodmoss, Nightshade"],
  "Create Food": ["In Mani Ylem", "Garlic, Ginseng, Mandrake Root"],
  "Feeblemind": ["Rel Wis", "Ginseng, Nightshade"],
  "Heal": ["In Mani", "Garlic, Ginseng, Spider's Silk"],
  "Magic Arrow": ["In Por Ylem", "Sulfurous Ash"],
  "Night Sight": ["In Lor", "Sulfurous Ash, Spider's Silk"],
  "Reactive Armor": ["Flam Sanct", "Garlic, Spider's Silk, Sulfurous Ash"],
  "Weaken": ["Des Mani", "Garlic, Nightshade"],
  "Agility": ["Ex Uus", "Bloodmoss, Mandrake Root"],
  "Cunning": ["Uus Wis", "Mandrake Root, Nightshade"],
  "Cure": ["An Nox", "Garlic, Ginseng"],
  "Harm": ["An Mani", "Nightshade, Spider's Silk"],
  "Magic Trap": ["In Jux", "Garlic, Spider's Silk, Sulfurous Ash"],
  "Magic Untrap": ["An Jux", "Bloodmoss, Sulfurous Ash"],
  "Protection": ["Uus Sanct", "Garlic, Ginseng, Sulfurous Ash"],
  "Strength": ["Uus Mani", "Mandrake Root, Nightshade"],
  "Bless": ["Rel Sanct", "Garlic, Mandrake Root"],
  "Fireball": ["Vas Flam", "Black Pearl"],
  "Magic Lock": ["An Por", "Garlic, Bloodmoss, Sulfurous Ash"],
  "Poison": ["In Nox", "Nightshade"],
  "Telekinesis": ["Ort Por Ylem", "Bloodmoss, Mandrake Root"],
  "Teleport": ["Rel Por", "Bloodmoss, Mandrake Root"],
  "Unlock": ["Ex Por", "Bloodmoss, Sulfurous Ash"],
  "Wall of Stone": ["In Sanct Ylem", "Bloodmoss, Garlic"],
  "Arch Cure": ["Vas An Nox", "Garlic, Ginseng, Mandrake Root"],
  "Arch Protection": ["Vas Uus Sanct", "Garlic, Ginseng, Mandrake Root, Sulfurous Ash"],
  "Curse": ["Des Sanct", "Nightshade, Garlic, Sulfurous Ash"],
  "Fire Field": ["In Flam Grav", "Black Pearl, Spider's Silk, Sulfurous Ash"],
  "Greater Heal": ["In Vas Mani", "Garlic, Ginseng, Mandrake Root, Spider's Silk"],
  "Lightning": ["Por Ort Grav", "Mandrake Root, Sulfurous Ash"],
  "Mana Drain": ["Ort Rel", "Black Pearl, Mandrake Root, Spider's Silk"],
  "Recall": ["Kal Ort Por", "Black Pearl, Bloodmoss, Mandrake Root"],
  "Blade Spirits": ["In Jux Hur Ylem", "Black Pearl, Mandrake Root, Nightshade"],
  "Dispel Field": ["An Grav", "Black Pearl, Spider's Silk, Sulfurous Ash, Garlic"],
  "Incognito": ["Kal In Ex", "Bloodmoss, Garlic, Nightshade"],
  "Magic Reflection": ["In Jux Sanct", "Garlic, Mandrake Root, Spider's Silk"],
  "Mind Blast": ["Por Corp Wis", "Black Pearl, Mandrake Root, Nightshade, Sulfurous Ash"],
  "Paralyze": ["An Ex Por", "Garlic, Mandrake Root, Spider's Silk"],
  "Poison Field": ["In Nox Grav", "Black Pearl, Nightshade, Spider's Silk"],
  "Summon Creature": ["Kal Xen", "Bloodmoss, Mandrake Root, Spider's Silk"],
  "Dispel": ["An Ort", "Garlic, Mandrake Root, Sulfurous Ash"],
  "Energy Bolt": ["Corp Por", "Black Pearl, Nightshade"],
  "Explosion": ["Vas Ort Flam", "Bloodmoss, Mandrake Root"],
  "Invisibility": ["An Lor Xen", "Bloodmoss, Nightshade"],
  "Mark": ["Kal Por Ylem", "Black Pearl, Bloodmoss, Mandrake Root"],
  "Mass Curse": ["Vas Des Sanct", "Garlic, Nightshade, Mandrake Root, Sulfurous Ash"],
  "Paralyze Field": ["In Ex Grav", "Black Pearl, Ginseng, Spider's Silk"],
  "Reveal": ["Wis Quas", "Bloodmoss, Sulfurous Ash"],
  "Chain Lightning": ["Vas Ort Grav", "Black Pearl, Bloodmoss, Mandrake Root, Sulfurous Ash"],
  "Energy Field": ["In Sanct Grav", "Black Pearl, Mandrake Root, Spider's Silk, Sulfurous Ash"],
  "Flamestrike": ["Kal Vas Flam", "Spider's Silk, Sulfurous Ash"],
  "Gate Travel": ["Vas Rel Por", "Black Pearl, Mandrake Root, Sulfurous Ash"],
  "Mana Vampire": ["Ort Sanct", "Black Pearl, Bloodmoss, Mandrake Root, Spider's Silk"],
  "Mass Dispel": ["Vas An Ort", "Garlic, Mandrake Root, Black Pearl, Sulfurous Ash"],
  "Meteor Swarm": ["Flam Kal Des Ylem", "Bloodmoss, Mandrake Root, Sulfurous Ash, Spider's Silk"],
  "Polymorph": ["Vas Ylem Rel", "Bloodmoss, Spider's Silk, Mandrake Root"],
  "Earthquake": ["In Vas Por", "Bloodmoss, Ginseng, Mandrake Root, Sulfurous Ash"],
  "Energy Vortex": ["Vas Corp Por", "Bloodmoss, Black Pearl, Mandrake Root, Nightshade"],
  "Resurrection": ["An Corp", "Bloodmoss, Garlic, Ginseng"],
  "Air Elemental": ["Kal Vas Xen Hur", "Bloodmoss, Mandrake Root, Spider's Silk"],
  "Summon Daemon": ["Kal Vas Xen Corp", "Bloodmoss, Mandrake Root, Spider's Silk, Sulfurous Ash"],
  "Earth Elemental": ["Kal Vas Xen Ylem", "Bloodmoss, Mandrake Root, Spider's Silk"],
  "Fire Elemental": ["Kal Vas Xen Flam", "Bloodmoss, Mandrake Root, Spider's Silk, Sulfurous Ash"],
  "Water Elemental": ["Kal Vas Xen An Flam", "Bloodmoss, Mandrake Root, Spider's Silk"],
};
const SPELL_SCHOOLS = [
  { key: "magery", label: "Magery", book: 0x08AC, iconStart: 0x08C0, spells: MAGERY_PAIRS },
  { key: "necromancy", label: "Necro", book: 0x2B00, iconStart: 0x5000, spells: NECROMANCY_SPELLS },
  { key: "chivalry", label: "Chivalry", book: 0x2B01, iconStart: 0x5100, spells: CHIVALRY_SPELLS },
  { key: "bushido", label: "Bushido", book: 0x2B07, iconStart: 0x5400, spells: BUSHIDO_SPELLS },
  { key: "ninjitsu", label: "Ninjitsu", book: 0x2B06, iconStart: 0x5300, spells: NINJITSU_SPELLS },
  { key: "spellweaving", label: "Weaving", book: 0x2B2F, iconStart: 0x59D8, spells: SPELLWEAVING_SPELLS },
  { key: "mysticism", label: "Mysticism", book: 0x2B32, iconStart: 0x5DC0, spells: MYSTICISM_SPELLS },
  { key: "mastery", label: "Mastery", book: 0x08AC, iconStart: 0x0945, spells: MASTERY_SPELLS },
];
// T2A (The Second Age, pre-AOS) era: only Magery exists — Necromancy, Chivalry,
// Bushido, Ninjitsu, Spellweaving, Mysticism and Mastery are all later expansions.
// On a T2A shard hide every non-Magery school (no tab, not openable). Flip to false
// for a modern/AOS+ shard to expose all schools again. (scene.aos is NOT a reliable
// T2A signal — a T2A shard may still set Core.AOS server-side for OPL tooltips.)
const T2A = true;
const VISIBLE_SCHOOLS = T2A ? SPELL_SCHOOLS.filter((s) => s.key === "magery") : SPELL_SCHOOLS;
const SB_CORNER_L = 0x08BB;     // left page-turn corner gump (prev spread)
const SB_CORNER_R = 0x08BC;     // right page-turn corner gump (next spread)
// Two pages per spread, 8 spell rows per page (16 per spread); rows 44px apart,
// matching the book art. Left page icons at x=54, right page at x=221; first row
// y=48. Names sit to the right of each icon (flex row inside the entry).
const SB_PER_PAGE = 8, SB_ROW_H = 44, SB_ROW_Y0 = 48;
const SB_COL_X = [54, 221];
let spellbookOn = false;
let spellSchool = "magery";   // remembered across opens (module-scoped)
let spellPage = 0;            // current spread index (0-based)
// One overlaid spell entry: the spell's icon gump + its name. If the icon gump
// 404s (exotic schools), onerror collapses the <img> so only the name shows.
// Render the spell list (no book art): each spell shows its name, power words
// (mantra) and reagents — the classic in-fiction spellbook info. Magery is grouped
// by its 8 circles. Click a row to cast.
function renderSpellSchool() {
  const book = document.getElementById("sb-book");
  const school = VISIBLE_SCHOOLS.find((s) => s.key === spellSchool) || VISIBLE_SCHOOLS[0];
  const isMagery = school.key === "magery";
  let html = "", lastCircle = 0;
  school.spells.forEach(([id, name], idx) => {
    if (isMagery) {
      const circle = Math.floor(idx / 8) + 1;        // 8 spells per circle
      if (circle !== lastCircle) { html += `<div class="sp-circle">Circle ${circle}</div>`; lastCircle = circle; }
    }
    const info = isMagery ? MAGERY_INFO[name] : null;
    const iconId = school.iconStart + idx;           // k-th spell icon = iconStart + k
    // The icon is draggable out onto the screen → a floating quick-cast button.
    html += `<div class="sp-row" data-id="${id}" data-icon="${iconId}" data-name="${name}" title="Cast ${name}">`
      + `<img class="sp-icon" src="gump/${iconId}.png" alt="" draggable="true" crossorigin="anonymous"`
      + ` onerror="this.onerror=null;this.style.visibility='hidden'">`
      + `<div class="sp-txt"><div class="sp-name">${name}</div>`;
    if (info) {
      html += `<div class="sp-words">${info[0]}</div>`
        + `<div class="sp-reags">${info[1]}</div>`;
    }
    html += "</div></div>";
  });
  book.innerHTML = html;
  for (const t of document.querySelectorAll("#sb-tabs .sb-tab"))
    t.classList.toggle("sel", t.dataset.school === spellSchool);
}
function buildSpellbook() {
  const tabs = document.getElementById("sb-tabs");
  if (tabs.childElementCount) { renderSpellSchool(); return; }   // wire once
  tabs.innerHTML = VISIBLE_SCHOOLS.map((s) =>
    `<div class="sb-tab" data-school="${s.key}">${s.label}</div>`).join("");
  tabs.addEventListener("click", (e) => {
    const tab = e.target.closest(".sb-tab");
    if (!tab) return;
    spellSchool = tab.dataset.school;
    renderSpellSchool();
  });
  document.getElementById("sb-book").addEventListener("click", (e) => {
    const row = e.target.closest(".sp-row");
    if (!row) return;
    // Cast → server replies with a target cursor when the spell needs one; the
    // existing target UI (scene.target) lets the player click the target.
    sendInput("cast:" + row.dataset.id);
  });
  wireSpellDragOut();   // dragging a spell icon out spawns a quick-cast button
  renderSpellSchool();
}
function refreshSpellMana() {
  const p = scene && scene.player;
  const el = document.getElementById("sb-mana");
  if (el) el.textContent = p ? `Mana: ${p.mana | 0} / ${p.manaMax | 0}` : "Mana: —";
}
function toggleSpellbook() {
  spellbookOn = !spellbookOn;
  const sb = document.getElementById("spellbook");
  sb.classList.toggle("on", spellbookOn);
  if (spellbookOn) { buildSpellbook(); refreshSpellMana(); }
}
function closeSpellbook() {
  spellbookOn = false;
  document.getElementById("spellbook").classList.remove("on");
}

// --- skills window (0x3A, toggled by the 'L' key; ✕/Esc close) ---
// Lists every skill the server sent (scene.skills) with value/base/cap; the lock
// indicator cycles up(↑)/down(↓)/locked(🔒) on click → `skilllock:<id>:<next>`
// (0x3A SkillStatusChangeRequest). A ▸ affordance (or row double-click) invokes an
// active skill → `useskill:<id>` (0x12 ActionRequest type 0x24). Passive skills do
// nothing server-side, which is harmless. Names by id (0-based) from the standard
// UO skill table; unknown ids fall back to "Skill #id".
const SKILL_NAMES = [
  "Alchemy", "Anatomy", "Animal Lore", "Item ID", "Arms Lore", "Parrying", "Begging",
  "Blacksmithy", "Bowcraft/Fletching", "Peacemaking", "Camping", "Carpentry",
  "Cartography", "Cooking", "Detecting Hidden", "Discordance", "Evaluating Intelligence",
  "Healing", "Fishing", "Forensic Evaluation", "Herding", "Hiding", "Provocation",
  "Inscription", "Lockpicking", "Magery", "Resisting Spells", "Tactics", "Snooping",
  "Musicianship", "Poisoning", "Archery", "Spirit Speak", "Stealing", "Tailoring",
  "Animal Taming", "Taste Identification", "Tinkering", "Tracking", "Veterinary",
  "Swordsmanship", "Mace Fighting", "Fencing", "Wrestling", "Lumberjacking", "Mining",
  "Meditation", "Stealth", "Remove Trap", "Necromancy", "Focus", "Chivalry", "Bushido",
  "Ninjitsu", "Spellweaving", "Mysticism", "Imbuing", "Throwing",
];
function skillName(id) { return SKILL_NAMES[id] || ("Skill #" + id); }
// Active skills that do something on "use" (most via a target cursor). Other ids
// are still double-clickable but the ▸ button is hidden for them.
const USABLE_SKILLS = new Set([
  1, 2, 3, 4, 6, 9, 12, 14, 15, 16, 17, 19, 20, 21, 22, 23, 24, 28, 30, 32, 33, 35,
  36, 38, 46, 47, 48, 56,
]);
const LOCK_ICONS = ["↑", "↓", "🔒"]; // up ↑ / down ↓ / locked 🔒
const LOCK_TITLES = ["raise (click: lower)", "lower (click: lock)", "locked (click: raise)"];
let skillsOn = false;
function toggleSkills() {
  skillsOn = !skillsOn;
  const sk = document.getElementById("skills");
  sk.classList.toggle("on", skillsOn);
  if (skillsOn) { sk._sig = null; refreshSkills(); }
}

// ---- player status bar (UO's pull-out vitals/stats gump) ----
// A draggable window with the player's name, HP/Mana/Stam bars + numbers, and
// STR/DEX/INT/Gold. Toggle with the H key or by clicking the HUD name; its dragged
// position is remembered across sessions (localStorage), so you can "pull it out"
// and leave it where you like.
let statusOn = false;
function toggleStatus() {
  statusOn = !statusOn;
  const el = document.getElementById("statusbar");
  el.classList.toggle("on", statusOn);
  if (statusOn) { bringToFront(el); refreshStatus(scene); }
}
function closeStatus() {
  statusOn = false;
  document.getElementById("statusbar").classList.remove("on");
}
// HUD (top-right character status panel) + journal visibility toggles. Both persist.
// The journal lives inside the HUD, so hiding the HUD hides it too; the journal
// toggle hides just the log while the HUD stays. U = HUD, J = journal.
let hudHidden = false, journalHidden = false;
function applyHudVisibility() {
  const hud = document.getElementById("hud"); if (hud) hud.style.display = hudHidden ? "none" : "";
  const jr = document.getElementById("journal"); if (jr) jr.style.display = journalHidden ? "none" : "";
}
function loadHudVisibility() {
  hudHidden = localStorage.getItem("anima.hudHidden") === "1";
  journalHidden = localStorage.getItem("anima.journalHidden") === "1";
  applyHudVisibility();
}
function toggleHud() {
  hudHidden = !hudHidden;
  localStorage.setItem("anima.hudHidden", hudHidden ? "1" : "0");
  applyHudVisibility();
  setStatus(hudHidden ? "status panel hidden (U)" : "status panel shown");
}
function toggleJournal() {
  journalHidden = !journalHidden;
  localStorage.setItem("anima.journalHidden", journalHidden ? "1" : "0");
  applyHudVisibility();
}
function refreshStatus(s) {
  if (!statusOn || !s || !s.player) return;
  const p = s.player;
  set("st-name", p.name || "(unnamed)");
  set("st-hp-n", `${p.hits | 0} / ${p.hitsMax | 0}`); bar("st-hp", p.hits, p.hitsMax);
  set("st-mana-n", `${p.mana | 0} / ${p.manaMax | 0}`); bar("st-mana", p.mana, p.manaMax);
  set("st-stam-n", `${p.stam | 0} / ${p.stamMax | 0}`); bar("st-stam", p.stam, p.stamMax);
  set("st-str", p.str | 0); set("st-dex", p.dex | 0); set("st-int", p.int | 0);
  set("st-gold", p.gold | 0);
}
function closeSkills() {
  skillsOn = false;
  document.getElementById("skills").classList.remove("on");
}
// Rebuild the list only when the skill data changes (value/base/cap/lock or set).
// ---- skill-gain / loss system messages ----
// UO never announced skill changes in T2A's silent way the renderer showed; the
// traditional client prints "Your skill in X has increased by 0.1." when a skill's
// BASE rises (ClassicUO diffs the 0x3A base value — `v` includes item/stat bonuses
// that fluctuate, so we track `b`). We append these as local journal lines.
const prevSkillBase = new Map();   // skill id -> last seen base (tenths)
let skillGainPrimed = false;       // skip the first scene so login isn't a flood
function checkSkillGains(s) {
  const skills = (s && s.skills) || [];
  if (!skillGainPrimed) {          // record baselines once; announce only later changes
    for (const sk of skills) prevSkillBase.set(sk.id | 0, sk.b | 0);
    skillGainPrimed = true;
    return;
  }
  for (const sk of skills) {
    const id = sk.id | 0, b = sk.b | 0;
    const prev = prevSkillBase.get(id);
    if (prev == null) { prevSkillBase.set(id, b); continue; }
    if (b !== prev) {
      const delta = (Math.abs(b - prev) / 10).toFixed(1);
      const verb = b > prev ? "increased" : "decreased";
      addSysMessage(`Your skill in ${skillName(id)} has ${verb} by ${delta}.`);
      prevSkillBase.set(id, b);
    }
  }
}

function refreshSkills() {
  if (!skillsOn) return;
  const win = document.getElementById("skills");
  const list = document.getElementById("sk-list");
  const skills = (scene && scene.skills) || [];
  const sig = skills.map((s) => `${s.id}:${s.v}:${s.b}:${s.c}:${s.lock}`).join("|");
  if (win._sig === sig) return;
  win._sig = sig;
  // Total skill points = sum of base values (tenths → divide by 10).
  let totalBase = 0;
  for (const s of skills) totalBase += (s.b | 0);
  set("sk-total", `Total: ${(totalBase / 10).toFixed(1)}  ·  ${skills.length} skills`);
  if (!skills.length) { list.innerHTML = '<div class="cont-empty">no skill data</div>'; return; }
  let html = "";
  for (const s of skills) {
    const lock = ((s.lock | 0) % 3 + 3) % 3;
    const usable = USABLE_SKILLS.has(s.id | 0);
    html += `<div class="sk-row${usable ? " usable" : ""}" data-id="${s.id}">`
      + `<span class="sk-lock" data-lock="${lock}" title="${LOCK_TITLES[lock]}">${LOCK_ICONS[lock]}</span>`
      + `<span class="sk-name" title="${skillName(s.id | 0)}">${skillName(s.id | 0)}</span>`
      + `<span class="sk-val">${((s.v | 0) / 10).toFixed(1)}</span>`
      + `<span class="sk-use" title="use skill">▸</span>`
      + (usable ? `<span class="sk-pop" title="pull out as a button">⧉</span>` : "")
      + `</div>`;
  }
  list.innerHTML = html;
}
// One delegated listener (wired once at startup): lock click cycles the lock; the
// ▸ button or a row double-click uses the skill.
function wireSkills() {
  const list = document.getElementById("sk-list");
  list.addEventListener("click", (e) => {
    const row = e.target.closest(".sk-row");
    if (!row) return;
    const id = row.dataset.id | 0;
    if (e.target.classList.contains("sk-lock")) {
      const next = ((e.target.dataset.lock | 0) + 1) % 3; // up→down→locked→up
      sendInput("skilllock:" + id + ":" + next);
      return;
    }
    if (e.target.classList.contains("sk-pop")) {
      addSkillButton(id);          // pull the skill out as a floating, draggable button
      return;
    }
    if (e.target.classList.contains("sk-use")) {
      sendInput("useskill:" + id);
    }
  });
  list.addEventListener("dblclick", (e) => {
    const row = e.target.closest(".sk-row");
    if (row) sendInput("useskill:" + (row.dataset.id | 0));
  });
}

// --- pulled-out skill buttons (UO SkillButtonGump): floating, draggable buttons
// that invoke a skill on click. Created from the skills list's ⧉ control, persisted
// in localStorage so they survive a reload. Click = use; drag = reposition; ✕ = remove.
const SKILLBTN_KEY = "anima.skillbtns";
let skillBtnCascade = 0;
function saveSkillButtons() {
  const arr = [];
  document.querySelectorAll(".skill-gump").forEach((el) => {
    arr.push({ id: +el.dataset.id | 0, x: parseInt(el.style.left, 10) || 0, y: parseInt(el.style.top, 10) || 0 });
  });
  try { localStorage.setItem(SKILLBTN_KEY, JSON.stringify(arr)); } catch (e) {}
}
function makeSkillButton(id, x, y) {
  id = id | 0;
  const el = document.createElement("div");
  el.className = "skill-gump";
  el.dataset.id = id;
  if (x == null) { x = 96 + (skillBtnCascade % 8) * 16; y = 130 + (skillBtnCascade % 8) * 16; skillBtnCascade++; }
  el.style.left = x + "px"; el.style.top = y + "px";
  el.innerHTML = `<span class="sg-name">${skillName(id)}</span><span class="sg-close gump-close">✕</span>`;
  // click vs drag: a stationary press uses the skill; a drag repositions it.
  el.addEventListener("mousedown", (e) => {
    if (e.target.classList.contains("sg-close")) return;
    e.preventDefault();
    bringToFront(el);
    const r = el.getBoundingClientRect();
    const ox = e.clientX - r.left, oy = e.clientY - r.top;
    const dx0 = e.clientX, dy0 = e.clientY;
    let moved = false;
    const move = (ev) => {
      if (Math.abs(ev.clientX - dx0) > 3 || Math.abs(ev.clientY - dy0) > 3) moved = true;
      el.style.left = Math.max(0, Math.min(window.innerWidth - 40, ev.clientX - ox)) + "px";
      el.style.top = Math.max(0, Math.min(window.innerHeight - 20, ev.clientY - oy)) + "px";
    };
    const up = () => {
      window.removeEventListener("mousemove", move); window.removeEventListener("mouseup", up);
      if (moved) { saveSkillButtons(); }
      else { sendInput("useskill:" + id); el.classList.add("flash"); setTimeout(() => el.classList.remove("flash"), 160); }
    };
    window.addEventListener("mousemove", move); window.addEventListener("mouseup", up);
  });
  el.querySelector(".sg-close").addEventListener("click", () => { el.remove(); saveSkillButtons(); });
  document.body.appendChild(el);
  return el;
}
function addSkillButton(id) { makeSkillButton(id, null, null); saveSkillButtons(); }

// "Show all names" (G key — ClassicUO's Ctrl+Shift all-names): single-click every
// in-view character — self, players, NPCs and animals — so the server returns each
// name. The names arrive as overhead text (shown regardless of the name-label
// setting) and also fill the persistent labels. Capped so it never floods the link.
function requestAllNames() {
  if (!scene) return;
  let n = 0;
  if (scene.player) { sendInput("click:" + (scene.player.serial >>> 0)); n++; }
  for (const m of scene.mobiles || []) {
    if (n >= 60) break;
    sendInput("click:" + (m.serial >>> 0));
    n++;
  }
  setStatus(`querying ${n} name${n === 1 ? "" : "s"}…`);
}
function loadSkillButtons() {
  let arr = [];
  try { arr = JSON.parse(localStorage.getItem(SKILLBTN_KEY) || "[]"); } catch (e) { arr = []; }
  for (const b of arr) makeSkillButton(b.id, b.x, b.y);
}

// --- spell quick-cast buttons: drag a spell icon out of the spellbook onto the
// screen → a floating icon button that casts on click, drags to reposition, ✕ to
// remove. Persisted (like skill buttons). ---
const SPELLBTN_KEY = "anima.spellbtns";
let spellBtnCascade = 0;
let spellDrag = null;   // { id, icon, name } while dragging an icon out of the book
function saveSpellButtons() {
  const arr = [];
  document.querySelectorAll(".spell-gump").forEach((el) => {
    arr.push({ id: +el.dataset.id | 0, icon: +el.dataset.icon | 0, name: el.dataset.name || "",
      x: parseInt(el.style.left, 10) || 0, y: parseInt(el.style.top, 10) || 0 });
  });
  try { localStorage.setItem(SPELLBTN_KEY, JSON.stringify(arr)); } catch (e) {}
}
function makeSpellButton(id, icon, name, x, y) {
  const el = document.createElement("div");
  el.className = "spell-gump";
  el.dataset.id = id | 0; el.dataset.icon = icon | 0; el.dataset.name = name || "";
  if (x == null) { x = 120 + (spellBtnCascade % 8) * 16; y = 150 + (spellBtnCascade % 8) * 16; spellBtnCascade++; }
  el.style.left = x + "px"; el.style.top = y + "px";
  el.title = name ? ("Cast " + name) : "Cast";
  el.innerHTML = `<img class="spell-gump-ic" src="gump/${icon | 0}.png" alt="" crossorigin="anonymous"`
    + ` onerror="this.style.visibility='hidden'"><span class="sg-close gump-close">✕</span>`;
  // Stationary press = cast; a drag repositions it (same model as skill buttons).
  el.addEventListener("mousedown", (e) => {
    if (e.target.classList.contains("sg-close")) return;
    e.preventDefault(); bringToFront(el);
    const r = el.getBoundingClientRect();
    const ox = e.clientX - r.left, oy = e.clientY - r.top, dx0 = e.clientX, dy0 = e.clientY;
    let moved = false;
    const move = (ev) => {
      if (Math.abs(ev.clientX - dx0) > 3 || Math.abs(ev.clientY - dy0) > 3) moved = true;
      el.style.left = Math.max(0, Math.min(window.innerWidth - 40, ev.clientX - ox)) + "px";
      el.style.top = Math.max(0, Math.min(window.innerHeight - 20, ev.clientY - oy)) + "px";
    };
    const up = () => {
      window.removeEventListener("mousemove", move); window.removeEventListener("mouseup", up);
      if (moved) saveSpellButtons();
      else { sendInput("cast:" + (id | 0)); el.classList.add("flash"); setTimeout(() => el.classList.remove("flash"), 160); }
    };
    window.addEventListener("mousemove", move); window.addEventListener("mouseup", up);
  });
  el.querySelector(".sg-close").addEventListener("click", () => { el.remove(); saveSpellButtons(); });
  document.body.appendChild(el);
  return el;
}
function loadSpellButtons() {
  let arr = [];
  try { arr = JSON.parse(localStorage.getItem(SPELLBTN_KEY) || "[]"); } catch (e) { arr = []; }
  for (const b of arr) makeSpellButton(b.id, b.icon, b.name, b.x, b.y);
}
// Wire the HTML5 drag-out: dragging a `.sp-icon` from the book drops a button on the
// screen. Registered once (idempotent via a flag).
let spellDragWired = false;
function wireSpellDragOut() {
  if (spellDragWired) return; spellDragWired = true;
  document.getElementById("sb-book").addEventListener("dragstart", (e) => {
    const ic = e.target.closest && e.target.closest(".sp-icon");
    const row = ic && ic.closest(".sp-row");
    if (!row) return;
    spellDrag = { id: +row.dataset.id | 0, icon: +row.dataset.icon | 0, name: row.dataset.name || "" };
    e.dataTransfer.effectAllowed = "copy";
    try { e.dataTransfer.setData("text/plain", String(spellDrag.id)); } catch (_) {}
  });
  document.addEventListener("dragover", (e) => { if (spellDrag) { e.preventDefault(); e.dataTransfer.dropEffect = "copy"; } });
  document.addEventListener("drop", (e) => {
    if (!spellDrag) return;
    e.preventDefault();
    makeSpellButton(spellDrag.id, spellDrag.icon, spellDrag.name, e.clientX - 22, e.clientY - 22);
    saveSpellButtons();
    spellDrag = null;
  });
  document.addEventListener("dragend", () => { spellDrag = null; });
}
// --- party panel (0xBF/0x06, toggled by the 'Y' key; ✕/Esc close) ---
// Lists party members with a name + health bar (hits/hitsMax), the leader marked
// with a crown. "Invite" sends `partyinvite` (the server then opens a target
// cursor — the existing target UI handles the click); "Leave" sends `partyleave`.
// When `scene.party.invite` is non-zero someone invited us: an Accept/Decline
// prompt appears (and the panel auto-opens so it's visible). Member name/hits are
// only known while that member is in view; out-of-view members show "Member"/no bar.
let partyOn = false;
function toggleParty() {
  partyOn = !partyOn;
  const w = document.getElementById("party");
  w.classList.toggle("on", partyOn);
  if (partyOn) { w._sig = null; refreshParty(); }
}
function closeParty() {
  partyOn = false;
  document.getElementById("party").classList.remove("on");
}
// Rebuild only when the party data changes (members, hits, leader, or invite).
function refreshParty() {
  const party = (scene && scene.party) || { leader: 0, members: [], invite: 0 };
  const invite = party.invite | 0;
  // Auto-open the panel when an invite arrives so the prompt is never missed.
  if (invite && !partyOn) {
    partyOn = true;
    document.getElementById("party").classList.add("on");
  }
  if (!partyOn) return;
  const win = document.getElementById("party");
  const sig = `${party.leader}|${invite}|` +
    (party.members || []).map((m) => `${m.serial}:${m.hits}:${m.hitsMax}:${m.name}`).join(",");
  if (win._sig === sig) return;
  win._sig = sig;

  // Incoming-invite prompt (Accept / Decline).
  const prompt = document.getElementById("pt-invite-prompt");
  prompt.classList.toggle("on", !!invite);
  if (invite) {
    prompt.innerHTML =
      '<div class="pt-itext">A party invitation is pending.</div>' +
      '<div class="pt-irow">' +
      `<button class="pt-btn" data-act="partyaccept" data-leader="${invite}">Accept</button>` +
      `<button class="pt-btn" data-act="partydecline" data-leader="${invite}">Decline</button>` +
      '</div>';
  } else {
    prompt.innerHTML = "";
  }

  // Member list with health bars.
  const list = document.getElementById("pt-list");
  const members = party.members || [];
  if (!members.length) {
    list.innerHTML = '<div class="pt-empty">Not in a party.</div>';
  } else {
    let html = "";
    for (const m of members) {
      const isLeader = (m.serial | 0) === (party.leader | 0);
      const max = m.hitsMax | 0;
      const pct = max > 0 ? Math.max(0, Math.min(100, Math.round((m.hits | 0) * 100 / max))) : 0;
      const hp = max > 0 ? `${m.hits | 0}/${max}` : "—";
      const name = (m.name || "Member").replace(/[<>&]/g, "");
      html += `<div class="pt-row${isLeader ? " leader" : ""}">`
        + `<div class="pt-head">`
        + (isLeader ? '<span class="pt-crown" title="leader">♛</span>' : "")
        + `<span class="pt-name">${name}</span>`
        + `<span class="pt-hp">${hp}</span>`
        + `</div>`
        + `<div class="pt-bar"><i style="width:${pct}%"></i></div>`
        + `</div>`;
    }
    list.innerHTML = html;
  }
}
// Wire the party panel once at startup: Invite/Leave buttons + the Accept/Decline
// prompt (delegated so it survives innerHTML rebuilds).
function wireParty() {
  document.getElementById("pt-invite").addEventListener("click", () => sendInput("partyinvite"));
  document.getElementById("pt-leave").addEventListener("click", () => sendInput("partyleave"));
  document.getElementById("pt-invite-prompt").addEventListener("click", (e) => {
    const btn = e.target.closest("button[data-act]");
    if (!btn) return;
    sendInput(btn.dataset.act + ":" + (btn.dataset.leader | 0));
  });
}
// The worn backpack is the equip entry on layer 21 (0x15).
function backpackSerial() {
  const p = scene && scene.player;
  if (!p || !p.equip) return null;
  const bp = p.equip.find((e) => e.layer === BACKPACK_LAYER);
  return bp ? (bp.serial >>> 0) : null;
}
// Open the backpack window AND ask the server to push its latest contents.
function openBackpack() {
  const s = backpackSerial();
  if (s == null) return;
  openContainer(s);
  sendInput("use:" + s);
}
// Rebuild the paperdoll body only when stats/equip actually changed (no flicker).
function refreshPaperdoll() {
  if (!paperdollOn) return;
  const pd = document.getElementById("paperdoll"), body = document.getElementById("pd-body");
  // Source: our own doll (pdTarget null) → scene.player; else the clicked mobile.
  const isSelf = pdTarget == null;
  const p = isSelf ? (scene && scene.player)
    : ((scene && scene.mobiles) || []).find((m) => (m.serial >>> 0) === (pdTarget >>> 0));
  if (!p) {
    set("pd-name", "—");
    body.innerHTML = '<div class="cont-empty">' + (isSelf ? "no character data" : "(out of view)") + "</div>";
    return;
  }
  const equip = (p.equip || []).slice().sort((a, b) => (a.layer | 0) - (b.layer | 0));
  const sig = [isSelf ? "s" : pdTarget, p.name, p.str, p.dex, p.int, p.hits, p.hitsMax, p.mana, p.manaMax,
    p.stam, p.stamMax, p.gold, p.body, p.hue,
    // Include each item's OPL name so the list re-renders (slot label → real name)
    // the moment its OPL arrives.
    equip.map((e) => `${e.layer}:${e.g}:${e.serial >>> 0}:${oplName(e.serial)}`).join(",")].join("|");
  if (pd._sig === sig) return;
  pd._sig = sig;
  set("pd-name", p.name || (isSelf ? "(unnamed)" : "(mobile)"));
  // The paperdoll DOLL: the base body gump (male 0x0C / female 0x0D) hued by skin,
  // then each worn item's paperdoll gump (AnimID + gender offset, hued by item),
  // stacked back→front at the same origin (ClassicUO style). Held weapons included.
  const female = p.body === 401 || p.body === 403;
  const dollBody = female ? 13 : 12;
  const gOff = female ? FEMALE_GUMP_OFFSET : MALE_GUMP_OFFSET;
  const skinQ = p.hue ? `?hue=${p.hue}` : "";
  const byLayer = {};
  for (const e of equip) if ((e.anim | 0) > 0) byLayer[e.layer] = e;
  let h = `<div id="pd-doll"><img src="gump/${dollBody}.png${skinQ}" alt="" crossorigin="anonymous">`;
  for (const layer of PAPERDOLL_ORDER) {
    // Layer 15 (Face) is a server pseudo-item — the face is already part of the body
    // gump and has no paperdoll art, so drawing it 404'd and showed a broken-image
    // "?" at the doll's top-left. Skip it here too (the equip list already skips it).
    if (layer === 15) continue;
    const e = byLayer[layer];
    if (!e) continue;
    const hueQ = e.hue ? `?hue=${e.hue}` : "";
    // Hide any item whose paperdoll gump is missing rather than show a broken "?".
    // Female items may lack a female gump → fall back to the male offset first.
    const hide = "this.onerror=null;this.style.display='none'";
    const onerr = female
      ? `this.onerror=function(){${hide}};this.src='gump/${e.anim + MALE_GUMP_OFFSET}.png${hueQ}'`
      : hide;
    // Tag each layer so hovering the figure (per-pixel hit-test) can resolve the item.
    h += `<img src="gump/${e.anim + gOff}.png${hueQ}" alt="" crossorigin="anonymous" draggable="false"`
      + ` data-serial="${e.serial >>> 0}" data-layer="${e.layer}" data-g="${e.g}" data-hue="${e.hue | 0}" onerror="${onerr}">`;
  }
  h += "</div>";
  // Stats: our own doll shows the full sheet; another mobile shows only what the
  // server actually sent for it (usually name + HP, sometimes nothing).
  h += '<div class="pd-stats">';
  if (p.str != null) h += `<div class="row"><span class="k">STR / DEX / INT</span><span>${p.str} / ${p.dex} / ${p.int}</span></div>`;
  if ((p.hitsMax | 0) > 0) h += `<div class="row"><span class="k">HP</span><span>${p.hits} / ${p.hitsMax}</span></div>`;
  if (isSelf) {
    h += `<div class="row"><span class="k">Mana</span><span>${p.mana} / ${p.manaMax}</span></div>`;
    h += `<div class="row"><span class="k">Stamina</span><span>${p.stam} / ${p.stamMax}</span></div>`;
    h += `<div class="row"><span class="k">Gold</span><span>${p.gold}</span></div>`;
  }
  h += "</div>";
  // Appearance: hair & facial hair are part of the body, not worn gear — show them
  // in their own section with an inline dye-colour swatch (no OPL/weight/AR exists).
  const hairItems = equip.filter((e) => e.layer === 11 || e.layer === 16);
  if (hairItems.length) {
    h += '<div id="pd-appear">';
    for (const e of hairItems) {
      const nm = e.layer === 11 ? "Hair" : "Beard";
      const hue = e.hue | 0;
      h += `<div class="ap-row"><span class="ap-k">${nm}</span>`
        + (hue ? `<span class="hue-sw" data-hue="${hue}"></span><span class="ap-hue">Hue ${hue & 0x3FFF}</span>`
               : '<span class="ap-hue">default</span>')
        + "</div>";
    }
    h += "</div>";
  }
  h += '<div id="pd-equip">';
  // Worn gear only. Hair (11) / beard (16) are shown in Appearance above; the Face
  // layer (15) is the character's virtual face (a server pseudo-item with no real
  // item art → it rendered as the "UNUSED" placeholder), not wearable gear — skip it.
  const worn = equip.filter((e) => e.layer !== 11 && e.layer !== 16 && e.layer !== 15);
  if (!worn.length) h += '<div class="cont-empty">(nothing equipped)</div>';
  for (const e of worn) {
    const serial = e.serial >>> 0;
    // Show the REAL item name (OPL line 0, e.g. "wide-brim hat") rather than the
    // generic slot label ("Head"). Request the OPL once; until it lands we show
    // the slot name as a placeholder, then re-render swaps in the real name.
    const nm = oplName(serial);
    if (!nm && !oplReq.has(serial)) { oplReq.add(serial); sendInput("oplreq:" + serial); }
    const slot = nm || EQUIP_SLOTS[e.layer] || ("Layer " + e.layer);
    // Backpack: our own → open it; another mobile's → SNOOP (a crime, warned).
    const isBp = e.layer === BACKPACK_LAYER;
    const attr = isBp ? (isSelf ? ' data-bp="1"' : ' data-snoop="1"') : "";
    // Tint the icon by the item's dye hue (server recolors the tile via ?hue=), so a
    // dyed robe/cloak/etc. shows its real colour in the list instead of base art.
    const hueQ = (e.hue | 0) ? `?hue=${e.hue | 0}` : "";
    h += `<div class="eq-row${isBp ? " bp" : ""}"${attr}>`
      + `<img class="eq-icon" src="art/static/${e.g}.png${hueQ}" alt="" draggable="false"`
      + ` data-serial="${e.serial >>> 0}" data-g="${e.g}" data-amount="1"`
      + ` data-layer="${e.layer}" data-hue="${e.hue | 0}"`
      + ` onerror="this.style.visibility='hidden'">`
      + `<span class="eq-slot">${slot}</span>`
      + (isBp ? `<span class="eq-open">${isSelf ? "open" : "snoop"} ▸</span>` : "")
      + "</div>";
  }
  h += "</div>";
  body.innerHTML = h;
  applyHueSwatches();
}
// Fill the inline hair/beard colour swatches (async hue→rgb; re-applied on load).
function applyHueSwatches() {
  const pd = document.getElementById("pd-body");
  if (!pd) return;
  for (const sw of pd.querySelectorAll(".hue-sw[data-hue]")) {
    const hx = hueHex((+sw.dataset.hue) | 0);
    if (hx) sw.style.background = hx;
  }
}

// --- container windows (one per serial; openContainer focuses an existing one) ---
const containerWins = new Map(); // serial -> { el, body, sig }
let containerCascade = 0;
// Item graphics that are spellbooks (double-click opens the spell-cast UI, not a
// container). Magery 0x0EFA plus the AOS+ school books for completeness.
const SPELLBOOK_GRAPHICS = new Set([0x0efa, 0x2252, 0x2253, 0x238c, 0x23a0, 0x2d50, 0x2d9d]);
function isSpellbook(g) { return SPELLBOOK_GRAPHICS.has((g | 0) & 0xffff); }

// Amount-tiered stackables show a bigger pile as the stack grows, like the real UO
// client: gold coins (0x0EED) become a small pile (0x0EEE) then a big pile (0x0EEF).
function stackGraphic(g, amount) {
  if ((g | 0) === 0x0eed) return amount > 5 ? 0x0eef : amount > 1 ? 0x0eee : 0x0eed;
  return g | 0;
}

function openContainer(serial) {
  serial = serial >>> 0;
  const existing = containerWins.get(serial);
  if (existing) { bringToFront(existing.el); existing.sig = null; refreshContainer(serial); return; }
  const el = document.createElement("div");
  el.className = "gump-win container-win";
  const off = (containerCascade++ % 9) * 26;
  el.style.left = (220 + off) + "px";
  el.style.top = (70 + off) + "px";
  el.innerHTML = '<div class="gump-title"><span>Container</span><span class="gump-close">✕</span></div>'
    + '<div class="gump-body cont-grid"></div>';
  document.body.appendChild(el);
  const body = el.querySelector(".cont-grid");
  el.querySelector(".gump-close").addEventListener("click", () => closeContainer(serial));
  // Leaving the window entirely clears any item OPL tooltip it was showing (there's
  // no PIXI pointerout for DOM cells to fall back on).
  el.addEventListener("mouseleave", () => { if (tipSerial != null) { tipSerial = null; hideTip(); } });
  makeDraggable(el, el.querySelector(".gump-title"));
  containerWins.set(serial, { el, body, sig: null });
  refreshContainer(serial);
}
function closeContainer(serial) {
  serial = serial >>> 0;
  const win = containerWins.get(serial);
  if (win) { win.el.remove(); containerWins.delete(serial); }
}
// Rebuild a container's grid only when its contents changed. Double-click an item →
// use it; also openContainer(itemSerial) so nested bags pop open (a non-container
// just opens an empty window the user can close — acceptable).
function refreshContainer(serial) {
  serial = serial >>> 0;
  const win = containerWins.get(serial);
  if (!win) return;
  const items = (scene && scene.contItems || []).filter((it) => (it.cont >>> 0) === serial);
  const sig = items.map((it) => `${it.serial >>> 0}:${it.g}:${it.amount | 0}`).join(",");
  if (win.sig === sig) return;
  win.sig = sig;
  const body = win.body;
  body.innerHTML = "";
  if (!items.length) { body.innerHTML = '<div class="cont-empty">(empty)</div>'; return; }
  for (const it of items) {
    const itemSerial = it.serial >>> 0;
    const cell = document.createElement("div");
    cell.className = "cont-item";
    cell.title = "double-click to use / open · drag to move";
    cell.draggable = false;                // pointer-drag (held-on-cursor), not native HTML5 drag
    cell.dataset.serial = itemSerial;
    cell.dataset.g = it.g;
    cell.dataset.amount = (it.amount | 0) || 1;
    const img = document.createElement("img");
    img.className = "cont-icon"; img.src = `art/static/${stackGraphic(it.g, it.amount | 0)}.png`;
    img.draggable = false;                  // let the cell own the drag
    img.onerror = () => { img.style.visibility = "hidden"; };
    cell.appendChild(img);
    if ((it.amount | 0) > 1) {
      const a = document.createElement("span");
      a.className = "cont-amt"; a.textContent = it.amount;
      cell.appendChild(a);
    }
    cell.addEventListener("dblclick", () => {
      // A spellbook opens the spell-casting UI, not a container view.
      if (isSpellbook(it.g)) { if (!spellbookOn) toggleSpellbook(); return; }
      sendInput("use:" + itemSerial);
      // Only open a container window if this item is ACTUALLY a container (`c`).
      // Otherwise (bandages, potions, food, …) double-click just uses it — opening
      // an empty container gump for those was the bug.
      if (it.c) openContainer(itemSerial);
    });
    body.appendChild(cell);
  }
}
function refreshContainers() { for (const serial of containerWins.keys()) refreshContainer(serial); }

// --- generic server gumps / dialogs (0xB0 / 0xDD) ------------------------
// Each scene.gumps entry is a server dialog (quest/NPC menu/confirm box) parsed
// (server-side) into positioned elements. We mirror one draggable .gump-win per
// serial: build on first sight, rebuild when its content signature changes, and
// remove when it's gone from scene.gumps. A button click collects the on-state of
// all checkboxes/radios + text-entry values and sends a `gump:` reply, then closes
// locally; the ✕ sends button 0 (cancel). These are normal windows — they don't
// block the rest of the game.
const gumpWins = new Map(); // serial -> { el, sig }
let gumpCascade = 0;
function gumpSignature(g) {
  return JSON.stringify([g.gumpId >>> 0, g.w | 0, g.h | 0, g.elements || []]);
}
function refreshGumps(scene) {
  const list = (scene && scene.gumps) || [];
  const seen = new Set();
  for (const g of list) {
    const serial = (g.serial >>> 0);
    seen.add(serial);
    const sig = gumpSignature(g);
    const existing = gumpWins.get(serial);
    if (existing && existing.sig === sig) continue; // unchanged
    if (existing) existing.el.remove();             // content changed → rebuild
    buildGumpWindow(serial, g, sig);
  }
  // Drop windows whose gump the server closed.
  for (const serial of [...gumpWins.keys()]) {
    if (!seen.has(serial)) { gumpWins.get(serial).el.remove(); gumpWins.delete(serial); }
  }
}
// ── Right-click context (popup) menu (0xBF/0x14) ───────────────────────────
// scene.popup = { serial, entries:[{ index, text }] } | null. We show a small
// menu div at the last cursor position; a row click sends popupsel and hides it;
// click-away / Esc / the popup clearing also hides it.
let popupEl = null;            // the live menu element (null = hidden)
let popupSerial = 0;           // serial the menu was opened for
function hidePopup() {
  if (popupEl) { popupEl.remove(); popupEl = null; popupSerial = 0; }
}
function refreshPopup(scene) {
  const p = scene && scene.popup;
  if (!p || !p.entries || !p.entries.length) { hidePopup(); return; }
  const serial = p.serial >>> 0;
  // Already showing this exact menu? Leave it where the user put it.
  if (popupEl && popupSerial === serial) return;
  hidePopup();
  popupSerial = serial;
  const el = document.createElement("div");
  el.className = "popup-menu";
  // Anchor at the cursor, clamped to stay on-screen.
  const x = Math.min(lastMenuX, window.innerWidth - 200);
  const y = Math.min(lastMenuY, window.innerHeight - (p.entries.length * 26 + 12));
  el.style.left = Math.max(4, x) + "px";
  el.style.top = Math.max(4, y) + "px";
  for (const e of p.entries) {
    const row = document.createElement("div");
    row.className = "popup-row";
    row.textContent = e.text || ("#" + e.index);
    const index = e.index | 0;
    row.addEventListener("click", (ev) => {
      ev.stopPropagation();
      sendInput("popupsel:" + serial + ":" + index);
      hidePopup();
    });
    el.appendChild(row);
  }
  document.body.appendChild(el);
  popupEl = el;
}

function buildGumpWindow(serial, g, sig) {
  const gumpId = g.gumpId >>> 0;
  const el = document.createElement("div");
  el.className = "gump-win dialog-win";
  const off = (gumpCascade++ % 8) * 24;
  el.style.left = (160 + off) + "px";
  el.style.top = (90 + off) + "px";
  const w = Math.max(80, g.w | 0), h = Math.max(48, g.h | 0);

  const title = document.createElement("div");
  title.className = "gump-title";
  title.innerHTML = '<span>Dialog</span><span class="gump-close">✕</span>';
  el.appendChild(title);

  const body = document.createElement("div");
  body.className = "gump-body";
  const canvas = document.createElement("div");
  canvas.className = "dialog-canvas";
  canvas.style.width = w + "px";
  canvas.style.height = h + "px";
  body.appendChild(canvas);
  el.appendChild(body);

  // Only page 0/1 elements are shown (page changes aren't tracked locally).
  for (const e of (g.elements || [])) {
    if ((e.page | 0) > 1) continue;
    canvas.appendChild(buildGumpElement(serial, gumpId, e));
  }

  // ✕ → cancel (button 0).
  title.querySelector(".gump-close").addEventListener("click", () => {
    sendInput(`gump:${serial}:${gumpId}:0`);
    closeGump(serial);
  });
  makeDraggable(el, title);
  document.body.appendChild(el);
  gumpWins.set(serial, { el, sig });
}
function buildGumpElement(serial, gumpId, e) {
  const node = document.createElement(e.t === "button" ? "button" : "div");
  node.className = "dlg-el";
  node.style.left = (e.x | 0) + "px";
  node.style.top = (e.y | 0) + "px";
  if (e.t === "bg") {
    node.classList.add("dlg-bg");
    if (e.w) node.style.width = (e.w | 0) + "px";
    if (e.h) node.style.height = (e.h | 0) + "px";
  } else if (e.t === "text") {
    node.classList.add("dlg-text");
    if (e.w) node.style.width = (e.w | 0) + "px";
    node.textContent = e.s || "";
  } else if (e.t === "button") {
    node.classList.add("dlg-btn");
    node.type = "button";
    node.textContent = (e.id | 0) || "?";
    node.title = "reply " + (e.id | 0);
    node.addEventListener("click", () => submitGump(serial, gumpId, e.id | 0));
  } else if (e.t === "check" || e.t === "radio") {
    const input = document.createElement("input");
    input.type = e.t === "check" ? "checkbox" : "radio";
    input.dataset.swid = (e.id | 0);
    if (e.on) input.checked = true;
    node.appendChild(input);
  } else if (e.t === "entry") {
    const input = document.createElement("input");
    input.type = "text";
    input.className = "dlg-entry";
    input.dataset.entryid = (e.id | 0);
    input.value = e.s || "";
    if (e.w) input.style.width = (e.w | 0) + "px";
    node.appendChild(input);
  }
  return node;
}
// Collect every checked switch + text-entry value in this gump and send the reply.
function submitGump(serial, gumpId, button) {
  const win = gumpWins.get(serial >>> 0);
  let cmd = `gump:${serial}:${gumpId}:${button}`;
  if (win) {
    const switches = [...win.el.querySelectorAll("input[data-swid]")]
      .filter((i) => i.checked).map((i) => i.dataset.swid);
    if (switches.length) cmd += ":sw=" + switches.join(",");
    // Text entries: skip commas/colons/equals which would break the delimited form.
    const entries = [...win.el.querySelectorAll("input[data-entryid]")]
      .map((i) => `${i.dataset.entryid}=${(i.value || "").replace(/[,:=]/g, " ")}`);
    if (entries.length) cmd += ":e=" + entries.join(",");
  }
  sendInput(cmd);
  closeGump(serial);
}
function closeGump(serial) {
  serial = serial >>> 0;
  const win = gumpWins.get(serial);
  if (win) { win.el.remove(); gumpWins.delete(serial); }
}

// ── book reader (0x93/0xD4 header + 0x66 pages) ────────────────────────────
// scene.book = { serial, title, author, writable, pageCount, pages:[[line,…],…] }
// | null. A dark gump opens when a book appears; if its pages are still empty we
// auto-request them (outgoing 0x66 via `bookreq`). Read-only — page editing for a
// writable book is not implemented (noted as a limitation). ✕ closes the reader.
let bookWin = null;        // the live reader element (null = closed)
let bookSerial = 0;        // serial of the book being shown
let bookPage = 0;          // current page index (0-based)
let bookRequested = 0;     // serial we've already sent a page request for
let bookDismissed = 0;     // serial the user closed (stays closed until a new book)
function closeBook() {
  if (bookWin) { bookWin.remove(); bookWin = null; }
  bookDismissed = bookSerial; // remember so refreshBook won't reopen this one
  bookPage = 0;
}
function refreshBook(scene) {
  const b = scene && scene.book;
  if (!b) { if (bookWin) { bookWin.remove(); bookWin = null; } bookSerial = 0; bookRequested = 0; bookDismissed = 0; return; }
  const serial = b.serial >>> 0;
  if (bookDismissed === serial) return; // user closed this one; leave it closed
  // New book → (re)build the window and reset to page 1.
  if (!bookWin || bookSerial !== serial) {
    if (bookWin) { bookWin.remove(); bookWin = null; }
    bookSerial = serial;
    bookPage = 0;
    buildBookWindow(b);
  }
  // Auto-request page content once if pages are still empty.
  const empty = !b.pages || b.pages.every((p) => !p || p.length === 0);
  if (empty && (b.pageCount | 0) > 0 && bookRequested !== serial) {
    bookRequested = serial;
    sendInput("bookreq:" + serial + ":" + (b.pageCount | 0));
  }
  renderBookPage(b);
}
function buildBookWindow(b) {
  const el = document.createElement("div");
  el.className = "gump-win book-win";
  const title = document.createElement("div");
  title.className = "gump-title";
  const name = (b.title || "Book") + (b.author ? " · " + b.author : "");
  const t = document.createElement("span");
  t.textContent = name;
  const x = document.createElement("span");
  x.className = "gump-close"; x.textContent = "✕";
  x.addEventListener("click", closeBook);
  title.appendChild(t); title.appendChild(x);
  el.appendChild(title);

  const body = document.createElement("div");
  body.className = "gump-body";
  const text = document.createElement("div");
  text.className = "book-text";
  body.appendChild(text);
  const nav = document.createElement("div");
  nav.className = "book-nav";
  const prev = document.createElement("button");
  prev.type = "button"; prev.className = "book-btn"; prev.textContent = "‹ Prev";
  const label = document.createElement("span");
  label.className = "book-pageno";
  const next = document.createElement("button");
  next.type = "button"; next.className = "book-btn"; next.textContent = "Next ›";
  prev.addEventListener("click", () => { if (bookPage > 0) { bookPage--; renderBookPage(scene && scene.book); } });
  next.addEventListener("click", () => {
    const bk = scene && scene.book;
    const last = bk ? (bk.pageCount | 0) - 1 : 0;
    if (bookPage < last) { bookPage++; renderBookPage(bk); }
  });
  nav.appendChild(prev); nav.appendChild(label); nav.appendChild(next);
  body.appendChild(nav);
  el.appendChild(body);
  el._text = text; el._label = label; el._prev = prev; el._next = next;

  makeDraggable(el, title);
  document.body.appendChild(el);
  bookWin = el;
}
function renderBookPage(b) {
  if (!bookWin || !b) return;
  const count = b.pageCount | 0;
  if (bookPage > count - 1) bookPage = Math.max(0, count - 1);
  const lines = (b.pages && b.pages[bookPage]) || [];
  bookWin._text.textContent = lines.length ? lines.join("\n") : "(blank page)";
  bookWin._label.textContent = "page " + (bookPage + 1) + " / " + Math.max(1, count);
  bookWin._prev.disabled = bookPage <= 0;
  bookWin._next.disabled = bookPage >= count - 1;
}

// --- vendor shop window (BUY + SELL) -------------------------------------
// Auto-opens when scene.shop arrives (a vendor was double-clicked). BUY lists the
// vendor's stock (its container's contItems matched to scene.shop.buy.prices by
// index) with a qty + Buy button; SELL lists pack items the vendor will buy with a
// qty + Sell button. ✕ closes (and suppresses reopen until the vendor window is
// gone). Mirrors the dark gump chrome; only acts via sendInput().
let shopWin = null;        // { el, body, sig }
let shopDismissed = false; // user closed it; don't reopen until scene.shop clears
let shopSort = { key: "name", dir: 1 }; // buy-list sort: key name|price|amount, dir 1/-1
function refreshShop(scene) {
  const shop = scene && scene.shop;
  if (!shop) { shopDismissed = false; if (shopWin) closeShop(); return; }
  if (shopDismissed) return;
  if (!shopWin) openShop();
  renderShop(shop);
}
function openShop() {
  const el = document.createElement("div");
  el.className = "gump-win";
  el.id = "shop-win";
  el.innerHTML = '<div class="gump-title"><span>Vendor</span><span class="gump-close">✕</span></div>'
    + '<div class="gump-body shop-body"></div>';
  document.body.appendChild(el);
  el.querySelector(".gump-close").addEventListener("click", () => { shopDismissed = true; closeShop(); });
  makeDraggable(el, el.querySelector(".gump-title"));
  // One delegated click handler for all Buy/Sell buttons.
  const body = el.querySelector(".shop-body");
  body.addEventListener("click", (e) => {
    // sort header: click a column to sort by it (click again toggles direction)
    const sk = e.target.closest(".shop-sortk");
    if (sk) {
      const k = sk.dataset.k;
      shopSort = { key: k, dir: shopSort.key === k ? -shopSort.dir : 1 };
      shopWin.sig = null; renderShop(scene && scene.shop);
      return;
    }
    const btn = e.target.closest(".shop-btn");
    if (!btn) return;
    const row = btn.closest(".shop-row");
    const qtyEl = row.querySelector(".shop-qty");
    let qty = Math.max(1, Math.min(60000, parseInt(qtyEl.value, 10) || 1));
    const serial = (+btn.dataset.serial) >>> 0;
    const vendor = (+btn.dataset.vendor) >>> 0;
    sendInput(`${btn.dataset.act}:${vendor}:${serial}x${qty}`);
  });
  shopWin = { el, body, sig: null };
}
function closeShop() {
  if (shopWin) { shopWin.el.remove(); shopWin = null; }
}
function renderShop(shop) {
  if (!shopWin) return;
  const buy = shop.buy, sell = shop.sell;
  // Match the vendor container's contItems to the price list by index.
  const buyItems = buy
    ? (scene && scene.contItems || []).filter((it) => (it.cont >>> 0) === (buy.cont >>> 0))
    : [];
  // Signature → only rebuild on change (preserves typed quantities; no flicker).
  const sig = JSON.stringify({
    sort: shopSort,
    bv: buy ? (buy.vendor >>> 0) : 0,
    bp: buy ? buy.prices : 0,
    bi: buyItems.map((it) => [it.serial >>> 0, it.g, it.amount | 0]),
    sv: sell ? (sell.vendor >>> 0) : 0,
    si: sell ? sell.items.map((it) => [it.serial >>> 0, it.g, it.amount | 0, it.price]) : 0,
  });
  if (shopWin.sig === sig) return;
  shopWin.sig = sig;
  let h = "";
  if (buy && buy.prices && buy.prices.length) {
    const vendor = buy.vendor >>> 0;
    // Pair each price to its container item by the item's X slot. ServUO's buy list
    // (0x74) is in sorted forward order, while the container-content packet (0x3C) is
    // written REVERSED but stamps each item with X = its 1-based position (Packets.cs
    // VendorBuyContent). Our items live in a HashMap (arrival order lost), so neither
    // forward nor reverse indexing is reliable — sorting by that X restores the exact
    // 0x74 order (this is also why ClassicUO sorts the vendor container by X).
    const pairItems = buyItems.slice().sort((a, b) => (a.x | 0) - (b.x | 0));
    let rows = pairItems.map((it, i) => ({ it, pr: buy.prices[i] })).filter((r) => r.pr)
      .map((r) => ({ g: r.it.g, serial: r.it.serial >>> 0, amount: r.it.amount | 0,
        name: r.pr.name || ("item " + r.it.g), price: r.pr.price | 0 }));
    const k = shopSort.key, d = shopSort.dir;
    rows.sort((a, b) => d * (k === "name" ? a.name.localeCompare(b.name) : (a[k] | 0) - (b[k] | 0)));
    const arrow = (key) => shopSort.key === key ? (shopSort.dir > 0 ? " ▲" : " ▼") : "";
    h += '<div class="shop-sect">Buy</div>';
    h += '<div class="shop-sortbar">Sort: '
      + `<span class="shop-sortk" data-k="name">Name${arrow("name")}</span>`
      + `<span class="shop-sortk" data-k="price">Price${arrow("price")}</span>`
      + `<span class="shop-sortk" data-k="amount">Qty${arrow("amount")}</span></div>`;
    for (const r of rows) {
      h += '<div class="shop-row">'
        + `<img class="shop-icon" src="art/static/${r.g}.png" onerror="this.style.visibility='hidden'">`
        + `<span class="shop-name" title="${esc(r.name)}">${esc(r.name)}</span>`
        + `<span class="shop-stock" title="vendor stock">x${r.amount || 1}</span>`
        + `<span class="shop-price">${r.price}gp</span>`
        + `<input class="shop-qty" type="number" min="1" max="${r.amount || 1}" value="1">`
        + `<button class="shop-btn" data-act="buy" data-vendor="${vendor}" data-serial="${r.serial}">Buy</button>`
        + "</div>";
    }
  }
  if (sell && sell.items && sell.items.length) {
    const vendor = sell.vendor >>> 0;
    h += '<div class="shop-sect">Sell</div>';
    for (const it of sell.items) {
      const name = it.name || ("item " + it.g);
      h += '<div class="shop-row">'
        + `<img class="shop-icon" src="art/static/${it.g}.png" onerror="this.style.visibility='hidden'">`
        + `<span class="shop-name" title="${esc(name)}">${esc(name)} (x${it.amount | 0})</span>`
        + `<span class="shop-price">${it.price}gp</span>`
        + `<input class="shop-qty" type="number" min="1" max="${it.amount | 0}" value="${it.amount | 0}">`
        + `<button class="shop-btn" data-act="sell" data-vendor="${vendor}" data-serial="${it.serial >>> 0}">Sell</button>`
        + "</div>";
    }
  }
  if (!h) h = '<div class="cont-empty">(vendor has nothing)</div>';
  shopWin.body.innerHTML = h;
}
// Minimal HTML-escape for vendor item names (server-supplied strings).
function esc(s) {
  return String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
}

// ---- item drag & drop ----------------------------------------------------
// UO move = lift (pickup 0x07) then drop (0x08). Dragging an item icon onto:
//   • another container window → move it into that bag
//   • the game canvas (ground) → drop it at that tile
//   • the paperdoll → equip it (server derives the layer from tiledata)
// UO "item on the cursor": once an item is lifted it sticks to the mouse and follows
// it until the player clicks a valid place target. This is the single shared held
// state; every drag source (world item, container icon, paperdoll icon, worn doll
// item) funnels into it, and every placement reads from it.
let cursorItem = null;          // { serial, g, amount } | null — the held item, or nothing
let liftDrag = false;           // true while the very press that lifted the item is still down
let placedAt = 0;               // perf time of the last placement (debounces the trailing mousedown)
// Pointer-drag arming (canvas sprites can't fire HTML5 dragstart): a left-press on a
// draggable item arms `groundDrag`; once the cursor moves past DRAG_THRESHOLD px the
// item lifts onto the cursor (held), and a release/next-click places it.
let groundDrag = null;          // { serial, g, amount, sx, sy, started } or null
let dragGhost = null;           // floating <img> glued to the cursor while an item is held
const DRAG_THRESHOLD = 5;       // px the pointer must move before a press becomes a drag

// Place/refresh the floating ghost image at the cursor (page coords).
function moveGhost(clientX, clientY) {
  if (!dragGhost) return;
  dragGhost.style.left = clientX + "px";
  dragGhost.style.top = clientY + "px";
}
// Tear down any in-progress ground drag (ghost + state).
function endGroundDrag() {
  if (dragGhost) { dragGhost.remove(); dragGhost = null; }
  groundDrag = null;
}

// Lift an item onto the cursor (UO pickup): set the shared held state, send the
// `pickup` ONCE, and show the floating ghost that now follows the mouse until the
// item is placed. Reused by every drag source so lifting behaves identically.
function liftToCursor(serial, g, amount, clientX, clientY) {
  serial = serial >>> 0; g = g | 0; amount = amount || 1;
  cursorItem = { serial, g, amount };
  sendInput("pickup:" + serial + (amount > 1 ? ":" + amount : ""));
  if (dragGhost) { dragGhost.remove(); dragGhost = null; }
  const img = document.createElement("img");
  img.src = `art/static/${g}.png`;
  img.style.cssText = "position:fixed;transform:translate(-50%,-50%);opacity:0.8;" +
    "pointer-events:none;z-index:100000;image-rendering:pixelated;";   // pointer-events:none → never blocks elementFromPoint
  img.onerror = () => { img.style.visibility = "hidden"; };
  document.body.appendChild(img);
  dragGhost = img;
  if (clientX != null) moveGhost(clientX, clientY);
}
// Drop the held item from the cursor: clear state and remove the ghost.
function clearCursorItem() {
  cursorItem = null; liftDrag = false;
  if (dragGhost) { dragGhost.remove(); dragGhost = null; }
}
// Resolve where a placement click landed and issue the matching drop/equip (the
// `pickup` already went out at lift, so only one command here). Returns true if the
// item was placed (caller clears it); false to KEEP holding (clicked an invalid spot).
function placeCursorItem(clientX, clientY) {
  if (!cursorItem) return false;
  const serial = cursorItem.serial;
  const el = document.elementFromPoint(clientX, clientY);
  const contWin = el && el.closest && el.closest(".container-win");
  if (contWin) {
    let tgt = null;
    for (const [s, w] of containerWins) if (w.el === contWin) { tgt = s; break; }
    if (tgt == null) return false;
    const r = contWin.getBoundingClientRect();
    const gx = Math.max(0, Math.min(150, Math.round(clientX - r.left)));
    const gy = Math.max(0, Math.min(120, Math.round(clientY - r.top - 20)));
    sendInput("drop:" + serial + ":" + gx + ":" + gy + ":0:" + tgt);
    return true;
  }
  if (el && el.closest && el.closest("#paperdoll")) {
    sendInput("equip:" + serial + ":0");   // layer 0 = server derives the wear layer
    return true;
  }
  if (el && el.closest && el.closest("#map")) {
    const gl = clientToGlobal(clientX, clientY), t = groundTileAt(gl.x, gl.y);
    sendInput("drop:" + serial + ":" + t.x + ":" + t.y + ":" + t.z + ":4294967295");
    return true;
  }
  return false;   // other UI / empty space → keep holding
}
// Esc while holding → return the item. Choice: drop it back into our own backpack
// (the layer-21 worn pack) if we know its serial — that's the safe "undo" that won't
// scatter loot on the floor; only if we have no backpack do we fall back to dropping
// on the ground tile under the cursor. Then clear the held state.
function returnCursorItem() {
  if (!cursorItem) return;
  const serial = cursorItem.serial;
  const bp = backpackSerial();
  if (bp != null) {
    sendInput("drop:" + serial + ":0:0:0:" + bp);
  } else {
    const gl = clientToGlobal(lastMenuX, lastMenuY), t = groundTileAt(gl.x, gl.y);
    sendInput("drop:" + serial + ":" + t.x + ":" + t.y + ":" + t.z + ":4294967295");
  }
  clearCursorItem();
}

function setupItemDnD() {
  // Promote an armed press (world item / container icon / paperdoll icon / worn doll
  // item) into a real lift once it moves past DRAG_THRESHOLD: the item jumps onto the
  // cursor (UO pickup) and the ghost follows the mouse until placed. Cancelling the
  // pending single-click for this item suppresses the name-request a plain click fires.
  window.addEventListener("mousemove", (e) => {
    if (groundDrag && !cursorItem && !groundDrag.started) {
      if (Math.abs(e.clientX - groundDrag.sx) < DRAG_THRESHOLD &&
          Math.abs(e.clientY - groundDrag.sy) < DRAG_THRESHOLD) return;
      groundDrag.started = true;
      if (clickPend && (clickPend.serial >>> 0) === groundDrag.serial) { clearTimeout(clickPend.timer); clickPend = null; }
      liftToCursor(groundDrag.serial, groundDrag.g, groundDrag.amount, e.clientX, e.clientY);
      liftDrag = true;            // the lifting press is still down; its release decides one-motion placement
      groundDrag = null;
    }
    if (cursorItem) moveGhost(e.clientX, e.clientY);
  });
  // Release of the LIFTING press: a one-motion drag that ends over a valid target
  // places immediately; ending over nothing leaves the item held for a later click.
  // (Separate placement clicks are handled in the pointerdown listener below.)
  window.addEventListener("mouseup", (e) => {
    if (e.button !== 0) return;
    if (groundDrag && !groundDrag.started) { groundDrag = null; return; }  // never moved → leave the click alone
    groundDrag = null;
    if (!liftDrag) return;        // not the lifting press → nothing to resolve here
    liftDrag = false;
    placedAt = performance.now();
    if (placeCursorItem(e.clientX, e.clientY)) clearCursorItem();
  });
  // A held item is placed by the NEXT primary click; when not holding, a left press on
  // a container / own-paperdoll item icon arms the same pointer-drag world items use.
  // Capture phase + stopImmediatePropagation while placing so the click resolves the
  // placement BEFORE PIXI entity handlers / canvas steering and does nothing else.
  // Guard: if a server TARGET cursor is up, the click must answer it, so don't place.
  window.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    if (cursorItem) {
      if (liftDrag) return;       // the lifting press is still down → its mouseup resolves it
      if (scene && scene.target && scene.target.active === 1 && !targetUIHidden) return;
      e.preventDefault(); e.stopPropagation();
      if (e.stopImmediatePropagation) e.stopImmediatePropagation();
      placedAt = performance.now();
      if (placeCursorItem(e.clientX, e.clientY)) clearCursorItem();
      return;                     // invalid spot → keep holding (the item stays on the cursor)
    }
    const cell = e.target.closest && e.target.closest(".cont-item[data-serial]");
    if (cell) {
      e.preventDefault();
      groundDrag = { serial: (+cell.dataset.serial) >>> 0, g: +cell.dataset.g | 0,
                     amount: (+cell.dataset.amount) || 1, sx: e.clientX, sy: e.clientY, started: false };
      return;
    }
    if (pdTarget == null) {       // own paperdoll only — can't move another mobile's gear
      const ic = e.target.closest && e.target.closest("#paperdoll .eq-icon[data-serial]");
      if (ic) {
        e.preventDefault();
        groundDrag = { serial: (+ic.dataset.serial) >>> 0, g: +ic.dataset.g | 0,
                       amount: 1, sx: e.clientX, sy: e.clientY, started: false };
      }
    }
  }, true);
  // Tooltip follows the cursor while visible.
  window.addEventListener("mousemove", (e) => {
    const t = document.getElementById("tip");
    if (t && t.style.display === "block") { t.style.left = (e.clientX + 14) + "px"; t.style.top = (e.clientY + 10) + "px"; }
  });
}

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
  const dx = mouseX - app.screen.width / 2, dy = mouseY - app.screen.height / 2;
  const range = Math.hypot(dx, dy);
  if (range < MOUSE_DEADZONE) return null;
  // screen → world dir: our iso is rotated one step (ClassicUO `facing - 1`).
  const dir = (screenDir(dx, dy) + 7) % 8;
  return { dir, run: range >= MOUSE_RUN_RANGE };
}

// What the player wants to do this frame (single source for prediction + send).
function activeMove() {
  if (chatting || wmOn) return null;   // don't walk while typing or with the world map open
  if (rightDown) { const m = mouseMove(); if (m) return m; }
  if (held.size) return { dir: [...held].pop(), run: shiftHeld };
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
  if (scene && scene.target && scene.target.active === 1 && !targetUIHidden) {
    targetConsumedAt = performance.now();
    sendInput("target:" + serial);   // answer the object-target cursor
    endTargetUI();
    return;
  }
  if (clickPend && clickPend.serial === serial) {  // second click in time → double-click
    clearTimeout(clickPend.timer); clickPend = null;
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
    } else {
      // Double-clicked a MOBILE → open its paperdoll (humanoid bodies only, like UO).
      const m = (scene.mobiles || []).find((x) => (x.serial >>> 0) === (serial >>> 0));
      if (m && (m.body | 0) >= 400 && (m.body | 0) <= 407) openMobilePaperdoll(serial);
    }
  } else {
    if (clickPend) clearTimeout(clickPend.timer);
    clickPend = { serial, timer: setTimeout(() => { sendInput("click:" + serial); clickPend = null; }, DBLCLICK_MS) };
    // Arm a ground-item pointer-drag: a left-press on a world item may turn into a
    // drag once the cursor moves past DRAG_THRESHOLD (see setupItemDnD). Until then
    // this stays a normal click; starting a drag cancels the pending name-request.
    if (isItem) {
      const it = (scene && scene.items || []).find((x) => (x.serial >>> 0) === (serial >>> 0));
      groundDrag = { serial: serial >>> 0, g: it ? it.g : 0, amount: (it && (it.amount | 0)) || 1, sx: e.clientX, sy: e.clientY, started: false };
    }
  }
}

// Invert the iso projection at the player's z to get the world tile under a click
// (renderer-space global coords minus the camera offset). Matches the forward
// projection isoX/isoY with HALF/ZSTEP; z is assumed to be the player's z.
function groundTileAt(gx, gy) {
  const z = scene && scene.player ? (scene.player.z | 0) : 0;
  const sx = gx - app.stage.position.x, sy = gy - app.stage.position.y;
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
function updateTargetUI() {
  const active = !!(scene && scene.target && scene.target.active === 1);
  if (active && !prevTargetActive) targetUIHidden = false; // fresh request → show again
  prevTargetActive = active;
  const show = active && !targetUIHidden;
  if (app && app.canvas) app.canvas.style.cursor = show ? "crosshair" : "";
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

// ---- user macros / hotkeys (client-only; persisted in localStorage) ----
// A macro is { id, key, ctrl, alt, shift, action } where `key` is a KeyboardEvent
// `e.code` and `action` is one of:
//   { t:"say", text } · { t:"cast", id } · { t:"skill", id } · { t:"ability", id }
//   { t:"war", on:0|1|"toggle" } · { t:"open", win:"paperdoll|backpack|spellbook|skills|minimap|worldmap" }
// Execution reuses the existing sendInput(...) commands + window toggles, so this is
// purely a client-side layer with no server changes.
const MACRO_KEY = "anima.macros";
let macros = [];
let macrosOn = false;
let warOn = 0;                  // local guess of war stance, for { t:"war", on:"toggle" }
let mcPending = null;           // combo captured in the editor's key field, pending "Add"

// Keys macros may NOT override: movement (KEY_DIR) + the bound window/chat/editor keys.
// These are handled before macro dispatch and rejected at add-time.
const RESERVED_CODES = new Set([
  ...Object.keys(KEY_DIR),
  "KeyT", "Enter", "NumpadEnter",
  "KeyM", "KeyB", "KeyP", "KeyI", "KeyK", "KeyL", "KeyN", "KeyO", "KeyY", "KeyG", "KeyH", "KeyU", "KeyJ",
  "Escape",
  "Tab", "Space", // war-mode toggle / auto-attack (handled in the game keydown)
]);
const OPEN_FNS = {
  paperdoll: () => togglePaperdoll(),
  backpack: () => openBackpack(),
  spellbook: () => toggleSpellbook(),
  skills: () => toggleSkills(),
  minimap: () => toggleMinimap(),
  worldmap: () => toggleWorldmap(),
  status: () => toggleStatus(),
};

function loadMacros() {
  try { const raw = localStorage.getItem(MACRO_KEY); if (raw) { const a = JSON.parse(raw); if (Array.isArray(a)) macros = a; } } catch {}
}
function saveMacros() {
  try { localStorage.setItem(MACRO_KEY, JSON.stringify(macros)); } catch {}
}
function codeLabel(code) {
  if (code.startsWith("Key")) return code.slice(3);
  if (code.startsWith("Digit")) return code.slice(5);
  if (code.startsWith("Numpad")) return "Num" + code.slice(6);
  return code;
}
function comboLabel(m) {
  const p = [];
  if (m.ctrl) p.push("Ctrl"); if (m.alt) p.push("Alt"); if (m.shift) p.push("Shift");
  p.push(codeLabel(m.key));
  return p.join("+");
}
function actionSummary(a) {
  switch (a.t) {
    case "say": return `say "${a.text}"`;
    case "cast": return `cast #${a.id}`;
    case "skill": return `use skill #${a.id}`;
    case "ability": return `ability #${a.id}`;
    case "war": return `war ${a.on}`;
    case "open": return `open ${a.win}`;
    default: return a.t;
  }
}
// Find a macro matching this keydown (modifiers must match; reserved keys never match).
function macroFor(e) {
  if (RESERVED_CODES.has(e.code)) return null;
  for (const m of macros) {
    if (m.key === e.code && !!m.ctrl === e.ctrlKey && !!m.alt === e.altKey && !!m.shift === e.shiftKey) return m;
  }
  return null;
}
function runMacroAction(a) {
  switch (a.t) {
    case "say": if (a.text) sendInput("say:" + a.text); break;
    case "cast": sendInput("cast:" + a.id); break;
    case "skill": sendInput("useskill:" + a.id); break;
    case "ability": sendInput("ability:" + a.id); break;
    case "war": {
      let on = a.on;
      if (on === "toggle") { warOn = warOn ? 0 : 1; on = warOn; }
      else { on = a.on ? 1 : 0; warOn = on; }
      sendInput("war:" + on);
      break;
    }
    case "open": { const fn = OPEN_FNS[a.win]; if (fn) fn(); break; }
  }
}
function toggleMacros() {
  macrosOn = !macrosOn;
  const w = document.getElementById("macros");
  w.classList.toggle("on", macrosOn);
  if (macrosOn) { renderMacroList(); document.getElementById("mc-key").focus(); }
}
function closeMacros() { macrosOn = false; document.getElementById("macros").classList.remove("on"); }
function renderMacroList() {
  const list = document.getElementById("mc-list");
  if (!macros.length) { list.innerHTML = '<div class="mc-empty">no macros yet — add one below</div>'; return; }
  list.innerHTML = "";
  for (const m of macros) {
    const row = document.createElement("div");
    row.className = "mc-row";
    const combo = document.createElement("span"); combo.className = "mc-combo"; combo.textContent = comboLabel(m);
    const act = document.createElement("span"); act.className = "mc-act"; act.textContent = actionSummary(m.action);
    const del = document.createElement("span"); del.className = "mc-del"; del.textContent = "✕"; del.title = "delete";
    del.addEventListener("click", () => { macros = macros.filter((x) => x.id !== m.id); saveMacros(); renderMacroList(); });
    row.append(combo, act, del);
    list.appendChild(row);
  }
}
function mcBuildParam() {
  const t = document.getElementById("mc-type").value;
  const p = document.getElementById("mc-param");
  if (t === "say") p.innerHTML = '<input id="mc-pv" class="mc-input" type="text" maxlength="128" placeholder="text to say" />';
  else if (t === "cast" || t === "skill" || t === "ability")
    p.innerHTML = '<input id="mc-pv" class="mc-input" type="number" min="0" placeholder="id" />';
  else if (t === "war")
    p.innerHTML = '<select id="mc-pv" class="mc-input"><option value="toggle">toggle</option><option value="1">on</option><option value="0">off</option></select>';
  else if (t === "open")
    p.innerHTML = '<select id="mc-pv" class="mc-input"><option>paperdoll</option><option>backpack</option><option>spellbook</option><option>skills</option><option>minimap</option><option>worldmap</option><option>status</option></select>';
}
function setupMacroEditor() {
  const win = document.getElementById("macros");
  const keyInput = document.getElementById("mc-key");
  const typeSel = document.getElementById("mc-type");
  const addBtn = document.getElementById("mc-add-btn");
  const msg = document.getElementById("mc-msg");
  // Keep all editor typing out of the game-input handler (it lives on window).
  win.addEventListener("keydown", (e) => {
    e.stopPropagation();
    if (e.code === "Escape") { e.preventDefault(); closeMacros(); }
  });
  // Key-capture field: focus it and press a key → record e.code + modifiers.
  keyInput.addEventListener("keydown", (e) => {
    e.preventDefault(); e.stopPropagation();
    if (e.code === "Escape") { mcPending = null; keyInput.value = ""; return; }
    if (/^(Control|Alt|Shift|Meta)/.test(e.code)) return;   // ignore bare modifier presses
    mcPending = { key: e.code, ctrl: e.ctrlKey, alt: e.altKey, shift: e.shiftKey };
    keyInput.value = comboLabel(mcPending);
  });
  typeSel.addEventListener("change", mcBuildParam);
  mcBuildParam();
  addBtn.addEventListener("click", () => {
    msg.textContent = "";
    if (!mcPending) { msg.textContent = "Click the Key field and press a key first."; return; }
    if (RESERVED_CODES.has(mcPending.key)) { msg.textContent = codeLabel(mcPending.key) + " is reserved — pick another key."; return; }
    const t = typeSel.value;
    const pv = document.getElementById("mc-pv");
    let action;
    if (t === "say") {
      const text = pv.value.trim();
      if (!text) { msg.textContent = "Enter the text to say."; return; }
      action = { t: "say", text };
    } else if (t === "cast" || t === "skill" || t === "ability") {
      const id = parseInt(pv.value, 10);
      if (!Number.isFinite(id) || id < 0) { msg.textContent = "Enter a valid numeric id."; return; }
      action = { t, id };
    } else if (t === "war") {
      const v = pv.value;
      action = { t: "war", on: v === "toggle" ? "toggle" : (v === "1" ? 1 : 0) };
    } else if (t === "open") {
      action = { t: "open", win: pv.value };
    }
    const id = Date.now().toString(36) + Math.random().toString(36).slice(2, 6);
    macros.push({ id, key: mcPending.key, ctrl: mcPending.ctrl, alt: mcPending.alt, shift: mcPending.shift, action });
    saveMacros();
    mcPending = null; keyInput.value = "";
    if (pv && pv.tagName === "INPUT") pv.value = "";
    renderMacroList();
  });
  document.getElementById("mc-close").addEventListener("click", closeMacros);
  makeDraggable(win, document.getElementById("mc-title"));
}

// Spell quick-cast chord: press K, then a circle digit (1-8), then a spell digit
// (1-8) → cast that Magery spell by position. E.g. K 1 1 = Clumsy, K 1 2 = Create
// Food, K 8 8 = Water Elemental. Active for ~1.5s after each key.
let spellChord = null;          // { circle: number|null, t: perf-ms } | null
const SPELL_CHORD_MS = 1500;
function chordDigit(code) {
  const m = /^Digit([1-8])$/.exec(code) || /^Numpad([1-8])$/.exec(code);
  return m ? +m[1] : null;
}
function armSpellChord() { spellChord = { circle: null, t: performance.now() }; setStatus("Spell: circle 1-8…"); }

function setupInput() {
  loadMacros();
  setupMacroEditor();
  window.addEventListener("keydown", (e) => {
    shiftHeld = e.shiftKey;
    if (chatting) return;
    // Spell chord capture (after K): consume circle/spell digits and cast.
    if (spellChord) {
      if (performance.now() - spellChord.t > SPELL_CHORD_MS) {
        spellChord = null;                       // timed out
      } else {
        const d = chordDigit(e.code);
        if (d != null) {
          e.preventDefault();
          if (spellChord.circle == null) { spellChord.circle = d; spellChord.t = performance.now(); setStatus(`Spell ${d}-_`); }
          else {
            const id = (spellChord.circle - 1) * 8 + d;   // Magery spell 1..64
            sendInput("cast:" + id);
            setStatus(`cast ${spellChord.circle}-${d} (${MAGERY_SPELLS[id - 1] || "spell " + id})`);
            spellChord = null;
          }
          return;
        }
        spellChord = null;                       // any non-digit cancels; fall through
      }
    }
    if (e.code === "KeyT" || e.code === "Enter") { e.preventDefault(); openChat(); return; }
    if (e.code === "KeyM") { e.preventDefault(); toggleMinimap(); return; }
    if (e.code === "KeyB") { e.preventDefault(); toggleWorldmap(); return; }
    if (e.code === "KeyP") { e.preventDefault(); togglePaperdoll(); return; }   // P = paperdoll
    if (e.code === "KeyI") { e.preventDefault(); openBackpack(); return; }       // I = backpack
    if (e.code === "KeyK") { e.preventDefault(); toggleSpellbook(); armSpellChord(); return; } // K = spellbook (+ spell chord)
    if (e.code === "KeyL") { e.preventDefault(); toggleSkills(); return; }         // L = skills
    if (e.code === "KeyN") { e.preventDefault(); toggleMute(); return; }
    if (e.code === "KeyO") { e.preventDefault(); toggleMacros(); return; }        // O = macro editor
    if (e.code === "KeyY") { e.preventDefault(); toggleParty(); return; }          // Y = party panel
    if (e.code === "KeyG") { e.preventDefault(); requestAllNames(); return; }      // G = show all names
    if (e.code === "KeyH") { e.preventDefault(); toggleStatus(); return; }          // H = status bar
    if (e.code === "KeyU") { e.preventDefault(); toggleHud(); return; }              // U = hide/show HUD status panel
    if (e.code === "KeyJ") { e.preventDefault(); toggleJournal(); return; }          // J = hide/show journal
    // Esc while holding an item on the cursor → return it (backpack, else ground).
    // Takes priority over closing windows so a held item is never silently lost.
    if (e.code === "Escape" && cursorItem) { e.preventDefault(); returnCursorItem(); return; }
    if (e.code === "Escape" && partyOn) { e.preventDefault(); closeParty(); return; }
    if (e.code === "Escape" && macrosOn) { e.preventDefault(); closeMacros(); return; }
    if (e.code === "Escape" && wmOn) { e.preventDefault(); closeWorldmap(); return; }
    if (e.code === "Escape" && paperdollOn) { e.preventDefault(); closePaperdoll(); return; }
    if (e.code === "Escape" && spellbookOn) { e.preventDefault(); closeSpellbook(); return; }
    if (e.code === "Escape" && skillsOn) { e.preventDefault(); closeSkills(); return; }
    if (e.code === "Escape" && shopWin) { e.preventDefault(); shopDismissed = true; closeShop(); return; }
    if (e.code === "Escape" && popupEl) { e.preventDefault(); hidePopup(); return; }
    if (e.code === "Escape" && bookWin) { e.preventDefault(); closeBook(); return; }
    // Esc cancels targeting: tell the SERVER to drop the cursor (so the spell/skill
    // waiting for a target is aborted, not left hanging) and hide the local UI.
    if (e.code === "Escape" && scene && scene.target && scene.target.active === 1 && !targetUIHidden) {
      e.preventDefault(); sendInput("targetcancel"); endTargetUI(); return;
    }
    // Tab = toggle war mode (ClassicUO default). preventDefault so it never moves
    // focus; send the opposite of the server's authoritative `scene.war`.
    if (e.code === "Tab") {
      e.preventDefault();
      const war = !!(scene && scene.war);
      sendInput("war:" + (war ? "0" : "1"));
      return;
    }
    // Space = auto-attack the nearest hostile (last target if still valid).
    // preventDefault so it never scrolls the page / triggers a focused button.
    if (e.code === "Space") {
      e.preventDefault();
      sendInput("autoattack");
      return;
    }
    // User macros: a non-reserved key+modifier combo runs its bound action.
    const mac = macroFor(e);
    if (mac) { e.preventDefault(); runMacroAction(mac.action); return; }
    if (e.code in KEY_DIR) {
      const d = KEY_DIR[e.code];
      // No direct send here: the prediction (enqueueSteps/processSteps) drives the
      // server now — it sends one walk per committed step. Just record the held key.
      if (!held.has(d)) trace(`KD dir=${d} run=${shiftHeld ? 1 : 0}`);
      held.add(d); e.preventDefault();
    }
  });
  window.addEventListener("keyup", (e) => {
    shiftHeld = e.shiftKey;
    if (e.code in KEY_DIR) { held.delete(KEY_DIR[e.code]); if (!held.size) trace("KU"); }
  });
  // Right-button movement: suppress the context menu, track the cursor, and hold
  // state. Position is tracked window-wide so dragging off-canvas still steers.
  const canvas = app.canvas;
  canvas.addEventListener("contextmenu", (e) => e.preventDefault());
  const track = (e) => {
    const r = canvas.getBoundingClientRect(); mouseX = e.clientX - r.left; mouseY = e.clientY - r.top;
    lastMenuX = e.clientX; lastMenuY = e.clientY;
    // A pending entity-RMB that drags past a few px is a steer, not a context menu.
    if (rmbEntity && !rmbEntity.steering &&
        (Math.abs(e.clientX - rmbEntity.x) > 6 || Math.abs(e.clientY - rmbEntity.y) > 6)) {
      promoteRmbSteer();
    }
  };
  canvas.addEventListener("mousedown", (e) => {
    if (e.button !== 2) return;
    track(e); e.preventDefault();
    // RMB on an entity defers steering (the PIXI pointerdown set `rmbEntity` first):
    // a quick tap opens its menu, a hold/drag promotes to steering. RMB on empty
    // ground steers immediately.
    if (rmbEntity && !rmbEntity.steering) return;
    rightDown = true;
  });
  // Left-click on empty ground while a target cursor is active → answer with a
  // tile (targetxy). Clicks that hit a mobile/item are handled by their PIXI
  // pointerdown first (which fires before this DOM mousedown) and set
  // targetConsumedAt, so we skip those here to avoid a double-resolve.
  canvas.addEventListener("mousedown", (e) => {
    if (e.button !== 0) return;
    if (!(scene && scene.target && scene.target.active === 1) || targetUIHidden) return;
    if (performance.now() - targetConsumedAt < 200) return; // a mob/item already answered
    // Our own avatar is always at the canvas centre but isn't a click target (so it
    // never eats steering). During a target cursor, a click on that centre band IS
    // self — answer with target:<self> so bandages / beneficial spells work on us.
    const r = canvas.getBoundingClientRect();
    const dxp = (e.clientX - r.left) - r.width / 2, dyp = (e.clientY - r.top) - r.height / 2;
    if (scene.player && Math.abs(dxp) < 28 && dyp > -68 && dyp < 14) {
      sendInput("target:" + (scene.player.serial >>> 0));
      endTargetUI();
      return;
    }
    const g = clientToGlobal(e.clientX, e.clientY);
    const t = groundTileAt(g.x, g.y);
    sendInput(`targetxy:${t.x}:${t.y}:${t.z}:0`);
    endTargetUI();
  });
  // (Click-to-walk removed per user request — left-click on empty ground no longer
  // pathfinds/auto-walks. The server-side `walkto` route + pathfinder remain.)
  window.addEventListener("mousemove", track);
  window.addEventListener("mouseup", (e) => {
    if (e.button !== 2) return;
    rightDown = false;
    // Decide the pending entity-RMB: if it never promoted to steering (released
    // before the hold timer + no drag), it was a quick tap → open the context menu.
    // If it steered, the character moved and NO menu pops. Either way it's resolved.
    if (rmbEntity) {
      if (rmbEntity.timer) { clearTimeout(rmbEntity.timer); rmbEntity.timer = null; }
      if (!rmbEntity.steering) { lastMenuX = e.clientX; lastMenuY = e.clientY; sendInput("popupreq:" + rmbEntity.serial); }
      rmbEntity = null;
    }
  });
  // Click anywhere outside an open context menu dismisses it (row clicks stop
  // propagation and dismiss themselves before this fires).
  window.addEventListener("mousedown", (e) => {
    if (popupEl && !popupEl.contains(e.target)) hidePopup();
  }, true);
  // While the cursor is over a DOM gump/panel (paperdoll, shop, dialog, worldmap…)
  // the canvas stops receiving mousemove, so PIXI never fires the entity pointerout
  // that would hide the world OPL tooltip — it'd otherwise stay stuck over the gump.
  // Clear the world tooltip on entering any panel. The paperdoll's own equip-icon
  // tooltip (pdTipEl, set by the icon's mouseover which bubbles first) is preserved.
  document.addEventListener("mouseover", (e) => {
    // Inventory/container item under the cursor → show its OPL tooltip (same flow
    // as world items). This must come BEFORE the panel-suppression below, since a
    // container window IS a gump and would otherwise hide the tooltip.
    const cell = e.target.closest && e.target.closest(".cont-item[data-serial]");
    if (cell) { hoverEntity((+cell.dataset.serial) >>> 0); return; }
    const overPanel = e.target.closest && e.target.closest(".gump-win, #worldmap, #paperdoll, .popup-menu");
    if (overPanel && pdTipEl == null && tipSerial != null) { tipSerial = null; hideTip(); }
  });
  // (Movement is no longer sent on a timer — the prediction sends one walk per
  // committed step in processSteps. `activeMove()` only drives the local prediction.)
  // In-game chat bar: Enter sends, Esc cancels. stopPropagation so typed keys never
  // reach the game-input handler (it also early-returns while `chatting`).
  const bar = document.getElementById("chatbar");
  bar.addEventListener("keydown", (e) => {
    e.stopPropagation();
    if (e.code === "Enter" || e.code === "NumpadEnter") { e.preventDefault(); submitChat(); }
    else if (e.code === "Escape") { e.preventDefault(); closeChat(); }
  });
  bar.addEventListener("blur", () => { if (chatting) closeChat(); });
  // World map: drag to pan, wheel to zoom, ✕/label to open/close.
  const wmc = document.getElementById("wmcanvas");
  let wmDrag = null;
  wmc.addEventListener("mousedown", (e) => {
    e.preventDefault();
    const r = wmc.getBoundingClientRect();
    if (e.shiftKey) { wmRemoveMarkerNear(e.clientX - r.left, e.clientY - r.top, wmc.clientWidth, wmc.clientHeight); return; }
    wmDrag = { x: e.clientX, y: e.clientY };
  });
  wmc.addEventListener("dblclick", (e) => {
    const r = wmc.getBoundingClientRect();
    wmAddMarkerAt(e.clientX - r.left, e.clientY - r.top, wmc.clientWidth, wmc.clientHeight);
  });
  window.addEventListener("mousemove", (e) => {
    if (wmOn) { const r = wmc.getBoundingClientRect(); wmMouse = { x: e.clientX - r.left, y: e.clientY - r.top }; }
    if (!wmDrag) { if (wmOn) drawWorldmap(); return; }
    wmPan.x += e.clientX - wmDrag.x; wmPan.y += e.clientY - wmDrag.y;
    wmDrag = { x: e.clientX, y: e.clientY }; drawWorldmap();
  });
  window.addEventListener("mouseup", () => { wmDrag = null; });
  wmc.addEventListener("mouseleave", () => { wmMouse = null; if (wmOn) drawWorldmap(); });
  wmc.addEventListener("wheel", (e) => {
    e.preventDefault();
    const r = wmc.getBoundingClientRect(), cx = e.clientX - r.left, cy = e.clientY - r.top;
    // Zoom proportional to the actual wheel delta (gentle, consistent for mouse vs
    // trackpad which fires many small events), and clamp the per-event step so a big
    // delta can't jump scale; range kept moderate.
    const f = Math.exp(-Math.max(-120, Math.min(120, e.deltaY)) * 0.0011);
    const ns = Math.min(8, Math.max(0.5, wmScale * f));
    const ratio = ns / wmScale;                      // keep the point under the cursor fixed
    wmPan.x = (cx - wmc.clientWidth / 2) * (1 - ratio) + ratio * wmPan.x;
    wmPan.y = (cy - wmc.clientHeight / 2) * (1 - ratio) + ratio * wmPan.y;
    wmScale = ns; drawWorldmap();
  }, { passive: false });
  document.getElementById("wmclose").addEventListener("click", closeWorldmap);
  document.getElementById("minilabel").addEventListener("click", openWorldmap);
  document.getElementById("mutebtn")?.addEventListener("click", toggleMute);
  // Options panel: button toggles, ✕ closes, title bar drags. Changes persist
  // immediately and apply live (audio volume now; display toggles next repaint).
  const optEl = document.getElementById("options");
  document.getElementById("optbtn")?.addEventListener("click", () => toggleOptions());
  document.getElementById("opt-close")?.addEventListener("click", () => toggleOptions(false));
  makeDraggable(optEl, optEl.querySelector(".gump-title"));
  const optBody = document.getElementById("opt-body");
  optBody.addEventListener("change", (e) => {
    const k = e.target.dataset.k; if (!k || e.target.type !== "checkbox") return;
    settings[k] = e.target.checked; saveSettings(); applyAudioSettings();
    if (k === "tooltips" && !settings.tooltips) { tipSerial = null; hideTip(); }
    if (k === "abilities") refreshAbilities(true);
    markDirty();
  });
  optBody.addEventListener("input", (e) => {
    const k = e.target.dataset.k; if (!k || e.target.type !== "range") return;
    settings[k] = (+e.target.value) / 100;
    const v = document.getElementById("optv-" + k); if (v) v.textContent = e.target.value;
    saveSettings(); applyAudioSettings();
  });
  // Paperdoll: ✕ closes, title bar drags, clicking the Backpack row opens it.
  document.getElementById("pd-close").addEventListener("click", closePaperdoll);
  makeDraggable(document.getElementById("paperdoll"), document.getElementById("pd-title"));
  document.getElementById("sb-close").addEventListener("click", closeSpellbook);
  makeDraggable(document.getElementById("spellbook"), document.getElementById("sb-title"));
  document.getElementById("sk-close").addEventListener("click", closeSkills);
  makeDraggable(document.getElementById("skills"), document.getElementById("sk-title"));
  // Status bar: ✕ closes, title drags; remember the dragged position across sessions.
  const stEl = document.getElementById("statusbar"), stTitle = document.getElementById("st-title");
  document.getElementById("st-close").addEventListener("click", closeStatus);
  makeDraggable(stEl, stTitle);
  try {
    const sp = JSON.parse(localStorage.getItem("anima.statusPos") || "null");
    if (sp && sp.left) { stEl.style.left = sp.left; stEl.style.top = sp.top; stEl.style.right = "auto"; }
  } catch (_) { /* ignore bad/missing saved position */ }
  stTitle.addEventListener("mouseup", () => {
    if (stEl.style.left) localStorage.setItem("anima.statusPos", JSON.stringify({ left: stEl.style.left, top: stEl.style.top }));
  });
  // Clicking the HUD name "pulls out" the movable status bar.
  const pn = document.getElementById("pname");
  if (pn) { pn.style.cursor = "pointer"; pn.title = "Open status bar (H)"; pn.addEventListener("click", toggleStatus); }
  loadHudVisibility();   // restore hidden HUD/journal state (U / J toggles)
  wireSkills();
  loadSkillButtons();   // restore any skill buttons the user pulled out previously
  loadSpellButtons();   // restore any spell quick-cast buttons dragged out earlier
  document.getElementById("pt-close").addEventListener("click", closeParty);
  makeDraggable(document.getElementById("party"), document.getElementById("pt-title"));
  wireParty();
  const pdb = document.getElementById("pd-body");
  pdb.addEventListener("click", (e) => {
    const row = e.target.closest(".eq-row");
    if (!row) return;
    if (row.dataset.bp === "1") openBackpack();
    else if (row.dataset.snoop === "1") {                 // another's pack → snoop
      const ic = row.querySelector(".eq-icon[data-serial]");
      if (ic) snoopBackpack((+ic.dataset.serial) >>> 0);
    }
  });
  // Hover an equipped item → show its OPL (name/weight/AR/properties). Hair & beard
  // have no OPL, so we show their slot name + dye-colour swatch instead.
  pdb.addEventListener("mouseover", (e) => {
    const ic = e.target.closest && e.target.closest(".eq-icon[data-serial]");
    if (ic) showEquipTip(ic);
  });
  pdb.addEventListener("mouseout", (e) => {
    const ic = e.target.closest && e.target.closest(".eq-icon[data-serial]");
    if (ic) { pdTipEl = null; hideTip(); }
  });
  // Hover the DOLL figure itself: per-pixel hit-test resolves the worn item/accessory
  // directly under the cursor (the intuitive UO way), in addition to the list below.
  pdb.addEventListener("mousemove", (e) => {
    if (e.target.closest && e.target.closest("#pd-doll")) dollHitTest(e);
  });
  // Drag a worn item OFF the figure: per-pixel hit-test picks the item under the
  // cursor (not just the topmost layer), then arms the shared pointer-drag — release
  // over a bag/ground unequips it there; over the doll re-equips. Self doll only.
  pdb.addEventListener("mousedown", (e) => {
    if (e.button !== 0 || pdTarget != null) return;        // left only, our own doll
    if (cursorItem || performance.now() - placedAt < 250) return; // holding / just placed → don't re-arm
    if (!(e.target.closest && e.target.closest("#pd-doll"))) return;
    const img = dollImgAt(e);
    if (!img) return;
    e.preventDefault();
    groundDrag = { serial: (+img.dataset.serial) >>> 0, g: +img.dataset.g | 0,
                   amount: 1, sx: e.clientX, sy: e.clientY, started: false };
  });
  pdb.addEventListener("mouseleave", () => {
    if (pdTipEl && pdTipEl.closest && pdTipEl.closest("#pd-doll")) { pdTipEl = null; hideTip(); }
  });
}
// In-game chat bar (replaces window.prompt). Enter opens it; type and Enter sends;
// Esc cancels. Prefix routes the channel: "/p <msg>" or "/party <msg>" → party,
// otherwise a normal in-game say.
function openChat() {
  if (chatting) return;
  chatting = true;
  held.clear();                     // stop walking while typing
  if (wasMoving) { sendInput("stop"); wasMoving = false; }
  const bar = document.getElementById("chatbar");
  bar.value = ""; bar.classList.add("on"); bar.focus();
}
function closeChat() {
  chatting = false;
  const bar = document.getElementById("chatbar");
  bar.classList.remove("on"); bar.blur();
}
function submitChat() {
  const bar = document.getElementById("chatbar");
  const text = bar.value.trim();
  if (text) {
    const m = /^\/(p|party)\s+(.+)$/i.exec(text);
    if (m) sendInput("party:" + m[2].trim());
    else sendInput("say:" + text);
  }
  closeChat();
}
function sendInput(cmd) { fetch("/input", { method: "POST", body: cmd }).catch(() => {}); }
// ---- movement diagnostic trace (POSTs to /log; server prints with ANIMA_DEBUG) ----
let TRACE = false;
function trace(m) { if (TRACE) fetch("/log", { method: "POST", body: Math.round(performance.now()) + " " + m }).catch(() => {}); }

main();
