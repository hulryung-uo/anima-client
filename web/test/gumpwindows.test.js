// The window layer of web/js/09-gumps.js and 10-housing.js: right-click to
// close and who opts out of it, the client-side context menu, the party panel's
// out-of-range dimming, and the stat lockers.
//
// The dimming is the one worth reading twice. The obvious test for "can the
// server still see this party member" is "are they in scene.mobiles?", and this
// code used to use it — but ServUO's Mobile.SetLocation only tells clients about
// the mobile that MOVED, so a member 171 tiles away is still sitting in the
// snapshot with the position they had when you last saw them. The right signal
// is the server's own 0xF0 tracking list, which reports exactly the members you
// CANNOT see. Both readings are pinned below, in a scene where they disagree.
const { newContext } = require("./harness.js");
const { test, ok, eq, deepEq } = require("./run.js");

function uiCtx() {
  const ctx = newContext();
  ctx.mountPage();
  ctx.loadAll();
  const sent = [];
  ctx.setFetch((u, init) => {
    if (String(u) === "/input") { sent.push(String(init && init.body)); return {}; }
    return { status: 404, ok: false, body: null };
  });
  ctx.sent = sent;
  // dom.js stops bubbling at <html> unless a node names its event parent; the
  // client's right-click-close is a DELEGATED listener on `document`, so the
  // chain has to reach it the way a browser's does.
  ctx.document.documentElement.__eventParent = ctx.document;
  ctx.run(`
    app = { canvas: { style: {}, clientWidth: 800, clientHeight: 600,
                      getBoundingClientRect: () => ({ left: 0, top: 0, width: 800, height: 600 }) },
            renderer: { width: 800, height: 600, events: { cursorStyles: {} } },
            stage: { position: { x: 0, y: 0 } } };
    world = new PIXI.Container();
  `);
  return ctx;
}
const rightClick = (ctx, el) => {
  const ev = ctx.event("contextmenu", { bubbles: true, clientX: 100, clientY: 100 });
  el.dispatchEvent(ev);
  return ev;
};

// ── right-click closes a window (ClassicUO Gump.CloseWithRightClick) ───────

test("right-clicking a gump runs its own ✕ — one close path, not a second one", () => {
  const ctx = uiCtx();
  ctx.run("scene = { player: { serial: 9, x: 5, y: 5, equip: [] }, mobiles: [], items: [] };");
  ctx.run("__closed = 0; makeWindowFrame({ cls: 'probe-win', title: 'Probe', onClose: () => { __closed++; } })");
  const win = ctx.document.querySelector(".probe-win");
  ok(win, "a window is up");
  const ev = rightClick(ctx, win.querySelector(".gump-body"));
  eq(ctx.run("__closed"), 1, "the ✕'s own handler ran, exactly once");
  eq(ev.defaultPrevented, true, "and Chrome's own menu is suppressed over the client");
});

test("the info bar and counter bar opt out — ClassicUO's CanCloseWithRightClick = false set", () => {
  // InfoBarGump.cs:28 and CounterBarGump.cs:147: a right-click there opens that
  // window's own context menu instead of closing it.
  const ctx = uiCtx();
  deepEq(ctx.run("[...RIGHT_CLICK_KEEP_OPEN].sort()"), ["counterbar", "infobar"], "the opt-out set");
  for (const id of ["infobar", "counterbar"]) {
    const el = ctx.document.getElementById(id);
    el.classList.add("on");
    const ev = rightClick(ctx, el);
    eq(ev.defaultPrevented, true, `#${id}: Chrome's menu is still suppressed`);
    eq(el.classList.contains("on"), true, `#${id}: but the window stays open`);
  }
});

test("a text field keeps the browser's own cut/copy/paste menu", () => {
  // ClassicUO never had to decide this; there is no in-game equivalent to
  // replace it with in a book page, a profile or the chat bar.
  const ctx = uiCtx();
  ctx.run("makeWindowFrame({ cls: 'probe-win', title: 'Probe', onClose: () => { __closed = true; } })");
  const body = ctx.document.querySelector(".probe-win .gump-body");
  body.innerHTML = '<textarea class="probe-text"></textarea>';
  const ev = rightClick(ctx, ctx.document.querySelector(".probe-text"));
  eq(ev.defaultPrevented, false, "the browser menu is allowed through");
  eq(ctx.run("typeof __closed"), "undefined", "and the window did not close");
});

