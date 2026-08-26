// The item-on-the-cursor layer: web/js/11-dragdrop.js.
//
// UO's move is two packets — 0x07 pickup, then 0x08 drop — with the item stuck
// to the mouse in between, and every source (a world sprite, a container cell, a
// paperdoll icon, an item's name plate) funnels through the same `groundDrag` →
// `liftToCursor` → `placeCursorItem` chain. What breaks here is never the happy
// path; it is the gestures that must NOT fire, and the flags left set afterwards:
//
//   • a double-click drifts a few pixels under the finger and must not lift;
//   • the flag that says "the lifting press is still down" must be cleared, or
//     it swallows the next click and the item can never be put down.
//
// Both were real regressions. Both are pinned below.
const { newContext } = require("./harness.js");
const { test, ok, eq, deepEq } = require("./run.js");

function dndCtx() {
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
  // The real page installs the window-level pointer listeners from boot(); a
  // test loads the CODE, not the page, so it installs them itself.
  ctx.run("setupItemDnD()");
  // There is no layout engine here, so nothing can hit-test itself: the test
  // says what is under the cursor, using the rects it set. Topmost = last in
  // document order, which is how the client stacks its windows (bringToFront is
  // an appendChild).
  ctx.document.elementFromPoint = (x, y) => {
    let hit = null;
    const walk = (el) => {
      if (el.rect) {
        const r = el.getBoundingClientRect();
        if (x >= r.left && x < r.right && y >= r.top && y < r.bottom) hit = el;
      }
      for (const c of el.children || []) walk(c);
    };
    walk(ctx.document.body);
    return hit;
  };
  return ctx;
}

function world(ctx, { items = [], contItems = [], equip = [], target = null } = {}) {
  ctx.set("__scene", {
    player: { serial: 9, x: 5, y: 5, z: 0, dir: 2, noto: 1, name: "Me", body: 400, equip },
    mobiles: [], items, contItems, statics: [],
    target: target || { active: 0, flag: 0 },
  });
  ctx.run("scene = __scene; updateTargetUI();");
  return ctx;
}
const BACKPACK = [{ serial: 0x500, layer: 21, g: 0x0E75 }];
const pointer = (ctx, type, init) =>
  ctx.fire("window", type, Object.assign({ button: 0, shiftKey: false, bubbles: true }, init));
// Arm a world-item drag exactly as 12-input.js's left-press on a sprite does.
const armWorld = (ctx, it, at = { x: 100, y: 100 }) => {
  ctx.set("__it", it);
  ctx.setNow(1000);
  ctx.run(`groundDrag = { serial: __it.serial >>> 0, g: __it.g | 0, amount: (__it.amount | 0) || 1,
                          st: !!__it.st, hue: __it.hue | 0,
                          sx: ${at.x}, sy: ${at.y}, started: false, t: performance.now() };`);
};
const held = (ctx) => ctx.run("cursorItem");

// ── promoting a press into a lift: the gestures that must NOT fire ─────────

test("a press that never moves is left alone as a click", () => {
  const ctx = world(dndCtx(), { items: [{ serial: 0x201, g: 0x0F3F, amount: 1, x: 5, y: 5 }] });
  armWorld(ctx, { serial: 0x201, g: 0x0F3F, amount: 1 });
  pointer(ctx, "pointerup", { clientX: 100, clientY: 100 });
  eq(held(ctx), null, "nothing lifted");
  eq(ctx.run("groundDrag"), null, "the arm is disposed of");
  deepEq(ctx.sent, [], "no 0x07 pickup on the wire");
});

