// The world map (web/js/07-hud.js) — the transform, the anchor and the markers.
//
// It had no coverage at all, and it is mostly coordinate arithmetic, which is
// the kind of code that breaks silently: a sign flip still draws a map, still
// pans, still puts markers somewhere. What it stops doing is agreeing with
// itself, so most of what is asserted here is a ROUND TRIP — a tile projected to
// the canvas and read back has to be the tile you started with, at every scale,
// pan and orientation. That property is what a click on the map depends on.
const { newContext } = require("./harness.js");
const { test, ok, eq, deepEq, near } = require("./run.js");

function wmCtx({ x = 1420, y = 1690 } = {}) {
  const ctx = newContext();
  ctx.mountPage();
  ctx.loadAll();
  const sent = [];
  ctx.setFetch((u, init) => {
    if (String(u) === "/input") { sent.push(String(init && init.body)); return {}; }
    return { status: 404, ok: false, body: null };
  });
  ctx.sent = sent;
  ctx.set("__scene", { player: { serial: 9, x, y, z: 0, name: "Me", equip: [] },
                       mobiles: [], items: [], statics: [], contItems: [],
                       target: { active: 0, flag: 0 } });
  ctx.run("scene = __scene;");
  // The canvas the transform measures itself against. No layout engine here, so
  // the test states the box.
  const cv = ctx.document.getElementById("wmcanvas");
  if (cv) cv.rect = { left: 0, top: 0, width: 600, height: 400 };
  ctx.cv = cv;
  ctx.run("drawWorldmap = () => { __drew = (__drew | 0) + 1; }; __drew = 0;");
  return ctx;
}
const W = 600, H = 400;
const toScreen = (ctx, x, y) => ctx.run(`wmWorldToScreen(${x}, ${y}, ${W}, ${H})`);
const toWorld = (ctx, x, y) => ctx.run(`wmScreenToWorld(${x}, ${y}, ${W}, ${H})`);

test("the canvas centre is the tile the map is anchored on", () => {
  const ctx = wmCtx({ x: 1420, y: 1690 });
  deepEq(toWorld(ctx, W / 2, H / 2), [1420, 1690], "pinned to the player");
  const [sx, sy] = toScreen(ctx, 1420, 1690);
  deepEq([sx, sy], [W / 2, H / 2], "…and that tile projects back to the centre");
});

test("a tile projected to the canvas and read back is the same tile", () => {
  const ctx = wmCtx();
  // Every combination that moves the transform: both orientations, three
  // scales, and a pan that is not a whole number of tiles.
  for (const flip of [false, true]) {
    for (const scale of [0.5, 1, 3]) {
      ctx.run(`wmOpts.flip = ${flip}; wmScale = ${scale}; wmPan = { x: -37, y: 19 };`);
      for (const [wx, wy] of [[1420, 1690], [1400, 1700], [1500, 1600], [0, 0], [6143, 4095]]) {
        const [sx, sy] = toScreen(ctx, wx, wy);
        deepEq(toWorld(ctx, sx, sy), [wx, wy],
               `flip=${flip} scale=${scale} tile ${wx},${wy}`);
      }
    }
  }
});

test("flipped, the map is the game's own iso orientation", () => {
  const ctx = wmCtx({ x: 1000, y: 1000 });
  ctx.run("wmOpts.flip = true; wmScale = 1; wmPan = { x: 0, y: 0 };");
  // +x and +y are the two screen-down diagonals, exactly as a tile is drawn in
  // the world: neither axis is straight down the canvas.
  const [ex, ey] = toScreen(ctx, 1010, 1000);
  const [nx, ny] = toScreen(ctx, 1000, 1010);
  ok(ex > W / 2 && ey > H / 2, `+x goes down-right (${ex},${ey})`);
  ok(nx < W / 2 && ny > H / 2, `+y goes down-left (${nx},${ny})`);
  near(ey - H / 2, ny - H / 2, 1e-9, "…and both drop the same distance");
});

