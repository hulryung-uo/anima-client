// The idle-fidget clock in web/js/06-movement.js, driven head-less.
//
// ClassicUO stands a mobile still for 30-60 s and then plays one of the body's
// three fidget animations — but only if the animation exists, only if the mobile
// is not moving/mounted/in war/dead, and never over a pose the server sent. Each
// of those is a branch nobody exercises by hand: you would have to stand in a
// field for a minute per case, on a live shard, and watch.
const { newContext } = require("./harness.js");
const { test, ok, eq, between, includes } = require("./run.js");

// /idleanim/<body> answers with {g: [group…], e: [exists?…]} — a human (400) has
// all three fidgets; 402 (the ghost body) has none; 6 (a crow) only the middle
// entry. The fixture IS the server here; nothing reaches one.
const IDLE_TABLE = { 400: { g: [5, 6, 34], e: [1, 1, 1] },
                     402: { g: [5, 6, 34], e: [0, 0, 0] },
                     6:   { g: [17, 1, 17], e: [0, 1, 0] } };

function fresh(seed) {
  const ctx = newContext(seed === undefined ? {} : { seed });
  ctx.load("00-state.js", "01-audio.js", "02-textures.js", "03-world.js", "06-movement.js");
  ctx.setFetch((u) => IDLE_TABLE[Number(String(u).split("/")[1].split("?")[0])] || { g: [], e: [] });
  return ctx;
}
// tickIdleAnim(state, now, body, moving, mounted, war, flying, ghost)
const tick = (ctx, st, now, body, o = {}) => {
  ctx.setNow(now);
  ctx.set("__st", st);
  ctx.run(`tickIdleAnim(__st, ${now}, ${body}, ${!!o.moving}, ${!!o.mounted}, ${!!o.war}, ${!!o.flying}, ${!!o.ghost})`);
  return st;
};
const asked = (ctx, prefix) => ctx.fetchLog.filter((u) => u.startsWith(prefix)).length;

test("the clock arms on first sight and stays quiet for 30-60 s", () => {
  const ctx = fresh();
  const st = {};
  tick(ctx, st, 0, 400);
  between(st.idleAt, 30000, 60000, "armed inside ClassicUO's 30-60 s window");
  ok(!st.act, "nothing fires on the arming tick");
  tick(ctx, st, 29999, 400);
  ok(!st.act, "silent before 30 s");
});

test("the first expiry asks the server once, and re-arms while it waits", async () => {
  const ctx = fresh();
  const st = {};
  tick(ctx, st, 0, 400);
  tick(ctx, st, 60001, 400);
  await ctx.flush();
  eq(asked(ctx, "idleanim/400"), 1, "one /idleanim request");
  between(st.idleAt, 60001 + 30000, 60001 + 60000,
          "re-armed even though the table had not landed — no busy-wait on the fetch");
  tick(ctx, st, st.idleAt + 1, 400);
  ok(st.act, "fidgets once the table is in");
  includes([5, 6, 34], st.act.group, "picked a real people fidget group");
  eq(st.act.idle, true, "flagged as a client-side idle, not a server pose");
  eq(st.act.fwd, true, "plays forward");
  eq(st.act.frameMs, 240, "at ClassicUO's 3 x 80 ms per frame");
});

test("every suppression rule holds, and each still pushes the clock out", () => {
  for (const [why, o] of [["moving", { moving: true }], ["mounted", { mounted: true }],
                          ["in war mode", { war: true }], ["dead", { ghost: true }]]) {
    const ctx = fresh();
    const st = {};
    tick(ctx, st, 0, 400);
    tick(ctx, st, 100000, 400, o);
    ok(!st.act, `no fidget while ${why}`);
    ok(st.idleAt >= 100000, `still re-armed while ${why} (idleAt ${st.idleAt})`);
  }
});

test("a pose the server sent is never overwritten", () => {
  const ctx = fresh();
  const st = { act: { group: 9, startMs: 0 } };
  tick(ctx, st, 0, 400);
  tick(ctx, st, 100000, 400);
  eq(st.act.group, 9, "the server's 0x6E/0xE2 animation still owns the mobile");
  ok(st.idleAt >= 130000, `…and pushed the idle clock out (idleAt ${st.idleAt})`);
});

test("a body with no fidget frames never fidgets, and is asked about once", async () => {
  const ctx = fresh();
  const st = {};
  tick(ctx, st, 0, 402);
  tick(ctx, st, 100000, 402);
  await ctx.flush();
  for (let i = 0; i < 50; i++) tick(ctx, st, 200000 + i * 100000, 402);
  ok(!st.act, "the ghost body stayed still");
  eq(asked(ctx, "idleanim/402"), 1, "…without re-asking the server 50 times");
});

test("ClassicUO's index flip finds the one animation a body does have", async () => {
  // A crow (6) has only entry 1. ClassicUO rolls 0..2 and, when the rolled entry
  // does not exist, flips to index 0 — so rolling 0 or 1 lands on the real one
  // and rolling 2 gives up. Over many rolls: both outcomes, and NEVER a group
  // whose frames are missing (that draws an empty sprite on a live shard).
  const ctx = fresh(31337);
  const warm = {};
  tick(ctx, warm, 0, 6); tick(ctx, warm, 100000, 6);
  await ctx.flush();
  let fired = 0, quiet = 0;
  for (let i = 0; i < 600; i++) {
    const st = {};
    tick(ctx, st, 0, 6);
    tick(ctx, st, 100000, 6);
    if (st.act) { fired++; eq(st.act.group, 1, "only the group that exists is ever played"); }
    else quiet++;
  }
  between(fired, 300, 500, `the flip finds the existing entry ~2/3 of the time (${fired}/600)`);
  between(quiet, 100, 300, `…and gives up the other ~1/3 (${quiet}/600)`);
});

test("all three people fidgets are reachable — the roll is inclusive", async () => {
  // ClassicUO's GetValue(0, 2) is INCLUSIVE of 2. An exclusive port silently
  // drops Fidget3 forever, which is invisible on a shard and obvious here.
  const ctx = fresh(9001);
  const warm = {};
  tick(ctx, warm, 0, 400); tick(ctx, warm, 100000, 400);
  await ctx.flush();
  const seen = new Set();
  for (let i = 0; i < 400; i++) {
    const st = {};
    tick(ctx, st, 0, 400);
    tick(ctx, st, 100000, 400);
    if (st.act) seen.add(st.act.group);
  }
  for (const g of [5, 6, 34]) includes(seen, g, `fidget group ${g} is reachable`);
});