test("a double-click's few pixels of drift never lift a world item", () => {
  // The old rule was 5px of motion, full stop — which turned every trackpad tap
  // into a pickup. A small drift is a drag only once the button has been held
  // past the double-click window.
  const ctx = world(dndCtx(), { items: [{ serial: 0x201, g: 0x0F3F, amount: 1, x: 5, y: 5 }] });
  armWorld(ctx, { serial: 0x201, g: 0x0F3F, amount: 1 });
  ctx.setNow(1040);                                  // 40ms in — a tap
  pointer(ctx, "pointermove", { clientX: 108, clientY: 103 });   // 8px: over DRAG_THRESHOLD
  eq(held(ctx), null, "8px inside the tap window is still a click");
  ctx.setNow(1249);
  pointer(ctx, "pointermove", { clientX: 110, clientY: 100 });
  eq(held(ctx), null, "…still, at 249ms");
  ctx.setNow(1251);
  pointer(ctx, "pointermove", { clientX: 110, clientY: 100 });
  ok(held(ctx), "past DRAG_HOLD_MS the same small drift IS a drag");
});

test("a big motion is unambiguous and lifts at once, however fast", () => {
  const ctx = world(dndCtx(), { items: [{ serial: 0x201, g: 0x0F3F, amount: 1, x: 5, y: 5 }] });
  armWorld(ctx, { serial: 0x201, g: 0x0F3F, amount: 1 });
  ctx.setNow(1005);                                  // 5ms — nowhere near a hold
  pointer(ctx, "pointermove", { clientX: 100 + 22, clientY: 100 });
  ok(held(ctx), "DRAG_FAR px lifts immediately");
  deepEq(ctx.sent, ["pickup:513"], "one 0x07, with no amount for a single item");
});

test("a motion under the threshold is not a drag at all, no matter how long you hold", () => {
  const ctx = world(dndCtx(), { items: [{ serial: 0x201, g: 0x0F3F, amount: 1, x: 5, y: 5 }] });
  armWorld(ctx, { serial: 0x201, g: 0x0F3F, amount: 1 });
  ctx.setNow(9000);
  pointer(ctx, "pointermove", { clientX: 105, clientY: 100 });   // 5px < DRAG_THRESHOLD
  eq(held(ctx), null, "5px is never a drag");
});

test("a container cell promotes only by LEAVING the cell — not by drift or hold", () => {
  // The hold/distance heuristic is dropped entirely for an arm that has a real
  // on-screen cell: a double-click's drift, however long held, never leaves a
  // ~40px cell; a real drag-out does almost immediately.
  const ctx = world(dndCtx(), { contItems: [{ serial: 0x201, cont: 0x500, g: 0x0F3F, amount: 1 }] });
  ctx.setNow(1000);
  ctx.run(`groundDrag = { serial: 0x201, g: 0x0F3F, amount: 1, st: false, hue: 0,
                          sx: 60, sy: 60, started: false, t: performance.now(),
                          rect: { left: 40, top: 40, right: 84, bottom: 84 } };`);
  ctx.setNow(30000);                                  // held for thirty seconds
  pointer(ctx, "pointermove", { clientX: 83, clientY: 83 });     // still inside the cell
  eq(held(ctx), null, "inside the cell, nothing lifts — this is what stops double-click-lifts");
  pointer(ctx, "pointermove", { clientX: 84, clientY: 60 });     // right edge is exclusive
  ok(held(ctx), "one pixel out of the cell and it lifts");
});

test("an item's NAME PLATE arms the very same groundDrag, cell rule and all", () => {
  // web/js/08-overlays.js `wirePlate` deliberately does not lift by itself; it
  // arms this, so the split dialog / locked refusal / place-on-release all come
  // along instead of being a second implementation.
  const ctx = world(dndCtx(), { items: [{ serial: 0x201, g: 0x0F3F, amount: 1, st: 1, hue: 7, x: 5, y: 5 }] });
  const plate = ctx.document.createElement("div");
  plate.rect = { left: 200, top: 40, width: 60, height: 14 };
  ctx.document.body.appendChild(plate);
  ctx.set("__plate", plate);
  ctx.setNow(1000);
  ctx.run("wirePlate(__plate, 0x201, false)");
  ctx.fire(plate, "mousedown", { button: 0, clientX: 210, clientY: 45, bubbles: true });
  const arm = ctx.run("groundDrag");
  eq(arm.serial, 0x201, "armed for the plate's item");
  eq(arm.hue, 7, "carrying the item's hue into the eventual ghost");
  ok(arm.rect, "with a rect — a plate is a real element, so it gets the cell rule");
  ctx.setNow(60000);
  pointer(ctx, "pointermove", { clientX: 240, clientY: 46 });    // still over the plate
  eq(held(ctx), null, "a long hold inside the plate is not a drag");
  pointer(ctx, "pointermove", { clientX: 300, clientY: 46 });
  ok(held(ctx), "dragging off the plate picks the item up");
  deepEq(ctx.sent, ["pickup:513"], "…through the ordinary pickup path");
});

