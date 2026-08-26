// Clicking a mobile or a ground item: web/js/12-input.js `onEntityPointerDown`
// and the follow mode it can start.
//
// One function decides between eight different outcomes from the same physical
// press — resolve a target cursor, follow, attack, use, open a paperdoll, open a
// container, sit, or just ask the server for a name — and the ORDER of those
// branches is the part that regresses. ClassicUO puts Alt+click follow
// (GameSceneInputHandler.cs:717-728) *after* the target cursor for a reason: a
// pending cursor owns the click. Every one of those orderings is pinned here.
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
  ctx.run(`
    app = { canvas: { style: {}, clientWidth: 800, clientHeight: 600,
                      getBoundingClientRect: () => ({ left: 0, top: 0, width: 800, height: 600 }) },
            renderer: { width: 800, height: 600, events: { cursorStyles: {} } },
            stage: { position: { x: 0, y: 0 } } };
    world = new PIXI.Container();
  `);
  return ctx;
}

// A world with us at (5,5), one other mobile, and whatever items the test wants.
function world(ctx, { mobiles, items, war = false, target = null } = {}) {
  ctx.set("__scene", {
    player: { serial: 9, x: 5, y: 5, z: 0, dir: 2, noto: 1, name: "Me", body: 400, equip: [] },
    mobiles: mobiles || [{ serial: 0x101, x: 6, y: 5, z: 0, noto: 1, name: "Bob", body: 400 }],
    items: items || [], statics: [], war,
    target: target || { active: 0, flag: 0 },
  });
  // poll() calls updateTargetUI() on every scene it lands, so the 0->1 edge
  // detector has always seen the request before any click can reach it.
  ctx.run("scene = __scene; updateTargetUI();");
  return ctx;
}
// A pointer event the way PIXI hands one to the entity handler.
const ev = (over = {}) => Object.assign(
  { button: 0, clientX: 700, clientY: 120, altKey: false, shiftKey: false,
    stopPropagation() {} }, over);
const down = (ctx, serial, isItem, over) => {
  ctx.set("__ev", ev(over));
  ctx.run(`onEntityPointerDown(${serial}, __ev, ${!!isItem})`);
};

// ── single click vs double click ───────────────────────────────────────────

test("a single click waits out the double-click window, then asks the server for the name", () => {
  // ClassicUO's DBLCLICK window; the click is not sent until it can no longer
  // become a double-click.
  const ctx = world(uiCtx());
  ctx.setNow(1000);
  down(ctx, 0x101, false);
  deepEq(ctx.sent, [], "nothing on the wire during the discrimination window");
  ctx.advance(249);
  deepEq(ctx.sent, [], "…still nothing at 249ms");
  ctx.advance(2);
  deepEq(ctx.sent, ["click:257"], "at 250ms it becomes a single click");
});

test("a second click inside the window is a double click — use, not click", () => {
  const ctx = world(uiCtx());
  ctx.setNow(1000);
  down(ctx, 0x101, false);
  ctx.setNow(1100);
  down(ctx, 0x101, false);
  deepEq(ctx.sent, ["use:257"], "one `use`, and the pending single click was cancelled");
  ctx.advance(1000);
  deepEq(ctx.sent, ["use:257"], "the cancelled timer never fires");
});

test("two clicks on DIFFERENT entities are two single clicks, not a double", () => {
  const ctx = world(uiCtx(), {
    mobiles: [{ serial: 0x101, x: 6, y: 5, noto: 1, name: "Bob", body: 400 },
              { serial: 0x102, x: 7, y: 5, noto: 1, name: "Ann", body: 400 }],
  });
  ctx.setNow(1000);
  down(ctx, 0x101, false);
  down(ctx, 0x102, false);
  ctx.advance(1000);
  deepEq(ctx.sent, ["click:258"], "the first click was dropped for the second, ClassicUO-style");
});

test("war mode turns a double-click on someone else into an attack", () => {
  const ctx = world(uiCtx(), { war: true });
  ctx.setNow(1000); down(ctx, 0x101, false);
  ctx.setNow(1050); down(ctx, 0x101, false);
  deepEq(ctx.sent, ["attack:257"], "attack instead of use — no paperdoll mid-fight");
});

