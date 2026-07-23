// anima-client renderer — isometric, real UO sprites, smooth (interpolated) camera.
//
// Tiles/statics live in ABSOLUTE world-iso coordinates in a persistent pool: as
// the player walks we only add/remove the edge tiles entering/leaving the view —
// never a full rebuild. The camera (stage offset) follows the player's *eased*
// position every frame, so movement scrolls smoothly. Entities are redrawn each
// frame at their interpolated positions with walk/idle animation frames.

const HALF = 22, ZSTEP = 4;
// Shared filter for statics drawn in the grayed-out beyond-view ring (see
// syncWorld's statics loop): grayscale + a brightness lift so it reads as a
// light gray matching the lifted land-tile gray below, rather than a dim/dark
// desaturation. desaturate()/brightness() aren't chainable in this PIXI build
// (neither returns `this`), so they're called on their own lines.
const STATIC_GRAY = new PIXI.ColorMatrixFilter();
STATIC_GRAY.desaturate();
STATIC_GRAY.brightness(1.25, true);
// ClassicUO people animation groups
const WALK = 0, RUN_UNARMED = 2, STAND = 4;
// War-mode idle stance: PAG_STAND_ONEHANDED_ATTACK (the combat-ready pose a person
// holds while standing in war mode). ClassicUO swaps the plain Stand (4) for this.
const PEOPLE_COMBAT_STAND = 7;
const ONMOUNT_WALK = 23, ONMOUNT_RUN = 24, ONMOUNT_STAND = 25;
const CHAR_ANIM_DELAY = 80; // ClassicUO Constants.CHARACTER_ANIMATION_DELAY (ms/frame)
// Animation GROUP NUMBERS differ by body type (ClassicUO): monster Walk=0/Stand=1,
// animal Walk=0/Run=1/Stand=2, people Walk=0/Run=2/Stand=4. `atype` (0 monster /
// 1 animal / 2 people) comes from the server (mobtypes.txt), which is authoritative
// over the raw body-range guess (fallback when `atype` is absent). Using the wrong
// type showed an attack pose while idle (the "cat → alligator when idle" bug).
function bodyType(body, atype) {
  return atype != null ? atype : body < 200 ? 0 : body < 400 ? 1 : 2;
}
function animGroup(moving, running, mounted, body, war, atype) {
  if (mounted) return moving ? (running ? ONMOUNT_RUN : ONMOUNT_WALK) : ONMOUNT_STAND;
  const t = bodyType(body, atype);
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
function resolveActionGroup(action, body, atype) {
  action = action | 0;
  if (bodyType(body, atype) === 2) { // people / humanoid
    if (action >= 200) return PEOPLE_CAST_DIRECTED; // spell cast gesture
    if (action >= 35) return action % 35;           // other out-of-range → fold in
    return action;                                  // direct people group (combat swings, etc.)
  }
  return action;                     // monsters/animals: action indexes their own group set
}

// Convert an 0xE2 NewMobileAnimation's `(typ, action, mode)` to a real per-body
// animation group, following ClassicUO `Mobile.GetObjectNewAnimation` /
// `GetObjectNewAnimationType_*` exactly. `typ` is the wire `AnimationType`
// (Server/Mobile.cs): 0 Attack, 1 Parry, 2 Block, 3 Die, 4 Impact, 5 Fidget,
// 6 Eat, 7 Emote, 8 Alert, 9 TakeOff, 10 Land, 11 Spell, 14 Pillage (12
// StartCombat/13 EndCombat/15 Spawn aren't special-cased upstream either — they
// fall through to group 0, same as here). `mode` is the wire "delay" byte,
// which ClassicUO uses only as a `mode % 2/3/4` seed to pick between a few
// cosmetically-equivalent variants of the same action (NOT a timing value).
// ClassicUO keys off the body's 5-way AnimationGroupsType (Monster/SeaMonster/
// Animal/Human/Equipment); our server only hands us the 3-way `atype`
// (0 monster+sea_monster / 1 animal / 2 people+equipment, see anima-assets
// mobtypes parsing + resolveActionGroup above), so the rare true-SeaMonster
// case plays the Monster variant here instead of ClassicUO's differing (often
// "no animation") one — a small, deliberate loss of fidelity consistent with
// the same collapse `resolveActionGroup`/`bodyType` already make for 0x6E.
// Likewise gargoyle-only flight variants (e.g. Emote-while-flying) aren't
// modeled — those fall back to the grounded/human variant. Returns `null` when
// ClassicUO's own mapping is "don't animate" (its `0xFF` sentinel), e.g. an
// Attack/Parry/Block/Impact/Alert/Spell sent to a mounted person.
function resolveTypedAnimGroup(typ, action, mode, body, atype, mounted) {
  const t = bodyType(body, atype); // 0 monster(+sea_monster), 1 animal, 2 people(+equipment)
  const monster = t === 0, animal = t === 1; // people/equipment: else
  switch (typ) {
    case 0: // Attack
      if (action > 10) return 0; // CUO GetObjectNewAnimationType_0: out-of-range action still plays group 0, not "no animation"
      if (monster) return mode % 4 === 1 ? 5 : mode % 4 === 2 ? 6 : 4;
      if (animal) return mode % 2 !== 0 ? 6 : 5;
      if (mounted) return action > 0 ? (action === 1 ? 27 : action === 2 ? 28 : 26) : 29;
      switch (action) {
        case 1: return 18;
        case 2: return 19;
        case 6: return 12;
        case 7: return 13;
        case 8: return 14;
        case 3: return 11;
        case 4: return 9;
        case 5: return 10;
        default: return 31;
      }
    case 1: case 2: // Parry / Block
      if (monster) return mode % 2 !== 0 ? 15 : 16;
      if (animal || mounted) return null;
      return 30;
    case 3: // Die
      if (monster) return mode % 2 !== 0 ? 2 : 3;
      if (animal) return mode % 2 !== 0 ? 21 : 22;
      return mode % 2 !== 0 ? 8 : 12;
    case 4: // Impact
      if (monster) return 10;
      if (animal) return 7;
      return mounted ? null : 20;
    case 5: // Fidget
      if (monster) return mode % 2 !== 0 ? 18 : 17;
      if (animal) return mode % 3 === 1 ? 10 : mode % 3 === 2 ? 3 : 9;
      return mounted ? null : (mode % 2 !== 0 ? 6 : 5);
    case 6: case 14: // Eat / Pillage
      if (monster) return 11;
      if (animal) return 3;
      return mounted ? null : 34;
    case 7: // Emote (e.g. .bow / .salute)
      if (mounted) return null;
      return action === 0 ? 32 : action === 1 ? 33 : 0;
    case 8: // Alert
      if (monster) return 11;
      if (animal) return 9;
      return mounted ? null : 33;
    case 9: // TakeOff
    case 10: // Land
      return monster ? 20 : null; // non-gargoyle person/animal: no anim (matches CUO)
    case 11: // Spell
      if (monster) return 12;
      if (mounted) return null;
      return action === 1 || action === 2 ? 17 : 16;
    default: // 12 StartCombat, 13 EndCombat, 15 Spawn, anything else
      return 0;
  }
}
let app, world, entLayer, mobs, overLayer, barLayer, itemLayer, guardLineLayer;
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
let lastTypedAnimSeq = 0;      // highest typed-animation (0xE2) event we've played
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

// ---- lift-rejection events (0x27 LiftRej) ----
let lastLiftRejectSeq = 0;     // highest lift-reject event seq we've already handled

// ---- item-drag completion events (0x28 EndDraggingItem / 0x29 accepted) ----
let lastDragCompletionSeq = 0; // highest drag-completion event seq we've handled

// ---- death-screen events (0x2C; actual dead state remains body-derived) ----
let lastDeathScreenSeq = 0;
let deathBannerUntil = 0;
let deathBannerTimer = null;

// ---- server-initiated container opens (0x24 DrawContainer) ----
let lastContainerOpenSeq = 0;  // highest container-open event seq we've already handled

// ---- Swing events (0x2F): briefly face the attacker toward the defender ----
let lastSwingSeq = 0;          // highest swing event seq we've already handled

// ---- server-initiated paperdoll open/refresh (0x88 DisplayPaperdoll) ----
let lastPaperdollSeq = 0;      // highest paperdoll-signal seq we've already handled
// ---- validated server external-URL requests (0xA5 OpenUrl) ----
let lastOpenUrlSeq = 0;
const openUrlQueue = [];
let openUrlWin = null;
// ---- server Tip/Notice windows (0xA6 ScrollMessage) ----
let lastTipNoticeSeq = 0;
const tipNoticeWindows = new Map(); // seq -> live DOM window
// ---- legacy modal text-entry dialogs (0xAB) ----
const textEntryWindows = new Map(); // seq -> live DOM window
const suppressedTextEntrySeqs = new Set(); // answered locally; wait for scene removal
// ---- character profile windows (0xB8) ----
const profileWindows = new Map(); // exact response seq -> live DOM window
const suppressedProfileSeqs = new Set(); // closed/saved locally; wait for scene removal
// ---- server-authorized logout (0xD1) ----
let lastLogoutAckSeq = 0;
let logoutPending = false;
// ---- High Seas smooth boat movement (0xF6) ----
let lastBoatMoveSeq = 0;
const boatGlides = new Map(); // entity serial -> queued rigid movement segments
// { serial, title, canLift } for whichever target the LAST server signal named —
// read by refreshPaperdoll() to show the real title line instead of the plain
// mobile name, when it matches the currently-displayed doll.
let pdServerInfo = null;
// ---- treasure/decoration map windows (0x90/0xF5 DisplayMap(New) + 0x56
// MapCommand) — one window per serial (unlike the paperdoll's single slot),
// so this is a per-serial seq gate, not one global counter. See
// `refreshMapWindows`'s doc for the open-vs-refresh split.
const lastMapOpenSeq = new Map(); // serial -> highest openSeq we've already opened for
// ClassicUO ServerErrorMessages._pickUpErrors, indexed by the wire reason byte;
// any reason >= 4 (including the "generic/Inspecific" 5) reads the same message
// as 4 (ClassicUO clamps `code >= 5` to `code = 4`).
const LIFT_REJECT_MSG = [
  "You can't pick that up.",           // 0 CannotLift
  "That is too far away.",             // 1 OutOfRange
  "That is out of sight.",             // 2 OutOfSight
  "That item does not belong to you.", // 3 BelongsToAnother
  "You are already holding an item.",  // 4 AlreadyHolding / generic fallback
];

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
  guardZones: false,             // guard-zone (guard line) boundary overlay — off by default
  debugMove: false,               // movement/Z debug HUD (WalkTo rejects, predicted vs server pos)
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
    + cb("abilities", "Weapon abilities")
    + cb("guardZones", "Guard-zone lines (R)")
    + cb("debugMove", "Movement debug")
    + '<div class="opt-sect">Session</div>'
    + `<button type="button" class="dlg-btn opt-logout"${logoutPending ? " disabled" : ""}>`
    + (logoutPending ? "LOGGING OUT…" : "LOG OUT") + "</button>";
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
    // SSE connects before the first poll resolves, so on a page reload it can
    // race `primeSeqRings` — a stale backlog sound could otherwise slip through
    // here before priming bumps `lastSoundSeq` past it. Bumping the seq above
    // (so poll's own replay-skip stays correct either way) without playing yet
    // covers that window; once primed, everything past the baseline plays live.
    if (!seqPrimed) return;
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
// Hue is baked into the cache key/URL (the server pre-hues each PNG), so every
// distinct dye of every item/body multiplies GPU-resident textures — an
// unbounded cache pins hundreds of MB after a long multi-town tour. Bound it
// with an LRU: texLastUsed tracks when each url was last actually drawn
// (touchTex, called on every texFor hit, PLUS a blanket per-poll sweep over every
// url a live sprite/anim-part could be showing — see forEachLiveTexUrl below,
// called from syncWorld()). Eviction only ever considers entries idle past
// TEX_IDLE_MS, so a texture a live sprite is still using — touched far more
// often than that — is never pulled out from under it; the budget (TEX_BUDGET)
// is picked high enough that ordinary town play never crosses it, so this only
// changes marathon sessions.
const texCache = new Map(), texLastUsed = new Map(), loading = new Set();
const TEX_BUDGET = 1500;          // ~200MB at UO's typical small-sprite sizes
const TEX_IDLE_MS = 5 * 60_000;   // don't evict anything touched more recently than this
const TEX_SWEEP_MS = 30_000;      // don't re-scan for eviction more than 1x/30s
let lastTexSweep = 0;
function touchTex(url) { if (url) texLastUsed.set(url, performance.now()); }
function texFor(url) {
  if (texCache.has(url)) { touchTex(url); return texCache.get(url); }
  if (!loading.has(url)) {
    loading.add(url);
    // markDirty() in the .then so a body/clothing frame that streams in gets
    // painted even while the character stands still (render-on-demand otherwise
    // wouldn't repaint an idle scene when a late texture arrives).
    PIXI.Assets.load(url).then((t) => {
      texCache.set(url, t); touchTex(url); loading.delete(url); markDirty(); sweepTexCache();
    }).catch(() => { texCache.set(url, null); touchTex(url); loading.delete(url); });
  }
  return null;
}
// Evict LRU entries once over TEX_BUDGET, throttled to at most 1 scan/TEX_SWEEP_MS
// (this can run on every texture load once near budget, so keep it cheap). Only
// evicts entries idle past TEX_IDLE_MS — see the cache's own comment above for why
// that's safe. Routes eviction through PIXI.Assets.unload(url), NOT a bare
// texture.destroy(true): Assets keeps its own url→texture cache (Loader.promiseCache
// + the top-level Cache), and unload() is what forgets the url there too — destroying
// the texture directly would leave a later PIXI.Assets.load(url) handing back the
// same (now-destroyed) Texture instead of actually reloading it.
//
// Belt-and-braces: touchTex alone isn't trusted to have caught everything (two
// real escapes found live: pruneFar's hysteresis-ring tiles, which sit on stage
// outside the "seen this poll" window loop that does the touching, and a
// mobile's st.partTex last-good fallback texture, whose OWN url only gets
// touched incidentally, not every frame it's actually the one drawn). If either
// escape (or a future one) evades touchTex bookkeeping, evicting a texture a
// live sprite still points at throws inside app.render() ("Cannot read
// properties of null (reading alphaMode)") and freezes the whole rAF loop (see
// frame()'s own resilience fix). So: build the live set fresh at sweep time and
// simply never evict anything in it, full stop, regardless of texLastUsed.
function sweepTexCache() {
  if (texCache.size <= TEX_BUDGET) return;
  const now = performance.now();
  if (now - lastTexSweep < TEX_SWEEP_MS) return;
  lastTexSweep = now;
  const live = new Set();
  forEachLiveTexUrl((u) => { if (u) live.add(u); });
  const stale = [];
  for (const [url, last] of texLastUsed) if (!live.has(url) && now - last >= TEX_IDLE_MS) stale.push([url, last]);
  if (!stale.length) return; // over budget but everything's still "recently" touched (or live) — wait
  stale.sort((a, b) => a[1] - b[1]); // oldest-touched first
  const over = texCache.size - TEX_BUDGET;
  for (let i = 0; i < Math.min(over, stale.length); i++) {
    const url = stale[i][0];
    texCache.delete(url); texLastUsed.delete(url);
    PIXI.Assets.unload(url).catch(() => {});
  }
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
// chair's pixel offset", which is what we do for those two (skipping ClassicUO's
// further per-pixel vertical-band squish shader — a deliberate, documented loss of
// fidelity; the task brief explicitly sanctions this "hold a stand frame" fallback).
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
    case 0: return { dir, group: 25, dx: 4, dy: 29 + mirrorOffsetY };  // North: mirror=true
    case 6: return { dir, group: 25, dx: -3, dy: 27 + mirrorOffsetY }; // West: mirror=false
    case 2: return { dir, group: 4, dx: 0, dy: 13 + offsetY };         // East: mirror=true
    default: return { dir, group: 4, dx: -9, dy: 14 + offsetY };       // South (4): mirror=false
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
  buildGameCursors();  // UO-style arrow / target-reticle mouse cursors over the game
  world = new PIXI.Container(); world.sortableChildren = true;
  entLayer = new PIXI.Graphics();
  mobs = new PIXI.Container();
  overLayer = new PIXI.Container(); // floating speech, always on top of the world
  // Names + HP bars (drawn over the world, non-interactive so they never eat clicks).
  barLayer = new PIXI.Container(); barLayer.eventMode = "none";
  // Invisible per-item click targets — kept BELOW `world` so a mobile sharing a
  // tile with an item wins the hit-test (mobiles are the priority).
  itemLayer = new PIXI.Container();
  // Guard-zone (guard line) overlay: above terrain/statics/items (`world`), below
  // everything entity-related (`entLayer`'s fallback dots, `mobs`, `barLayer`,
  // `overLayer`) — see the "guard-zone (guard line) boundary overlay" section.
  guardLineLayer = new PIXI.Graphics();
  app.stage.addChild(itemLayer, world, guardLineLayer, entLayer, mobs, barLayer, overLayer);

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
    if (dirty && now - lastDraw >= RENDER_MS) {
      // A render throw (e.g. a texture destroyed out from under a still-live
      // sprite — see the texture-cache eviction comments above) must not kill
      // this loop: requestAnimationFrame(frame) below would never run if
      // app.render() threw uncaught, permanently freezing the client on one bad
      // frame. Log it, force a retry next frame (markDirty), and move on.
      try { app.render(); } catch (err) { console.error("app.render() failed, skipping frame:", err); markDirty(); }
      dirty = false; lastDraw = now;
    }
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

// ---- two-stage login page: credentials → server-provided character list ----
let loginWired = false;
let characterStage = false;
let characterListKey = "";
function wireLogin() {
  if (loginWired) return; loginWired = true;
  const go = document.getElementById("lg-go");
  const backButton = document.getElementById("lg-back");
  const deleteButton = document.getElementById("lg-delete");
  const createToggle = document.getElementById("lg-create");
  const createPanel = document.getElementById("lg-create-panel");
  const createToggleRow = document.getElementById("lg-create-toggle");
  const slotRow = document.getElementById("lg-slot-row");
  const slotSelect = document.getElementById("lg-slot");
  const credentials = ["lg-host", "lg-port", "lg-user", "lg-pass"].map(id => document.getElementById(id));
  const statInputs = ["lg-str", "lg-dex", "lg-int"].map(id => document.getElementById(id));
  const updateCreation = () => {
    createPanel.classList.toggle("on", createToggle.checked);
    slotSelect.disabled = !characterStage || createToggle.checked || slotSelect.options.length === 0;
    const canDelete = characterStage && !createToggle.checked && slotSelect.options.length > 0;
    backButton.style.display = characterStage ? "block" : "none";
    backButton.disabled = !characterStage;
    deleteButton.style.display = canDelete ? "block" : "none";
    deleteButton.disabled = !canDelete;
    if (characterStage) go.textContent = createToggle.checked ? "Create" : "Play";
    const total = statInputs.reduce((sum, input) => sum + (Number(input.value) || 0), 0);
    const totalEl = document.getElementById("lg-stat-total");
    totalEl.textContent = `Total: ${total} / 90`;
    totalEl.style.color = total === 90 ? "#8896a5" : "#e5a04d";
  };
  createToggle.addEventListener("change", updateCreation);
  for (const input of statInputs) input.addEventListener("input", updateCreation);
  updateCreation();
  slotRow.style.display = "none";
  createToggleRow.style.display = "none";

  const submit = async () => {
    const host = (document.getElementById("lg-host").value || "127.0.0.1").trim();
    const port = Number(document.getElementById("lg-port").value || 2594);
    const username = (document.getElementById("lg-user").value || "").trim();
    const password = document.getElementById("lg-pass").value || "";
    const msg = document.getElementById("lg-msg");
    if (!username) { msg.textContent = "Enter an account name."; return; }

    let create = null;
    if (characterStage && createToggle.checked) {
      const name = (document.getElementById("lg-char-name").value || "").trim();
      const [strength, dexterity, intelligence] = statInputs.map(input => Number(input.value));
      if (!name) { msg.textContent = "Enter a character name."; return; }
      if (!/^[A-Za-z][A-Za-z .'-]{1,15}$/.test(name) || /[ .'-]{2}/.test(name)) {
        msg.textContent = "Use 2–16 letters with single spaces, dashes, periods, or apostrophes.";
        return;
      }
      if ([strength, dexterity, intelligence].some(value => value < 10 || value > 60)
          || strength + dexterity + intelligence !== 90) {
        msg.textContent = "STR, DEX, and INT must each be 10–60 and total 90.";
        return;
      }
      create = {
        name,
        female: document.getElementById("lg-gender").value === "female",
        profession: document.getElementById("lg-profession").value,
        strength, dexterity, intelligence,
        city_index: Number(document.getElementById("lg-city").value),
      };
    }

    msg.textContent = characterStage
      ? (create ? "Creating character…" : "Entering world…")
      : "Connecting…";
    go.disabled = true;
    backButton.disabled = true;
    try {
      const endpoint = characterStage ? "character" : "login";
      const body = characterStage
        ? (create ? { create } : { slot: Number(slotSelect.value) })
        : { host, port, username, password, interactive: true, character_slot: null, create: null };
      const response = await fetch(endpoint, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      });
      if (!response.ok) throw new Error(await response.text() || `HTTP ${response.status}`);
    } catch (error) {
      msg.textContent = "Login request failed: " + error.message;
      go.disabled = false;
      backButton.disabled = false;
    }
  };
  go.addEventListener("click", submit);
  backButton.addEventListener("click", async () => {
    if (!characterStage) return;
    const msg = document.getElementById("lg-msg");
    msg.textContent = "Returning to account login…";
    go.disabled = true;
    backButton.disabled = true;
    deleteButton.disabled = true;
    try {
      const response = await fetch("character", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ cancel: true }),
      });
      if (!response.ok) throw new Error(await response.text() || `HTTP ${response.status}`);
    } catch (error) {
      msg.textContent = "Cancel request failed: " + error.message;
      go.disabled = false;
      backButton.disabled = false;
      deleteButton.disabled = false;
    }
  });
  deleteButton.addEventListener("click", async () => {
    const option = slotSelect.selectedOptions[0];
    if (!characterStage || !option) return;
    const name = option.dataset.name || option.textContent;
    if (!window.confirm(`Permanently delete ${name}? This cannot be undone.`)) return;
    const msg = document.getElementById("lg-msg");
    msg.textContent = `Deleting ${name}…`;
    go.disabled = true;
    backButton.disabled = true;
    deleteButton.disabled = true;
    try {
      const response = await fetch("character", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ delete_slot: Number(option.value) }),
      });
      if (!response.ok) throw new Error(await response.text() || `HTTP ${response.status}`);
    } catch (error) {
      msg.textContent = "Delete request failed: " + error.message;
      go.disabled = false;
      backButton.disabled = false;
      deleteButton.disabled = false;
    }
  });
  for (const input of document.querySelectorAll("#login input, #login select"))
    input.addEventListener("keydown", (e) => { if (e.code === "Enter") submit(); });

  window.updateCharacterLoginStage = (active, slots = [], capacity = 0) => {
    characterStage = active;
    for (const input of credentials) input.disabled = active;
    slotRow.style.display = active ? "flex" : "none";
    createToggleRow.style.display = active ? "flex" : "none";
    if (!active) {
      characterListKey = "";
      createToggle.checked = false;
      go.textContent = "Connect";
      updateCreation();
      return;
    }
    const key = JSON.stringify([slots, capacity]);
    if (key !== characterListKey) {
      characterListKey = key;
      slotSelect.replaceChildren(...slots.map(slot => {
        const option = document.createElement("option");
        option.value = String(slot.index);
        option.textContent = `Slot ${slot.index + 1} — ${slot.name}`;
        option.dataset.name = slot.name;
        return option;
      }));
      const full = slots.length >= capacity;
      createToggle.disabled = full;
      createToggle.checked = slots.length === 0 && !full;
    }
    go.textContent = createToggle.checked ? "Create" : "Play";
    updateCreation();
  };
}
// True when a key event is going to a text field (login form, etc.), so the global
// game-input handler must not consume it (otherwise letters like a/w/s/d/m/b/t —
// movement + hotkeys — never reach the field and typing drops characters).
function isTypingTarget(el) {
  if (!el) return false;
  const t = el.tagName;
  return t === "INPUT" || t === "TEXTAREA" || t === "SELECT" || el.isContentEditable;
}
function showLogin(auth, msg, slots, capacity) {
  wireLogin();
  const el = document.getElementById("login");
  if (el) el.classList.add("on");
  const m = document.getElementById("lg-msg");
  const go = document.getElementById("lg-go");
  if (auth === "characters") {
    window.updateCharacterLoginStage(true, slots || [], capacity || 0);
    if (m) m.textContent = "Choose a character or create one in an empty slot.";
    if (go) go.disabled = false;
  } else if (auth === "connecting") {
    if (m) m.textContent = msg || "Connecting…";
    if (go) go.disabled = true;
  } else if (auth === "error") {
    window.updateCharacterLoginStage(false);
    if (m) m.textContent = "Login failed: " + (msg || "unknown error");
    if (go) go.disabled = false;
  } else {
    window.updateCharacterLoginStage(false);
    if (m) m.textContent = msg || "";
    if (go) go.disabled = false;
  }
}
function hideLogin() {
  const el = document.getElementById("login");
  if (el && el.classList.contains("on")) el.classList.remove("on");
}

