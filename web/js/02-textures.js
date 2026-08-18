// ---- texture + frame-count caches ----
// Hue is baked into the cache key/URL (the server pre-hues each PNG), so every
// distinct dye of every item/body multiplies GPU-resident textures — an
// unbounded cache pins hundreds of MB after a long multi-town tour. Bound it
// with an LRU: texLastUsed tracks when each url was last actually drawn
// (touchTex, called on every texFor hit, PLUS a blanket per-poll sweep over every
// url a live sprite/anim-part could be showing — see forEachLiveTexUrl below,
// called from syncWorld()). Eviction only ever considers entries idle past
// TEX_IDLE_MS, so a texture a live sprite is still using — touched far more
// often than that — is never pulled out from under it; the budget (TEX_BUDGET)
// is picked high enough that ordinary town play never crosses it, so this only
// changes marathon sessions.
const texCache = new Map(), texLastUsed = new Map(), loading = new Set();
const TEX_BUDGET = 1500;          // ~200MB at UO's typical small-sprite sizes
const TEX_IDLE_MS = 5 * 60_000;   // don't evict anything touched more recently than this
const TEX_SWEEP_MS = 30_000;      // don't re-scan for eviction more than 1x/30s
let lastTexSweep = 0;
function touchTex(url) { if (url) texLastUsed.set(url, performance.now()); }
function texFor(url) {
  if (texCache.has(url)) { touchTex(url); return texCache.get(url); }
  if (!loading.has(url)) {
    loading.add(url);
    // markDirty() in the .then so a body/clothing frame that streams in gets
    // painted even while the character stands still (render-on-demand otherwise
    // wouldn't repaint an idle scene when a late texture arrives).
    PIXI.Assets.load(url).then((t) => {
      texCache.set(url, t); touchTex(url); loading.delete(url); markDirty(); sweepTexCache();
    }).catch(() => { texCache.set(url, null); touchTex(url); loading.delete(url); });
  }
  return null;
}
// Evict LRU entries once over TEX_BUDGET, throttled to at most 1 scan/TEX_SWEEP_MS
// (this can run on every texture load once near budget, so keep it cheap). Only
// evicts entries idle past TEX_IDLE_MS — see the cache's own comment above for why
// that's safe. Routes eviction through PIXI.Assets.unload(url), NOT a bare
// texture.destroy(true): Assets keeps its own url→texture cache (Loader.promiseCache
// + the top-level Cache), and unload() is what forgets the url there too — destroying
// the texture directly would leave a later PIXI.Assets.load(url) handing back the
// same (now-destroyed) Texture instead of actually reloading it.
//
// Belt-and-braces: touchTex alone isn't trusted to have caught everything (two
// real escapes found live: pruneFar's hysteresis-ring tiles, which sit on stage
// outside the "seen this poll" window loop that does the touching, and a
// mobile's st.partTex last-good fallback texture, whose OWN url only gets
// touched incidentally, not every frame it's actually the one drawn). If either
// escape (or a future one) evades touchTex bookkeeping, evicting a texture a
// live sprite still points at throws inside app.render() ("Cannot read
// properties of null (reading alphaMode)") and freezes the whole rAF loop (see
// frame()'s own resilience fix). So: build the live set fresh at sweep time and
// simply never evict anything in it, full stop, regardless of texLastUsed.
function sweepTexCache() {
  if (texCache.size <= TEX_BUDGET) return;
  const now = performance.now();
  if (now - lastTexSweep < TEX_SWEEP_MS) return;
  lastTexSweep = now;
  const live = new Set();
  forEachLiveTexUrl((u) => { if (u) live.add(u); });
  const stale = [];
  for (const [url, last] of texLastUsed) if (!live.has(url) && now - last >= TEX_IDLE_MS) stale.push([url, last]);
  if (!stale.length) return; // over budget but everything's still "recently" touched (or live) — wait
  stale.sort((a, b) => a[1] - b[1]); // oldest-touched first
  const over = texCache.size - TEX_BUDGET;
  for (let i = 0; i < Math.min(over, stale.length); i++) {
    const url = stale[i][0];
    texCache.delete(url); texLastUsed.delete(url);
    PIXI.Assets.unload(url).catch(() => {});
  }
}