test("…but never on YOURSELF: double-clicking your own body in war is still `use`", () => {
  const ctx = world(uiCtx(), { war: true });
  ctx.setNow(1000); down(ctx, 9, false);
  ctx.setNow(1050); down(ctx, 9, false);
  deepEq(ctx.sent, ["use:9"], "you cannot attack yourself");
});

test("only a real container opens a window; a door or a potion just gets used", () => {
  const cases = [
    ["a chest (c:1)", { serial: 0x201, g: 0x0E7C, c: 1 }, true],
    ["a door", { serial: 0x201, g: 0x0675 }, false],
    ["a potion", { serial: 0x201, g: 0x0F0E, amount: 1 }, false],
  ];
  for (const [why, item, opens] of cases) {
    const ctx = world(uiCtx(), { items: [Object.assign({ x: 5, y: 5, z: 0 }, item)] });
    ctx.setNow(1000); down(ctx, 0x201, true);
    ctx.setNow(1050); down(ctx, 0x201, true);
    deepEq(ctx.sent, ["use:513"], `${why}: one use`);
    eq(!!ctx.run(`dialogWindow("containers", 0x201)`), opens, `${why}: window ${opens ? "opens" : "does not open"}`);
  }
});

test("a hand-opened corpse is remembered, so SkipEmptyCorpse never hides it", () => {
  // ClassicUO's `ManualOpenedCorpses` — you asked to see it.
  const ctx = world(uiCtx(), { items: [{ serial: 0x301, g: 0x2006, c: 1, x: 5, y: 5, z: 0 }] });
  eq(ctx.run("manualOpenedCorpses.has(0x301)"), false, "not remembered before the click");
  ctx.setNow(1000); down(ctx, 0x301, true);
  ctx.setNow(1050); down(ctx, 0x301, true);
  eq(ctx.run("manualOpenedCorpses.has(0x301)"), true, "…and remembered after it");
});

test("a paperdoll opens only for a humanoid body (UO's 400-407)", () => {
  for (const [body, opens] of [[400, true], [401, true], [407, true], [408, false], [200, false]]) {
    const ctx = world(uiCtx(), { mobiles: [{ serial: 0x101, x: 6, y: 5, noto: 1, name: "Bob", body }] });
    ctx.setNow(1000); down(ctx, 0x101, false);
    ctx.setNow(1050); down(ctx, 0x101, false);
    eq(ctx.run("pdTarget"), opens ? 0x101 : null, `body ${body}: paperdoll ${opens ? "opens" : "stays shut"}`);
    eq(ctx.document.getElementById("paperdoll").classList.contains("on"), opens, `body ${body}: panel state`);
  }
});

test("the right button never single-clicks, double-clicks or uses anything", () => {
  const ctx = world(uiCtx());
  ctx.setNow(1000);
  down(ctx, 0x101, false, { button: 2 });
  ctx.advance(1000);
  deepEq(ctx.sent, [], "no click:, no use: — the right button is the menu/steer button");
  eq(ctx.run("clickPend"), null, "and no pending single click was armed");
});

// ── the target cursor owns the click ───────────────────────────────────────

test("with a cursor up, a plain click answers it instead of clicking the entity", () => {
  const ctx = world(uiCtx(), { target: { active: 1, flag: 0 } });
  ctx.setNow(1000);
  down(ctx, 0x101, false);
  deepEq(ctx.sent, ["target:257"], "the cursor is answered immediately, no 250ms wait");
  ctx.advance(1000);
  deepEq(ctx.sent, ["target:257"], "…and no single click follows it");
  eq(ctx.run("targetingActive()"), false, "the crosshair is gone");
  ok(ctx.run("targetConsumedAt") > 0, "the ground handler is told this click is spent");
});