test("a plate press is ignored while something is already on the cursor", () => {
  const ctx = world(dndCtx(), { items: [{ serial: 0x201, g: 0x0F3F, amount: 1, x: 5, y: 5 }] });
  ctx.run("cursorItem = { serial: 0x999, g: 1, amount: 1, hue: 0 };");
  const plate = ctx.document.createElement("div");
  plate.rect = { left: 200, top: 40, width: 60, height: 14 };
  ctx.document.body.appendChild(plate);
  ctx.set("__plate", plate);
  ctx.run("wirePlate(__plate, 0x201, false)");
  ctx.fire(plate, "mousedown", { button: 0, clientX: 210, clientY: 45, bubbles: true });
  eq(ctx.run("groundDrag"), null, "you cannot pick up a second item");
});

// ── the stack-split dialog (ClassicUO SplitMenuGump) ───────────────────────

test("dragging a stack opens the split dialog and sends NOTHING until it is confirmed", () => {
  const ctx = world(dndCtx(), { items: [{ serial: 0x201, g: 0x0EED, amount: 200, st: 1, x: 5, y: 5 }] });
  armWorld(ctx, { serial: 0x201, g: 0x0EED, amount: 200, st: 1 });
  ctx.setNow(1005);
  pointer(ctx, "pointermove", { clientX: 140, clientY: 100 });
  eq(held(ctx), null, "nothing is on the cursor yet");
  deepEq(ctx.sent, [], "and no pickup has gone out — the stack is untouched");
  ok(ctx.document.querySelector(".split-win"), "the split dialog is up");
  eq(ctx.run("splitWin.amount"), 200, "…offering the whole pile as the maximum");
  ctx.run("splitWin.input.value = '37'; confirmSplitDialog();");
  deepEq(ctx.sent, ["pickup:513:37"], "only now, and for the chosen amount");
  eq(held(ctx).amount, 37, "the cursor holds 37");
  eq(ctx.document.querySelector(".split-win"), null, "the dialog closed");
});

test("Cancel abandons the drag and leaves the stack alone", () => {
  const ctx = world(dndCtx(), { items: [{ serial: 0x201, g: 0x0EED, amount: 200, st: 1, x: 5, y: 5 }] });
  armWorld(ctx, { serial: 0x201, g: 0x0EED, amount: 200, st: 1 });
  ctx.setNow(1005);
  pointer(ctx, "pointermove", { clientX: 140, clientY: 100 });
  ctx.document.querySelector(".split-cancel").click();
  deepEq(ctx.sent, [], "nothing was ever sent for this press");
  eq(held(ctx), null, "nothing held");
  eq(ctx.document.querySelector(".split-win"), null, "gone");
});

