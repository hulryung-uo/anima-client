const { newContext } = require("./harness.js");
const { test, ok, eq, deepEq, throws } = require("./run.js");
const KEY = "anima.preferences.v1";
function context(values = {}) {
  const ctx = newContext(); ctx.mountPage();
  for (const [k, v] of Object.entries(values)) ctx.localStorage.setItem(k, typeof v === "string" ? v : JSON.stringify(v));
  ctx.loadAll(); return ctx;
}
function backup(values) { return JSON.stringify({format:"anima-preferences",version:1,values}); }
function snapshot(ctx) { return ctx.run("preferenceStorage.source()"); }
function restore(ctx, values, source = snapshot(ctx)) {
  ctx.set("restoreValues", values); ctx.set("restoreSource", source);
  return ctx.run("preferenceStorage.replace(restoreValues, restoreSource)");
}
function persisted(ctx) { return JSON.parse(ctx.localStorage.getItem(KEY)); }

test("invalid option types and ranges cannot reach audio or input and original values remain recoverable", () => {
  const old = JSON.stringify({musicVol:5,containerScale:-100,alwaysRun:"false",sfx:false});
  const ctx = context({"anima.settings":old});
  eq(ctx.run("settings.musicVol"), .3); eq(ctx.run("settings.containerScale"), 100);
  eq(ctx.run("settings.alwaysRun"), false); eq(ctx.run("settings.sfx"), false);
  ok(!ctx.document.getElementById("preference-warning").hidden);
  ctx.run("settings.music=false; saveSettings()");
  eq(ctx.localStorage.getItem("anima.settings"), old); eq(ctx.localStorage.getItem(KEY), null);
  ctx.run("preferenceStorage.recover()");
  eq(persisted(ctx).recovery.source.legacy["anima.settings"], old);
  eq(JSON.parse(persisted(ctx).values["anima.settings"]).music, false);
  eq(ctx.run("preferenceStorage.status().damaged.length"), 0);
});
test("inaccessible storage does not abort renderer boot and pending settings can later be saved", () => {
  const ctx = newContext(); ctx.mountPage();
  const get = ctx.localStorage.getItem;
  ctx.localStorage.getItem = () => { throw new Error("blocked fixture"); };
  ctx.loadAll(); ok(ctx.booted);
  ok(!ctx.document.getElementById("preference-warning").hidden);
  ctx.run("settings.music=false; saveSettings()");
  eq(ctx.run("preferenceStorage.status().pending"), 1);
  ctx.localStorage.getItem = get;
  ok(ctx.run("preferenceStorage.flush()"));
  eq(JSON.parse(persisted(ctx).values["anima.settings"]).music, false);
});
test("legacy migration retains other groups and unknown option fields without applying them", () => {
  const ctx = context({"anima.settings":{music:true,future:{keep:1}},"anima.markers":[{x:100,y:200,name:"Home"}]});
  eq(ctx.run("settings.future"), undefined);
  ctx.run("settings.music=false; saveSettings()");
  deepEq(JSON.parse(persisted(ctx).values["anima.settings"]).future, {keep:1});
  eq(JSON.parse(persisted(ctx).values["anima.markers"])[0].name, "Home");
  eq(JSON.parse(ctx.localStorage.getItem("anima.settings")).music, true, "legacy source is untouched");
});
test("atomic restore leaves the original unchanged if storage is full", () => {
  const ctx = context({"anima.settings":{music:true}}), source = snapshot(ctx);
  ctx.localStorage.setItem = () => { throw new Error("quota fixture"); };
  throws(() => restore(ctx, {"anima.settings":'{"music":false}'}, source), /quota/);
  eq(ctx.localStorage.getItem(KEY), null);
  eq(JSON.parse(ctx.localStorage.getItem("anima.settings")).music, true);
});
test("restored settings can be reviewed and reverted to the automatic previous copy", () => {
  const ctx = context({"anima.settings":{music:true}});
  restore(ctx, {"anima.settings":'{"music":false}'});
  const previous = ctx.run("preferenceStorage.parseBackup(preferenceStorage.previousData())");
  restore(ctx, previous);
  eq(JSON.parse(persisted(ctx).values["anima.settings"]).music, true);
  eq(JSON.parse(persisted(ctx).recovery.source.legacy["anima.settings"]).music, false);
});
test("stale backup review cannot overwrite a later preference change", () => {
  const ctx = context(), source = snapshot(ctx);
  ctx.run("settings.music=false;saveSettings()");
  const before = ctx.localStorage.getItem(KEY);
  throws(() => restore(ctx, {"anima.settings":'{"music":true}'}, source), /changed in another window/);
  eq(ctx.localStorage.getItem(KEY), before);
});
test("backups reject unsupported versions, account data and invalid settings as a whole", () => {
  const ctx = context();
  for (const text of ["{", '{"format":"anima-preferences","version":2,"values":{}}', backup({"anima.launcher.browser.v1":"{}"}), backup({"anima.settings":'{"musicVol":5}'}), backup({"anima.macros":"[null]"})]) {
    ctx.set("text", text); throws(() => ctx.run("preferenceStorage.parseBackup(text)"));
  }
  eq(ctx.localStorage.getItem(KEY), null);
});
test("malformed envelope is preserved exactly during recovery", () => {
  const original = '{"version":'; const ctx = context({[KEY]:original});
  ctx.run("preferenceStorage.recover()");
  eq(ctx.run("preferenceStorage.recoveryData()"), original);
  throws(() => ctx.run("preferenceStorage.previousData()"), /damaged/);
});
test("export excludes account storage and exports the current pending changes", () => {
  const ctx = context({"anima.launcher.browser.v1":"not-renderer-data", "anima.settings":{music:true}});
  ctx.localStorage.setItem = () => { throw new Error("quota fixture"); };
  ctx.run("settings.music=false;saveSettings()");
  const exported = JSON.parse(ctx.run("preferenceStorage.exportData()"));
  eq(JSON.parse(exported.values["anima.settings"]).music, false);
  ok(!Object.hasOwn(exported.values,"anima.launcher.browser.v1"));
  ctx.set("data", JSON.stringify(exported)); ctx.run("preferenceStorage.parseBackup(data)");
});
test("malformed arrays do not crash shortcuts or map UI and a damaged macro is never partly executed", () => {
  const good = {id:"one",key:"F9",actions:[{t:"say",text:"hello"}]};
  const ctx = context({"anima.markers":null,"anima.skillbtns":{},"anima.spellbtns":[null],"anima.macros":[good,{id:"two",key:"F10",actions:[{t:"say",text:"first"},null]}]});
  const before = ctx.fetchLog.length;
  ctx.run("loadMacros();loadSkillButtons();loadSpellButtons()");
  eq(ctx.run("macros.length"), 1); eq(ctx.run("wmMarkers.length"), 0);
  eq(ctx.fetchLog.length, before, "loading never executes a macro");
});
test("a restore preview is explicit, invalid replacement clears it, and live play prevents applying", () => {
  const ctx = context(); ctx.set("text", backup({"anima.markers":"[]"}));
  ctx.run('previewPreferences(text,"test.json")');
  ok(!ctx.document.getElementById("pref-apply").disabled);
  ctx.run("scene={player:{serial:1}};renderPreferenceStatus();applyPreferencePreview()");
  ok(ctx.document.getElementById("pref-apply").disabled); eq(ctx.localStorage.getItem(KEY), null);
  ctx.run('scene=null;previewPreferences("bad","bad.json")');
  ok(ctx.document.getElementById("pref-preview").hidden); ok(ctx.document.getElementById("pref-apply").disabled);
});
test("a late file read cannot replace the most recently chosen backup", async () => {
  const ctx = context(), input = ctx.document.getElementById("pref-file"); let resolve;
  input.files = [{name:"old.json",size:10,text:()=>new Promise(r=>{resolve=r;})}];
  ctx.fire(input,"change");
  input.files = [{name:"new.json",size:10,text:async()=>backup({"anima.markers":"[]"})}];
  ctx.fire(input,"change"); await ctx.flush();
  resolve(backup({"anima.settings":"{}"})); await ctx.flush();
  eq(ctx.document.getElementById("pref-file-name").textContent,"new.json");
});
test("export leaves an explicit save link in the modal and releases it on close", () => {
  const ctx = context();
  ctx.run("openPreferencePanel()");
  ctx.fire(ctx.document.getElementById("pref-export"), "click");
  const link = ctx.document.getElementById("pref-download");
  ok(!link.hidden); eq(link.download, "anima-settings.json"); ok(link.href.startsWith("blob:"));
  ctx.fire(ctx.document.getElementById("pref-close"), "click");
  ok(link.hidden); eq(ctx.run("preferenceDownloadUrl"), null);
});
test("an unreadable previous copy cannot leave another backup armed for restore", () => {
  const ctx = context({[KEY]:"broken"}); ctx.run("preferenceStorage.recover()");
  ctx.set("text", backup({"anima.settings":"{}"})); ctx.run('previewPreferences(text,"chosen.json")');
  ctx.fire(ctx.document.getElementById("pref-previous"), "click");
  ok(ctx.document.getElementById("pref-preview").hidden);
  ok(ctx.document.getElementById("pref-apply").disabled);
});
