const { newContext } = require("./harness.js");
const { test, ok, eq } = require("./run.js");
const emptyProfiles = { servers: [], accounts: [], passwords: false, persistent: true };
function fixture(handler) {
  const ctx = newContext(); ctx.mountPage(); ctx.loadAll();
  ctx.set("Option", function (text, value) {
    const el = ctx.document.createElement("option"); el.textContent = text; el.value = value; return el;
  });
  const calls = [];
  ctx.setFetch((url, init) => {
    calls.push({ url, init });
    return handler ? handler(url, init) : url === "launcher" ? emptyProfiles : {};
  });
  ctx.run('showLogin("login", "")');
  return { ctx, calls, el: id => ctx.document.getElementById(id) };
}

test("Connect stays disabled while profiles load or fail, and retry restores it", async () => {
  let resolve, retry = false;
  const pending = new Promise(r => { resolve = r; });
  const { ctx, calls, el } = fixture(url => url === "launcher" ? retry ? emptyProfiles : pending : {});
  ok(el("lg-go").disabled);
  ctx.fire(el("lg-user"), "keydown", { code: "Enter" }); await ctx.flush();
  eq(calls.filter(c => c.url === "login").length, 0);
  resolve({ ok: false, status: 500, json: async () => ({ error: "Profiles need recovery", recoverable: true }) });
  await ctx.flush(); ok(el("lg-go").disabled);
  ctx.run('showLogin("login", "")'); ok(el("lg-go").disabled, "scene refresh cannot enable an unavailable account");
  retry = true; el("lg-retry-profiles").click(); await ctx.flush();
  ok(!el("lg-go").disabled, "successful retry enables Connect without another scene poll");
});

test("profile writes disable Connect until they finish, including across scene refresh", async () => {
  const { ctx, el } = fixture(); await ctx.flush(); ok(!el("lg-go").disabled);
  let resolve; ctx.set("saveWait", new Promise(r => { resolve = r; }));
  const action = ctx.run('launcherAction(() => saveWait, "Saved")');
  ok(el("lg-go").disabled);
  ctx.run('showLogin("login", "")'); ok(el("lg-go").disabled);
  resolve(); await action; ok(!el("lg-go").disabled);
});

test("finishing credential storage cannot unlock a pending login request", async () => {
  let rejectLogin;
  const { ctx, el, calls } = fixture(url => url === "launcher" ? emptyProfiles : url === "login"
    ? new Promise((_, reject) => { rejectLogin = reject; }) : {});
  await ctx.flush();
  ctx.run(`launcherPrepareLogin = async () => {
    launcherSetBusy(true); await Promise.resolve(); launcherSetBusy(false);
    return {host:"127.0.0.1",port:25111,username:"fixture",password:""};
  }`);
  el("lg-go").click(); await ctx.flush();
  ok(el("lg-go").disabled, "credential save completion does not release submit's lock");
  ctx.run('showLogin("login", "")');
  ok(el("lg-go").disabled, "an unchanged scene cannot release submit's lock");
  ctx.fire(el("lg-pass"), "keydown", { code: "Enter" }); await ctx.flush();
  eq(calls.filter(c => c.url === "login").length, 1);
  rejectLogin(new Error("Fixture transport failure")); await ctx.flush();
  ok(!el("lg-go").disabled, "request failure allows retry");
});

test("profile recovery required during credential save leaves Connect disabled", async () => {
  const { ctx, el, calls } = fixture(); await ctx.flush();
  ctx.run(`launcherPrepareLogin = async () => {
    launcherSetBusy(true);
    try { launcherReady = false; throw new Error("Profiles need recovery"); }
    finally { launcherSetBusy(false); }
  }`);
  el("lg-go").click(); await ctx.flush();
  ok(el("lg-go").disabled); eq(calls.filter(c => c.url === "login").length, 0);
});

test("unavailable profile storage does not disable an existing character session", async () => {
  const { ctx, el, calls } = fixture(); await ctx.flush();
  ctx.run('launcherReady = false; showLogin("characters", "", [{index:0,name:"Fixture"}], 5, [], null, {choice_id:"session"})');
  ok(!el("lg-go").disabled);
  ctx.run("launcherSetBusy(false)"); ok(!el("lg-go").disabled);
  el("lg-go").click(); await ctx.flush();
  eq(calls.filter(c => c.url === "character").length, 1);
});