test("the split amount is clamped to 1..amount however it is typed", () => {
  const ctx = world(dndCtx(), { items: [{ serial: 0x201, g: 0x0EED, amount: 12, st: 1, x: 5, y: 5 }] });
  // liftToCursor omits the `:amount` suffix for a lift of one, so "1" is
  // spelled `pickup:513` — the same bytes a single item would send.
  for (const [typed, want] of [["0", ""], ["-4", ""], ["99", ":12"], ["", ""], ["abc", ""], ["7", ":7"]]) {
    ctx.run("clearCursorItem(); closeSplitDialog();");
    ctx.sent.length = 0;
    armWorld(ctx, { serial: 0x201, g: 0x0EED, amount: 12, st: 1 });
    ctx.setNow(1005);
    pointer(ctx, "pointermove", { clientX: 140, clientY: 100 });
    ctx.run(`splitWin.input.value = ${JSON.stringify(typed)}; confirmSplitDialog();`);
    deepEq(ctx.sent, [`pickup:513${want}`], `"${typed}" → ${want || ":1 (spelled bare)"}`);
  }
});

test("Shift skips the dialog, and so does anything that is not a stackable stack", () => {
  // ClassicUO GameActions.PickUp opens the gump only when
  // `HoldShiftToSplitStack == Keyboard.Shift`, and the profile default is false
  // — so the gump shows exactly when Shift is UP. It also needs
  // `item.ItemData.IsStackable`, which is what `st` carries.
  const cases = [
    ["Shift held over a stack", { serial: 0x201, g: 0x0EED, amount: 200, st: 1 }, { shiftKey: true }, "pickup:513:200"],
    ["a non-stackable 'stack'", { serial: 0x201, g: 0x1F03, amount: 3 }, {}, "pickup:513:3"],
    ["a single item", { serial: 0x201, g: 0x0F3F, amount: 1, st: 1 }, {}, "pickup:513"],
  ];
  for (const [why, it, mods, want] of cases) {
    const ctx = world(dndCtx(), { items: [Object.assign({ x: 5, y: 5 }, it)] });
    armWorld(ctx, it);
    ctx.setNow(1005);
    pointer(ctx, "pointermove", Object.assign({ clientX: 140, clientY: 100 }, mods));
    eq(ctx.document.querySelector(".split-win"), null, `${why}: no dialog`);
    deepEq(ctx.sent, [want], `${why}: lifted straight away`);
  }
});

test("the split dialog reads the LIVE shift key, not the mirrored one", () => {
  // The dialog's own keydown handler stopPropagation()s Shift while it has
  // focus, which leaves the mirrored `shiftHeld` stale.
  const ctx = world(dndCtx(), { items: [{ serial: 0x201, g: 0x0EED, amount: 200, st: 1, x: 5, y: 5 }] });
  ctx.run("shiftHeld = true;");                       // stale
  armWorld(ctx, { serial: 0x201, g: 0x0EED, amount: 200, st: 1 });
  ctx.setNow(1005);
  pointer(ctx, "pointermove", { clientX: 140, clientY: 100, shiftKey: false });
  ok(ctx.document.querySelector(".split-win"), "a stale shiftHeld did not skip the dialog");
});

test("a mousedown outside the split dialog abandons it", () => {
  const ctx = world(dndCtx(), { items: [{ serial: 0x201, g: 0x0EED, amount: 200, st: 1, x: 5, y: 5 }] });
  armWorld(ctx, { serial: 0x201, g: 0x0EED, amount: 200, st: 1 });
  ctx.setNow(1005);
  pointer(ctx, "pointermove", { clientX: 140, clientY: 100 });
  ok(ctx.run("splitWin"), "dialog up");
  // The abandon listener is on `window`, in the capture phase.
  ctx.fire("window", "mousedown", { button: 0, target: ctx.document.body });
  eq(ctx.run("splitWin"), null, "clicking away abandons it");
  deepEq(ctx.sent, [], "nothing was ever sent for this press");
});

// ── the locked item ClassicUO refuses to even try ──────────────────────────

