// Container windows (web/js/10-housing.js) in both of the modes they render in,
// and the corpses that open themselves.
//
// The authentic mode draws ServUO's own gump art and puts each item at the (x, y)
// the server stored for it; the grid mode is a titled list with click-to-loot.
// Which one you get is decided by four inputs at once — the 0x24 gump id, whether
// this container is a corpse, and two Options toggles — plus a fifth state that
// is neither: "0x24 has not named this container YET", which must look like
// nothing at all rather than flashing a grid and swapping a poll later.
const { newContext } = require("./harness.js");
const { test, ok, eq, ne, deepEq } = require("./run.js");

function contCtx() {
  const ctx = newContext();
  ctx.mountPage();
  ctx.loadAll();
  const sent = [];
  ctx.setFetch((u, init) => {
    if (String(u) === "/input") { sent.push(String(init && init.body)); return {}; }
    return { status: 404, ok: false, body: null };
  });
  ctx.sent = sent;
  ctx.run(`
    app = { canvas: { style: {}, clientWidth: 800, clientHeight: 600,
                      getBoundingClientRect: () => ({ left: 0, top: 0, width: 800, height: 600 }) },
            renderer: { width: 800, height: 600, events: { cursorStyles: {} } },
            stage: { position: { x: 0, y: 0 } } };
    world = new PIXI.Container();
    __sfx = [];
    playSfx = (id) => { __sfx.push(id); };
  `);
  ctx.sfx = () => ctx.run("__sfx");
  return ctx;
}

const BAG = 0x777, BACKPACK = [{ serial: 0x500, layer: 21, g: 0x0E75 }];
// A scene with one open-able container. `gump` undefined = 0x24 has not landed;
// null = it landed carrying 0 (a shard that `[set GumpID 0`).
function world(ctx, { gump, items = [], contInfo = null, hidden = false,
                      groundItems = [], target = null, equip = BACKPACK } = {}) {
  const contGumps = {};
  if (gump !== undefined) contGumps[String(BAG)] = gump === null ? 0 : gump;
  ctx.set("__scene", {
    player: { serial: 9, x: 5, y: 5, z: 0, name: "Me", equip, hidden },
    mobiles: [], items: groundItems, statics: [],
    contItems: items.map((it) => Object.assign({ cont: BAG }, it)),
    contGumps,
    contInfo: contInfo ? { [String(BAG)]: contInfo } : {},
    target: target || { active: 0, flag: 0 },
  });
  ctx.run("scene = __scene; updateTargetUI();");
  return ctx;
}
const winOf = (ctx, serial = BAG) => ctx.run(`dialogWindow("containers", ${serial})`);
const cells = (ctx, serial = BAG) => winOf(ctx, serial).body.querySelectorAll(".cont-item");

// ── which mode a container renders in ──────────────────────────────────────

test("a named gump draws the real bag art and places items at the server's own coordinates", () => {
  const ctx = world(contCtx(), {
    gump: 0x3C,
    items: [{ serial: 0x201, g: 0x0F3F, amount: 1, x: 40, y: 70, hue: 0 }],
  });
  ctx.run(`openContainer(${BAG})`);
  const win = winOf(ctx);
  eq(win.el.classList.contains("cont-authentic"), true, "chromeless authentic window");
  eq(win.body.classList.contains("cont-art"), true, "the body IS the gump art");
  eq(win.body.querySelector(".cont-bg").src, "gump/60.png", "…fetched by 0x24's gump id");
  eq(cells(ctx).length, 1, "one item");
  deepEq([cells(ctx)[0].style.left, cells(ctx)[0].style.top], ["40px", "70px"],
         "at the (x, y) 0x3C carried, raw — the gump's own pixel space");
  eq(cells(ctx)[0].style.position, "absolute", "absolutely placed, not flowed");
});

test("an item's x is SIGNED — ClassicUO's `(short)item.X`", () => {
  // A `| 0` on an already-u16 value would place a negative x off at 65000-odd.
  const ctx = world(contCtx(), {
    gump: 0x3C,
    items: [{ serial: 0x201, g: 0x0F3F, amount: 1, x: 0xFFFB, y: 0xFFF8, hue: 0 }],
  });
  ctx.run(`openContainer(${BAG})`);
  deepEq([cells(ctx)[0].style.left, cells(ctx)[0].style.top], ["-5px", "-8px"], "-5, -8");
});

