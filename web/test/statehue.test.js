// The whole-mobile state tint — ClassicUO's `overridedHue`
// (`Views/MobileView.cs:95-135`). It REPLACES every layer's own dye, so a
// poisoned creature is green from across the screen rather than green on a 30px
// health bar. Two things are worth pinning and neither is obvious from reading
// the function: the constants are ClassicUO's Profile defaults (a "nicer" green
// would be a silent divergence), and the precedence is LAST-WINS because
// ClassicUO writes three consecutive plain `if`s, not `else if`s.
//
// Verified live against ServUO on 2026-08-30 by poisoning a dragon: the body
// URL went to `?hue=68` and every wing, not just the bar, drew green.
const { newContext } = require("./harness.js");
const { test, eq, ok } = require("./run.js");

function hueCtx() {
  const ctx = newContext();
  ctx.load("00-state.js", "03-world.js", "06-movement.js");
  return ctx;
}
// A body id inside IsHuman's 0x0190..0x0193, and one well outside it.
const HUMAN = 400, CREATURE = 17;

test("the constants are ClassicUO's Profile defaults", () => {
  const ctx = hueCtx();
  eq(ctx.get("HUE_POISON"), 0x0044, "Profile.PoisonHue");
  eq(ctx.get("HUE_PARALYZED"), 0x014c, "Profile.ParalyzedHue");
  eq(ctx.get("HUE_INVULNERABLE"), 0x0030, "Profile.InvulnerableHue");
  eq(ctx.get("HUE_DEAD_CREATURE"), 0x0386, "MobileView's dead-creature grey");
  eq(ctx.get("HUE_HIDDEN"), 0x038e, "MobileView's hidden wash");
});

test("each state picks its own tint", () => {
  const ctx = hueCtx();
  const hue = (ent, hidden, dead, body) => {
    ctx.set("__e", ent);
    return ctx.run(`mobileStateHue(__e, ${!!hidden}, ${!!dead}, ${body | 0})`);
  };
  eq(hue({}, false, false, CREATURE), 0, "an ordinary mobile keeps its own dye");
  eq(hue({ poisoned: true }, false, false, CREATURE), 0x0044, "poisoned is green");
  eq(hue({ para: true }, false, false, CREATURE), 0x014c, "paralysed");
  eq(hue({ yellow: true }, false, false, CREATURE), 0x0030, "yellow hits");
  eq(hue({}, true, false, CREATURE), 0x038e, "hidden outranks everything");
  eq(hue({ poisoned: true }, true, false, CREATURE), 0x038e, "…including poison");
});

test("death greys a creature but leaves a human ghost to its own alpha", () => {
  const ctx = hueCtx();
  const hue = (ent, hidden, dead, body) => {
    ctx.set("__e", ent);
    return ctx.run(`mobileStateHue(__e, ${!!hidden}, ${!!dead}, ${body | 0})`);
  };
  eq(hue({}, false, true, CREATURE), 0x0386, "a dead creature greys out");
  eq(hue({}, false, true, HUMAN), 0, "a human ghost is drawn translucent instead");
  eq(hue({ poisoned: true }, false, true, CREATURE), 0x0386,
     "dead is checked before poison, so a poisoned corpse is grey not green");
});

// The order is load-bearing. ClassicUO's three plain `if`s mean the LAST true
// one wins, so a mobile that is both paralysed and yellow-hits draws yellow.
// Observed live: a Blessed NPC reported para+yellow together and drew 0x30.
test("later states overwrite earlier ones, in ClassicUO's order", () => {
  const ctx = hueCtx();
  const hue = (ent) => { ctx.set("__e", ent); return ctx.run("mobileStateHue(__e, false, false, 17)"); };
  eq(hue({ poisoned: true, para: true }), 0x014c, "paralysis beats poison");
  eq(hue({ para: true, yellow: true }), 0x0030, "yellow hits beats paralysis");
  eq(hue({ poisoned: true, para: true, yellow: true }), 0x0030, "and beats all three");
});

test("a mobile we have no record for is not tinted", () => {
  const ctx = hueCtx();
  ctx.set("__e", null);
  eq(ctx.run("mobileStateHue(__e, false, false, 17)"), 0, "no entity, no tint");
  ok(ctx.run("mobileStateHue(__e, true, false, 17)") === 0x038e,
     "…but hidden is resolved by the caller and still applies");
});

// ClassicUO guards BOTH the paralysis and the yellow-hits tint with
// `NotorietyFlag != Invulnerable` (MobileView.cs:126/:135) and guards poison
// with nothing. An invulnerable mobile already reads as invulnerable from its
// notoriety colour; painting the whole body over that would hide it.
test("an invulnerable mobile is not tinted by paralysis or yellow hits", () => {
  const ctx = hueCtx();
  const hue = (ent) => { ctx.set("__e", ent); return ctx.run("mobileStateHue(__e, false, false, 17)"); };
  eq(hue({ noto: 7, para: true }), 0, "paralysis does not paint an invulnerable");
  eq(hue({ noto: 7, yellow: true }), 0, "nor do yellow hits");
  eq(hue({ noto: 7, poisoned: true }), 0x0044,
     "…but poison is unguarded there, so an invulnerable still shows green");
  eq(hue({ noto: 1, para: true }), 0x014c, "an innocent is tinted as before");
  eq(hue({ noto: 6, yellow: true }), 0x0030, "and so is a murderer");
});
