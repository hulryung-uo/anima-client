// The macro runner (web/js/13-macros.js).
//
// A macro is a sequence, and sequences are where timing lives: a delay that
// gates the whole macro rather than just its own step, a wait-for-target that
// gives up instead of hanging, a fresh trigger that replaces whatever was
// running. None of it was covered, and none of it can be checked by reading —
// which is why it is driven here with the test's own clock and timer queue,
// where "50 ms later" is a statement rather than a hope.
const { newContext } = require("./harness.js");
const { test, ok, eq, deepEq } = require("./run.js");

function macCtx() {
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
    macros = []; stopMacro();
  `);
  ctx.set("__scene", { player: { serial: 9, x: 10, y: 10, z: 0, name: "Me", equip: [] },
                       mobiles: [], items: [], statics: [], contItems: [],
                       target: { active: 0, flag: 0 } });
  ctx.run("scene = __scene;");
  ctx.setNow(1000);
  ctx.go = (actions) => { ctx.set("__m", { actions }); ctx.run("runMacro(__m)"); };
  ctx.said = () => ctx.sent.filter((s) => s.startsWith("say:")).map((s) => s.slice(4));
  ctx.cursor = (up) => ctx.run(`scene.target = { active: ${up ? 1 : 0}, flag: 0 };`);
  return ctx;
}

// ── a plain sequence ───────────────────────────────────────────────────────

test("every step of a sequence runs, in order, in one go", () => {
  const ctx = macCtx();
  ctx.go([{ t: "say", text: "one" }, { t: "say", text: "two" }, { t: "say", text: "three" }]);
  deepEq(ctx.said(), ["one", "two", "three"], "all three, in the order written");
  eq(ctx.run("macroRun"), null, "and the run is finished, not parked");
});

test("an empty macro does nothing and leaves nothing running", () => {
  const ctx = macCtx();
  ctx.go([]);
  deepEq(ctx.sent, [], "nothing sent");
  eq(ctx.run("macroRun"), null, "nothing armed");
});

test("a macro saved before sequences existed still runs its one step", () => {
  const ctx = macCtx();
  ctx.set("__m", { action: { t: "say", text: "old blob" } });     // the pre-sequence shape
  ctx.run("runMacro(__m)");
  deepEq(ctx.said(), ["old blob"], "read, not migrated — an old blob keeps working");
});

test("a verb this build has never heard of is skipped, not fatal", () => {
  const ctx = macCtx();
  ctx.go([{ t: "say", text: "before" }, { t: "teleport-to-luna" }, { t: "say", text: "after" }]);
  deepEq(ctx.said(), ["before", "after"], "the rest of the sequence still ran");
});

// ── delay gates the whole macro, not the step it sits on ───────────────────

test("a delay stops the macro there and lets it go when the time is up", () => {
  const ctx = macCtx();
  ctx.go([{ t: "say", text: "before" }, { t: "delay", ms: 500 }, { t: "say", text: "after" }]);
  deepEq(ctx.said(), ["before"], "the steps after the delay have not run");
  ctx.advance(499);
  deepEq(ctx.said(), ["before"], "…not a moment early");
  ctx.advance(2);
  deepEq(ctx.said(), ["before", "after"], "and then it continues");
  eq(ctx.run("macroRun"), null, "finished");
});

test("the delay does not cost the step it sits on a tick of its own", () => {
  // ClassicUO sets one `_nextTimer` for the whole macro (MacroManager.cs:1167-1175)
  // rather than parking on the Delay step, so 500 ms means 500 ms — not 500 plus
  // a scheduler tick.
  const ctx = macCtx();
  ctx.go([{ t: "delay", ms: 500 }, { t: "say", text: "go" }]);
  ctx.advance(500);
  deepEq(ctx.said(), ["go"], "exactly on time");
});

test("two delays add up rather than sharing one deadline", () => {
  const ctx = macCtx();
  ctx.go([{ t: "delay", ms: 200 }, { t: "say", text: "a" },
          { t: "delay", ms: 300 }, { t: "say", text: "b" }]);
  ctx.advance(200); deepEq(ctx.said(), ["a"], "first");
  ctx.advance(299); deepEq(ctx.said(), ["a"], "still waiting out the second");
  ctx.advance(2);   deepEq(ctx.said(), ["a", "b"], "second");
});

test("a trailing delay ends the macro rather than leaving it parked", () => {
  const ctx = macCtx();
  ctx.go([{ t: "say", text: "x" }, { t: "delay", ms: 100 }]);
  ok(ctx.run("macroRun"), "parked on the delay for now");
  ctx.advance(101);
  eq(ctx.run("macroRun"), null, "and then it is done");
});

// ── wait for target ────────────────────────────────────────────────────────

test("wait-for-target holds the macro until the cursor comes up", () => {
  const ctx = macCtx();
  ctx.go([{ t: "say", text: "cast" }, { t: "waittarget" }, { t: "say", text: "aimed" }]);
  deepEq(ctx.said(), ["cast"], "waiting");
  ctx.advance(50);
  deepEq(ctx.said(), ["cast"], "still waiting — polling, not spinning");
  ctx.cursor(true);
  ctx.advance(50);
  deepEq(ctx.said(), ["cast", "aimed"], "the cursor released it");
});

test("wait-for-target gives up after ClassicUO's 5 s and carries on anyway", () => {
  // MacroManager.cs:1129-1146 — it never fails the macro, it just stops waiting.
  const ctx = macCtx();
  ctx.go([{ t: "waittarget" }, { t: "say", text: "carried on" }]);
  ctx.advance(4900);
  deepEq(ctx.said(), [], "not yet");
  ctx.advance(200);
  deepEq(ctx.said(), ["carried on"], "gave up and continued rather than hanging");
});

test("a cursor already up does not cost the macro a tick", () => {
  const ctx = macCtx();
  ctx.cursor(true);
  ctx.go([{ t: "waittarget" }, { t: "say", text: "straight through" }]);
  deepEq(ctx.said(), ["straight through"], "no wait at all");
});

test("two waits in one macro each get their own deadline", () => {
  const ctx = macCtx();
  ctx.go([{ t: "waittarget" }, { t: "say", text: "a" },
          { t: "waittarget" }, { t: "say", text: "b" }]);
  ctx.cursor(true); ctx.advance(50);
  deepEq(ctx.said(), ["a", "b"], "both released by the cursor");
  // …and the second must not inherit the first's expired deadline.
  const ctx2 = macCtx();
  ctx2.go([{ t: "waittarget" }, { t: "say", text: "a" }, { t: "waittarget" }, { t: "say", text: "b" }]);
  ctx2.advance(5100);                    // first one times out
  deepEq(ctx2.said(), ["a"], "the first gave up; the second is now waiting afresh");
  ctx2.advance(5100);
  deepEq(ctx2.said(), ["a", "b"], "and gets its own full 5 s before it does");
});

// ── one macro at a time ────────────────────────────────────────────────────

test("a fresh trigger replaces whatever was running", () => {
  // ClassicUO SetMacroToExecute (MacroManager.cs:355) — the new one wins.
  const ctx = macCtx();
  ctx.go([{ t: "say", text: "first" }, { t: "delay", ms: 1000 }, { t: "say", text: "never" }]);
  deepEq(ctx.said(), ["first"], "parked mid-sequence");
  ctx.go([{ t: "say", text: "second" }]);
  ctx.advance(2000);
  deepEq(ctx.said(), ["first", "second"], "the interrupted tail never fired");
});

test("stopping a parked macro clears it, and no later wake-up finishes it", () => {
  const ctx = macCtx();
  ctx.go([{ t: "delay", ms: 500 }, { t: "say", text: "never" }]);
  ctx.run("stopMacro()");
  eq(ctx.run("macroRun"), null, "stopped");
  eq(ctx.run("macroTimer"), 0, "…and the timer handle with it");
  ctx.advance(2000);
  deepEq(ctx.said(), [], "nothing woke up later to finish it");
});

// What actually makes a stop safe is not the clearTimeout in `stopMacro` — with
// today's callers a stray timer is inert anyway — but the `while (macroRun)`
// gate the timer lands on. Deleting the clearTimeout leaves the suite green;
// deleting the gate would not, so the gate is what is pinned here. Driven by
// calling `pumpMacro` straight out, which is exactly what a stray timer does.
test("a wake-up that arrives after the macro was stopped does nothing at all", () => {
  const ctx = macCtx();
  ctx.go([{ t: "delay", ms: 500 }, { t: "say", text: "never" }]);
  ctx.run("stopMacro()");
  ctx.setNow(ctx.run("performance.now()") + 5000);   // well past the deadline it had
  ctx.run("pumpMacro()");
  deepEq(ctx.said(), [], "no step ran");
  eq(ctx.run("macroTimer"), 0, "and it did not re-arm itself");
});

test("a wake-up cannot run a step before its time, whoever delivers it", () => {
  const ctx = macCtx();
  ctx.go([{ t: "delay", ms: 500 }, { t: "say", text: "on time" }]);
  ctx.run("pumpMacro()");                            // an early, spurious wake-up
  deepEq(ctx.said(), [], "the deadline still holds");
  ctx.advance(501);
  deepEq(ctx.said(), ["on time"], "and the step runs when it was due, once");
});

// ── what triggers one ──────────────────────────────────────────────────────

test("a mouse-button macro matches its button and its exact modifiers", () => {
  const ctx = macCtx();
  ctx.set("__macros", [{ id: 1, button: 3, ctrl: true, actions: [{ t: "say", text: "x" }] },
                       { id: 2, button: 3, actions: [{ t: "say", text: "y" }] }]);
  ctx.run("macros = __macros;");
  const at = (button, mods) => {
    ctx.set("__e", Object.assign({ ctrlKey: false, altKey: false, shiftKey: false }, mods));
    const m = ctx.run(`macroForButton(${button}, __e)`);
    return m && m.id;
  };
  eq(at(3, { ctrlKey: true }), 1, "Ctrl+button 3");
  eq(at(3, {}), 2, "plain button 3 is a different macro");
  eq(at(3, { ctrlKey: true, shiftKey: true }), null, "an extra modifier is not a match");
  eq(at(4, { ctrlKey: true }), null, "another button is not a match");
});

test("a wheel macro tells up from down", () => {
  const ctx = macCtx();
  ctx.set("__macros", [{ id: 1, wheel: "up", actions: [] }, { id: 2, wheel: "down", actions: [] }]);
  ctx.run("macros = __macros;");
  ctx.set("__e", { ctrlKey: false, altKey: false, shiftKey: false });
  eq(ctx.run("macroForWheel(true, __e).id"), 1, "up");
  eq(ctx.run("macroForWheel(false, __e).id"), 2, "down");
});

test("a key macro never steals a reserved key, and a wheel/button one never answers a key", () => {
  const ctx = macCtx();
  // F5 / F6 / F7: not movement, not one of the client's own hotkeys.
  ctx.set("__macros", [{ id: 1, key: "F5", actions: [] },
                       { id: 2, key: "F6", wheel: "up", actions: [] },
                       { id: 3, key: "F7", button: 3, actions: [] }]);
  ctx.run("macros = __macros;");
  const press = (code, mods) => {
    ctx.set("__e", Object.assign({ code, ctrlKey: false, altKey: false, shiftKey: false }, mods));
    const m = ctx.run("macroFor(__e)");
    return m && m.id;
  };
  eq(press("F5"), 1, "an ordinary key binding answers");
  eq(press("F6"), null, "a wheel-bound macro is not a key binding");
  eq(press("F7"), null, "nor is a button-bound one");
  for (const code of ctx.run("[...RESERVED_CODES]").slice(0, 4)) {
    ctx.set("__macros2", [{ id: 9, key: code, actions: [] }]);
    ctx.run("macros = __macros2;");
    eq(press(code), null, `${code} is reserved and cannot be bound`);
  }
});