// ---- seq-ring priming (skip a stale backlog replay on page reload) ----
// Every event "ring" above (character anims 0x6E/0xE2, damage 0x0B, effects
// 0x70/0xC0/0xC7, lift-rejects 0x27, container-opens 0x24, swings 0x2F,
// paperdoll 0x88, external URLs 0xA5, tips/notices 0xA6, sounds 0x54) is keyed
// by a monotonic `seq` that lives in the
// anima-net play server's `World`, NOT on this page: reloading the browser
// resets every `lastXSeq` variable above to 0, but the live ServUO session
// (and its already-fired backlog under those seqs) keeps running underneath —
// it's tied to the server connection, not the tab. Left unprimed, the very
// first poll after a reload would treat that whole backlog as "new": stale
// animations/damage numbers/sounds replay once, and — worse — *sticky*
// signals like the paperdoll and the last container-open pop their windows
// back open even though nothing just happened.
//
// Fix: on the FIRST scene ingest after page load, bump every ring's last-seen
// seq up to its current max WITHOUT running the per-event handler, then flip
// `seqPrimed`. Every later poll runs ingestX()/playSounds() as normal, so a
// genuinely new event (seq beyond this baseline) still fires immediately.
let seqPrimed = false;
let wasInWorld = false;
function maxSeq(arr) {
  let m = 0;
  if (arr) for (const ev of arr) { const sq = ev.seq | 0; if (sq > m) m = sq; }
  return m;
}
function primeSeqRings(s) {
  lastAnimSeq = Math.max(lastAnimSeq, maxSeq(s.anims));
  lastTypedAnimSeq = Math.max(lastTypedAnimSeq, maxSeq(s.tanims));
  lastDamageSeq = Math.max(lastDamageSeq, maxSeq(s.damage));
  lastEffectSeq = Math.max(lastEffectSeq, maxSeq(s.effects));
  lastLiftRejectSeq = Math.max(lastLiftRejectSeq, maxSeq(s.liftRejects));
  lastDragCompletionSeq = Math.max(lastDragCompletionSeq, maxSeq(s.dragCompletions));
  if (s.deathScreen) lastDeathScreenSeq = Math.max(lastDeathScreenSeq, s.deathScreen.seq | 0);
  lastContainerOpenSeq = Math.max(lastContainerOpenSeq, maxSeq(s.containerOpens));
  lastSwingSeq = Math.max(lastSwingSeq, maxSeq(s.swings));
  lastSoundSeq = Math.max(lastSoundSeq, maxSeq(s.sounds));
  if (s.paperdoll) lastPaperdollSeq = Math.max(lastPaperdollSeq, s.paperdoll.seq | 0);
  lastOpenUrlSeq = Math.max(lastOpenUrlSeq, maxSeq(s.openUrls));
  lastTipNoticeSeq = Math.max(lastTipNoticeSeq, maxSeq(s.tips));
  if (s.logoutAck) lastLogoutAckSeq = Math.max(lastLogoutAckSeq, s.logoutAck.seq | 0);
  lastBoatMoveSeq = Math.max(lastBoatMoveSeq, maxSeq(s.boatMoves));
  // Per-serial, unlike the rings above — see `lastMapOpenSeq`'s doc.
  if (s.maps) for (const m of s.maps) lastMapOpenSeq.set(m.serial >>> 0, m.openSeq | 0);
}