test("the grid toggle overrides the art for EVERY container, and titles it", () => {
  const ctx = world(contCtx(), {
    gump: 0x3C,
    contInfo: { g: 0x0E79, name: "a pouch", hue: 0 },
    items: [{ serial: 0x201, g: 0x0F3F, amount: 3, x: 40, y: 70, hue: 0 }],
  });
  ctx.run("settings.gridContainers = true;");
  ctx.run(`openContainer(${BAG})`);
  const win = winOf(ctx);
  eq(win.el.classList.contains("cont-authentic"), false, "no authentic chrome");
  eq(win.body.classList.contains("cont-grid"), true, "a plain grid");
  eq(win.body.querySelector(".cont-bg"), null, "…and no bag art");
  ok(win.label.textContent.includes("a pouch"), "the container names itself");
  ok(win.label.querySelector(".cont-title-icon"), "with a small icon, so a pouch reads unlike a box");
  ok(!cells(ctx)[0].style.position, "grid cells flow rather than being absolutely placed");
  eq(cells(ctx)[0].querySelector(".cont-amt").textContent, "3", "a stack shows its count");
});

test("a corpse falls back to the click-to-loot grid, which is what GridLootGump exists for", () => {
  const ctx = world(contCtx(), {
    gump: 9,               // CORPSE_GUMP
    items: [{ serial: 0x201, g: 0x0F3F, amount: 1, x: 40, y: 70, hue: 0 }],
  });
  eq(ctx.run("settings.gridLoot"), true, "on by default");
  ctx.run(`openContainer(${BAG})`);
  const win = winOf(ctx);
  eq(win.body.classList.contains("cont-loot"), true, "loot mode");
  ok(win.body.querySelector(".loot-bar .loot-all"), "with a Loot all button");
  cells(ctx)[0].click();
  deepEq(ctx.sent, ["pickup:513", "drop:513:65535:65535:0:1280"],
         "one click takes it: the lift is required, and 0xFFFF/0xFFFF lets the pack place it");
});

test("switched off, a corpse gets its real gump art back", () => {
  const ctx = world(contCtx(), { gump: 9, items: [{ serial: 0x201, g: 0x0F3F, amount: 1, x: 4, y: 4 }] });
  ctx.run("settings.gridLoot = false;");
  ctx.run(`openContainer(${BAG})`);
  const win = winOf(ctx);
  eq(win.body.classList.contains("cont-art"), true, "authentic corpse gump");
  ok(win.body.querySelector(".cont-eye"), "including ClassicUO's blinking eye overlay");
});

test("a container the server has not NAMED yet shows nothing — no grid flash", () => {
  // 0x24 arrives separately from the contents. Flashing the grid and swapping to
  // the art one poll later is worse than a beat of nothing.
  const ctx = world(contCtx(), { gump: undefined, items: [{ serial: 0x201, g: 0x0F3F, amount: 1, x: 4, y: 4 }] });
  ctx.run(`openContainer(${BAG})`);
  const win = winOf(ctx);
  eq(win.el.classList.contains("cont-authentic"), true, "chromeless (transparent → invisible)");
  eq(cells(ctx).length, 0, "nothing drawn at all yet");
  eq(win.label.textContent, "", "not even a title");

  // …and the moment the id lands, the same window fills in.
  ctx.run(`scene.contGumps["${BAG}"] = 0x3C; refreshContainer(${BAG});`);
  eq(cells(ctx).length, 1, "the items appear as soon as 0x24 does");
  eq(winOf(ctx).body.querySelector(".cont-bg").src, "gump/60.png", "with the art it named");
});

test("a gump id of ZERO that WAS named is a different state — it renders, it does not wait", () => {
  // A shard can `[set GumpID 0`. Reading that as "not heard yet" would leave the
  // window invisible for good.
  const ctx = world(contCtx(), { gump: null, items: [{ serial: 0x201, g: 0x0F3F, amount: 1, x: 4, y: 4 }] });
  ctx.run(`openContainer(${BAG})`);
  eq(cells(ctx).length, 1, "drawn, as a grid");
  eq(winOf(ctx).body.classList.contains("cont-grid"), true, "there is no art to draw");
});

