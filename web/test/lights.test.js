// The night / light pass in web/js/04-boot.js (fxFrame), driven head-less.
//
// Darkness is the one renderer feature that is invisible to a screenshot taken
// at noon, and the whole pass is gated on `fxTint` — so a regression here shows
// up only as "the shard looked a bit bright last night". Everything below is
// asserted against the 2d-canvas calls the pass makes, which is exactly what a
// player would see.
const { newContext } = require("./harness.js");
const { test, ok, eq, deepEq } = require("./run.js");

function nightCtx() {
  const ctx = newContext();
  ctx.load("00-state.js", "01-audio.js", "02-textures.js", "03-world.js", "04-boot.js");
  // Every light.mul shape resolves instantly to a 100x100 mask, so a draw has a
  // measurable size and position. (The real one decodes a .mul and 404s in CI.)
  ctx.run("lightShape = (id, colour) => ({ width: 100, height: 100, __id: id, __c: colour });");
  ctx.set("__ctx", ctx.ctx2d);
  ctx.run(`
    app = { canvas: { clientWidth: 800, clientHeight: 600 },
            renderer: { width: 800, height: 600 },
            stage: { position: { x: 0, y: 0 } } };
    camZoom = 1;
    fxCanvas = { width: 800, height: 600 };
    fxCtx = __ctx;
    fxTint = 0.5;                 // deep night: past the 0.05 gate
    addSysMessage = () => {};
    playSfx = () => {};
  `);
  return ctx;
}
const frame = (ctx, now) => { ctx.clearCalls(); ctx.setNow(now); ctx.run(`fxFrame(${now})`); return ctx.calls("drawImage"); };

test("a carried light is offset into the hand; a ground light is not", () => {
  const ctx = nightCtx();
  // A wall torch on the ground, and the same torch carried by someone facing
  // East (dir 2 -> the equipped-light offset dx 22, dy 55).
  ctx.run(`scene = { light: 30, weather: 0xFF, weatherN: 0, lights: [
    { x: 5, y: 5, z: 0, r: 3, id: 7, c: 0 },
    { x: 5, y: 5, z: 0, r: 3, id: 7, c: 0, dx: 22, dy: 55 },
  ] };`);
  const drawn = frame(ctx, 1000);
  eq(drawn.length, 2, "both scene lights drew");
  const [a, b] = drawn;
  // drawImage(img, x, y, w, h) with x = sx - w/2, so the delta IS the offset.
  deepEq([b[1] - a[1], b[2] - a[2]], [22, 55], "the carried light is nudged into the hand");
  eq(a[0].__id, 7, "the ground light uses light.mul shape 7");
  eq(b[0].__id, 7, "…and so does the carried one");
});

test("an effect lights the ground only when its art says it emits light", async () => {
  const ctx = nightCtx();
  // One ordinary ground light is in the scene as well, so this case also shows
  // the two passes coexisting. It is no longer REQUIRED — see the test below,
  // which is the one that pins an effect lighting ground that has no light of
  // its own.
  ctx.run(`scene = { light: 30, weather: 0xFF, weatherN: 0, lights: [
    { x: 5, y: 5, z: 0, r: 3, id: 7, c: 0 } ] };`);
  // /iteminfo/<graphic> is the server's tiledata: `lt` is the light flag, `lid`
  // the shape. Both graphics here are real and were measured against the shard's
  // own tiledata rather than invented: 0x3709 (flamestrike) answers lt=1,
  // lid=30, and 0x376A answers lt=0. Worth having in the fixture because this
  // file used to say the flag existed only in theory — a sweep of the effect
  // ranges found 80 light-flagged graphics on this data, so "no effect art
  // carries it" was simply wrong.
  ctx.setFetch((u) => (u === "iteminfo/14089" ? { anim: 0, lt: 1, lid: 30 } : { anim: 0, lt: 0, lid: 0 }));
  ctx.run(`
    fxEffects.length = 0;
    fxEffects.push({ src: 5, kind: 0, born: 0, fm: 80, frames: [0x3709], hue: 0,
                     sprite: { x: 100, y: 300, visible: true, destroyed: false } });
    fxEffects.push({ src: 0, kind: 0, born: 0, fm: 80, frames: [0x3709], hue: 0,
                     sprite: { x: 200, y: 300, visible: true, destroyed: false } });
    fxEffects.push({ src: 7, kind: 0, born: 0, fm: 80, frames: [0x376A], hue: 0,
                     sprite: { x: 300, y: 300, visible: true, destroyed: false } });
  `);
  eq(frame(ctx, 1100).length, 1, "only the ground light before /iteminfo lands — no guess");
  await ctx.flush(2);
  const drawn = frame(ctx, 1200);
  eq(drawn.length, 2, "exactly one of the three effects lit anything");
  const fx = drawn[1];
  deepEq([fx[1], fx[2]], [100 - 50, 300 - 22 - 50],
         "…at the effect's own live position, tile-centred, mask-centred");
  eq(fx[0].__id, 1, "…using the SOURCE mobile's shape 1, not the art's own lid 30");
  // The other two: ClassicUO only lights an effect that has a Source, and only
  // when the art carries the light flag.
  eq(ctx.fetchLog.filter((u) => u === "iteminfo/14089").length, 1, "one /iteminfo per graphic, cached after");
});

