// The web/js test runner. `node web/test/run.js` — that is the whole gate step.
//
// No test framework, on purpose: this repo is near-zero-dep (CLAUDE.md), the
// core has ONE dependency and it is the protocol's zlib. A runner is a list, a
// loop and an exit code, and node ships the assertions. Adding jest/vitest here
// would put a few hundred packages behind a gate whose entire job is to be
// trustworthy.
//
// Writing a test:
//
//     // web/test/thing.test.js
//     const { newContext } = require("./harness.js");
//     const { test, ok, eq } = require("./run.js");
//
//     test("a pile draws twice", () => {
//       const ctx = newContext().load("00-state.js", "03-world.js");
//       ctx.run("...");
//       eq(ctx.run("itemPool.size"), 1, "one item pooled");
//     });
//
// `test` takes an async function too. Each file gets its own registry, each
// test its own vm context if it makes one — nothing leaks between them.
//
// Usage:  node web/test/run.js [file-or-substring ...]
//         node web/test/run.js --list

const fs = require("node:fs");
const path = require("node:path");
const assert = require("node:assert");

const HERE = __dirname;

// ── registry ───────────────────────────────────────────────────────────────
let pending = [];
let current = null;                 // the test being run, for assertion counting

function test(name, fn) {
  if (typeof fn !== "function") throw new Error(`test(${JSON.stringify(name)}) has no body`);
  pending.push({ name, fn });
}

// ── assertions ─────────────────────────────────────────────────────────────
// Every one counts itself, so the suite can report the number that matters: how
// many claims about the client held, not how many `test(...)` blocks ran.
const counted = (fn) => (...a) => {
  if (current) current.asserts++;
  return fn(...a);
};
const show = (v) => {
  try {
    const s = typeof v === "string" ? JSON.stringify(v) : require("node:util").inspect(v, { depth: 2, breakLength: 100 });
    return s.length > 300 ? s.slice(0, 297) + "..." : s;
  } catch { return String(v); }
};

const ok = counted((cond, msg) => {
  if (!cond) throw new assert.AssertionError({ message: msg || "expected a truthy value", actual: cond, expected: true, operator: "ok" });
});
const eq = counted((actual, expected, msg) => {
  if (!Object.is(actual, expected)) {
    throw new assert.AssertionError({
      message: `${msg || "not equal"}\n      expected: ${show(expected)}\n      actual:   ${show(actual)}`,
      actual, expected, operator: "===" });
  }
});
const ne = counted((actual, unexpected, msg) => {
  if (Object.is(actual, unexpected)) throw new assert.AssertionError({ message: `${msg || "should differ"} (both ${show(actual)})`, actual, expected: unexpected, operator: "!==" });
});
// Values built INSIDE the vm context carry that realm's Array/Object prototypes,
// and deepStrictEqual compares prototypes — so a page's `[1,2]` is never deep-
// strict-equal to a test's `[1,2]`. Rebuild plain containers in this realm first;
// everything else (class instances, DOM nodes, functions) is passed through and
// still compared by identity.
function local(v, depth = 0, seen = new Map()) {
  if (v === null || typeof v !== "object" || depth > 12) return v;
  if (seen.has(v)) return seen.get(v);            // a cycle, or the same object twice
  const tag = Object.prototype.toString.call(v);
  if (tag === "[object Array]") {
    // NOT v.map(): Array.prototype.map builds its result from the RECEIVER's
    // constructor, so mapping a vm array hands back another vm array and
    // nothing has been brought across at all.
    const out = [];
    seen.set(v, out);
    for (let i = 0; i < v.length; i++) out[i] = local(v[i], depth + 1, seen);
    return out;
  }
  if (tag === "[object Map]") {
    const out = new Map();
    seen.set(v, out);
    for (const [k, x] of v) out.set(local(k, depth + 1, seen), local(x, depth + 1, seen));
    return out;
  }
  if (tag === "[object Set]") {
    const out = new Set();
    seen.set(v, out);
    for (const x of v) out.add(local(x, depth + 1, seen));
    return out;
  }
  if (tag !== "[object Object]") return v;
  const proto = Object.getPrototypeOf(v);
  if (proto !== null && proto !== Object.prototype && !("constructor" in proto && proto.constructor.name === "Object")) return v;
  const out = {};
  seen.set(v, out);
  for (const k of Object.keys(v)) out[k] = local(v[k], depth + 1, seen);
  return out;
}

