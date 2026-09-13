const { newContext } = require("./harness.js");
const { test, ok, eq, includes } = require("./run.js");

function pollContext() {
  const ctx = newContext(); ctx.mountPage();
  ctx.load("00-state.js", "04-connection.js", "05-poll.js");
  ctx.set("showLogin", () => {}); ctx.set("setStatus", () => {}); ctx.set("diag", {});
  return ctx;
}
test("scene polling never overlaps while a previous body is pending", async () => {
  const ctx = pollContext(); let finish;
  ctx.setFetch(() => ({ ok: true, json: () => new Promise(resolve => { finish = resolve; }) }));
  const first = ctx.run("poll()"); await ctx.flush();
  await ctx.run("poll()"); await ctx.run("poll(true)");
  eq(ctx.fetchLog.length, 1);
  finish({ auth: "login", msg: "Newest" }); await first;
  eq(ctx.run("scene.msg"), "Newest"); ok(!ctx.run("scenePollPending"));
});
test("timed-out scene bodies cannot overwrite a recovered connection", async () => {
  const ctx = pollContext(); let late, signal;
  ctx.setFetch((_, init) => { signal = init.signal; return { ok: true, json: () => new Promise(resolve => { late = resolve; }) }; });
  const first = ctx.run("poll()"); await ctx.flush();
  ctx.advance(5000); await first;
  ok(signal.aborted); ok(!ctx.run("sceneTransportAvailable"));
  ok(!ctx.document.getElementById("client-connection").hidden);
  ctx.setFetch(() => ({ ok: true, json: async () => ({ auth: "login", msg: "Recovered" }) }));
  await ctx.run("poll(true)");
  late({ auth: "error", msg: "Stale" }); await ctx.flush();
  eq(ctx.run("scene.msg"), "Recovered"); ok(ctx.run("sceneTransportAvailable"));
  ok(ctx.document.getElementById("client-connection").hidden);
});
test("failed scene requests back off instead of hammering the stopped client", async () => {
  const ctx = pollContext(); ctx.setFetch(() => { throw new Error("offline"); });
  await ctx.run("poll()");
  for (let i = 0; i < 5; i++) { ctx.advance(150); await ctx.run("poll()"); }
  eq(ctx.fetchLog.length, 1);
  ctx.advance(250); await ctx.run("poll()"); eq(ctx.fetchLog.length, 2);
});
test("disconnect stops macros and movement without replaying them on recovery", () => {
  const ctx = newContext(); ctx.mountPage(); ctx.loadAll(); ctx.setFetch(() => ({ ok: true })); ctx.fetchLog.length = 0;
  ctx.run('held.add(0); macroRun = {actions:[], i:0}; setSceneTransport(false)');
  eq(ctx.run("held.size"), 0); eq(ctx.run("macroRun"), null);
  ctx.run('sendInput("say:must-not-send"); held.add(2)');
  eq(ctx.fetchLog.length, 0); eq(ctx.run("activeMove()"), null);
  ctx.run("setSceneTransport(true)"); eq(ctx.run("held.size"), 0);
});
test("connection notice pauses keyboard access and restores the previous field", () => {
  const ctx = pollContext(), login = ctx.document.getElementById("login"), host = ctx.document.getElementById("lg-host");
  host.focus();
  ctx.run("setSceneTransport(false)");
  ok(login.inert); eq(ctx.document.activeElement.id, "client-connection-retry");
  ctx.run("setSceneTransport(false); setSceneTransport(true)");
  ok(!login.inert); eq(ctx.document.activeElement, host);
});
test("late cancellation errors cannot alter a newer login attempt", async () => {
  const ctx = pollContext(); let reject, submitted;
  ctx.setFetch((_, init) => { submitted = JSON.parse(init.body); return new Promise((resolve, no) => { reject = no; }); });
  ctx.run('updateLoginConnection("connecting", {attempt_id:7, cancellable:true})');
  const pending = ctx.run("cancelLoginConnection()");
  eq(submitted.attempt_id, 7);
  ctx.run('updateLoginConnection("connecting", {attempt_id:8, cancellable:true})');
  reject(new Error("old failure")); await pending;
  eq(ctx.run("loginConnectionId"), 8);
  eq(ctx.document.getElementById("lg-msg").textContent, "");
  ok(!ctx.document.getElementById("lg-cancel-connection").disabled);
});