test("a window that already handled its own right-click is not closed a second time", () => {
  // Containers, tip notices, text-entry dialogs and profiles preventDefault() in
  // their own handler; the delegated one must see that and stand down.
  const ctx = uiCtx();
  ctx.run("__mine = 0; makeWindowFrame({ cls: 'probe-win', title: 'Probe', onClose: () => { __closed = true; } })");
  const win = ctx.document.querySelector(".probe-win");
  win.addEventListener("contextmenu", (e) => { e.preventDefault(); ctx.run("__mine++;"); });
  rightClick(ctx, win.querySelector(".gump-body"));
  eq(ctx.run("__mine"), 1, "the window's own close ran");
  eq(ctx.run("typeof __closed"), "undefined", "…and the delegated one did not run a second one");
});

test("a right-click on bare page space suppresses Chrome's menu and closes nothing", () => {
  const ctx = uiCtx();
  const ev = rightClick(ctx, ctx.document.body);
  eq(ev.defaultPrevented, true, "no Chrome menu anywhere over the client");
});

// ── the client-side context menu (ClassicUO ContextMenuControl) ────────────

const menu = (ctx) => ctx.document.querySelector(".client-menu");
const rows = (ctx) => (menu(ctx) ? menu(ctx).querySelectorAll(".popup-row") : []);

test("a client menu draws its rows, its separators and its tick column", () => {
  const ctx = uiCtx();
  ctx.run(`__flag = false;
    openClientMenu(100, 100, [
      { label: "Plain", run: () => { __ran = "plain"; } },
      {},
      { label: "Toggle", checked: () => __flag, run: () => { __flag = !__flag; } },
      { label: "Off", checked: false },
      { label: "Greyed", disabled: true, run: () => { __ran = "greyed"; } },
    ]);`);
  eq(rows(ctx).length, 4, "four rows — the label-less entry is a separator, not a row");
  eq(menu(ctx).querySelectorAll(".popup-sep").length, 1, "…and it drew one");
  eq(rows(ctx)[0].textContent, "Plain", "a plain entry has no tick column at all");
  eq(rows(ctx)[1].textContent, "  Toggle", "an unticked checkable keeps its column, so nothing jitters");
  eq(rows(ctx)[3].className.includes("popup-row-off"), true, "a disabled row is marked");
  rows(ctx)[3].click();
  eq(ctx.run("typeof __ran"), "undefined", "…and does not run");
});

test("clicking an entry runs it and closes the menu", () => {
  const ctx = uiCtx();
  ctx.run(`openClientMenu(100, 100, [{ label: "Go", run: () => { __ran = true; } }]);`);
  rows(ctx)[0].click();
  eq(ctx.run("__ran"), true, "it ran");
  eq(menu(ctx), null, "and the menu is gone");
  eq(ctx.run("clientMenuEl"), null, "…with its state cleared");
});

test("a checked entry re-reads its function, so an open menu never shows a stale tick", () => {
  const ctx = uiCtx();
  ctx.run(`__flag = false;
           __entries = [{ label: "Toggle", checked: () => __flag, run: () => { __flag = !__flag; } }];
           openClientMenu(100, 100, __entries);`);
  eq(rows(ctx)[0].textContent, "  Toggle", "unticked");
  rows(ctx)[0].click();
  ctx.run("openClientMenu(100, 100, __entries);");
  eq(rows(ctx)[0].textContent, "✓ Toggle", "ticked on the next open");
});

test("clicking away closes the menu; clicking inside it does not", () => {
  const ctx = uiCtx();
  ctx.run(`openClientMenu(100, 100, [{ label: "Go", run: () => {} }]);`);
  ctx.fire("window", "mousedown", { target: menu(ctx) });
  ok(menu(ctx), "a press inside the menu leaves it up");
  ctx.fire("window", "mousedown", { target: ctx.document.body });
  eq(menu(ctx), null, "a press outside closes it");
});

test("Escape closes the menu and is stopped there — it must not also close a window behind it", () => {
  const ctx = uiCtx();
  ctx.run(`openClientMenu(100, 100, [{ label: "Go", run: () => {} }]);`);
  const ev = ctx.event("keydown", { code: "Escape" });
  ctx.sandbox.dispatchEvent(ev);
  eq(menu(ctx), null, "closed");
  eq(ev.propagationStopped, true, "…and the Escape chain in setupInput never sees it");
  eq(ev.defaultPrevented, true, "…nor the browser");
  // The dismiss listeners are removed with the menu, so a later Escape is free.
  const after = ctx.event("keydown", { code: "Escape" });
  ctx.sandbox.dispatchEvent(after);
  eq(after.propagationStopped, false, "with no menu up, Escape belongs to the game again");
});

