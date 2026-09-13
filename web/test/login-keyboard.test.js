const { newContext } = require("./harness.js");
const { test, ok, eq, deepEq } = require("./run.js");

function fixture() {
  const ctx = newContext(); ctx.mountPage(); ctx.loadAll();
  ctx.set("initLauncher", () => {}); ctx.set("launcherOnAuth", () => {});
  ctx.set("launcherReady", true);
  ctx.set("Option", function (text, value) {
    const el = ctx.document.createElement("option"); el.textContent = text; el.value = value; return el;
  });
  const requests = [], prepared = [];
  const credentials = { host: "127.0.0.1", port: 25111, username: "qa-crafter", password: "" };
  ctx.set("launcherPrepareLogin", async () => { prepared.push(true); return credentials; });
  ctx.setFetch((url, init) => { requests.push({ url, init }); return {}; });
  ctx.run('showLogin("login", "")');
  requests.length = 0; // Ignore the wizard's initial professions.json load.
  const enter = (element, extra = {}) => {
    const event = ctx.event("keydown", { code: "Enter", key: "Enter", bubbles: true, ...extra });
    element.dispatchEvent(event); return event;
  };
  return { ctx, requests, prepared, credentials, enter, el: id => ctx.document.getElementById(id) };
}

test("confirming a saved account or another native select never submits login", async () => {
  const { ctx, enter, requests, prepared } = fixture();
  for (const select of ctx.document.querySelectorAll("#login select")) {
    const event = enter(select); await ctx.flush();
    ok(!event.defaultPrevented, "the native menu retains Enter behavior");
  }
  eq(prepared.length, 0, "selecting does not save profiles or prepare credentials");
  eq(requests.length, 0, "selecting does not contact the server");
});

test("Enter on the worlds file picker or password checkbox does not connect", async () => {
  const { ctx, enter, el, requests, prepared } = fixture();
  for (const id of ["lg-worlds-file", "lg-save-password"]) {
    ok(!enter(el(id)).defaultPrevented); await ctx.flush();
  }
  eq(prepared.length, 0); eq(requests.length, 0);
});

test("a deliberate Enter in credentials still submits exactly one login", async () => {
  const { ctx, enter, el, requests, prepared, credentials } = fixture();
  ok(enter(el("lg-pass")).defaultPrevented); await ctx.flush();
  eq(prepared.length, 1); eq(requests.length, 1); eq(requests[0].url, "login");
  deepEq(JSON.parse(requests[0].init.body), { ...credentials, interactive: true, character_slot: null, create: null });
});

test("IME confirmation and held Enter do not submit credentials", async () => {
  const { ctx, enter, el, requests, prepared } = fixture();
  ok(!enter(el("lg-user"), { isComposing: true }).defaultPrevented);
  ok(!enter(el("lg-pass"), { repeat: true }).defaultPrevented);
  await ctx.flush(); eq(prepared.length, 0); eq(requests.length, 0);
});