const deepEq = counted((actual, expected, msg) => {
  actual = local(actual); expected = local(expected);
  try { assert.deepStrictEqual(actual, expected); }
  catch (e) { throw new assert.AssertionError({ message: `${msg || "not deep-equal"}\n      expected: ${show(expected)}\n      actual:   ${show(actual)}`, actual, expected, operator: "deepStrictEqual" }); }
});
const near = counted((actual, expected, eps, msg) => {
  if (!(Math.abs(actual - expected) <= eps)) {
    throw new assert.AssertionError({ message: `${msg || "not within tolerance"}\n      expected: ${show(expected)} ±${eps}\n      actual:   ${show(actual)}`, actual, expected, operator: "near" });
  }
});
const between = counted((actual, lo, hi, msg) => {
  if (!(actual >= lo && actual <= hi)) {
    throw new assert.AssertionError({ message: `${msg || "out of range"}\n      expected: ${lo}..${hi}\n      actual:   ${show(actual)}`, actual, expected: `${lo}..${hi}`, operator: "between" });
  }
});
const includes = counted((haystack, needle, msg) => {
  const has = typeof haystack === "string" ? haystack.includes(needle)
            : haystack && typeof haystack.includes === "function" ? haystack.includes(needle)
            : haystack instanceof Set ? haystack.has(needle) : false;
  if (!has) throw new assert.AssertionError({ message: `${msg || "not found"}\n      looked for: ${show(needle)}\n      in:         ${show(haystack)}`, actual: haystack, expected: needle, operator: "includes" });
});
const throws = counted((fn, want, msg) => {
  let threw = null;
  try { fn(); } catch (e) { threw = e; }
  if (!threw) throw new assert.AssertionError({ message: `${msg || "expected a throw"} — nothing was thrown`, operator: "throws" });
  if (want) {
    const s = String(threw.message);
    const hit = want instanceof RegExp ? want.test(s) : s.includes(want);
    if (!hit) throw new assert.AssertionError({ message: `${msg || "wrong error"}\n      expected: ${show(want)}\n      actual:   ${show(s)}`, operator: "throws" });
  }
  return threw;
});
const fail = (msg) => { if (current) current.asserts++; throw new assert.AssertionError({ message: msg || "failed", operator: "fail" }); };

// ── running ────────────────────────────────────────────────────────────────
function discover(args) {
  const all = fs.readdirSync(HERE).filter((f) => f.endsWith(".test.js")).sort();
  if (!args.length) return all;
  const hits = all.filter((f) => args.some((a) => f.includes(a.replace(/^.*[/\\]/, "").replace(/\.test\.js$/, ""))));
  if (!hits.length) throw new Error(`web/test: nothing matches ${args.join(" ")} (have: ${all.join(", ")})`);
  return hits;
}

// Point at the line in the TEST file, not at run.js's own assertion plumbing.
function where(err, file) {
  const line = (err.stack || "").split("\n").find((l) => l.includes(file));
  return line ? line.trim().replace(/^at\s+/, "at ") : null;
}

async function main() {
  const args = process.argv.slice(2).filter((a) => a !== "--list");
  const listOnly = process.argv.includes("--list");
  const files = discover(args);
  const started = Date.now();
  let ran = 0, asserts = 0;
  const failures = [];

  // A web/js file that throws asynchronously (a fetch chain, an await in the
  // boot path) would otherwise vanish. It must not: a suite that reports green
  // over a client that died on load is worse than no suite at all.
  let unhandled = null;
  process.on("unhandledRejection", (err) => { unhandled = err; });

  for (const file of files) {
    pending = [];
    const full = path.join(HERE, file);
    try {
      delete require.cache[full];
      require(full);
    } catch (err) {
      failures.push({ file, name: "<loading the test file>", err });
      continue;
    }
    if (listOnly) { for (const t of pending) console.log(`${file}  ${t.name}`); continue; }
    for (const t of pending) {
      current = { asserts: 0 };
      try {
        await t.fn();
        if (unhandled) throw unhandled;
      } catch (err) {
        failures.push({ file, name: t.name, err });
      } finally {
        unhandled = null;
        ran++;
        asserts += current.asserts;
        current = null;
      }
    }
  }
  if (listOnly) return 0;

  const secs = ((Date.now() - started) / 1000).toFixed(1);
  if (failures.length) {
    console.error("");
    for (const f of failures) {
      console.error(`FAIL  ${f.file} — ${f.name}`);
      for (const line of String(f.err && f.err.message || f.err).split("\n")) console.error(`      ${line}`);
      const at = where(f.err, f.file) || where(f.err, "web/");
      if (at) console.error(`      ${at}`);
      console.error("");
    }
    console.error(`web/test: ${failures.length} of ${ran} tests FAILED ` +
                  `(${files.length} files, ${asserts} assertions, ${secs}s)`);
    return 1;
  }
  console.log(`web/test: ${ran} tests, ${asserts} assertions over ${files.length} files — ok (${secs}s)`);
  return 0;
}

module.exports = { test, ok, eq, ne, deepEq, near, between, includes, throws, fail, assert };

if (require.main === module) {
  main().then((code) => process.exit(code), (err) => {
    console.error("web/test: the runner itself failed");
    console.error(err && err.stack || err);
    process.exit(1);
  });
}
