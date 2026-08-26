// Ground piles in web/js/05-poll.js: does a stack of more than one draw as a heap?
//
// ClassicUO draws a stackable item whose amount is > 1 twice, the copy shifted
// (-5,-5), so a pile of arrows reads as a pile. The interesting part is not the
// second sprite — it is that the copy is not a click target, tracks the item's
// animation frame, and appears and disappears as the amount crosses 1 (dropping
// one more arrow, picking one back up).
const { newContext } = require("./harness.js");
const { test, ok, eq, deepEq } = require("./run.js");

function worldCtx() {
  const ctx = newContext();
  ctx.load("dialogs.js", "00-state.js", "01-audio.js", "02-textures.js", "03-world.js",
           "04-boot.js", "05-poll.js", "06-movement.js", "07-hud.js", "08-overlays.js",
           "09-gumps.js", "10-housing.js", "11-dragdrop.js", "12-input.js");
  // Every art request resolves instantly to a 20px-tall texture; the real one
  // fetches a PNG the play server renders out of art.mul.
  ctx.run(`
    world = new PIXI.Container();
    texFor = (url) => ({ height: 20, width: 20, __url: url });
    app = { canvas: { clientWidth: 800, clientHeight: 600 },
            renderer: { width: 800, height: 600 }, stage: { position: { x: 0, y: 0 } } };
  `);
  return ctx;
}

const item = (over) => Object.assign({ serial: 1, g: 0x0F3F, x: 5, y: 5, z: 0, amount: 1, hue: 0 }, over);
function sync(ctx, items) {
  ctx.setNow(1000);
  ctx.set("__scene", { map: { radius: 1, cx: 5, cy: 5, tiles: [], viewRange: 1 },
                       statics: [], items, mobiles: [], player: { serial: 9, x: 5, y: 5, z: 0 } });
  ctx.run("scene = __scene; syncWorld(__scene);");
  return ctx.run("itemPool.get(1)");
}
// The heap copy is the entry sprite's own child.
const heapOf = (e) => (e && e.sp.children.filter((c) => c !== e.sp)) || [];

test("a stack the server marks as a pile gets a second sprite", () => {
  const ctx = worldCtx();
  const e = sync(ctx, [item({ amount: 200, st: 1, pl: 1 })]);
  const kids = heapOf(e);
  eq(kids.length, 1, "one extra sprite");
  deepEq([kids[0].x, kids[0].y], [-5, -5], "offset up-left, ClassicUO's (-5,-5)");
  eq(kids[0].texture, e.sp.texture, "same art as the item itself");
  eq(kids[0].eventMode, "none", "never a click target — clicking a pile must hit the item");
  deepEq([kids[0].anchor.x, kids[0].anchor.y], [0.5, 1.0], "anchored like the item, so the copy is a clean 5px shift");
});

test("nothing that is not a pile grows a heap", () => {
  for (const [why, over] of [["a single item", { amount: 1, st: 1 }],
                             ["a gold pile (amount-tiered art instead)", { g: 0x0EEF, amount: 600, st: 1 }],
                             ["a non-stackable", { amount: 3 }]]) {
    const ctx = worldCtx();
    eq(heapOf(sync(ctx, [item(over)])).length, 0, `no heap for ${why}`);
  }
});

test("the heap appears and disappears as the amount crosses one", () => {
  const ctx = worldCtx();
  eq(heapOf(sync(ctx, [item({ amount: 1, st: 1 })])).length, 0, "one arrow: no heap");
  eq(heapOf(sync(ctx, [item({ amount: 2, st: 1, pl: 1 })])).length, 1, "dropping a second arrow grows the heap");
  eq(heapOf(sync(ctx, [item({ amount: 1, st: 1 })])).length, 0, "picking one back up loses it again");
  // The rebuild must not move or re-graphic the item itself.
  const e = ctx.run("itemPool.get(1)");
  eq(e.g, 0x0F3F, "the graphic survived the rebuild");
});

test("an animated stack cycles both copies together", () => {
  const ctx = worldCtx();
  const e = sync(ctx, [item({ amount: 5, st: 1, pl: 1, a: [0x0F3F, 0x0F40, 0x0F41], ai: 100 })]);
  eq(ctx.run("animatedStatics.size"), 1, "the animated stack registered");
  ctx.run("tickAnimatedStatics(100000)");
  const kids = heapOf(e);
  eq(kids[0].texture, e.sp.texture, "the heap copy followed the animation frame");
  ok(String(e.sp.texture.__url).length > 0, `…to a real art url (${e.sp.texture.__url})`);
});