test("the signature carries everything that changes the LOOK but not the items", () => {
  // Found live twice: a late 0x24 left a window stuck as a grid, and a `[set Hue`
  // on an item in an open backpack left its icon the old colour.
  const ctx = world(contCtx(), { gump: 0x3C, contInfo: { g: 0x0E79, name: "a pouch" },
                                 items: [{ serial: 0x201, g: 0x0F3F, amount: 1, x: 4, y: 4, hue: 0 }] });
  const sig = () => ctx.run(`containerSignature(scene, ${BAG})`);
  const base = sig();
  const moved = [
    ["a late 0x24 gump id", `scene.contGumps["${BAG}"] = 0x3D;`],
    ["the container's own name", `scene.contInfo["${BAG}"].name = "a bag";`],
    ["the grid-containers toggle", "settings.gridContainers = true;"],
    ["the grid-loot toggle", "settings.gridLoot = false;"],
    ["the container scale", "settings.containerScale = 150;"],
    ["an item's hue", "scene.contItems[0].hue = 33;"],
    ["an item's amount", "scene.contItems[0].amount = 9;"],
  ];
  for (const [why, mutate] of moved) {
    const c = world(contCtx(), { gump: 0x3C, contInfo: { g: 0x0E79, name: "a pouch" },
                                 items: [{ serial: 0x201, g: 0x0F3F, amount: 1, x: 4, y: 4, hue: 0 }] });
    const before = c.run(`containerSignature(scene, ${BAG})`);
    c.run(mutate);
    ne(c.run(`containerSignature(scene, ${BAG})`), before, `${why} changes the signature`);
  }
  eq(sig(), base, "and nothing moved in the control");
  // "not heard yet" and "heard, and it was 0" must not fold together.
  const unheard = world(contCtx(), { gump: undefined, items: [] });
  const zero = world(contCtx(), { gump: null, items: [] });
  ne(unheard.run(`containerSignature(scene, ${BAG})`), zero.run(`containerSignature(scene, ${BAG})`),
     "'?' is not 0 — folding them would strand the wait branch forever");
});

test("chess and backgammon boards are never scaled (ClassicUO ContainerGump.GetScale)", () => {
  const ctx = contCtx();
  ctx.run("settings.containerScale = 200;");
  eq(ctx.run("containerScaleFor(0x091A)"), 1, "chess stays at 1x");
  eq(ctx.run("containerScaleFor(0x092E)"), 1, "backgammon too");
  eq(ctx.run("containerScaleFor(0x3C)"), 2, "a backpack follows the setting");
  ctx.run("settings.containerScale = 9999;");
  eq(ctx.run("containerScaleFor(0x3C)"), 2, "clamped to ClassicUO's 200 maximum");
  ctx.run("settings.containerScale = 1;");
  eq(ctx.run("containerScaleFor(0x3C)"), 0.5, "…and to its 50 minimum");
});

// ── opening, closing and the sounds ────────────────────────────────────────

test("re-opening a container raises the one that is up rather than making a second", () => {
  const ctx = world(contCtx(), { gump: 0x3C, items: [] });
  ctx.run(`openContainer(${BAG})`);
  const first = winOf(ctx).el;
  ctx.run(`openContainer(${BAG})`);
  eq(ctx.run(`dialogWindows("containers").size`), 1, "still one window");
  eq(winOf(ctx).el, first, "the same element");
  eq(ctx.document.body.children[ctx.document.body.children.length - 1], first, "raised to the front");
});

test("the open sound waits for the gump id, and the close sound is the pair's second entry", () => {
  // ClassicUO plays OpenSound when the gump is created, ClosedSound client-side
  // on close (closing sends no packet, so the server never sounds it).
  const ctx = world(contCtx(), { gump: undefined, items: [] });
  ctx.run(`openContainer(${BAG})`);
  deepEq(ctx.sfx(), [], "nothing to sound yet — we do not know what this container is");
  ctx.run(`scene.contGumps["${BAG}"] = 0x3C; refreshContainer(${BAG});`);
  deepEq(ctx.sfx(), [0x48], "a backpack's open sound");
  ctx.run(`refreshContainer(${BAG}); refreshContainer(${BAG});`);
  deepEq(ctx.sfx(), [0x48], "…once per open, not once per refresh");
  ctx.run(`closeContainer(${BAG})`);
  deepEq(ctx.sfx(), [0x48, 0x58], "and its close sound");
  eq(winOf(ctx), undefined, "the window is gone");
});

test("a silent gump makes no sound, and closing a window that is not open makes none either", () => {
  const ctx = world(contCtx(), { gump: 9, items: [] });     // a corpse: not in CONTAINER_SOUNDS
  ctx.run(`openContainer(${BAG}); closeContainer(${BAG});`);
  deepEq(ctx.sfx(), [], "corpses are silent, exactly as ClassicUO's default ContainerData is");
  ctx.run(`closeContainer(${BAG})`);
  deepEq(ctx.sfx(), [], "…and a second close sounds nothing");
});

