// A small DOM, enough to RUN web/js/*.js — not to be a browser.
//
// The renderer is only half PixiJS: the HUD, every gump window, the paperdoll,
// the housing panel and all of dialogs.js are plain DOM, built with innerHTML
// and read back with querySelector. A harness whose `getElementById` hands out a
// fresh do-nothing stub each call can load those files but cannot test them —
// nothing an element is told ever comes back out of it. So this keeps real
// parent/child links, real attributes, a real class list and real event
// dispatch with bubbling, and parses the innerHTML strings the client actually
// writes.
//
// What it deliberately is NOT: no layout (every box measures 0 unless a test
// sets `el.rect`), no CSS cascade (`style` is a plain property bag), no HTML
// error recovery (a malformed tag throws instead of guessing). Anything the
// client does not do, this does not do.

const VOID = new Set(["area", "base", "br", "col", "embed", "hr", "img", "input",
                      "link", "meta", "param", "source", "track", "wbr"]);

// Properties that live on the element rather than in its attribute map, because
// the client reads and writes them directly (`el.value`, `input.checked`, …).
// setAttribute mirrors into them so markup-built elements answer the same way.
const PROPS = { value: "", checked: false, disabled: false, src: "", href: "",
                title: "", type: "", placeholder: "", selected: false, min: "",
                max: "", step: "", name: "", alt: "" };

