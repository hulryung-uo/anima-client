# Graphics cache lifetime and recovery

The renderer retains terrain, art and animation PNGs by URL, including hue.
`web/js/02-textures.js` now manages the following limits:

| Resource | Retention / admission policy |
| --- | --- |
| Pixi textures | 1,500 entries or 256 MiB of estimated RGBA pixels; evict cold, unused entries until both fit |
| Texture work | At most 16 actual Pixi loads; 512 queued URLs; drop queued prefetches not requested for 2 seconds |
| Texture failures | Retry after 2 seconds, doubling up to 60 seconds; retain at most 512 failure records |
| Alpha hit masks | One byte per pixel; owned by the matching retained texture and discarded with it; fallback image requests capped at 16 with a 5-second deadline |
| Animation metadata | 4,096 records / 65,536 total draw-center pairs; counts and centers evict together in LRU order |
| Animation requests | At most 8 physical requests; a 5-second deadline includes the body; failed requests retry after 2 seconds |
| Animation responses | At most 1 MiB, including streaming bodies without Content-Length; validate frame count and center pairs before caching |

Texture budgets are **soft for live references**. Every sweep protects the URL
pools, animated statics' retained frames and mobile fallback parts. It also walks
the actual stage and protects texture **source identity**, covering corpse child
layers, stationary house previews, effect fallback frames and derived meshes or
slices. A large visible working set may exceed the budget. New textures get a
one-second grace period to reach the scene poll. Polling retries eviction even
when there are no more load completions.

Eviction uses `PIXI.Assets.unload`, which clears Pixi's loader and top-level
caches. A URL cannot reload until its asynchronous unload finishes. The bundled
Pixi 8.19.0 loader was directly verified to return the old, subsequently destroyed
texture when load and unload overlap without this guard. Directly destroying
textures is insufficient because Pixi also retains resolved URL promises.

A failed PNG no longer becomes a permanently cached null. Failed animation
metadata stays unknown and retryable; only a valid `frames: 0, c: []` response
means a confirmed missing animation. Timed-out requests cannot overwrite a
newer count or its centers. Physical work slots remain occupied until the actual
operation settles, even when a test transport ignores abort. Pixi image loads
are not cancellable through this wrapper; it does not free a slot on a pretend
timeout. Requests for art omitted from a full queue are retried by the normal
world/frame passes.

## Verification — 2026-09-13

- Nineteen headless regressions exercise real renderer functions, live URL and
  stage-source protection, byte/count pressure, polling after the final load,
  queue admission, retries, mask lifetime, response limits and stale metadata.
  One test runs the **actual vendored Pixi Assets/Loader/Cache and Texture
  destruction**, with a generated parser and no browser or GPU.
- A controlled before/after probe compared source at `ab38450` with the changed
  source. Five 64 MiB texture fixtures retained 320 MiB before and 256 MiB after
  an idle sweep. A rejected texture request remained absent before and retried
  successfully after; rejected metadata remained zero before and recovered to
  three frames after. These are estimated surface sizes, not allocated heap or
  measured GPU/RSS reductions.
- An isolated loopback fixture ran in Chrome through CUA using the actual
  bundled Pixi, WebGL and **1,608 generated 44×44 PNGs**. It finished in 4,293 ms,
  observed at most 16 active loads / 512 queued URLs, and retained 1,500 textures
  after eviction. Eight on-stage images survived; transparent-pixel misses and
  opaque-pixel hits remained correct. A cold texture reloaded successfully.
  A PNG and animation-info endpoint each returned 503 once and then succeeded;
  both recovered, with two HTTP requests each. The DOM result and screenshot
  showed PASS; captured console warnings/errors were empty.
- The complete `scripts/check.sh` run returned actual exit 0: 333 web tests /
  1,664 assertions, 23 Python tooling tests, Rust/WASM checks and desktop tests.
  Fixture/report records: `/tmp/anima-graphics-qa.json`,
  `/tmp/anima-graphics-cache-browser-result.json`,
  `/tmp/anima-graphics-cache-before-after.json`, and
  `/tmp/anima-graphics-cache-gate.log`.

CI run 34727768842 also passed all Linux, macOS and Windows jobs for `e6bb50c`.

Verified texture source SHA-256:
`b764014e0db733ee0f95413e3ad92fb8e301ddaecfffacafcb4a8ce83b9e6bdf`.

These checks do not establish whole-client memory limits, stock-art visual
correctness, Windows WebView behavior or long-session live-shard readiness.
Decoded images, GPU allocations, in-flight/unloading resources and the visible
working set add to retained-cache estimates. The separate light-shape cache is covered below; native graphics caches and
whole-process allocations still need their own audit. The source is newer than the
v0.7.0 draft installers and needs a subsequent installer build and runtime checks.

## World-map disk cache isolation — 2026-09-13

The old world-map path was one global `anima-worldmap0-s1.png` under the system
temporary directory. It accepted any readable bytes and did not identify the
selected game-data folder. Switching resource packages could therefore reuse
another package's map; a damaged cache could persist indefinitely.

The native cache now uses one versioned slot per canonical resource directory
and renderer step. Its source fingerprint includes file sizes and modification
times for the map UOP, statics/index, tiledata, radar colours and optional map-diff
files. A missing required file disables reuse. Rendering that overlaps a source
change does not publish an outdated cache entry. Payload length and checksum
validation reject truncated or changed records; reads are capped at 128 MiB.
Unique staging names and atomic replacement support concurrent writers in the
same process or different processes. Cache failures still allow the fresh image
to be displayed. The old shared cache is ignored and left untouched.

Six tests cover directory/step isolation, all source-file changes, same-length
modification-time changes, missing files and changes during rendering, corrupt/
truncated/oversized records, and eight concurrent writers. `scripts/check.sh`
returned actual exit 0; log: `/tmp/anima-worldmap-cache-gate.log`. The renderer's
map pixels and map-data parsing were not changed. Fingerprinting uses metadata;
changes that deliberately preserve both file size and modification time are not
detected. This is not a full content-hash or live shard-map patch cache. These
changes postdate v0.8.1 and require a later installer. CI run 34729855987 passed all Linux, macOS and Windows jobs for the
world-map cache repair (`661d4b5`), including the new native cache tests.


## Retryable, bounded light masks — 2026-09-13

Previously an image error installed a permanent null record, so a temporary
failure kept that light on the generic radial fallback until app reload. Colour
variants also accumulated without a retention or load-admission limit.

The canvas mask cache now tracks at most 256 variants and eight active image
loads. Failed requests retry when the shape is needed, with 2–60 second backoff.
A five-second deadline removes the image source, releases its slot and ignores
late callbacks. Invalid IDs/colours are refused before requesting an image.
Successful images have a 16 MiB estimated RGBA retention budget, with a one-second
grace for freshly loaded or currently drawn masks. Cold entries release their
image sources; the normal graphics sweep also drains excess retention when the
scene stops drawing lights. An individual decoded image above 16 MiB is rejected.

The byte budget is soft while the visible/fresh working set exceeds it; this is
not a browser-native decoder, network or whole-process memory ceiling. The
existing radial fallback remains available during loading, admission pressure
or failures. Native light-file allocation bounds and actual stock-mask visuals
remain separate audit items.

Six deterministic tests cover error/recovery, admission/deadlines/late callbacks,
cold versus recently used variants, byte pressure, invalid/oversized images and
idle-scene cleanup. The ordinary lighting composition tests also pass. No UI
automation or live-shard connection was used for this repair.
