// End to end: does a fidget the idle clock starts actually reach the SPRITE?
//
// idle.test.js proves the clock; this proves the wiring. drawMobs() is the real
// one — the only stubs are the texture cache (a PNG per frame from the play
// server) and the frame-count table, so what is asserted is which animation URLs
// the renderer asks for, frame by frame, which is precisely what a player sees.
const { newContext } = require("./harness.js");
const { test, ok, eq, includes } = require("./run.js");

function drawCtx() {
  const ctx = newContext({ seed: 4242 });
  ctx.load("dialogs.js", "00-state.js", "01-audio.js", "02-textures.js", "03-world.js",
           "04-boot.js", "05-poll.js", "06-movement.js", "07-hud.js", "08-overlays.js",
           "09-gumps.js", "10-housing.js", "11-dragdrop.js", "12-input.js");
  const urls = [];
  ctx.set("__urls", urls);
  ctx.run(`
    world = new PIXI.Container();
    mobs = new PIXI.Container();
    entLayer = new PIXI.Graphics();
    barLayer = new PIXI.Container();
    overLayer = new PIXI.Container();
    app = { canvas: { clientWidth: 800, clientHeight: 600 },
            renderer: { width: 800, height: 600 },
            stage: { position: { x: 0, y: 0 }, scale: { set() {} } },
            screen: { width: 800, height: 600 } };
    texFor = (u) => { __urls.push(u); return { height: 40, width: 30, __url: u }; };
    // Every people group has 5 frames, so no draw is waiting on a fetch.
    for (const g of [0, 2, 4, 5, 6, 34]) for (let d = 0; d < 8; d++) {
      frameCount.set('400/' + g + '/' + d, 5);
      frameCtr.set('400/' + g + '/' + d, [[10,-4],[10,-4],[10,-4],[10,-4],[10,-4]]);
    }
  `);
  ctx.setFetch(() => ({ g: [5, 6, 34], e: [1, 1, 1] }));   // /idleanim/400
  const mob = { serial: 0x101, x: 10, y: 10, z: 0, dir: 2, body: 400, at: 2, noto: 1,
                name: "Vendor", hits: 100, hitsMax: 100, hue: 0, equip: [], mounted: 0 };
  ctx.set("__scene", { player: { serial: 9, x: 10, y: 12, z: 0, dir: 2, body: 400, at: 2, name: "me" },
                       mobiles: [mob], items: [], statics: [], lights: [],
                       map: { radius: 1, cx: 10, cy: 12, tiles: [], viewRange: 1 } });
  ctx.run(`
    scene = __scene;
    anim.clear();
    anim.set("m257", { body: 400, at: 2, dir: 2, x: 10, y: 10, z: 0, rx: 10, ry: 10, rz: 0,
                       tx: 10, ty: 10, animMoving: false, animPhase: 0, mobRec: __scene.mobiles[0] });
  `);
  ctx.urls = urls;
  ctx.draw = (now) => { urls.length = 0; ctx.setNow(now); ctx.run("drawMobs()"); };
  ctx.groups = () => new Set(urls.filter((u) => u.startsWith("anim/400/")).map((u) => Number(u.split("/")[2])));
  return ctx;
}

test("a standing mobile draws the people stand group and arms the idle clock", () => {
  const ctx = drawCtx();
  ctx.draw(0);
  includes(ctx.groups(), 4, "group 4 (people stand)");
  const st = ctx.run('anim.get("m257")');
  ok(st.idleAt >= 30000 && st.idleAt <= 60000, `idle clock armed at ${st.idleAt}`);
});

test("the fidget the clock picks is the art the sprite asks for", async () => {
  const ctx = drawCtx();
  ctx.draw(0);
  const armed = ctx.run('anim.get("m257")').idleAt;
  ctx.draw(70000);                       // first expiry kicks /idleanim
  await ctx.flush();
  ctx.draw(armed > 70000 ? armed + 1 : 140000);
  const st = ctx.run('anim.get("m257")');
  ok(st.act, "a fidget started");
  eq(st.act.idle, true, "…as a client-side idle");
  includes([5, 6, 34], st.act.group, "…on a real people fidget group");
  const drawn = ctx.groups();
  includes(drawn, st.act.group, "the sprite is asking for THAT group's art");
  ok(!drawn.has(4), `…and not the stand group any more (${[...drawn]})`);
});

test("the fidget plays its frames and then hands the body back to standing", async () => {
  const ctx = drawCtx();
  ctx.draw(0);
  const armed = ctx.run('anim.get("m257")').idleAt;
  ctx.draw(70000);
  await ctx.flush();
  ctx.draw(armed > 70000 ? armed + 1 : 140000);
  const act = ctx.run('anim.get("m257")').act;
  const g = act.group, start = act.startMs;

  ctx.draw(start + 500);
  includes(ctx.groups(), g, "mid-fidget: still that group");
  const mid = ctx.urls.filter((u) => u.startsWith(`anim/400/${g}/2/`)).pop();
  ok(mid && mid.endsWith("/2.png"), `advancing through its frames (${mid})`);

  ctx.draw(start + 5 * 240 + 10);
  ok(!ctx.run('anim.get("m257")').act, "after 5 x 240 ms it is retired");
  ctx.draw(start + 5 * 240 + 20);
  includes(ctx.groups(), 4, "…and the body is standing again");
});
