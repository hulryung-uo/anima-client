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
function liftToCursor(serial, g, amount, clientX, clientY, hue) {
  serial = serial >>> 0; g = g | 0; amount = amount || 1; hue = hue | 0;
  cursorItem = { serial, g, amount, hue };
  sendInput("pickup:" + serial + (amount > 1 ? ":" + amount : ""));
  if (dragGhost) { dragGhost.remove(); dragGhost = null; }
  const img = document.createElement("img");
  img.src = `art/static/${g}.png${hueQuery(hue)}`;
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
  // so the target comes from the enclosing .trade-win via its dialog family, not a
  // single global session). The opponent's side (.tr-theirs-grid) is
  // intentionally NOT a target here — we can't place items into their half.
  const tradeGrid = el && el.closest && el.closest(".tr-mine-grid");
  const tradeWinEl = tradeGrid && tradeGrid.closest(".trade-win");
  if (tradeWinEl) {
    let tgt = null;
    for (const [s, w] of dialogWindows("trades")) if (w.el === tradeWinEl) { tgt = s; break; }
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
    for (const [s, w] of dialogWindows("containers")) if (w.el === contWin) { tgt = s; break; }
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
let splitWin = null;   // { el, serial, g, amount, hue, clientX, clientY, input, slider } | null
function closeSplitDialog() {
  if (splitWin) { splitWin.el.remove(); splitWin = null; }
}
function confirmSplitDialog() {
  if (!splitWin) return;
  const n = Math.max(1, Math.min(splitWin.amount, parseInt(splitWin.input.value, 10) || 1));
  const { serial, g, hue, clientX, clientY } = splitWin;
  closeSplitDialog();
  liftToCursor(serial, g, n, clientX, clientY, hue);
}
function openSplitDialog(serial, g, amount, clientX, clientY, hue) {
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
  splitWin = { el, serial, g, amount, hue: hue | 0, clientX, clientY, input, slider };
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
function textEntryEl(seq) {
  const win = dialogWindow("textEntry", seq);
  return win ? win.el : null;
}

function respondTextEntry(seq, accepted) {
  const el = textEntryEl(seq);
  if (!el) return;
  const text = el._input ? el._input.value : "";
  dismissDialog("textEntry", seq);
  sendInput("textentry:" + seq + ":" + (accepted ? "1" : "0") + ":" + text);
}

function silentlyCloseTextEntry(seq) {
  const el = textEntryEl(seq);
  if (!el || !el._canClose) return;
  dismissDialog("textEntry", seq);
  sendInput("textentryclose:" + seq);
}

function buildTextEntryWindow(dialog) {
  const seq = Number(dialog.seq) || 0;

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
  const cascade = (dialogWindows("textEntry").size % 6) * 14;
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
  input.focus();
  return { el };
}

registerDialog({
  id: "textEntry",
  source: (s) => ((s && s.textEntryDialogs) || []).filter((d) => Number(d.seq)),
  key: (d) => Number(d.seq) || 0,
  // No sig: the server never revises an OPEN text-entry dialog — it sends a new
  // seq instead — so the window is built once and then left alone.
  dismiss: "session",
  build: buildTextEntryWindow,
  close: (win) => (win.el._modalLayer || win.el).remove(), // the modal layer owns the window
});

// ---- character profiles (0xB8 request/display/update) ---------------------
// ClassicUO commits an editable profile when its gump is disposed. The native
// driver compares against the exact response's original body, so this command
// closes unchanged text without emitting a needless update packet.
function closeProfileWindow(seq) {
  const win = dialogWindow("profiles", seq);
  if (!win) return;
  const el = win.el;
  dismissDialog("profiles", seq);
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

function buildProfileWindow(profile) {
  const seq = Number(profile.seq) || 0;

  const el = document.createElement("div");
  el.className = "gump-win profile-win";
  el.setAttribute("role", "dialog");
  const offset = (dialogWindows("profiles").size % 8) * 22;
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
  el.addEventListener("mousedown", (e) => {
    // Don't re-append (bringToFront = appendChild) when the press is on the ✕ /
    // minimize button: moving the element in the DOM during mousedown cancels
    // the subsequent click in Chrome, so the close button would never fire.
    if (e.target.closest(".gump-close")) return;
    bringToFront(el);
  });
  document.body.appendChild(el);
  makeDraggable(el, title);
  if (canEdit) body.focus();
  return { el };
}

registerDialog({
  id: "profiles",
  source: (s) => ((s && s.profiles) || []).filter((p) => Number(p.seq)),
  key: (p) => Number(p.seq) || 0,
  dismiss: "session",
  build: buildProfileWindow,
});

// Shared logout flow for both the Options-panel button (.opt-logout) and the
// HUD's visible "log out" button (#logoutbtn): confirm, guard against a
// second click while a request is already in flight, then fire the same 0xD1
// "logout" input command. refreshLogoutAck() below undoes the pending state
// if the server refuses it.
function requestLogout() {
  if (logoutPending) return;
  if (!window.confirm("Log out of this character?")) return;
  logoutPending = true;
  setLogoutButtonsPending(true);
  sendInput("logout");
}
// Reflect logoutPending on whichever logout button(s) currently exist in the
// DOM — the Options one only exists once that panel has been rendered.
function setLogoutButtonsPending(pending) {
  const hudBtn = document.getElementById("logoutbtn");
  if (hudBtn) { hudBtn.disabled = pending; hudBtn.textContent = pending ? "…" : "⎋ log out"; }
  const optBtn = document.querySelector(".opt-logout");
  if (optBtn) { optBtn.disabled = pending; optBtn.textContent = pending ? "LOGGING OUT…" : "LOG OUT"; }
}
function refreshLogoutAck(s) {
  const ack = s && s.logoutAck;
  if (!ack) return;
  const seq = Number(ack.seq) || 0;
  if (!seq || seq <= lastLogoutAckSeq) return;
  lastLogoutAckSeq = seq;
  if (ack.allowed === true) return; // play server switches to auth/login immediately
  logoutPending = false;
  setLogoutButtonsPending(false);
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
// The server often takes a beat to clear/replace its prompt, so a re-poll can
// still report the SAME key we just submitted for — the family's session guard
// keeps it closed until the key actually changes (either to a different prompt,
// chained straight out of ServUO's OnResponse/OnCancel, or away entirely once
// the server catches up).
function keyOfPrompt(p) { return p ? (p.kind || "unicode") + ":" + p.serial + ":" + p.promptId : null; }
function closePromptDialog() {
  if (promptDialogKey) closeDialog("prompt", promptDialogKey);
}
function submitPromptDialog() {
  if (!promptWin) return;
  const text = promptWin._input.value;
  dismissDialog("prompt", promptDialogKey);
  sendInput("prompt:" + text);
}
function cancelPromptDialog() {
  if (!promptWin) return;
  dismissDialog("prompt", promptDialogKey);
  sendInput("promptcancel");
}
function openPromptDialog(key, kind) {
  promptDialogKey = key;
  const { el, body } = makeWindowFrame({
    cls: "prompt-win", bodyCls: "prompt-body",
    title: kind === "ascii" ? "Enter ASCII response" : "Enter response",
    onClose: cancelPromptDialog,
  });
  body.innerHTML = '<input type="text" class="prompt-input" maxlength="128">'
    + '<div class="prompt-actions"><button class="dlg-btn prompt-ok">OK</button>'
    + '<button class="dlg-btn prompt-cancel">Cancel</button></div>';
  const input = el.querySelector(".prompt-input");
  promptWin = el;
  promptWin._input = input;
  el.querySelector(".prompt-ok").addEventListener("click", submitPromptDialog);
  el.querySelector(".prompt-cancel").addEventListener("click", cancelPromptDialog);
  // Keep Enter/Esc local to this window (stopPropagation, same pattern as the
  // split-stack dialog) so the global game-input handler never also sees them —
  // don't leak movement/hotkey keystrokes to the game while typing a reply.
  el.addEventListener("keydown", (e) => {
    e.stopPropagation();
    if (e.code === "Enter" || e.code === "NumpadEnter") { e.preventDefault(); submitPromptDialog(); }
    else if (e.code === "Escape") { e.preventDefault(); cancelPromptDialog(); }
  });
  input.focus();
  return { el };
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
registerDialog({
  id: "prompt",
  // Keyed by the prompt's IDENTITY, not by an active-flag edge: ServUO routinely
  // CHAINS prompts (the next one is set right inside OnResponse/OnCancel — see
  // GuildCharterPrompt.cs, AdminGump.cs), so it can go straight from prompt A to
  // prompt B without ever dipping through active:0. A new key here is a new
  // dialog; an edge-only check would never see the transition.
  source: (s) => (s && s.prompt && s.prompt.active === 1 ? [s.prompt] : []),
  key: keyOfPrompt,
  dismiss: "session",
  build: (p, { key }) => openPromptDialog(key, p.kind),
  close: (win) => { win.el.remove(); promptWin = null; promptDialogKey = null; },
});


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
      const { serial, g, amount, st, hue } = groundDrag;
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
      if (amount > 1 && st && !e.shiftKey) { openSplitDialog(serial, g, amount, e.clientX, e.clientY, hue); return; }
      liftToCursor(serial, g, amount, e.clientX, e.clientY, hue);
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
                     hue: +cell.dataset.hue | 0,
                     sx: e.clientX, sy: e.clientY, started: false, t: performance.now(),
                     rect: cell.getBoundingClientRect() };
      return;
    }
    if (pdTarget == null) {       // own paperdoll only — can't move another mobile's gear
      const ic = e.target.closest && e.target.closest("#paperdoll .eq-icon[data-serial]");
      if (ic) {
        e.preventDefault();
        groundDrag = { serial: (+ic.dataset.serial) >>> 0, g: +ic.dataset.g | 0,
                       amount: 1, hue: +ic.dataset.hue | 0,
                       sx: e.clientX, sy: e.clientY, started: false, t: performance.now(),
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