test("Alt+click follows a mobile — but NOT while the server is waiting for a target", () => {
  // ClassicUO GameSceneInputHandler.cs:717-728. Alt is only a movement modifier
  // when nothing else wants the click, which is why this branch sits BELOW the
  // target-cursor one.
  const free = world(uiCtx());
  down(free, 0x101, false, { altKey: true });
  eq(free.run("followTarget"), 0x101, "following Bob");
  ok(free.run("localJournal.some((l) => /Now following/.test(l.text))"), "ClassicUO's NowFollowing line");
  free.advance(1000);
  deepEq(free.sent, [], "following is client-side — no protocol verb, and no name request");

  const aiming = world(uiCtx(), { target: { active: 1, flag: 0 } });
  down(aiming, 0x101, false, { altKey: true });
  eq(aiming.run("followTarget"), 0, "the pending cursor keeps the click");
  deepEq(aiming.sent, ["target:257"], "…and it is answered");
});

test("you cannot follow an item or yourself (ClassicUO's `ent is Mobile`)", () => {
  const item = world(uiCtx(), { items: [{ serial: 0x201, g: 0x0E7C, x: 6, y: 5, z: 0 }] });
  down(item, 0x201, true, { altKey: true });
  eq(item.run("followTarget"), 0, "an item is not followable");
  const me = world(uiCtx());
  me.run("startFollowing(9)");
  eq(me.run("followTarget"), 0, "and neither are you");
  const ghost = world(uiCtx());
  ghost.run("startFollowing(0x999)");
  eq(ghost.run("followTarget"), 0, "nor a serial that is not a mobile in view");
});

// ── follow mode's own clock (ClassicUO GameScene.cs:736-759) ───────────────

// Move the followed mobile and run one frame of follow.
function followFrame(ctx, now, x, y) {
  ctx.setNow(now);
  if (x != null) ctx.run(`scene.mobiles[0].x = ${x}; scene.mobiles[0].y = ${y};`);
  ctx.run("followTick()");
}

test("follow only re-paths while the distance is over 3 — UO's Chebyshev distance", () => {
  const ctx = world(uiCtx());
  ctx.run("followTarget = 0x101; followSentAt = 0; followSentTile = null;");
  followFrame(ctx, 1000, 8, 8);          // dx 3, dy 3 → Chebyshev 3
  deepEq(ctx.sent, [], "a diagonal three tiles away is still 'distance 3' — close enough");
  followFrame(ctx, 2000, 9, 5);          // dx 4 → 4
  deepEq(ctx.sent, ["walkto:9,5"], "four tiles out, walk to them");
});

test("a route is issued once per walk cadence, and refreshed if the goal has not moved", () => {
  // A new walkto resets the server's denied-tile blacklist, so re-issuing faster
  // than it can step is wasteful; but a route the server quietly abandoned would
  // strand us, hence the 2s refresh.
  const ctx = world(uiCtx());
  ctx.run("followTarget = 0x101; followSentAt = 0; followSentTile = null;");
  followFrame(ctx, 1000, 12, 5);
  deepEq(ctx.sent, ["walkto:12,5"], "first route");
  followFrame(ctx, 1200, 12, 5);
  deepEq(ctx.sent, ["walkto:12,5"], "throttled inside 400ms");
  followFrame(ctx, 1500, 12, 5);
  deepEq(ctx.sent, ["walkto:12,5"], "same tile, and not yet 2s — still nothing");
  followFrame(ctx, 3100, 12, 5);
  deepEq(ctx.sent, ["walkto:12,5", "walkto:12,5"], "2s on, the same goal is re-issued");
  followFrame(ctx, 3600, 13, 5);
  deepEq(ctx.sent, ["walkto:12,5", "walkto:12,5", "walkto:13,5"], "a moved goal re-paths on the next cadence");
});

test("the followed mobile leaving view stops the follow and cancels our live route", () => {
  const ctx = world(uiCtx());
  ctx.run("followTarget = 0x101; followSentAt = 0; followSentTile = null;");
  followFrame(ctx, 1000, 12, 5);
  ctx.run("scene.mobiles = [];");
  followFrame(ctx, 2000);
  eq(ctx.run("followTarget"), 0, "follow ended");
  // The play server has no "cancel route" verb, so re-aiming at our own tile is
  // what makes prepare_walkto resolve to an empty path and stop.
  eq(ctx.sent[ctx.sent.length - 1], "walkto:5,5", "the route is aimed at the tile we already stand on");
  ok(ctx.run("localJournal.some((l) => /Stopped following/.test(l.text))"), "ClassicUO's StoppedFollowing line");
});