test("right-clicking a container closes it, and keeps the click off the world behind", () => {
  // Once the title bar's ✕ is hidden in authentic mode this is the only close
  // affordance (ClassicUO ContainerGump.cs:151).
  const ctx = world(contCtx(), { gump: 0x3C, items: [] });
  ctx.run(`openContainer(${BAG})`);
  const el = winOf(ctx).el;
  const ev = ctx.event("contextmenu", { bubbles: true });
  el.dispatchEvent(ev);
  eq(winOf(ctx), undefined, "closed");
  eq(ev.defaultPrevented, true, "no Chrome menu");
  eq(ev.propagationStopped, true, "…and the canvas behind never sees it");
});

test("an iconized container is restored by a DOUBLE click, never a single one", () => {
  // ClassicUO GumpPicContainerOnMouseDoubleClick — a single click still drags
  // the collapsed gump out of the way.
  const ctx = world(contCtx(), { gump: 0x3C, items: [] });
  ctx.run(`openContainer(${BAG})`);
  ctx.run(`dialogWindow("containers", ${BAG}).minimized = true; refreshContainer(${BAG});`);
  eq(winOf(ctx).el.classList.contains("cont-iconized"), true, "collapsed to the pin art");
  eq(winOf(ctx).body.querySelector(".cont-bg").src, "gump/80.png", "…which is 0x3C's IconizedGraphic");
  ctx.fire(winOf(ctx).el, "click", { bubbles: true });
  eq(winOf(ctx).minimized, true, "a single click leaves it collapsed");
  ctx.fire(winOf(ctx).el, "dblclick", { bubbles: true });
  eq(winOf(ctx).minimized, false, "a double click restores it");
  eq(winOf(ctx).el.classList.contains("cont-iconized"), false, "back to the bag");
});

test("only the four gumps that ship an iconized art can collapse at all", () => {
  // ClassicUO: IconizedGraphic == 0 → an empty MinimizerArea.
  const ctx = contCtx();
  deepEq(ctx.run("Object.keys(CONTAINER_ICONIZE).map(Number)"), [0x3C, 0x775E, 0x7760, 0x7762],
         "a backpack and the three chest-of-drawers gumps");
  const other = world(contCtx(), { gump: 0x3D, items: [] });
  other.run(`openContainer(${BAG})`);
  eq(other.run(`dialogWindow("containers", ${BAG}).body.querySelector(".cont-min")`), null,
     "no minimize hot-spot on a gump with no icon art");
  other.run(`dialogWindow("containers", ${BAG}).minimized = true; refreshContainer(${BAG});`);
  eq(other.run(`dialogWindow("containers", ${BAG}).minimized`), false, "and the flag is forced back off");
});

test("the minimize hot-spot is a click, not a drag, and never fires with an item on the cursor", () => {
  const ctx = world(contCtx(), { gump: 0x3C, items: [] });
  ctx.run(`openContainer(${BAG})`);
  const hit = winOf(ctx).body.querySelector(".cont-min");
  ok(hit, "the hot-spot is at ClassicUO's MinimizerArea");
  deepEq([hit.style.left, hit.style.top], ["105px", "162px"], "…at its coordinates");

  const press = (dx, dy) => {
    ctx.fire(hit, "mousedown", { button: 0, clientX: 200, clientY: 200, bubbles: true });
    ctx.fire("window", "mouseup", { button: 0, clientX: 200 + dx, clientY: 200 + dy });
  };
  press(9, 0);
  eq(winOf(ctx).minimized, undefined, "a press that moved 9px was a drag, not a click");
  ctx.run("cursorItem = { serial: 1, g: 1, amount: 1, hue: 0 };");
  press(0, 0);
  eq(winOf(ctx).minimized, undefined, "ClassicUO skips minimize while ItemHold.Enabled");
  ctx.run("cursorItem = null;");
  press(2, 2);
  eq(winOf(ctx).minimized, true, "…and a real click collapses it");
});

// ── SkipEmptyCorpse ────────────────────────────────────────────────────────

