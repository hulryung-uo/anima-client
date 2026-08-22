// Browser WASM protocol client: same Pixi page as `play`, but World lives in
// `anima-wasm` and the map window is `GET /terrain.json` from the assets bin.
// Activate with `/?wasm=1` (or `/wasm.html`, which redirects here).

const WASM_RELAY_DEFAULT = "ws://127.0.0.1:2595/relay?target=1";

let wasmClient = null;
let wasmWs = null;
let wasmInWorld = false;
let wasmLoadError = "";
let wasmInitPromise = null;
let WasmClientCtor = null;
let wasmTerrain = { map: { tiles: [] }, statics: [], lights: [] };
let wasmTerrainKey = "";
let wasmTerrainGen = 0;
let wasmJournal = [];
let wasmJournalSeq = 0;

function wasmFlush() {
  if (!wasmClient || !wasmWs || wasmWs.readyState !== 1) return;
  const bytes = wasmClient.take_outbox();
  if (bytes && bytes.length) wasmWs.send(bytes);
}

function wasmEnsure() {
  if (wasmInitPromise) return wasmInitPromise;
  wasmInitPromise = import("./pkg/anima_wasm.js")
    .then(async (mod) => {
      await mod.default();
      WasmClientCtor = mod.WasmClient;
    })
    .catch((err) => {
      wasmLoadError =
        "Missing web/pkg (wasm-pack build crates/anima-wasm --target web --out-dir web/pkg). " +
        err;
    });
  return wasmInitPromise;
}

function wasmParseSerial(s) {
  s = String(s || "").trim();
  if (/^0x/i.test(s)) {
    const n = parseInt(s, 16);
    return Number.isFinite(n) ? n >>> 0 : 0;
  }
  const n = Number(s);
  return Number.isFinite(n) ? n >>> 0 : 0;
}

function wasmSplitOnce(s, sep) {
  const i = s.indexOf(sep);
  if (i < 0) return [s, ""];
  return [s.slice(0, i), s.slice(i + sep.length)];
}

