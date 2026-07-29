// ── dialog window lifecycle ────────────────────────────────────────────────
// One implementation of "keep a set of windows in sync with the current scene
// snapshot", shared by every server-driven dialog family (generic gumps, legacy
// menus, hue pickers, trades, maps, containers, book, shop, popup, prompt,
// text-entry, profiles, the house-design panel).
//
// Why this exists rather than a refresh function per family: the snapshot in
// `scene` is POLLED, so one that was BUILT BEFORE our reply reached the server
// still lists a dialog the player just answered. Every family therefore has to
// remember what it dismissed and skip it until the server catches up, or the
// window visibly pops back for one poll before closing for good. That's four
// steps (record on close, skip while dismissed, drop the record once the item
// leaves the snapshot, and don't confuse a genuinely new dialog for the
// dismissed one) repeated once per family — and two of the thirteen copies were
// missing them, which is exactly the bug this replaced. Declaring a family here
// gets all four for free; forgetting them is no longer possible.
//
// A family is declared with registerDialog() and never calls its own refresh:
// syncDialogs(scene) drives all of them once per poll, in registration order.

// Every registered family, in registration order (windows layer in that order
// on a poll that opens several at once).
const dialogFamilies = [];

// How a family decides WHICH windows should exist:
//   "list"   the snapshot's list IS the window set — open what appears, close
//            what disappears. The default, and what most dialogs are.
//   "seq"    the item carries a monotonic open-counter; a window opens only when
//            that counter ADVANCES, so closing one locally doesn't reopen it
//            while the item lingers in the snapshot. Still auto-closes when the
//            item leaves. (Treasure maps: the server re-sends the same map item
//            on every update, and only a fresh 0x90/0xF5 bumps `openSeq`.)
//   "local"  app code owns opening and closing (a double-click, a 0x24 event);
//            the snapshot only supplies CONTENT. Never auto-opens, never
//            auto-closes. (Containers.)
const OPEN_POLICIES = new Set(["list", "seq", "local"]);

// How a family suppresses a window the player closed, until the server agrees:
//   "content"  remember key → signature; skip while the snapshot still shows
//              that exact content. If the server sends DIFFERENT content under
//              the same key that's a real new dialog, so it opens. Needed
//              wherever the transport can reuse a key (World::add_gump upserts
//              gumps by serial).
//   "session"  remember the key alone; skip until the key leaves the snapshot.
//              For dialogs whose content keeps changing right up until the
//              server tears them down — a trade's items and offered gold move
//              after the player hits cancel, and a content-keyed guard would let
//              one of those updates reopen the window.
//   "none"     nothing closes locally; the window only ever goes away because
//              the server dropped the item. (The house-design panel: its ✕ asks
//              the server to end the session and waits for that to land.)
const DISMISS_POLICIES = new Set(["content", "session", "none"]);

// Declare a dialog family. Fields:
//   id       name, for errors and debugging
//   source   (scene) => items[]. A singleton dialog returns [] or [theOne];
//            that's the whole reason singletons need no special case here.
//            Omitted for open:"local" (there is no server-side list).
//   key      (item) => stable identity (serial, seq, composite string, …).
//            Windows are kept in a Map under this.
//   sig      (item, scene) => string, the content signature: what "changed"
//            means. `scene` is there for families whose content lives outside
//            their own item (a trade's goods are in scene.contItems). For
//            open:"local" it takes (scene, key) instead, since there is no item.
//            Omit when content never changes after build.
//   seq      (item) => number. Required for open:"seq".
//   build    (item, ctx) => win, where win is at least { el }. ctx is
//            {key, sig, scene, previous}; `previous` is the window being replaced on a
//            rebuild (else undefined), so a family can carry LOCAL ui state
//            across server-driven content changes — the selected page of a
//            multi-page gump, say, which the server's refresh shouldn't reset.
//   update   (win, item, ctx) => void. Optional. WITH it, a signature change
//            updates the window in place; WITHOUT it, a signature change tears
//            the window down and rebuilds it (right for gumps/menus, whose
//            layout is server-authored and can change wholesale).
//   close    (win, key) => void. Optional teardown; defaults to removing win.el.
//            Use it for families that hold state beyond the element.
//   reopen   (win, item) => void. open:"seq" only: the server re-issued a dialog
//            that is ALREADY open (the counter advanced but the window never
//            went away) — raise it, rather than silently doing nothing.
function registerDialog(spec) {
  const open = spec.open || "list";
  const dismiss = spec.dismiss || "none";
  if (!OPEN_POLICIES.has(open)) throw new Error(`dialog ${spec.id}: bad open policy ${open}`);
  if (!DISMISS_POLICIES.has(dismiss)) throw new Error(`dialog ${spec.id}: bad dismiss policy ${dismiss}`);
  if (open === "seq" && !spec.seq) throw new Error(`dialog ${spec.id}: open:"seq" needs seq()`);
  if (open === "local" && dismiss !== "none") {
    // Nothing re-opens a "local" window from the snapshot, so there is nothing
    // for a guard to protect against — asking for one means the family was
    // modelled wrong.
    throw new Error(`dialog ${spec.id}: open:"local" cannot use a dismiss guard`);
  }
  const family = {
    ...spec,
    open,
    dismiss,
    wins: new Map(),       // key -> win
    dismissed: new Map(),  // key -> signature ("content") or true ("session")
    seqSeen: new Map(),    // key -> highest seq we've opened for (open:"seq")
  };
  dialogFamilies.push(family);
  return family;
}