test("a locked-down world item refuses the lift instead of asking and being rejected", () => {
  // ClassicUO GameActions.cs:457 refuses the drag rather than sending a 0x07
  // whose only possible answer is a 0x27 reject — the refusal is what makes a
  // forge feel fixed instead of broken.
  const ctx = world(dndCtx(), { items: [{ serial: 0x201, g: 0x0FB1, amount: 1, lk: 1, x: 5, y: 5 }] });
  armWorld(ctx, { serial: 0x201, g: 0x0FB1, amount: 1 });
  ctx.setNow(1005);
  pointer(ctx, "pointermove", { clientX: 140, clientY: 100 });
  deepEq(ctx.sent, [], "no pickup on the wire");
  eq(held(ctx), null, "nothing on the cursor");
  ok(ctx.run("localJournal.some((l) => /locked down/i.test(l.text))"), "and the player is told why");
  eq(ctx.document.querySelector("img[style*='z-index:100000']"), null, "no ghost either");
});

test("the same item unlocked lifts normally", () => {
  const ctx = world(dndCtx(), { items: [{ serial: 0x201, g: 0x0FB1, amount: 1, x: 5, y: 5 }] });
  armWorld(ctx, { serial: 0x201, g: 0x0FB1, amount: 1 });
  ctx.setNow(1005);
  pointer(ctx, "pointermove", { clientX: 140, clientY: 100 });
  deepEq(ctx.sent, ["pickup:513"], "lifted");
});

// ── where a release puts it (placeCursorItem) ──────────────────────────────

// Give the page one drop target with a known screen box.
function dropTarget(ctx, html, rects) {
  ctx.mount(html);
  for (const [sel, rect] of rects) {
    const el = ctx.document.querySelector(sel);
    if (!el) throw new Error(`dropTarget: nothing matches ${sel}`);
    el.rect = rect;
  }
  return ctx.document;
}
// Lift an item and release the same press at (x, y) — the one-motion drag.
function liftAndRelease(ctx, x, y, it = { serial: 0x201, g: 0x0F3F, amount: 1 }) {
  armWorld(ctx, it);
  ctx.setNow(1005);
  pointer(ctx, "pointermove", { clientX: 122, clientY: 100 });
  ctx.sent.length = 0;                                // drop the pickup; assert the placement
  pointer(ctx, "pointerup", { clientX: x, clientY: y });
}

test("a one-motion drag onto the ground drops at the tile under the cursor", () => {
  const ctx = world(dndCtx(), { items: [{ serial: 0x201, g: 0x0F3F, amount: 1, x: 5, y: 5 }] });
  dropTarget(ctx, '<div id="map"></div>', [["#map", { left: 0, top: 0, width: 800, height: 600 }]]);
  // Release exactly over where tile (12,7) is DRAWN, so the drop's coordinates
  // are the client's own iso projection inverted — a round trip, not a constant.
  const at = { x: ctx.run("isoX(12, 7)"), y: ctx.run("isoY(12, 7, 0)") };
  liftAndRelease(ctx, at.x, at.y);
  eq(ctx.sent.length, 1, "one drop command");
  eq(ctx.sent[0], "drop:513:12:7:0:4294967295",
     "the tile under the cursor, and container 0xFFFFFFFF = 'the ground'");
  eq(held(ctx), null, "the cursor is empty again");
  eq(ctx.run("liftDrag"), false, "…and the lifting-press flag is cleared");
});

test("released over the paperdoll it equips, and the server derives the layer", () => {
  const ctx = world(dndCtx(), { items: [{ serial: 0x201, g: 0x13BB, amount: 1, x: 5, y: 5 }] });
  dropTarget(ctx, '<div id="paperdoll"><div class="pd-body"></div></div>',
             [["#paperdoll", { left: 900, top: 100, width: 200, height: 400 }],
              [".pd-body", { left: 910, top: 110, width: 180, height: 380 }]]);
  liftAndRelease(ctx, 1000, 300);
  deepEq(ctx.sent, ["equip:513:0"], "layer 0 = 'you work it out'");
});

