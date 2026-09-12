const { newContext } = require("./harness.js");
const { test, ok, eq, deepEq, includes } = require("./run.js");
const clone = value => JSON.parse(JSON.stringify(value));
function profiles() {
  return { persistent: true, passwords: true, servers: [
    { id: "one", name: "Home shard", host: "127.0.0.1", port: 2594, shard: 0, notes: "Friends", cache: { checked_at: 1000, reachable: true, latency_ms: 8, clients: 4, uptime_hours: 9, details_at: 1000 } },
    { id: "two", name: "Other world", host: "example.test", port: 2593, shard: 0, notes: "", cache: null },
  ], accounts: [
    { id: "a", server_id: "one", username: "player", label: "Main", remember_password: true, characters: [{ index: 2, name: "Aria" }], last_used: 1000 },
    { id: "b", server_id: "two", username: "crafter", label: "Crafter", remember_password: false, characters: [], last_used: null },
  ] };
}
async function setup() {
  const ctx = newContext(); ctx.mountPage(); ctx.load("00-state.js", "04-launcher.js");
  const state = profiles(), calls = [];
  ctx.localStorage.setItem("anima.launcher.selection.v1", JSON.stringify({ server: "one", accounts: { one: "a", two: "b" } }));
  ctx.setFetch((url, init) => {
    calls.push({ url, init });
    if (init.body) {
      const body = JSON.parse(init.body);
      if (body.op === "save_server") {
        const next = { ...body }; delete next.op;
        const index = state.servers.findIndex(s => s.id === body.id);
        if (index >= 0) state.servers[index] = next; else state.servers.push(next);
      }
      if (body.op === "save_account") {
        const next = { id: body.id, server_id: body.server_id, username: body.username, label: body.label, remember_password: body.remember_password, characters: [], last_used: null };
        const index = state.accounts.findIndex(a => a.id === body.id);
        if (index >= 0) state.accounts[index] = next; else state.accounts.push(next);
      }
    }
    return { ok: true, json: async () => clone(state) };
  });
  ctx.run("initLauncher()"); await ctx.run("launcherInitPromise");
  return { ctx, state, calls, el: id => ctx.document.getElementById(id) };
}
test("server switching scopes accounts and clears the password field", async () => {
  const { ctx, el } = await setup();
  eq(el("lg-user").value, "player"); ok(el("lg-pass").placeholder.includes("Saved securely"));
  el("lg-pass").value = "unsubmitted-secret";
  ctx.run('launcherSelectServer("two")');
  eq(el("lg-user").value, "crafter"); eq(el("lg-pass").value, "");
  eq(el("lg-account-list").children.length, 2, "only New account plus this server's account");
  eq(el("lg-save-password").checked, false);
  ctx.run('launcherSelectServer("one")');
  includes(el("lg-cached-characters").textContent, "Aria");
  includes(el("lg-info-status").textContent, "last check");
});
test("connecting uses a saved account reference without reading its password", async () => {
  const { ctx, calls, el } = await setup();
  const login = await ctx.run("launcherPrepareLogin()");
  eq(login.account_id, "a"); eq(login.password, "");
  const writes = calls.filter(c => c.init.body).map(c => JSON.parse(c.init.body));
  deepEq(writes.map(c => c.op), ["save_server", "save_account"]);
  eq(writes[1].remember_password, true);
  eq(el("lg-pass").value, "");
  ok(calls.every(c => c.init.headers["X-Anima-Launcher"] === "1"));
  ok(!calls.some(c => /password|secret/.test(c.url)), "no password retrieval endpoint");
});
test("typing a saved password sends it only to the native save/login path", async () => {
  const { ctx, calls, el } = await setup(); el("lg-pass").value = "temporary-secret";
  const login = await ctx.run("launcherPrepareLogin()");
  eq(login.password, "temporary-secret");
  eq(JSON.parse(calls.at(-1).init.body).password, "temporary-secret");
  for (let i = 0; i < ctx.localStorage.length; i++) ok(!ctx.localStorage.getItem(ctx.localStorage.key(i)).includes("temporary-secret"));
});
test("storage errors preserve the draft and do not announce a saved account", async () => {
  const { ctx, el } = await setup(); el("lg-account-label").value = "My new label"; el("lg-pass").value = "keep-this";
  ctx.setFetch(() => ({ ok: false, status: 400, json: async () => ({ error: "Vault locked" }) }));
  await ctx.run('launcherAction(launcherSaveAccount, "Account saved.")');
  eq(el("lg-profile-msg").textContent, "Vault locked"); eq(el("lg-pass").value, "keep-this");
  eq(el("lg-account-label").value, "My new label"); ok(!ctx.run("launcherBusy()"));
});
test("changing a saved endpoint needs confirmation before any password operation", async () => {
  const { ctx, calls, el } = await setup(); const before = calls.length;
  el("lg-host").value = "different.test"; ctx.answer.confirm = false;
  let failed = false;
  try { await ctx.run("launcherPrepareLogin()"); } catch (_) { failed = true; }
  ok(failed); eq(calls.length, before, "no save or password request after cancel");
});
test("character selection clears the transient password and displays the next stage", async () => {
  const { ctx, el } = await setup(); el("lg-pass").value = "discard";
  ctx.run('launcherOnAuth("characters", [{index:2,name:"Aria"}])'); await ctx.flush();
  eq(el("lg-pass").value, ""); ok(el("lg-shell").classList.contains("character-stage"));
  ctx.run('launcherOnAuth("login", [])'); ok(!el("lg-shell").classList.contains("character-stage"));
});
test("browser profile serialization excludes a supplied password", () => {
  const ctx = newContext(); ctx.mountPage(); ctx.load("00-state.js", "04-launcher.js");
  ctx.run('launcherData = {servers:[],accounts:[],passwords:false,persistent:true}');
  ctx.run('launcherData = launcherBrowserCommand({op:"save_server",id:"s",name:"World",host:"localhost",port:2594,shard:0,notes:""})');
  ctx.run('launcherData = launcherBrowserCommand({op:"save_account",id:"a",server_id:"s",label:"Main",username:"player",password:"never-persist",remember_password:false})');
  ok(!ctx.localStorage.getItem("anima.launcher.browser.v1").includes("never-persist"));
  eq(ctx.run("launcherData.accounts.length"), 1);
});
