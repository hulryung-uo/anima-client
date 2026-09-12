const { newContext } = require("./harness.js");
const { test, ok, eq } = require("./run.js");

function audioFixture() {
  const ctx = newContext(); ctx.mountPage(); ctx.loadAll();
  class BufferFixture { constructor() { this.length = 22050; this.numberOfChannels = 1; } }
  ctx.set("AudioBuffer", BufferFixture);
  const decodes = [], sources = [];
  ctx.set("audioCtx", {
    state: "running",
    decodeAudioData: () => new Promise(resolve => decodes.push(() => resolve(new BufferFixture()))),
    createBufferSource: () => {
      const source = { connect() {}, start() { this.started = true; }, stop() { this.stopped = true; this.onended?.(); } };
      sources.push(source); return source;
    },
  });
  ctx.setFetch(() => ({ ok: true, arrayBuffer: async () => new ArrayBuffer(0) }));
  ctx.run('scene = {sessionId:"one"}; settings.sfx = true;');
  return { ctx, decodes, sources };
}

test("unmuting before a pending decode finishes does not replay the pre-mute effect", async () => {
  const { ctx, decodes, sources } = audioFixture();
  ctx.run("playSfx(1, 0, 0)"); await ctx.flush(); eq(decodes.length, 1);
  ctx.run("toggleMute(); toggleMute()"); ok(!ctx.run("audioMuted"));
  decodes[0](); await ctx.flush(); eq(sources.length, 0);
  ctx.run("playSfx(1, 0, 0)"); eq(sources.length, 1); ok(sources[0].started);
  eq(ctx.fetchLog.filter(url => url.startsWith("sound/")).length, 1, "a newly requested effect can reuse the decoded asset");
});

test("turning off effects stops active nodes and pending effects remain cancelled after re-enabling", async () => {
  const { ctx, decodes, sources } = audioFixture();
  ctx.run("playSfx(1, 0, 0)"); await ctx.flush(); decodes[0](); await ctx.flush();
  eq(ctx.run("activeSfx.size"), 1);
  ctx.run("playSfx(2, 0, 0)"); await ctx.flush();
  ctx.run("settings.sfx = false; applyAudioSettings(); playSfx(1, 0, 0)");
  ok(sources[0].stopped); eq(ctx.run("activeSfx.size"), 0); eq(sources.length, 1);
  ctx.run("settings.sfx = true; applyAudioSettings()");
  decodes[1](); await ctx.flush(); eq(sources.length, 1);
});

test("transport loss cancels a pending sound even when transport returns before decode", async () => {
  const { ctx, decodes, sources } = audioFixture();
  ctx.run("playSfx(1, 0, 0)"); await ctx.flush();
  ctx.run("setSceneTransport(false); setSceneTransport(true)");
  decodes[0](); await ctx.flush(); eq(sources.length, 0);
  ctx.run("playSfx(1, 0, 0)"); eq(sources.length, 1);
});

test("a world transition stops active effects and blocks a late sound decode", async () => {
  const { ctx, decodes, sources } = audioFixture();
  ctx.run("playSfx(1, 0, 0)"); await ctx.flush(); decodes[0](); await ctx.flush();
  ctx.run("playSfx(2, 0, 0)"); await ctx.flush();
  ctx.run("reloadForSessionChange()");
  ok(sources[0].stopped); eq(ctx.run("activeSfx.size"), 0);
  decodes[1](); await ctx.flush(); eq(sources.length, 1);
});

test("a delayed effect cannot borrow a replacement world's position", async () => {
  const { ctx, decodes, sources } = audioFixture();
  ctx.run("playSfx(1, 1000, 2000)"); await ctx.flush();
  ctx.run('scene = {sessionId:"two", player:{x:1000,y:2000}}');
  decodes[0](); await ctx.flush(); eq(sources.length, 0);
});
