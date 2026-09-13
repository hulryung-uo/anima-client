const { newContext } = require("./harness.js");
const { test, ok, eq, deepEq } = require("./run.js");

function fixture() {
  const ctx = newContext(); ctx.mountPage(); ctx.loadAll();
  ctx.fetchLog.length = 0; // loadAll also starts the independent skill-info request
  ctx.setFetch(() => info(3));
  return ctx;
}
function info(frames) { return { frames, c: Array.from({ length: frames }, (_, i) => [i, -i]) }; }
async function request(ctx, body) { ctx.run(`framesFor(${body},0,0)`); await ctx.flush(); }

test("a failed animation request is retried instead of permanently hiding body parts", async () => {
  const ctx = fixture(); let calls = 0;
  ctx.setFetch(async () => { if (++calls === 1) throw new Error("offline"); return info(3); });
  await request(ctx, 400);
  ok(!ctx.get("frameCount").has("400/0/0"), "a transport failure is not a known missing animation");
  eq(ctx.get("animInfoPending").size, 0); eq(ctx.get("animInfoLoads").size, 0);
  await request(ctx, 400); eq(calls, 1);
  ctx.setNow(2000); await request(ctx, 400);
  eq(ctx.run("framesFor(400,0,0)"), 3); deepEq(ctx.run("centerFor(400,0,0,2)"), [2, -2]);
  eq(ctx.get("animInfoFailures").size, 0); eq(ctx.get("loading").size, 0);
});

test("a confirmed zero-frame animation stays distinguishable from pending data", async () => {
  const ctx = fixture(); ctx.setFetch(() => info(0));
  await request(ctx, 400); eq(ctx.get("frameCount").get("400/0/0"), 0);
  eq(ctx.run("framesFor(400,0,0)"), 1, "render arithmetic keeps a nonzero divisor");
  eq(ctx.run("centerFor(400,0,0,0)"), null);
  ctx.setNow(60_000); await request(ctx, 400); eq(ctx.fetchLog.length, 1);
});

test("animation fetch admission is separate from texture work and keeps physical slots after timeout", async () => {
  const ctx = fixture(), finish = [], signals = [];
  ctx.setFetch((_, init) => { signals.push(init.signal); return new Promise(resolve => finish.push(resolve)); });
  ctx.run("for(let i=0;i<100;i++) framesFor(i,0,0)");
  eq(ctx.fetchLog.length, 8); eq(ctx.get("loading").size, 0);
  ctx.run('texFor("body.png")'); await ctx.flush(); ok(ctx.get("texCache").has("body.png"));
  ctx.advance(5000); ok(signals.every(signal => signal.aborted));
  eq(ctx.get("animInfoPending").size, 0); eq(ctx.get("animInfoLoads").size, 8);
  await request(ctx, 100); eq(ctx.fetchLog.length, 8, "an ignored abort does not allow more physical requests");
  for (const resolve of finish) resolve(info(3)); await ctx.flush();
  eq(ctx.get("animInfoLoads").size, 0); eq(ctx.get("frameCount").size, 0);
  ctx.setFetch(() => info(3)); await request(ctx, 100); eq(ctx.run("framesFor(100,0,0)"), 3);
});

test("a timed-out body cannot overwrite a newer animation count or its draw centers", async () => {
  const ctx = fixture(); let finishBody, signal;
  ctx.setFetch((_, init) => { signal = init.signal; return { ok: true,
    text: () => new Promise(resolve => { finishBody = resolve; }) }; });
  await request(ctx, 400); ctx.advance(5000); ok(signal.aborted);
  ctx.advance(2000); ctx.setFetch(() => info(4)); await request(ctx, 400);
  eq(ctx.run("framesFor(400,0,0)"), 4);
  finishBody(JSON.stringify(info(2))); await ctx.flush();
  eq(ctx.run("framesFor(400,0,0)"), 4); deepEq(ctx.run("centerFor(400,0,0,3)"), [3, -3]);
});

test("HTTP failures and malformed metadata do not become missing-animation records", async () => {
  const ctx = fixture();
  const cases = [
    { ok: false, status: 503, body: info(0) },
    { frames: -1, c: [] }, { frames: 2, c: [[0, 0]] },
    { frames: 1, c: [["0", 0]] }, { frames: 1, c: [[0, 0, 0]] },
    { ok: true, text: async () => "broken JSON" },
  ];
  for (let i = 0; i < cases.length; i++) {
    ctx.setFetch(() => cases[i]); await request(ctx, i);
  }
  eq(ctx.get("frameCount").size, 0); eq(ctx.get("animInfoPending").size, 0);
  eq(ctx.get("animInfoFailures").size, cases.length);
});

test("animation response limits include chunked bodies without a content length", async () => {
  const ctx = fixture(); let reads = 0, canceled = 0, released = 0;
  ctx.setFetch(() => ({ ok: true, body: { getReader: () => ({
    read: async () => { reads++; return { done: false, value: new Uint8Array(600_000) }; },
    cancel: async () => { canceled++; }, releaseLock: () => { released++; },
  }) } }));
  await request(ctx, 400); eq(reads, 2); eq(canceled, 1); eq(released, 1);
  eq(ctx.get("frameCount").size, 0); eq(ctx.get("animInfoPending").size, 0);
});

test("metadata entry eviction preserves hot animations and removes their centers together", async () => {
  const ctx = fixture(); ctx.setFetch(() => info(0));
  for (let i = 0; i < 4096; i++) await request(ctx, i);
  ctx.run("framesFor(0,0,0)"); await request(ctx, 4096);
  eq(ctx.get("frameCount").size, 4096); eq(ctx.get("frameCtr").size, 4096);
  ok(ctx.get("frameCount").has("0/0/0")); ok(!ctx.get("frameCount").has("1/0/0"));
  ok(!ctx.get("frameCtr").has("1/0/0"));
  await request(ctx, 1); ok(ctx.get("frameCount").has("1/0/0"));
});

test("large metadata entries respect the total center budget, including center-only cache hits", async () => {
  const ctx = fixture(); ctx.setFetch(() => info(32768));
  await request(ctx, 0); await request(ctx, 1); ctx.run("centerFor(0,0,0,0)");
  await request(ctx, 2);
  eq(ctx.get("frameCenterCount"), 65536); eq(ctx.get("frameCtr").size, 2);
  ok(ctx.get("frameCount").has("0/0/0")); ok(!ctx.get("frameCount").has("1/0/0"));
});
