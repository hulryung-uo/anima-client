// Name plates, the journal's tabs and the ignore list (web/js/08-overlays.js).
//
// 66 top-level functions and five of them were so much as named in a test. What
// is covered here is the part a player argues with: which things get a name over
// them, what that name says, where the label ends up when two of them collide,
// which tab a line lands in, and who gets silenced.
const { newContext } = require("./harness.js");
const { test, ok, eq, ne, deepEq } = require("./run.js");

function ovCtx() {
  const ctx = newContext();
  ctx.mountPage();
  ctx.loadAll();
  ctx.run(`
    app = { canvas: { style: {}, clientWidth: 800, clientHeight: 600,
                      getBoundingClientRect: () => ({ left: 0, top: 0, width: 800, height: 600 }) },
            renderer: { width: 800, height: 600, events: { cursorStyles: {} } },
            stage: { position: { x: 0, y: 0 } } };
    world = new PIXI.Container();
    ignoredNames = new Map();
  `);
  ctx.set("__scene", { player: { serial: 9, x: 10, y: 10, z: 0, name: "Me", noto: 1, equip: [] },
                       mobiles: [], items: [], statics: [], contItems: [],
                       target: { active: 0, flag: 0 } });
  ctx.run("scene = __scene;");
  return ctx;
}

// ── which things get a plate (NameOverHeadManager.IsAllowed) ───────────────

const CORPSE = 0x2006;

test("the four filters are ClassicUO's four, corpses being items included", () => {
  const ctx = ovCtx();
  const allowed = (filter, isMobile, isCorpse) => {
    ctx.run(`settings.plateFilter = ${JSON.stringify(filter)};`);
    return ctx.run(`plateAllowed(${!!isMobile}, ${!!isCorpse})`);
  };
  for (const [isMobile, isCorpse, what] of [[true, false, "a mobile"],
                                            [false, false, "an item"],
                                            [false, true, "a corpse"]]) {
    eq(allowed("all", isMobile, isCorpse), true, `all: ${what}`);
  }
  deepEq([allowed("mobiles", true, false), allowed("mobiles", false, false), allowed("mobiles", false, true)],
         [true, false, false], "mobiles: only mobiles, and a corpse is not one");
  deepEq([allowed("items", true, false), allowed("items", false, false), allowed("items", false, true)],
         [false, true, true], "items: every item, corpses among them — a corpse IS an item");
  deepEq([allowed("mobcorpses", true, false), allowed("mobcorpses", false, false), allowed("mobcorpses", false, true)],
         [true, false, true], "mobiles+corpses: those two and nothing else");
});

test("plates come on with the setting, or while Ctrl and Shift are both down", () => {
  const ctx = ovCtx();
  ctx.run("settings.namePlates = false; plateMods.ctrl = false; plateMods.shift = false;");
  eq(ctx.run("platesActive()"), false, "off");
  ctx.run("plateMods.ctrl = true;");
  eq(ctx.run("platesActive()"), false, "Ctrl alone is not the gesture");
  ctx.run("plateMods.shift = true;");
  eq(ctx.run("platesActive()"), true, "Ctrl+Shift is");
  ctx.run("plateMods.ctrl = false; plateMods.shift = false; settings.namePlates = true;");
  eq(ctx.run("platesActive()"), true, "and the latch holds it on with no keys at all");
});

// ── what the plate says (NameOverheadGump.SetName) ─────────────────────────

test("an OPL name outranks the tiledata one", () => {
  const ctx = ovCtx();
  ctx.run(`scene.opl = { 513: ["a masterwork dagger"] };
           staticNameCache.set(0x0F51, "dagger");`);
  eq(ctx.run(`itemPlateName({ serial: 513, g: 0x0F51, amount: 1 })`), "a masterwork dagger",
     "what the server said about THIS item beats what the art is called");
});

test("a stack is named with its count, and a corpse never is", () => {
  const ctx = ovCtx();
  ctx.run(`staticNameCache.set(0x0EED, "gold coin"); staticNameCache.set(${CORPSE}, "corpse");`);
  eq(ctx.run(`itemPlateName({ serial: 1, g: 0x0EED, amount: 250 })`), "250 gold coin", "a pile counts itself");
  eq(ctx.run(`itemPlateName({ serial: 2, g: 0x0EED, amount: 1 })`), "gold coin", "one coin does not");
  eq(ctx.run(`itemPlateName({ serial: 3, g: ${CORPSE}, amount: 5 })`), "corpse",
     "a corpse's amount is its body id, not a count — prefixing it would read as '5 corpse'");
});

