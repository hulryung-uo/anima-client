const { newContext } = require("./harness.js");
const { test, ok, eq, deepEq, throws } = require("./run.js");
const GEOMETRY = "anima.winGeom", ENVELOPE = "anima.preferences.v1";
function context(values = {}, options = {}) {
  const ctx = newContext(options); ctx.mountPage();
  for (const [key, value] of Object.entries(values)) ctx.localStorage.setItem(key, typeof value === "string" ? value : JSON.stringify(value));
  ctx.loadAll(); return ctx;
}
function frame(ctx, cls = "fixture-win", resizable = true) {
  ctx.set("fixtureClass", cls); ctx.set("fixtureResizable", resizable);
  return ctx.run('makeWindowFrame({cls:fixtureClass,title:"Fixture",resizable:fixtureResizable,pos:{left:30,top:40}})');
}
function saved(ctx) { return JSON.parse(JSON.parse(ctx.localStorage.getItem(ENVELOPE)).values[GEOMETRY]); }

test("malformed saved window geometry cannot prevent dialogs from opening", () => {
  for (const original of ["null", "[]", "17", '"bad"', '{".fixture-win":null}', '{".fixture-win":{"left":"far","top":10,"w":-1,"h":100}}']) {
    const ctx = context({ [GEOMETRY]: original });
    const win = frame(ctx);
    eq(win.el.style.left, "30px"); eq(win.el.style.top, "40px");
    ok(!win.body.style.width); ok(!ctx.document.getElementById("preference-warning").hidden);
    eq(ctx.localStorage.getItem(GEOMETRY), original, "bad source is not overwritten by opening a window");
    ctx.run("preferenceStorage.recover()");
    eq(JSON.parse(ctx.localStorage.getItem(ENVELOPE)).recovery.source.legacy[GEOMETRY], original);
  }
});

test("window positions and sizes participate in settings backup and restore", () => {
  const geometry = { ".fixture-win": { left: 75, top: 90, w: 240, h: 120 } };
  const ctx = context({ [GEOMETRY]: geometry });
  const backup = JSON.parse(ctx.run("preferenceStorage.exportData()"));
  deepEq(JSON.parse(backup.values[GEOMETRY]), geometry);
  const destination = context(); destination.set("geometryBackup", JSON.stringify(backup));
  destination.run("preferenceStorage.replace(preferenceStorage.parseBackup(geometryBackup), preferenceStorage.source())");
  const win = frame(destination);
  eq(win.el.style.left, "75px"); eq(win.el.style.top, "90px");
  eq(win.body.style.width, "240px"); eq(win.body.style.height, "120px");
});

test("geometry added after an older settings migration is adopted without losing other groups", () => {
  const old = { version: 1, values: { "anima.settings": '{"music":false}' } };
  const geometry = { ".fixture-win": { left: 80, top: 100 } };
  const ctx = context({ [ENVELOPE]: old, [GEOMETRY]: geometry });
  eq(frame(ctx).el.style.left, "80px");
  ctx.run('saveWinGeom(".fixture-win", {w:300,h:200})');
  const record = JSON.parse(ctx.localStorage.getItem(ENVELOPE));
  eq(JSON.parse(record.values["anima.settings"]).music, false);
  deepEq(saved(ctx)[".fixture-win"], { left: 80, top: 100, w: 300, h: 200 });
  ok(record.migratedKeys.includes(GEOMETRY));
  deepEq(JSON.parse(ctx.localStorage.getItem(GEOMETRY)), geometry);
});

test("restoring a backup without geometry does not resurrect its untouched legacy source", () => {
  const geometry = { ".fixture-win": { left: 150, top: 160 } };
  const ctx = context({ [GEOMETRY]: geometry });
  ctx.run("preferenceStorage.replace({}, preferenceStorage.source())");
  const restarted = context({ [GEOMETRY]: geometry, [ENVELOPE]: ctx.localStorage.getItem(ENVELOPE) });
  eq(frame(restarted).el.style.left, "30px");
  const previous = restarted.run("preferenceStorage.parseBackup(preferenceStorage.previousData())");
  deepEq(JSON.parse(previous[GEOMETRY]), geometry, "the explicit previous copy still restores it");
});

test("geometry saves merge another window's newer positions and retain this window's size", () => {
  const initial = { ".fixture-win": { left: 15, top: 20, w: 100, h: 80 } };
  const first = context({ [GEOMETRY]: initial }), second = context({ [GEOMETRY]: initial });
  first.run('saveWinGeom(".other-win", {left:300,top:210})');
  second.localStorage.setItem(ENVELOPE, first.localStorage.getItem(ENVELOPE));
  second.run('saveWinGeom(".fixture-win", {left:50,top:60})');
  deepEq(saved(second)[".other-win"], { left: 300, top: 210 });
  deepEq(saved(second)[".fixture-win"], { left: 50, top: 60, w: 100, h: 80 });
});

test("failed geometry saves remain usable and exportable while the stored original stays intact", () => {
  const original = JSON.stringify({ ".fixture-win": { left: 50, top: 60 } });
  const ctx = context({ [GEOMETRY]: original });
  ctx.localStorage.setItem = () => { throw new Error("Quota exceeded"); };
  ctx.run('saveWinGeom(".fixture-win", {left:200,top:220})');
  eq(frame(ctx).el.style.left, "200px");
  eq(ctx.localStorage.getItem(GEOMETRY), original); eq(ctx.localStorage.getItem(ENVELOPE), null);
  const exported = JSON.parse(ctx.run("preferenceStorage.exportData()"));
  eq(JSON.parse(exported.values[GEOMETRY])[".fixture-win"].top, 220);
  ok(ctx.run("preferenceStorage.status().pending") > 0);
});

test("a restored offscreen window keeps a reachable title bar on a smaller display", () => {
  const ctx = context({ [GEOMETRY]: { ".fixture-win": { left: 5000, top: 4000 } } }, { width: 600, height: 400 });
  const win = frame(ctx);
  ok(parseFloat(win.el.style.left) <= 560); ok(parseFloat(win.el.style.top) <= 376);
});

test("invalid geometry in a backup is rejected before any preference replacement", () => {
  const ctx = context();
  ctx.set("badGeometryBackup", JSON.stringify({ format: "anima-preferences", version: 1, values: { [GEOMETRY]: '{".fixture-win":{"left":5,"top":null}}' } }));
  throws(() => ctx.run("preferenceStorage.parseBackup(badGeometryBackup)"), /invalid values/);
  eq(ctx.localStorage.getItem(ENVELOPE), null);
});