function wasmCommandToAction(cmd) {
  const body = String(cmd || "").trim();
  if (!body || body === "stop") return null;
  if (body.startsWith("hdesign:")) return wasmHdesignAction(body.slice(8));
  const [verb, arg] = wasmSplitOnce(body, ":");
  switch (verb) {
    case "walk": {
      const p = arg.split(":");
      return { type: "Walk", dir: (Number(p[0]) || 0) & 7, run: p[1] === "1" };
    }
    case "run":
      return { type: "Walk", dir: (Number(arg) || 0) & 7, run: true };
    case "say":
    case "whisper":
    case "yell":
    case "emote":
    case "guild":
    case "alliance":
      return { type: "Say", text: arg, mode: verb };
    case "party":
      return { type: "PartySay", text: arg };
    case "use":
      return { type: "Use", serial: wasmParseSerial(arg) };
    case "click":
      return { type: "Click", serial: wasmParseSerial(arg) };
    case "attack":
      return { type: "Attack", serial: wasmParseSerial(arg) };
    case "autoattack":
      return { type: "AutoAttack" };
    case "attacklast":
      return { type: "AttackLast" };
    case "pickup": {
      const p = arg.split(":");
      return { type: "PickUp", serial: wasmParseSerial(p[0]), amount: Number(p[1]) || 1 };
    }
    case "drop": {
      const p = arg.split(":");
      return {
        type: "Drop",
        serial: wasmParseSerial(p[0]),
        x: Number(p[1]) || 0,
        y: Number(p[2]) || 0,
        z: Number(p[3]) || 0,
        container: p[4] != null && p[4] !== "" ? wasmParseSerial(p[4]) : 0xffffffff,
      };
    }
    case "equip": {
      const p = arg.split(":");
      return { type: "Equip", serial: wasmParseSerial(p[0]), layer: Number(p[1]) || 0 };
    }
    case "war":
      return { type: "WarMode", on: arg === "1" || arg === "on" };
    case "cast":
      return { type: "CastSpell", spell: Number(arg) || 0 };
    case "castbook": {
      const [spell, book] = wasmSplitOnce(arg, ":");
      return { type: "CastSpellFromBook", spell: Number(spell) || 0, book: wasmParseSerial(book) };
    }
    case "opendoor":
      return { type: "OpenDoor" };
    case "lastweapon":
      return { type: "EquipLastWeapon" };
    case "allnames":
      return { type: "AllNames" };
    case "racechange": {
      const p = arg.split(":");
      if (p.length < 5) return null;
      return {
        type: "ChangeRace",
        skin_hue: Number(p[0]) || 0,
        hair_style: Number(p[1]) || 0,
        hair_hue: Number(p[2]) || 0,
        beard_style: Number(p[3]) || 0,
        beard_hue: Number(p[4]) || 0,
      };
    }
    case "racechangecancel":
      return { type: "ChangeRaceCancel" };
    case "uostore":
      return { type: "OpenUOStore" };
    case "virtue":
      return { type: "InvokeVirtue", id: Number(arg) || 0 };
    case "animate":
      return arg ? { type: "EmoteAction", action: arg } : null;
    case "ability":
      return { type: "UseAbility", ability: Number(arg) || 0 };
    case "disarm":
      return { type: "DisarmRequest" };
    case "stun":
      return { type: "StunRequest" };
    case "flying":
      return { type: "ToggleFlying" };
    case "useskill":
      return { type: "UseSkill", skill: Number(arg) || 0 };
    case "skilllock": {
      const [skill, lock] = wasmSplitOnce(arg, ":");
      return { type: "SkillLock", skill: Number(skill) || 0, lock: Number(lock) || 0 };
    }
    case "statlock": {
      const [stat, lock] = wasmSplitOnce(arg, ":");
      return { type: "StatLock", stat: Number(stat) || 0, lock: Number(lock) || 0 };
    }
    case "target":
      return { type: "TargetObject", serial: wasmParseSerial(arg) };
    case "targetxy": {
      const p = arg.split(":");
      return {
        type: "TargetGround",
        x: Number(p[0]) || 0,
        y: Number(p[1]) || 0,
        z: Number(p[2]) || 0,
        graphic: Number(p[3]) || 0,
      };
    }
    case "targetcancel":
      return { type: "TargetCancel" };
    case "oplreq":
      return { type: "OplRequest", serial: wasmParseSerial(arg) };
    case "gump": {
      const p = arg.split(":");
      const action = {
        type: "GumpResponse",
        serial: wasmParseSerial(p[0]),
        gump_id: wasmParseSerial(p[1]),
        button: wasmParseSerial(p[2] || "0"),
        switches: [],
        entries: [],
      };
      for (const seg of p.slice(3)) {
        if (seg.startsWith("sw=")) {
          action.switches = seg.slice(3).split(",").filter(Boolean).map(Number);
        } else if (seg.startsWith("e=")) {
          for (const pair of seg.slice(2).split(",")) {
            const [id, text] = wasmSplitOnce(pair, "=");
            action.entries.push({ id: Number(id) || 0, text: text || "" });
          }
        }
      }
      return action;
    }
    case "popupreq":
      return { type: "PopupRequest", serial: wasmParseSerial(arg) };
    case "popupsel": {
      const [serial, index] = wasmSplitOnce(arg, ":");
      return { type: "PopupSelect", serial: wasmParseSerial(serial), index: Number(index) || 0 };
    }
    case "menusel": {
      const [serial, index] = wasmSplitOnce(arg, ":");
      return { type: "LegacyMenuSelect", serial: wasmParseSerial(serial), index: Number(index) || 0 };
    }
    case "boat": {
      const p = arg.split(":");
      return { type: "BoatMove", dir: (Number(p[0]) || 0) & 7, run: p[1] === "1" };
    }
    case "boatstop":
      return { type: "BoatStop" };
    case "logout":
      return { type: "Logout" };
    case "help":
      return { type: "HelpRequest" };
    case "guildmenu":
      return { type: "GuildMenu" };
    case "questmenu":
      return { type: "QuestMenu" };
    case "bandage": {
      const [bandage, target] = wasmSplitOnce(arg, ":");
      return {
        type: "BandageTarget",
        bandage: wasmParseSerial(bandage),
        target: target ? wasmParseSerial(target) : 0,
      };
    }
    default:
      return null;
  }
}

