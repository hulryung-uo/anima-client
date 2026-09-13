const fs = require("node:fs"), path = require("node:path"), vm = require("node:vm");
const { newContext } = require("./harness.js");
const { test, ok, eq, ne, deepEq } = require("./run.js");

function fixture() {
  const ctx = newContext(); ctx.mountPage(); ctx.loadAll();
  const loads = [], unloads = [];
  ctx.get("PIXI").Assets.load = async url => { loads.push(url); return texture(); };
  ctx.get("PIXI").Assets.unload = async url => { unloads.push(url); };
  return { ctx, loads, unloads };
}
function texture(width = 4, height = 4) { return { source: { pixelWidth: width, pixelHeight: height } }; }
async function load(ctx, url) { ctx.set("__url", url); ctx.run("texFor(__url)"); await ctx.flush(); return ctx.get("texCache").get(url); }

test("RGBA pressure releases the coldest art while displayed art keeps its texture", async () => {
  const { ctx, unloads } = fixture();
  ctx.get("PIXI").Assets.load = async () => texture(4096, 4096); // 64 MiB per decoded surface
  for (let i = 0; i < 5; i++) await load(ctx, `art/${i}.png`);
  eq(ctx.get("texBytes"), 320 * 1024 * 1024, "new arrivals get a poll to become visible");
  ctx.run('tilePool.set("live", {url:"art/0.png"})');
  ctx.setNow(2000); ctx.run("sweepTexCache()"); await ctx.flush();
  deepEq(unloads, ["art/1.png"]); eq(ctx.get("texBytes"), 256 * 1024 * 1024);
  ok(ctx.get("texCache").has("art/0.png"));
});

test("every live reference is protected even when the visible working set exceeds the budget", async () => {
  const { ctx, unloads } = fixture();
  ctx.get("PIXI").Assets.load = async () => texture(4096, 4096);
  for (const url of ["tile", "static", "static-frame", "item", "item-frame", "last-good"]) await load(ctx, url);
  ctx.run(`tilePool.set("ring", {url:"tile"});
    staticPool.set("animated", {_texUrl:"static", _frameUrls:["static-frame", "not-loaded"]});
    itemPool.set(1, {url:"item", sp:{_frameUrls:["item-frame"]}});
    anim.set(1, {partTex:new Map([["body", {url:"last-good"}]])});`);
  ctx.setNow(600_000); ctx.run("sweepTexCache(); forEachLiveTexUrl(touchTex)");
  eq(unloads.length, 0); eq(ctx.get("texCache").size, 6);
  ok(!ctx.get("texLastUsed").has("not-loaded"), "prefetch placeholders do not leak LRU metadata");
});

test("scene polling drains over-budget art after the last image completion", async () => {
  const { ctx, unloads } = fixture();
  ctx.get("PIXI").Assets.load = async () => texture(4096, 4096);
  for (let i = 0; i < 5; i++) await load(ctx, `old-town/${i}.png`);
  ctx.run(`world=new PIXI.Container(); mobs=new PIXI.Container(); entLayer=new PIXI.Graphics();
    overLayer=new PIXI.Container(); itemLayer=new PIXI.Container(); barLayer=new PIXI.Container();
    scene={sessionId:"one",player:{serial:9,x:1000,y:2000},map:{radius:0,tiles:[]},mobiles:[],items:[],journal:[],sounds:[]};`);
  ctx.setNow(2000); ctx.run("syncWorld(scene)"); await ctx.flush();
  eq(unloads.length, 1); eq(ctx.get("texBytes"), 256 * 1024 * 1024);
});

test("stationary previews, corpse child layers and shared-source meshes stay alive without URL pool entries", async () => {
  const { ctx, unloads } = fixture();
  ctx.get("PIXI").Assets.load = async () => texture(4096, 4096);
  for (const url of ["corpse-clothing", "house-preview", "slice", "cold", "another"]) await load(ctx, url);
  ctx.run(`world=new PIXI.Container();
    var corpse=new PIXI.Container(); world.addChild(corpse);
    corpse.addChild(new PIXI.Sprite(texCache.get("corpse-clothing")));
    world.addChild(new PIXI.Sprite(texCache.get("house-preview")));
    world.addChild(new PIXI.Sprite({source:texCache.get("slice").source}));`);
  ctx.setNow(600_000); ctx.run("sweepTexCache()"); await ctx.flush();
  deepEq(unloads, ["cold"]);
  ctx.run("world.removeChildren()");
  await load(ctx, "new-pressure"); ctx.setNow(602_000); ctx.run("sweepTexCache()"); await ctx.flush();
  ok(unloads.includes("corpse-clothing"), "removing the last on-stage owner makes it reclaimable");
});

test("entry limits also apply to thousands of tiny textures and their metadata", async () => {
  const { ctx } = fixture();
  for (let i = 0; i < 1600; i++) await load(ctx, `tiny/${i}.png`);
  ctx.setNow(2000); ctx.run("sweepTexCache()"); await ctx.flush();
  eq(ctx.get("texCache").size, 1500); eq(ctx.get("texLastUsed").size, 1500); eq(ctx.get("texSizes").size, 1500);
  eq(ctx.get("texBytes"), 1500 * 64);
});

