// Walking beats a server-sent pose.
//
// ClassicUO drops the animation the moment it evaluates a step —
// `if (AnimationFromServer) SetAnimation(0xFF)` in `Mobile.ProcessSteps`
// (:704-709) — so a mobile that is struck or swings and then walks switches to
// its walk cycle instantly instead of gliding across the ground still cycling
// the swing frames. `st.act` is only ever set from 0x6E/0xE2, which is exactly
// what `AnimationFromServer` marks there.
//
// This is here because the live check could not pin it. Driving a real shard,
// `st.animMoving = mv` and the cancel that follows it happen in the SAME frame,
// so a sampler can never catch a frame that is both posed and moving — the
// evidence was only circumstantial (a bow held 2736 ms standing still but
// 144 ms while walking, with movement live on the very next sample). Here the
// clock is the test's, so the overlap is exact instead of lucky.
const { newContext } = require("./harness.js");
const { test, ok, eq } = require("./run.js");

function moveCtx() {
  const ctx = newContext();
  ctx.load("dialogs.js", "00-state.js", "01-audio.js", "02-textures.js", "03-world.js",
           "04-boot.js", "05-poll.js", "06-movement.js", "07-hud.js", "08-overlays.js",
           "09-gumps.js", "10-housing.js", "11-dragdrop.js", "12-input.js");
  ctx.run(`
    world = new PIXI.Container(); mobs = new PIXI.Container();
    entLayer = new PIXI.Graphics(); barLayer = new PIXI.Container();
    overLayer = new PIXI.Container();
    app = { canvas: { clientWidth: 800, clientHeight: 600 },
            renderer: { width: 800, height: 600 },
            stage: { position: { x: 0, y: 0 }, scale: { set() {} } },
            screen: { width: 800, height: 600 } };
    texFor = () => ({ height: 40, width: 30 });
    scene = { player: { serial: 9, x: 10, y: 10, z: 0, dir: 2, body: 400, at: 2, name: "me" },
              mobiles: [], items: [], statics: [], lights: [],
              map: { radius: 1, cx: 10, cy: 10, tiles: [], viewRange: 1 } };
    anim.clear();
  `);
  // One mobile, standing on its target tile, holding a server pose (group 9 is
  // a swing — what a 0x6E for a combat blow delivers).
  ctx.run(`
    anim.set("m1", { body: 400, at: 2, dir: 2, x: 10, y: 10, z: 0,
                     rx: 10, ry: 10, rz: 0, tx: 10, ty: 10,
                     animMoving: false, animPhase: 0, stepDur: 300,
                     act: { group: 9, startMs: 0, frameMs: 100, frames: 5 } });
  `);
  return ctx;
}

test("a server pose survives while the mobile stands still", () => {
  const ctx = moveCtx();
  ctx.setNow(0);
  ctx.run("renderFrame(16)");
  const st = ctx.run('anim.get("m1")');
  ok(st.act, "still posed");
  eq(st.act.group, 9, "on the group the server sent");
  eq(st.animMoving, false, "and not moving");
});

test("the frame a step begins is the frame the pose is dropped", () => {
  const ctx = moveCtx();
  ctx.setNow(0);
  ctx.run("renderFrame(16)");
  ok(ctx.run('anim.get("m1").act'), "posed before the step");
  // The server put it one tile away: from here on it is interpolating.
  ctx.run('Object.assign(anim.get("m1"), { tx: 11, ty: 10 });');
  ctx.setNow(16);
  ctx.run("renderFrame(16)");
  const st = ctx.run('anim.get("m1")');
  eq(st.animMoving, true, "the step is live");
  eq(st.act, null, "…and the pose was dropped in that same frame");
});

test("a pose that arrives mid-step never gets a frame of its own", () => {
  const ctx = moveCtx();
  ctx.setNow(0);
  ctx.run('Object.assign(anim.get("m1"), { act: null, tx: 13, ty: 10 });');
  ctx.run("renderFrame(16)");
  eq(ctx.run('anim.get("m1").animMoving'), true, "walking");
  // 0x6E lands now — this is the case the live probe could not stage.
  ctx.run('anim.get("m1").act = { group: 9, startMs: 16, frameMs: 100, frames: 5 };');
  ctx.setNow(32);
  ctx.run("renderFrame(16)");
  eq(ctx.run('anim.get("m1").act'), null, "cancelled on the next frame, unplayed");
});

test("the pose comes back once the mobile has arrived", () => {
  const ctx = moveCtx();
  ctx.setNow(0);
  ctx.run('Object.assign(anim.get("m1"), { act: null, tx: 11, ty: 10 });');
  ctx.run("renderFrame(16)");
  eq(ctx.run('anim.get("m1").animMoving'), true, "walking");
  // Let it finish the step and run past `moveUntil`.
  for (let t = 16; t <= 2000; t += 16) { ctx.setNow(t); ctx.run("renderFrame(16)"); }
  eq(ctx.run('anim.get("m1").animMoving'), false, "arrived and settled");
  ctx.run('anim.get("m1").act = { group: 9, startMs: 2000, frameMs: 100, frames: 5 };');
  ctx.setNow(2016);
  ctx.run("renderFrame(16)");
  ok(ctx.run('anim.get("m1").act'), "a pose sent to a standing mobile is kept");
});
