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
STATIC_GRAY.brightness(1.1, true);
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
// Mouse-wheel camera zoom: scales app.stage (world sprites auto-scale as its
// children); world-anchored HTML/2D-canvas overlays fold camZoom into their
// manual (stage + worldIso) math — see drawOverheads/drawDamage/drawBars/fxFrame.
let camZoom = 1;
const ZOOM_MIN = 0.6, ZOOM_MAX = 2.6;
// Wheel/trackpad zoom rate: camZoom *= exp(-deltaY * K). At K=0.001 a clamped
// mouse notch (|deltaY|→100) is ~10% per notch (the old fixed-factor feel), while
// a trackpad's many tiny deltas accumulate smoothly instead of compounding.
const ZOOM_WHEEL_K = 0.001;
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
// ---- character profile windows (0xB8) ----
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
  autoOpenDoors: true,            // walking into a closed door opens it (ClassicUO TryOpenDoors) — on by default
  gridLoot: true,                 // corpses open a click-to-loot grid instead of the authentic corpse gump
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
    + cb("autoOpenDoors", "Auto-open doors")
    + cb("gridLoot", "Click-to-loot corpses")
    + '<label class="opt-row"><button type="button" class="dlg-btn opt-infobar">Info bar</button></label>'
    + '<label class="opt-row"><button type="button" class="dlg-btn opt-counterbar">Counter bar</button></label>'
    + '<label class="opt-row"><button type="button" class="dlg-btn opt-ignorelist">Ignore list</button></label>'
    + '<label class="opt-row"><button type="button" class="dlg-btn opt-combatbook">Combat book</button></label>'
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