test("unflipped, the map is the plain north-up square the image actually is", () => {
  const ctx = wmCtx({ x: 1000, y: 1000 });
  ctx.run("wmOpts.flip = false; wmScale = 2; wmPan = { x: 0, y: 0 };");
  deepEq(toScreen(ctx, 1010, 1000), [W / 2 + 20, H / 2], "+x is straight right");
  deepEq(toScreen(ctx, 1000, 1010), [W / 2, H / 2 + 20], "+y is straight down");
});

// ── the anchor: pinned to the player, or free ──────────────────────────────

test("free view keeps the view still: the tile under the centre becomes the anchor", () => {
  const ctx = wmCtx({ x: 1420, y: 1690 });
  ctx.run("wmOpts.flip = true; wmScale = 2; wmPan = { x: 60, y: -40 };");
  const before = toWorld(ctx, W / 2, H / 2);
  ctx.run("wmSetFreeView(true)");
  eq(ctx.run("wmPan.x"), 0, "the pan folded into the anchor");
  eq(ctx.run("wmPan.y"), 0, "…in both axes");
  deepEq(toWorld(ctx, W / 2, H / 2), before,
         "and the same tile is still under the centre — the view did not jump");
});

test("leaving free view drops straight back onto the player", () => {
  const ctx = wmCtx({ x: 1420, y: 1690 });
  ctx.run("wmFree = { x: 100, y: 200 }; wmGoto = { x: 100, y: 200 }; wmPan = { x: 9, y: 9 };");
  ctx.run("wmSetFreeView(false)");
  eq(ctx.run("wmFree"), null, "unpinned");
  eq(ctx.run("wmGoto"), null, "…and the goto marker goes with it");
  deepEq(toWorld(ctx, W / 2, H / 2), [1420, 1690], "back on the player");
});

test("setting free view to what it already is does nothing", () => {
  const ctx = wmCtx();
  ctx.run("wmPan = { x: 5, y: 5 }; __drew = 0;");
  ctx.run("wmSetFreeView(false)");                    // already off
  deepEq([ctx.run("wmPan.x"), ctx.run("wmPan.y")], [5, 5], "the pan was not reset");
  eq(ctx.run("__drew"), 0, "and nothing was redrawn");
});

test("the map follows the player while it is pinned", () => {
  const ctx = wmCtx({ x: 1420, y: 1690 });
  deepEq(toWorld(ctx, W / 2, H / 2), [1420, 1690], "here");
  ctx.run("scene.player.x = 1500; scene.player.y = 1600;");
  deepEq(toWorld(ctx, W / 2, H / 2), [1500, 1600], "…and there, with no other state touched");
});

// ── markers ────────────────────────────────────────────────────────────────

test("a marker is placed at the tile under the click, not at the click", () => {
  const ctx = wmCtx({ x: 1420, y: 1690 });
  ctx.run("wmOpts.flip = true; wmScale = 2; wmPan = { x: 0, y: 0 };");
  ctx.answer.prompt = "  Britain bank  ";
  ctx.run(`wmAddMarkerAt(${W / 2 + 40}, ${H / 2 + 20}, ${W}, ${H})`);
  const m = ctx.run("wmMarkers")[0];
  deepEq(toWorld(ctx, W / 2 + 40, H / 2 + 20), [m.x, m.y], "at the tile the click resolved to");
  eq(m.name, "Britain bank", "and the name is trimmed");
});

test("cancelling the name prompt places nothing", () => {
  const ctx = wmCtx();
  ctx.answer.prompt = null;
  ctx.run(`wmAddMarkerAt(300, 200, ${W}, ${H})`);
  eq(ctx.run("wmMarkers.length"), 0, "cancel is not an empty name");
});

test("an empty name is still a marker — it is a pin, not a label", () => {
  const ctx = wmCtx();
  ctx.answer.prompt = "";
  ctx.run(`wmAddMarkerAt(300, 200, ${W}, ${H})`);
  eq(ctx.run("wmMarkers.length"), 1, "placed");
  eq(ctx.run("wmMarkers")[0].name, "", "unlabelled");
});