function dialogFamily(id) {
  const family = dialogFamilies.find((f) => f.id === id);
  if (!family) throw new Error(`no dialog family ${id}`);
  return family;
}

// The window for `key`, or undefined. Lets app code reach into a family it
// declared (a click handler needing its own window's inputs, say).
function dialogWindow(id, key) {
  return dialogFamily(id).wins.get(key);
}
function dialogWindows(id) {
  return dialogFamily(id).wins;
}

// Tear a window down without recording a dismissal — for closes the SERVER
// initiated, and for app code that manages its own "local" windows.
function closeDialog(id, key) {
  const family = dialogFamily(id);
  const win = family.wins.get(key);
  if (!win) return;
  family.wins.delete(key);
  if (family.close) family.close(win, key);
  else if (win.el) win.el.remove();
}

// The player closed/answered this window: tear it down NOW (no waiting a poll
// for the server's echo) and remember it, so a snapshot built before the reply
// landed can't rebuild it. This is the call every ✕ / Cancel / submit path
// should make; see the DISMISS_POLICIES note for what gets remembered.
function dismissDialog(id, key) {
  const family = dialogFamily(id);
  const win = family.wins.get(key);
  if (family.dismiss === "content") family.dismissed.set(key, win ? win._sig : null);
  else if (family.dismiss === "session") family.dismissed.set(key, true);
  closeDialog(id, key);
}

// The player explicitly asked for this dialog again (re-right-clicked the same
// mobile, say). Drop the guard so it can reopen even though the server still has
// the identical item in the snapshot and would otherwise stay suppressed.
function undismissDialog(id, key) {
  dialogFamily(id).dismissed.delete(key);
}

// True while `key` is being suppressed for this snapshot's content.
function isDialogDismissed(family, key, sig) {
  if (!family.dismissed.has(key)) return false;
  if (family.dismiss === "session") return true;
  if (family.dismissed.get(key) === sig) return true;
  // Same key, different content — a genuinely new dialog. Stop suppressing.
  family.dismissed.delete(key);
  return false;
}

