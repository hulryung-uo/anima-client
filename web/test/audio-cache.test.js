const { newContext } = require("./harness.js");
const { test, ok, eq, deepEq } = require("./run.js");

class BufferFixture {
  constructor(bytes = 4) { this.length = bytes / 4; this.numberOfChannels = 1; }
}
function fixture() {
  const ctx = newContext(); ctx.mountPage(); ctx.load("00-state.js", "01-audio.js", "04-connection.js", "05-poll.js");
  ctx.set("AudioBuffer", BufferFixture);
  const sources = [];
  const audio = {
    state: "running", decodeAudioData: async () => new BufferFixture(),
    createBufferSource: () => {
      const src = { connect() {}, start() { this.started = true; }, stop() { this.stopped = true; } };
      sources.push(src); return src;
    },
  };
  ctx.set("audioCtx", audio); ctx.set("setStatus", () => {});
  ctx.run('scene = {sessionId:"one"}; settings.sfx = true;');
  ctx.setFetch(() => ({ ok: true, arrayBuffer: async () => new ArrayBuffer(0) }));
  return { ctx, audio, sources };
}

test("decoded cache evicts cold sounds by bytes while active playback keeps its buffer", async () => {
  const { ctx, audio, sources } = fixture();
  audio.decodeAudioData = async () => new BufferFixture(48 * 1024 * 1024);
  await ctx.run("loadSfx(1); loadSfx(2)"); await ctx.flush();
  ctx.run("playSfx(1, 0, 0)"); ok(sources[0].started);
  await ctx.run("loadSfx(3)");
  ok(ctx.run("sfxBuffers.has(1)")); ok(!ctx.run("sfxBuffers.has(2)"));
  eq(ctx.run("sfxBufferBytes"), 96 * 1024 * 1024);
  await ctx.run("loadSfx(4)"); // buffer 1 is now cold but still playing
  ok(!ctx.run("sfxBuffers.has(1)")); ok(!sources[0].stopped);
  eq(sources[0].buffer.length * 4, 48 * 1024 * 1024);
  const before = ctx.fetchLog.length;
  await ctx.run("loadSfx(2)"); eq(ctx.fetchLog.length, before + 1);
  ok(ctx.run("sfxBufferBytes <= SFX_CACHE_BYTES"));
});

test("hundreds of small decoded sounds cannot exceed the cache entry limit", async () => {
  const { ctx } = fixture();
  for (let id = 0; id < 600; id++) await ctx.run(`loadSfx(${id})`);
  eq(ctx.run("sfxBuffers.size"), 512); eq(ctx.run("sfxBufferBytes"), 2048);
  const requests = ctx.fetchLog.length;
  await ctx.run("loadSfx(599)"); eq(ctx.fetchLog.length, requests);
  await ctx.run("loadSfx(0)"); eq(ctx.fetchLog.length, requests + 1);
});

test("a valid decoded sound larger than the retention budget can be used without evicting the cache", async () => {
  const { ctx, audio } = fixture();
  const small = await ctx.run("loadSfx(1)");
  const large = new BufferFixture(129 * 1024 * 1024);
  audio.decodeAudioData = async () => large;
  eq(await ctx.run("loadSfx(2)"), large);
  ok(!ctx.run("sfxBuffers.has(2)")); eq(ctx.run("sfxBufferBytes"), 4);
  eq(await ctx.run("loadSfx(1)"), small);
});

test("a burst coalesces duplicate IDs and caps actual work without queueing stale effects", async () => {
  const { ctx } = fixture(); const finish = [];
  ctx.setFetch(() => new Promise(resolve => finish.push(resolve)));
  const first = ctx.run("loadSfx(1)"); eq(ctx.run("loadSfx(1)"), first);
  const pending = ctx.run("Array.from({length:100}, (_,i) => loadSfx(i+1))");
  eq(ctx.fetchLog.length, 4); eq(ctx.run("sfxLoads.size"), 4);
  for (const done of finish) done({ ok: true, arrayBuffer: async () => new ArrayBuffer(0) });
  const results = await Promise.all(pending);
  eq(results.filter(Boolean).length, 4); eq(ctx.run("sfxLoads.size"), 0);
});

test("timed-out non-cancellable decoders keep their slots until they really finish", async () => {
  const { ctx, audio } = fixture(); const decodes = [], signals = [];
  audio.decodeAudioData = () => new Promise(resolve => decodes.push(resolve));
  ctx.setFetch((_, init) => { signals.push(init.signal); return { ok: true, arrayBuffer: async () => new ArrayBuffer(0) }; });
  const pending = ctx.run("Array.from({length:4}, (_,i) => loadSfx(i))"); await ctx.flush();
  ctx.advance(5000); deepEq(await Promise.all(pending), [null, null, null, null]);
  ok(signals.every(signal => signal.aborted)); eq(ctx.run("sfxPending.size"), 0);
  ctx.advance(1000); eq(await ctx.run("loadSfx(100)"), null); eq(ctx.fetchLog.length, 4);
  for (const done of decodes) done(new BufferFixture()); await ctx.flush();
  eq(ctx.run("sfxLoads.size"), 0); eq(ctx.run("sfxBuffers.size"), 0);
  audio.decodeAudioData = async () => new BufferFixture();
  ok(await ctx.run("loadSfx(100)")); eq(ctx.fetchLog.length, 5);
});

