// The target cursor in web/js/12-input.js: which reticle a 0x6C flag picks, what
// the banner says, when the criminal-action question is asked, and what each of
// the target verbs (F / Shift+F / V / X / Shift+X) actually puts on the wire.
//
// This is the half of the client where being wrong is expensive and invisible:
// a harmful cursor answered on an Innocent is how a blue character goes
// criminal, and there is no undo. ClassicUO gates that question on BOTH sides —
// TargetManager.cs:266 (`serial != Player` and OUR notoriety Innocent/Ally) and
// :272-281 (the TARGET's NotorietyFlag against the cursor's TargetType) — so
// every one of those branches is pinned here rather than discovered live.
const { newContext } = require("./harness.js");
const { test, ok, eq, deepEq } = require("./run.js");

// The whole client, in one scope, with the page's real <body>. `sendInput`
// POSTs to /input (13-macros.js), so the commands the client issues are read
// off the fetch stub — nothing in web/js is replaced.
function uiCtx() {
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
  `);
  // The three reticles are painted to an offscreen canvas and read back as a
  // data URI — which the fake canvas answers identically for all three. Give
  // them distinct sentinels instead: this asserts WHICH reticle the flag picks,
  // not what it looks like.
  ctx.run(`CURSOR_ARROW = "arrow"; CURSOR_TARGET = "amber";
           CURSOR_TARGET_WAR = "red"; CURSOR_TARGET_GOOD = "blue";`);
  return ctx;
}

// A scene with a pending target cursor of the given 0x6C type and one other
// mobile to aim at. `noto` is OUR notoriety, `theirs` is the target's.
function aiming(ctx, { flag = 0, noto = 1, theirs = 1, war = false, active = 1 } = {}) {
  ctx.set("__scene", {
    player: { serial: 9, x: 5, y: 5, z: 0, noto, name: "Me", equip: [] },
    mobiles: [{ serial: 0x101, x: 6, y: 5, z: 0, noto: theirs, name: "Bob" }],
    items: [], statics: [], war,
    target: { active, flag },
  });
  ctx.run("scene = __scene; updateTargetUI();");
  return ctx;
}
const hint = (ctx) => ctx.document.getElementById("targethint");
const crim = (ctx) => ctx.document.getElementById("crimconfirm");
// A real keydown carries all four modifier flags; 13-macros.js `modsMatch`
// compares them strictly, so a test that leaves them undefined would never
// match a macro and would silently skip the "a macro owns this combo" branch.
const press = (ctx, code, init = {}) =>
  ctx.fire("window", "keydown", Object.assign(
    { code, target: ctx.document.body, ctrlKey: false, altKey: false, metaKey: false, shiftKey: false },
    init));

// ── the reticle and the banner ─────────────────────────────────────────────

test("each 0x6C cursor type picks its own reticle and says so in words", () => {
  // ClassicUO tints the aura by TargetType (GameCursor.cs:361-379): Neutral
  // 0x03b2, Harmful 0x0023 red, Beneficial 0x005A blue.
  for (const [flag, reticle, words, cls] of [
    [0, "amber", "Select a target…", ""],
    [1, "red", "Select a HARMFUL target…", "harmful"],
    [2, "blue", "Select a BENEFICIAL target…", "beneficial"],
  ]) {
    const ctx = aiming(uiCtx(), { flag });
    eq(ctx.run("targetReticle()"), reticle, `flag ${flag} → the ${reticle} reticle`);
    eq(ctx.run("app.canvas.style.cursor"), reticle, `…and the canvas actually wears it`);
    ok(hint(ctx).textContent.startsWith(words), `flag ${flag} banner reads "${words}"`);
    eq(hint(ctx).className, cls, `flag ${flag} banner class`);
    eq(hint(ctx).style.display, "block", "the banner is up while a cursor is pending");
  }
});

test("a scene with no `flag` field falls back to the war-mode guess", () => {
  // The pre-`flag` server: all we knew was whether we were in war mode.
  const ctx = uiCtx();
  ctx.set("__scene", { player: { serial: 9, x: 5, y: 5, equip: [] }, mobiles: [], items: [],
                       war: true, target: { active: 1 } });
  ctx.run("scene = __scene; updateTargetUI();");
  eq(ctx.run("targetCursorFlag()"), null, "no flag on the wire");
  eq(ctx.run("targetReticle()"), "red", "war mode still colours it red");
  eq(hint(ctx).className, "", "…but the banner cannot claim it is harmful");
  ctx.run("scene.war = false; updateTargetUI();");
  eq(ctx.run("targetReticle()"), "amber", "out of war, neutral");
});

test("Esc hides the cursor UI locally and a fresh request brings it back", () => {
  const ctx = aiming(uiCtx(), { flag: 1 });
  eq(ctx.run("targetingActive()"), true, "a cursor is pending");
  ctx.run("endTargetUI()");                      // what Esc / an answered click calls
  eq(ctx.run("targetingActive()"), false, "hidden locally");
  eq(hint(ctx).style.display, "none", "banner gone");
  eq(ctx.run("app.canvas.style.cursor"), "arrow", "cursor back to the arrow");
  // Still active:1 in the scene — the server has not caught up. Re-polling must
  // NOT un-hide it, or Esc would flicker back on every poll.
  ctx.run("updateTargetUI()");
  eq(ctx.run("targetingActive()"), false, "a re-poll of the same request stays hidden");
  // A 1→0→1 edge is a NEW request and shows again.
  ctx.run("scene.target.active = 0; updateTargetUI(); scene.target.active = 1; updateTargetUI();");
  eq(ctx.run("targetingActive()"), true, "the next 0x6C shows the cursor again");
});

test("the gold hover highlight only tints while a cursor is up, and puts the tint back", () => {
  const ctx = aiming(uiCtx(), { flag: 1 });
  ctx.run("__sp = new PIXI.Sprite(); __sp.tint = 0x00ff00;");
  ctx.run("targetHighlightOn(__sp)");
  eq(ctx.run("__sp.tint"), 0xffd24a, "tinted gold while targeting");
  ctx.run("targetHighlightOff(__sp)");
  eq(ctx.run("__sp.tint"), 0x00ff00, "the sprite's own tint came back");
  // Ending the cursor drops a live highlight too — otherwise a mobile stays
  // gold forever after the target resolves.
  ctx.run("targetHighlightOn(__sp); endTargetUI();");
  eq(ctx.run("__sp.tint"), 0x00ff00, "endTargetUI clears the highlight");
  ctx.run("targetHighlightOn(__sp)");
  eq(ctx.run("__sp.tint"), 0x00ff00, "…and nothing highlights with no cursor pending");
});

// ── the criminal-action question (ClassicUO TargetManager.cs:263-300) ───────

test("a harmful cursor on an Innocent asks before it flags you", () => {
  const ctx = aiming(uiCtx(), { flag: 1, noto: 1, theirs: 1 });
  eq(ctx.run("confirmCriminalTarget(0x101, false)"), false, "the click is held, not answered");
  deepEq(ctx.sent, [], "nothing on the wire yet — this is the whole point");
  eq(crim(ctx).style.display, "block", "the question is up");
  eq(ctx.document.getElementById("crimconfirm-who").textContent, "Target: Bob", "it names who");
  ctx.document.getElementById("crimconfirm-yes").click();
  deepEq(ctx.sent, ["target:257"], "Yes answers the cursor the click was for");
  eq(crim(ctx).style.display, "none", "and the question goes away");
});

test("No leaves the cursor open and sends nothing — ClassicUO's QuestionGump does not cancel", () => {
  const ctx = aiming(uiCtx(), { flag: 1, noto: 1, theirs: 1 });
  ctx.run("confirmCriminalTarget(0x101, false)");
  ctx.document.getElementById("crimconfirm-no").click();
  deepEq(ctx.sent, [], "nothing sent");
  eq(crim(ctx).style.display, "none", "question dismissed");
  eq(ctx.run("targetingActive()"), true, "the cursor is still waiting for a target");
});

test("every gate ClassicUO checks is a case that must NOT ask", () => {
  const cases = [
    ["yourself (TargetManager.cs:266 `serial != Player`)", { flag: 1, noto: 1 }, 9, false],
    ["an item (`SerialHelper.IsMobile`)", { flag: 1, noto: 1, theirs: 1 }, 0x101, true],
    ["already gray — our own noto 3", { flag: 1, noto: 3, theirs: 1 }, 0x101, false],
    ["already a murderer — our own noto 6", { flag: 1, noto: 6, theirs: 1 }, 0x101, false],
    ["a neutral cursor", { flag: 0, noto: 1, theirs: 1 }, 0x101, false],
    ["a harmful cursor on a Criminal(4)", { flag: 1, noto: 1, theirs: 4 }, 0x101, false],
    ["a harmful cursor on a Murderer(6)", { flag: 1, noto: 1, theirs: 6 }, 0x101, false],
    ["a beneficial cursor on an Innocent", { flag: 2, noto: 1, theirs: 1 }, 0x101, false],
    ["a mobile we cannot see", { flag: 1, noto: 1, theirs: 1 }, 0x999, false],
  ];
  for (const [why, opts, serial, isItem] of cases) {
    const ctx = aiming(uiCtx(), opts);
    eq(ctx.run(`confirmCriminalTarget(${serial}, ${isItem})`), true, `no question for ${why}`);
    eq(crim(ctx).style.display !== "block", true, `…and no modal for ${why}`);
  }
});

test("our own notoriety 0 (Unknown) is treated as Innocent — the side that still warns", () => {
  // 08-overlays.js leans on 0 to colour your own name blue, but measured live a
  // GM character reported 3. Only `> 2` is safe to skip, so 0 must still ask.
  const ctx = aiming(uiCtx(), { flag: 1, noto: 0, theirs: 1 });
  eq(ctx.run("confirmCriminalTarget(0x101, false)"), false, "an unknown-noto caster is still warned");
  const ally = aiming(uiCtx(), { flag: 1, noto: 2, theirs: 1 });
  eq(ally.run("confirmCriminalTarget(0x101, false)"), false, "…and so is an Ally(2)");
});

test("the beneficial half is off by default and works when switched on", () => {
  // ClassicUO Profile.cs:106-107 — EnabledCriminalActionQuery defaults true,
  // EnabledBeneficialCriminalActionQuery defaults false.
  eq(uiCtx().run("SETTINGS_DEFAULTS.criminalQuery"), true, "harmful query on by default");
  eq(uiCtx().run("SETTINGS_DEFAULTS.beneficialCriminalQuery"), false, "beneficial query off by default");
  for (const theirs of [3, 4, 6]) {
    const off = aiming(uiCtx(), { flag: 2, noto: 1, theirs });
    eq(off.run("confirmCriminalTarget(0x101, false)"), true, `noto ${theirs}: silent while the option is off`);
    const on = aiming(uiCtx(), { flag: 2, noto: 1, theirs });
    on.run("settings.beneficialCriminalQuery = true;");
    eq(on.run("confirmCriminalTarget(0x101, false)"), false, `noto ${theirs}: asks once switched on`);
  }
  // Ally(2) and Enemy(5) are not in ClassicUO's beneficial set.
  for (const theirs of [1, 2, 5]) {
    const on = aiming(uiCtx(), { flag: 2, noto: 1, theirs });
    on.run("settings.beneficialCriminalQuery = true;");
    eq(on.run("confirmCriminalTarget(0x101, false)"), true, `noto ${theirs} never flags a beneficial cast`);
  }
});

test("turning the harmful query off answers the cursor with no question at all", () => {
  const ctx = aiming(uiCtx(), { flag: 1, noto: 1, theirs: 1 });
  ctx.run("settings.criminalQuery = false;");
  eq(ctx.run("confirmCriminalTarget(0x101, false)"), true, "the click goes straight through");
  deepEq(ctx.sent, [], "confirmCriminalTarget itself never sends — the caller does");
});

// ── last target · target self · bandage · attack last ──────────────────────

test("F with no cursor open is silent — ClassicUO TargetManager.TargetLast():466", () => {
  const ctx = aiming(uiCtx(), { flag: 0, active: 0 });
  ctx.run("lastTargetInfo = { serial: 0x101 };");
  press(ctx, "KeyF");
  deepEq(ctx.sent, [], "nothing sent with no cursor to answer");
});

test("F with a cursor but nothing remembered says so instead of sending junk", () => {
  // ClassicUO would send its cleared buffer (serial 0, graphic 0xFFFF).
  const ctx = aiming(uiCtx(), { flag: 0 });
  eq(ctx.run("lastTargetInfo"), null, "nothing remembered yet");
  press(ctx, "KeyF");
  deepEq(ctx.sent, [], "no junk reply on the wire");
  ok(ctx.run("localJournal.some((l) => /No last target/.test(l.text))"), "the journal says why");
});

test("F replays an entity target, and a tile target as the same targetxy bytes", () => {
  const ctx = aiming(uiCtx(), { flag: 0 });
  ctx.run("rememberTargetEntity(0x101)");
  press(ctx, "KeyF");
  deepEq(ctx.sent, ["target:257"], "the remembered mobile");
  eq(ctx.run("targetingActive()"), false, "answering closes the cursor UI");

  const t = aiming(uiCtx(), { flag: 0 });
  t.run("rememberTargetTile(1000, 2000, 7, 0x0CE0)");
  press(t, "KeyF");
  deepEq(t.sent, ["targetxy:1000:2000:7:3296"], "a static replays x:y:z:graphic verbatim");
  // A LAND tile is the same shape with graphic 0 (ClassicUO SetLand vs SetStatic).
  const l = aiming(uiCtx(), { flag: 0 });
  l.run("rememberTargetTile(3, 4, -5, 0)");
  press(l, "KeyF");
  deepEq(l.sent, ["targetxy:3:4:-5:0"], "land keeps its negative z");
});

test("a self-target is never recorded as the last target (TargetManager.cs:262)", () => {
  const ctx = aiming(uiCtx(), { flag: 0 });
  ctx.run("rememberTargetEntity(0x101)");
  ctx.run("rememberTargetEntity(9)");             // our own serial
  deepEq(ctx.run("lastTargetInfo"), { serial: 0x101 }, "the mob we were fighting survived");
  // V goes through the same rule.
  press(ctx, "KeyV");
  deepEq(ctx.sent, ["target:9"], "V answers with our own serial");
  deepEq(ctx.run("lastTargetInfo"), { serial: 0x101 }, "…and still did not overwrite it");
});

test("V needs a cursor and a player, and Shift+V is not V", () => {
  const off = aiming(uiCtx(), { flag: 0, active: 0 });
  press(off, "KeyV");
  deepEq(off.sent, [], "no cursor, no reply");
  const ctx = aiming(uiCtx(), { flag: 0 });
  press(ctx, "KeyV", { shiftKey: true });
  deepEq(ctx.sent, [], "Shift+V is left for a macro / the shift-run modifier");
});

test("Shift+F attacks the remembered target, and falls back to the server's own last attack", () => {
  const ctx = aiming(uiCtx(), { flag: 0, active: 0 });
  press(ctx, "KeyF", { shiftKey: true });
  deepEq(ctx.sent, ["attacklast"], "nothing remembered → the server replays World::last_attack");
  ctx.run("rememberTargetEntity(0x101)");
  press(ctx, "KeyF", { shiftKey: true });
  eq(ctx.sent[1], "attack:257", "…once something is remembered, attack that");
  // A TILE is not attackable: lastTargetInfo without a serial must fall back.
  ctx.run("rememberTargetTile(1, 2, 3, 4)");
  press(ctx, "KeyF", { shiftKey: true });
  eq(ctx.sent[2], "attacklast", "a remembered tile is not an attack target");
});

// A backpack (layer 21) holding the given contents, the shape `cbFind` walks.
function carrying(ctx, contents) {
  ctx.run("scene.player.equip = [{ serial: 0x500, layer: 21, g: 0x0E75 }];");
  ctx.set("__cont", contents.map((c) => Object.assign({ cont: 0x500 }, c)));
  ctx.run("scene.contItems = __cont;");
}

test("X bandages self through 0xBF/0x2C, and says so when there are none", () => {
  // ClassicUO MacroType.BandageSelf (MacroManager.cs:1324-1337) is
  // Send_TargetSelectedObject(bandage, target) — one packet, no cursor.
  const ctx = aiming(uiCtx(), { flag: 0, active: 0 });
  press(ctx, "KeyX");
  deepEq(ctx.sent, [], "no bandage → nothing on the wire");
  ok(ctx.run("localJournal.some((l) => /no bandages/i.test(l.text))"), "the journal says why");
  carrying(ctx, [{ serial: 0x701, g: 0x0E21, amount: 10, hue: 0 }]);
  press(ctx, "KeyX");
  deepEq(ctx.sent, ["bandage:1793"], "the clean bandage in the pack");
});

test("a BLOODY bandage is not a bandage (PlayerMobile.FindBandage: 0x0E21 only)", () => {
  const ctx = aiming(uiCtx(), { flag: 0, active: 0 });
  carrying(ctx, [{ serial: 0x701, g: 0x0E20, amount: 10, hue: 0 }]);   // used/bloody
  press(ctx, "KeyX");
  deepEq(ctx.sent, [], "0x0E20 does not answer the hotkey");
});

test("Shift+X bandages the last target, and refuses without one", () => {
  const ctx = aiming(uiCtx(), { flag: 0, active: 0 });
  carrying(ctx, [{ serial: 0x701, g: 0x0E21, amount: 3, hue: 0 }]);
  press(ctx, "KeyX", { shiftKey: true });
  deepEq(ctx.sent, [], "no last target → nothing sent");
  ctx.run("rememberTargetEntity(0x101)");
  press(ctx, "KeyX", { shiftKey: true });
  deepEq(ctx.sent, ["bandage:1793:257"], "one packet carrying both serials");
  // A remembered TILE cannot be bandaged either.
  ctx.run("rememberTargetTile(1, 2, 3, 4)");
  press(ctx, "KeyX", { shiftKey: true });
  deepEq(ctx.sent, ["bandage:1793:257"], "a tile is refused, exactly like Shift+F's fallback arm");
});

test("the hotkeys stay out of the way of typing, modifiers and user macros", () => {
  const cases = [
    ["while the chat bar is open", (c) => c.run("chatting = true"), {}],
    ["on a key repeat", () => {}, { repeat: true }],
    ["with Ctrl held", () => {}, { ctrlKey: true }],
    ["with Alt held", () => {}, { altKey: true }],
    ["with Meta held", () => {}, { metaKey: true }],
  ];
  for (const [why, setup, init] of cases) {
    const ctx = aiming(uiCtx(), { flag: 0 });
    ctx.run("rememberTargetEntity(0x101)");
    setup(ctx);
    press(ctx, "KeyF", init);
    deepEq(ctx.sent, [], `F is not consumed ${why}`);
  }
  // Focus in a text field.
  const typing = aiming(uiCtx(), { flag: 0 });
  typing.run("rememberTargetEntity(0x101)");
  const input = typing.document.createElement("input");
  typing.document.body.appendChild(input);
  typing.fire("window", "keydown", { code: "KeyF", target: input });
  deepEq(typing.sent, [], "F typed into an <input> reaches the field, not the game");
  // A user macro bound to plain F owns the combo (13-macros.js macroFor).
  const macro = aiming(uiCtx(), { flag: 0 });
  macro.run("rememberTargetEntity(0x101); macros = [{ key: 'KeyF', acts: [] }];");
  press(macro, "KeyF");
  deepEq(macro.sent, [], "a macro bound to F wins over the built-in verb");
});

// ── the z a map static must answer with (ClassicUO TargetManager.Target) ────

test("a SURFACE static answers with z + tiledata height, anything else with its own z", () => {
  // `if (Version >= CV_7090 && itemData.IsSurface) z += itemData.Height` — and
  // ServUO undoes it before validating, so dropping it fails every surface
  // target. `pf` bit 1 is TileFlag.Surface, `h` the tiledata height.
  const ctx = uiCtx();
  eq(ctx.run("staticTargetZ({ z: 10, pf: 2, h: 6 })"), 16, "a table: Surface, height 6");
  eq(ctx.run("staticTargetZ({ z: 0, pf: 2, h: 5 })"), 5, "stone stairs: Surface, height 5");
  eq(ctx.run("staticTargetZ({ z: 10, pf: 2, h: 0 })"), 10, "a cave floor: Surface, height 0 — unchanged");
  eq(ctx.run("staticTargetZ({ z: 10, pf: 0, h: 20 })"), 10, "a tree is Impassable, not Surface");
  eq(ctx.run("staticTargetZ({ z: -8, pf: 2, h: 3 })"), -5, "negative z still adds");
  eq(ctx.run("staticTargetZ({ z: 4 })"), 4, "a static past PATH_RADIUS reports no flags and is left alone");
});

test("clicking a static answers the cursor with the RAW map graphic, not the drawn one", () => {
  // ClassicUO sends `Static.Graphic`, which SetGraphicBySeason has already
  // overwritten — so a winter tree fails ServUO's GetStaticTiles check there.
  // `st.g` is the unremapped id; `st.dg` is what we drew.
  const ctx = aiming(uiCtx(), { flag: 0 });
  ctx.run(`__sp = { _st: { x: 100, y: 200, z: 2, g: 0x0CE0, dg: 0x0D97, pf: 2, h: 4 }, height: 40 };`);
  ctx.run("onStaticPointerDown(__sp, { button: 0, clientX: 700, clientY: 100 })");
  deepEq(ctx.sent, ["targetxy:100:200:6:3296"], "raw graphic 0x0CE0, z 2 + height 4");
  deepEq(ctx.run("lastTargetInfo"), { g: 0x0CE0, x: 100, y: 200, z: 6 },
         "…and the SAME adjusted z is remembered, so a replay resends identical bytes");
  eq(ctx.run("targetingActive()"), false, "the cursor is answered");
});

test("a static drawn over the avatar does not steal the self-target under it", () => {
  // Standing under a tree, every self-cast would land on the tree: our own body
  // is not a click target, so nothing would win the click back.
  const ctx = aiming(uiCtx(), { flag: 0 });
  ctx.run(`__sp = { _st: { x: 100, y: 200, z: 2, g: 0x0CE0, pf: 0, h: 0 }, height: 40 };`);
  // Dead centre of the 800x600 canvas — inside clickIsSelfBand.
  ctx.run("onStaticPointerDown(__sp, { button: 0, clientX: 400, clientY: 300 })");
  deepEq(ctx.sent, [], "the click is left alone for the canvas's self-band handler");
  eq(ctx.run("targetConsumedAt"), 0, "…and not marked as consumed");
  eq(ctx.run("targetingActive()"), true, "the cursor is still open");
});

test("multi placement and house design own the static click (ClassicUO SendMultiTarget)", () => {
  // Answering a house placement with a static makes ServUO validate the tile
  // and cancel the whole target on any mismatch, so these fall through to the
  // canvas handler, which sends graphic 0.
  for (const key of ["placement", "houseDesign"]) {
    const ctx = aiming(uiCtx(), { flag: 0 });
    ctx.run(`scene.${key} = { multiId: 1, xOff: 0, yOff: 0, zOff: 0, tiles: [], parts: [] };`);
    ctx.run(`__sp = { _st: { x: 100, y: 200, z: 2, g: 0x0CE0, pf: 2, h: 4 }, height: 40 };`);
    ctx.run("onStaticPointerDown(__sp, { button: 0, clientX: 700, clientY: 100 })");
    deepEq(ctx.sent, [], `scene.${key} keeps the click`);
  }
});

test("the right button never answers a target cursor from a static", () => {
  const ctx = aiming(uiCtx(), { flag: 0 });
  ctx.run(`__sp = { _st: { x: 1, y: 2, z: 0, g: 5, pf: 0, h: 0 }, height: 10 };`);
  ctx.run("onStaticPointerDown(__sp, { button: 2, clientX: 700, clientY: 100 })");
  deepEq(ctx.sent, [], "a static has no context menu — the right button still steers");
  eq(ctx.run("targetingActive()"), true, "the cursor is untouched");
});
