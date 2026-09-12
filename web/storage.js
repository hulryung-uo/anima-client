// Renderer preferences only. Accounts and credentials have their own store.
// One envelope makes a restore atomic; legacy per-key values remain untouched.
const preferenceStorage = (() => {
  const KEY = "anima.preferences.v1", LIMIT = 1024 * 1024;
  const rules = new Map(), pending = new Map();
  let error = "", listener = () => {}, lastKnown = null;
  const object = v => !!v && typeof v === "object" && !Array.isArray(v);
  const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
  function register(key, normalize) { rules.set(key, normalize); }
  function json(key, normalize) {
    register(key, raw => {
      const value = JSON.parse(raw), next = normalize(value);
      return { raw: next == null ? null : JSON.stringify(next), invalid: next == null || !same(value, next) };
    });
  }
  function read() {
    try {
      const raw = localStorage.getItem(KEY);
      if (raw === null) {
        const values = {};
        for (const key of rules.keys()) {
          const value = localStorage.getItem(key);
          if (value !== null) values[key] = value;
        }
        lastKnown = { doc: { version: 1, values }, source: { legacy: values } }; return lastKnown;
      }
      if (raw.length > LIMIT * 4) return { fatal: "Saved settings are too large to read safely." };
      try {
        const doc = JSON.parse(raw);
        if (!object(doc) || doc.version !== 1 || !object(doc.values)) throw new Error();
        lastKnown = { doc, source: { envelope: raw } }; return lastKnown;
      } catch (_) {
        return { fatal: "Saved settings are damaged or from an unsupported version.", source: { envelope: raw } };
      }
    } catch (_) { return { doc: lastKnown?.doc, fatal: "Device storage is unavailable. Changes will last only for this session." }; }
  }
  function checked(key, raw) {
    if (raw == null) return { raw: null, invalid: false };
    if (typeof raw !== "string" || raw.length > LIMIT) return { raw: null, invalid: true };
    try { return rules.get(key)(raw); }
    catch (_) { return { raw: null, invalid: true }; }
  }
  function inspect(snapshot = read()) {
    const values = {}, damaged = [];
    for (const key of rules.keys()) {
      const item = checked(key, snapshot.doc?.values[key]);
      if (item.invalid) damaged.push(key);
      if (item.raw !== null) values[key] = item.raw;
    }
    return { snapshot, values, damaged };
  }
  function status() {
    const { snapshot, damaged } = inspect();
    return { message: snapshot.fatal || (damaged.length ? "Some saved settings are invalid. Safe defaults are in use; the original is preserved." : error),
      damaged, pending: pending.size, recoverable: !!snapshot.source && (!!snapshot.fatal || damaged.length > 0),
      recovery: !!snapshot.doc?.recovery, available: !!snapshot.doc };
  }
  function notify() { listener(status()); }
  function getItem(key) {
    if (!rules.has(key)) throw new Error("Unknown preference: " + key);
    if (pending.has(key)) return pending.get(key);
    return checked(key, read().doc?.values[key]).raw;
  }
  function write(doc) {
    const text = JSON.stringify(doc);
    if (text.length > LIMIT * 4) throw new Error("Settings and their recovery copy exceed the size limit.");
    localStorage.setItem(KEY, text);
  }
  function flush() {
    const { snapshot, damaged } = inspect();
    if (snapshot.fatal || damaged.length) {
      error = "Recover the saved settings before saving changes."; notify(); return false;
    }
    try {
      const values = { ...snapshot.doc.values, ...Object.fromEntries(pending) };
      write({ ...snapshot.doc, values }); pending.clear(); error = ""; notify(); return true;
    } catch (_) { error = "Settings could not be saved. Your changes work for this session; free some device storage and retry."; notify(); return false; }
  }
  function setItem(key, raw) {
    if (!rules.has(key)) throw new Error("Unknown preference: " + key);
    const value = checked(key, String(raw));
    if (value.invalid) { error = "An invalid setting was not saved."; notify(); return false; }
    pending.set(key, value.raw); return flush();
  }
  function exportData() {
    const { values } = inspect();
    const text = JSON.stringify({ format: "anima-preferences", version: 1,
      exportedAt: new Date().toISOString(), values: { ...values, ...Object.fromEntries(pending) } }, null, 2);
    if (new TextEncoder().encode(text).length > LIMIT * 4) throw new Error("The backup exceeds 4 MB. Reduce the stored data before exporting.");
    return text;
  }
  function parseBackup(text) {
    if (text.length > LIMIT * 4 || new TextEncoder().encode(text).length > LIMIT * 4) throw new Error("Choose a settings backup smaller than 4 MB.");
    let doc;
    try { doc = JSON.parse(text); } catch (_) { throw new Error("This file is not valid JSON."); }
    if (!object(doc) || doc.format !== "anima-preferences" || doc.version !== 1 || !object(doc.values)) {
      throw new Error("Choose an Anima settings backup (version 1).");
    }
    const values = {};
    for (const [key, raw] of Object.entries(doc.values)) {
      if (!rules.has(key) || typeof raw !== "string") throw new Error("The backup contains unsupported settings: " + key);
      const item = checked(key, raw);
      if (item.invalid || item.raw === null) throw new Error("The backup contains invalid values: " + key);
      values[key] = item.raw;
    }
    return values;
  }
  function replace(values, expectedSource) {
    values = parseBackup(JSON.stringify({ format: "anima-preferences", version: 1, values }));
    const snapshot = read();
    if (!snapshot.source) throw new Error(snapshot.fatal);
    if (!same(snapshot.source, expectedSource)) throw new Error("Saved settings changed in another window. Review the backup again.");
    // The original and the replacement are committed in one setItem. A quota
    // failure leaves everything unchanged, including the previous recovery copy.
    const original = snapshot.doc ? { legacy: snapshot.doc.values } : snapshot.source;
    write({ version: 1, values, recovery: { at: new Date().toISOString(), source: original } });
    pending.clear(); error = ""; notify();
  }
  function recover() {
    const { snapshot, values } = inspect();
    replace({ ...values, ...Object.fromEntries(pending) }, snapshot.source);
  }
  function recoveryData() {
    const recovery = read().doc?.recovery;
    if (!recovery?.source) throw new Error("No recovery copy is available.");
    return recovery.source.envelope ?? JSON.stringify(recovery.source.legacy, null, 2);
  }
  function previousData() {
    const source = read().doc?.recovery?.source;
    if (!source) throw new Error("No previous settings are available.");
    let doc;
    try { doc = source.envelope ? JSON.parse(source.envelope) : { version: 1, values: source.legacy }; }
    catch (_) { throw new Error("The recovery copy contains damaged data. Download it to inspect the original."); }
    if (doc.version !== 1 || !object(doc.values)) throw new Error("The recovery copy is from an unsupported version.");
    const values = Object.fromEntries(Object.entries(doc.values).filter(([key]) => rules.has(key)));
    const text = JSON.stringify({ format: "anima-preferences", version: 1, values });
    parseBackup(text); return text;
  }
  function source() { return read().source; }
  return { getItem, setItem, flush, status, register, json, object, exportData, parseBackup,
    replace, recover, recoveryData, previousData, source, relevant(key) { return key === null || key === KEY || rules.has(key); },
    onChange(fn) { listener = fn; notify(); }, limit: LIMIT * 4 };
})();