test("the menu is clamped back on-screen when it would open off the right/bottom edge", () => {
  const ctx = uiCtx();               // the harness window is 1280x800
  ctx.run(`openClientMenu(5000, 5000, [{ label: "A", run: () => {} }, { label: "B", run: () => {} }]);`);
  eq(menu(ctx).style.left, (1280 - 220) + "px", "clamped to the right edge");
  eq(menu(ctx).style.top, (800 - (2 * 26 + 12)) + "px", "…and to the bottom, by the real row count");
  ctx.run(`openClientMenu(-500, -500, [{ label: "A", run: () => {} }]);`);
  eq(menu(ctx).style.left, "4px", "and off the top-left too");
  eq(menu(ctx).style.top, "4px", "…both axes");
});

// ── the party panel ────────────────────────────────────────────────────────

function party(ctx, { members, leader = 9, invite = 0, tracked = null, mobiles = null } = {}) {
  ctx.set("__scene", {
    player: { serial: 9, x: 100, y: 100, z: 0, name: "Me", equip: [] },
    mobiles: mobiles || [], items: [], statics: [],
    party: { leader, invite, members },
    tracked,
  });
  ctx.run("scene = __scene; partyOn = true; document.getElementById('party').classList.add('on');");
  ctx.run("wireParty(); refreshParty();");
  return ctx;
}
const ptRows = (ctx) => ctx.document.querySelectorAll("#pt-list .pt-row");
const M = (over) => Object.assign(
  { serial: 0x101, name: "Bob", hits: 12, hitsMax: 25, mana: 5, manaMax: 25, stam: 20, stamMax: 25 }, over);

test("a stranger's bars are a percentage — only your own row prints real hit points", () => {
  // ServUO's AttributeNormalizer caps another player's reported max at 25, so
  // "25/25" is meaningless to print; our own row carries the unnormalized status.
  const ctx = party(uiCtx(), { members: [M({ serial: 9, name: "Me", hits: 83, hitsMax: 100 }), M()] });
  eq(ptRows(ctx).length, 2, "two rows");
  eq(ptRows(ctx)[0].querySelector(".pt-hp").textContent, "83/100", "our own, in real numbers");
  eq(ptRows(ctx)[1].querySelector(".pt-hp").textContent, "48%", "a stranger's, as a ratio");
  eq(ptRows(ctx)[0].querySelector(".pt-bar i").style.width, "83%", "the bar agrees");
});

test("the leader wears the crown, and only a leader is offered the kick button", () => {
  const mine = party(uiCtx(), { leader: 9, members: [M({ serial: 9, name: "Me" }), M()] });
  ok(ptRows(mine)[0].querySelector(".pt-crown"), "we are the leader");
  ok(!ptRows(mine)[1].querySelector(".pt-crown"), "Bob is not");
  ok(ptRows(mine)[1].querySelector("button.kick"), "…and we may remove him");
  ok(!ptRows(mine)[0].querySelector("button.kick"), "but not ourselves");
  ok(!ptRows(mine)[0].querySelector('button[data-act="tell"]'), "and there is no ✉ to ourselves");

  // ClassicUO draws Kick on every row and lets the server ignore a non-leader's
  // press; a button that silently does nothing is worse than an absent one.
  const theirs = party(uiCtx(), { leader: 0x101, members: [M({ serial: 9, name: "Me" }), M()] });
  ok(!theirs.document.querySelector("#pt-list button.kick"), "a follower is offered no kick at all");
});

test("out-of-range dimming reads the server's 0xF0 list, NOT scene.mobiles", () => {
  // The scene deliberately disagrees with itself: Bob is still sitting in
  // scene.mobiles one tile away (ServUO never removed him when we walked off),
  // and 0xF0 says we cannot see him. 0xF0 wins.
  const stale = { serial: 0x101, x: 101, y: 100, z: 0, noto: 1, name: "Bob", body: 400 };
  const ctx = party(uiCtx(), {
    members: [M({ serial: 9, name: "Me" }), M()],
    mobiles: [stale],
    tracked: { on: 1, party: [{ serial: 0x101, x: 900, y: 900 }] },
  });
  eq(ptRows(ctx)[1].className.includes("pt-stale"), true,
     "listed by 0xF0 as un-seeable → dimmed, even though scene.mobiles still holds him");
  eq(ptRows(ctx)[0].className.includes("pt-stale"), false, "we are never stale to ourselves");

  // Absent from the 0xF0 list = the server can see them = the reading is live.
  const near = party(uiCtx(), {
    members: [M({ serial: 9, name: "Me" }), M()],
    mobiles: [stale],
    tracked: { on: 1, party: [] },
  });
  eq(ptRows(near)[1].className.includes("pt-stale"), false, "not in the 0xF0 list → live");
});