async function poll() {
  const t0 = performance.now();
  try {
    const r = await fetch("scene.json?" + Date.now());
    if (!r.ok) throw new Error(r.status);
    scene = await r.json();
    // Not in world yet (login-page mode): show the login form instead of rendering.
    if (scene && scene.auth) {
      // A completed/lost game session owns a large amount of DOM and seq-gated
      // renderer state. Reload once on the world→login transition so none of it
      // leaks into the next character; the new page sees auth immediately and
      // therefore does not loop.
      if (wasInWorld) { window.location.reload(); return; }
      showLogin(scene.auth, scene.msg, scene.slots, scene.capacity);
      return;
    }
    wasInWorld = true;
    hideLogin();
    if (!seqPrimed) { primeSeqRings(scene); seqPrimed = true; }
    ingestBoatMoves(scene);
    updateAnimStates(scene);
    const ts = performance.now();
    syncWorld(scene); // diffs only — no full rebuild
    diag.sync = performance.now() - ts;
    markDirty(); // a fresh poll may change tiles/entities → redraw once
    ingestSpeech(scene); // float new speech above its speaker
    ingestAnims(scene); // play new character animations (0x6E: combat swings, bows…)
    ingestTypedAnims(scene); // play new typed animations (0xE2: emotes, gestures, alerts…)
    ingestDamage(scene); // float new combat damage numbers (0x0B)
    ingestEffects(scene); // spawn new graphical effects (0x70/0xC0/0xC7)
    ingestLiftRejects(scene); // clear the held item + show a message (0x27 LiftRej)
    ingestDragCompletions(scene); // reconcile held-item cursor acknowledgements (0x28/0x29)
    ingestDeathScreen(scene); // start ClassicUO's short death banner timer (0x2C)
    ingestContainerOpens(scene); // open a window for each server-initiated container open (0x24)
    ingestOpenUrls(scene); // ask before opening each validated external URL (0xA5)
    ingestSwings(scene); // briefly face the attacker toward the defender (0x2F Swing)
    ingestPaperdoll(scene); // open/refresh a paperdoll the server told us to show (0x88)
    refreshMapWindows(scene); // treasure/decoration map windows (0x90/0xF5 + 0x56)
    refreshTip(); // update the hover tooltip if its OPL just arrived/changed
    drawMinimap(scene);
    updateGuardZones(scene); // guard-zone overlay: refetch on facet change, redraw clipped to view
    refreshBuffs(scene); // reconcile the buff/debuff bar with scene.buffs
    refreshAbilities(); // keep the weapon special-ability bar in sync with the equipped weapon
    if (wmOn) drawWorldmap();  // keep the open world map tracking the player
    if (scene.player) hud(scene);
    updateMoveDebug(scene); // movement/Z debug HUD (Options → "Movement debug")
    refreshPaperdoll();   // keep the paperdoll live (equip/stats change)
    if (spellbookOn) { refreshSpellMana(); refreshSpellbookContent(); } // keep the spellbook live (mana + book content)
    if (skillsOn) refreshSkills();  // keep the skills window live (values/locks change)
    checkSkillGains(scene);  // announce skill base changes as journal system messages
    refreshParty();   // keep the party panel live + surface incoming invites (0xBF/0x06)
    refreshContainers();  // keep open container windows live (items move/disappear)
    refreshShop(scene);   // vendor buy/sell window (auto-opens on scene.shop)
    refreshGumps(scene);  // server-sent generic gumps/dialogs (0xB0/0xDD)
    refreshLegacyMenus(scene); // legacy icon/question menus (0x7C)
    refreshHuePickers(scene); // server dye color pickers (0x95)
    refreshTipNotices(scene); // pageable tips / close-only notices (0xA6)
    refreshTextEntryDialogs(scene); // legacy modal text-entry dialogs (0xAB)
    refreshProfiles(scene); // editable self / read-only character profiles (0xB8)
    refreshLogoutAck(scene); // restore UI if the server denied a 0xD1 logout
    refreshPopup(scene);  // right-click context menu (0xBF/0x14)
    refreshBook(scene);   // open book reader (0x93/0xD4 + 0x66)
    refreshPrompt(scene); // server text-prompt dialog (0x9A ASCII / 0xC2 Unicode)
    refreshTrade(scene);  // secure trade window(s) (0x6F), one per session, auto-open/close
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

// 0x2C is a one-shot screen effect, while `player.dead` is authoritative state
// derived from ServUO's complete ghost-body set. ClassicUO keeps the grayscale
// effect for the whole ghost lifetime but shows “You are dead” for only 1.5s.
function ingestDeathScreen(s) {
  const ev = s && s.deathScreen;
  if (!ev) return;
  const seq = ev.seq | 0;
  if (seq <= lastDeathScreenSeq) return;
  lastDeathScreenSeq = seq;
  deathBannerUntil = performance.now() + 1500;
  if (deathBannerTimer != null) clearTimeout(deathBannerTimer);
  deathBannerTimer = setTimeout(() => {
    deathBannerTimer = null;
    deathBannerUntil = 0;
    updateDeathUI(scene);
  }, 1500);
}

// The body fallback keeps this renderer compatible with an older scene producer.
// Both clear on resurrection when the body reverts to a living id.
function updateDeathUI(s) {
  const p = s && s.player;
  const dead = !!(p && (typeof p.dead === "boolean" ? p.dead : isGhostBody(p.body)));
  const map = document.getElementById("map");
  if (map) map.classList.toggle("dead", dead);
  const banner = document.getElementById("deadbanner");
  if (banner) banner.style.display = dead && performance.now() < deathBannerUntil ? "block" : "none";
}

// ClassicUO BoatMovingManager velocity table. A segment starts when its 0xF6
// reaches this renderer; bursts are queued instead of collapsing intermediate
// tiles, keeping the hull and every passenger on one rigid timeline.
function boatMoveDuration(speed) {
  speed |= 0;
  if (speed === 2) return 1000;
  if (speed === 3) return 500;
  if (speed === 4) return 250;
  if (speed > 4) return speed * 10;
  return 500;
}

function queueBoatGlide(serial, from, to, duration, now) {
  serial >>>= 0;
  let queue = boatGlides.get(serial);
  if (!queue) { queue = []; boatGlides.set(serial, queue); }
  while (queue.length && now >= queue[0].end) queue.shift();
  const previous = queue.length ? queue[queue.length - 1] : null;
  const start = previous ? previous.end : now;
  const source = previous ? previous.to : from;
  queue.push({
    from: { x: Number(source.x), y: Number(source.y), z: Number(source.z || 0) },
    to: { x: Number(to.x), y: Number(to.y), z: Number(to.z || 0) },
    start,
    end: start + duration,
  });
  // A background tab can receive a short burst when it wakes. Keep enough
  // segments to preserve those intermediate tiles instead of jumping ahead.
  if (queue.length > 32) {
    const first = queue[0], latest = queue[queue.length - 1];
    const t = now <= first.start ? 0 : Math.min(1, (now - first.start) / (first.end - first.start));
    const current = {
      x: first.from.x + (first.to.x - first.from.x) * t,
      y: first.from.y + (first.to.y - first.from.y) * t,
      z: first.from.z + (first.to.z - first.from.z) * t,
    };
    queue.splice(0, queue.length, { from: current, to: latest.to, start: now, end: now + duration });
  }
}

function boatVisual(serial, fallback, now) {
  const queue = boatGlides.get(serial >>> 0);
  if (!queue) return { ...fallback, active: false };
  while (queue.length && now >= queue[0].end) queue.shift();
  if (!queue.length) {
    boatGlides.delete(serial >>> 0);
    return { ...fallback, active: false };
  }
  const segment = queue[0];
  const t = now <= segment.start ? 0 : Math.min(1, (now - segment.start) / (segment.end - segment.start));
  return {
    x: segment.from.x + (segment.to.x - segment.from.x) * t,
    y: segment.from.y + (segment.to.y - segment.from.y) * t,
    z: segment.from.z + (segment.to.z - segment.from.z) * t,
    active: true,
  };
}

function ingestBoatMoves(s) {
  const now = performance.now();
  for (const movement of (s && s.boatMoves) || []) {
    const seq = Number(movement.seq) || 0;
    if (!seq || seq <= lastBoatMoveSeq) continue;
    lastBoatMoveSeq = seq;
    const duration = boatMoveDuration(movement.speed);
    for (const entity of movement.entities || []) {
      if (!entity.from || !entity.to) continue;
      queueBoatGlide(entity.serial, entity.from, entity.to, duration, now);
    }
  }
}

function updateAnimStates(s) {
  const now = performance.now();
  const seen = new Set();
  // The chair we're seated on disappeared/moved/changed graphic (someone else used
  // it, a GM deleted it, …) — stand up rather than leave the avatar seated on thin
  // air. Cheap: once per poll (~150ms), not per rendered frame.
  if (sitting && !(s.items || []).some((it) => (it.x | 0) === sitting.x && (it.y | 0) === sitting.y && (it.g | 0) === sitting.graphic)) {
    standUp();
  }
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
      // A real committed step is always more authoritative than a cosmetic
      // Swing-flash facing (see `ingestSwings`/`drawMobs`) — drop it now
      // rather than waiting out its ~350ms timer.
      st.faceOverride = null;
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
    const boatPos = boatVisual(p.serial, { x: p.x, y: p.y, z: p.z ?? 0 }, now);
    if (boatPos.active) {
      pred.steps.length = 0; pred.t0 = 0;
      pred.x = p.x; pred.y = p.y; pred.z = p.z ?? pred.z; pred.dir = p.dir ?? pred.dir;
      pred.rx = boatPos.x; pred.ry = boatPos.y; pred.rz = boatPos.z; pred.moving = true;
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
  // Beyond this Chebyshev distance from the player, `m.tiles` is the grayed-out
  // land-only context ring (server sends land but no statics out there — see
  // scene.rs's `beyond_view`). Older/degraded scenes without `viewRange` treat
  // the whole window as "in view" (no ring), matching the old behaviour.
  const viewRange = (m.viewRange != null) ? m.viewRange : m.radius;
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
      // Grayed-out context ring past the player's actual view range (ClassicUO-
      // style "you remember the land is there"): land only, dimmed/desaturated,
      // no art/texmap loads — see the render branch below and its `gray` pool flag.
      const beyondView = Math.max(Math.abs(col - m.radius), Math.abs(row - m.radius)) > viewRange;
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
      // Unchanged: nothing to rebuild. (LRU freshness for every pool entry —
      // including tiles this loop never revisits, e.g. pruneFar's hysteresis
      // ring — is stamped once per poll in one blanket pass; see
      // forEachLiveTexUrl()/touchTex() at the end of this function.)
      // Unchanged real tile: nothing to rebuild. A colour-fallback tile is re-evaluated
      // so it can upgrade to real art — but only rebuild it once the texture actually
      // arrives; while it's still pending (or the art is missing) keep the existing
      // placeholder instead of re-creating an identical Graphics every poll. A gray
      // beyond-view tile (below) never has real art to upgrade to, so it's pooled
      // with `fallback: false` and skips this re-check entirely — UNLESS `beyondView`
      // itself flipped since the last poll (the player crossed the draw-distance
      // boundary for this tile), which must force a rebuild into/out of grayscale.
      if (e && e.g === t.g && e.z === z0 && !!e.gray === beyondView) {
        if (!e.fallback) continue;
        if (texFor(e.url) == null) continue;
      }
      // corner heights (ClassicUO: top=this, right=(x+1,y), bottom=(x+1,y+1), left=(x,y+1)).
      // At the window's SE edge a neighbour falls outside the grid (null); rather than
      // skip the tile — which flashes the black page background at the diamond's rim —
      // fall back to this tile's own Z so it still renders (flat).
      const z1 = zAt(x + 1, y) ?? z0, z2 = zAt(x + 1, y + 1) ?? z0, z3 = zAt(x, y + 1) ?? z0;
      const sloped = !(z0 === z1 && z1 === z2 && z2 === z3);

      // Until the art/texmap PNG has streamed in, draw a flat diamond in the tile's
      // server-provided average colour instead of `continue`-ing (which would leave the
      // black page background showing through). The `fallback` flag makes the unchanged-
      // check above re-evaluate it every poll until the real texture resolves.
      let sp, texUrl, fallback = false;
      if (beyondView) {
        // Grayscale, dimmed diamond — no textured art or statics ever load for
        // this ring (the server never sends them out here; see scene.rs), so
        // this branch never touches texFor and never sets `fallback`.
        const L = Math.min(255, Math.round((0.3 * t.c[0] + 0.59 * t.c[1] + 0.11 * t.c[2]) * 0.65 + 95));
        sp = makeColorTile(x, y, z0, z1, z2, z3, [L, L, L]);
      } else if (!sloped) {
        texUrl = `art/land/${t.g}.png`;
        const tex = texFor(texUrl);
        if (tex) sp = makeFlatTile(x, y, z0, tex);
        else { sp = makeColorTile(x, y, z0, z0, z0, z0, t.c); fallback = true; }
      } else if (t.tx > 0) {
        texUrl = `texmap/${t.tx}.png`; // seamless texture for slopes
        const tex = texFor(texUrl);
        if (tex) sp = makeStretchedTile(x, y, z0, z1, z2, z3, tex, true);
        else { sp = makeColorTile(x, y, z0, z1, z2, z3, t.c); fallback = true; }
      } else {
        texUrl = `art/land/${t.g}.png`; // no texmap → stretch the art
        const tex = texFor(texUrl);
        if (tex) sp = makeStretchedTile(x, y, z0, z1, z2, z3, tex, false);
        else { sp = makeColorTile(x, y, z0, z1, z2, z3, t.c); fallback = true; }
      }
      if (e) { world.removeChild(e.sp); e.sp.destroy(); }
      world.addChild(sp);
      tilePool.set(key, { sp, g: t.g, z: z0, url: texUrl, fallback, gray: beyondView });
    }
  }
  // Server now sends statics across the whole land window, including the
  // grayed-out beyond-view ring — gray those to match the land tiles there
  // (see STATIC_GRAY above). `beyondView` is per-static, re-evaluated every
  // poll so a static flips gray/color as the player walks past `VR`.
  const P = s.player || { x: 0, y: 0 };
  const VR = (s.map && s.map.viewRange != null) ? s.map.viewRange : (s.map ? s.map.radius : 18);
  for (const st of s.statics || []) {
    const key = `${st.x},${st.y},${st.g},${st.z}`;
    seenS.add(key);
    const beyondView = Math.max(Math.abs(st.x - P.x), Math.abs(st.y - P.y)) > VR;
    if (staticPool.has(key)) {
      // Unchanged sprite identity, but the player may have crossed the view
      // boundary since the last poll — flip its filter without rebuilding it.
      const ex = staticPool.get(key);
      if (ex && ex._gray !== beyondView) {
        ex.filters = beyondView ? [STATIC_GRAY] : null;
        ex._gray = beyondView;
      }
      continue; // unchanged; see the blanket LRU-touch note above
    }
    const texUrl = `art/static/${st.g}.png`;
    const tex = texFor(texUrl);
    if (!tex) continue;
    const sp = new PIXI.Sprite(tex);
    sp.anchor.set(0.5, 1.0);
    sp.x = isoX(st.x, st.y); sp.y = isoY(st.x, st.y, st.z) + HALF;
    sp.zIndex = depthZ(st.x, st.y, st.pz ?? st.z, 4);
    sp._gray = beyondView;
    sp.filters = beyondView ? [STATIC_GRAY] : null;
    if (st.ms != null) {
      sp._boatSerial = st.ms >>> 0;
      sp._boatBaseX = st.x; sp._boatBaseY = st.y; sp._boatBaseZ = st.z;
      sp._boatBaseSpriteX = sp.x; sp._boatBaseSpriteY = sp.y;
      sp._boatPzOffset = (st.pz ?? st.z) - st.z; sp._boatDepthBias = 4;
    }
    // Tile + foliage flag for the transparency pass (circle-of-transparency / foliage fade).
    sp._tx = st.x; sp._ty = st.y; sp._foliage = !!st.f;
    sp._texUrl = texUrl; // so the "still on screen" branch above can keep it LRU-fresh
    // Animated static (flames/fountains/water wheels): the server baked the ART
    // tile-id frame sequence (`a`) + per-frame interval ms (`ai`). Prefetch each
    // frame's texture and store them so the animation pass can swap sp.texture.
    if (Array.isArray(st.a) && st.a.length > 1) {
      const frameUrls = st.a.map((id) => `art/static/${id}.png`);
      sp._frames = frameUrls.map((u) => texFor(u));
      sp._frameUrls = frameUrls;   // so tickAnimatedStatics/touch can re-resolve/re-stamp by url
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
    // A corpse (graphic 0x2006) carries the dead creature's Corpse.def-remapped
    // body, facing and death-pose group from the server (see scene.rs). Once that
    // anim's frame count AND the last frame's texture have both loaded, draw the
    // held death-pose frame instead of the generic corpse art; until then (or if
    // the anim is absent) `corpseUrl` stays null and we fall through to the static
    // art below, same as any other item.
    let corpseUrl = null, corpseFrame = -1, corpseTex = null;
    if (it.g === 0x2006 && it.body != null) {
      const dir = it.dir & 7, dg = it.dg | 0;
      framesFor(it.body, dg, dir); // kick the animinfo (frame-count/centers) load
      const fk = `${it.body}/${dg}/${dir}`;
      const loaded = frameCount.has(fk) ? frameCount.get(fk) : 0;
      if (loaded > 0) {
        corpseFrame = loaded - 1; // the death pose's final (held) frame
        const url = `anim/${it.body}/${dg}/${dir}/${corpseFrame}.png` + (it.hue ? `?hue=${it.hue}` : "");
        const t = texFor(url);
        if (t) { corpseUrl = url; corpseTex = t; }
      }
    }
    const e = itemPool.get(key);
    if (e && e.g === it.g && e.x === it.x && e.y === it.y && e.z === iz && e.corpseUrl === corpseUrl) continue; // unchanged; see the blanket LRU-touch note above
    const itemTexUrl = corpseUrl || `art/static/${it.g}.png`;
    const tex = corpseTex || texFor(itemTexUrl);
    if (!tex) continue; // await art, retry next poll
    if (e) { world.removeChild(e.sp); e.sp.destroy(); }
    const sp = new PIXI.Sprite(tex);
    const x = isoX(it.x, it.y), y = isoY(it.x, it.y, iz);
    // A resolved death-pose frame anchors by its draw-center, same as a mobile's
    // anim frames (see drawMobs' `part()`); otherwise (loading, or a non-corpse
    // item) foot-anchor like any static.
    const c = corpseUrl ? centerFor(it.body, it.dg | 0, it.dir & 7, corpseFrame) : null;
    if (c) {
      sp.anchor.set(0, 0);
      sp.x = x - c[0]; sp.y = (y - 3) - tex.height - c[1];
    } else {
      sp.anchor.set(0.5, 1.0);
      sp.x = x; sp.y = y + HALF;
    }
    sp.zIndex = depthZ(it.x, it.y, it.pz ?? iz, 5); // bias 5: just above same-tile statics
    sp._boatSerial = it.serial >>> 0;
    sp._boatBaseX = it.x; sp._boatBaseY = it.y; sp._boatBaseZ = iz;
    sp._boatBaseSpriteX = sp.x; sp._boatBaseSpriteY = sp.y;
    sp._boatPzOffset = (it.pz ?? iz) - iz; sp._boatDepthBias = 5;
    // Tile + foliage flag for the transparency pass (circle-of-transparency / foliage fade).
    sp._tx = it.x; sp._ty = it.y; sp._foliage = !!it.f;
    sp.eventMode = "static"; sp.cursor = "pointer";
    const serial = it.serial;
    sp.on("pointerdown", (ev) => onEntityPointerDown(serial, ev, true)); // world item → loot on dbl-click
    sp.on("pointerover", () => { hoverEntity(serial); targetHighlightOn(sp); });
    sp.on("pointerout", () => { hoverOut(serial); targetHighlightOff(sp); });
    world.addChild(sp);
    itemPool.set(key, { sp, g: it.g, x: it.x, y: it.y, z: iz, corpseUrl, url: itemTexUrl });
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
  // The static/item pool may have changed shape this poll (new sprite could need
  // fading right where we already stand) — force one more full transparencyPass
  // scan next frame regardless of whether the player's tile moved. See the flag's
  // own comment above transparencyPass().
  transparencyDirty = true;
  // Stamp EVERY texture a live sprite/anim-part could currently be showing as
  // "just used", once per poll — not only the ones this function's own diff
  // loops happen to touch. Two real escapes found live, both live sprites the
  // window loops above never revisit: (1) pruneFar's hysteresis-ring tiles
  // (kept in tilePool at Chebyshev radius+1..+4 — on stage/screen for camera-
  // slide hysteresis — but outside the span×span window loop that walks
  // m.tiles); (2) a mobile's st.partTex last-good fallback (drawMobs reuses an
  // old, already-drawn texture the instant a frame's current url hasn't
  // resolved yet, without going through texFor/touchTex for THAT texture's own
  // url). Left stale past TEX_IDLE_MS, either could get destroyed out from
  // under an on-stage sprite by sweepTexCache → app.render() throws → the rAF
  // loop dies (see frame()'s own resilience fix). ~1.3k Map.set calls at a
  // typical view radius — trivial next to the rest of this function.
  forEachLiveTexUrl(touchTex);
}
// Every texture url currently referenced by a live, on-stage sprite or a
// mobile's per-part fallback (drawMobs's st.partTex — see part() below for why
// it stores {tex,url} pairs, not just the texture). Two call sites: (1) a
// per-poll touchTex sweep (syncWorld, above) so none of these ever look "idle"
// to sweepTexCache's LRU scan; (2) sweepTexCache's own belt-and-braces
// exclude-list, so even a url this pass fails to touch can never be evicted
// while still referenced live.
function forEachLiveTexUrl(fn) {
  for (const e of tilePool.values()) fn(e.url);
  for (const sp of staticPool.values()) {
    fn(sp._texUrl);
    if (sp._frameUrls) for (const u of sp._frameUrls) fn(u);
  }
  for (const e of itemPool.values()) fn(e.url);
  for (const st of anim.values()) {
    if (st.partTex) for (const e of st.partTex.values()) fn(e.url);
  }
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

// A flat-colour diamond placeholder for a land tile whose art/texmap PNG hasn't
// streamed in yet (or a window-edge tile with no neighbour to slope against). Uses
// the server-provided average colour `c` so terrain never flashes the black page
// background during load/scroll; replaced by the textured tile on a later poll once
// texFor() resolves. Corner heights follow the same layout as makeStretchedTile.
function makeColorTile(x, y, z0, z1, z2, z3, c) {
  const Bx = (x - y) * HALF, By = (x + y) * HALF;
  const g = new PIXI.Graphics();
  g.poly([
    Bx,        By - HALF - z0 * ZSTEP, // top
    Bx + HALF, By        - z1 * ZSTEP, // right
    Bx,        By + HALF - z2 * ZSTEP, // bottom
    Bx - HALF, By        - z3 * ZSTEP, // left
  ]).fill(Array.isArray(c) && c.length === 3 ? ((c[0] << 16) | (c[1] << 8) | c[2]) : 0x101418);
  const avgZ = Math.abs(z0 - z2) <= Math.abs(z3 - z1) ? (z0 + z2) >> 1 : (z3 + z1) >> 1;
  g.zIndex = depthZ(x, y, avgZ - 2, 0);
  return g;
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
// Mobile.ProcessSteps): X/Y advance by the step's time fraction; Z eases toward
// the step's target at its own decoupled catch-up rate (see below); a completed
// step commits to the base and the next begins (carrying the time remainder for
// continuous motion). Turns are consumed instantly (facing only).
function processSteps(now, dt) {
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
      // Keep the queued step's Z target current. It was captured once at enqueue
      // time from whatever `scene.map` snapshot was on hand then; for a tile the
      // server's scene build hadn't yet resolved authoritatively (see scene.rs
      // `sz_chain`) that snapshot can be a stale/cheap guess. As fresher polls
      // land — updating `scene.map` in place — re-read it here every frame so a
      // correction arrives as a smooth mid-glide adjustment instead of only
      // surfacing later via the at-rest reconcile (the visible "pop").
      const freshSz = tileSZ(s.x, s.y);
      if (freshSz !== null) s.z = freshSz;
      pred.rx = pred.x + (s.x - pred.x) * prog;
      pred.ry = pred.y + (s.y - pred.y) * prog;
      // Z eases from the SOURCE tile's Z (`pred.z`, still the pre-step tile until
      // this step commits) to the step's target `s.z`, but FASTER than x/y — it
      // fully resolves within the first `ZEASE_FRAC` of the step (ClassicUO's
      // `Offset.Z = (destZ-srcZ) * x * 4/frames`: Z is done in the first ~4
      // frames, well before the tile boundary). An ease-out shapes it so the
      // vertical speed doesn't lurch at the start. This is FRAME-RATE INDEPENDENT
      // (locked to `prog`, not accumulated per `dt`) and, crucially, reaches the
      // target BEFORE the step commits — so nothing trails into the next step:
      // the old exponential chase left ~8% unresolved every step, which on a
      // real staircase (+5 risers) read as the avatar floating ~2-3px below the
      // steps while climbing, then a snap-up at the top step's commit followed by
      // the descent — the "climbs, jerks up, then comes down" bounce.
      const zt = Math.min(1, prog / ZEASE_FRAC);
      const ze = 1 - (1 - zt) * (1 - zt); // ease-out quad
      pred.rz = pred.z + (s.z - pred.z) * ze;
      pred.dir = s.dir; pred.moving = true;
    }
    if (prog >= 1) {                       // step complete → commit base, carry remainder
      // Commit the base; rz is already at s.z (the mid-step ease resolves Z within
      // the first ZEASE_FRAC of the step), so setting it here is just a belt-and-
      // braces exactness — no residual to snap, which is what removes the
      // staircase bounce.
      if (!s.turn) { pred.x = s.x; pred.y = s.y; pred.z = s.z; pred.rz = s.z; }
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

function applyBoatSpriteGlides(now) {
  const move = (sp) => {
    if (sp._boatSerial == null) return;
    const base = { x: sp._boatBaseX, y: sp._boatBaseY, z: sp._boatBaseZ };
    const visual = boatVisual(sp._boatSerial, base, now);
    const dx = isoX(visual.x, visual.y) - isoX(base.x, base.y);
    const dy = isoY(visual.x, visual.y, visual.z) - isoY(base.x, base.y, base.z);
    sp.x = sp._boatBaseSpriteX + dx;
    sp.y = sp._boatBaseSpriteY + dy;
    sp.zIndex = depthZ(
      visual.x,
      visual.y,
      visual.z + (sp._boatPzOffset || 0),
      sp._boatDepthBias || 4,
    );
    if (visual.active) markDirty();
  };
  for (const sp of staticPool.values()) move(sp);
  for (const entry of itemPool.values()) move(entry.sp);
}

function renderFrame(dt) {
  if (!scene) return;
  const now = performance.now();
  moveIntent = activeMove();   // mouse (RMB) or held keys → drives prediction
  // Safety net: a teleport/recall/GM move while seated (activeMove() only stands us
  // up on a fresh movement *intent*, not on the real position jumping under us) —
  // don't leave the avatar looking stuck on a now-distant chair.
  if (sitting && pred && cheby(Math.round(pred.x) - sitting.x, Math.round(pred.y) - sitting.y) > 1) standUp();
  // Player: append predicted steps while a key is held, then interpolate the queue.
  const me = anim.get("self");
  if (me && pred) {
    enqueueSteps(now);
    processSteps(now, dt);
    let carriedByBoat = false;
    if (scene.player) {
      const boatPos = boatVisual(
        scene.player.serial,
        { x: pred.rx, y: pred.ry, z: pred.rz ?? pred.z },
        now,
      );
      if (boatPos.active) {
        pred.rx = boatPos.x; pred.ry = boatPos.y; pred.rz = boatPos.z; pred.moving = true;
        carriedByBoat = true;
        markDirty();
      }
    }
    me.rx = pred.rx; me.ry = pred.ry; me.rz = pred.rz; me.z = pred.z; me.dir = pred.dir;
    // Boat offsets carry a standing passenger without playing a walk cycle.
    me.animMoving = carriedByBoat ? false : pred.moving;
    me.stepDur = stepDelay(!!(moveIntent && moveIntent.run), mounted());
    // Leg cadence tied to GROUND COVERED (cyclesPerTile): walking unchanged
    // (80ms/frame); running takes bigger strides so the legs don't whirl. Phase
    // is a 0..1 cycle fraction.
    me.animPhase = me.animMoving
      ? ((me.animPhase || 0) + cyclesPerTile(!!(moveIntent && moveIntent.run)) * dt / (me.stepDur || 300)) % 1
      : 0;
    if (scene.player) me.body = scene.player.body;
  }
  // Glide the OTHER entities (mobiles) toward their target tile at constant
  // velocity, timed to their measured step cadence (×1.12 margin so they're still
  // moving when the next tile arrives). The player ("self") is driven by the queue
  // above, not this glide. Snap on big jumps.
  for (const [id, st] of anim) {
    if (st === me) continue;
    const serial = id.startsWith("m") ? Number(id.slice(1)) : 0;
    const boatPos = serial
      ? boatVisual(serial, { x: st.tx, y: st.ty, z: st.z | 0 }, now)
      : { active: false };
    if (boatPos.active) {
      st.rx = boatPos.x; st.ry = boatPos.y; st.rz = boatPos.z;
      st.animMoving = false; st.animPhase = 0;
      markDirty();
      continue;
    }
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
  applyBoatSpriteGlides(now);
  // camera follows the eased player so the avatar stays centered (eased z too).
  // Seated: follow the chair TILE (not the sprite's small pixel nudge, see
  // trySit()/chairSeatFor()) so the camera settles exactly like it would after any
  // other one-tile step, with the avatar still centered.
  const self = anim.get("self");
  if (self) {
    const camX = sitting ? isoX(sitting.x, sitting.y) : isoX(self.rx, self.ry);
    const camY = sitting ? isoY(sitting.x, sitting.y, sitting.z) : isoY(self.rx, self.ry, self.rz ?? self.z);
    app.stage.position.set(app.screen.width / 2 - camX, app.screen.height / 2 - camY);
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
    if (!tex) { tex = texFor(sp._frameUrls ? sp._frameUrls[idx] : `art/static/${sp._afids[idx]}.png`); frames[idx] = tex; }
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
// Full-pool scan is only worth redoing when it could actually change: the player
// crossed a tile (fade radius is tile-relative) or syncWorld just rebuilt part of
// the static/item pool (a new/removed sprite could need (un)fading even at the
// same tile). `transparencyDirty` is set at the end of every syncWorld() call
// (each poll, ~150ms) — cheap insurance against tracking exact pool deltas —
// so this still cuts the scan from every rendered frame (60Hz) to at most once
// per poll plus once per tile crossing, instead of every single frame.
let transparencyDirty = true;
let lastCotKey = null;
function transparencyPass() {
  let ptx, pty, pz;
  if (sitting) { ptx = sitting.x; pty = sitting.y; pz = sitting.z; } // seated: fade around the chair, not the (unmoved) real tile
  else if (pred) { ptx = Math.round(pred.rx); pty = Math.round(pred.ry); pz = pred.z; }
  else if (scene && scene.player) { ptx = scene.player.x; pty = scene.player.y; pz = scene.player.z; }
  else return;
  const cotKey = ptx + "," + pty + "," + (pz | 0);
  if (!transparencyDirty && cotKey === lastCotKey) return; // player tile unchanged, pool unchanged
  lastCotKey = cotKey; transparencyDirty = false;
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

// Every ServUO `Body.IsGhost` id and the living people body used to animate it.
// 970 is the legacy male death-shroud body (`H_Male_Robe_Deathshroud`).
const GHOST_ANIMATION_BODIES = new Map([
  [402, 400], [403, 401], // human male/female
  [607, 605], [608, 606], // elf male/female
  [694, 666], [695, 667], // gargoyle male/female
  [970, 400],             // legacy male death shroud
]);
const isGhostBody = (b) => GHOST_ANIMATION_BODIES.has(b | 0);
const ghostAnimationBody = (b) => GHOST_ANIMATION_BODIES.get(b | 0) ?? (b | 0);

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
  // drawMobs() runs every rendered frame (60Hz) for animation, but `scene` itself
  // only changes once per ~150ms poll (poll() assigns a brand-new parsed object
  // each time) — so build this lookup once per scene and stash it there; a fresh
  // scene object naturally invalidates it, no separate epoch/dirty flag needed.
  let mobById = scene._mobById;
  if (!mobById) {
    mobById = new Map();
    for (const m of scene.mobiles || []) mobById.set("m" + m.serial, m);
    scene._mobById = mobById;
  }
  for (const [id, st] of anim) {
    diag.ents++;
    // A cosmetic Swing (0x2F) flash (see `ingestSwings`) briefly overrides the
    // DISPLAYED facing without touching `st.dir`/`pred.dir` — those stay 100%
    // driven by the committed walk stream (server confirms / local prediction),
    // which is what `enqueueSteps`' turn-vs-move split (mirroring anima-core
    // `Walker::step`'s `is_turn = facing != dir`) reads. Expire it on time, or
    // the instant `touch()` (in `updateAnimStates`) sees this entity actually
    // move a tile — a real step is always more authoritative than the flash.
    let faceDir = st.dir;
    if (st.faceOverride) {
      if (performance.now() < st.faceOverride.until) faceDir = st.faceOverride.dir;
      else st.faceOverride = null;
    }
    // We only know run/mount state for our own player; other mobiles walk/stand.
    const isSelf = id === "self";
    // Sitting (chair double-click, see trySit()) is a pure render overlay: while
    // seated, the local avatar's facing/pose come from the chair-table resolution
    // instead of the real predicted state — nothing below this ever touches World
    // or `pred`.
    const d = (isSelf && sitting) ? sitting.dir : (faceDir & 7);
    const moving = (isSelf && sitting) ? false : !!st.animMoving; // set in renderFrame (glide + held/mouse)
    const running = isSelf && !!(moveIntent && moveIntent.run);
    // Look up this entity's scene record (self → player; else mobile) for skin hue,
    // worn equipment, and mount state. Mount is per-entity: self uses player.mounted,
    // others use their own `mounted`/`mountAnim` fields.
    const ent = isSelf ? scene.player : mobById.get(id);
    const mounted = !!(ent && ent.mounted);
    const mountAnim = (ent && (ent.mountAnim | 0)) || 0;
    // Ghost bodies use their race/sex-equivalent living people animation, rendered
    // translucent with equipment hidden. Self uses the bridge's authoritative dead
    // bit; nearby mobiles fall back to the same complete body mapping.
    const ghost = ent && typeof ent.dead === "boolean" ? ent.dead : isGhostBody(st.body);
    const bodyAnim = ghost ? ghostAnimationBody(st.body) : (st.body | 0);
    // Hidden (mobile-update status-flags 0x80: Hiding/stealth skill, or a GM
    // `[set Hidden true`). Seeing it at all means the server allows us to
    // perceive this mobile (self, or an ally in Detect Hidden range) — UO
    // gives visual feedback for that by rendering it semi-transparent.
    const hidden = !!(ent && ent.hidden);
    // Authoritative animation type from the server (mobtypes.txt). A ghost is drawn
    // with a living human body, so it animates as people (2) regardless of st.at.
    const atype = ghost ? 2 : (st.at != null ? (st.at | 0) : null);
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
    // resolveActionGroup() folds those onto the body's real group set; a *typed*
    // 0xE2 event instead needs resolveTypedAnimGroup()'s ClassicUO-style dispatch,
    // which can also decide there's nothing to play (e.g. an emote while mounted).
    const ag = (act && !ghost)
      ? (act.typed
          ? resolveTypedAnimGroup(act.typ, act.action, act.mode, bodyAnim, atype, mounted)
          : resolveActionGroup(act.group, bodyAnim, atype))
      : 0;
    if (act && !ghost) {
      if (ag == null) {
        st.act = null; // no valid animation for this body/mount combo — revert now
      } else {
        framesFor(bodyAnim, ag, d); // kick the frame-count/centers load
        const fk = `${bodyAnim}/${ag}/${d}`;
        const loaded = frameCount.has(fk) ? Math.max(1, frameCount.get(fk)) : 0;
        const fi = Math.floor((performance.now() - act.startMs) / act.frameMs);
        if (loaded > 0 && fi >= loaded) st.act = null; // played every frame → done
      }
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
      group = animGroup(moving, running, mounted, bodyAnim, inWar, atype);
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
    // Seated overrides whatever the above picked (even a pending action anim —
    // ClassicUO's TryGetSittingInfo/seated-draw path takes priority unconditionally
    // too): frame 0 of chairSeatFor()'s group, always. framesFor() here just kicks
    // off loading that (group,d)'s frame-count/centers so centerFor() below
    // positions the sprite correctly instead of falling back to the foot anchor.
    if (isSelf && sitting) {
      group = sitting.group;
      frames = framesFor(bodyAnim, group, d);
      frame = 0;
    }
    const skinHue = ent && ent.hue ? ent.hue : 0;
    // Compose the character from stable PARTS (mount, body, each worn layer). Two
    // fixes for the walk/run "naked↔dressed" flicker and the layer-swap bug:
    //  • PER-PART last-good texture (`st.partTex`): when a part's texture for the
    //    current frame is still loading, reuse its previous frame instead of dropping
    //    it — so no layer (or the body) ever vanishes for a frame mid-walk. Stored as
    //    {tex,url} pairs (not just the texture): the url is what forEachLiveTexUrl()
    //    touches every poll, so a fallback that's the ONLY thing currently drawn for
    //    a part doesn't go idle in texLastUsed and get evicted out from under it —
    //    its own url is never re-passed through texFor()/touchTex() while it's the
    //    fallback (the current frame's url is what's being requested instead).
    //  • STABLE per-part keys + rank-based z (not a shifting array index): a layer
    //    that's momentarily missing no longer shoves the others into different slots
    //    and swaps their textures.
    if (!st.partTex) st.partTex = new Map();
    const entries = [];
    // bodyId/grp/frm identify the source frame so we can fetch its draw-center and
    // position the part correctly (ClassicUO math) rather than foot-anchoring it.
    const part = (key, url, rank, interactive, bodyId, grp, frm) => {
      let t = url ? texFor(url) : null;
      if (t) st.partTex.set(key, { tex: t, url });
      else { const fb = st.partTex.get(key); t = fb ? fb.tex : null; }
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
    // Seated: draw at the chair's tile + ClassicUO's pixel nudge (chairSeatFor())
    // instead of the real predicted position — the avatar visually "sits down" onto
    // the seat while the actual World/prediction state never changes (trySit()).
    const x = (isSelf && sitting) ? isoX(sitting.x, sitting.y) + sitting.dx : isoX(st.rx, st.ry);
    const y = (isSelf && sitting) ? isoY(sitting.x, sitting.y, sitting.z) + sitting.dy : isoY(st.rx, st.ry, st.rz ?? st.z);
    if (entries.length) {
      entries.sort((a, b) => a.rank - b.rank);
      // zIndex only changes when the mobile crosses a tile (assigning it forces a
      // re-sort). All parts share the body's depth; a rank epsilon (≪ the per-z step
      // of 16) keeps them back→front regardless of which parts are present this frame.
      const zi = (isSelf && sitting)
        ? depthZ(sitting.x, sitting.y, sitting.z + 1, 8)
        : depthZ(Math.round(st.rx), Math.round(st.ry), st.z + 1, 8);
      for (const e of entries) {
        const key = id + "#" + e.key;
        let sp = mobSprites.get(key);
        if (!sp) {
          sp = new PIXI.Sprite(e.tex);
          sp.anchor.set(0.5, 1.0);
          // Only the body is the click target; mount/clothing/hair never eat clicks.
          // Clicking YOURSELF is a real interaction too (single-click = your name in
          // your notoriety colour, double-click = your paperdoll), so the "self" body
          // is a click target like any other mobile — its serial comes from scene.player.
          const clickSerial = id === "self"
            ? (scene.player ? ((scene.player.serial >>> 0) + "") : null)
            : (e.interactive ? id.slice(1) : null);
          if (clickSerial != null) {
            sp.eventMode = "static";
            sp.cursor = "pointer";
            sp.on("pointerdown", (ev) => onEntityPointerDown(clickSerial, ev));
            // OPL tooltip on hover (same flow as world items) + target highlight.
            sp.on("pointerover", () => { hoverEntity(clickSerial); targetHighlightOn(sp); });
            sp.on("pointerout", () => { hoverOut(clickSerial); targetHighlightOff(sp); });
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
        // Dead humans render as translucent ghosts; a hidden mobile (still visible to
        // us, per the scene's `hidden` flag) renders semi-transparent too, so we know
        // we're hidden even though we can see ourselves. Sprites are pooled/persistent,
        // so alpha must be reset to 1 every frame for a body that is neither (else a
        // former ghost/hidden mobile stays faint after it dies again/unhides).
        sp.alpha = ghost ? 0.45 : (hidden ? 0.5 : 1);
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

// ---- overhead speech (ClassicUO MessageManager / overhead text) ----
const OVERHEAD_HEAD = 68;   // px above the feet anchor — clears the head (incl. hats/hair)

// Client-side system messages (skill gains, etc.) that aren't in the server journal.
// They render after the server's journal lines in `hud()`. Capped FIFO so old notices
// don't pile up forever at the bottom.
const localJournal = [];           // { text } — newest last
const LOCAL_JOURNAL_MAX = 40;
// Monotonic counter, bumped on every addSysMessage — NOT derived from length or
// newest-text. Once localJournal is at its cap, pushing another line no longer
// changes .length, and a new line whose text happens to repeat the previous one
// wouldn't change "newest text" either; either alone would make hud()'s journal
// change-signature miss a real append and silently drop the line from the DOM.
let localJournalSeq = 0;
function addSysMessage(text) {
  localJournal.push({ text });
  localJournalSeq++;
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
  const c = o.fc || msgColor(o.type, o.hue); // fc = a forced colour (e.g. notoriety name)
  if (o._c !== c) { o.el.style.color = c; o._c = c; }
}

// Float a mobile's name above its head in its notoriety colour (single-click, like
// ClassicUO). Works for yourself too (scene.player carries `noto`); for others the
// name/notoriety come from the mobile (or its OPL if the name hasn't loaded yet).
function showNameOverhead(serial, tries) {
  if (!scene) return;
  const sv = serial >>> 0;
  const isSelf = scene.player && sv === (scene.player.serial >>> 0);
  let name, noto;
  // ServUO doesn't send your OWN notoriety (self arrives via 0x20/0x22, which carry
  // no noto byte), so it comes through as 0 — default yourself to Innocent (blue), the
  // classic "your name" colour. Other mobiles carry real notoriety (crim/murderer/…).
  if (isSelf) { name = scene.player.name; noto = (scene.player.noto | 0) || 1; }
  else {
    const m = (scene.mobiles || []).find((x) => (x.serial >>> 0) === sv);
    if (!m) return;
    name = m.name || (scene.opl && scene.opl[sv] && scene.opl[sv][0]) || "";
    noto = m.noto | 0;
  }
  if (!name) {                       // name not loaded yet — the server click fetches its
    if ((tries | 0) < 2) setTimeout(() => showNameOverhead(sv, (tries | 0) + 1), 400); // OPL; retry
    return;
  }
  name = name.replace(/\s+/g, " ").trim(); // OPL names can carry tabs ("Carl\tthe tailor")
  const id = isSelf ? "self" : "m" + sv;
  if (!anim.has(id)) return;         // not in view
  const el = document.createElement("div");
  el.className = "oh-label oh-name";
  el.textContent = name;
  namesEl().appendChild(el);
  const o = { id, text: name, type: 6, hue: 0, born: performance.now(), ttl: 3000, el, _c: null,
              fc: cssColor(notoColor(noto)) };
  applyOverheadColor(o);
  overheads.push(o);
  while (overheads.length > 40) { const x = overheads.shift(); if (x.el) x.el.remove(); }
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

// Play new *typed* animation events (0xE2): an emote/gesture/alert/… on a mobile.
// Unlike 0x6E, `typ`/`act` aren't a raw animation group — resolveTypedAnimGroup()
// (called from drawMobs, where the body/mount state is known) converts them.
// ClassicUO never uses the wire "delay" as a timing value here (SetAnimation is
// called with the default interval), so — unlike ingestAnims — we don't stretch
// frameMs by it; it's kept only as `mode` for the per-body group resolver.
function ingestTypedAnims(s) {
  if (!s || !s.tanims) return;
  const now = performance.now();
  const pserial = s.player ? (s.player.serial >>> 0) : 0;
  for (const ev of s.tanims) {
    const seq = ev.seq | 0;
    if (seq <= lastTypedAnimSeq) continue;
    lastTypedAnimSeq = seq;
    const serial = ev.serial >>> 0;
    const id = serial === pserial ? "self" : "m" + serial;
    const st = anim.get(id);
    if (!st) continue;                               // actor not in view
    st.act = { typed: true, typ: ev.typ | 0, action: ev.act | 0, mode: ev.mode | 0,
               fwd: true, startMs: now, frameMs: CHAR_ANIM_FRAME_MS };
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

// The server refused our last pickup (0x27 LiftRej): the item never left its
// source, so just clear the held drag-ghost locally — NOT a drop (nothing ever
// moved, so sending one would wrongly ask the server to place an item it never
// gave us) — and surface the reason as a system journal line, for each `seq`
// newer than the last we handled.
function ingestLiftRejects(s) {
  if (!s || !s.liftRejects) return;
  for (const ev of s.liftRejects) {
    const seq = ev.seq | 0;
    if (seq <= lastLiftRejectSeq) continue;
    lastLiftRejectSeq = seq;
    if (cursorItem) clearCursorItem();
    const reason = ev.reason | 0;
    addSysMessage(LIFT_REJECT_MSG[reason] || LIFT_REJECT_MSG[LIFT_REJECT_MSG.length - 1]);
  }
}

// Reconcile the two legacy server acknowledgements that make ClassicUO release
// its held-item cursor. Our placement UI is optimistic: it clears the ghost as
// soon as it sends drop/equip, so pendingPlacements identifies acknowledgements
// for those already-finished operations. In that case we consume the pending
// entry without touching a newer item the user may now be holding. With no
// pending placement, the server is explicitly ending the active drag and we
// mirror ClassicUO by clearing it.
function ingestDragCompletions(s) {
  if (!s || !s.dragCompletions) return;
  for (const ev of s.dragCompletions) {
    const seq = ev.seq | 0;
    if (seq <= lastDragCompletionSeq) continue;
    lastDragCompletionSeq = seq;

    let pendingIndex = -1;
    if ((ev.packet | 0) === 0x28 && ev.token != null) {
      const token = ev.token >>> 0;
      pendingIndex = pendingPlacements.indexOf(token);
    }
    if (pendingIndex < 0 && pendingPlacements.length) pendingIndex = 0;
    if (pendingIndex >= 0) {
      pendingPlacements.splice(pendingIndex, 1);
    } else if (cursorItem) {
      clearCursorItem();
    }
  }
}

// The server itself opened a container we did NOT double-click ourselves (0x24
// DrawContainer — a banker's "bank" speech, a GM `[bank`, a snoop pick, …).
// Reuses the same openContainer() window our own double-clicks build.
function ingestContainerOpens(s) {
  if (!s || !s.containerOpens) return;
  for (const ev of s.containerOpens) {
    const seq = ev.seq | 0;
    if (seq <= lastContainerOpenSeq) continue;
    lastContainerOpenSeq = seq;
    openContainer(ev.serial >>> 0);
  }
}

// The 8-direction (dx,dy sign) -> UO facing lookup, inverting DIR_DELTA. `dx`/`dy`
// MUST be integer TILE deltas (like ClassicUO's own facing math) — feeding it eased
// render-position deltas (`rx`/`ry`) is wrong: sub-tile easing residue (e.g. rx a
// hair ahead of ry while both are converging on the same tile) makes `Math.sign`
// see a nonzero component on an axis that's actually settled, turning a true
// cardinal facing into a diagonal.
function dirToward(dx, dy) {
  const sx = Math.sign(dx), sy = Math.sign(dy);
  if (!sx && !sy) return null;
  const d = DIR_DELTA.findIndex(([ddx, ddy]) => ddx === sx && ddy === sy);
  return d < 0 ? null : d;
}

// Integer TILE coordinates for an anim-map id, for facing math (see `dirToward`'s
// doc) — never the eased render position. "self" has no `tx`/`ty` on its anim
// entry (only `pred` tracks its committed base tile; see `updateAnimStates`);
// every other entity's anim entry carries the server's current tile as `tx`/`ty`.
function tileOf(id) {
  if (id === "self") return pred ? { x: pred.x, y: pred.y } : null;
  const st = anim.get(id);
  return st ? { x: st.tx, y: st.ty } : null;
}

// The server just told us `attacker` swung at `defender` (0x2F Swing) — purely
// cosmetic feedback: briefly face the attacker toward the defender via a
// render-layer-only override (see `drawMobs`'s `faceOverride` handling). Never
// write `st.dir`/`pred.dir` here — those belong exclusively to the committed
// walk stream (server confirms / local prediction), and `enqueueSteps`' turn-
// vs-move split (mirroring anima-core `Walker::step`'s `is_turn = facing !=
// dir`) reads `pred.dir` as "the player's actual current facing". Stomping it
// with a combat-facing flash desyncs that split from the server's real state,
// causing a one-tile mispredict (a phantom turn-then-move) the instant you walk
// right after swinging — the server's real position then arrives and the
// client rubber-bands to correct it.
function ingestSwings(s) {
  if (!s || !s.swings) return;
  const now = performance.now();
  const pserial = s.player ? (s.player.serial >>> 0) : 0;
  for (const ev of s.swings) {
    const seq = ev.seq | 0;
    if (seq <= lastSwingSeq) continue;
    lastSwingSeq = seq;
    const attacker = ev.attacker >>> 0, defender = ev.defender >>> 0;
    const isSelf = attacker === pserial;
    const aId = isSelf ? "self" : "m" + attacker;
    const dId = defender === pserial ? "self" : "m" + defender;
    const a = anim.get(aId);
    if (!a) continue;                               // attacker isn't in view
    const at = tileOf(aId), dt = tileOf(dId);
    if (!at || !dt) continue;                        // either lacks a known tile yet
    const dir = dirToward(dt.x - at.x, dt.y - at.y);
    if (dir == null) continue;
    a.faceOverride = { dir, until: now + 350 }; // ~350ms flash; drawMobs expires/clears it
  }
}

// The server just told us to show a paperdoll (0x88 DisplayPaperdoll) — sent on
// every double-click of a mobile (ours or another's), even a re-click of the
// same one after we'd closed its window; `seq` never repeats, so each request
// (re)opens/refreshes regardless of local dismiss state. Prefer this over the
// client-side body-range guess in onEntityPointerDown (kept as a fallback for
// a shard that never sends this at all) — it's authoritative and carries the
// real title line.
function ingestPaperdoll(s) {
  const p = s && s.paperdoll;
  if (!p) return;
  const seq = p.seq | 0;
  if (seq <= lastPaperdollSeq) return;
  lastPaperdollSeq = seq;
  const serial = p.serial >>> 0;
  const pserial = s.player ? (s.player.serial >>> 0) : 0;
  pdServerInfo = { serial, title: p.title || "", canLift: !!p.canLift };
  pdTarget = serial === pserial ? null : serial;
  paperdollOn = true;
  const pd = document.getElementById("paperdoll");
  pd.classList.add("on"); pd._sig = null;
  refreshPaperdoll();
}

function spawnEffect(ev, now) {
  let frames = (ev.frames && ev.frames.length) ? ev.frames : [ev.g | 0];
  const hue = ev.hue | 0;
  // animdata interval is a small tick count; clamp to a lively per-frame range.
  let fm = (ev.interval | 0) > 0 ? Math.min(150, Math.max(50, (ev.interval | 0) * 50)) : 80;
  // Lightning (kind 1) has no ART animation — its graphic arrives as 0. ClassicUO
  // draws it as the 10-frame lightning GUMP strip (0x4E20..0x4E29, ~50ms/frame,
  // additive); mirror that instead of drawing art tile 0 (the "UNUSED" placeholder).
  if ((ev.kind | 0) === 1) {
    frames = [20000, 20001, 20002, 20003, 20004, 20005, 20006, 20007, 20008, 20009];
    fm = 50;
  }
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
  if ((ev.kind | 0) === 1) sprite.blendMode = "add"; // lightning: additive flash, like ClassicUO
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
    // Lightning frames are GUMP art (0x4E20 strip); everything else is ART tiles.
    const base = o.kind === 1 ? `gump/${g}.png` : `art/static/${g}.png`;
    const tex = texFor(base + (o.hue ? `?hue=${o.hue}` : ""));
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
// Poison overrides the fraction color entirely: in UO the health bar turns a
// distinct bright green while poisoned, independent of remaining HP, so it
// reads as "poisoned" rather than "healthy" even at a glance. Deliberately a
// cleaner/brighter green than the >50% healthy green (0x46a758) so the two
// don't get confused.
const POISON_COLOR = 0x2fd44a;

// Greedy vertical de-collide for floating name labels — the same greedy-AABB
// idea as the world map's POI label declutter (wmPlaceLabel: an AABB per label,
// skip/keep against everything already placed this pass), except here we NUDGE
// a colliding label straight up instead of hiding it — 2+ named mobiles standing
// close together would otherwise render illegible stacked/overlapping text.
// `boxes` accumulates this pass's already-placed labels (fresh array per
// drawBars() call); returns the (possibly nudged) CSS y to use as the label's
// bottom anchor (labels are positioned bottom-center via
// `transform: translate(-50%,-100%)` — see `.nm-label` in index.html).
function placeNameLabel(boxes, cx, bottom, w, h) {
  const pad = 2;
  let top = bottom - h, moved = true, guard = 0;
  while (moved && guard++ < 16) {
    moved = false;
    for (const b of boxes) {
      if (cx - w / 2 < b.r && cx + w / 2 > b.l && top < b.b + pad && bottom > b.t - pad) {
        bottom = b.t - pad; top = bottom - h; moved = true; // push above whatever it collided with
      }
    }
  }
  boxes.push({ l: cx - w / 2, r: cx + w / 2, t: top, b: bottom });
  return bottom;
}

// Draw a name + HP bar above each OTHER mobile, anchored to its interpolated iso
// position (like the overhead speech). Objects are cached per serial and only
// redrawn when their value/notoriety changes; pruned when the mobile leaves view.
function drawBars(now) {
  if (!scene) return;
  const seen = new Set();
  const nameBoxes = []; // this pass's placed name-label boxes, for placeNameLabel()
  const nameJobs = []; // {d, serial, cx, naturalBottom, nm} — laid out after the loop, once, in a deterministic order
  let changed = false;
  const lastAttack = (scene.lastAttack | 0) >>> 0; // current auto-attack target (0 = none)
  // The server's authoritative combat opponent (0xAA ChangeCombatant) — usually
  // the same mobile as lastAttack, but the server can retarget on its own (e.g. a
  // pet defending itself), so it's tracked + highlighted separately.
  const combatant = (scene.combatant | 0) >>> 0;
  for (const m of scene.mobiles || []) {
    const id = "m" + m.serial;
    const st = anim.get(id);
    if (!st) continue;                       // not yet interpolated / left view
    const x = isoX(st.rx, st.ry);
    const feetY = isoY(st.rx, st.ry, st.rz ?? st.z);
    const topY = feetY - BAR_HEAD;       // name + target marker: above the head
    const barY = feetY + 2;              // HP bar: down at the feet (ClassicUO-style)
    // Is this the current attack target (ours or the server's combatant)?
    // Highlight its bar + draw a marker.
    const serial = m.serial >>> 0;
    const tgt = (lastAttack !== 0 && serial === lastAttack) || (combatant !== 0 && serial === combatant);
    // --- HP bar (only when the server gave us hits/hitsMax) ---
    if (settings.bars && (m.hitsMax | 0) > 0) {
      seen.add(id);
      let g = hpBars.get(id);
      if (!g) { g = new PIXI.Graphics(); barLayer.addChild(g); hpBars.set(id, g); changed = true; }
      const frac = Math.max(0, Math.min(1, m.hits / m.hitsMax));
      const poisoned = !!m.poisoned;
      if (g._frac !== frac || g._noto !== m.noto || g._tgt !== tgt || g._poisoned !== poisoned) {
        g.clear();
        // dark backing + notoriety-tinted border, then the health fill. The current
        // target gets a thicker bright-red border so it stands out.
        g.rect(-BAR_W / 2 - 1, -1, BAR_W + 2, BAR_H + 2).fill({ color: 0x000000, alpha: 0.6 })
         .stroke({ color: tgt ? 0xff2d2d : notoColor(m.noto), width: tgt ? 2 : 1 });
        // Bar length still reflects the real HP fraction — only the color signals
        // poison (a poisoned mobile at 20% HP still shows a short bar, just green).
        if (frac > 0) g.rect(-BAR_W / 2, 0, BAR_W * frac, BAR_H).fill(poisoned ? POISON_COLOR : hpColor(frac));
        g._frac = frac; g._noto = m.noto; g._tgt = tgt; g._poisoned = poisoned;
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
      // Only a text change invalidates the cached size — mark it for the read
      // phase below instead of measuring inline (that would force a reflow per
      // label per frame, since the loop just dirtied this div's layout).
      if (d._t !== nm) { d.textContent = nm; d._t = nm; d._measure = true; }
      if (d._noto !== m.noto) { d.style.color = cssColor(notoColor(m.noto)); d._noto = m.noto; }
      const fx = window.innerWidth / app.renderer.width, fy = window.innerHeight / app.renderer.height;
      const cx = (app.stage.x + x) * fx;
      const naturalBottom = (app.stage.y + topY - 2) * fy;
      nameJobs.push({ d, serial, cx, naturalBottom, nm });
    } else {
      const d = nameDivs.get(id); if (d) d.style.display = "none";
    }
  }
  // Read phase: only labels whose text changed this frame (d._measure, set
  // above) need a fresh offsetWidth/Height — that's the only thing that can
  // actually change a label's size, so steady state (no text changes) reads
  // nothing and forces zero reflows.
  for (const { d, nm } of nameJobs) {
    if (d._measure) {
      // A generous estimate covers the very first frame, before the div has
      // ever been laid out (offsetWidth/Height are 0 pre-layout).
      d._w = d.offsetWidth || (nm.length * 7 + 10);
      d._h = d.offsetHeight || 15;
      d._measure = false;
    }
  }
  // Write phase: sort deterministically (by on-screen bottom, then serial) so
  // the greedy de-collide pass below always visits labels in the same order
  // regardless of scene.mobiles' iteration order (a Rust HashMap — unordered,
  // so it can reshuffle frame-to-frame as mobiles enter/leave view). Without
  // this, which label gets nudged out of a collision could swap arbitrarily.
  nameJobs.sort((a, b) => a.naturalBottom - b.naturalBottom || a.serial - b.serial);
  for (const { d, cx, naturalBottom } of nameJobs) {
    const bottom = placeNameLabel(nameBoxes, cx, naturalBottom, d._w, d._h);
    d.style.left = cx + "px";
    d.style.top = bottom + "px";
    d.style.display = "block";
  }
  // Prune name/bar objects whose mobile left view (don't leak PIXI objects / DOM).
  for (const [id, g] of hpBars) if (!seen.has(id)) { barLayer.removeChild(g); g.destroy(); hpBars.delete(id); changed = true; }
  for (const [id, d] of nameDivs) if (!seen.has(id)) { d.remove(); nameDivs.delete(id); }
  for (const [id, mk] of tgtMarkers) if (!seen.has(id)) { barLayer.removeChild(mk); mk.destroy(); tgtMarkers.delete(id); changed = true; }
  if (changed) markDirty(); // first appearance / value change → repaint once
}

// Movement/Z debug overlay (Options → "Movement debug", settings.debugMove).
// Diagnoses: (a) a walkto that silently failed server-side — the server pushes
// a "System: walkto ..." journal line on rejection/abandonment, which this
// surfaces prominently; (b) stair/Z-transition weirdness — shows the
// server-authoritative (x,y,z) next to the eased client-predicted (rx,ry,rz)
// so a mismatch/lag is visible. Runs off the existing ~150ms poll cycle (not
// per animation frame) and is a pure no-op when the setting is off.
function updateMoveDebug(s) {
  const el = document.getElementById("movedbg");
  if (!el) return;
  if (!settings.debugMove || !s || !s.player) { el.style.display = "none"; return; }
  el.style.display = "block";
  const p = s.player;
  const self = anim.get("self"); // eased predicted state (see updateAnimStates)
  const fmt = (v) => (typeof v === "number" ? v.toFixed(1) : "-");
  const notes = (s.journal || [])
    .filter((j) => j.name === "System" && (j.text || "").startsWith("walkto"))
    .slice(-3);
  let html = `<div>server (${p.x}, ${p.y}, ${p.z})</div>`
    + `<div>eased (${fmt(self && self.rx)}, ${fmt(self && self.ry)}, ${fmt(self && self.rz)})</div>`;
  for (const n of notes) html += `<div class="mdbg-note">${n.text}</div>`;
  el.innerHTML = html;
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
  // Signature-skip (same pattern as the paperdoll/skills/party panels, `_sig`
  // stashed on the element): hud() runs every ~150ms poll, but the journal itself
  // usually hasn't grown since the last one — rebuilding its whole DOM unchanged
  // is pure waste.
  //
  // Server half: the interactive play server stamps every line with a monotonic
  // `seq` (anima-net/src/bin/play_server.rs), so the newest line's seq is a cheap,
  // reliable change signal there. The non-interactive scene-bin FILE mode
  // (anima-net/src/bin/scene.rs) never emits `seq` at all AND caps the journal at
  // 12 lines — a seq-or-length-only signature would stop changing forever the
  // moment that cap is first hit, even as lines keep rotating through, freezing
  // the panel. So: use seq when the newest line actually has one, else fall back
  // to a full-content signature (cheap here — that mode's array is ≤12 long).
  const jSrc = s.journal || [];
  const jLen = jSrc.length;
  const jLastLine = jLen ? jSrc[jLen - 1] : null;
  const jTail = jLastLine && jLastLine.seq != null
    ? jLastLine.seq
    : jSrc.map((l) => (l.name || "") + "" + (l.text || "")).join("");
  // Local half: a monotonic counter (bumped in addSysMessage, not derived from
  // length/newest-text) so a repeated-text line, or the ring hitting its own cap,
  // still registers as a change.
  const jSig = `${jLen}:${jTail}:${localJournalSeq}`;
  if (j._sig !== jSig) {
    j._sig = jSig;
    // Keep following the newest line only if already scrolled to the bottom (don't
    // yank the view while the user is reading back).
    const atBottom = j.scrollHeight - j.scrollTop - j.clientHeight < 24;
    j.innerHTML = "";
    for (const line of jSrc) {
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
  }
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
function makeDraggable(win, handle, onMove) {
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
      if (onMove) onMove(x, y);
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
// `graphic` = the school's spellbook ITEM id (world/equip/backpack graphic, as
// opposed to `book`'s GUMP art id above) — ServUO `Spellbook` subclass
// constructors. Used to match a known 0xBF/0x1B content entry (scene.spellbooks)
// to its school, and to find the player's own book of that school among their
// equip/backpack items (see `knownSpellbookFor`/`findOwnSpellbook`). Mastery has
// none: Skill Masteries aren't cast from a real spellbook's bit-mask content in
// the same way (ServUO `BookOfMasteries` uses its own gump), so that school
// always renders at full brightness regardless of `scene.spellbooks`.
const SPELL_SCHOOLS = [
  { key: "magery", label: "Magery", book: 0x08AC, graphic: 0x0EFA, iconStart: 0x08C0, spells: MAGERY_PAIRS },
  { key: "necromancy", label: "Necro", book: 0x2B00, graphic: 0x2253, iconStart: 0x5000, spells: NECROMANCY_SPELLS },
  { key: "chivalry", label: "Chivalry", book: 0x2B01, graphic: 0x2252, iconStart: 0x5100, spells: CHIVALRY_SPELLS },
  { key: "bushido", label: "Bushido", book: 0x2B07, graphic: 0x238C, iconStart: 0x5400, spells: BUSHIDO_SPELLS },
  { key: "ninjitsu", label: "Ninjitsu", book: 0x2B06, graphic: 0x23A0, iconStart: 0x5300, spells: NINJITSU_SPELLS },
  { key: "spellweaving", label: "Weaving", book: 0x2B2F, graphic: 0x2D50, iconStart: 0x59D8, spells: SPELLWEAVING_SPELLS },
  { key: "mysticism", label: "Mysticism", book: 0x2B32, graphic: 0x2D9D, iconStart: 0x5DC0, spells: MYSTICISM_SPELLS },
  { key: "mastery", label: "Mastery", book: 0x08AC, graphic: 0, iconStart: 0x0945, spells: MASTERY_SPELLS },
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
// Find the scene.spellbooks entry for `school` (matched by the book's ITEM
// graphic), or null if we don't know that school's content yet (book never
// opened this session, or `school.graphic` is 0 — Mastery). Callers must treat
// null as "unknown", NOT "empty book", and leave that school's rendering as
// it was before this feature existed (every spell at full brightness).
function knownSpellbookFor(school) {
  if (!school.graphic) return null;
  const list = (scene && scene.spellbooks) || [];
  return list.find((b) => ((b.graphic | 0) & 0xffff) === school.graphic) || null;
}
// Is global spell id `id` ABSENT from `book`'s 64-bit content mask? `content`
// arrives from anima-net split into two u32 halves, `lo` (bits 0..31) and `hi`
// (bits 32..63) — see `build_scene`'s doc for why (JS Number precision).
function spellMissing(book, id) {
  const bit = id - book.offset;
  if (bit < 0 || bit > 63) return true; // outside this book's range entirely
  const half = bit < 32 ? (book.lo >>> 0) : (book.hi >>> 0);
  return ((half >>> (bit % 32)) & 1) === 0;
}
// Find the player's own spellbook item of the given ITEM graphic: worn on the
// one-handed slot (Layer.OneHanded == 1) or sitting in any container we know
// the contents of (usually the backpack). null if we don't currently see one.
function findOwnSpellbook(graphic) {
  const p = scene && scene.player;
  if (p && p.equip) {
    const worn = p.equip.find((e) => (e.layer | 0) === 1 && ((e.g | 0) & 0xffff) === graphic);
    if (worn) return worn.serial >>> 0;
  }
  const items = (scene && scene.contItems) || [];
  const it = items.find((i) => ((i.g | 0) & 0xffff) === graphic);
  return it ? (it.serial >>> 0) : null;
}
// ServUO only ever sends 0xBF/0x1B spellbook content in reply to actually
// opening the book (Spellbook.OnDoubleClick → DisplayTo). So when the K window
// opens, double-click (via the existing use:<serial> plumbing) every VISIBLE
// school's book we haven't already asked about. The container dblclick handler
// already treats a spellbook specially (toggles this same window instead of
// opening a container view — see `isSpellbook`), and DisplayTo's other traffic
// (a repeat world/equip/container-slot packet, plus a 0x24 DisplaySpellbook we
// don't even parse) is otherwise a harmless no-op, so this has no visible side
// effect beyond the content arriving. Sent at most once per book serial ever,
// so reopening K repeatedly doesn't re-spam the request.
const spellbookContentRequested = new Set();
function requestUnknownSpellbookContents() {
  for (const school of VISIBLE_SCHOOLS) {
    if (!school.graphic || knownSpellbookFor(school)) continue; // Mastery, or already known
    const serial = findOwnSpellbook(school.graphic);
    if (serial == null || spellbookContentRequested.has(serial)) continue;
    spellbookContentRequested.add(serial);
    sendInput("use:" + serial);
  }
}
function renderSpellSchool() {
  const book = document.getElementById("sb-book");
  const school = VISIBLE_SCHOOLS.find((s) => s.key === spellSchool) || VISIBLE_SCHOOLS[0];
  const isMagery = school.key === "magery";
  const known = knownSpellbookFor(school); // null = content not known → don't dim anything
  let html = "", lastCircle = 0;
  school.spells.forEach(([id, name], idx) => {
    if (isMagery) {
      const circle = Math.floor(idx / 8) + 1;        // 8 spells per circle
      if (circle !== lastCircle) { html += `<div class="sp-circle">Circle ${circle}</div>`; lastCircle = circle; }
    }
    const info = isMagery ? MAGERY_INFO[name] : null;
    const iconId = school.iconStart + idx;           // k-th spell icon = iconStart + k
    // A spell the book doesn't actually contain is dimmed but still clickable —
    // there's no local rule enforcement, the server just refuses the cast.
    const missing = known != null && spellMissing(known, id);
    const title = missing ? `Cast ${name} (not in this book)` : `Cast ${name}`;
    // The icon is draggable out onto the screen → a floating quick-cast button.
    html += `<div class="sp-row${missing ? " sp-missing" : ""}" data-id="${id}" data-icon="${iconId}" data-name="${name}" title="${title}">`
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
// Re-render the current school once new spellbook content arrives (scene.
// spellbooks changed) so the K window doesn't stay frozen at whatever it knew
// the moment it opened. Signature-gated: an unrelated scene poll (nothing
// spellbook-related changed) must not rebuild the list and reset scroll
// position for no reason.
let sbSpellbooksSig = null;
function refreshSpellbookContent() {
  if (!spellbookOn) return;
  const sig = JSON.stringify((scene && scene.spellbooks) || []);
  if (sig === sbSpellbooksSig) return;
  sbSpellbooksSig = sig;
  renderSpellSchool();
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
  if (spellbookOn) { buildSpellbook(); refreshSpellMana(); requestUnknownSpellbookContents(); }
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

// Skills introduced in AOS or later (ServUO SkillName ≥ 46: Necromancy, Focus,
// Chivalry, Bushido, Ninjitsu, Spellweaving, Mysticism, Imbuing, Throwing). Everything
// below is the classic/T2A set. We only show T2A skills on a non-AOS shard; on AOS we
// list T2A and AOS skills in separate groups.
const AOS_SKILL_MIN = 46;
let skillSort = { key: "name", dir: 1 };  // key: name | value | base
function skillRowHtml(s) {
  const lock = ((s.lock | 0) % 3 + 3) % 3;
  const usable = USABLE_SKILLS.has(s.id | 0);
  return `<div class="sk-row${usable ? " usable" : ""}" data-id="${s.id}">`
    + `<span class="sk-lock" data-lock="${lock}" title="${LOCK_TITLES[lock]}">${LOCK_ICONS[lock]}</span>`
    + `<span class="sk-name" title="${skillName(s.id | 0)}">${skillName(s.id | 0)}</span>`
    + `<span class="sk-val">${((s.v | 0) / 10).toFixed(1)}</span>`
    + `<span class="sk-use" title="use skill">▸</span>`
    + (usable ? `<span class="sk-pop" title="pull out as a button">⧉</span>` : "")
    + `</div>`;
}
function refreshSkills() {
  if (!skillsOn) return;
  const win = document.getElementById("skills");
  const list = document.getElementById("sk-list");
  const aos = !!(scene && scene.aos);
  let skills = (scene && scene.skills) || [];
  if (!aos) skills = skills.filter((s) => (s.id | 0) < AOS_SKILL_MIN); // T2A only on non-AOS shards
  const sig = JSON.stringify({
    sort: skillSort, aos,
    s: skills.map((s) => `${s.id}:${s.v}:${s.b}:${s.c}:${s.lock}`),
  });
  if (win._sig === sig) return;
  win._sig = sig;
  // Total skill points = sum of base values (tenths → divide by 10).
  let totalBase = 0;
  for (const s of skills) totalBase += (s.b | 0);
  set("sk-total", `Total: ${(totalBase / 10).toFixed(1)}  ·  ${skills.length} skills`);
  // Sort header (clickable; same column toggles ascending/descending).
  const arrow = (k) => (skillSort.key === k ? (skillSort.dir > 0 ? " ▲" : " ▼") : "");
  document.getElementById("sk-sortbar").innerHTML = "Sort: "
    + `<span class="sk-sortk" data-k="name">Name${arrow("name")}</span>`
    + `<span class="sk-sortk" data-k="value">Value${arrow("value")}</span>`
    + `<span class="sk-sortk" data-k="base">Base${arrow("base")}</span>`;
  if (!skills.length) { list.innerHTML = '<div class="cont-empty">no skill data</div>'; return; }
  const cmp = (a, b) => {
    const d = skillSort.dir;
    if (skillSort.key === "name") return d * skillName(a.id | 0).localeCompare(skillName(b.id | 0));
    if (skillSort.key === "value") return d * ((a.v | 0) - (b.v | 0));
    return d * ((a.b | 0) - (b.b | 0)); // base
  };
  const rows = (arr) => arr.slice().sort(cmp).map(skillRowHtml).join("");
  if (aos) {
    // Group T2A vs AOS+ skills, each sorted.
    const t2a = skills.filter((s) => (s.id | 0) < AOS_SKILL_MIN);
    const aosk = skills.filter((s) => (s.id | 0) >= AOS_SKILL_MIN);
    let html = "";
    if (t2a.length) html += '<div class="sk-group">T2A</div>' + rows(t2a);
    if (aosk.length) html += '<div class="sk-group">AOS</div>' + rows(aosk);
    list.innerHTML = html;
  } else {
    list.innerHTML = rows(skills);
  }
}
// One delegated listener (wired once at startup): lock click cycles the lock; the
// ▸ button or a row double-click uses the skill.
function wireSkills() {
  const list = document.getElementById("sk-list");
  // Sort-header clicks: pick a column; clicking the active column flips direction.
  document.getElementById("sk-sortbar").addEventListener("click", (e) => {
    const k = e.target.closest && e.target.closest(".sk-sortk");
    if (!k) return;
    const key = k.dataset.k;
    skillSort = { key, dir: skillSort.key === key ? -skillSort.dir : 1 };
    const win = document.getElementById("skills"); if (win) win._sig = null;
    refreshSkills();
  });
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
      // Another player's HP is always NORMALIZED by the server (ServUO
      // AttributeNormalizer, max 25) — nobody sees a stranger's real hit points in
      // UO — so a full-health ally arrives as "25/25", which is meaningless to
      // print. Show a percentage for other members; only our OWN entry carries the
      // real hits/max (our unnormalized self status), so show true numbers there.
      const isSelf = (m.serial | 0) === ((scene.player && scene.player.serial) | 0);
      const hp = max <= 0 ? "—" : isSelf ? `${m.hits | 0}/${max}` : `${pct}%`;
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
// --- secure trade windows (0x6F player-to-player trade, one per session) ---
// scene.trades = [{ opponent, opponentSerial, myCont, theirCont, myAccept,
// theirAccept, myOfferGold, myOfferPlat, theirOfferGold, theirOfferPlat,
// balanceGold, balancePlat }, …] (see anima-net scene.rs trades_json). Items on
// each side are ordinary scene.contItems keyed by container serial —
// SecureTradeEquip reuses the 0x25 AddToContainer wire format server-side, so
// filtering by myCont/theirCont is all a container window would do anyway.
// Unlike the party panel (toggled by 'Y') or the backpack (toggled by 'I'), a
// trade is something the SERVER opens — trading is peer-to-peer with no
// consent required, so more than one stranger can have a session open with us
// at once. One window per session, keyed by OUR OWN container serial
// (`myCont`, the value every outgoing trade command addresses), built/torn
// down the same way `containerWins`/`gumpWins` manage their multi-window
// lifecycle: build on first sight, refresh in place while the signature is
// unchanged, remove once the session drops off scene.trades.
const tradeWins = new Map(); // myCont -> { el, sig, myCont, goldIn, platIn, balanceGold, balancePlat }
let tradeCascade = 0;
// Build one side's item grid from scene.contItems, reusing the exact `.cont-item`
// markup/styling a normal container window uses. `readOnly` (the opponent's
// side) skips the drag-arm data attribute so `setupItemDnD` won't let us lift
// items we don't own; both sides still show the hover OPL tooltip (delegated
// on `.cont-item[data-serial]` regardless of the `ro` flag).
function renderTradeGrid(gridEl, cont, readOnly) {
  const items = (scene && scene.contItems || []).filter((it) => (it.cont >>> 0) === (cont >>> 0));
  gridEl.innerHTML = "";
  if (!items.length) { gridEl.innerHTML = '<div class="cont-empty">(empty)</div>'; return; }
  for (const it of items) {
    const cell = document.createElement("div");
    cell.className = "cont-item";
    cell.title = readOnly ? "" : "drag to move";
    cell.draggable = false;
    cell.dataset.serial = it.serial >>> 0;
    cell.dataset.g = it.g;
    cell.dataset.amount = (it.amount | 0) || 1;
    cell.dataset.st = it.st ? "1" : "0";
    if (readOnly) cell.dataset.ro = "1";
    const img = document.createElement("img");
    img.className = "cont-icon";
    img.src = `art/static/${stackGraphic(it.g, it.amount | 0)}.png`;
    img.draggable = false;
    img.onerror = () => { img.style.visibility = "hidden"; };
    cell.appendChild(img);
    if ((it.amount | 0) > 1) {
      const a = document.createElement("span");
      a.className = "cont-amt"; a.textContent = it.amount;
      cell.appendChild(a);
    }
    gridEl.appendChild(cell);
  }
}
function tradeInputFocused(win) {
  const a = document.activeElement;
  return a === win.goldIn || a === win.platIn;
}
// Send our gold/plat offer, clamped client-side to the account balance the
// server last gave us (action-4 UpdateLedger, `win.balanceGold`/`balancePlat`)
// — mirrors ClassicUO's TradingGump entry handler, which clamps rather than
// letting the player type more than they have.
function sendTradeGold(win) {
  const gold = Math.max(0, Math.min(win.balanceGold, parseInt(win.goldIn.value, 10) || 0));
  const plat = Math.max(0, Math.min(win.balancePlat, parseInt(win.platIn.value, 10) || 0));
  sendInput("tradegold:" + win.myCont + ":" + gold + ":" + plat);
}
function closeTradeWindow(myCont) {
  const win = tradeWins.get(myCont);
  if (win) { win.el.remove(); tradeWins.delete(myCont); }
}
function buildTradeWindow(myCont) {
  const el = document.createElement("div");
  el.className = "gump-win trade-win";
  const off = (tradeCascade++ % 6) * 24;
  el.style.left = (340 + off) + "px";
  el.style.top = (90 + off) + "px";
  el.innerHTML =
    '<div class="gump-title"><span>TRADE · <span class="tr-name"></span></span><span class="gump-close">✕</span></div>'
    + '<div class="gump-body">'
    + '<div class="tr-cols">'
    + '<div class="tr-col">'
    + '<div class="tr-col-title">You</div>'
    + '<div class="tr-grid tr-mine-grid"></div>'
    + '<label class="tr-accept"><input type="checkbox" class="tr-accept-cb"> I accept</label>'
    + '<div class="tr-gold-row">'
    + '<input class="tr-gold-in tr-gold" type="number" min="0" inputmode="numeric" placeholder="gold" autocomplete="off">'
    + '<input class="tr-gold-in tr-plat" type="number" min="0" inputmode="numeric" placeholder="plat" autocomplete="off">'
    + '</div>'
    + '<div class="tr-balance"></div>'
    + '</div>'
    + '<div class="tr-col">'
    + '<div class="tr-col-title tr-their-name">Them</div>'
    + '<div class="tr-grid tr-theirs-grid"></div>'
    + '<span class="tr-accept tr-their-accept">waiting…</span>'
    + '<div class="tr-their-gold">0 gold / 0 plat</div>'
    + '</div>'
    + '</div>'
    + '<button class="dlg-btn tr-cancel">Cancel Trade</button>'
    + '</div>';
  document.body.appendChild(el);
  const win = {
    el, sig: null, myCont,
    goldIn: el.querySelector(".tr-gold"), platIn: el.querySelector(".tr-plat"),
    balanceGold: 0, balancePlat: 0,
  };
  el.querySelector(".gump-close").addEventListener("click", () => {
    sendInput("tradecancel:" + myCont);
    closeTradeWindow(myCont); // close locally now — don't wait a poll for the server's echo
  });
  const cancelBtn = el.querySelector(".tr-cancel");
  cancelBtn.addEventListener("click", () => {
    sendInput("tradecancel:" + myCont);
    closeTradeWindow(myCont);
  });
  el.querySelector(".tr-accept-cb").addEventListener("change", (e) => {
    sendInput("tradeaccept:" + myCont + ":" + (e.target.checked ? "1" : "0"));
    // A checkbox is an <input>, so isTypingTarget() treats it as a typing target
    // while it holds focus — EVERY game key (not just letters) would silently
    // die, and a stray Space would natively re-toggle it. Blur to release focus,
    // matching how other windows (e.g. closeChat) avoid stealing the keyboard.
    e.target.blur();
  });
  for (const inp of [win.goldIn, win.platIn]) {
    inp.addEventListener("change", () => sendTradeGold(win));
    // Keep Enter/Esc local to this field (same pattern as the split/prompt
    // dialogs) so typing a gold amount never leaks a digit/movement key to
    // the global game-input handler.
    inp.addEventListener("keydown", (e) => {
      e.stopPropagation();
      if (e.code === "Enter" || e.code === "NumpadEnter") { e.preventDefault(); sendTradeGold(win); inp.blur(); }
    });
  }
  makeDraggable(el, el.querySelector(".gump-title"));
  tradeWins.set(myCont, win);
  return win;
}
// Rebuild a session's window only when its data (or either side's items)
// actually changed.
function renderTradeWindow(win, t) {
  win.el.querySelector(".tr-name").textContent = t.opponent || "someone";
  win.el.querySelector(".tr-their-name").textContent = t.opponent || "Them";
  renderTradeGrid(win.el.querySelector(".tr-mine-grid"), t.myCont, false);
  renderTradeGrid(win.el.querySelector(".tr-theirs-grid"), t.theirCont, true);
  win.el.querySelector(".tr-accept-cb").checked = !!t.myAccept;
  const theirAccept = win.el.querySelector(".tr-their-accept");
  theirAccept.textContent = t.theirAccept ? "✓ accepted" : "waiting…";
  theirAccept.classList.toggle("yes", !!t.theirAccept);
  win.el.querySelector(".tr-their-gold").textContent = `${t.theirOfferGold | 0} gold / ${t.theirOfferPlat | 0} plat`;
  win.balanceGold = t.balanceGold | 0;
  win.balancePlat = t.balancePlat | 0;
  win.el.querySelector(".tr-balance").textContent = `balance: ${win.balanceGold} gold / ${win.balancePlat} plat`;
  // Cap what can be typed to the account balance the server last gave us —
  // mirrors ClassicUO's TradingGump clamping the entry to `Gold`/`Platinum`.
  win.goldIn.max = win.balanceGold;
  win.platIn.max = win.balancePlat;
  // Don't clobber the field while the player is mid-keystroke in it.
  if (!tradeInputFocused(win)) {
    win.goldIn.value = t.myOfferGold | 0;
    win.platIn.value = t.myOfferPlat | 0;
  }
}
// Auto-open a window for each session in scene.trades, refresh the ones whose
// data changed, and auto-close any window whose session dropped off the list
// (cancelled, completed, or the opponent walked away).
function refreshTrade(scene) {
  const list = (scene && scene.trades) || [];
  const seen = new Set();
  for (const t of list) {
    const myCont = t.myCont >>> 0;
    seen.add(myCont);
    const items = (scene.contItems || []).filter(
      (it) => (it.cont >>> 0) === myCont || (it.cont >>> 0) === (t.theirCont >>> 0)
    );
    const sig = [
      t.theirCont, t.opponent, t.myAccept, t.theirAccept,
      t.myOfferGold, t.myOfferPlat, t.theirOfferGold, t.theirOfferPlat,
      t.balanceGold, t.balancePlat,
      items.map((it) => `${it.cont >>> 0}:${it.serial >>> 0}:${it.g}:${it.amount | 0}`).join(","),
    ].join("|");
    const win = tradeWins.get(myCont) || buildTradeWindow(myCont);
    if (win.sig === sig) continue;
    win.sig = sig;
    renderTradeWindow(win, t);
  }
  for (const myCont of [...tradeWins.keys()]) {
    if (!seen.has(myCont)) closeTradeWindow(myCont);
  }
}

// ---- treasure/decoration map windows (0x90/0xF5 DisplayMap(New) + 0x56
// MapCommand — ServUO `Scripts/Items/Tools/MapItem.cs`; one window per
// serial, built dynamically like .trade-win/.container-win) ----
const mapWins = new Map(); // serial -> { el, sig, canvas }
let mapCascade = 0;
function closeMapWindow(serial) {
  const win = mapWins.get(serial);
  if (win) { win.el.remove(); mapWins.delete(serial); }
}
function buildMapWindow(serial) {
  const el = document.createElement("div");
  el.className = "gump-win map-win";
  const off = (mapCascade++ % 8) * 22;
  el.style.left = (260 + off) + "px";
  el.style.top = (80 + off) + "px";
  el.innerHTML = '<div class="gump-title"><span>Map</span><span class="gump-close">✕</span></div>'
    + '<div class="gump-body"><div class="map-canvas"></div></div>';
  document.body.appendChild(el);
  el.querySelector(".gump-close").addEventListener("click", () => closeMapWindow(serial));
  makeDraggable(el, el.querySelector(".gump-title"));
  const win = { el, sig: null, canvas: el.querySelector(".map-canvas") };
  mapWins.set(serial, win);
  return win;
}
// Rebuild a map window's art/pins only when its content signature changed.
// Bounds/size never change for a given serial in practice, but PINS do via
// bare 0x56 traffic that does NOT bump `openSeq` (see `MapView::open_seq`'s
// doc) — so this must run on every poll for an already-open window,
// independent of `refreshMapWindows`'s open-a-NEW-window gate. The
// background is the (constant, ServUO always sends 0x139D) parchment gump
// art stretched via CSS to the map's own w×h box — pins are drawn at their
// raw wire (x, y) with NO further rescale, because stretching the background
// to fill that exact box already makes it line up (see `MapView`'s Rust doc
// for why no client-side pin math is needed either way). Pin index 0 is the
// treasure/chest pin (ServUO `MapItem.RemovePin` refuses to remove it) —
// drawn with the `.chest` variant so it reads as the goal.
function renderMapWindow(win, m) {
  const sig = JSON.stringify([m.gumpArt, m.w, m.h, m.pins, m.editable]);
  if (win.sig === sig) return;
  win.sig = sig;
  const w = m.w | 0, h = m.h | 0;
  const c = win.canvas;
  c.style.width = w + "px";
  c.style.height = h + "px";
  c.innerHTML = `<img class="map-bg" src="gump/${m.gumpArt | 0}.png" alt=""`
    + ` onerror="this.onerror=null;this.style.display='none'">`;
  (m.pins || []).forEach((p, i) => {
    const pin = document.createElement("div");
    pin.className = "map-pin" + (i === 0 ? " chest" : "");
    pin.style.left = (p[0] | 0) + "px";
    pin.style.top = (p[1] | 0) + "px";
    pin.title = i === 0 ? "treasure" : ("pin " + i);
    c.appendChild(pin);
  });
}
// Open a NEW window only when a serial's `openSeq` (scene.maps[].openSeq)
// advances past what we've already opened for — see `lastMapOpenSeq`'s doc:
// this is what stops a user-closed map window from popping back open on
// every poll just because World still carries the same MapView. Content of
// any ALREADY-open window is still refreshed every poll regardless (a pin
// can change via a bare 0x56 that doesn't bump `openSeq`). A window whose
// map fell out of scene.maps entirely (the item was deleted, or a facet
// switch purged it — see `World::on_map_change`) is closed to match.
function refreshMapWindows(scene) {
  const list = (scene && scene.maps) || [];
  const seen = new Set();
  for (const m of list) {
    const serial = m.serial >>> 0;
    seen.add(serial);
    const seq = m.openSeq | 0;
    const isNew = seq > (lastMapOpenSeq.get(serial) || 0);
    let win = mapWins.get(serial);
    if (isNew) {
      lastMapOpenSeq.set(serial, seq);
      if (win) bringToFront(win.el);
      else win = buildMapWindow(serial);
    }
    if (win) renderMapWindow(win, m);
  }
  for (const serial of [...mapWins.keys()]) {
    if (!seen.has(serial)) closeMapWindow(serial);
  }
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
  // Prefer the server's own title line (0x88 DisplayPaperdoll — e.g. "Anima the
  // Adventurer") over the plain mobile name, when it's for THIS target.
  const targetSerial = isSelf ? ((scene && scene.player && scene.player.serial) >>> 0) : (pdTarget >>> 0);
  const serverTitle = (pdServerInfo && (pdServerInfo.serial >>> 0) === targetSerial) ? pdServerInfo.title : null;
  const sig = [isSelf ? "s" : pdTarget, p.name, serverTitle, p.str, p.dex, p.int, p.hits, p.hitsMax, p.mana, p.manaMax,
    p.stam, p.stamMax, p.gold, p.body, p.hue,
    // Include each item's OPL name so the list re-renders (slot label → real name)
    // the moment its OPL arrives.
    equip.map((e) => `${e.layer}:${e.g}:${e.serial >>> 0}:${oplName(e.serial)}`).join(",")].join("|");
  if (pd._sig === sig) return;
  pd._sig = sig;
  set("pd-name", serverTitle || p.name || (isSelf ? "(unnamed)" : "(mobile)"));
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
    // Equipconv.def override: the server already resolved a gender-correct
    // absolute gump id (anima-net `equip_conv_gump`) when this item's (wearer
    // body, AnimID) has a conversion — use it as-is. Otherwise fall back to the
    // plain AnimID + gender-offset convention.
    const gid = e.gump != null ? e.gump : e.anim + gOff;
    // Hide any item whose paperdoll gump is missing rather than show a broken "?".
    // Female items (no explicit override) may lack a female gump → fall back to
    // the male offset first; an explicit `gump` is already gender-resolved, so it
    // just hides on error instead of guessing another id.
    const hide = "this.onerror=null;this.style.display='none'";
    const onerr = (e.gump == null && female)
      ? `this.onerror=function(){${hide}};this.src='gump/${e.anim + MALE_GUMP_OFFSET}.png${hueQ}'`
      : hide;
    // Tag each layer so hovering the figure (per-pixel hit-test) can resolve the item.
    h += `<img src="gump/${gid}.png${hueQ}" alt="" crossorigin="anonymous" draggable="false"`
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
  h += `<div class="pd-profile-actions"><button type="button" class="dlg-btn pd-profile"`
    + ` data-profile="${targetSerial}">PROFILE</button></div>`;
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
    cell.draggable = false;                // pointer-drag (held-on-cursor), not native HTML5 drag
    cell.dataset.serial = itemSerial;
    cell.dataset.g = it.g;
    cell.dataset.amount = (it.amount | 0) || 1;
    cell.dataset.st = it.st ? "1" : "0";
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
      // Belt and braces: a completed double-click on this cell means the press
      // was a click, not a drag, no matter what — disarm a still-pending
      // groundDrag for this same serial so a stray later pointermove (or the
      // dialog/gump that "use" may pop, covering the cell) can't still promote
      // it into a lift out from under the click that just resolved.
      if (groundDrag && (groundDrag.serial >>> 0) === itemSerial) groundDrag = null;
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
// (server-side) into positioned elements, each tagged with its gump "page"
// (0 = always visible; see scene.rs parse_gump_layout). We mirror one draggable
// .gump-win per serial: build on first sight, rebuild when its content signature
// changes, and remove when it's gone from scene.gumps. Every window tracks its
// own current page (starts at 1); "page-jump" buttons (pageflag 0) just flip
// that locally via applyGumpPage() — no packet is ever sent for those. A real
// reply button (pageflag 1) collects the on-state of all checkboxes/radios +
// text-entry values (across every page, not just the visible one — they stay
// in the DOM, just hidden) and sends a `gump:` reply, then closes locally; the
// ✕ sends button 0 (cancel). These are normal windows — they don't block the
// rest of the game.
const gumpWins = new Map(); // serial -> { el, sig, page, nodes, … }
let gumpCascade = 0;
// Remembered screen position per gump KIND (gumpId), like ClassicUO's saved gump
// locations. ServUO craft/menu gumps close and REOPEN with a fresh serial on every
// selection, so keying position by serial (or cascading each build) walked the
// window down-right across the screen. Reopening the same kind now lands where the
// last one of that kind sat; a user drag updates the remembered spot.
const gumpPos = new Map();       // gumpId → { left, top }
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
    // Preserve the locally-selected page across a content refresh (the server
    // resending the same gump shouldn't kick the player back to page 1) — but
    // clamp it to the new layout's highest real page, in case the refreshed
    // gump has fewer pages than before (else the window would show only the
    // page-0 chrome, no page ever matching).
    const maxPage = (g.elements || []).reduce((m, e) => Math.max(m, e.page | 0), 0);
    const page = existing ? Math.min(existing.page, maxPage || 1) : 1;
    if (existing) existing.el.remove();             // content changed → rebuild
    buildGumpWindow(serial, g, sig, page);
  }
  // Drop windows whose gump the server closed.
  for (const serial of [...gumpWins.keys()]) {
    if (!seen.has(serial)) { gumpWins.get(serial).el.remove(); gumpWins.delete(serial); }
  }
}

// ── Legacy item/question menus (0x7C → 0x7D) ──────────────────────────────
// Several may be open at once. Each window is keyed by the server menu serial;
// answering removes it locally immediately and suppresses the same snapshot
// until the server consumes the response (avoids one-poll flicker/reopening).
const legacyMenuWins = new Map(); // serial -> { el, sig, selected }
const legacyMenuDismissed = new Map(); // serial -> signature just answered/canceled
let legacyMenuCascade = 0;

function legacyMenuSignature(menu) {
  return JSON.stringify([menu.menuId | 0, menu.question || "", menu.kind || "question", menu.entries || []]);
}

function answerLegacyMenu(serial, index) {
  const win = legacyMenuWins.get(serial);
  if (!win) return;
  legacyMenuDismissed.set(serial, win.sig);
  win.el.remove();
  legacyMenuWins.delete(serial);
  sendInput("menusel:" + serial + ":" + index);
}

function buildLegacyMenuWindow(menu, sig) {
  const serial = menu.serial >>> 0;
  const entries = menu.entries || [];
  const itemMenu = menu.kind === "items";
  const el = document.createElement("div");
  el.className = "gump-win legacy-menu-win";
  const off = (legacyMenuCascade++ % 8) * 22;
  el.style.left = (220 + off) + "px";
  el.style.top = (110 + off) + "px";
  el.innerHTML = '<div class="gump-title"><span>Menu</span><span class="gump-close">✕</span></div>'
    + '<div class="gump-body legacy-menu-body"><div class="legacy-menu-question"></div>'
    + '<div class="legacy-menu-entries"></div><div class="legacy-menu-actions">'
    + '<button class="dlg-btn legacy-menu-continue">Continue</button>'
    + '<button class="dlg-btn legacy-menu-cancel">Cancel</button></div></div>';
  el.querySelector(".legacy-menu-question").textContent = menu.question || "Choose an option";
  const list = el.querySelector(".legacy-menu-entries");
  if (itemMenu) list.classList.add("legacy-item-grid");

  const state = { el, sig, selected: entries.length ? (entries[0].index | 0) : 0 };
  for (const entry of entries) {
    const index = entry.index | 0;
    const label = document.createElement("label");
    label.className = itemMenu ? "legacy-item-choice" : "legacy-question-choice";
    const radio = document.createElement("input");
    radio.type = "radio";
    radio.name = "legacy-menu-" + serial;
    radio.value = String(index);
    radio.checked = index === state.selected;
    radio.addEventListener("change", () => { state.selected = index; });
    label.appendChild(radio);
    if (itemMenu) {
      const img = document.createElement("img");
      img.src = "art/static/" + (entry.graphic | 0) + ".png" + ((entry.hue | 0) ? ("?hue=" + (entry.hue | 0)) : "");
      img.alt = "";
      img.draggable = false;
      img.addEventListener("error", () => { img.style.visibility = "hidden"; });
      label.appendChild(img);
      label.addEventListener("dblclick", (event) => {
        event.preventDefault();
        answerLegacyMenu(serial, index);
      });
    }
    const text = document.createElement("span");
    text.textContent = entry.text || ("Option " + index);
    label.appendChild(text);
    list.appendChild(label);
  }

  const proceed = el.querySelector(".legacy-menu-continue");
  proceed.disabled = state.selected === 0;
  proceed.addEventListener("click", () => {
    if (state.selected) answerLegacyMenu(serial, state.selected);
  });
  el.querySelector(".legacy-menu-cancel").addEventListener("click", () => answerLegacyMenu(serial, 0));
  el.querySelector(".gump-close").addEventListener("click", () => answerLegacyMenu(serial, 0));
  el.addEventListener("keydown", (event) => {
    if (event.code === "Escape") {
      event.preventDefault(); event.stopPropagation(); answerLegacyMenu(serial, 0);
    } else if (event.code === "Enter" || event.code === "NumpadEnter") {
      event.preventDefault(); event.stopPropagation();
      if (state.selected) answerLegacyMenu(serial, state.selected);
    }
  });
  document.body.appendChild(el);
  makeDraggable(el, el.querySelector(".gump-title"));
  legacyMenuWins.set(serial, state);
}

function refreshLegacyMenus(scene) {
  const list = (scene && scene.legacyMenus) || [];
  const seen = new Set();
  for (const menu of list) {
    const serial = menu.serial >>> 0;
    const sig = legacyMenuSignature(menu);
    seen.add(serial);
    if (legacyMenuDismissed.get(serial) === sig) continue;
    if (legacyMenuDismissed.has(serial)) legacyMenuDismissed.delete(serial); // changed menu, same serial
    const existing = legacyMenuWins.get(serial);
    if (existing && existing.sig === sig) continue;
    if (existing) { existing.el.remove(); legacyMenuWins.delete(serial); }
    buildLegacyMenuWindow(menu, sig);
  }
  for (const serial of [...legacyMenuWins.keys()]) {
    if (!seen.has(serial)) {
      legacyMenuWins.get(serial).el.remove();
      legacyMenuWins.delete(serial);
    }
  }
  for (const serial of [...legacyMenuDismissed.keys()]) {
    if (!seen.has(serial)) legacyMenuDismissed.delete(serial);
  }
}

// ── Server dye hue pickers (0x95 request/response) ─────────────────────────
// ClassicUO presents 1000 ordinary dyed hues as five 20×10 grids. Graduation
// g contains hues `2 + g + cell*5`, covering exactly ServUO's clipped 2..1001
// range. A server-owned picker has no cancel packet, so these windows have no X.
const huePickerWins = new Map();       // serial -> picker window state
const huePickerDismissed = new Map();  // serial -> signature just answered
let huePickerCascade = 0;
let dyedPalettePromise = null;

function loadDyedPalette() {
  if (!dyedPalettePromise) {
    dyedPalettePromise = fetch("hues/dyed.json", { cache: "force-cache" })
      .then((response) => {
        if (!response.ok) throw new Error("palette HTTP " + response.status);
        return response.json();
      })
      .then((data) => {
        if ((data.start | 0) !== 2 || !Array.isArray(data.colors) || data.colors.length !== 1000) {
          throw new Error("invalid dyed palette");
        }
        return data.colors;
      })
      .catch((error) => {
        console.warn("dye palette unavailable", error);
        dyedPalettePromise = null; // allow a later picker to retry
        return Array(1000).fill("#444");
      });
  }
  return dyedPalettePromise;
}

function huePickerSignature(picker) {
  return JSON.stringify([picker.graphic | 0]);
}

function dyedHue(graduation, cell) {
  return 2 + (graduation | 0) + (cell | 0) * 5;
}

function updateHuePickerPreview(state) {
  state.label.textContent = "Hue " + state.selectedHue;
  state.preview.src = "art/static/" + state.graphic + ".png?hue=" + state.selectedHue;
}

function renderHuePickerGrid(state) {
  state.grid.innerHTML = "";
  for (let cell = 0; cell < 200; cell++) {
    const hue = dyedHue(state.graduation, cell);
    const swatch = document.createElement("button");
    swatch.type = "button";
    swatch.className = "hue-picker-swatch" + (hue === state.selectedHue ? " selected" : "");
    swatch.style.backgroundColor = state.colors[hue - 2] || "#444";
    swatch.title = "Hue " + hue;
    swatch.setAttribute("aria-label", "Hue " + hue);
    swatch.addEventListener("click", () => {
      state.selectedCell = cell;
      state.selectedHue = hue;
      for (const old of state.grid.querySelectorAll(".selected")) old.classList.remove("selected");
      swatch.classList.add("selected");
      updateHuePickerPreview(state);
    });
    swatch.addEventListener("dblclick", () => answerHuePicker(state.serial, hue));
    state.grid.appendChild(swatch);
  }
}

function answerHuePicker(serial, hue) {
  const state = huePickerWins.get(serial);
  if (!state) return;
  huePickerDismissed.set(serial, state.sig);
  state.el.remove();
  huePickerWins.delete(serial);
  sendInput("huepick:" + serial + ":" + hue);
}

function buildHuePickerWindow(picker, sig) {
  const serial = picker.serial >>> 0;
  const graphic = (picker.graphic | 0) || 0x0FAB;
  const el = document.createElement("div");
  el.className = "gump-win hue-picker-win";
  const off = (huePickerCascade++ % 8) * 22;
  el.style.left = (250 + off) + "px";
  el.style.top = (90 + off) + "px";
  el.innerHTML = '<div class="gump-title"><span>Dye color</span></div>'
    + '<div class="gump-body hue-picker-body"><div class="hue-picker-toolbar">'
    + '<div class="hue-picker-preview"><img alt="Dye preview" draggable="false"></div>'
    + '<div class="hue-picker-controls"><span class="hue-picker-label">Hue 3</span>'
    + '<label>Graduation <input class="hue-picker-slider" type="range" min="0" max="4" step="1" value="1"></label>'
    + '</div></div><div class="hue-picker-grid" aria-label="Dye colors"></div>'
    + '<button class="dlg-btn hue-picker-apply">Apply color</button></div>';
  document.body.appendChild(el);
  const state = {
    el, sig, serial, graphic, graduation: 1, selectedCell: 0, selectedHue: 3, colors: null,
    grid: el.querySelector(".hue-picker-grid"),
    preview: el.querySelector(".hue-picker-preview img"),
    label: el.querySelector(".hue-picker-label"),
  };
  huePickerWins.set(serial, state);
  updateHuePickerPreview(state);
  state.preview.addEventListener("error", () => { state.preview.style.visibility = "hidden"; });
  const slider = el.querySelector(".hue-picker-slider");
  slider.addEventListener("input", () => {
    state.graduation = slider.value | 0;
    // ClassicUO keeps the selected grid cell while changing graduation.
    state.selectedHue = dyedHue(state.graduation, state.selectedCell);
    if (state.colors) renderHuePickerGrid(state);
    updateHuePickerPreview(state);
  });
  el.querySelector(".hue-picker-apply").addEventListener("click", () => {
    answerHuePicker(serial, state.selectedHue);
  });
  el.addEventListener("keydown", (event) => {
    if (event.code === "Escape") {
      // ClassicUO sets CanCloseWithRightClick=false for server-owned pickers.
      event.preventDefault(); event.stopPropagation();
    } else if (event.code === "Enter" || event.code === "NumpadEnter") {
      event.preventDefault(); event.stopPropagation(); answerHuePicker(serial, state.selectedHue);
    }
  });
  makeDraggable(el, el.querySelector(".gump-title"));
  state.grid.textContent = "Loading colors…";
  loadDyedPalette().then((colors) => {
    if (huePickerWins.get(serial) !== state) return;
    state.colors = colors;
    renderHuePickerGrid(state);
  });
}

function refreshHuePickers(scene) {
  const list = (scene && scene.huePickers) || [];
  const seen = new Set();
  for (const picker of list) {
    const serial = picker.serial >>> 0;
    const sig = huePickerSignature(picker);
    seen.add(serial);
    if (huePickerDismissed.get(serial) === sig) continue;
    if (huePickerDismissed.has(serial)) huePickerDismissed.delete(serial);
    const existing = huePickerWins.get(serial);
    if (existing && existing.sig === sig) continue;
    if (existing) { existing.el.remove(); huePickerWins.delete(serial); }
    buildHuePickerWindow(picker, sig);
  }
  for (const serial of [...huePickerWins.keys()]) {
    if (!seen.has(serial)) {
      huePickerWins.get(serial).el.remove();
      huePickerWins.delete(serial);
    }
  }
  for (const serial of [...huePickerDismissed.keys()]) {
    if (!seen.has(serial)) huePickerDismissed.delete(serial);
  }
}
// ── Right-click context (popup) menu (0xBF/0x14) ───────────────────────────
// scene.popup = { serial, entries:[{ index, text }] } | null. We show a small
// menu div at the last cursor position; a row click sends popupsel and hides it;
// click-away / Esc / the popup clearing also hides it.
let popupEl = null;            // the live menu element (null = hidden)
let popupSerial = 0;           // serial the menu was opened for
let popupDismissed = 0;        // serial the user closed (Esc / click-away). The server keeps
                               // its popup set until we select or its target is removed, so
                               // without this refreshPopup would re-open the menu next poll.
                               // Cleared on a fresh popupreq (below) or when the server drops it.
function hidePopup(dismissed) {
  if (dismissed && popupSerial) popupDismissed = popupSerial;
  if (popupEl) { popupEl.remove(); popupEl = null; popupSerial = 0; }
}
function refreshPopup(scene) {
  const p = scene && scene.popup;
  if (!p || !p.entries || !p.entries.length) { hidePopup(); popupDismissed = 0; return; }
  const serial = p.serial >>> 0;
  if (serial === popupDismissed) return;   // user closed this one — wait for a new/cleared popup
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
    row.className = "popup-row" + (e.hl ? " popup-row-hl" : ""); // 0x01 = highlighted default action
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

function buildGumpWindow(serial, g, sig, page) {
  const gumpId = g.gumpId >>> 0;
  const el = document.createElement("div");
  el.className = "gump-win dialog-win";
  // Reopen at the remembered spot for this gump KIND; only a first-seen kind
  // cascades (so distinct dialogs don't stack exactly). This keeps a craft/menu
  // gump anchored across its close-and-reopen-with-a-new-serial cycle.
  const saved = gumpPos.get(gumpId);
  if (saved) {
    el.style.left = saved.left + "px";
    el.style.top = saved.top + "px";
  } else {
    const off = (gumpCascade++ % 8) * 24;
    el.style.left = (160 + off) + "px";
    el.style.top = (90 + off) + "px";
    gumpPos.set(gumpId, { left: 160 + off, top: 90 + off });
  }
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

  // `page` is this window's *local* current page (the server never sees it —
  // UO pages are a pure client-side layout concept, ClassicUO's Gump.ActivePage).
  // Page 1 is shown initially. Every element is built once and stays in the DOM
  // (so checkbox/text-entry state on a page you navigate away from survives);
  // applyGumpPage() just toggles which ones are visible. Elements with no
  // "page" token before them in the layout parsed to page 0 and are shown on
  // every page.
  const win = { el, sig, serial, gumpId, canvas, page: page || 1, nodes: [] };
  for (const e of (g.elements || [])) {
    const node = buildGumpElement(win, e);
    node.dataset.page = e.page | 0;
    win.nodes.push(node);
    canvas.appendChild(node);
  }
  applyGumpPage(win);

  // ✕ → cancel (button 0).
  title.querySelector(".gump-close").addEventListener("click", () => {
    sendInput(`gump:${serial}:${gumpId}:0`);
    closeGump(serial);
  });
  // Remember where the user drags this window, keyed by kind, so the next reopen
  // (fresh serial) lands there too.
  makeDraggable(el, title, (x, y) => gumpPos.set(gumpId, { left: x, top: y }));
  document.body.appendChild(el);
  gumpWins.set(serial, win);
}
// Show/hide this window's elements for its current local page: page-0
// elements are always visible; everything else only shows while it's the
// active page. Called on first build and again whenever a pageflag-0 button
// flips `win.page` — that's a pure local redraw, no packet goes to the server
// (ClassicUO Button.ButtonAction.SwitchPage).
function applyGumpPage(win) {
  for (const node of win.nodes) {
    const p = Number(node.dataset.page) || 0;
    node.style.display = (p === 0 || p === win.page) ? "" : "none";
  }
}
// ── UO gump HTML mini-parser ────────────────────────────────────────────
// Servers embed a small HTML subset in gump text — both `text`/`croppedtext`
// strings AND resolved `htmlgump`/`xmfhtml*` blocks arrive as the same "t":
// "text" JSON shape (anima-core's gump_layout.rs keeps both raw; anima-net's
// scene.rs shapes them identically — see those files' doc comments). E.g.
// ServUO's CraftGump sends literal `<CENTER>ALCHEMY MENU</CENTER>`. This
// turns that string into safe DOM nodes: CENTER/LEFT/RIGHT become a
// block-level wrapper with the matching text-align (.gh-center/.gh-left/
// .gh-right, scoped under .dialog-win in index.html), B/I/U/BIG/SMALL become
// inline <span> classes, BASEFONT COLOR sets a running text color (sanitized
// — #rrggbb/#rgb or a small named-color whitelist, never the raw attribute
// text), BR is a line break, and any other tag (<A HREF>, <P>, …) is
// stripped but its inner text kept.
//
// SAFETY: the string is tokenized by hand on '<'/'>' and every node is built
// with createElement/createTextNode — never innerHTML/outerHTML and never a
// tag name taken from the server — so a malicious server string (even
// `<script>…</script>`) can only ever become inert text content, never a
// real element, attribute, or executable markup.
const GUMP_NAMED_COLORS = new Set([
  "red", "cyan", "blue", "darkblue", "lightblue", "purple", "yellow", "lime",
  "magenta", "white", "silver", "gray", "grey", "black", "orange", "brown",
  "maroon", "green", "olive",
]);
// `raw` is whatever came after `color=` in a BASEFONT tag, already isolated
// by a regex that stops at the first quote/space — validate it's an actual
// #rrggbb/#rgb hex or one of the whitelisted names before it ever reaches
// `style.color`; anything else (an attempted CSS/style-breakout string,
// junk) is dropped (returns null, meaning "leave color unset").
function gumpSanitizeColor(raw) {
  const hex = (raw || "").trim().replace(/^#/, "");
  if (/^[0-9a-fA-F]{6}$/.test(hex) || /^[0-9a-fA-F]{3}$/.test(hex)) return "#" + hex.toLowerCase();
  const name = (raw || "").trim().toLowerCase();
  return GUMP_NAMED_COLORS.has(name) ? name : null;
}
// Decode the handful of entities UO gump text actually uses. Deliberately a
// fixed whitelist regex (not a generic &name; decoder) — only ever produces
// plain characters, never re-introduces '<'/'>' as anything but literal text
// (the result is inserted via createTextNode, so it can't become markup
// even if it contains those characters).
function gumpDecodeEntities(s) {
  return s.replace(/&(amp|lt|gt|nbsp|quot|apos|#39);/gi, (m, name) => {
    switch (name.toLowerCase()) {
      case "amp": return "&";
      case "lt": return "<";
      case "gt": return ">";
      case "nbsp": return "\u00A0";
      case "quot": return '"';
      case "apos": case "#39": return "'";
      default: return m;
    }
  });
}
// Parse a gump text/html string into a DocumentFragment of safe DOM nodes.
// `boxWidth` isn't consulted directly (an alignment wrapper is a `width:100%`
// block div — it centers within whatever explicit CSS width the caller has
// already set on the element, e.g. croppedtext's `w`); a `null`/absent width
// (a plain unbounded `text`) still parses fine, it just has no box to center
// within. Malformed input (unclosed tags, stray closes, no `>`) degrades
// gracefully — it never throws and never drops trailing text.
function renderGumpHtml(raw, boxWidth) {
  const root = document.createDocumentFragment();
  const str = String(raw == null ? "" : raw);
  // One stack frame per currently-open recognized/unknown tag: `el` is where
  // new nodes get appended (the fragment for the untouched root, or the
  // span/div pushed for a recognized tag, or the PARENT's `el` again for an
  // unknown tag — so it's tracked for matching but contributes no DOM node).
  // `name` is the upper-cased tag name a closing tag must match.
  const stack = [{ el: root, name: null }];
  // BASEFONT's color is a running property, not a stack frame (real gump
  // text rarely closes it) — applies to every later text run until changed
  // or reset by a bare <basefont>.
  let color = null;

  const top = () => stack[stack.length - 1];
  const appendText = (chunk) => {
    if (!chunk) return;
    const text = gumpDecodeEntities(chunk);
    if (!text) return;
    if (color) {
      const span = document.createElement("span");
      span.style.color = color; // already sanitized — see gumpSanitizeColor
      span.appendChild(document.createTextNode(text));
      top().el.appendChild(span);
    } else {
      top().el.appendChild(document.createTextNode(text));
    }
  };
  const openBlock = (cls, name) => {
    const div = document.createElement("div");
    div.className = cls;
    top().el.appendChild(div);
    stack.push({ el: div, name });
  };
  const openInline = (cls, name) => {
    const span = document.createElement("span");
    span.className = cls;
    top().el.appendChild(span);
    stack.push({ el: span, name });
  };
  const closeTag = (name) => {
    // Pop back to (and including) the nearest matching open frame; a stray
    // close with no match (or one that would pop the implicit root) is
    // simply ignored rather than throwing.
    for (let i = stack.length - 1; i > 0; i--) {
      if (stack[i].name === name) { stack.length = i; return; }
    }
  };

  let i = 0;
  while (i < str.length) {
    const lt = str.indexOf("<", i);
    if (lt === -1) { appendText(str.slice(i)); break; }
    appendText(str.slice(i, lt));
    const gt = str.indexOf(">", lt);
    if (gt === -1) {
      // Unterminated '<' — nothing left can parse as a tag; keep the rest
      // as literal text instead of throwing or silently dropping it.
      appendText(str.slice(lt));
      break;
    }
    const body = str.slice(lt + 1, gt).trim();
    i = gt + 1;
    if (!body) continue;
    const closing = body[0] === "/";
    const rest = closing ? body.slice(1) : body;
    const m = rest.match(/^[A-Za-z][A-Za-z0-9]*/);
    if (!m) continue; // "<>", "</>", "<123>" — not a tag we can name; skip
    const name = m[0].toUpperCase();

    if (closing) { closeTag(name); continue; }

    switch (name) {
      case "CENTER": openBlock("gh-center", name); break;
      case "LEFT": openBlock("gh-left", name); break;
      case "RIGHT": openBlock("gh-right", name); break;
      case "BR": top().el.appendChild(document.createElement("br")); break;
      case "B": case "BOLD": openInline("gh-b", name); break;
      case "I": case "EM": openInline("gh-i", name); break;
      case "U": openInline("gh-u", name); break;
      case "BIG": openInline("gh-big", name); break;
      case "SMALL": openInline("gh-small", name); break;
      case "BASEFONT": {
        const cm = rest.match(/color\s*=\s*"?([^"\s>]+)"?/i);
        color = cm ? gumpSanitizeColor(cm[1]) : null; // bare <basefont> resets
        break;
      }
      // Unknown tag (<A HREF>, <P>, …): stripped, inner text kept — track it
      // (so a later matching close pops cleanly) without adding a DOM node.
      default: stack.push({ el: top().el, name }); break;
    }
  }
  return root;
}
function buildGumpElement(win, e) {
  const { serial, gumpId } = win;
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
    // croppedtext (w present) gets a clip box (.dlg-text-crop: overflow
    // hidden at exactly w px) — it never wraps, it clips. Plain text (no w,
    // scene.rs's parse_gump_layout never emits one for it) gets no width and
    // no clip, just runs on past its start point; both are single-line
    // (.dlg-text: white-space: nowrap, in index.html). This same shape also
    // carries a resolved htmlgump/xmfhtml* block (anima-net's scene.rs shapes
    // both identically, `w` always present for those) — either way `s` may
    // carry raw UO gump-HTML (`<CENTER>…</CENTER>`, `<basefont color=…>`,
    // …), which renderGumpHtml turns into safe, styled DOM nodes instead of
    // literal tag text.
    if (e.w) { node.classList.add("dlg-text-crop"); node.style.width = (e.w | 0) + "px"; }
    node.appendChild(renderGumpHtml(e.s || "", e.w ? (e.w | 0) : null));
  } else if (e.t === "button") {
    node.classList.add("dlg-btn");
    node.type = "button";
    // Draw the real button gump art (a small image sized to the art) so it sits in
    // the slot the gump intended — not a wide numbered box that overlaps the text.
    // Fall back to the reply id text if the art is missing.
    if (e.g) {
      node.classList.add("img");
      const img = document.createElement("img");
      img.className = "dlg-btn-img";
      img.src = `gump/${e.g | 0}.png`;
      img.alt = "";
      img.onerror = () => { img.remove(); node.classList.remove("img"); node.textContent = (e.id | 0) || "?"; };
      node.appendChild(img);
    } else {
      node.textContent = (e.id | 0) || "?";
    }
    // pageflag 0 = local page-jump (switch to page `param`, never touches the
    // network); pageflag 1 (or absent, for callers that never set it) = a real
    // reply button that sends 0xB1 GumpResponse with this element's reply id.
    if ((e.pageflag | 0) === 0) {
      node.title = "page " + (e.param | 0);
      node.addEventListener("click", () => { win.page = e.param | 0; applyGumpPage(win); });
    } else {
      node.title = "reply " + (e.id | 0);
      node.addEventListener("click", () => submitGump(serial, gumpId, e.id | 0));
    }
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
// Serial queue for optimistic drop/equip commands awaiting a possible 0x28/0x29
// acknowledgement. Current ServUO does not emit these packets, so cap the queue
// defensively; older/freeshard servers that do emit them consume it in order.
const pendingPlacements = [];
const MAX_PENDING_PLACEMENTS = 32;
let placedAt = 0;               // perf time of the last placement (debounces the trailing mousedown)
// Pointer-drag arming (canvas sprites can't fire HTML5 dragstart): a left-press on a
// draggable item arms `groundDrag`; once the press becomes a real drag the item lifts
// onto the cursor (held), and a release/next-click places it. A quick tap must NOT
// promote to a drag — a click or double-click on a trackpad routinely drifts a few px
// under the finger, and the old 5px-only rule turned every such tap into a pickup.
// So a small drift only counts once the button has been held past the double-click
// window (DRAG_HOLD_MS); a clearly-intentional larger motion (DRAG_FAR) lifts at once.
// For arms that DO have a natural on-screen cell (a container-grid icon or a
// paperdoll `.eq-icon` row entry — see `rect` below), that hold/distance heuristic
// is dropped entirely in favor of a hard rule: promote only once the pointer
// actually LEAVES the cell it was pressed on. A double-click's drift, however
// long held, never leaves a ~40px cell; a real drag-out always does almost
// immediately. World items / the worn-doll figure have no such cell (a world
// sprite's screen position pans with the camera), so they keep the old heuristic.
let groundDrag = null;          // { serial, g, amount, st, sx, sy, started, t, rect? } or null
let dragGhost = null;           // floating <img> glued to the cursor while an item is held
const DRAG_THRESHOLD = 6;       // min px of motion before a held press is even a drag candidate
const DRAG_FAR = 22;            // px of motion that means "definitely a drag" regardless of hold time
const DRAG_HOLD_MS = 250;       // a small drift only becomes a drag after the button's been held this long (> a tap)

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
function sendPlacement(command, serial) {
  pendingPlacements.push(serial >>> 0);
  if (pendingPlacements.length > MAX_PENDING_PLACEMENTS) pendingPlacements.shift();
  sendInput(command);
}
// Resolve where a placement click landed and issue the matching drop/equip (the
// `pickup` already went out at lift, so only one command here). Returns true if the
// item was placed (caller clears it); false to KEEP holding (clicked an invalid spot).
function placeCursorItem(clientX, clientY) {
  if (!cursorItem) return false;
  const serial = cursorItem.serial;
  const el = document.elementFromPoint(clientX, clientY);
  // Our own side of an open secure trade (.tr-mine-grid) is a drop target too —
  // dropping there is a normal container drop targeting THAT WINDOW's own trade
  // container serial (multiple sessions can be open at once, one window each,
  // so the target comes from the enclosing .trade-win via `tradeWins`, not a
  // single global session). The opponent's side (.tr-theirs-grid) is
  // intentionally NOT a target here — we can't place items into their half.
  const tradeGrid = el && el.closest && el.closest(".tr-mine-grid");
  const tradeWinEl = tradeGrid && tradeGrid.closest(".trade-win");
  if (tradeWinEl) {
    let tgt = null;
    for (const [s, w] of tradeWins) if (w.el === tradeWinEl) { tgt = s; break; }
    if (tgt == null) return false;
    const r = tradeGrid.getBoundingClientRect();
    const gx = Math.max(0, Math.min(150, Math.round(clientX - r.left)));
    const gy = Math.max(0, Math.min(120, Math.round(clientY - r.top)));
    sendPlacement("drop:" + serial + ":" + gx + ":" + gy + ":0:" + tgt, serial);
    return true;
  }
  const contWin = el && el.closest && el.closest(".container-win");
  if (contWin) {
    let tgt = null;
    for (const [s, w] of containerWins) if (w.el === contWin) { tgt = s; break; }
    if (tgt == null) return false;
    const r = contWin.getBoundingClientRect();
    const gx = Math.max(0, Math.min(150, Math.round(clientX - r.left)));
    const gy = Math.max(0, Math.min(120, Math.round(clientY - r.top - 20)));
    sendPlacement("drop:" + serial + ":" + gx + ":" + gy + ":0:" + tgt, serial);
    return true;
  }
  if (el && el.closest && el.closest("#paperdoll")) {
    sendPlacement("equip:" + serial + ":0", serial); // layer 0 = server derives wear layer
    return true;
  }
  if (el && el.closest && el.closest("#map")) {
    const gl = clientToGlobal(clientX, clientY);
    // A held item released over a MOBILE's on-screen body (ourself or anyone
    // else) is a server-level drop-on-mobile, same as ClassicUO's convention:
    // x=y=0xFFFF (sentinel "no tile"), z=0, container=that mobile's serial.
    // ServUO's Mobile.OnDragDrop then does the rest — AddToBackpack on
    // ourself, OpenTrade on another player/NPC (the 0x6F reply pops the trade
    // window buildTradeWindow() already builds). Checked before the plain
    // ground drop so standing on/near someone doesn't just drop at their feet.
    const mob = mobileAt(gl.x, gl.y);
    if (mob != null) {
      sendPlacement("drop:" + serial + ":65535:65535:0:" + mob, serial);
      return true;
    }
    const t = groundTileAt(gl.x, gl.y);
    sendPlacement("drop:" + serial + ":" + t.x + ":" + t.y + ":" + t.z + ":4294967295", serial);
    return true;
  }
  return false;   // other UI / empty space → keep holding
}
// Resolve which mobile (the player's own body, or another) sits under a
// renderer-space point — used to target a held-item drop at a mobile instead
// of the bare ground. `mobSprites` holds each entity's persistent part
// sprites (see drawMobs); the "#body" part is the whole-character hit target.
// Prefers an exact hit inside the body sprite's screen bounds; a nearby-center
// fallback (~28px) covers thin/foreshortened facing frames near the edges.
function mobileAt(gx, gy) {
  const HIT_R = 28;
  let best = null, bestD = Infinity;
  const test = (id, serial) => {
    const sp = mobSprites.get(id + "#body");
    if (!sp || !sp.visible) return;
    const b = sp.getBounds();
    if (b.containsPoint(gx, gy)) { best = serial; bestD = -1; return; }
    if (bestD === -1) return;                 // an exact hit already won
    const d = Math.hypot(gx - (b.x + b.width / 2), gy - (b.y + b.height / 2));
    if (d <= HIT_R && d < bestD) { best = serial; bestD = d; }
  };
  const pserial = scene && scene.player ? (scene.player.serial >>> 0) : 0;
  if (pserial) test("self", pserial);
  for (const m of (scene && scene.mobiles) || []) test("m" + (m.serial >>> 0), m.serial >>> 0);
  return best;
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
    sendPlacement("drop:" + serial + ":0:0:0:" + bp, serial);
  } else {
    const gl = clientToGlobal(lastMenuX, lastMenuY), t = groundTileAt(gl.x, gl.y);
    sendPlacement("drop:" + serial + ":" + t.x + ":" + t.y + ":" + t.z + ":4294967295", serial);
  }
  clearCursorItem();
}

// ---- stack-split dialog (ClassicUO SplitMenuGump) -------------------------
// Dragging a stack (amount > 1) without Shift pops this at the cursor instead of
// lifting the whole pile: a slider + numeric field (kept in sync) pick how many
// to take. OK/Enter confirms and feeds liftToCursor() the chosen amount, exactly
// as if that many had been dragged; Cancel/Esc/✕/clicking away abandons the drag
// and leaves the stack untouched (nothing was ever sent to the server for this
// press — the pickup packet only goes out once liftToCursor actually runs).
let splitWin = null;   // { el, serial, g, amount, clientX, clientY, input, slider } | null
function closeSplitDialog() {
  if (splitWin) { splitWin.el.remove(); splitWin = null; }
}
function confirmSplitDialog() {
  if (!splitWin) return;
  const n = Math.max(1, Math.min(splitWin.amount, parseInt(splitWin.input.value, 10) || 1));
  const { serial, g, clientX, clientY } = splitWin;
  closeSplitDialog();
  liftToCursor(serial, g, n, clientX, clientY);
}
function openSplitDialog(serial, g, amount, clientX, clientY) {
  closeSplitDialog();
  const el = document.createElement("div");
  el.className = "gump-win split-win";
  el.style.left = Math.max(4, Math.min(window.innerWidth - 180, clientX - 75)) + "px";
  el.style.top = Math.max(4, Math.min(window.innerHeight - 110, clientY - 30)) + "px";
  el.innerHTML = '<div class="gump-title"><span>Split Stack</span><span class="gump-close">✕</span></div>'
    + '<div class="gump-body split-body">'
    + `<input type="range" class="split-slider" min="1" max="${amount}" value="${amount}">`
    + `<input type="number" class="split-input" min="1" max="${amount}" value="${amount}">`
    + '<div class="split-actions"><button class="dlg-btn split-ok">OK</button>'
    + '<button class="dlg-btn split-cancel">Cancel</button></div>'
    + '</div>';
  document.body.appendChild(el);
  const slider = el.querySelector(".split-slider"), input = el.querySelector(".split-input");
  splitWin = { el, serial, g, amount, clientX, clientY, input, slider };
  // Slider and text field mirror each other (both clamped to 1..amount); typing an
  // out-of-range value just clamps rather than rejecting the keystroke.
  slider.addEventListener("input", () => { input.value = slider.value; });
  input.addEventListener("input", () => {
    slider.value = Math.max(1, Math.min(amount, parseInt(input.value, 10) || 1));
  });
  el.querySelector(".split-ok").addEventListener("click", confirmSplitDialog);
  el.querySelector(".split-cancel").addEventListener("click", closeSplitDialog);
  el.querySelector(".gump-close").addEventListener("click", closeSplitDialog);
  // Keep Enter/Esc local to this window (stopPropagation, same pattern as the
  // macro editor's win-level keydown) so the global game-input handler never also
  // sees them — it would otherwise be a no-op here anyway since isTypingTarget()
  // already skips it while the number field has focus, but this also covers a
  // click landing on the slider/buttons, which aren't typing targets.
  el.addEventListener("keydown", (e) => {
    e.stopPropagation();
    if (e.code === "Enter") { e.preventDefault(); confirmSplitDialog(); }
    else if (e.code === "Escape") { e.preventDefault(); closeSplitDialog(); }
  });
  input.focus(); input.select();
  makeDraggable(el, el.querySelector(".gump-title"));
}

// ---- server external-URL confirmation (0xA5 OpenUrl) -----------------------
// Core already accepts only bounded, credential-free absolute HTTP(S) URLs.
// Validate again at the navigation boundary so an older/misconfigured scene
// producer still cannot turn this UI into a javascript:/file: launcher.
function normalizedServerHttpUrl(raw) {
  if (typeof raw !== "string" || raw.length === 0 || raw.length > 2048) return null;
  try {
    const url = new URL(raw);
    if ((url.protocol !== "http:" && url.protocol !== "https:") || !url.hostname) return null;
    if (url.username || url.password) return null;
    return url.href;
  } catch (_) {
    return null;
  }
}

function closeOpenUrlDialog() {
  if (openUrlWin) { openUrlWin.remove(); openUrlWin = null; }
  // Yield once so a link's default new-tab navigation runs before building the
  // next queued consent dialog.
  setTimeout(showNextOpenUrlDialog, 0);
}

function showNextOpenUrlDialog() {
  if (openUrlWin || openUrlQueue.length === 0) return;
  const request = openUrlQueue.shift();
  const url = normalizedServerHttpUrl(request.url);
  if (!url) { showNextOpenUrlDialog(); return; }

  const parsed = new URL(url);
  const el = document.createElement("div");
  el.className = "gump-win open-url-win";
  el.innerHTML = '<div class="gump-title"><span>Open external page?</span><span class="gump-close">✕</span></div>'
    + '<div class="gump-body open-url-body">'
    + '<div class="open-url-warning">The game server requested permission to open this website.</div>'
    + '<div class="open-url-host"></div><code class="open-url-value"></code>'
    + '<div class="open-url-actions"></div></div>';

  el.querySelector(".open-url-host").textContent = parsed.host;
  el.querySelector(".open-url-value").textContent = url;
  const actions = el.querySelector(".open-url-actions");
  const open = document.createElement("a");
  open.className = "dlg-btn open-url-open";
  open.textContent = "Open in new tab";
  open.href = url;
  open.target = "_blank";
  open.rel = "noopener noreferrer";
  open.referrerPolicy = "no-referrer";
  const cancel = document.createElement("button");
  cancel.className = "dlg-btn open-url-cancel";
  cancel.textContent = "Cancel";
  actions.append(open, cancel);

  document.body.appendChild(el);
  openUrlWin = el;
  // Leave the anchor connected through its default activation; remove the
  // dialog on the next task after the browser has committed the new-tab open.
  open.addEventListener("click", () => setTimeout(closeOpenUrlDialog, 0));
  cancel.addEventListener("click", closeOpenUrlDialog);
  el.querySelector(".gump-close").addEventListener("click", closeOpenUrlDialog);
  el.addEventListener("keydown", (e) => {
    e.stopPropagation();
    if (e.code === "Escape") { e.preventDefault(); closeOpenUrlDialog(); }
  });
  makeDraggable(el, el.querySelector(".gump-title"));
  cancel.focus(); // safe default: Enter does not silently accept on initial focus
}

function ingestOpenUrls(s) {
  for (const event of (s && s.openUrls) || []) {
    const seq = Number(event.seq) || 0;
    if (seq <= lastOpenUrlSeq) continue;
    lastOpenUrlSeq = seq;
    const url = normalizedServerHttpUrl(event.url);
    if (url && openUrlQueue.length < 16) openUrlQueue.push({ seq, url });
  }
  showNextOpenUrlDialog();
}

// ---- Tip of the Day / Notice windows (0xA6 ScrollMessage) ------------------
function removeTipNoticeWindow(seq, notifyServer) {
  const el = tipNoticeWindows.get(seq);
  if (el) el.remove();
  tipNoticeWindows.delete(seq);
  if (notifyServer) sendInput("tipclose:" + seq); // local-only Action; no UO packet
}

function navigateTipNotice(seq, next) {
  removeTipNoticeWindow(seq, false);
  sendInput("tipnav:" + seq + ":" + (next ? "1" : "0"));
}

function openTipNoticeWindow(tip) {
  const seq = Number(tip.seq) || 0;
  if (!seq || tipNoticeWindows.has(seq)) return;
  const pageable = tip.kind === "tip";
  const el = document.createElement("div");
  el.className = "gump-win tip-notice-win " + (pageable ? "pageable" : "notice");
  el.innerHTML = '<div class="gump-title"><span></span><span class="gump-close">✕</span></div>'
    + '<div class="gump-body tip-notice-body"><div class="tip-notice-text"></div>'
    + '<div class="tip-notice-actions"></div></div>';
  el.querySelector(".gump-title span").textContent = pageable ? "Tip of the Day" : "Notice";
  el.querySelector(".tip-notice-text").textContent = String(tip.text || "");
  const actions = el.querySelector(".tip-notice-actions");
  const button = (label, fn, className) => {
    const b = document.createElement("button");
    b.className = "dlg-btn " + className;
    b.textContent = label;
    b.addEventListener("click", fn);
    actions.appendChild(b);
    return b;
  };
  if (pageable) {
    button("Previous", () => navigateTipNotice(seq, false), "tip-notice-prev");
    button("Next", () => navigateTipNotice(seq, true), "tip-notice-next");
  }
  const close = button("Close", () => removeTipNoticeWindow(seq, true), "tip-notice-close");

  // ClassicUO places pageable tips around (200,100), notices around (20,20).
  // Cascade concurrent windows slightly so repeated packets remain visible.
  const cascade = (tipNoticeWindows.size % 6) * 18;
  el.style.left = (pageable ? 200 + cascade : 20 + cascade) + "px";
  el.style.top = (pageable ? 100 + cascade : 20 + cascade) + "px";
  document.body.appendChild(el);
  tipNoticeWindows.set(seq, el);
  el.querySelector(".gump-close").addEventListener("click", () => removeTipNoticeWindow(seq, true));
  el.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    removeTipNoticeWindow(seq, true);
  });
  el.addEventListener("keydown", (e) => {
    e.stopPropagation();
    if (e.code === "Escape") { e.preventDefault(); removeTipNoticeWindow(seq, true); }
  });
  makeDraggable(el, el.querySelector(".gump-title"));
  close.focus();
}

function refreshTipNotices(s) {
  const active = new Set();
  for (const tip of (s && s.tips) || []) {
    const seq = Number(tip.seq) || 0;
    if (!seq) continue;
    active.add(seq);
    if (seq > lastTipNoticeSeq) {
      lastTipNoticeSeq = seq;
      openTipNoticeWindow(tip);
    }
  }
  // The server replied to navigation, the local close Action landed, or the
  // bounded core list expired this window: remove it without another command.
  for (const seq of [...tipNoticeWindows.keys()]) {
    if (!active.has(seq)) removeTipNoticeWindow(seq, false);
  }
}

// ---- legacy modal text-entry dialogs (0xAB / response 0xAC) ---------------
function removeTextEntryWindow(seq) {
  const el = textEntryWindows.get(seq);
  if (el) (el._modalLayer || el).remove();
  textEntryWindows.delete(seq);
}

function respondTextEntry(seq, accepted) {
  const el = textEntryWindows.get(seq);
  if (!el) return;
  const text = el._input ? el._input.value : "";
  suppressedTextEntrySeqs.add(seq);
  removeTextEntryWindow(seq);
  sendInput("textentry:" + seq + ":" + (accepted ? "1" : "0") + ":" + text);
}

function silentlyCloseTextEntry(seq) {
  const el = textEntryWindows.get(seq);
  if (!el || !el._canClose) return;
  suppressedTextEntrySeqs.add(seq);
  removeTextEntryWindow(seq);
  sendInput("textentryclose:" + seq);
}

function openTextEntryWindow(dialog) {
  const seq = Number(dialog.seq) || 0;
  if (!seq || textEntryWindows.has(seq) || suppressedTextEntrySeqs.has(seq)) return;

  const layer = document.createElement("div");
  layer.className = "text-entry-modal-layer";
  const el = document.createElement("div");
  el.className = "gump-win text-entry-win";
  el.setAttribute("role", "dialog");
  el.setAttribute("aria-modal", "true");
  el.innerHTML = '<div class="gump-title"><span>Text Entry</span><span class="gump-close">✕</span></div>'
    + '<div class="gump-body text-entry-body">'
    + '<div class="text-entry-text"></div><div class="text-entry-description"></div>'
    + '<input type="text" class="text-entry-input" autocomplete="off">'
    + '<div class="text-entry-actions"><button class="dlg-btn text-entry-ok">OK</button>'
    + '<button class="dlg-btn text-entry-cancel">Cancel</button></div></div>';

  el.querySelector(".text-entry-text").textContent = String(dialog.text || "");
  el.querySelector(".text-entry-description").textContent = String(dialog.description || "");
  const input = el.querySelector(".text-entry-input");
  const maxLength = Math.min(65522, Math.max(0, Number(dialog.maxLength) || 0));
  if (maxLength > 0) input.maxLength = maxLength;
  if ((Number(dialog.variant) || 0) === 2) {
    input.inputMode = "numeric";
    input.addEventListener("input", () => {
      const filtered = input.value.replace(/[^\p{N}]/gu, "");
      if (filtered !== input.value) input.value = filtered;
    });
  }

  el._input = input;
  el._modalLayer = layer;
  el._canClose = dialog.canClose === true;
  const close = el.querySelector(".gump-close");
  if (!el._canClose) close.hidden = true;
  else close.addEventListener("click", () => silentlyCloseTextEntry(seq));
  el.querySelector(".text-entry-ok").addEventListener("click", () => respondTextEntry(seq, true));
  el.querySelector(".text-entry-cancel").addEventListener("click", () => respondTextEntry(seq, false));
  el.addEventListener("contextmenu", (e) => {
    if (!el._canClose) return;
    e.preventDefault();
    silentlyCloseTextEntry(seq);
  });
  el.addEventListener("keydown", (e) => {
    e.stopPropagation();
    if (e.code === "Enter" || e.code === "NumpadEnter") {
      e.preventDefault();
      respondTextEntry(seq, true);
    } else if (e.code === "Escape") {
      // ClassicUO does not make this gump Esc-closeable. Keep Escape local so
      // it cannot accidentally cancel a game target while the modal has focus.
      e.preventDefault();
    }
  });

  // ClassicUO fixes these gumps at (143,172) and marks them non-movable.
  const cascade = (textEntryWindows.size % 6) * 14;
  el.style.left = (143 + cascade) + "px";
  el.style.top = (172 + cascade) + "px";
  // IsModal=true in ClassicUO: intercept all pointer input before it can reach
  // the world canvas or another gump. Button handlers still run before this
  // bubbling boundary stops the event.
  for (const eventName of ["pointerdown", "mousedown", "click", "contextmenu"]) {
    layer.addEventListener(eventName, (e) => {
      e.stopPropagation();
      if (e.target === layer || eventName === "contextmenu") e.preventDefault();
    });
  }
  layer.appendChild(el);
  document.body.appendChild(layer);
  textEntryWindows.set(seq, el);
  input.focus();
}

function refreshTextEntryDialogs(s) {
  const active = new Set();
  for (const dialog of (s && s.textEntryDialogs) || []) {
    const seq = Number(dialog.seq) || 0;
    if (!seq) continue;
    active.add(seq);
    if (!suppressedTextEntrySeqs.has(seq) && !textEntryWindows.has(seq)) {
      openTextEntryWindow(dialog);
    }
  }
  for (const seq of [...textEntryWindows.keys()]) {
    if (!active.has(seq)) removeTextEntryWindow(seq);
  }
  for (const seq of [...suppressedTextEntrySeqs]) {
    if (!active.has(seq)) suppressedTextEntrySeqs.delete(seq);
  }
}

// ---- character profiles (0xB8 request/display/update) ---------------------
function removeProfileWindow(seq) {
  const el = profileWindows.get(seq);
  if (el) el.remove();
  profileWindows.delete(seq);
}

// ClassicUO commits an editable profile when its gump is disposed. The native
// driver compares against the exact response's original body, so this command
// closes unchanged text without emitting a needless update packet.
function closeProfileWindow(seq) {
  const el = profileWindows.get(seq);
  if (!el) return;
  suppressedProfileSeqs.add(seq);
  removeProfileWindow(seq);
  if (el._canEdit) {
    sendInput("profileupdate:" + seq + ":" + el._body.value);
  } else {
    sendInput("profileclose:" + seq);
  }
}

function toggleProfileMinimized(el) {
  const minimized = !el.classList.contains("minimized");
  el.classList.toggle("minimized", minimized);
  const button = el.querySelector(".profile-minimize");
  if (button) {
    button.textContent = minimized ? "□" : "—";
    button.title = minimized ? "Restore" : "Minimize";
  }
}

function openProfileWindow(profile) {
  const seq = Number(profile.seq) || 0;
  if (!seq || profileWindows.has(seq) || suppressedProfileSeqs.has(seq)) return;

  const el = document.createElement("div");
  el.className = "gump-win profile-win";
  el.setAttribute("role", "dialog");
  const offset = (profileWindows.size % 8) * 22;
  el.style.left = (245 + offset) + "px";
  el.style.top = (86 + offset) + "px";
  el.innerHTML = '<div class="gump-title profile-title"><span>CHARACTER PROFILE</span>'
    + '<span class="profile-title-actions"><span class="gump-close profile-minimize" title="Minimize">—</span>'
    + '<span class="gump-close profile-close" title="Close">✕</span></span></div>'
    + '<div class="gump-body profile-body"><div class="profile-header"></div>'
    + '<textarea class="profile-text" maxlength="511" spellcheck="true"></textarea>'
    + '<div class="profile-footer"></div><div class="profile-mode"></div></div>';

  el.querySelector(".profile-header").textContent = String(profile.header || "");
  el.querySelector(".profile-footer").textContent = String(profile.footer || "");
  const body = el.querySelector(".profile-text");
  body.value = String(profile.body || "");
  const canEdit = profile.canEdit === true;
  body.readOnly = !canEdit;
  el.querySelector(".profile-mode").textContent = canEdit
    ? "Editable · changes save when closed"
    : "Read only";
  el._body = body;
  el._canEdit = canEdit;
  el._serial = Number(profile.serial) >>> 0;

  el.querySelector(".profile-close").addEventListener("click", () => closeProfileWindow(seq));
  el.querySelector(".profile-minimize").addEventListener("click", () => toggleProfileMinimized(el));
  const title = el.querySelector(".profile-title");
  title.addEventListener("dblclick", (event) => {
    if (event.target.closest(".gump-close")) return;
    toggleProfileMinimized(el);
  });
  el.addEventListener("contextmenu", (event) => {
    event.preventDefault();
    closeProfileWindow(seq);
  });
  el.addEventListener("mousedown", () => bringToFront(el));
  document.body.appendChild(el);
  makeDraggable(el, title);
  profileWindows.set(seq, el);
  if (canEdit) body.focus();
}

function refreshProfiles(s) {
  const active = new Set();
  for (const profile of (s && s.profiles) || []) {
    const seq = Number(profile.seq) || 0;
    if (!seq) continue;
    active.add(seq);
    if (!suppressedProfileSeqs.has(seq) && !profileWindows.has(seq)) {
      openProfileWindow(profile);
    }
  }
  for (const seq of [...profileWindows.keys()]) {
    if (!active.has(seq)) removeProfileWindow(seq);
  }
  for (const seq of [...suppressedProfileSeqs]) {
    if (!active.has(seq)) suppressedProfileSeqs.delete(seq);
  }
}

function refreshLogoutAck(s) {
  const ack = s && s.logoutAck;
  if (!ack) return;
  const seq = Number(ack.seq) || 0;
  if (!seq || seq <= lastLogoutAckSeq) return;
  lastLogoutAckSeq = seq;
  if (ack.allowed === true) return; // play server switches to auth/login immediately
  logoutPending = false;
  const options = document.getElementById("options");
  if (options && options.classList.contains("on")) renderOptions();
  setStatus("The server refused the logout request.");
}

// ---- server text-prompt dialog (ClassicUO ASCII/Unicode "enter text") -----
// scene.prompt = { active, serial, promptId, kind } (0x9A/0xC2). The
// question text itself is NOT carried on that packet — ServUO sends it
// separately as a journal/system line just before opening the prompt — so
// this is only the response box: pet rename, house sign, guild abbreviation,
// … (~38 flows).
let promptWin = null;        // the live dialog element (null = hidden)
let promptDialogKey = null;  // (kind,serial,promptId) identity this dialog was built for
// Key we just answered/canceled locally: the server often takes a beat to
// clear/replace its prompt, so a re-poll can still report the SAME key we just
// submitted for — suppress reopening it until the key actually changes (either
// to a different prompt, chained straight out of ServUO's OnResponse/OnCancel,
// or to `null` once the server catches up and clears it).
let promptSuppressKey = null;
function keyOfPrompt(p) { return p ? (p.kind || "unicode") + ":" + p.serial + ":" + p.promptId : null; }
function closePromptDialog() {
  if (promptWin) { promptWin.remove(); promptWin = null; }
  promptDialogKey = null;
}
function submitPromptDialog() {
  if (!promptWin) return;
  const text = promptWin._input.value;
  promptSuppressKey = promptDialogKey;
  closePromptDialog();
  sendInput("prompt:" + text);
}
function cancelPromptDialog() {
  if (!promptWin) return;
  promptSuppressKey = promptDialogKey;
  closePromptDialog();
  sendInput("promptcancel");
}
function openPromptDialog(key, kind) {
  closePromptDialog();
  promptDialogKey = key;
  const el = document.createElement("div");
  el.className = "gump-win prompt-win";
  const title = kind === "ascii" ? "Enter ASCII response" : "Enter response";
  el.innerHTML = '<div class="gump-title"><span>' + title + '</span><span class="gump-close">✕</span></div>'
    + '<div class="gump-body prompt-body">'
    + '<input type="text" class="prompt-input" maxlength="128">'
    + '<div class="prompt-actions"><button class="dlg-btn prompt-ok">OK</button>'
    + '<button class="dlg-btn prompt-cancel">Cancel</button></div>'
    + '</div>';
  document.body.appendChild(el);
  const input = el.querySelector(".prompt-input");
  promptWin = el;
  promptWin._input = input;
  el.querySelector(".prompt-ok").addEventListener("click", submitPromptDialog);
  el.querySelector(".prompt-cancel").addEventListener("click", cancelPromptDialog);
  el.querySelector(".gump-close").addEventListener("click", cancelPromptDialog);
  // Keep Enter/Esc local to this window (stopPropagation, same pattern as the
  // split-stack dialog) so the global game-input handler never also sees them —
  // don't leak movement/hotkey keystrokes to the game while typing a reply.
  el.addEventListener("keydown", (e) => {
    e.stopPropagation();
    if (e.code === "Enter" || e.code === "NumpadEnter") { e.preventDefault(); submitPromptDialog(); }
    else if (e.code === "Escape") { e.preventDefault(); cancelPromptDialog(); }
  });
  input.focus();
}
// Open/rebuild the dialog whenever the pending prompt's IDENTITY (serial +
// kind + promptId) differs from the one the current dialog was built for — NOT on
// the active flag's 0→1 edge. ServUO routinely chains prompts (the next
// `Prompt` is set right inside `OnResponse`/OnCancel, e.g.
// GuildCharterPrompt.cs, AdminGump.cs), so the server can go straight from
// prompt A to prompt B without ever dipping through `active:0` — an edge-only
// check would never see a transition and the dialog would never reopen for B.
// Close it once the server reports no prompt pending (e.g. it timed out
// server-side), and forget any suppressed key so a later, genuinely new
// prompt reusing old ids isn't mistaken for the one we already answered.
function refreshPrompt(s) {
  const p = s && s.prompt && s.prompt.active === 1 ? s.prompt : null;
  const key = keyOfPrompt(p);
  if (key === null) {
    closePromptDialog();
    promptSuppressKey = null;
    return;
  }
  if (key === promptSuppressKey) return; // we just answered/canceled this one — wait for the server to move on
  if (key !== promptDialogKey) openPromptDialog(key, p.kind); // fresh or chained prompt — (re)build for it
}

function setupItemDnD() {
  // Promote an armed press (world item / container icon / paperdoll icon / worn doll
  // item) into a real lift once it's unambiguously a drag: the item jumps onto the
  // cursor (UO pickup) and the ghost follows the mouse until placed. Cancelling the
  // pending single-click for this item suppresses the name-request a plain click fires.
  //
  // Listens on POINTER events, not mouse events, and this isn't cosmetic: the
  // container-cell/paperdoll-icon arm site below calls `e.preventDefault()` on
  // `pointerdown` (to suppress the browser's native image-drag/text-selection
  // gesture over the icon), and per the Pointer Events spec, canceling `pointerdown`
  // suppresses EVERY subsequent compatibility mouse event (mousemove/mouseup/click)
  // for that press — confirmed live (an instrumented `window.addEventListener`
  // counter) both on a plain test element and on the real `.cont-item` cells: a
  // genuine 40-190px drag produced zero `mousemove`/`mouseup` events and never
  // promoted at all. `pointermove`/`pointerup` are unaffected by that suppression
  // (only the legacy compatibility events are), so they're the only reliable way
  // to observe the rest of a press that started with a cancelled `pointerdown`.
  window.addEventListener("pointermove", (e) => {
    if (groundDrag && !cursorItem && !groundDrag.started) {
      if (groundDrag.rect) {
        // Container-cell / paperdoll-icon arm: it has a real on-screen cell, so
        // skip the hold/distance heuristic entirely — a double-click's drift,
        // however long held, never leaves a ~40px cell; a real drag-out does
        // almost immediately. This is what actually stops "double-click lifts
        // it" for these two sources (see the long comment above).
        const r = groundDrag.rect;
        const outside = e.clientX < r.left || e.clientX >= r.right || e.clientY < r.top || e.clientY >= r.bottom;
        if (!outside) return;
      } else {
        // World item / worn-doll figure: no natural cell to test "left it"
        // against (a world sprite's screen position pans with the camera), so
        // keep the original hold-time/distance heuristic.
        const moved = Math.max(Math.abs(e.clientX - groundDrag.sx), Math.abs(e.clientY - groundDrag.sy));
        if (moved < DRAG_THRESHOLD) return;                 // hasn't moved enough to be a drag at all
        // A small drift is a drag only if the button's been held past a tap; a big
        // motion is unambiguously a drag and lifts immediately. This lets a click /
        // double-click that wobbles a few px still resolve as a click.
        if (moved < DRAG_FAR && (performance.now() - (groundDrag.t || 0)) < DRAG_HOLD_MS) return;
      }
      groundDrag.started = true;
      if (clickPend && (clickPend.serial >>> 0) === groundDrag.serial) { clearTimeout(clickPend.timer); clickPend = null; }
      const { serial, g, amount, st } = groundDrag;
      groundDrag = null;
      // A STACKABLE stack (amount > 1): mirror ClassicUO's SplitMenuGump — a plain
      // drag opens a split dialog to pick how many to lift; SHIFT+drag skips it and
      // takes the whole pile immediately (ClassicUO GameActions.PickUp only opens
      // the gump when `ProfileManager.CurrentProfile.HoldShiftToSplitStack ==
      // Keyboard.Shift`; the profile default is `false`, so the gump shows exactly
      // when Shift is UP). `amount > 1` alone isn't enough — PickUp also requires
      // `item.ItemData.IsStackable` (`st`); a non-stackable item just lifts whole.
      // Read the live modifier off the event, not the mirrored `shiftHeld` — the
      // split dialog's own keydown handler stopPropagation()s Shift while it has
      // focus, which would otherwise leave `shiftHeld` stuck stale.
      if (amount > 1 && st && !e.shiftKey) { openSplitDialog(serial, g, amount, e.clientX, e.clientY); return; }
      liftToCursor(serial, g, amount, e.clientX, e.clientY);
      liftDrag = true;            // the lifting press is still down; its release decides one-motion placement
    }
    if (cursorItem) moveGhost(e.clientX, e.clientY);
  });
  // Release of the LIFTING press: a one-motion drag that ends over a valid target
  // places immediately; ending over nothing leaves the item held for a later click.
  // (Separate placement clicks are handled in the pointerdown listener below.)
  // `pointerup`, not `mouseup` — same compatibility-event-suppression reason as
  // the listener above; a cancelled `pointerdown` means `mouseup` never fires.
  window.addEventListener("pointerup", (e) => {
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
      if (liftDrag) return;       // the lifting press is still down → its pointerup resolves it
      if (scene && scene.target && scene.target.active === 1 && !targetUIHidden) return;
      e.preventDefault(); e.stopPropagation();
      if (e.stopImmediatePropagation) e.stopImmediatePropagation();
      placedAt = performance.now();
      if (placeCursorItem(e.clientX, e.clientY)) clearCursorItem();
      return;                     // invalid spot → keep holding (the item stays on the cursor)
    }
    const cell = e.target.closest && e.target.closest(".cont-item[data-serial]");
    if (cell) {
      // The opponent's half of an open trade window renders with the same
      // `.cont-item` markup (for the shared icon/tooltip styling) but is
      // read-only — `data-ro` marks it so we never arm a lift of an item
      // that isn't ours to move.
      if (cell.dataset.ro === "1") return;
      e.preventDefault();
      // `rect`: the cell's own screen bounds at arm time — see the promotion
      // listener above. This is what makes a double-click safe here: leaving
      // the cell is the ONLY thing that promotes, not how long/far within it.
      groundDrag = { serial: (+cell.dataset.serial) >>> 0, g: +cell.dataset.g | 0,
                     amount: (+cell.dataset.amount) || 1, st: cell.dataset.st === "1",
                     sx: e.clientX, sy: e.clientY, started: false, t: performance.now(),
                     rect: cell.getBoundingClientRect() };
      return;
    }
    if (pdTarget == null) {       // own paperdoll only — can't move another mobile's gear
      const ic = e.target.closest && e.target.closest("#paperdoll .eq-icon[data-serial]");
      if (ic) {
        e.preventDefault();
        groundDrag = { serial: (+ic.dataset.serial) >>> 0, g: +ic.dataset.g | 0,
                       amount: 1, sx: e.clientX, sy: e.clientY, started: false, t: performance.now(),
                       rect: ic.getBoundingClientRect() };
      }
    }
  }, true);
  // A mousedown anywhere outside an open split dialog abandons it (nothing was
  // ever lifted for this press, so there's nothing to undo). Its own OK/Cancel/✕
  // handle clicks on the dialog itself before this ever sees them.
  window.addEventListener("mousedown", (e) => {
    if (splitWin && !splitWin.el.contains(e.target)) closeSplitDialog();
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
                     st: !!(it && it.st), sx: e.clientX, sy: e.clientY, started: false, t: performance.now() };
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
    // A form field has focus (login form, etc.) → let it receive the keystroke;
    // don't steal movement/hotkey letters (a, w, s, d, m, b, t…) from typing.
    if (isTypingTarget(e.target)) return;
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
    if (e.code === "KeyR") { e.preventDefault(); toggleGuardZones(); return; }        // R = guard-zone lines
    // Esc while holding an item on the cursor → return it (backpack, else ground).
    // Takes priority over closing windows so a held item is never silently lost.
    if (e.code === "Escape" && cursorItem) { e.preventDefault(); returnCursorItem(); return; }
    // Belt-and-braces: the dialog's own keydown listener (stopPropagation) already
    // handles Esc while it has focus; this only catches it somehow losing focus.
    if (e.code === "Escape" && splitWin) { e.preventDefault(); closeSplitDialog(); return; }
    if (e.code === "Escape" && partyOn) { e.preventDefault(); closeParty(); return; }
    if (e.code === "Escape" && macrosOn) { e.preventDefault(); closeMacros(); return; }
    if (e.code === "Escape" && wmOn) { e.preventDefault(); closeWorldmap(); return; }
    if (e.code === "Escape" && paperdollOn) { e.preventDefault(); closePaperdoll(); return; }
    if (e.code === "Escape" && spellbookOn) { e.preventDefault(); closeSpellbook(); return; }
    if (e.code === "Escape" && skillsOn) { e.preventDefault(); closeSkills(); return; }
    if (e.code === "Escape" && shopWin) { e.preventDefault(); shopDismissed = true; closeShop(); return; }
    if (e.code === "Escape" && legacyMenuWins.size) {
      e.preventDefault();
      const serial = [...legacyMenuWins.keys()].pop();
      answerLegacyMenu(serial, 0);
      return;
    }
    if (e.code === "Escape" && popupEl) { e.preventDefault(); hidePopup(true); return; }
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
    if (isTypingTarget(e.target)) return;
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
      if (!rmbEntity.steering) { lastMenuX = e.clientX; lastMenuY = e.clientY; popupDismissed = 0; sendInput("popupreq:" + rmbEntity.serial); }
      rmbEntity = null;
    }
  });
  // Click anywhere outside an open context menu dismisses it (row clicks stop
  // propagation and dismiss themselves before this fires).
  window.addEventListener("mousedown", (e) => {
    if (popupEl && !popupEl.contains(e.target)) hidePopup(true);
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
    if (k === "guardZones") updateGuardZones(scene);
    markDirty();
  });
  optBody.addEventListener("input", (e) => {
    const k = e.target.dataset.k; if (!k || e.target.type !== "range") return;
    settings[k] = (+e.target.value) / 100;
    const v = document.getElementById("optv-" + k); if (v) v.textContent = e.target.value;
    saveSettings(); applyAudioSettings();
  });
  optBody.addEventListener("click", (e) => {
    const button = e.target.closest(".opt-logout");
    if (!button || logoutPending) return;
    if (!window.confirm("Log out of this character?")) return;
    logoutPending = true;
    button.disabled = true;
    button.textContent = "LOGGING OUT…";
    sendInput("logout");
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
  // Trade windows are wired at build time (buildTradeWindow), one per session
  // — there's no static #trade element to wire once at startup anymore.
  const pdb = document.getElementById("pd-body");
  pdb.addEventListener("click", (e) => {
    const profile = e.target.closest(".pd-profile[data-profile]");
    if (profile) {
      const serial = (+profile.dataset.profile) >>> 0;
      const existing = [...profileWindows.values()].find((win) => win._serial === serial);
      if (existing) bringToFront(existing);
      else sendInput("profile:" + serial);
      return;
    }
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