test("released over a container window it drops at the gump's own pixel space", () => {
  const ctx = world(dndCtx(), { items: [{ serial: 0x201, g: 0x0F3F, amount: 1, x: 5, y: 5 }] });
  ctx.run("openContainer(0x777)");
  const win = ctx.run(`dialogWindow("containers", 0x777).el`);
  const body = ctx.run(`dialogWindow("containers", 0x777).body`);
  win.rect = { left: 200, top: 100, width: 260, height: 220 };
  body.rect = { left: 204, top: 120, width: 252, height: 196 };
  liftAndRelease(ctx, 254, 170);
  // Measured against the BODY (the coordinates ServUO stores), not the window
  // minus a hardcoded title bar.
  deepEq(ctx.sent, ["drop:513:50:50:0:1911"], "50,50 inside the bag, container 0x777");
});

test("a drop past the container's far edge is clamped so the icon still lands inside", () => {
  const ctx = world(dndCtx(), { items: [{ serial: 0x201, g: 0x0F3F, amount: 1, x: 5, y: 5 }] });
  ctx.run("openContainer(0x777)");
  const win = ctx.run(`dialogWindow("containers", 0x777).el`);
  const body = ctx.run(`dialogWindow("containers", 0x777).body`);
  win.rect = { left: 200, top: 100, width: 260, height: 220 };
  body.rect = { left: 200, top: 120, width: 252, height: 196 };
  liftAndRelease(ctx, 449, 313);
  // 252 - 44 = 208 wide, 196 - 44 = 152 tall (ITEM_ICON_PX).
  deepEq(ctx.sent, ["drop:513:208:152:0:1911"], "clamped to the body minus one icon");
});

test("released over a MOBILE it is a drop-on-mobile, not a drop at their feet", () => {
  // ServUO's Mobile.OnDragDrop then decides: AddToBackpack on ourself,
  // OpenTrade on someone else. x=y=0xFFFF is the "no tile" sentinel, and the
  // container is the mobile's own serial. Checked before the plain ground drop
  // so standing next to someone does not just litter the floor.
  const ctx = world(dndCtx(), { items: [{ serial: 0x201, g: 0x0F3F, amount: 1, x: 5, y: 5 }] });
  ctx.set("__scene2", Object.assign({}, ctx.run("scene"),
    { mobiles: [{ serial: 0x101, x: 6, y: 5, z: 0, noto: 1, name: "Bob", body: 400 }] }));
  ctx.run("scene = __scene2;");
  dropTarget(ctx, '<div id="map"></div>', [["#map", { left: 0, top: 0, width: 800, height: 600 }]]);
  // One body sprite for Bob, covering a box on screen. `mobSprites` is the real
  // map drawMobs fills; the stand-in only has to answer getBounds().
  ctx.run(`mobSprites.set("m257#body#0", { visible: true, getBounds: () => ({
             x: 300, y: 200, width: 40, height: 60,
             containsPoint: (px, py) => px >= 300 && px < 340 && py >= 200 && py < 260 }) });`);
  liftAndRelease(ctx, 320, 230);
  deepEq(ctx.sent, ["drop:513:65535:65535:0:257"], "addressed to Bob, not to a tile");
});

test("released over our own half of a trade window it goes into THAT session's container", () => {
  // Multiple sessions can be open at once, so the target comes from the
  // enclosing .trade-win, never a single global "current trade".
  const ctx = world(dndCtx(), { items: [{ serial: 0x201, g: 0x0F3F, amount: 1, x: 5, y: 5 }] });
  ctx.set("__trades", [
    { opponent: "Ann", opponentSerial: 0x101, myCont: 0x900, theirCont: 0x901,
      myAccept: 0, theirAccept: 0 },
    { opponent: "Cid", opponentSerial: 0x102, myCont: 0x910, theirCont: 0x911,
      myAccept: 0, theirAccept: 0 },
  ]);
  ctx.run("scene.trades = __trades; syncDialogs(scene);");
  eq(ctx.run(`dialogWindows("trades").size`), 2, "two trade windows");
  const win = ctx.run(`dialogWindow("trades", 0x910).el`);
  const grid = win.querySelector(".tr-mine-grid");
  win.rect = { left: 400, top: 100, width: 300, height: 260 };
  grid.rect = { left: 410, top: 150, width: 150, height: 120 };
  liftAndRelease(ctx, 430, 170);
  deepEq(ctx.sent, ["drop:513:20:20:0:2320"], "into Cid's session (0x910), at 20,20 in its grid");
});