test("with no 0xF0 at all, distance is the fallback — and an unseen member is stale", () => {
  // A stale position that is ALREADY far away cannot belong to someone in range.
  const far = party(uiCtx(), {
    members: [M({ serial: 9, name: "Me" }), M()],
    mobiles: [{ serial: 0x101, x: 100 + 19, y: 100, z: 0, name: "Bob" }],
  });
  eq(ptRows(far)[1].className.includes("pt-stale"), true, "19 tiles out is past ServUO's update range");
  const close = party(uiCtx(), {
    members: [M({ serial: 9, name: "Me" }), M()],
    mobiles: [{ serial: 0x101, x: 100 + 18, y: 100, z: 0, name: "Bob" }],
  });
  eq(ptRows(close)[1].className.includes("pt-stale"), false, "18 is still inside it");
  const gone = party(uiCtx(), { members: [M({ serial: 9, name: "Me" }), M()], mobiles: [] });
  eq(ptRows(gone)[1].className.includes("pt-stale"), true, "not in the world at all → dimmed");
});

// KNOWN BUG, documented rather than fixed (this file does not change web/js).
//
// `pt-stale` is read from scene.tracked.party inside the render, but
// scene.tracked is NOT part of refreshParty's signature — so when the ONLY
// thing that changed is the 0xF0 tracking list, the guarded refresh returns
// early and the row does not dim. That is worst in exactly the case the
// dimming exists for: once a member is out of range their hits/mana/stam
// FREEZE, so their own half of the signature stops moving too. In a
// two-person party at full health, standing still, nothing changes and the
// honest "this reading is old" marker simply never appears.
//
// The assertion below pins the WRONG behaviour on purpose. If it starts
// failing, the bug was fixed — delete this test, and the fix belongs in
// refreshParty's `sig`.
test("BUG: the out-of-range dimming does not appear until some OTHER party field moves", () => {
  const ctx = party(uiCtx(), {
    members: [M({ serial: 9, name: "Me", hits: 100, hitsMax: 100 }), M()],
    mobiles: [{ serial: 0x101, x: 101, y: 100, z: 0, name: "Bob" }],
    tracked: { on: 1, party: [] },
  });
  eq(ptRows(ctx)[1].className.includes("pt-stale"), false, "in range to start with");
  ctx.run("scene.tracked.party = [{ serial: 0x101, x: 900, y: 900 }]; refreshParty();");
  eq(ptRows(ctx)[1].className.includes("pt-stale"), false,
     "BUG: 0xF0 now says we cannot see Bob, and the row is still undimmed");
  ctx.run("scene.party.members[0].mana = 49; refreshParty();");
  eq(ptRows(ctx)[1].className.includes("pt-stale"), true,
     "…it only lands once something already in the signature happens to move");
});

test("a member whose mana moved repaints — the signature covers every bar it draws", () => {
  // Found live: hashing only hits/name left a member whose mana changed frozen.
  const ctx = party(uiCtx(), { members: [M({ serial: 9, name: "Me" }), M({ mana: 5 })] });
  eq(ptRows(ctx)[1].querySelector(".pt-bar.mana i").style.width, "20%", "5 of 25");
  ctx.run("scene.party.members[1].mana = 25; refreshParty();");
  eq(ptRows(ctx)[1].querySelector(".pt-bar.mana i").style.width, "100%", "…and it followed the change");
  ctx.run("scene.party.members[1].stam = 5; refreshParty();");
  eq(ptRows(ctx)[1].querySelector(".pt-bar.stam i").style.width, "20%", "stamina too");
});

test("an incoming invite opens the panel by itself, and Accept/Decline answer the leader", () => {
  const ctx = uiCtx();
  ctx.run("wireParty();");
  ctx.set("__scene", { player: { serial: 9, x: 1, y: 1, name: "Me", equip: [] }, mobiles: [], items: [],
                       party: { leader: 0, invite: 0x0102, members: [] } });
  ctx.run("scene = __scene; partyOn = false; refreshParty();");
  eq(ctx.run("partyOn"), true, "the panel opened itself — an invite must never be missed");
  const prompt = ctx.document.getElementById("pt-invite-prompt");
  eq(prompt.classList.contains("on"), true, "the prompt is up");
  prompt.querySelector('[data-act="partyaccept"]').click();
  deepEq(ctx.sent, ["partyaccept:258"], "answered to the inviting leader's serial");
  ctx.sent.length = 0;
  prompt.querySelector('[data-act="partydecline"]').click();
  deepEq(ctx.sent, ["partydecline:258"], "…and Decline the same way");
});