// ---- per-pixel hit-testing for interactive world sprites ----
// PIXI hit-tests a sprite by its rectangular bounds by default. UO art is mostly
// transparent (isometric tiles, thin signposts/hangers, foreshortened mobile
// frames), so a fully-transparent part of one sprite can steal a click from
// whatever is actually visible underneath it — measured live, a house sign
// (graphic 0x0BD2) sits UNDER its own hanger (graphic 0x0B98) at the identical
// zIndex, and the hanger's rectangular bounds fully CONTAIN the sign's;
// double-clicking the visible sign body hit the hanger instead (which has no
// double-click behaviour, so nothing happened). ClassicUO hit-tests per PIXEL
// (the art's actual opaque pixels), which is why this works there — so we do
// the same, via a custom `hitArea` per sprite.
//
// The mask is built once per texture URL — not per sprite, not per frame — and
// shared by every sprite currently showing that art. `null` marks a URL that
// failed to rasterize (CORS/404/tainted canvas) so it's never retried; those
// sprites simply fall back to plain bounds-based hit-testing, exactly like
// today, so this can never make clicking WORSE than before.
const alphaMaskCache = new Map();   // url -> {w, h, bits:Uint8Array} | null
const alphaMaskPending = new Set(); // urls currently being rasterized (dedupe)
function requestAlphaMask(url) {
  if (alphaMaskCache.has(url) || alphaMaskPending.has(url)) return;
  alphaMaskPending.add(url);
  // Prefer the image PIXI's own loader already decoded for this exact texture
  // (texture.source.resource — an HTMLImageElement or ImageBitmap depending on
  // which loader parser handled it) over fetching it again: this url is almost
  // always already in texCache by the time a sprite using it is clickable, so
  // this skips a redundant network round trip entirely. Only falls back to a
  // fresh Image() when that resource isn't reachable (texture not cached yet,
  // or a resource type drawImage() can't use) — the art is served same-origin
  // by our own play server, and PIXI.Assets.load(url) likely already fetched
  // this exact URL, so the browser's HTTP cache makes even that nearly free.
  const cachedTex = texCache.get(url);
  const resource = cachedTex && cachedTex.source && cachedTex.source.resource;
  const reusable = resource && (
    (typeof HTMLImageElement !== "undefined" && resource instanceof HTMLImageElement) ||
    (typeof ImageBitmap !== "undefined" && resource instanceof ImageBitmap) ||
    (typeof HTMLCanvasElement !== "undefined" && resource instanceof HTMLCanvasElement)
  );
  if (reusable) { rasterizeAlphaMask(url, resource); return; }
  const img = new Image();
  img.onload = () => rasterizeAlphaMask(url, img);
  img.onerror = () => { alphaMaskCache.set(url, null); alphaMaskPending.delete(url); };
  img.src = url;
}
// Shared by both sources above (a reused texture resource, or a freshly loaded
// Image): draw into an offscreen canvas, read back alpha, store one byte per
// pixel (>8 alpha ~= opaque enough to count as "hit"). Cache null on failure
// (CORS/tainted canvas/zero-size) so a bad url is never retried in a loop.
function rasterizeAlphaMask(url, img) {
  try {
    const w = img.naturalWidth || img.width, h = img.naturalHeight || img.height;
    if (!w || !h) throw new Error("empty image");
    const cv = document.createElement("canvas");
    cv.width = w; cv.height = h;
    const ctx = cv.getContext("2d");
    ctx.drawImage(img, 0, 0);
    const data = ctx.getImageData(0, 0, w, h).data; // same-origin → never taints the canvas
    const bits = new Uint8Array(w * h);
    for (let p = 0, i = 3; p < bits.length; p++, i += 4) bits[p] = data[i] > 8 ? 1 : 0;
    alphaMaskCache.set(url, { w, h, bits });
  } catch { alphaMaskCache.set(url, null); } // degrade to bounds-based, never retry
  alphaMaskPending.delete(url);
}
// Plain-rectangle test — PIXI's own default Sprite.containsPoint formula —
// used as the fallback whenever a mask isn't ready (still loading) or isn't
// available (failed to load): identical to today's bounds-based hit-testing.
function boundsContains(sp, x, y) {
  const w = sp.width, h = sp.height;
  const x0 = -w * sp.anchor.x, y0 = -h * sp.anchor.y;
  return x >= x0 && x <= x0 + w && y >= y0 && y <= y0 + h;
}
// A custom hitArea object — PIXI calls `.contains(x, y)` with LOCAL coordinates
// (already anchor-relative, the same space Sprite.containsPoint uses), so a
// point is converted to texture-pixel space via the sprite's own anchor before
// consulting the mask. `getUrl()` is read at CLICK time, not baked in at
// attach time: mobile part sprites are persistent and swap textures frame to
// frame (see drawMobs' `part()`/st.partTex), so the mask must track whatever
// art is currently shown, not whatever was showing when the hitArea was set.
function pixelHitArea(sp, getUrl) {
  // Warm the mask NOW, at attach time, instead of waiting for the first click to
  // discover it's missing: a mask that isn't ready falls back to bounds, so
  // without this the FIRST click on any sprite is still the old rectangle
  // behaviour (measured: the first click on the house sign went to a different
  // overlapping item; the second, once the mask had loaded, hit the sign).
  const first = getUrl();
  if (first) requestAlphaMask(first);
  return {
    contains(x, y) {
      const url = getUrl();
      if (!url) return boundsContains(sp, x, y);
      const mask = alphaMaskCache.get(url);
      if (mask === undefined) { requestAlphaMask(url); return boundsContains(sp, x, y); }
      if (mask === null) return boundsContains(sp, x, y);
      const w = sp.width, h = sp.height;
      if (!w || !h) return false;
      // A depth-sliced mobile part is only a horizontal strip of the frame
      // (`_sliceY`/`_sliceH`); map local y into that strip of the full mask.
      const srcH = sp._sliceH || mask.h;
      const srcY = sp._sliceY || 0;
      const px = Math.floor((x + sp.anchor.x * w) / w * mask.w);
      const py = Math.floor((y + sp.anchor.y * h) / h * srcH) + srcY;
      if (px < 0 || px >= mask.w || py < 0 || py >= mask.h) return false; // outside the texture rect
      return mask.bits[py * mask.w + px] !== 0;
    },
  };
}