test("released over nothing, the item stays on the cursor", () => {
  const ctx = world(dndCtx(), { items: [{ serial: 0x201, g: 0x0F3F, amount: 1, x: 5, y: 5 }] });
  liftAndRelease(ctx, 700, 500);          // no drop target mounted at all
  deepEq(ctx.sent, [], "nothing placed");
  ok(held(ctx), "still holding it — a UO cursor item does not fall on the floor");
});

test("the stale-drag flag does not swallow the next click", () => {
  // THE regression this file exists for. After a one-motion drag that landed on
  // nothing, `liftDrag` must be false — otherwise the next pointerdown returns
  // early ("the lifting press is still down") and the item can never be placed.
  const ctx = world(dndCtx(), { items: [{ serial: 0x201, g: 0x0F3F, amount: 1, x: 5, y: 5 }] });
  dropTarget(ctx, '<div id="map"></div>', [["#map", { left: 0, top: 0, width: 800, height: 600 }]]);
  liftAndRelease(ctx, 900, 500);          // released outside #map → still holding
  ok(held(ctx), "still holding");
  eq(ctx.run("liftDrag"), false, "the lifting press is over");
  pointer(ctx, "pointerdown", { clientX: 400, clientY: 300 });
  eq(held(ctx), null, "the NEXT click places it");
  eq(ctx.sent.length, 1, "one drop");
  ok(/^drop:513:/.test(ctx.sent[0]), `a real placement (${ctx.sent[0]})`);
});

test("while the lifting press is still down, a stray pointerdown does not place", () => {
  const ctx = world(dndCtx(), { items: [{ serial: 0x201, g: 0x0F3F, amount: 1, x: 5, y: 5 }] });
  dropTarget(ctx, '<div id="map"></div>', [["#map", { left: 0, top: 0, width: 800, height: 600 }]]);
  armWorld(ctx, { serial: 0x201, g: 0x0F3F, amount: 1 });
  ctx.setNow(1005);
  pointer(ctx, "pointermove", { clientX: 122, clientY: 100 });
  ctx.sent.length = 0;
  eq(ctx.run("liftDrag"), true, "the press that lifted it is still down");
  pointer(ctx, "pointerdown", { clientX: 400, clientY: 300 });
  deepEq(ctx.sent, [], "its own pointerup owns the placement, not a second press");
  ok(held(ctx), "still held");
});

test("a held item never places while the server is waiting for a target", () => {
  // The click has to answer the cursor; placing it would eat the click and leave
  // the shard waiting forever.
  const ctx = world(dndCtx(), { items: [{ serial: 0x201, g: 0x0F3F, amount: 1, x: 5, y: 5 }],
                                target: { active: 1, flag: 0 } });
  dropTarget(ctx, '<div id="map"></div>', [["#map", { left: 0, top: 0, width: 800, height: 600 }]]);
  ctx.run("cursorItem = { serial: 0x201, g: 0x0F3F, amount: 1, hue: 0, src: null };");
  pointer(ctx, "pointerdown", { clientX: 400, clientY: 300 });
  deepEq(ctx.sent, [], "no drop");
  ok(held(ctx), "still held");
});

test("Esc while holding returns the item to the pack with ServUO's 'you place it' sentinel", () => {
  // 0xFFFF/0xFFFF is a real wire value, not a guess: ServUO's Item.DropToItem
  // routes x == -1 && y == -1 to OnDroppedOnto → Container.DropItem. Sending 0,0
  // pinned every returned item to the top-left corner.
  const ctx = world(dndCtx(), { equip: BACKPACK });
  ctx.run("cursorItem = { serial: 0x201, g: 0x0F3F, amount: 1, hue: 0, src: null };");
  ctx.run("returnCursorItem()");
  deepEq(ctx.sent, ["drop:513:65535:65535:0:1280"], "back into the worn backpack");
  eq(held(ctx), null, "and off the cursor");
});