test("removing takes the nearest marker within reach, and only that one", () => {
  const ctx = wmCtx({ x: 1000, y: 1000 });
  ctx.run("wmOpts.flip = false; wmScale = 1; wmPan = { x: 0, y: 0 };");
  ctx.set("__ms", [{ x: 1000, y: 1000, name: "centre" },
                   { x: 1005, y: 1000, name: "near" },
                   { x: 1200, y: 1200, name: "far" }]);
  ctx.run("wmMarkers = __ms;");
  ctx.run(`wmRemoveMarkerNear(${W / 2 + 5}, ${H / 2}, ${W}, ${H})`);   // on "near"
  deepEq(ctx.run("wmMarkers.map((m) => m.name)"), ["centre", "far"], "only the nearest went");
});

test("a click nowhere near a marker removes nothing", () => {
  const ctx = wmCtx({ x: 1000, y: 1000 });
  ctx.run("wmOpts.flip = false; wmScale = 1; wmPan = { x: 0, y: 0 };");
  ctx.set("__ms", [{ x: 1000, y: 1000, name: "centre" }]);
  ctx.run("wmMarkers = __ms;");
  ctx.run(`wmRemoveMarkerNear(${W / 2 + 40}, ${H / 2 + 40}, ${W}, ${H})`);
  eq(ctx.run("wmMarkers.length"), 1, "40px away is out of the 12px reach");
});

test("the player marker is placed on the player, whatever the map is looking at", () => {
  const ctx = wmCtx({ x: 1420, y: 1690 });
  ctx.run("wmFree = { x: 100, y: 200 };");            // looking somewhere else entirely
  ctx.answer.prompt = "home";
  ctx.run("wmAddMarkerOnPlayer()");
  deepEq([ctx.run("wmMarkers")[0].x, ctx.run("wmMarkers")[0].y], [1420, 1690], "on the player");
});

// ── go to ──────────────────────────────────────────────────────────────────

test("go-to takes a coordinate, unpins the view and marks the destination", () => {
  const ctx = wmCtx({ x: 1420, y: 1690 });
  ctx.answer.prompt = "5290, 1176";
  ctx.run("wmGotoLocation()");
  deepEq([ctx.run("wmFree.x"), ctx.run("wmFree.y")], [5290, 1176], "free view on the destination");
  deepEq([ctx.run("wmGoto.x"), ctx.run("wmGoto.y")], [5290, 1176], "…and a marker showing where");
  deepEq(toWorld(ctx, W / 2, H / 2), [5290, 1176], "which is what the centre now shows");
});

test("go-to accepts a space as well as a comma, and negative coordinates", () => {
  for (const [raw, want] of [["1420 1690", [1420, 1690]], [" 12 , 34 ", [12, 34]],
                             ["-5,-9", [-5, -9]]]) {
    const ctx = wmCtx();
    ctx.answer.prompt = raw;
    ctx.run("wmGotoLocation()");
    deepEq([ctx.run("wmFree.x"), ctx.run("wmFree.y")], want, `"${raw}"`);
  }
});

test("go-to says so rather than jumping somewhere arbitrary on nonsense", () => {
  const ctx = wmCtx({ x: 1420, y: 1690 });
  ctx.answer.prompt = "britain";
  ctx.run("wmGotoLocation()");
  eq(ctx.run("wmFree"), null, "the view did not move");
  ok(ctx.run("localJournal.some((l) => /expected two numbers/i.test(l.text))"),
     "and the player is told what it wanted");
});

test("cancelling go-to leaves the view alone", () => {
  const ctx = wmCtx();
  ctx.run("wmPan = { x: 7, y: 7 };");
  ctx.answer.prompt = null;
  ctx.run("wmGotoLocation()");
  eq(ctx.run("wmFree"), null, "still pinned");
  deepEq([ctx.run("wmPan.x"), ctx.run("wmPan.y")], [7, 7], "and not re-centred");
});