test("an auto-opened corpse with nothing in it hides itself, and reveals the moment loot lands", () => {
  // Hidden, not closed: 0x3C contents arrive AFTER the 0x24 that opened the
  // window, so an empty corpse and one whose items are in flight look identical
  // at this instant.
  const ctx = world(contCtx(), { gump: 9, items: [] });
  ctx.run("settings.skipEmptyCorpse = true;");
  ctx.run(`autoOpenedCorpses.add(${BAG}); openContainer(${BAG});`);
  eq(winOf(ctx).el.style.display, "none", "hidden rather than covering the screen after every kill");
  ctx.run(`scene.contItems = [{ cont: ${BAG}, serial: 0x201, g: 0x0F3F, amount: 1, x: 4, y: 4, hue: 0 }];
           refreshContainer(${BAG});`);
  eq(winOf(ctx).el.style.display, "", "the next refresh reveals it");
});

// Was a documented bug, now fixed. The hide used to live INSIDE the
// `settings.skipEmptyCorpse` guard, so turning the option off never reached the
// line that would put the window back and an already-hidden corpse stayed
// invisible until it was closed and reopened. The reveal is unconditional now,
// outside the guard, next to the other "shed the previous mode" resets.
test("switching SkipEmptyCorpse off reveals a corpse window it had already hidden", () => {
  const ctx = world(contCtx(), { gump: 9, items: [] });
  ctx.run("settings.skipEmptyCorpse = true;");
  ctx.run(`autoOpenedCorpses.add(${BAG}); openContainer(${BAG});`);
  eq(winOf(ctx).el.style.display, "none", "hidden while the option is on");
  ctx.run("settings.skipEmptyCorpse = false; refreshOpenContainers();");
  eq(winOf(ctx).el.style.display, "", "…and it comes back the moment the option goes off");
  ctx.run("settings.skipEmptyCorpse = true; refreshOpenContainers();");
  eq(winOf(ctx).el.style.display, "none", "…and hides again when it goes back on");
});

test("a corpse YOU opened is never hidden, however empty it is", () => {
  const ctx = world(contCtx(), { gump: 9, items: [] });
  ctx.run("settings.skipEmptyCorpse = true;");
  ctx.run(`autoOpenedCorpses.add(${BAG}); manualOpenedCorpses.add(${BAG}); openContainer(${BAG});`);
  ne(winOf(ctx).el.style.display, "none", "you asked to see it — ClassicUO's ManualOpenedCorpses");
});

test("SkipEmptyCorpse touches nothing you opened by hand and nothing that is not a corpse", () => {
  for (const [why, setup] of [
    ["a corpse nobody auto-opened", `openContainer(${BAG});`],
    ["a chest", `autoOpenedCorpses.add(${BAG}); scene.contGumps["${BAG}"] = 0x3C; openContainer(${BAG});`],
  ]) {
    const ctx = world(contCtx(), { gump: 9, items: [] });
    ctx.run("settings.skipEmptyCorpse = true;");
    ctx.run(setup);
    ne(winOf(ctx).el.style.display, "none", `${why} is never hidden`);
  }
  const off = world(contCtx(), { gump: 9, items: [] });
  off.run(`autoOpenedCorpses.add(${BAG}); openContainer(${BAG});`);
  ne(winOf(off).el.style.display, "none", "and the option is off by default");
});

// ── auto-open corpses (ClassicUO PlayerMobile.TryOpenCorpses) ──────────────

const corpse = (serial, x, y) => ({ serial, g: 0x2006, c: 1, x, y, z: 0 });

test("auto-open is off by default, and opens each nearby corpse exactly once when on", () => {
  const off = world(contCtx(), { gump: 0x3C, groundItems: [corpse(0x301, 5, 5)] });
  off.run("autoOpenCorpses(scene)");
  deepEq(off.sent, [], "off by default, exactly as ClassicUO's AutoOpenCorpses is");

  const ctx = world(contCtx(), { gump: 0x3C, groundItems: [corpse(0x301, 5, 5), corpse(0x302, 6, 6)] });
  ctx.run("settings.autoOpenCorpses = true; autoOpenCorpses(scene);");
  deepEq(ctx.sent, ["use:769", "use:770"], "a real double-click each, and the 0x24 reply opens the window");
  ctx.run("autoOpenCorpses(scene); autoOpenCorpses(scene);");
  deepEq(ctx.sent, ["use:769", "use:770"], "…never a second time for the same corpse");
});