const ENT = { amp: "&", lt: "<", gt: ">", quot: '"', "#39": "'", apos: "'", nbsp: " " };
const decode = (s) => s.replace(/&(#?\w+);/g, (m, e) => (e in ENT ? ENT[e] : (e[0] === "#" ? String.fromCharCode(+e.slice(1)) : m)));
const encode = (s) => s.replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" }[c]));

class Text {
  constructor(data) { this.nodeType = 3; this.data = String(data); this.parentNode = null; }
  get textContent() { return this.data; }
  set textContent(v) { this.data = String(v); }
}

class ClassList {
  constructor(el) { this._el = el; }
  get _set() {
    const v = this._el.getAttribute("class");
    return new Set(v ? v.trim().split(/\s+/).filter(Boolean) : []);
  }
  _write(s) { this._el.setAttribute("class", [...s].join(" ")); }
  add(...n) { const s = this._set; n.forEach((x) => s.add(x)); this._write(s); }
  remove(...n) { const s = this._set; n.forEach((x) => s.delete(x)); this._write(s); }
  contains(n) { return this._set.has(n); }
  toggle(n, force) {
    const s = this._set, on = force === undefined ? !s.has(n) : !!force;
    if (on) s.add(n); else s.delete(n);
    this._write(s);
    return on;
  }
  get length() { return this._set.size; }
  toString() { return [...this._set].join(" "); }
  [Symbol.iterator]() { return this._set[Symbol.iterator](); }
}

class Element {
  constructor(tag, doc) {
    this.nodeType = 1;
    this.tagName = String(tag).toUpperCase();
    this.ownerDocument = doc;
    this.attributes = new Map();
    this.childNodes = [];
    this.parentNode = null;
    this.style = {};
    this.dataset = {};
    this.classList = new ClassList(this);
    this.listeners = new Map();          // type -> [fn]
    this.rect = null;                    // a test may set a layout box here
    this.scrollTop = 0;
    this.scrollHeight = 0;
    Object.assign(this, PROPS);
  }

  // ── attributes ──────────────────────────────────────────────────────────
  getAttribute(n) { const v = this.attributes.get(n); return v === undefined ? null : v; }
  hasAttribute(n) { return this.attributes.has(n); }
  removeAttribute(n) {
    this.attributes.delete(n);
    if (n.startsWith("data-")) delete this.dataset[dataKey(n)];
    else if (n in PROPS) this[n] = PROPS[n];
  }
  setAttribute(n, v) {
    v = String(v);
    this.attributes.set(n, v);
    if (n.startsWith("data-")) this.dataset[dataKey(n)] = v;
    else if (n in PROPS) this[n] = n === "checked" || n === "disabled" || n === "selected" ? true : v;
    else if (n === "style") parseStyle(v, this.style);
  }
  get id() { return this.getAttribute("id") || ""; }
  set id(v) { this.setAttribute("id", v); }
  get className() { return this.getAttribute("class") || ""; }
  set className(v) { this.setAttribute("class", v); }

  // ── tree ────────────────────────────────────────────────────────────────
  get children() { return this.childNodes.filter((n) => n.nodeType === 1); }
  get firstChild() { return this.childNodes[0] || null; }
  get lastChild() { return this.childNodes[this.childNodes.length - 1] || null; }
  get firstElementChild() { return this.children[0] || null; }
  get parentElement() { return this.parentNode && this.parentNode.nodeType === 1 ? this.parentNode : null; }
  get nextSibling() {
    const k = this.parentNode ? this.parentNode.childNodes : [];
    return k[k.indexOf(this) + 1] || null;
  }
  appendChild(c) {
    if (c.parentNode) c.parentNode.removeChild(c);
    c.parentNode = this;
    this.childNodes.push(c);
    return c;
  }
  append(...cs) { for (const c of cs) this.appendChild(typeof c === "string" ? new Text(c) : c); }
  insertBefore(c, ref) {
    if (!ref) return this.appendChild(c);
    if (c.parentNode) c.parentNode.removeChild(c);
    const i = this.childNodes.indexOf(ref);
    if (i < 0) throw new Error("insertBefore: reference node is not a child");
    c.parentNode = this;
    this.childNodes.splice(i, 0, c);
    return c;
  }
  removeChild(c) {
    const i = this.childNodes.indexOf(c);
    if (i < 0) throw new Error("removeChild: node is not a child");
    this.childNodes.splice(i, 1);
    c.parentNode = null;
    return c;
  }
  replaceChildren(...cs) {
    for (const c of this.childNodes.slice()) this.removeChild(c);
    this.append(...cs);
  }
  remove() { if (this.parentNode) this.parentNode.removeChild(this); }
  contains(n) { for (let p = n; p; p = p.parentNode) if (p === this) return true; return false; }
  cloneNode(deep) {
    const el = new Element(this.tagName, this.ownerDocument);
    for (const [k, v] of this.attributes) el.setAttribute(k, v);
    Object.assign(el.style, this.style);
    if (deep) for (const c of this.childNodes) el.appendChild(c.nodeType === 3 ? new Text(c.data) : c.cloneNode(true));
    return el;
  }

  // ── content ─────────────────────────────────────────────────────────────
  get textContent() { return this.childNodes.map((n) => n.textContent).join(""); }
  set textContent(v) {
    this.childNodes.forEach((n) => { n.parentNode = null; });
    this.childNodes = [];
    if (v !== "" && v != null) this.appendChild(new Text(v));
  }
  get innerHTML() { return this.childNodes.map(serialize).join(""); }
  set innerHTML(html) {
    this.childNodes.forEach((n) => { n.parentNode = null; });
    this.childNodes = [];
    parseInto(this, String(html), this.ownerDocument);
  }
  get outerHTML() { return serialize(this); }

  // ── queries ─────────────────────────────────────────────────────────────
  matches(sel) { return parseSelector(sel).some((seq) => matchSeq(this, seq)); }
  closest(sel) { for (let p = this; p; p = p.parentElement) if (p.matches(sel)) return p; return null; }
  querySelector(sel) { return this.querySelectorAll(sel)[0] || null; }
  querySelectorAll(sel) {
    const groups = parseSelector(sel), out = [];
    walk(this, (el) => { if (el !== this && groups.some((g) => matchSeq(el, g))) out.push(el); });
    return out;
  }
  getElementsByClassName(c) { return this.querySelectorAll("." + c); }

  // ── events ──────────────────────────────────────────────────────────────
  addEventListener(type, fn) {
    if (typeof fn !== "function") return;          // the client never passes handleEvent objects
    const l = this.listeners.get(type) || [];
    l.push(fn);
    this.listeners.set(type, l);
  }
  removeEventListener(type, fn) {
    const l = this.listeners.get(type);
    if (!l) return;
    const i = l.indexOf(fn);
    if (i >= 0) l.splice(i, 1);
  }
  dispatchEvent(ev) { return dispatch(this, ev); }

  // ── layout / focus ──────────────────────────────────────────────────────
  getBoundingClientRect() {
    const r = this.rect || { left: 0, top: 0, width: 0, height: 0 };
    const left = r.left || 0, top = r.top || 0, width = r.width || 0, height = r.height || 0;
    return { left, top, width, height, right: left + width, bottom: top + height, x: left, y: top };
  }
  get offsetWidth() { return this.getBoundingClientRect().width; }
  get offsetHeight() { return this.getBoundingClientRect().height; }
  get clientWidth() { return this.getBoundingClientRect().width; }
  get clientHeight() { return this.getBoundingClientRect().height; }
  focus() { this.ownerDocument.activeElement = this; }
  blur() { if (this.ownerDocument.activeElement === this) this.ownerDocument.activeElement = this.ownerDocument.body; }
  select() {}
  scrollIntoView() {}
  click() { this.dispatchEvent(new DomEvent("click", { bubbles: true })); }
  // <canvas>: the client only ever asks for a 2d context, and the harness owns
  // what that context records — see harness.js.
  getContext(kind) { return this.ownerDocument.__context2d(this, kind); }
  toDataURL(type) { return `data:${type || "image/png"};base64,`; }
  toBlob(cb) { cb(null); }
  play() { return Promise.resolve(); }
  pause() {}
}

const dataKey = (n) => n.slice(5).replace(/-([a-z])/g, (_, c) => c.toUpperCase());

function parseStyle(css, into) {
  for (const decl of css.split(";")) {
    const i = decl.indexOf(":");
    if (i < 0) continue;
    const k = decl.slice(0, i).trim().replace(/-([a-z])/g, (_, c) => c.toUpperCase());
    if (k) into[k] = decl.slice(i + 1).trim();
  }
}

function walk(root, fn) {
  for (const c of root.childNodes) {
    if (c.nodeType !== 1) continue;
    fn(c);
    walk(c, fn);
  }
}

function serialize(n) {
  if (n.nodeType === 3) return encode(n.data);
  const attrs = [...n.attributes].map(([k, v]) => ` ${k}="${v.replace(/"/g, "&quot;")}"`).join("");
  const tag = n.tagName.toLowerCase();
  if (VOID.has(tag)) return `<${tag}${attrs}>`;
  return `<${tag}${attrs}>${n.childNodes.map(serialize).join("")}</${tag}>`;
}

// ── the HTML the client writes ────────────────────────────────────────────
// Tag soup only: elements, attributes, text, comments. No <template>, no
// namespaces, no implied end tags. A stray `</div>` or an unclosed element is
// an ERROR, not something to paper over — in a test it means the string the
// client built is malformed, which is exactly the sort of thing worth failing.
function parseInto(root, html, doc) {
  const stack = [root];
  const re = /<!--[\s\S]*?-->|<\/([a-zA-Z][\w-]*)\s*>|<([a-zA-Z][\w-]*)((?:\s+[^\s=/>]+(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s"'=<>`]+))?)*)\s*(\/?)>/g;
  let at = 0, m;
  const text = (s) => { if (s) stack[stack.length - 1].appendChild(new Text(decode(s))); };
  while ((m = re.exec(html))) {
    text(html.slice(at, m.index));
    at = re.lastIndex;
    if (m[0].startsWith("<!--")) continue;
    if (m[1]) {                                        // closing tag
      const tag = m[1].toUpperCase();
      const top = stack[stack.length - 1];
      // Strict, and a browser is not: a browser would insert implied end tags
      // and carry on with a tree nobody meant. The client never relies on that
      // (no bare <li>/<td>/<option>), so mis-nesting here is a real bug in the
      // string the client just built, and a test should see it.
      if (stack.length < 2 || top.tagName !== tag) {
        throw new Error(`innerHTML: </${m[1].toLowerCase()}> closes ` +
          (stack.length < 2 ? "nothing" : `<${top.tagName.toLowerCase()}>`) +
          ` in ${JSON.stringify(clip(html))}`);
      }
      stack.pop();
      continue;
    }
    const el = doc.createElement(m[2]);
    for (const a of m[3].matchAll(/([^\s=/>]+)(?:\s*=\s*("([^"]*)"|'([^']*)'|([^\s"'=<>`]+)))?/g)) {
      el.setAttribute(a[1], decode(a[3] ?? a[4] ?? a[5] ?? ""));
    }
    stack[stack.length - 1].appendChild(el);
    if (!m[4] && !VOID.has(m[2].toLowerCase())) stack.push(el);
  }
  text(html.slice(at));
  if (stack.length > 1) {
    throw new Error(`innerHTML: unclosed <${stack[stack.length - 1].tagName.toLowerCase()}> in ${JSON.stringify(clip(html))}`);
  }
}
const clip = (s) => (s.length > 160 ? s.slice(0, 157) + "..." : s);

// ── selectors ─────────────────────────────────────────────────────────────
// "a, b" groups; " " descendant and ">" child combinators; compound simple
// selectors of tag / #id / .class / [attr] / [attr="v"] / [attr=v] / *.
const selCache = new Map();
function parseSelector(sel) {
  const hit = selCache.get(sel);
  if (hit) return hit;
  const groups = String(sel).split(",").map((part) => {
    const seq = [];
    // Split on combinators, keeping ">" as its own step.
    for (const tok of part.trim().split(/\s+/)) {
      if (!tok) continue;
      if (tok === ">") { seq.push({ child: true }); continue; }
      const gt = tok.split(">");                       // "a>b" with no spaces
      gt.forEach((t, i) => {
        if (i) seq.push({ child: true });
        if (t) seq.push(compound(t, sel));
      });
    }
    if (!seq.length) throw new Error(`empty selector in ${JSON.stringify(sel)}`);
    return seq;
  });
  selCache.set(sel, groups);
  return groups;
}
function compound(t, whole) {
  const c = { tag: null, id: null, classes: [], attrs: [] };
  const re = /([.#]?)([\w-]+)|\[([\w-]+)(?:([~^$*|]?=)"?'?([^\]"']*)"?'?)?\]|\*/g;
  let m, seen = 0;
  while ((m = re.exec(t))) {
    seen = re.lastIndex;
    if (m[3]) { c.attrs.push([m[3], m[4] ? m[5] : null]); continue; }
    if (m[0] === "*") continue;
    if (m[1] === ".") c.classes.push(m[2]);
    else if (m[1] === "#") c.id = m[2];
    else c.tag = m[2].toUpperCase();
  }
  if (seen !== t.length) throw new Error(`unsupported selector ${JSON.stringify(whole)} (at ${JSON.stringify(t)})`);
  return c;
}
function matchOne(el, c) {
  if (c.child) return false;
  if (c.tag && el.tagName !== c.tag) return false;
  if (c.id && el.id !== c.id) return false;
  for (const k of c.classes) if (!el.classList.contains(k)) return false;
  for (const [a, v] of c.attrs) {
    if (!el.hasAttribute(a)) return false;
    if (v !== null && el.getAttribute(a) !== v) return false;
  }
  return true;
}
// Right-to-left, the way a browser does it: cheap rejection on the subject.
function matchSeq(el, seq) {
  let i = seq.length - 1;
  if (!matchOne(el, seq[i])) return false;
  i--;
  let node = el.parentElement;
  while (i >= 0) {
    const strict = seq[i].child;
    if (strict) i--;
    if (i < 0) return true;
    if (strict) {
      if (!node || !matchOne(node, seq[i])) return false;
      node = node.parentElement;
      i--;
    } else {
      let p = node, hit = null;
      while (p && !hit) { if (matchOne(p, seq[i])) hit = p; p = p.parentElement; }
      if (!hit) return false;
      node = hit.parentElement;
      i--;
    }
  }
  return true;
}

// ── events ────────────────────────────────────────────────────────────────
class DomEvent {
  constructor(type, init = {}) {
    this.type = type;
    this.bubbles = init.bubbles !== false;
    this.cancelable = init.cancelable !== false;
    this.defaultPrevented = false;
    this.propagationStopped = false;
    this.target = null;
    this.currentTarget = null;
    // Whatever else the test put in `init` (key, button, clientX, …) rides along
    // verbatim: the client reads those straight off the event object.
    Object.assign(this, init);
    this.bubbles = init.bubbles !== false;
  }
  preventDefault() { this.defaultPrevented = true; }
  stopPropagation() { this.propagationStopped = true; }
  stopImmediatePropagation() { this.propagationStopped = true; this.immediateStopped = true; }
}

function dispatch(target, ev) {
  ev.target = ev.target || target;
  const path = [];
  for (let n = target; n; n = n.parentNode || n.__eventParent) {
    path.push(n);
    if (!ev.bubbles && n === target) break;
  }
  for (const n of path) {
    const l = n.listeners && n.listeners.get(ev.type);
    if (l) {
      ev.currentTarget = n;
      for (const fn of l.slice()) {
        fn.call(n, ev);
        if (ev.immediateStopped) break;
      }
    }
    if (ev.propagationStopped) break;
  }
  return !ev.defaultPrevented;
}

class Document {
  constructor() {
    this.nodeType = 9;
    this.listeners = new Map();
    this.documentElement = new Element("html", this);
    this.head = new Element("head", this);
    this.body = new Element("body", this);
    this.documentElement.appendChild(this.head);
    this.documentElement.appendChild(this.body);
    this.activeElement = this.body;
    this.__context2d = () => null;              // harness.js installs the real one
    this.parentNode = null;
  }
  createElement(tag) { return new Element(tag, this); }
  createTextNode(t) { return new Text(t); }
  createDocumentFragment() { return new Element("#fragment", this); }
  getElementById(id) {
    let hit = null;
    walk(this.documentElement, (el) => { if (!hit && el.id === id) hit = el; });
    return hit;
  }
  querySelector(sel) { return this.documentElement.querySelector(sel); }
  querySelectorAll(sel) { return this.documentElement.querySelectorAll(sel); }
  getElementsByClassName(c) { return this.querySelectorAll("." + c); }
  addEventListener(t, f) { Element.prototype.addEventListener.call(this, t, f); }
  removeEventListener(t, f) { Element.prototype.removeEventListener.call(this, t, f); }
  dispatchEvent(ev) { return dispatch(this, ev); }
  get hidden() { return false; }
  get visibilityState() { return "visible"; }
  get readyState() { return "complete"; }
}

module.exports = { Document, Element, Text, DomEvent };