function wasmHdesignAction(rest) {
  const [cmd, arg] = wasmSplitOnce(rest, ":");
  const p = arg.split(":").filter((s) => s.length);
  const n = (i) => Number(p[i]);
  switch (cmd) {
    case "add":
      return { type: "HouseDesign", cmd: "AddItem", graphic: n(0), x: n(1), y: n(2) };
    case "del":
      return { type: "HouseDesign", cmd: "DeleteItem", graphic: n(0), x: n(1), y: n(2), z: n(3) };
    case "stair":
      return { type: "HouseDesign", cmd: "AddStair", graphic: n(0), x: n(1), y: n(2) };
    case "roof":
      return { type: "HouseDesign", cmd: "AddRoof", graphic: n(0), x: n(1), y: n(2), z: n(3) };
    case "roofdel":
      return { type: "HouseDesign", cmd: "DeleteRoof", graphic: n(0), x: n(1), y: n(2), z: n(3) };
    case "floor":
      return { type: "HouseDesign", cmd: "GoToFloor", floor: n(0) };
    case "commit":
      return { type: "HouseDesign", cmd: "Commit" };
    case "close":
      return { type: "HouseDesign", cmd: "Close" };
    case "clear":
      return { type: "HouseDesign", cmd: "Clear" };
    case "revert":
      return { type: "HouseDesign", cmd: "Revert" };
    case "backup":
      return { type: "HouseDesign", cmd: "Backup" };
    case "restore":
      return { type: "HouseDesign", cmd: "Restore" };
    case "sync":
      return { type: "HouseDesign", cmd: "Sync" };
    default:
      return null;
  }
}

function wasmSendInput(cmd) {
  if (!wasmClient) return;
  const action = wasmCommandToAction(cmd);
  if (!action) return;
  const err = wasmClient.apply_action_json(JSON.stringify(action));
  if (err) console.warn("wasm action", err, action);
  wasmFlush();
}

function wasmPos(p) {
  return p && typeof p === "object" ? p : { x: 0, y: 0, z: 0 };
}

function wasmAtype(body) {
  body = body | 0;
  return body < 200 ? 0 : body < 400 ? 1 : 2;
}

function wasmEquipFor(obs, serial) {
  const out = [];
  for (const it of obs.items || []) {
    if ((it.container >>> 0) !== (serial >>> 0) || !(it.layer | 0)) continue;
    out.push({ serial: it.serial, layer: it.layer, g: it.graphic, anim: 0, hue: 0 });
  }
  return out;
}

function wasmMounted(obs, serial) {
  return (obs.items || []).some(
    (it) => (it.container >>> 0) === (serial >>> 0) && (it.layer | 0) === 25 && (it.graphic | 0)
  ) ? 1 : 0;
}

function wasmGumpElement(e) {
  if (!e || typeof e !== "object") return e;
  if (e.t) return e;
  const t = e.type;
  const page = e.page | 0;
  switch (t) {
    case "background":
      return { t: "bg", x: e.x, y: e.y, w: e.w, h: e.h, page };
    case "image":
      return { t: "bg", x: e.x, y: e.y, page };
    case "button":
      return { t: "button", x: e.x, y: e.y, g: e.graphic, id: e.reply_id, page, pageflag: e.pageflag, param: e.param };
    case "text":
      return e.w != null ? { t: "text", x: e.x, y: e.y, w: e.w, s: e.s, page } : { t: "text", x: e.x, y: e.y, s: e.s, page };
    case "html": {
      let s = "";
      const text = e.text;
      if (text && typeof text.literal === "string") s = text.literal;
      else if (text && text.cliloc) s = "#" + (text.cliloc.id | 0);
      return { t: "text", x: e.x, y: e.y, w: e.w, s, page };
    }
    case "check":
      return { t: "check", x: e.x, y: e.y, id: e.id, on: e.on, page };
    case "radio":
      return { t: "radio", x: e.x, y: e.y, id: e.id, on: e.on, g: e.group, page };
    case "entry":
      return { t: "entry", x: e.x, y: e.y, w: e.w, id: e.id, s: e.s, lim: e.limit, page };
    case "tilepic":
      return { t: "tilepic", x: e.x, y: e.y, g: e.graphic, hue: e.hue, page };
    default:
      return { t: t || "bg", x: e.x, y: e.y, page };
  }
}