function wasmContext() {
  const ctx = newContext(); ctx.mountPage(); ctx.loadAll();
  ctx.run(`wasmEnsure = async () => {};
    WasmClientCtor = class {
      constructor() { this.feeds = 0; this.list = {}; }
      take_outbox() { return new Uint8Array(); }
      feed() { this.feeds++; }
      login_error() { return ""; }
      observation_json() { return "{}"; }
      character_list_json() { return JSON.stringify(this.list); }
    };`);
  return ctx;
}
test("WASM close before open settles login and permits another connection", async () => {
  const ctx = wasmContext();
  const failed = ctx.run('wasmConnect("fixture", "fixture")').then(() => "ok", e => e.message);
  await ctx.flush(); ctx.sockets[0].onclose();
  includes(await failed, "Connection lost"); eq(ctx.run("wasmClient"), null);
  const next = ctx.run('wasmConnect("fixture", "fixture")'); await ctx.flush();
  ctx.sockets[1].onopen(); await next; ok(ctx.run("wasmClient !== null"));
});
test("WASM ignores callbacks belonging to a replaced socket", async () => {
  const ctx = wasmContext();
  const first = ctx.run('wasmConnect("one", "fixture")'); await ctx.flush();
  const old = ctx.sockets[0], oldMessage = old.onmessage, oldClose = old.onclose;
  old.onopen(); await first;
  const second = ctx.run('wasmConnect("two", "fixture")'); await ctx.flush();
  ctx.sockets[1].onopen(); await second;
  oldMessage({ data: new Uint8Array([0x82, 3]).buffer }); oldClose();
  eq(ctx.run("wasmClient.feeds"), 0); eq(ctx.run("wasmConnectionError"), ""); ok(old.closed);
});
test("WASM handshake timeout closes transport but human character choice has no deadline", async () => {
  const ctx = wasmContext();
  const first = ctx.run('wasmConnect("one", "fixture")').then(() => "ok", e => e.message);
  await ctx.flush(); ctx.advance(20000);
  includes(await first, "too long"); ok(ctx.sockets[0].closed);
  const second = ctx.run('wasmConnect("two", "fixture")'); await ctx.flush();
  ctx.sockets[1].onopen(); await second;
  ctx.run('wasmClient.list = {slots:[], slotCount:5}');
  eq((await ctx.run("wasmPollScene()")).auth, "characters");
  ctx.advance(120000); ok(ctx.run("wasmClient !== null")); ok(!ctx.sockets[1].closed);
});

test("WASM layout identity captures the connected relay and account and clears on disconnect", async () => {
  const ctx = wasmContext();
  ctx.document.getElementById("lg-relay").value = "ws://fixture.invalid:2595/relay?target=2";
  const connecting = ctx.run('wasmConnect("layout-account", "not-in-layout")');
  await ctx.flush(); ctx.sockets[0].onopen(); await connecting;
  const identity = ctx.run("wasmLayoutIdentity");
  eq(identity, JSON.stringify(["relay-v1", "ws://fixture.invalid:2595/relay?target=2", "layout-account"]));
  ctx.document.getElementById("lg-relay").value = "ws://other.invalid/relay";
  eq(ctx.run("wasmLayoutIdentity"), identity, "editing the form does not rebind the live character");
  ok(!identity.includes("not-in-layout"));
  eq(ctx.run('wasmMergeScene({player:{serial:42},mobiles:[],items:[]}).layoutIdentity'), identity);
  ctx.run("wasmDisconnect()"); eq(ctx.run("wasmLayoutIdentity"), null);
});