// Reconcile one family against the snapshot.
function syncDialogFamily(family, scene) {
  // "local": the window set is app-owned, so there's nothing to open or close
  // here — only content to push into whatever is already on screen.
  if (family.open === "local") {
    for (const [key, win] of [...family.wins]) {
      const sig = family.sig ? family.sig(scene, key) : null;
      if (sig !== null && win._sig === sig) continue;
      win._sig = sig;
      if (family.update) family.update(win, scene, { key, sig, scene });
    }
    return;
  }

  const items = family.source(scene) || [];
  const seen = new Set();
  for (const item of items) {
    const key = family.key(item);
    seen.add(key);
    const sig = family.sig ? family.sig(item, scene) : null;
    if (isDialogDismissed(family, key, sig)) continue;

    let win = family.wins.get(key);
    if (!win) {
      // A "seq" family opens only on a counter it hasn't seen yet; note the
      // counter either way so a window closed later doesn't spring back.
      if (family.open === "seq") {
        const seq = family.seq(item) | 0;
        const highest = family.seqSeen.get(key) | 0;
        family.seqSeen.set(key, Math.max(highest, seq));
        if (seq <= highest) continue;
      }
      win = family.build(item, { key, sig, scene });
      if (!win) continue;                // build declined (nothing to show yet)
      win._sig = sig;
      family.wins.set(key, win);
      if (family.update) family.update(win, item, { key, sig, scene });
      continue;
    }
    if (family.open === "seq") {
      const seq = family.seq(item) | 0;
      const highest = family.seqSeen.get(key) | 0;
      family.seqSeen.set(key, Math.max(highest, seq));
      if (seq > highest && family.reopen) family.reopen(win, item);
    }
    if (sig !== null && win._sig === sig) continue;   // unchanged
    if (family.update) {
      win._sig = sig;
      family.update(win, item, { key, sig, scene });
    } else {
      // No in-place update: the layout is server-authored, so rebuild it — but
      // hand the outgoing window to build() so local ui state can survive.
      const previous = win;
      closeDialog(family.id, key);
      const rebuilt = family.build(item, { key, sig, scene, previous });
      if (rebuilt) { rebuilt._sig = sig; family.wins.set(key, rebuilt); }
    }
  }

  // Gone from the snapshot → the server closed it.
  for (const key of [...family.wins.keys()]) {
    if (!seen.has(key)) closeDialog(family.id, key);
  }
  // …and once it's gone, stop suppressing it, so a later dialog reusing the key
  // isn't swallowed and the guard map can't grow without bound.
  for (const key of [...family.dismissed.keys()]) {
    if (!seen.has(key)) family.dismissed.delete(key);
  }
  // Same for the open-counter high-water marks.
  for (const key of [...family.seqSeen.keys()]) {
    if (!seen.has(key)) family.seqSeen.delete(key);
  }
}

// Adopt the snapshot's open-counters WITHOUT opening anything — the open:"seq"
// half of main.js's `primeSeqRings`. Call once, on the first snapshot after page
// load, or every dialog the world still happens to be carrying (a treasure map
// opened before a refresh) would pop open as if the server had just sent it.
function primeDialogSeqs(scene) {
  for (const family of dialogFamilies) {
    if (family.open !== "seq") continue;
    for (const item of family.source(scene) || []) {
      family.seqSeen.set(family.key(item), family.seq(item) | 0);
    }
  }
}

// Drive every family. Call once per poll, after `scene` is swapped in.
function syncDialogs(scene) {
  for (const family of dialogFamilies) syncDialogFamily(family, scene);
}

// ── shared window chrome ───────────────────────────────────────────────────
// The dark title bar + ✕ + draggable body that every dialog wears. Returns the
// pieces a family needs; the family fills `body` with its own content.
//
// `cascade` staggers successive windows of the same family so they don't stack
// exactly on top of each other; pass the family's own counter object. `pos`
// overrides it outright (a remembered per-kind position, say).
function makeWindowFrame({ cls, title, bodyCls, cascade, pos, onClose, draggable = true }) {
  const el = document.createElement("div");
  el.className = "gump-win" + (cls ? " " + cls : "");
  if (pos) {
    el.style.left = pos.left + "px";
    el.style.top = pos.top + "px";
  } else if (cascade) {
    const off = (cascade.n++ % (cascade.wrap || 8)) * (cascade.step || 22);
    el.style.left = (cascade.left + off) + "px";
    el.style.top = (cascade.top + off) + "px";
  }
  const bar = document.createElement("div");
  bar.className = "gump-title";
  const label = document.createElement("span");
  label.textContent = title || "";
  const closer = document.createElement("span");
  closer.className = "gump-close";
  closer.textContent = "✕";
  bar.appendChild(label);
  bar.appendChild(closer);
  const body = document.createElement("div");
  body.className = "gump-body" + (bodyCls ? " " + bodyCls : "");
  el.appendChild(bar);
  el.appendChild(body);
  document.body.appendChild(el);
  if (onClose) closer.addEventListener("click", onClose);
  if (draggable) makeDraggable(el, bar);
  return { el, bar, body, label, closer };
}