test("at noon the veil eases away and the whole pass stops drawing", () => {
  const ctx = nightCtx();
  ctx.run(`scene = { light: 30, weather: 0xFF, weatherN: 0, lights: [
    { x: 5, y: 5, z: 0, r: 3, id: 7, c: 0 } ] };`);
  ok(frame(ctx, 1000).length > 0, "night draws");
  ctx.run("scene.light = 0;");                 // 0 = full daylight
  let drawn = [];
  for (let i = 0; i < 40; i++) drawn = frame(ctx, 1300 + i * 200);
  eq(drawn.length, 0, `nothing is drawn at noon (fxTint ${ctx.run("fxTint")})`);
});

// The effect pass used to sit inside `if (… scene.lights.length …)`, so a spell
// over ground with nothing else lit on it lit nothing — which is precisely when
// a spell should BE the light. It never actually bit, because `lights_json`
// always pushes the player's own glow and the list is therefore never empty.
// That glow is our own addition and not a ClassicUO port (it has no personal
// light), so the coupling would have come alive the day anyone removed it for
// fidelity. This test is what stops that happening quietly.
test("an effect lights ground that has no light of its own", async () => {
  const ctx = nightCtx();
  ctx.run(`scene = { light: 30, weather: 0xFF, weatherN: 0, lights: [] };`);
  ctx.setFetch((u) => (u === "iteminfo/14089" ? { anim: 0, lt: 1, lid: 30 } : { anim: 0, lt: 0, lid: 0 }));
  ctx.run(`
    fxEffects.length = 0;
    fxEffects.push({ src: 5, kind: 0, born: 0, fm: 80, frames: [0x3709], hue: 0,
                     sprite: { x: 100, y: 300, visible: true, destroyed: false } });
  `);
  frame(ctx, 1100);              // first pass only asks /iteminfo
  await ctx.flush(2);
  const drawn = frame(ctx, 1200);
  eq(drawn.length, 1, "the effect lit the ground with no scene light present");
  deepEq([drawn[0][1], drawn[0][2]], [100 - 50, 300 - 22 - 50],
         "…at the effect's own position, as it does when other lights are around");
});

// A scene with no lights AND no effects still paints the darkness itself — the
// veil is not conditional on anything having a glow in it.
test("an empty light list still darkens the world", () => {
  const ctx = nightCtx();
  ctx.run(`scene = { light: 30, weather: 0xFF, weatherN: 0, lights: [] };
           fxEffects.length = 0;`);
  ctx.clearCalls();
  ctx.setNow(1100); ctx.run("fxFrame(1100)");
  eq(ctx.calls("drawImage").length, 0, "nothing to punch");
  ok(ctx.calls("fillRect").length > 0, "…but the night veil is still drawn");
});
