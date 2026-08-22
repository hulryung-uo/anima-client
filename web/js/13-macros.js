// ---- user macros / hotkeys (client-only; persisted in localStorage) ----
// A macro is { id, key, ctrl, alt, shift, action } where `key` is a KeyboardEvent
// `e.code` and `action` is one of:
//   { t:"say", text } · { t:"cast", id } · { t:"skill", id } · { t:"ability", id }
//   { t:"war", on:0|1|"toggle" } · { t:"open", win:"paperdoll|backpack|spellbook|skills|minimap|worldmap" }
//   { t:"opendoor" } · { t:"lastweapon" } · { t:"allnames" } · { t:"virtue", id } · { t:"emote", text }
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
    case "opendoor": return "open door";
    case "lastweapon": return "equip last weapon";
    case "allnames": return "all names";
    case "virtue": return `virtue #${a.id}`;
    case "emote": return `emote ${a.text}`;
    case "drawroofs": return "toggle draw roofs";
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
    case "cast": castSpell(a.id); break;
    case "skill": sendInput("useskill:" + a.id); break;
    case "ability": sendInput("ability:" + a.id); break;
    case "opendoor": sendInput("opendoor"); break;
    case "lastweapon": sendInput("lastweapon"); break;
    case "allnames": sendInput("allnames"); break;
    case "virtue": sendInput("virtue:" + a.id); break;
    case "emote": if (a.text) sendInput("animate:" + a.text); break;
    case "war": {
      let on = a.on;
      if (on === "toggle") { warOn = warOn ? 0 : 1; on = warOn; }
      else { on = a.on ? 1 : 0; warOn = on; }
      sendInput("war:" + on);
      break;
    }
    case "open": { const fn = OPEN_FNS[a.win]; if (fn) fn(); break; }
    case "drawroofs":
      settings.drawRoofs = !settings.drawRoofs;
      saveSettings();
      rebuildStatics();
      rebuildItems();
      break;
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
  else if (t === "emote") p.innerHTML = '<input id="mc-pv" class="mc-input" type="text" maxlength="32" placeholder="bow, salute, …" />';
  else if (t === "cast" || t === "skill" || t === "ability" || t === "virtue")
    p.innerHTML = '<input id="mc-pv" class="mc-input" type="number" min="0" placeholder="id" />';
  else if (t === "war")
    p.innerHTML = '<select id="mc-pv" class="mc-input"><option value="toggle">toggle</option><option value="1">on</option><option value="0">off</option></select>';
  else if (t === "open")
    p.innerHTML = '<select id="mc-pv" class="mc-input"><option>paperdoll</option><option>backpack</option><option>spellbook</option><option>skills</option><option>minimap</option><option>worldmap</option><option>status</option></select>';
  else p.innerHTML = "";
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
    } else if (t === "cast" || t === "skill" || t === "ability" || t === "virtue") {
      const id = parseInt(pv.value, 10);
      if (!Number.isFinite(id) || id < 0) { msg.textContent = "Enter a valid numeric id."; return; }
      action = { t, id };
    } else if (t === "emote") {
      const text = pv.value.trim();
      if (!text) { msg.textContent = "Enter the emote verb (bow, salute, …)."; return; }
      action = { t: "emote", text };
    } else if (t === "opendoor" || t === "lastweapon" || t === "allnames") {
      action = { t };
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

// ---- fixed HUD panel drag persistence (localStorage) ----
function loadPanelPos(key) {
  try { const p = JSON.parse(localStorage.getItem(key)); if (p && Number.isFinite(p.x) && Number.isFinite(p.y)) return p; } catch (e) {}
  return null;
}
function savePanelPos(key, x, y) { try { localStorage.setItem(key, JSON.stringify({ x, y })); } catch (e) {} }
// Clamp a saved/target position into the current viewport (matches makeDraggable's clamp).
function clampPanel(x, y) {
  return { x: Math.max(0, Math.min(window.innerWidth - 40, x)), y: Math.max(0, Math.min(window.innerHeight - 24, y)) };
}

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
            castSpell(id);
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
    // +/- (and numpad +/-) = camera zoom in/out, same step as the mouse wheel.
    if (e.code === "Equal" || e.code === "NumpadAdd") {
      e.preventDefault(); camZoom = Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, camZoom * 1.1)); markDirty(); return;
    }
    if (e.code === "Minus" || e.code === "NumpadSubtract") {
      e.preventDefault(); camZoom = Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, camZoom / 1.1)); markDirty(); return;
    }
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
    if (e.code === "Escape" && shopWin) { e.preventDefault(); dismissDialog("shop", SHOP_KEY); return; }
    if (e.code === "Escape" && dialogWindows("legacyMenus").size) {
      e.preventDefault();
      const serial = [...dialogWindows("legacyMenus").keys()].pop();
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
    // Always drop a walk key, even if focus has moved into a text field —
    // `isTypingTarget` on keyup is what left W/↑ latched and walking north.
    if (e.code in KEY_DIR) { held.delete(KEY_DIR[e.code]); if (!held.size) trace("KU"); }
  });
  window.addEventListener("blur", releaseMoveKeys);
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) releaseMoveKeys();
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
    // Custom house design takes priority over everything below while a session is
    // open (scene.houseDesign) — see handleHouseDesignClick's doc comment above. It
    // returns false whenever there's no session, so ordinary target-cursor/steering
    // clicks are completely unaffected — this is a pure no-op against an older
    // server that never sends scene.houseDesign.
    if (handleHouseDesignClick(e)) return;
    if (inspectPick) {
      const gl = clientToGlobal(e.clientX, e.clientY);
      const t = groundTileAt(gl.x, gl.y);
      inspectPicked({ x: t.x, y: t.y });
      return;
    }
    if (!(scene && scene.target && scene.target.active === 1) || targetUIHidden) return;
    if (performance.now() - targetConsumedAt < 200) return; // a mob/item already answered
    // Our own avatar is always at the canvas centre but isn't a click target (so it
    // never eats steering). During a target cursor, a click on that centre band IS
    // self — answer with target:<self> so bandages / beneficial spells work on us.
    // The band must track the avatar's on-screen size, which scales with camZoom
    // (the sprite is a child of app.stage, scaled by camZoom). dxp/dyp and the
    // 28/68/14 constants are already CSS-client px tuned at zoom 1, so the per-window
    // stretch cancels and only camZoom needs folding in — else a self-heal misses
    // the (larger) body when zoomed in, or grabs nearby ground tiles when zoomed out.
    const r = canvas.getBoundingClientRect();
    const dxp = (e.clientX - r.left) - r.width / 2, dyp = (e.clientY - r.top) - r.height / 2;
    if (scene.player && Math.abs(dxp) < 28 * camZoom && dyp > -68 * camZoom && dyp < 14 * camZoom) {
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
  // Multi placement preview: recompute the hovered tile while a placement
  // target is pending (scene.placement) — a plain target cursor gives no sense
  // of where a house will land or how big it is, so we track the tile here and
  // let updatePlacementPreview draw the footprint under it. Cheap early-out
  // when no placement is pending, which is every ordinary mousemove.
  canvas.addEventListener("mousemove", (e) => {
    if (!(scene && scene.placement)) return;
    const g = clientToGlobal(e.clientX, e.clientY);
    const t = groundTileAt(g.x, g.y);
    if (t.x === placementHoverX && t.y === placementHoverY) return; // same tile → no rebuild
    placementHoverX = t.x; placementHoverY = t.y;
    updatePlacementPreview();
  });
  // House-design ghost: same idea, while a design session is open — track the
  // hovered tile so updateHouseDesignGhost can show the selected piece there.
  // Cheap early-out when there's no session, which is every ordinary mousemove
  // against an older server (or outside design mode entirely).
  canvas.addEventListener("mousemove", (e) => {
    if (!(scene && scene.houseDesign)) return;
    const g = clientToGlobal(e.clientX, e.clientY);
    const t = groundTileAt(g.x, g.y);
    if (t.x === hdesignHoverX && t.y === hdesignHoverY) return; // same tile → no rebuild
    hdesignHoverX = t.x; hdesignHoverY = t.y;
    updateHouseDesignGhost();
  });
  window.addEventListener("mouseup", (e) => {
    if (e.button !== 2) return;
    endRightMouse(e.clientX, e.clientY);
  });
  // Same release path as mouseup: a cancelled pointerdown (PIXI drag) never
  // emits mouseup, which latched RMB-steer and ran the character unattended.
  window.addEventListener("pointerup", (e) => {
    if (e.button !== 2) return;
    endRightMouse(e.clientX, e.clientY);
  });
  window.addEventListener("pointercancel", releaseMoveKeys);
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
    const cell = e.target.closest && e.target.closest(".cont-item[data-serial], .cb-slot[data-serial]");
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
  document.getElementById("logoutbtn")?.addEventListener("click", () => requestLogout());
  // Options panel: button toggles, ✕ closes, title bar drags. Changes persist
  // immediately and apply live (audio volume now; display toggles next repaint).
  const optEl = document.getElementById("options");
  document.getElementById("optbtn")?.addEventListener("click", () => toggleOptions());
  document.getElementById("opt-close")?.addEventListener("click", () => toggleOptions(false));
  makeDraggable(optEl, optEl.querySelector(".gump-title"));
  const optBody = document.getElementById("opt-body");
  // The category rail: switching tabs just re-renders the body.
  document.getElementById("opt-tabs")?.addEventListener("click", (e) => {
    const b = e.target.closest("[data-cat]");
    if (!b) return;
    optCat = b.dataset.cat;
    renderOptions();
  });
  // One dispatcher per EVENT KIND, not one for both. A checkbox click fires
  // `input` THEN `change`, and a range fires `input` on every move plus `change`
  // on release — so a single listener bound to both would run each option's
  // onChange twice (a double `rebuildStatics()` on every toggle, which is a
  // visible hitch). Checkboxes therefore listen on `change` only, ranges on
  // `input` only, each ignoring anything whose descriptor is the other kind.
  const optDesc = (k) => OPTIONS.find((o) => o.key === k);
  optBody.addEventListener("change", (e) => {
    const k = e.target.dataset.k; if (!k) return;
    const o = optDesc(k); if (!o || o.kind !== "checkbox") return;
    settings[k] = e.target.checked;
    saveSettings();
    if (o.onChange) o.onChange();
    markDirty();
  });
  optBody.addEventListener("input", (e) => {
    const k = e.target.dataset.k; if (!k) return;
    const o = optDesc(k); if (!o || (o.kind !== "range" && o.kind !== "intRange")) return;
    // `intRange` carries its own units; `range` is a 0..1 value shown ×100.
    settings[k] = o.kind === "intRange" ? +e.target.value : (+e.target.value) / 100;
    const span = document.getElementById("optv-" + k);
    if (span) span.textContent = e.target.value;
    saveSettings();
    if (o.onChange) o.onChange();
    markDirty();
  });
  optBody.addEventListener("click", (e) => {
    if (e.target.closest(".opt-journal")) { toggleJournal(); return; }
    if (e.target.closest(".opt-infobar")) { toggleInfoBar(); return; }
    if (e.target.closest(".opt-counterbar")) { toggleCounterBar(); return; }
    if (e.target.closest(".opt-ignorelist")) { toggleIgnoreList(); return; }
    if (e.target.closest(".opt-combatbook")) { toggleCombatBook(); return; }
    if (e.target.closest(".opt-racialbook")) { toggleRacialBook(); return; }
    if (e.target.closest(".opt-netstats")) { toggleNetStats(); return; }
    if (e.target.closest(".opt-inspector")) { toggleInspector(); return; }
    if (!e.target.closest(".opt-logout")) return;
    requestLogout();
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
  // HUD panel: draggable by its title (also carries #status, which lives inside it).
  const hudEl = document.getElementById("hud");
  const hudTitle = hudEl.querySelector("h1");
  makeDraggable(hudEl, hudTitle, (x, y) => savePanelPos("anima.hudPos", x, y));
  {
    const p = loadPanelPos("anima.hudPos");
    if (p) { const c = clampPanel(p.x, p.y); hudEl.style.left = c.x + "px"; hudEl.style.top = c.y + "px"; hudEl.style.right = "auto"; }
  }
  // Minimap cluster: minimap canvas is the drag handle; #minilabel and #buffs
  // (neither has a safe handle of their own — the label has a click handler, the
  // buff bar is pointer-events:none) follow it by their fixed default-CSS offsets.
  const miniEl = document.getElementById("minimap");
  const labelEl = document.getElementById("minilabel");
  const buffsEl = document.getElementById("buffs");
  const mr = miniEl.getBoundingClientRect(), lr = labelEl.getBoundingClientRect(), br = buffsEl.getBoundingClientRect();
  const dLabel = { x: lr.left - mr.left, y: lr.top - mr.top };
  const dBuffs = { x: br.left - mr.left, y: br.top - mr.top };
  const placeCluster = (x, y) => {
    miniEl.style.left = x + "px"; miniEl.style.top = y + "px"; miniEl.style.right = "auto";
    labelEl.style.left = (x + dLabel.x) + "px"; labelEl.style.top = (y + dLabel.y) + "px";
    buffsEl.style.left = (x + dBuffs.x) + "px"; buffsEl.style.top = (y + dBuffs.y) + "px";
  };
  makeDraggable(miniEl, miniEl, (x, y) => { placeCluster(x, y); savePanelPos("anima.miniPos", x, y); });
  {
    const p = loadPanelPos("anima.miniPos");
    if (p) { const c = clampPanel(p.x, p.y); placeCluster(c.x, c.y); }
  }
  // Trade windows are wired at build time (buildTradeWindow), one per session
  // — there's no static #trade element to wire once at startup anymore.
  const pdb = document.getElementById("pd-body");
  pdb.addEventListener("click", (e) => {
    const profile = e.target.closest(".pd-profile[data-profile]");
    if (profile) {
      const serial = (+profile.dataset.profile) >>> 0;
      const existing = [...dialogWindows("profiles").values()].find((w) => w.el._serial === serial);
      if (existing) bringToFront(existing.el);
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
                   amount: 1, hue: +img.dataset.hue | 0,
                   sx: e.clientX, sy: e.clientY, started: false };
  });
  pdb.addEventListener("mouseleave", () => {
    if (pdTipEl && pdTipEl.closest && pdTipEl.closest("#pd-doll")) { pdTipEl = null; hideTip(); }
  });
}
// In-game chat bar (replaces window.prompt). Enter opens it; type and Enter sends;
// Esc cancels. A leading prefix routes the channel; anything else is normal
// in-game speech.
//
// Each entry maps a set of typed prefixes to the `/input` command that carries
// it. Party is its own packet (0xBF/0x06); the rest are ordinary speech with a
// different MessageType byte, which the server routes — see
// `anima_core::agent::SpeechMode`.
const CHAT_PREFIXES = [
  { keys: ["p", "party"], cmd: "party" },
  { keys: ["w", "whisper"], cmd: "whisper" },
  { keys: ["y", "yell", "shout"], cmd: "yell" },
  { keys: ["e", "em", "emote", "me"], cmd: "emote" },
  { keys: ["g", "guild"], cmd: "guild" },
  { keys: ["a", "alliance", "ally"], cmd: "alliance" },
  // Server chat system (0xB2). `/c hello` talks in the joined channel; the
  // channel verbs take a name. `chatopen` first — see `parse_command`.
  { keys: ["c", "chat"], cmd: "chatsay" },
  { keys: ["cjoin", "chatjoin"], cmd: "chatjoin" },
  { keys: ["ccreate", "chatcreate"], cmd: "chatcreate" },
];
// The same table for verbs that take NO argument: `submitChat`'s pattern
// requires a trailing word, so these would otherwise be spoken out loud.
const CHAT_BARE_PREFIXES = [
  { keys: ["chatopen"], cmd: "chatopen" },
  { keys: ["cleave", "chatleave"], cmd: "chatleave" },
];
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
    const bare = /^\/([a-z]+)$/i.exec(text);
    const bareRoute = bare && CHAT_BARE_PREFIXES.find((p) => p.keys.includes(bare[1].toLowerCase()));
    const m = /^\/([a-z]+)\s+(.+)$/i.exec(text);
    const route = m && CHAT_PREFIXES.find((p) => p.keys.includes(m[1].toLowerCase()));
    // An unrecognized "/word" is spoken verbatim rather than swallowed — the
    // shard may well define it as a server command (e.g. "/help").
    if (bareRoute) sendInput(bareRoute.cmd);
    else if (route) sendInput(route.cmd + ":" + m[2].trim());
    else sendInput("say:" + text);
  }
  closeChat();
}
function sendInput(cmd) {
  if (WASM_MODE) wasmSendInput(cmd);
  else fetch("/input", { method: "POST", body: cmd }).catch(() => {});
}
// ---- movement diagnostic trace (POSTs to /log; server prints with ANIMA_DEBUG) ----
let TRACE = false;
function trace(m) { if (TRACE) fetch("/log", { method: "POST", body: Math.round(performance.now()) + " " + m }).catch(() => {}); }