test("an unknown graphic asks once and is unlabelled until the answer lands", () => {
  const ctx = ovCtx();
  const asked = [];
  ctx.run("staticNameCache.clear(); platePending.clear();");
  ctx.set("__asked", asked);
  ctx.run("staticTileName = (g, cb) => { __asked.push(g); };");
  eq(ctx.run(`itemPlateName({ serial: 1, g: 0x1234, amount: 1 })`), "", "nothing to show yet");
  ctx.run(`itemPlateName({ serial: 2, g: 0x1234, amount: 1 })`);
  ctx.run(`itemPlateName({ serial: 3, g: 0x1234, amount: 1 })`);
  deepEq(asked, [0x1234], "asked once for the GRAPHIC, not once per frame or per item");
});

test("a graphic the server has no name for stays unlabelled instead of retrying forever", () => {
  const ctx = ovCtx();
  ctx.run("staticNameCache.clear(); platePending.clear(); staticNameCache.set(0x1234, null);");
  const asked = [];
  ctx.set("__asked", asked);
  ctx.run("staticTileName = (g) => { __asked.push(g); };");
  eq(ctx.run(`itemPlateName({ serial: 1, g: 0x1234, amount: 1 })`), "", "no name");
  deepEq(asked, [], "and no second request — a cached null is an answer");
});

// ── where the label goes when two collide ──────────────────────────────────

test("a lone label sits exactly where it was asked to", () => {
  const ctx = ovCtx();
  ctx.set("__boxes", []);
  eq(ctx.run("placeNameLabel(__boxes, 100, 200, 60, 14)"), 200, "unmoved");
  deepEq(ctx.run("__boxes")[0], { l: 70, r: 130, t: 186, b: 200 }, "and its box is recorded");
});

test("two labels on the same spot stack upward instead of overprinting", () => {
  const ctx = ovCtx();
  ctx.set("__boxes", []);
  const a = ctx.run("placeNameLabel(__boxes, 100, 200, 60, 14)");
  const b = ctx.run("placeNameLabel(__boxes, 100, 200, 60, 14)");
  eq(a, 200, "the first keeps the spot");
  ok(b < a, `the second was pushed up (${b} vs ${a})`);
  eq(b, 186 - 2, "to just above the first, by the 2px pad");
});

test("a third label clears BOTH of the ones already there", () => {
  const ctx = ovCtx();
  ctx.set("__boxes", []);
  ctx.run("placeNameLabel(__boxes, 100, 200, 60, 14)");
  ctx.run("placeNameLabel(__boxes, 100, 200, 60, 14)");
  const c = ctx.run("placeNameLabel(__boxes, 100, 200, 60, 14)");
  const boxes = ctx.run("__boxes");
  eq(c, 168, "stacked a third row up: 200 → 184 → 168, each a label plus the 2px pad");
  for (const b of boxes.slice(0, 2)) ok(c - 14 >= b.b || c <= b.t, "clear of the ones below it");
});

test("labels that do not overlap horizontally are left alone", () => {
  const ctx = ovCtx();
  ctx.set("__boxes", []);
  ctx.run("placeNameLabel(__boxes, 100, 200, 60, 14)");
  eq(ctx.run("placeNameLabel(__boxes, 300, 200, 60, 14)"), 200, "different column, same row");
});

// ── the journal's tabs ─────────────────────────────────────────────────────

test("each kind of line lands in the tab a player would look for it in", () => {
  const ctx = ovCtx();
  const cls = (line, local) => { ctx.set("__l", line); return ctx.run(`journalClass(__l, ${!!local})`); };
  eq(cls({ type: 0, serial: 0x101, name: "Ann", text: "hi" }), "speech", "someone talking");
  eq(cls({ type: 13, serial: 0x101, name: "Ann" }), "guild", "guild");
  eq(cls({ type: 14, serial: 0x101, name: "Ann" }), "guild", "alliance");
  eq(cls({ type: 7, serial: 0x101, name: "Ann" }), "guild", "party");
  eq(cls({ type: 1, serial: 0x101, name: "Ann" }), "system", "a system message type");
  eq(cls({ type: 0, serial: 0xFFFFFFFF, name: "" }), "system", "the no-source serial");
  eq(cls({ type: 0, serial: 0, name: "" }), "system", "…and serial 0");
  eq(cls({ type: 0, serial: 0x101, name: "System" }), "system", "anything calling itself System");
  eq(cls({ type: 0, serial: 0x101, name: "Ann" }, true), "system", "our own client notices");
});