const frameCount = new Map();
function framesFor(body, group, dir) {
  const k = `${body}/${group}/${dir}`;
  if (frameCount.has(k)) return Math.max(1, frameCount.get(k));
  if (!loading.has("i" + k)) {
    loading.add("i" + k);
    fetch(`animinfo/${body}/${group}/${dir}`).then((r) => r.json())
      .then((j) => { frameCount.set(k, j.frames | 0); frameCtr.set(k, j.c || []); })
      .catch(() => frameCount.set(k, 0));
  }
  return 5;
}
// Per-frame draw-center [cx, cy] (from animinfo). The renderer positions a part's
// sprite at (screenX - cx, screenY - height - cy) — ClassicUO's draw math — so the
// body, worn equipment, held items and a rider on a mount all align instead of
// being foot-anchored at the same point. null until the animinfo load lands.
const frameCtr = new Map(); // "body/group/dir" -> [[cx,cy],...]
function centerFor(body, group, dir, frame) {
  const c = frameCtr.get(`${body}/${group}/${dir}`);
  return c && c[frame] ? c[frame] : null;
}


// ---- light.mul shapes (ClassicUO LightsLoader) ------------------------------
// Each light-emitting graphic names one of ~100 hand-drawn masks through its
// tiledata Quality byte; the server decodes light.mul and serves them as white
// PNGs whose alpha is the intensity (see anima-assets `lights.rs`). Fetched as
// plain <img> rather than through the PIXI texture cache: the night overlay is a
// 2D canvas, not a PIXI layer, and there are at most a hundred of them.
// Keyed by "<id>/<colour>": a coloured light is the same mask with ClassicUO's
// intensity ramp for that colour baked into the RGB, which the server does
// (anima-assets `light_colored`) because it owns the curve tables.
const lightShapes = new Map();   // key -> HTMLImageElement | null (null = 404, don't retry)
function lightShape(id, color) {
  if (id == null) return null;
  id = id | 0; color = color | 0;
  const key = `${id}/${color}`;
  if (lightShapes.has(key)) {
    const img = lightShapes.get(key);
    return img && img.complete && img.naturalWidth ? img : null;
  }
  const img = new Image();
  img.onerror = () => lightShapes.set(key, null);
  img.src = `light/${id}.png` + (color ? `?c=${color}` : "");
  lightShapes.set(key, img);
  return null;   // next frame, once it has decoded
}