test("stopping with no route of ours live sends nothing", () => {
  const ctx = world(uiCtx());
  ctx.run("startFollowing(0x101); stopFollowing();");
  deepEq(ctx.sent, [], "no walkto had gone out, so there is nothing to cancel");
  eq(ctx.run("followTarget"), 0, "still stopped");
});

// ── the right button: context menu vs steering ─────────────────────────────

test("a quick right-tap on an entity opens its context menu and never steers", () => {
  const ctx = uiCtx();
  world(ctx);
  ctx.setNow(1000);
  down(ctx, 0x101, false, { button: 2, clientX: 640, clientY: 220 });
  eq(ctx.run("rmbEntity && rmbEntity.serial"), 0x101, "the press is pending a tap/hold decision");
  eq(ctx.run("rightDown"), false, "not steering yet");
  deepEq([ctx.run("lastMenuX"), ctx.run("lastMenuY")], [640, 220], "the menu is anchored at the cursor");
  ctx.setNow(1100);                                  // released inside STEER_HOLD_MS
  ctx.run("endRightMouse(640, 220)");
  deepEq(ctx.sent, ["popupreq:257"], "0xBF/0x13 popup request");
  eq(ctx.run("rmbEntity"), null, "the pending press is spent");
});

test("holding the right button past 180ms steers instead, and the release opens no menu", () => {
  const ctx = uiCtx();
  world(ctx);
  ctx.setNow(1000);
  down(ctx, 0x101, false, { button: 2, clientX: 640, clientY: 220 });
  ctx.advance(200);                                  // the STEER_HOLD_MS timer fires
  eq(ctx.run("rmbEntity && rmbEntity.steering"), true, "promoted to steering");
  eq(ctx.run("rightDown"), true, "…and the character is now walking toward the cursor");
  ctx.run("endRightMouse(640, 220)");
  deepEq(ctx.sent, [], "a hold was a move, not a tap — no context menu");
  eq(ctx.run("rightDown"), false, "the release stops the steer");
});

test("a right-release that never touched the world leaves follow mode alone", () => {
  // ClassicUO ends follow on a right-click, but that arm sits after
  // `if (!UIManager.IsMouseOverWorld) return false` — a right-click that closed
  // a gump must not also cancel the follow.
  const ctx = world(uiCtx());
  ctx.run("startFollowing(0x101);");
  ctx.run("rightDown = false; rmbEntity = null; endRightMouse(10, 10);");
  eq(ctx.run("followTarget"), 0x101, "still following after a right-click on a DOM panel");
  ctx.run("rightDown = true; endRightMouse(10, 10);");
  eq(ctx.run("followTarget"), 0, "a right-click that started on the world does end it");
});

test("a lost keyup clears every movement latch instead of walking off north forever", () => {
  const ctx = world(uiCtx());
  ctx.setNow(1000);
  ctx.run("held.add(0); shiftHeld = true; wasMoving = true;");
  down(ctx, 0x101, false, { button: 2 });
  eq(ctx.run("rmbEntity !== null"), true, "an entity right-press is pending");
  ctx.run("releaseMoveKeys()");
  eq(ctx.run("held.size"), 0, "held keys dropped");
  eq(ctx.run("rightDown"), false, "right-button steering dropped");
  eq(ctx.run("shiftHeld"), false, "the run modifier dropped");
  eq(ctx.run("rmbEntity"), null, "the pending context-menu press dropped");
  deepEq(ctx.sent, ["stop"], "and one immediate stop, not a wait for the next 120ms tick");
  ctx.run("releaseMoveKeys()");
  deepEq(ctx.sent, ["stop"], "a second release sends no second stop");
});