test("with no backpack, Esc drops it on the ground under the last cursor position", () => {
  const ctx = world(dndCtx(), { equip: [] });
  ctx.run("cursorItem = { serial: 0x201, g: 0x0F3F, amount: 1, hue: 0, src: null };");
  ctx.run("lastMenuX = 400; lastMenuY = 300; returnCursorItem();");
  eq(ctx.sent.length, 1, "one drop");
  ok(/:4294967295$/.test(ctx.sent[0]), `on the ground (${ctx.sent[0]})`);
});

// ── the counter bar: bind a slot without costing you the item ──────────────

test("dropping on a counter cell binds it AND puts the item back where it came from", () => {
  // ClassicUO CounterItem.OnMouseUp is this same pair: SetGraphic from the held
  // item, then DropItem back to hold.X/hold.Y/hold.Container.
  const ctx = world(dndCtx(), {
    equip: BACKPACK,
    contItems: [{ serial: 0x201, cont: 0x500, g: 0x0F3F, amount: 5, x: 30, y: 40, st: 1, hue: 0 }],
  });
  dropTarget(ctx, '<div class="cb-slot" data-i="2"></div>',
             [[".cb-slot", { left: 10, top: 10, width: 40, height: 40 }]]);
  ctx.run("liftToCursor(0x201, 0x0F3F, 5, 20, 20, 0)");
  ctx.sent.length = 0;
  ctx.run("liftDrag = true;");
  pointer(ctx, "pointerup", { clientX: 20, clientY: 20 });
  deepEq(ctx.sent, ["drop:513:30:40:0:1280"], "a WHOLE lift goes back to its exact spot");
  eq(ctx.run("counterSlots[2] && counterSlots[2].g"), 0x0F3F, "and the slot is bound to the graphic");
});

test("a PART of a stack goes back through the container's own placement, not to a spot", () => {
  // An exact-coordinate drop is OnDroppedInto, which would leave a second little
  // pile sitting on top of the first; 0xFFFF/0xFFFF is OnDroppedOnto, which
  // re-stacks it onto the pile it was split from.
  const ctx = world(dndCtx(), {
    equip: BACKPACK,
    contItems: [{ serial: 0x201, cont: 0x500, g: 0x0EED, amount: 200, x: 30, y: 40, st: 1, hue: 0 }],
  });
  dropTarget(ctx, '<div class="cb-slot" data-i="0"></div>',
             [[".cb-slot", { left: 10, top: 10, width: 40, height: 40 }]]);
  ctx.run("liftToCursor(0x201, 0x0EED, 37, 20, 20, 0)");   // a partial lift
  ctx.sent.length = 0;
  ctx.run("liftDrag = true;");
  pointer(ctx, "pointerup", { clientX: 20, clientY: 20 });
  deepEq(ctx.sent, ["drop:513:65535:65535:0:1280"], "let the container re-stack it");
});

// ── the optimistic-placement queue ─────────────────────────────────────────

test("the pending-placement queue is capped so a server that never acks cannot grow it", () => {
  // Current ServUO emits no 0x28/0x29, so nothing consumes this.
  const ctx = world(dndCtx(), { equip: BACKPACK });
  for (let i = 0; i < 100; i++) ctx.run(`sendPlacement("drop:${i}:0:0:0:1", ${i})`);
  eq(ctx.run("pendingPlacements.length"), 32, "capped at MAX_PENDING_PLACEMENTS");
  eq(ctx.run("pendingPlacements[0]"), 68, "the OLDEST entries are the ones dropped");
  eq(ctx.sent.length, 100, "every command still went out");
});