function wasmMergeScene(obs) {
  const p = obs.player || {};
  const pos = wasmPos(p.pos);
  const serial = p.serial >>> 0;
  for (const j of obs.new_journal || []) {
    wasmJournalSeq += 1;
    wasmJournal.push({
      seq: wasmJournalSeq,
      serial: j.serial >>> 0,
      name: j.name || "",
      text: j.display || j.text || "",
      type: j.msg_type | 0,
      hue: j.hue | 0,
      cliloc: j.cliloc | 0,
    });
  }
  while (wasmJournal.length > 50) wasmJournal.shift();
  const mobiles = (obs.mobiles || []).map((m) => {
    const mp = wasmPos(m.pos);
    const body = m.body | 0;
    return {
      serial: m.serial,
      x: mp.x, y: mp.y, z: mp.z, dir: 0,
      body, at: wasmAtype(body), noto: m.notoriety | 0, name: m.name || "",
      hits: m.hits, hitsMax: m.hits_max, hue: 0,
      equip: wasmEquipFor(obs, m.serial),
      mounted: wasmMounted(obs, m.serial), mountAnim: 0, mountOff: 0,
    };
  });
  const items = [];
  const contItems = [];
  for (const it of obs.items || []) {
    if (it.is_multi) continue;
    if (it.container != null) {
      const cp = wasmPos(it.pos);
      contItems.push({
        serial: it.serial, cont: it.container, g: it.graphic, amount: it.amount,
        x: cp.x, y: cp.y, c: 0,
      });
      continue;
    }
    const ip = wasmPos(it.pos);
    const row = {
      x: ip.x, y: ip.y, z: ip.z, g: it.graphic, serial: it.serial, amount: it.amount, pz: ip.z,
    };
    if ((it.graphic | 0) === 0x2006) {
      row.body = it.amount | 0;
      row.dir = 0;
      row.dg = 0;
      row.hue = 0;
    }
    items.push(row);
  }
  const skills = (obs.skills || []).map((s) => ({
    id: s.id, v: Math.round((s.value || 0) * 10), b: Math.round((s.base || 0) * 10),
    c: Math.round((s.cap || 0) * 10), lock: s.lock | 0,
  }));
  const gumps = (obs.gumps || []).map((g) => ({
    serial: g.serial, gumpId: g.gump_id, x: 80, y: 80, w: 0, h: 0,
    elements: (g.elements || []).map(wasmGumpElement),
  }));
  const tgt = obs.pending_target;
  const weather = obs.weather || {};
  const party = obs.party || {};
  const members = (party.members || []).map((serialOrObj) => {
    if (serialOrObj && typeof serialOrObj === "object") return serialOrObj;
    const m = mobiles.find((x) => (x.serial >>> 0) === (serialOrObj >>> 0));
    return {
      serial: serialOrObj >>> 0,
      name: (m && m.name) || "Member",
      hits: (m && m.hits) || 0, hitsMax: (m && m.hitsMax) || 0,
      mana: 0, manaMax: 0, stam: 0, stamMax: 0,
    };
  });
  return {
    player: {
      serial, x: pos.x, y: pos.y, z: pos.z, dir: p.direction | 0,
      body: p.body | 0, dead: !!p.dead, at: wasmAtype(p.body), name: p.name || "",
      noto: 0, hue: 0, mounted: wasmMounted(obs, serial), mountAnim: 0, mountOff: 0,
      hits: p.hits, hitsMax: p.hits_max, mana: p.mana, manaMax: p.mana_max,
      stam: p.stam, stamMax: p.stam_max,
      str: p.strength, dex: p.dexterity, int: p.intelligence, gold: p.gold,
      armor: p.armor, resistFire: p.fire_resistance, resistCold: p.cold_resistance,
      resistPoison: p.poison_resistance, resistEnergy: p.energy_resistance,
      weight: p.weight, weightMax: p.weight_max, followers: p.followers, followersMax: p.followers_max,
      race: p.race | 0,
      maxResistPhysical: p.maxResistPhysical, maxResistFire: p.maxResistFire,
      maxResistCold: p.maxResistCold, maxResistPoison: p.maxResistPoison,
      maxResistEnergy: p.maxResistEnergy, defenseChance: p.defenseChance,
      defenseChanceMax: p.defenseChanceMax, hitChance: p.hitChance,
      swingSpeed: p.swingSpeed, damageChance: p.damageChance, lowerRegCost: p.lowerRegCost,
      spellDamage: p.spellDamage, fasterCastRecovery: p.fasterCastRecovery,
      fasterCasting: p.fasterCasting, lowerManaCost: p.lowerManaCost,
      equip: wasmEquipFor(obs, serial),
    },
    map: wasmTerrain.map || { tiles: [] },
    statics: wasmTerrain.statics || [],
    lights: wasmTerrain.lights || [],
    mobiles, items, contItems,
    journal: wasmJournal,
    skills, gumps,
    buffs: obs.buffs || [],
    target: tgt ? { active: 1, kind: tgt.target_type | 0 } : { active: 0, kind: 0 },
    war: !!obs.war,
    lastAttack: obs.last_attack || 0,
    combatant: obs.combatant || 0,
    aos: !!obs.aos,
    facet: obs.map_index | 0,
    season: obs.season | 0,
    light: obs.light | 0,
    weather: weather.kind | 0,
    weatherN: weather.intensity | 0,
    questArrow: obs.quest_arrow || null,
    party: { leader: party.leader || 0, members, invite: party.pending_invite || 0 },
    armedAbility: obs.armed_ability | 0,
    activeSpells: obs.active_spell_icons || [],
    shop: null, popup: null, legacyMenus: [], huePickers: obs.hue_pickers || [],
    raceChange: obs.race_change || null, tips: [],
    textEntryDialogs: [], profiles: [], logoutAck: null, boatMoves: [],
    book: null, spellbooks: [], opl: {}, prompt: { active: 0 },
    liftRejects: [], dragCompletions: [], deathScreen: null, containerOpens: [],
    swings: [], paperdoll: null, openUrls: [], trades: [], maps: [],
    deaths: [], sounds: [], anims: [], tanims: [], damage: [], effects: [],
    dragAnims: [], music: null, chat: null, bboard: null,
    contGumps: {}, contInfo: {},
    net: { pingUs: null, in: 0, out: 0, pin: 0, pout: 0 },
    stats: { confirms: 0, denies: 0 },
  };
}