// Shape checks run before JSON reaches rendering or input code. Unknown object
// fields survive normal saves; invalid known fields/items use safe defaults.
const prefObject = preferenceStorage.object;
const prefNumber = (n, min, max) => Number.isFinite(n) && n >= min && n <= max;
const prefInteger = (n, min = 0, max = 65535) => Number.isInteger(n) && prefNumber(n, min, max);
const prefText = (s, max = 256) => typeof s === "string" && s.length <= max;
function prefArray(v, check, limit = 2000) {
  return Array.isArray(v) && v.length <= limit ? v.filter(check) : null;
}
for (const name of ["infoBarOn", "counterBarOn", "cbWarnOn", "netStatsOn", "inspectOn", "ignoreListOn", "combatBookOn", "racialBookOn", "hudHidden", "journalHidden", "partyLoot"]) {
  preferenceStorage.register("anima." + name, raw => ({raw: ["0", "1"].includes(raw) ? raw : null, invalid: !["0", "1"].includes(raw)}));
}
preferenceStorage.register("anima.cbWarnAt", raw => ({raw: prefInteger(+raw, 0, 1000000) ? raw : null, invalid: !prefInteger(+raw, 0, 1000000)}));
preferenceStorage.register("anima.journalTab", raw => ({raw: ["all", "speech", "guild", "system"].includes(raw) ? raw : null, invalid: !["all", "speech", "guild", "system"].includes(raw)}));
for (const name of ["poiCats", "infoBarFields", "ignoreList"]) preferenceStorage.json("anima." + name, v => prefArray(v, x => prefText(x), 512));
preferenceStorage.json("anima.markers", v => prefArray(v, x => prefObject(x) && prefInteger(x.x) && prefInteger(x.y) && prefText(x.name)));
preferenceStorage.json("anima.counterSlots", v => prefArray(v, x => prefObject(x) && prefInteger(x.g) && (x.hue === null || prefInteger(x.hue)) && prefInteger(x.cmp, 0, 1000000), 128));
for (const name of ["hudPos", "miniPos"]) preferenceStorage.json("anima." + name, v => prefObject(v) && prefNumber(v.x, -100000, 100000) && prefNumber(v.y, -100000, 100000) ? v : null);
preferenceStorage.json("anima.statusPos", v => prefObject(v) && /^-?\d+(\.\d+)?px$/.test(v.left) && /^-?\d+(\.\d+)?px$/.test(v.top) ? v : null);
for (const name of ["skillbtns", "spellbtns"]) preferenceStorage.json("anima." + name, v => prefArray(v, x => prefObject(x) && prefInteger(x.id) && prefNumber(x.x, -100000, 100000) && prefNumber(x.y, -100000, 100000) && (name !== "spellbtns" || (prefInteger(x.icon) && prefText(x.name))), 256));