test("the panel's verbs each send their own command", () => {
  const ctx = party(uiCtx(), { members: [M({ serial: 9, name: "Me" }), M()] });
  ctx.document.getElementById("pt-invite").click();
  ctx.document.getElementById("pt-leave").click();
  ctx.document.querySelector("#pt-list button.kick").click();
  deepEq(ctx.sent, ["partyinvite", "partyleave", "partykick:257"], "invite, leave, kick");
});

test("'party can loot my corpse' is our own copy of a value the server never reports", () => {
  // ClassicUO tracks it client-side too; persisting it is what makes it survive
  // a reload the way its PartyGump toggle does.
  const ctx = party(uiCtx(), { members: [M({ serial: 9, name: "Me" }), M()] });
  const box = ctx.document.getElementById("pt-loot");
  eq(box.checked, false, "off by default");
  box.checked = true;
  ctx.fire(box, "change", { bubbles: true });
  deepEq(ctx.sent, ["partyloot:1"], "0xBF/0x06/0x06");
  eq(ctx.localStorage.getItem("anima.partyLoot"), "1", "…and remembered across a reload");
});

test("the loot row is hidden while you are in no party", () => {
  const ctx = party(uiCtx(), { members: [] });
  eq(ctx.document.getElementById("pt-loot-row").style.display, "none", "nothing to loot-share with");
  eq(ctx.document.querySelector("#pt-list .pt-empty").textContent, "Not in a party.", "and it says so");
});

// ── the stat lockers ───────────────────────────────────────────────────────

const locks = (ctx) => ctx.document.querySelectorAll("#statusbar .st-lock");

test("each stat locker shows the state 0x11 reported", () => {
  const ctx = uiCtx();
  ctx.run("wireStatLocks(); statusOn = true;");
  ctx.set("__scene", { player: { serial: 9, name: "Me", hits: 1, hitsMax: 1, str: 90, dex: 80, int: 70,
                                 strLock: 0, dexLock: 1, intLock: 2, equip: [] } });
  ctx.run("scene = __scene; refreshStatus(scene);");
  deepEq([...locks(ctx)].map((e) => e.textContent), ["↑", "↓", "🔒"], "up / down / locked");
  deepEq([...locks(ctx)].map((e) => String(e.dataset.lock)), ["0", "1", "2"], "…and the state each click will cycle from");
  eq(locks(ctx)[2].classList.contains("locked"), true, "the locked one is marked");
  // A shard that sends a value outside 0..2 must still land on a real icon.
  ctx.run("scene.player.strLock = 4; refreshStatus(scene);");
  eq(locks(ctx)[0].textContent, "↓", "an out-of-range lock byte wraps (4 % 3) rather than drawing undefined");
});

test("clicking a locker cycles up → down → locked → up, ClassicUO's StatusGump order", () => {
  const ctx = uiCtx();
  ctx.run("wireStatLocks(); statusOn = true;");
  ctx.set("__scene", { player: { serial: 9, name: "Me", hits: 1, hitsMax: 1, str: 90, dex: 80, int: 70,
                                 strLock: 0, dexLock: 0, intLock: 0, equip: [] } });
  ctx.run("scene = __scene; refreshStatus(scene);");
  for (const [from, to] of [[0, 1], [1, 2], [2, 0]]) {
    ctx.sent.length = 0;
    ctx.run(`scene.player.dexLock = ${from}; refreshStatus(scene);`);
    locks(ctx)[1].click();
    deepEq(ctx.sent, [`statlock:1:${to}`], `DEX ${from} → ${to}`);
  }
  // The stat index comes off the row, so each locker addresses its own stat.
  ctx.sent.length = 0;
  locks(ctx)[0].click(); locks(ctx)[2].click();
  deepEq(ctx.sent, ["statlock:0:1", "statlock:2:1"], "STR is 0 and INT is 2");
});

test("a click on the status bar that is not on a locker sends nothing", () => {
  const ctx = uiCtx();
  ctx.run("wireStatLocks();");
  ctx.document.getElementById("st-name").click();
  deepEq(ctx.sent, [], "the delegated handler ignores everything else");
});