async function wasmRefreshTerrain(obs) {
  const pos = wasmPos(obs.player && obs.player.pos);
  const key = `${pos.x | 0},${pos.y | 0},${pos.z | 0},${obs.map_index | 0},${obs.season | 0}`;
  if (key === wasmTerrainKey) return;
  const gen = ++wasmTerrainGen;
  try {
    const r = await fetch(
      `terrain.json?x=${pos.x | 0}&y=${pos.y | 0}&z=${pos.z | 0}&map=${obs.map_index | 0}&season=${obs.season | 0}`
    );
    if (!r.ok || gen !== wasmTerrainGen) return;
    wasmTerrain = await r.json();
    wasmTerrainKey = key;
  } catch (_) { /* assets bin down — keep last window */ }
}

async function wasmPollScene() {
  await wasmEnsure();
  if (wasmLoadError) return { auth: "error", msg: wasmLoadError };
  if (!wasmClient) return { auth: "login" };
  const err = wasmClient.login_error();
  if (err) return { auth: "error", msg: err };
  let obs = {};
  try { obs = JSON.parse(wasmClient.observation_json()); } catch (_) { obs = {}; }
  if (obs.player && obs.player.serial) {
    wasmInWorld = true;
    await wasmRefreshTerrain(obs);
    wasmFlush();
    return wasmMergeScene(obs);
  }
  let list = {};
  try { list = JSON.parse(wasmClient.character_list_json()); } catch (_) { list = {}; }
  if (Array.isArray(list.slots)) {
    const rej = list.deleteRejected;
    return {
      auth: "characters",
      slots: list.slots,
      cities: list.cities || [],
      capacity: list.slotCount || 0,
      error: rej && rej.text,
    };
  }
  return { auth: "connecting", msg: "Connecting…" };
}

