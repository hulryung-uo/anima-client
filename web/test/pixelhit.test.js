// Which container item is under the cursor.
//
// ClassicUO hit-tests by PIXEL (`ItemGump.Contains` → `Arts.PixelCheck`,
// :196-228), so a press through the transparent corner of one icon falls to
// whatever is drawn beneath it. Our cells are divs and used to catch their whole
// bounding box. That is not a corner case in a real backpack: measured live on a
// 31-item pack, 286 of the cell pairs overlapped and there were 58 distinct
// points where the press grabbed an item that was fully transparent there.
//
// Fixtures here are two overlapping items whose alpha the test writes, so
// "transparent" and "painted" mean exactly what the test says they mean.
const { newContext } = require("./harness.js");
const { test, ok, eq, deepEq } = require("./run.js");

const BAG = 0x777;
// TOP is drawn second (later in document order paints on top). Its left half is
// transparent; BOTTOM is solid. They overlap on screen.
const TOP = 0x301, BOTTOM = 0x302;

function pixelCtx() {
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
    playSfx = () => {};
  `);
  // The real page installs the window-level pointer listeners from boot();
  // a test loads the code, not the page, so it installs them itself.
  ctx.run("setupItemDnD()");
  ctx.set("__scene", {
    player: { serial: 9, x: 5, y: 5, z: 0, name: "Me",
              equip: [{ serial: 0x500, layer: 21, g: 0x0E75 }] },
    mobiles: [], items: [], statics: [],
    contItems: [
      { cont: BAG, serial: BOTTOM, g: 0x0F3F, amount: 1, x: 20, y: 20, hue: 0 },
      { cont: BAG, serial: TOP,    g: 0x0F41, amount: 1, x: 10, y: 10, hue: 0 },
    ],
    contGumps: { [String(BAG)]: 0x3C }, contInfo: {}, target: { active: 0, flag: 0 },
  });
  ctx.run("scene = __scene; updateTargetUI();");
  ctx.run(`openContainer(${BAG})`);
  const win = ctx.run(`dialogWindow("containers", ${BAG})`);
  win.el.rect = { left: 100, top: 100, width: 200, height: 200 };
  win.body.rect = { left: 100, top: 100, width: 200, height: 200 };
  // Both icons 40x40. TOP sits at body+10, BOTTOM at body+20, so they overlap
  // over the square (120,120)-(150,150).
  const place = (serial, l, t, alpha) => {
    const all = [...win.body.querySelectorAll(".cont-item[data-serial]")];
    const cell = all.find((c) => (+c.dataset.serial) === serial);
    if (!cell) throw new Error(`no cell for ${serial}; have ${all.map((c) => c.dataset.serial)}`);
    const img = cell.querySelector("img");
    cell.rect = { left: l, top: t, width: 40, height: 40 };
    img.rect = { left: l, top: t, width: 40, height: 40 };
    img.natural = { w: 40, h: 40 };
    img.alpha = alpha;
    return cell;
  };
  ctx.top = place(TOP, 110, 110, (x) => (x < 20 ? 0 : 255));   // left half transparent
  ctx.bottom = place(BOTTOM, 120, 120, () => 255);             // solid
  ctx.at = (x, y) => ctx.run(`contItemAt(${x}, ${y}, __from)`);
  ctx.from = (cell) => ctx.set("__from", cell);
  return ctx;
}

test("a point over the top item's painted half picks the top item", () => {
  const ctx = pixelCtx();
  ctx.from(ctx.top);
  eq(ctx.at(140, 130), ctx.top, "its right half is solid, and it is on top");
});

test("a point over the top item's TRANSPARENT half falls to what is drawn there", () => {
  const ctx = pixelCtx();
  ctx.from(ctx.top);          // the box test would have answered TOP here
  eq(ctx.at(125, 130), ctx.bottom, "the item actually painted at that pixel");
});

test("a point over no item's art belongs to the container, not to a bounding box", () => {
  const ctx = pixelCtx();
  ctx.from(ctx.top);
  eq(ctx.at(112, 115), null, "TOP is transparent there and BOTTOM does not reach it");
});

test("an icon whose pixels cannot be read keeps the old box behaviour", () => {
  const ctx = pixelCtx();
  // No natural size: a real <img> mid-load. It must stay clickable rather than
  // becoming a hole in the UI while the art is on its way.
  const img = ctx.top.querySelector("img");
  img.natural = undefined; img.alpha = undefined;
  ctx.from(ctx.top);
  eq(ctx.at(125, 130), ctx.top, "an undecodable icon answers for its whole box");
});

test("the press lifts the item the pixels say, not the one whose box was on top", () => {
  const ctx = pixelCtx();
  const img = ctx.top.querySelector("img");
  ctx.setNow(1000);
  // Press inside TOP's box but on its transparent half, then drag out of it.
  ctx.fire("window", "pointerdown", { button: 0, bubbles: true, target: img,
                                      clientX: 125, clientY: 130 });
  const armed = ctx.run("groundDrag && groundDrag.serial");
  eq(armed, 0x302, "the press armed BOTTOM, the item painted at that pixel");
  ctx.setNow(1005);
  ctx.fire("window", "pointermove", { button: 0, bubbles: true, clientX: 400, clientY: 400 });
  deepEq(ctx.sent.filter((s) => s.startsWith("pickup:")), ["pickup:770"],
         "…and the drag lifted it");
});