test("a late expired decode cannot overwrite a successfully retried sound", async () => {
  const { ctx, audio } = fixture(); const decodes = [];
  audio.decodeAudioData = () => new Promise(resolve => decodes.push(resolve));
  const first = ctx.run("loadSfx(1)"); await ctx.flush();
  ctx.advance(6000); eq(await first, null);
  const retry = ctx.run("loadSfx(1)"); await ctx.flush();
  const replacement = new BufferFixture(16); decodes[1](replacement); eq(await retry, replacement);
  decodes[0](new BufferFixture(4)); await ctx.flush();
  eq(await ctx.run("loadSfx(1)"), replacement); eq(ctx.run("sfxBufferBytes"), 16);
});

test("body timeouts abort and a response arriving later is not decoded", async () => {
  const { ctx, audio } = fixture(); let body, decoded = 0, signal;
  audio.decodeAudioData = async () => { decoded++; return new BufferFixture(); };
  ctx.setFetch((_, init) => { signal = init.signal; return { ok: true, arrayBuffer: () => new Promise(resolve => { body = resolve; }) }; });
  const pending = ctx.run("loadSfx(1)"); await ctx.flush();
  ctx.advance(5000); eq(await pending, null); ok(signal.aborted);
  body(new ArrayBuffer(4)); await ctx.flush(); eq(decoded, 0); eq(ctx.run("sfxLoads.size"), 0);
});

test("missing and failed sounds back off with bounded failure metadata", async () => {
  const { ctx, audio } = fixture(); let decoded = 0;
  audio.decodeAudioData = async () => { decoded++; return new BufferFixture(); };
  ctx.setFetch(() => ({ ok: false, status: 404 }));
  for (let id = 0; id < 140; id++) await ctx.run(`loadSfx(${id})`);
  eq(ctx.run("sfxFailures.size"), 128); eq(decoded, 0);
  await ctx.run("loadSfx(139)"); eq(ctx.fetchLog.length, 140);
  ctx.advance(60000); await ctx.run("loadSfx(139)"); eq(ctx.fetchLog.length, 141);
  ctx.setFetch(() => ({ ok: false, status: 503 }));
  await ctx.run("loadSfx(500)"); await ctx.run("loadSfx(500)"); eq(ctx.fetchLog.length, 142);
  ctx.advance(1000); ctx.setFetch(() => ({ ok: true, arrayBuffer: async () => new ArrayBuffer(0) }));
  ok(await ctx.run("loadSfx(500)")); eq(ctx.fetchLog.length, 143);
});

test("oversized declared and streamed bodies never reach the audio decoder", async () => {
  const { ctx, audio } = fixture(); let decoded = 0, cancelled = false, released = false, reads = 0;
  audio.decodeAudioData = async () => { decoded++; return new BufferFixture(); };
  ctx.setFetch(() => ({ ok: true, headers: { get: () => String(33 * 1024 * 1024) }, arrayBuffer: async () => { throw new Error("body must not be read"); } }));
  eq(await ctx.run("loadSfx(1)"), null); eq(decoded, 0);
  const chunk = new Uint8Array(1024 * 1024);
  ctx.setFetch(() => ({ ok: true, headers: { get: () => null }, body: { getReader: () => ({
    read: async () => { reads++; return { done: false, value: chunk }; },
    cancel: async () => { cancelled = true; }, releaseLock: () => { released = true; },
  }) } }));
  eq(await ctx.run("loadSfx(2)"), null); eq(decoded, 0);
  ok(cancelled); ok(released); eq(reads, 33);
});

test("normal streamed audio is assembled exactly and stock-sized long effects are allowed", async () => {
  const { ctx, audio } = fixture(); let bytes, release = false;
  audio.decodeAudioData = async data => { bytes = new Uint8Array(data); return new BufferFixture(); };
  const chunks = [new Uint8Array([1, 2]), new Uint8Array([3, 4, 5])];
  ctx.setFetch(() => ({ ok: true, headers: { get: () => "5" }, body: { getReader: () => ({
    read: async () => chunks.length ? { done: false, value: chunks.shift() } : { done: true },
    releaseLock: () => { release = true; },
  }) } }));
  ok(await ctx.run("loadSfx(1)")); deepEq([...bytes], [1, 2, 3, 4, 5]); ok(release);
  const longWav = new ArrayBuffer(21187628);
  ctx.setFetch(() => ({ ok: true, headers: { get: () => String(longWav.byteLength) }, arrayBuffer: async () => longWav }));
  ok(await ctx.run("loadSfx(2)")); eq(bytes.byteLength, longWav.byteLength);
});

test("late effects warm the cache without playing an outdated event", async () => {
  const { ctx, audio, sources } = fixture(); let decoded;
  audio.decodeAudioData = () => new Promise(resolve => { decoded = resolve; });
  ctx.run("playSfx(1,0,0)"); await ctx.flush(); ctx.advance(1001);
  decoded(new BufferFixture()); await ctx.flush(); eq(sources.length, 0);
  ctx.run("playSfx(1,0,0)"); eq(sources.length, 1); eq(ctx.fetchLog.length, 1);
});

test("inaudible effects and full playback slots do not start more downloads", async () => {
  const { ctx, sources } = fixture();
  ctx.run('scene.player={x:1000,y:1000}; playSfx(1,1022,1000)'); eq(ctx.fetchLog.length, 0);
  await ctx.run("loadSfx(1)");
  ctx.run("for(let i=0;i<8;i++)playSfx(1,0,0);playSfx(2,0,0)");
  eq(sources.length, 8); eq(ctx.fetchLog.length, 1);
  for (const id of [-1, 65536, 1.5]) eq(await ctx.run(`loadSfx(${id})`), null);
  eq(ctx.fetchLog.length, 1);
});
