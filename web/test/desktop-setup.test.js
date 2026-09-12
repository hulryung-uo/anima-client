const fs = require("node:fs");
const path = require("node:path");
const { newContext } = require("./harness.js");
const { test, ok, eq } = require("./run.js");
const root = path.resolve(__dirname, "../../crates/anima-desktop/frontend-dist");
const html = fs.readFileSync(path.join(root, "index.html"), "utf8").match(/<body>([\s\S]*)<\/body>/)[1];
const script = fs.readFileSync(path.join(root, "setup.js"), "utf8");
function ready(folder = "/uo") {
  return { report: {path:folder,ready:true,checks:[]}, saved_path:"",active_path:"",config_path:"/config.json",config_error:null,recoverable:false,notice:"",startup_error:"",initialising:false,starting:false,running:false };
}
async function setup(initial = ready()) {
  const ctx = newContext(); ctx.mount(html);
  ctx.set("nativeInvoke", async () => initial);
  ctx.run("window.__TAURI__ = {core:{invoke:(...args) => nativeInvoke(...args)}}");
  ctx.run(script); await ctx.flush();
  return ctx;
}
test("desktop setup cannot save a typed folder until that exact path is checked", async () => {
  const ctx = await setup(); const folder = ctx.document.getElementById("folder"), apply = ctx.document.getElementById("apply");
  ok(!apply.disabled);
  folder.value = "/not-checked"; ctx.fire(folder, "input"); ok(apply.disabled);
  ctx.set("nativeInvoke", async () => ready("/not-checked"));
  await ctx.run('call("setup_check", {path:"/not-checked"})'); ok(!apply.disabled);
});
test("late desktop status cannot undo a newer folder selection", async () => {
  const ctx = await setup(); let old;
  ctx.set("nativeInvoke", command => command === "setup_status" ? new Promise(resolve => { old = resolve; }) : Promise.resolve(ready("/new")));
  const polling = ctx.run("refresh()"); await ctx.flush();
  await ctx.run('call("setup_choose")');
  old(ready("/old")); await polling;
  eq(ctx.document.getElementById("folder").value, "/new");
});
test("choosing the original folder replaces an unverified edit", async () => {
  const ctx = await setup(); const folder = ctx.document.getElementById("folder");
  folder.value = "/mistyped"; ctx.fire(folder, "input");
  await ctx.run('call("setup_choose")'); eq(folder.value, "/uo");
  ok(!ctx.document.getElementById("apply").disabled);
});
test("valid files cannot bypass corrupt settings or an active launch", async () => {
  const cfg = ready(); cfg.config_error = "Recovery required"; cfg.recoverable = true;
  const ctx = await setup(cfg);
  ok(ctx.document.getElementById("apply").disabled); ok(!ctx.document.getElementById("recover").hidden);
  ctx.set("next", {...ready(),starting:true}); ctx.run("render(next)");
  ok(ctx.document.getElementById("apply").disabled); ok(ctx.document.getElementById("browse").disabled);
});