function wasmDisconnect() {
  wasmInWorld = false;
  wasmClient = null;
  if (wasmWs) {
    try { wasmWs.close(); } catch (_) {}
    wasmWs = null;
  }
}

function wasmRelayUrl() {
  const el = document.getElementById("lg-relay");
  return ((el && el.value) || WASM_RELAY_DEFAULT).trim();
}

async function wasmConnect(username, password) {
  await wasmEnsure();
  if (wasmLoadError) throw new Error(wasmLoadError);
  if (wasmWs) {
    try { wasmWs.close(); } catch (_) {}
    wasmWs = null;
  }
  wasmClient = new WasmClientCtor(username, password);
  wasmInWorld = false;
  wasmJournal = [];
  wasmJournalSeq = 0;
  wasmTerrainKey = "";
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(wasmRelayUrl());
    wasmWs = ws;
    ws.binaryType = "arraybuffer";
    ws.onopen = () => { wasmFlush(); resolve(); };
    ws.onmessage = (ev) => {
      if (!wasmClient) return;
      wasmClient.feed(new Uint8Array(ev.data));
      wasmFlush();
    };
    ws.onerror = () => reject(new Error("WebSocket error (is anima-relay running?)"));
    ws.onclose = () => {
      if (!wasmInWorld) {
        const msg = document.getElementById("lg-msg");
        if (msg && msg.textContent === "Connecting…") msg.textContent = "disconnected";
      }
    };
  });
}

async function wasmSubmitLogin() {
  const username = (document.getElementById("lg-user").value || "").trim();
  const password = document.getElementById("lg-pass").value || "";
  const msg = document.getElementById("lg-msg");
  if (!username) { if (msg) msg.textContent = "Enter an account name."; return; }
  if (msg) msg.textContent = "Connecting…";
  try {
    await wasmConnect(username, password);
  } catch (e) {
    if (msg) msg.textContent = "Login request failed: " + e.message;
    const go = document.getElementById("lg-go");
    const back = document.getElementById("lg-back");
    if (go) go.disabled = false;
    if (back) back.disabled = false;
  }
}

function wasmPlaySlot(slot) {
  const msg = document.getElementById("lg-msg");
  if (!wasmClient || slot == null) {
    if (msg) msg.textContent = "Select a character.";
    return;
  }
  if (!wasmClient.play_character(slot)) {
    if (msg) msg.textContent = wasmClient.login_error() || "could not play that slot";
    const go = document.getElementById("lg-go");
    if (go) go.disabled = false;
    return;
  }
  wasmFlush();
}

function wasmCreateCharacter(create) {
  if (!wasmClient) return "not connected";
  const err = wasmClient.create_character(JSON.stringify(create));
  wasmFlush();
  return err;
}

function wasmDeleteSlot(slot) {
  if (!wasmClient) return false;
  const ok = wasmClient.delete_character(slot);
  wasmFlush();
  return ok;
}

function wasmPrepareLoginUi() {
  const title = document.querySelector(".login-title");
  if (title) title.textContent = "anima-wasm";
  const row = document.getElementById("lg-relay-row");
  if (row) row.style.display = "";
  const relay = document.getElementById("lg-relay");
  if (relay && !relay.value) relay.value = WASM_RELAY_DEFAULT;
}

if (WASM_MODE) wasmEnsure();

main();