test("the all tab takes everything and each other tab takes only its own", () => {
  const ctx = ovCtx();
  const acc = (key, line) => { ctx.set("__l", line); return ctx.run(`journalTabAccepts(${JSON.stringify(key)}, __l, false)`); };
  const speech = { type: 0, serial: 0x101, name: "Ann" };
  const guild = { type: 13, serial: 0x101, name: "Ann" };
  deepEq([acc("all", speech), acc("all", guild)], [true, true], "all takes both");
  deepEq([acc("speech", speech), acc("speech", guild)], [true, false], "speech takes speech");
  deepEq([acc("guild", guild), acc("guild", speech)], [true, false], "guild takes guild");
});

// ── the ignore list (ClassicUO IgnoreManager) ──────────────────────────────

test("a name is matched however it was typed, and past its title", () => {
  const ctx = ovCtx();
  ctx.run(`ignoreName("Carl")`);
  eq(ctx.run(`isIgnoredName("carl")`), true, "case does not save you");
  eq(ctx.run(`isIgnoredName("  CARL  ")`), true, "nor does whitespace");
  eq(ctx.run(`isIgnoredName("Carl\\tthe tailor")`), true,
     "an OPL name carries a tabbed title — the character is still Carl");
  eq(ctx.run(`isIgnoredName("Carla")`), false, "and it is not a prefix match");
});

test("an empty name is never ignorable — it would silence everyone unnamed", () => {
  const ctx = ovCtx();
  eq(ctx.run(`ignoreName("")`), false, "refused");
  eq(ctx.run(`ignoreName("   ")`), false, "whitespace too");
  eq(ctx.run(`isIgnoredName("")`), false, "and an empty name never matches the list");
  eq(ctx.run("ignoredNames.size"), 0, "nothing was added");
});

test("adding the same character twice says so instead of duplicating", () => {
  const ctx = ovCtx();
  eq(ctx.run(`ignoreName("Carl")`), true, "added");
  eq(ctx.run(`ignoreName("carl")`), false, "the second is refused");
  eq(ctx.run("ignoredNames.size"), 1, "one entry");
  ok(ctx.run(`localJournal.some((l) => /already exist in a list/.test(l.text))`),
     "with ClassicUO's own wording");
});

test("the list keeps the name as it was typed, not the lowercased key", () => {
  const ctx = ovCtx();
  ctx.run(`ignoreName("Carl the Tailor")`);
  deepEq([...ctx.run("[...ignoredNames.values()]")], ["Carl the Tailor"], "shown back as written");
});

test("ignoring a mobile refuses yourself, an invulnerable, and one with no name yet", () => {
  const ctx = ovCtx();
  ctx.run(`scene.mobiles = [
    { serial: 0x101, name: "Ann", noto: 1 },
    { serial: 0x102, name: "Healer", noto: 1, yellow: true },
    { serial: 0x103, name: "", noto: 1 }];`);
  eq(ctx.run("ignoreMobile(9)"), false, "yourself");
  eq(ctx.run("ignoreMobile(0x102)"), false, "a yellow-hits mobile — a GM or a quest NPC");
  eq(ctx.run("ignoreMobile(0x103)"), false, "one whose name we have never been told");
  ok(ctx.run(`localJournal.some((l) => /name is not known yet/.test(l.text))`), "…and says which");
  eq(ctx.run("ignoreMobile(0x101)"), true, "an ordinary one goes on the list");
  eq(ctx.run(`isIgnoredName("Ann")`), true, "and is ignored from then on");
});

test("a spell line is never silenced, whoever cast it", () => {
  const ctx = ovCtx();
  ctx.run(`ignoreName("Carl")`);
  ctx.set("__spell", { type: ctx.get("MSG_SPELL"), name: "Carl", text: "Kal Vas Flam" });
  ctx.set("__talk", { type: 0, name: "Carl", text: "hello" });
  eq(ctx.run("isIgnoredLine(__talk)"), true, "his chatter is gone");
  eq(ctx.run("isIgnoredLine(__spell)"), false,
     "his spell words are not — you need to see what is being cast at you");
});

test("un-ignoring puts them back", () => {
  const ctx = ovCtx();
  ctx.run(`ignoreName("Carl")`);
  ctx.run(`unignoreName("CARL")`);
  eq(ctx.run(`isIgnoredName("Carl")`), false, "matched by the same key it was added under");
  eq(ctx.run("ignoredNames.size"), 0, "gone");
});

// ── the health bar's colour ────────────────────────────────────────────────

test("the health bar goes green, amber, red as it drains", () => {
  const ctx = ovCtx();
  const c = (f) => ctx.run(`hpColor(${f})`);
  eq(c(1), 0x46a758, "full: green");
  eq(c(0.51), 0x46a758, "just over half: still green");
  eq(c(0.5), 0xd9a441, "half: amber");
  eq(c(0.26), 0xd9a441, "…down to a quarter");
  eq(c(0.25), 0xe5484d, "a quarter: red");
  eq(c(0), 0xe5484d, "dead: red");
});