test("the range is ClassicUO's own AutoOpenCorpseRange, measured Chebyshev", () => {
  const ctx = world(contCtx(), {
    gump: 0x3C,
    groundItems: [corpse(0x301, 7, 7),     // dx 2, dy 2 → distance 2
                  corpse(0x302, 8, 5)],    // dx 3        → distance 3
  });
  eq(ctx.run("settings.autoOpenCorpseRange"), 2, "ClassicUO's default");
  ctx.run("settings.autoOpenCorpses = true; autoOpenCorpses(scene);");
  deepEq(ctx.sent, ["use:769"], "a diagonal two tiles away is inside range; three tiles is not");
  ctx.run("settings.autoOpenCorpseRange = 3; autoOpenCorpses(scene);");
  deepEq(ctx.sent, ["use:769", "use:770"], "widening the range picks the far one up");
});

test("both of ClassicUO's CorpseOpenOptions guards hold", () => {
  // Its default (3) is both at once: a window opening on its own would swallow
  // the click a target cursor is waiting for, and would give away a hidden
  // character.
  const aiming = world(contCtx(), { gump: 0x3C, groundItems: [corpse(0x301, 5, 5)],
                                    target: { active: 1, flag: 0 } });
  aiming.run("settings.autoOpenCorpses = true; autoOpenCorpses(scene);");
  deepEq(aiming.sent, [], "not while a target cursor is up");

  const hidden = world(contCtx(), { gump: 0x3C, groundItems: [corpse(0x301, 5, 5)], hidden: true });
  hidden.run("settings.autoOpenCorpses = true; autoOpenCorpses(scene);");
  deepEq(hidden.sent, [], "not while hidden");

  // …and a corpse skipped while guarded is NOT remembered, so it opens later.
  hidden.run("scene.player.hidden = false; autoOpenCorpses(scene);");
  deepEq(hidden.sent, ["use:769"], "revealing yourself opens it after all");
});

test("only a real corpse graphic auto-opens", () => {
  const ctx = world(contCtx(), { gump: 0x3C,
    groundItems: [{ serial: 0x401, g: 0x0E7C, c: 1, x: 5, y: 5, z: 0 }] });   // a chest on the floor
  ctx.run("settings.autoOpenCorpses = true; autoOpenCorpses(scene);");
  deepEq(ctx.sent, [], "0x2006 only — a chest at your feet does not fly open");
});

test("the remembered-corpse set is bounded, because ServUO recycles serials", () => {
  const ctx = world(contCtx(), { gump: 0x3C, groundItems: [] });
  ctx.run(`settings.autoOpenCorpses = true;
           for (let i = 1; i <= 600; i++) autoOpenedCorpses.add(i);
           autoOpenCorpses(scene);`);
  eq(ctx.run("autoOpenedCorpses.size"), 512, "capped at AUTO_CORPSE_MEMORY");
  eq(ctx.run("autoOpenedCorpses.has(1)"), false, "the OLDEST serials are the ones forgotten");
  eq(ctx.run("autoOpenedCorpses.has(600)"), true, "the newest are kept");
});

// ── loot-all and grabItem ──────────────────────────────────────────────────

test("Loot all takes a snapshot first, because each grab mutates the list it walks", () => {
  const ctx = world(contCtx(), {
    gump: 9,
    items: [{ serial: 0x201, g: 0x0F3F, amount: 1, x: 1, y: 1, hue: 0 },
            { serial: 0x202, g: 0x0F3F, amount: 1, x: 2, y: 2, hue: 0 },
            { serial: 0x203, g: 0x0F3F, amount: 1, x: 3, y: 3, hue: 0 }],
  });
  ctx.run(`openContainer(${BAG})`);
  winOf(ctx).body.querySelector(".loot-all").click();
  deepEq(ctx.sent, ["pickup:513", "drop:513:65535:65535:0:1280",
                    "pickup:514", "drop:514:65535:65535:0:1280",
                    "pickup:515", "drop:515:65535:65535:0:1280"],
         "every item, in order, as a lift/drop pair");
});

test("with no backpack there is nowhere to loot into, and it says so instead of half-doing it", () => {
  const ctx = world(contCtx(), { gump: 9, equip: [],
                                 items: [{ serial: 0x201, g: 0x0F3F, amount: 1, x: 1, y: 1, hue: 0 }] });
  ctx.run(`openContainer(${BAG})`);
  cells(ctx)[0].click();
  deepEq(ctx.sent, [], "no half-move — the lift alone would leave it on the cursor");
  ok(ctx.run("localJournal.some((l) => /No backpack/i.test(l.text))"), "and the journal says why");
});