test("bounded loader work coalesces duplicates, pumps its queue and drops stale prefetches", async () => {
  const { ctx } = fixture(); const pending = [], urls = [];
  ctx.get("PIXI").Assets.load = url => { urls.push(url); return new Promise(resolve => pending.push(resolve)); };
  ctx.run('for(let i=0;i<1000;i++) {texFor("burst/"+i); texFor("burst/"+i);}');
  eq(urls.length, 16); eq(ctx.get("loading").size, 16); eq(ctx.get("texQueue").size, 512);
  pending[0](texture()); await ctx.flush(); eq(urls.length, 17, "next queued image starts without waiting for scene poll");
  ctx.setNow(3000);
  for (const resolve of pending) resolve(texture()); await ctx.flush();
  eq(urls.length, 17, "old town's abandoned prefetches are not downloaded");
  eq(ctx.get("loading").size, 0); eq(ctx.get("texQueue").size, 0);
  await load(ctx, "burst/999"); eq(urls.at(-1), "burst/999", "omitted art can be requested on the next view");
  pending.at(-1)(texture()); await ctx.flush();
});

test("texture transport failures retry after cooldown without caching permanent holes", async () => {
  const { ctx } = fixture(); let calls = 0;
  ctx.get("PIXI").Assets.load = async () => { calls++; if (calls === 1) throw new Error("outage"); return texture(); };
  eq(await load(ctx, "recover.png"), undefined); eq(ctx.get("loading").size, 0);
  await load(ctx, "recover.png"); eq(calls, 1);
  ctx.setNow(2000); ok(await load(ctx, "recover.png")); eq(calls, 2);
  eq(ctx.get("texFailures").size, 0);
});

test("the bundled Pixi loader cannot hand the client a texture still being unloaded", async () => {
  // Actual vendored Assets/Loader/Cache + Texture lifetime, no browser/GPU or network.
  // Pixi's unload is asynchronous even when the original load has completed.
  const box = { console, setTimeout, clearTimeout, URL, URLSearchParams, performance,
    navigator: { userAgent: "Node" }, document: { baseURI: "http://fixture.invalid/" } };
  vm.createContext(box);
  vm.runInContext(fs.readFileSync(path.join(__dirname, "../vendor/pixi.min.js"), "utf8"), box);
  const pixi = box.PIXI;
  await pixi.Assets.init({ skipDetections: true });
  let finishUnload;
  pixi.Assets.loader.parsers.unshift({ name: "cache-fixture", test: url => url.endsWith(".fixture"),
    load: async () => new pixi.Texture({ source: new pixi.TextureSource({ width: 4096, height: 4096 }) }),
    unload: async t => { await new Promise(resolve => { finishUnload = resolve; }); t.destroy(true); },
  });
  const { ctx } = fixture(); ctx.get("PIXI").Assets = pixi.Assets;
  const first = await load(ctx, "old.fixture");
  for (let i = 0; i < 4; i++) await load(ctx, `new${i}.fixture`);
  ctx.setNow(2000); ctx.run("sweepTexCache()"); await ctx.flush();
  eq(ctx.run('texFor("old.fixture")'), null, "old SDK cache value is not rebound during release");
  eq(first.destroyed, false); finishUnload(); await ctx.flush(); eq(first.destroyed, true);
  const replacement = await load(ctx, "old.fixture");
  ne(replacement, first); eq(replacement.destroyed, false); ok(replacement.source);
});

test("failed asynchronous unloads settle their guard and permit a later retry", async () => {
  const { ctx } = fixture(); ctx.get("PIXI").Assets.load = async () => texture(4096, 4096);
  ctx.get("PIXI").Assets.unload = async () => { throw new Error("parser failed to dispose"); };
  for (let i = 0; i < 5; i++) await load(ctx, `discard/${i}`);
  ctx.setNow(2000); ctx.run("sweepTexCache()"); await ctx.flush();
  eq(ctx.get("texUnloading").size, 0); eq(ctx.run('texFor("discard/0")'), null);
  ctx.setNow(4000); ok(await load(ctx, "discard/0"));
});

test("pixel hit masks retain transparency, follow frames and are freed with their textures", async () => {
  const { ctx } = fixture();
  const img = ctx.document.createElement("img"); img.natural = { w: 4, h: 4 }; img.alpha = x => x < 2 ? 0 : 255;
  ctx.set("HTMLImageElement", img.constructor);
  ctx.get("PIXI").Assets.load = async () => ({ source: { resource: img, pixelWidth: 4096, pixelHeight: 4096 } });
  await load(ctx, "old.png");
  ctx.run('var hit = pixelHitArea({width:4,height:4,anchor:{x:0,y:0}}, () => "old.png")');
  eq(ctx.run("hit.contains(0,0)"), false); eq(ctx.run("hit.contains(3,0)"), true);
  for (let i = 0; i < 4; i++) await load(ctx, `more/${i}`);
  ctx.setNow(2000); ctx.run("sweepTexCache()"); await ctx.flush();
  ok(!ctx.get("alphaMaskCache").has("old.png"));
  await load(ctx, "old.png"); ctx.run('requestAlphaMask("old.png")');
  eq(ctx.run("hit.contains(0,0)"), false, "a revisited image rebuilds its precise hit area");
});

test("a late fallback image cannot resurrect an evicted mask", async () => {
  const { ctx } = fixture(); const images = [];
  ctx.set("Image", function () { images.push(this); });
  ctx.get("PIXI").Assets.load = async () => texture(4096, 4096);
  await load(ctx, "old.png"); ctx.run('requestAlphaMask("old.png")');
  const late = images[0].onload;
  for (let i = 0; i < 4; i++) await load(ctx, `more/${i}`);
  ctx.setNow(2000); ctx.run("sweepTexCache()"); await ctx.flush();
  eq(images[0].src, ""); eq(ctx.get("alphaMaskPending").size, 0);
  late(); ok(!ctx.get("alphaMaskCache").has("old.png"));
});
