// ---- seq-ring priming (skip a stale backlog replay on page reload) ----
// Every event "ring" above (character anims 0x6E/0xE2, damage 0x0B, effects
// 0x70/0xC0/0xC7, lift-rejects 0x27, container-opens 0x24, swings 0x2F,
// paperdoll 0x88, external URLs 0xA5, tips/notices 0xA6, sounds 0x54,
// server-chat lines 0xB2) is keyed
// by a monotonic `seq` that lives in the
// anima-net play server's `World`, NOT on this page: reloading the browser
// resets every `lastXSeq` variable above to 0, but the live ServUO session
// (and its already-fired backlog under those seqs) keeps running underneath —
// it's tied to the server connection, not the tab. Left unprimed, the very
// first poll after a reload would treat that whole backlog as "new": stale
// animations/damage numbers/sounds replay once, and — worse — *sticky*
// signals like the paperdoll and the last container-open pop their windows
// back open even though nothing just happened.
//
// Fix: on the FIRST scene ingest after page load, bump every ring's last-seen
// seq up to its current max WITHOUT running the per-event handler, then flip
// `seqPrimed`. Every later poll runs ingestX()/playSounds() as normal, so a
// genuinely new event (seq beyond this baseline) still fires immediately.
let seqPrimed = false;
let wasInWorld = false;
function maxSeq(arr) {
  let m = 0;
  if (arr) for (const ev of arr) { const sq = ev.seq | 0; if (sq > m) m = sq; }
  return m;
}
function primeSeqRings(s) {
  lastAnimSeq = Math.max(lastAnimSeq, maxSeq(s.anims));
  lastTypedAnimSeq = Math.max(lastTypedAnimSeq, maxSeq(s.tanims));
  lastDamageSeq = Math.max(lastDamageSeq, maxSeq(s.damage));
  lastEffectSeq = Math.max(lastEffectSeq, maxSeq(s.effects));
  lastLiftRejectSeq = Math.max(lastLiftRejectSeq, maxSeq(s.liftRejects));
  lastDragCompletionSeq = Math.max(lastDragCompletionSeq, maxSeq(s.dragCompletions));
  if (s.deathScreen) lastDeathScreenSeq = Math.max(lastDeathScreenSeq, s.deathScreen.seq | 0);
  lastContainerOpenSeq = Math.max(lastContainerOpenSeq, maxSeq(s.containerOpens));
  lastSwingSeq = Math.max(lastSwingSeq, maxSeq(s.swings));
  lastSoundSeq = Math.max(lastSoundSeq, maxSeq(s.sounds));
  if (s.paperdoll) lastPaperdollSeq = Math.max(lastPaperdollSeq, s.paperdoll.seq | 0);
  lastOpenUrlSeq = Math.max(lastOpenUrlSeq, maxSeq(s.openUrls));
  lastTipNoticeSeq = Math.max(lastTipNoticeSeq, maxSeq(s.tips));
  if (s.logoutAck) lastLogoutAckSeq = Math.max(lastLogoutAckSeq, s.logoutAck.seq | 0);
  lastBoatMoveSeq = Math.max(lastBoatMoveSeq, maxSeq(s.boatMoves));
  lastChatSeq = Math.max(lastChatSeq, maxSeq(s.chat && s.chat.lines));
  // Per-key open-counters live in the dialog families, not in a ring here.
  primeDialogSeqs(s);
}

async function poll() {
  const t0 = performance.now();
  try {
    const r = await fetch("scene.json?" + Date.now());
    if (!r.ok) throw new Error(r.status);
    scene = await r.json();
    // Not in world yet (login-page mode): show the login form instead of rendering.
    if (scene && scene.auth) {
      // A completed/lost game session owns a large amount of DOM and seq-gated
      // renderer state. Reload once on the world→login transition so none of it
      // leaks into the next character; the new page sees auth immediately and
      // therefore does not loop.
      if (wasInWorld) { window.location.reload(); return; }
      showLogin(scene.auth, scene.msg, scene.slots, scene.capacity, scene.cities, scene.error);
      return;
    }
    wasInWorld = true;
    hideLogin();
    if (!seqPrimed) { primeSeqRings(scene); seqPrimed = true; }
    ingestBoatMoves(scene);
    updateAnimStates(scene);
    const ts = performance.now();
    syncWorld(scene); // diffs only — no full rebuild
    diag.sync = performance.now() - ts;
    markDirty(); // a fresh poll may change tiles/entities → redraw once
    ingestSpeech(scene); // float new speech above its speaker
    ingestChat(scene); // print new server-chat lines (0xB2) into the journal
    ingestAnims(scene); // play new character animations (0x6E: combat swings, bows…)
    ingestTypedAnims(scene); // play new typed animations (0xE2: emotes, gestures, alerts…)
    ingestDamage(scene); // float new combat damage numbers (0x0B)
    ingestEffects(scene); // spawn new graphical effects (0x70/0xC0/0xC7)
    ingestLiftRejects(scene); // clear the held item + show a message (0x27 LiftRej)
    ingestDragCompletions(scene); // reconcile held-item cursor acknowledgements (0x28/0x29)
    ingestDeathScreen(scene); // start ClassicUO's short death banner timer (0x2C)
    ingestContainerOpens(scene); // open a window for each server-initiated container open (0x24)
    ingestOpenUrls(scene); // ask before opening each validated external URL (0xA5)
    ingestSwings(scene); // briefly face the attacker toward the defender (0x2F Swing)
    ingestPaperdoll(scene); // open/refresh a paperdoll the server told us to show (0x88)
    // Every server-driven dialog window in one pass — generic gumps, legacy
    // menus, hue pickers, trades, treasure maps, containers, the book reader,
    // the vendor window, the context menu, text-entry, profiles, the text
    // prompt and the house-design panel. See web/dialogs.js; each family is
    // declared with registerDialog() next to the code that builds it.
    syncDialogs(scene);
    refreshTip(); // update the hover tooltip if its OPL just arrived/changed
    drawMinimap(scene);
    updateGuardZones(scene); // guard-zone overlay: refetch on facet change, redraw clipped to view
    refreshBuffs(scene); // reconcile the buff/debuff bar with scene.buffs
    syncArmedAbility(); // adopt the server's arm (0xBF/0x21 clears it after every use)
    refreshAbilities(); // keep the weapon special-ability bar in sync with the equipped weapon
    if (wmOn) drawWorldmap();  // keep the open world map tracking the player
    if (scene.player) hud(scene);
    updateMoveDebug(scene); // movement/Z debug HUD (Options → "Movement debug")
    refreshPaperdoll();   // keep the paperdoll live (equip/stats change)
    if (spellbookOn) { refreshSpellMana(); refreshSpellbookContent(); refreshActiveSpells(); } // keep the spellbook live (mana + book content + active stances)
    if (skillsOn) refreshSkills();  // keep the skills window live (values/locks change)
    checkSkillGains(scene);  // announce skill base changes as journal system messages
    if (scene.bboard) requestMissingBBSummaries(scene, scene.bboard); // 0x71 headers
    refreshParty();   // keep the party panel live + surface incoming invites (0xBF/0x06)
    refreshTipNotices(scene); // pageable tips / close-only notices (0xA6)
    refreshLogoutAck(scene); // restore UI if the server denied a 0xD1 logout
    updateTargetUI(); // reflect the server's target-cursor state (crosshair + banner)
    updatePlacementPreview(); // rebuild/clear the house-footprint preview if scene.placement changed
    updateHouseDesignGhost(); // rebuild/clear the design-piece ghost if the session/selection changed
    updateDeathUI(scene); // grayscale + "You are dead" banner while the player is a ghost
    playSounds(scene);   // play new sound effects (0x54)
    updateMusic(scene);  // sync background music (0x6D)
    setStatus("live · " + new Date().toLocaleTimeString());
  } catch (e) {
    setStatus("waiting for scene… (" + e + ")");
  }
  diag.poll = performance.now() - t0;
  if (diag.poll > 150) console.warn(`[diag] slow poll ${diag.poll.toFixed(0)}ms`);
}

// 0x2C is a one-shot screen effect, while `player.dead` is authoritative state
// derived from ServUO's complete ghost-body set. ClassicUO keeps the grayscale
// effect for the whole ghost lifetime but shows “You are dead” for only 1.5s.
function ingestDeathScreen(s) {
  const ev = s && s.deathScreen;
  if (!ev) return;
  const seq = ev.seq | 0;
  if (seq <= lastDeathScreenSeq) return;
  lastDeathScreenSeq = seq;
  deathBannerUntil = performance.now() + 1500;
  if (deathBannerTimer != null) clearTimeout(deathBannerTimer);
  deathBannerTimer = setTimeout(() => {
    deathBannerTimer = null;
    deathBannerUntil = 0;
    updateDeathUI(scene);
  }, 1500);
}

// The body fallback keeps this renderer compatible with an older scene producer.
// Both clear on resurrection when the body reverts to a living id.
function updateDeathUI(s) {
  const p = s && s.player;
  const dead = !!(p && (typeof p.dead === "boolean" ? p.dead : isGhostBody(p.body)));
  const map = document.getElementById("map");
  if (map) map.classList.toggle("dead", dead);
  const banner = document.getElementById("deadbanner");
  if (banner) banner.style.display = dead && performance.now() < deathBannerUntil ? "block" : "none";
}

// ClassicUO BoatMovingManager velocity table. A segment starts when its 0xF6
// reaches this renderer; bursts are queued instead of collapsing intermediate
// tiles, keeping the hull and every passenger on one rigid timeline.
function boatMoveDuration(speed) {
  speed |= 0;
  if (speed === 2) return 1000;
  if (speed === 3) return 500;
  if (speed === 4) return 250;
  if (speed > 4) return speed * 10;
  return 500;
}

function queueBoatGlide(serial, from, to, duration, now) {
  serial >>>= 0;
  let queue = boatGlides.get(serial);
  if (!queue) { queue = []; boatGlides.set(serial, queue); }
  while (queue.length && now >= queue[0].end) queue.shift();
  const previous = queue.length ? queue[queue.length - 1] : null;
  const start = previous ? previous.end : now;
  const source = previous ? previous.to : from;
  queue.push({
    from: { x: Number(source.x), y: Number(source.y), z: Number(source.z || 0) },
    to: { x: Number(to.x), y: Number(to.y), z: Number(to.z || 0) },
    start,
    end: start + duration,
  });
  // A background tab can receive a short burst when it wakes. Keep enough
  // segments to preserve those intermediate tiles instead of jumping ahead.
  if (queue.length > 32) {
    const first = queue[0], latest = queue[queue.length - 1];
    const t = now <= first.start ? 0 : Math.min(1, (now - first.start) / (first.end - first.start));
    const current = {
      x: first.from.x + (first.to.x - first.from.x) * t,
      y: first.from.y + (first.to.y - first.from.y) * t,
      z: first.from.z + (first.to.z - first.from.z) * t,
    };
    queue.splice(0, queue.length, { from: current, to: latest.to, start: now, end: now + duration });
  }
}

function boatVisual(serial, fallback, now) {
  const queue = boatGlides.get(serial >>> 0);
  if (!queue) return { ...fallback, active: false };
  while (queue.length && now >= queue[0].end) queue.shift();
  if (!queue.length) {
    boatGlides.delete(serial >>> 0);
    return { ...fallback, active: false };
  }
  const segment = queue[0];
  const t = now <= segment.start ? 0 : Math.min(1, (now - segment.start) / (segment.end - segment.start));
  return {
    x: segment.from.x + (segment.to.x - segment.from.x) * t,
    y: segment.from.y + (segment.to.y - segment.from.y) * t,
    z: segment.from.z + (segment.to.z - segment.from.z) * t,
    active: true,
  };
}

function ingestBoatMoves(s) {
  const now = performance.now();
  for (const movement of (s && s.boatMoves) || []) {
    const seq = Number(movement.seq) || 0;
    if (!seq || seq <= lastBoatMoveSeq) continue;
    lastBoatMoveSeq = seq;
    const duration = boatMoveDuration(movement.speed);
    for (const entity of movement.entities || []) {
      if (!entity.from || !entity.to) continue;
      queueBoatGlide(entity.serial, entity.from, entity.to, duration, now);
    }
  }
}

function updateAnimStates(s) {
  const now = performance.now();
  const seen = new Set();
  // The chair we're seated on disappeared/moved/changed graphic (someone else used
  // it, a GM deleted it, …) — stand up rather than leave the avatar seated on thin
  // air. Cheap: once per poll (~150ms), not per rendered frame.
  if (sitting && !(s.items || []).some((it) => (it.x | 0) === sitting.x && (it.y | 0) === sitting.y && (it.g | 0) === sitting.graphic)) {
    standUp();
  }
  const touch = (id, x, y, z, dir, body, fb) => {
    seen.add(id);
    let st = anim.get(id);
    if (!st) { st = { rx: x, ry: y, stepDur: 300 }; anim.set(id, st); }
    if (st.tx !== x || st.ty !== y) {
      st.moveUntil = now + 650;
      // Measure this entity's real step cadence so we can glide one tile over
      // exactly that time → continuous motion (no walk-one-tile-then-pause).
      if (st.prevMoveT) st.stepDur = Math.min(600, Math.max(120, now - st.prevMoveT));
      st.prevMoveT = now;
      // A real committed step is always more authoritative than a cosmetic
      // Swing-flash facing (see `ingestSwings`/`drawMobs`) — drop it now
      // rather than waiting out its ~350ms timer.
      st.faceOverride = null;
    }
    Object.assign(st, { tx: x, ty: y, z, dir, body, fallback: fb });
  };
  for (const m of s.mobiles || []) touch("m" + m.serial, m.x, m.y, m.z ?? 0, m.dir ?? 4, m.body, notoColor(m.noto));
  // The player is rendered from the predicted position (set in renderFrame); here
  // we just seed/reconcile prediction against the authoritative server position.
  const p = s.player;
  if (p) {
    if (!pred) {
      pred = {
        x: p.x, y: p.y, z: p.z ?? 0, dir: p.dir ?? 4,
        steps: [], t0: 0, lastEnq: 0, enqGate: 0, moving: false,
        rx: p.x, ry: p.y, rz: p.z ?? 0,
        sx: p.x, sy: p.y, sz: p.z ?? 0, psx: p.x, psy: p.y,
      };
    }
    // Has the authoritative position settled (unchanged since the previous poll)?
    const serverStable = pred.psx === p.x && pred.psy === p.y;
    pred.psx = p.x; pred.psy = p.y;
    pred.sx = p.x; pred.sy = p.y; pred.sz = p.z ?? pred.sz; // server pos (lags ~poll)
    const denies = s.stats?.denies ?? 0;
    const denied = denies > lastDenies;
    lastDenies = denies;
    const off = cheby(pred.x - p.x, pred.y - p.y); // base vs authoritative
    if (off > SNAP_DIST) {
      // Real desync/teleport (gate, recall, big shove): jump everything instantly.
      pred.steps.length = 0; pred.t0 = 0;
      pred.x = p.x; pred.y = p.y; pred.z = p.z ?? pred.z; pred.dir = p.dir ?? pred.dir;
      pred.rx = p.x; pred.ry = p.y; pred.rz = p.z ?? pred.rz;
    } else if (denied) {
      // ClassicUO DenyWalk→Reset: drop the queue, set base to the server position;
      // the render *eases* there (processSteps), so a 1-tile correction glides in.
      pred.steps.length = 0; pred.t0 = 0;
      pred.x = p.x; pred.y = p.y; pred.z = p.z ?? pred.z; pred.dir = p.dir ?? pred.dir;
    } else if (!moveIntent && pred.steps.length === 0 && serverStable && off > 0
               && performance.now() - lastWalkSentAt > RECONCILE_HOLDOFF) {
      // Idle, queue drained, and the server has been settled at a DIFFERENT tile for
      // long enough that it isn't just the last walk's confirm still in flight — a
      // genuine divergence (shove, short teleport, drift). Converge the base. The
      // holdoff is what kills the "뒤로갔다 앞으로" yank: right after you stop, the
      // server momentarily looks settled one tile back (its confirm lagging a poll),
      // and snapping to it then re-snapping forward is exactly the artifact CUO never
      // shows. Prediction is 1:1 with the server, so inside the window we just trust it.
      pred.x = p.x; pred.y = p.y; pred.z = p.z ?? pred.z; pred.dir = p.dir ?? pred.dir;
    } else if (!moveIntent && pred.steps.length === 0 && serverStable && off === 0) {
      // Keep Z authoritative at rest (server-forced Z changes on the tile we're
      // already standing on). `off === 0` is load-bearing: Z belongs to a TILE,
      // so adopting the server's Z while it still reports a DIFFERENT tile
      // paints a foreign tile's height onto ours. That is the height bob on
      // slopes — stop walking, the server is momentarily one tile behind (its
      // confirm lagging a poll, inside RECONCILE_HOLDOFF so the x/y branch above
      // correctly declines to converge), and this branch used to take its Z
      // anyway: the avatar dropped/rose to the old tile's height and then
      // snapped back when the server caught up. Measured live near (1484,1603):
      // 9 foreign-Z adoptions per ~40s of walking, 0 after this guard. When the
      // server genuinely IS on another tile, the branch above converges x, y and
      // Z together once the holdoff proves it's a real divergence.
      pred.z = p.z ?? pred.z;
    }
    const boatPos = boatVisual(p.serial, { x: p.x, y: p.y, z: p.z ?? 0 }, now);
    if (boatPos.active) {
      pred.steps.length = 0; pred.t0 = 0;
      pred.x = p.x; pred.y = p.y; pred.z = p.z ?? pred.z; pred.dir = p.dir ?? pred.dir;
      pred.rx = boatPos.x; pred.ry = boatPos.y; pred.rz = boatPos.z; pred.moving = true;
    }
    if (!anim.has("self")) anim.set("self", { rx: pred.rx, ry: pred.ry, rz: pred.rz, stepDur: 400, fallback: 0xffffff });
    seen.add("self");
  }
  for (const id of [...anim.keys()]) if (!seen.has(id)) anim.delete(id);
}

// add/remove only the tiles/statics that entered/left the view
function syncWorld(s) {
  // Keep the calculateNewZ static index in step with the scene the rest of
  // this function is about to render (once per poll — see its own doc).
  rebuildStaticIndex(s.statics);
  rebuildItemIndex(s.items);
  const m = s.map || { radius: 14, tiles: [], cx: 0, cy: 0 };
  const span = 2 * m.radius + 1;
  // Beyond this Chebyshev distance from the player, `m.tiles` is the grayed-out
  // land-only context ring (server sends land but no statics out there — see
  // scene.rs's `beyond_view`). Older/degraded scenes without `viewRange` treat
  // the whole window as "in view" (no ring), matching the old behaviour.
  const viewRange = (m.viewRange != null) ? m.viewRange : m.radius;
  const seenT = new Set(), seenS = new Set();

  // height of a tile in the window (for corner slopes); null if outside
  const zAt = (x, y) => {
    const col = x - m.cx + m.radius, row = y - m.cy + m.radius;
    if (col < 0 || col >= span || row < 0 || row >= span) return null;
    const t = m.tiles[row * span + col];
    return t ? (t.z | 0) : null;
  };
  for (let row = 0; row < span; row++) {
    for (let col = 0; col < span; col++) {
      const t = m.tiles[row * span + col];
      // Land graphics 0–2 are UO's "no draw"/void tiles (their art is a literal
      // "NO DRAW" placeholder bitmap). They appear under building floors / off-map;
      // ClassicUO never draws them, so neither do we (drop any stale sprite too).
      const x = m.cx + (col - m.radius), y = m.cy + (row - m.radius);
      const key = x + "," + y;
      // Grayed-out context ring past the player's actual view range (ClassicUO-
      // style "you remember the land is there"): land only, dimmed/desaturated,
      // no art/texmap loads — see the render branch below and its `gray` pool flag.
      const beyondView = Math.max(Math.abs(col - m.radius), Math.abs(row - m.radius)) > viewRange;
      if (!t || !t.g || t.g <= 2) {
        const e = tilePool.get(key);
        if (e) { world.removeChild(e.sp); e.sp.destroy(); tilePool.delete(key); }
        continue;
      }
      // Hidden by the Z ceiling (e.g. surface terrain over a basement): drop any
      // existing sprite and don't draw, so the floor below is revealed.
      if (t.h) {
        const e = tilePool.get(key);
        if (e) { world.removeChild(e.sp); e.sp.destroy(); tilePool.delete(key); }
        continue;
      }
      seenT.add(key);
      const z0 = t.z | 0;
      const e = tilePool.get(key);
      // Unchanged: nothing to rebuild. (LRU freshness for every pool entry —
      // including tiles this loop never revisits, e.g. pruneFar's hysteresis
      // ring — is stamped once per poll in one blanket pass; see
      // forEachLiveTexUrl()/touchTex() at the end of this function.)
      // Unchanged real tile: nothing to rebuild. A colour-fallback tile is re-evaluated
      // so it can upgrade to real art — but only rebuild it once the texture actually
      // arrives; while it's still pending (or the art is missing) keep the existing
      // placeholder instead of re-creating an identical Graphics every poll. A gray
      // beyond-view tile (below) never has real art to upgrade to, so it's pooled
      // with `fallback: false` and skips this re-check entirely — UNLESS `beyondView`
      // itself flipped since the last poll (the player crossed the draw-distance
      // boundary for this tile), which must force a rebuild into/out of grayscale.
      if (e && e.g === t.g && e.z === z0 && !!e.gray === beyondView) {
        if (!e.fallback) continue;
        if (texFor(e.url) == null) continue;
      }
      // corner heights (ClassicUO: top=this, right=(x+1,y), bottom=(x+1,y+1), left=(x,y+1)).
      // At the window's SE edge a neighbour falls outside the grid (null); rather than
      // skip the tile — which flashes the black page background at the diamond's rim —
      // fall back to this tile's own Z so it still renders (flat).
      const z1 = zAt(x + 1, y) ?? z0, z2 = zAt(x + 1, y + 1) ?? z0, z3 = zAt(x, y + 1) ?? z0;
      const sloped = !(z0 === z1 && z1 === z2 && z2 === z3);

      // Until the art/texmap PNG has streamed in, draw a flat diamond in the tile's
      // server-provided average colour instead of `continue`-ing (which would leave the
      // black page background showing through). The `fallback` flag makes the unchanged-
      // check above re-evaluate it every poll until the real texture resolves.
      let sp, texUrl, fallback = false;
      if (beyondView) {
        // Grayscale, dimmed diamond — no textured art or statics ever load for
        // this ring (the server never sends them out here; see scene.rs), so
        // this branch never touches texFor and never sets `fallback`.
        const L = Math.min(255, Math.round((0.3 * t.c[0] + 0.59 * t.c[1] + 0.11 * t.c[2]) * 0.65 + 50));
        sp = makeColorTile(x, y, z0, z1, z2, z3, [L, L, L]);
      } else if (!sloped || !(t.tx > 0)) {
        // Flat 44x44 diamond at this tile's own Z — either the ground really is
        // level, or there is no texmap and ClassicUO refuses to stretch at all.
        // `Land.ApplyStretch` bails the moment the texmap entry is empty and
        // sets `AverageZ = MinZ = z`, so such a tile is drawn flat however the
        // ground around it stands, seams and all. We used to stretch the tile's
        // own 44x44 art onto the quad instead, which is seamless but smears the
        // diamond over a steep slope — the reason texmaps exist. Measured
        // against the real tiledata this is 23 of 2724 land graphics and every
        // one of them is Wet, i.e. the whole footprint is water at a shoreline,
        // which ClassicUO deliberately never stretches (`IsStretched` is
        // pre-seeded to `TexID == 0 && IsWet` purely to refuse it).
        texUrl = `art/land/${t.g}.png`;
        const tex = texFor(texUrl);
        if (tex) sp = makeFlatTile(x, y, z0, tex);
        else { sp = makeColorTile(x, y, z0, z0, z0, z0, t.c); fallback = true; }
      } else {
        texUrl = `texmap/${t.tx}.png`; // seamless texture for slopes
        const tex = texFor(texUrl);
        if (tex) sp = makeStretchedTile(x, y, z0, z1, z2, z3, tex);
        else { sp = makeColorTile(x, y, z0, z1, z2, z3, t.c); fallback = true; }
      }
      if (e) { world.removeChild(e.sp); e.sp.destroy(); }
      world.addChild(sp);
      tilePool.set(key, { sp, g: t.g, z: z0, url: texUrl, fallback, gray: beyondView });
    }
  }
  // Server now sends statics across the whole land window, including the
  // grayed-out beyond-view ring — gray those to match the land tiles there
  // (see STATIC_GRAY above). `beyondView` is per-static, re-evaluated every
  // poll so a static flips gray/color as the player walks past `VR`.
  const P = s.player || { x: 0, y: 0 };
  const VR = (s.map && s.map.viewRange != null) ? s.map.viewRange : (s.map ? s.map.radius : 18);
  for (const st of s.statics || []) {
    const key = `${st.x},${st.y},${st.g},${st.z}`;
    seenS.add(key);
    const beyondView = Math.max(Math.abs(st.x - P.x), Math.abs(st.y - P.y)) > VR;
    if (staticPool.has(key)) {
      // Unchanged sprite identity, but the player may have crossed the view
      // boundary since the last poll — flip its filter without rebuilding it.
      const ex = staticPool.get(key);
      if (ex && ex._gray !== beyondView) {
        ex.filters = beyondView ? [STATIC_GRAY] : null;
        ex._gray = beyondView;
      }
      continue; // unchanged; see the blanket LRU-touch note above
    }
    const texUrl = `art/static/${st.g}.png`;
    const tex = texFor(texUrl);
    if (!tex) continue;
    const sp = new PIXI.Sprite(tex);
    sp.anchor.set(0.5, 1.0);
    sp.x = isoX(st.x, st.y); sp.y = isoY(st.x, st.y, st.z) + HALF;
    sp.zIndex = depthZ(st.x, st.y, st.pz ?? st.z, 4);
    sp._gray = beyondView;
    sp.filters = beyondView ? [STATIC_GRAY] : null;
    if (st.ms != null) {
      sp._boatSerial = st.ms >>> 0;
      sp._boatBaseX = st.x; sp._boatBaseY = st.y; sp._boatBaseZ = st.z;
      sp._boatBaseSpriteX = sp.x; sp._boatBaseSpriteY = sp.y;
      sp._boatPzOffset = (st.pz ?? st.z) - st.z; sp._boatDepthBias = 4;
    }
    // Tile + foliage flag for the transparency pass (circle-of-transparency / foliage fade).
    sp._tx = st.x; sp._ty = st.y; sp._foliage = !!st.f;
    sp._texUrl = texUrl; // so the "still on screen" branch above can keep it LRU-fresh
    // Animated static (flames/fountains/water wheels): the server baked the ART
    // tile-id frame sequence (`a`) + per-frame interval ms (`ai`). Prefetch each
    // frame's texture and store them so the animation pass can swap sp.texture.
    if (Array.isArray(st.a) && st.a.length > 1) {
      const frameUrls = st.a.map((id) => `art/static/${id}.png`);
      sp._frames = frameUrls.map((u) => texFor(u));
      sp._frameUrls = frameUrls;   // so tickAnimatedStatics/touch can re-resolve/re-stamp by url
      sp._afids = st.a;            // keep ids so late-loading frames can be resolved
      sp._ai = st.ai || 200;
      sp._fbase = performance.now();
      sp._fidx = -1;
      animatedStatics.add(sp);
    }
    world.addChild(sp);
    staticPool.set(key, sp);
  }

  // Dynamic world items (doors, furniture, signs, corpses…): draw their REAL art
  // like statics, depth-sorted, instead of as dots. Persistent pool keyed by serial
  // (items move/open/disappear → re-create when graphic or position changes).
  const seenI = new Set();
  for (const it of s.items || []) {
    if (it.serial === undefined || !it.g) continue;
    const key = it.serial;
    seenI.add(key);
    const iz = it.z | 0;
    // A corpse (graphic 0x2006) carries the dead creature's Corpse.def-remapped
    // body, facing and death-pose group from the server (see scene.rs). Once that
    // anim's frame count AND the last frame's texture have both loaded, draw the
    // held death-pose frame instead of the generic corpse art; until then (or if
    // the anim is absent) `corpseUrl` stays null and we fall through to the static
    // art below, same as any other item.
    let corpseUrl = null, corpseFrame = -1, corpseTex = null;
    if (it.g === 0x2006 && it.body != null) {
      const dir = it.dir & 7, dg = it.dg | 0;
      framesFor(it.body, dg, dir); // kick the animinfo (frame-count/centers) load
      const fk = `${it.body}/${dg}/${dir}`;
      const loaded = frameCount.has(fk) ? frameCount.get(fk) : 0;
      if (loaded > 0) {
        corpseFrame = loaded - 1; // the death pose's final (held) frame
        const url = `anim/${it.body}/${dg}/${dir}/${corpseFrame}.png` + (it.hue ? `?hue=${it.hue}` : "");
        const t = texFor(url);
        if (t) { corpseUrl = url; corpseTex = t; }
      }
    }
    const e = itemPool.get(key);
    // A stack on the GROUND has to pick its amount-tiered art too, exactly like the
    // container/paperdoll icons do — otherwise 60,000 gold lying at your feet draws
    // as a single coin (the OPL correctly said 60,000, but the sprite didn't).
    // Comparing the RESOLVED graphic below is what makes a growing/shrinking pile
    // re-texture: `it.g` never changes when only the amount does.
    const stackG = corpseUrl ? (it.g | 0) : stackGraphic(it.g, it.amount | 0);
    // Dye hue: the server bakes the recolor into the art (`?hue=`, PartialHue
    // already folded into bit 0x8000 — see scene.rs `item_art_hue`). It has to be
    // in the change key below for the same reason `stackG` is: a `[set Hue` leaves
    // the graphic and position identical, so without it a dyed item keeps its old
    // colour until it happens to move.
    // Never for a corpse: scene.rs excludes 0x2006 from `item_art_hue` because its
    // `hue` is the Corpse.def-remapped BODY hue, not an art dye. Keying off
    // `corpseUrl` instead of the graphic tinted the generic corpse tile on every
    // poll before the death-pose animinfo resolved — and forever when that fetch
    // came back with frameCount 0.
    const iHue = it.g === 0x2006 ? 0 : (it.hue | 0);
    const hueQ = hueQuery(iHue);
    if (e && e.g === stackG && e.hue === iHue && e.x === it.x && e.y === it.y && e.z === iz && e.corpseUrl === corpseUrl) continue; // unchanged; see the blanket LRU-touch note above
    const itemTexUrl = corpseUrl || `art/static/${stackG}.png${hueQ}`;
    const tex = corpseTex || texFor(itemTexUrl);
    if (!tex) continue; // await art, retry next poll
    if (e) { animatedStatics.delete(e.sp); world.removeChild(e.sp); e.sp.destroy(); }
    const sp = new PIXI.Sprite(tex);
    const x = isoX(it.x, it.y), y = isoY(it.x, it.y, iz);
    // A resolved death-pose frame anchors by its draw-center, same as a mobile's
    // anim frames (see drawMobs' `part()`); otherwise (loading, or a non-corpse
    // item) foot-anchor like any static.
    const c = corpseUrl ? centerFor(it.body, it.dg | 0, it.dir & 7, corpseFrame) : null;
    if (c) {
      sp.anchor.set(0, 0);
      sp.x = x - c[0]; sp.y = (y - 3) - tex.height - c[1];
    } else {
      sp.anchor.set(0.5, 1.0);
      sp.x = x; sp.y = y + HALF;
    }
    sp.zIndex = depthZ(it.x, it.y, it.pz ?? iz, 5); // bias 5: just above same-tile statics
    sp._boatSerial = it.serial >>> 0;
    sp._boatBaseX = it.x; sp._boatBaseY = it.y; sp._boatBaseZ = iz;
    sp._boatBaseSpriteX = sp.x; sp._boatBaseSpriteY = sp.y;
    sp._boatPzOffset = (it.pz ?? iz) - iz; sp._boatDepthBias = 5;
    // Tile + foliage flag for the transparency pass (circle-of-transparency / foliage fade).
    sp._tx = it.x; sp._ty = it.y; sp._foliage = !!it.f;
    sp.eventMode = "static"; sp.cursor = "pointer";
    // Per-pixel hit-testing (see pixelHitArea above). The closure re-reads the
    // CURRENT frame because an animated item's texture is swapped under it by
    // `tickAnimatedStatics` — testing a campfire's click against frame 0 while a
    // taller flame is drawn makes the click fall through to whatever is behind it.
    // `pixelHitArea` calls `getUrl()` on every `contains`, so following `_fidx`
    // costs nothing for the still items that keep `_frameUrls` undefined.
    sp.hitArea = pixelHitArea(sp, () =>
      (sp._frameUrls && sp._fidx >= 0 ? sp._frameUrls[sp._fidx] : itemTexUrl));
    const serial = it.serial;
    sp.on("pointerdown", (ev) => onEntityPointerDown(serial, ev, true)); // world item → loot on dbl-click
    sp.on("pointerover", () => { hoverEntity(serial); targetHighlightOn(sp); });
    sp.on("pointerout", () => { hoverOut(serial); targetHighlightOff(sp); });
    // Animated dynamic item (campfire, spell field, brazier): the server bakes the
    // same animdata frame list statics get (`a`/`ai`, see scene.rs `anim_frames`),
    // so hand it to the very same tick pass — a spawned campfire has to flicker
    // exactly like a mapped one. Skipped for a corpse, whose texture is an anim
    // frame this pass must not swap out from under.
    if (!corpseUrl && Array.isArray(it.a) && it.a.length > 1) {
      const frameUrls = it.a.map((id) => `art/static/${id}.png${hueQ}`);
      sp._frames = frameUrls.map((u) => texFor(u));
      sp._frameUrls = frameUrls;
      sp._afids = it.a;
      sp._ai = it.ai || 200;
      sp._fidx = -1;
      animatedStatics.add(sp);
    }
    world.addChild(sp);
    itemPool.set(key, { sp, g: stackG, hue: iHue, x: it.x, y: it.y, z: iz, corpseUrl, url: itemTexUrl });
    markDirty();
  }
  for (const [k, e] of itemPool) {
    if (!seenI.has(k)) { animatedStatics.delete(e.sp); world.removeChild(e.sp); e.sp.destroy(); itemPool.delete(k); markDirty(); }
  }

  // Tiles: keep once drawn — only drop them when they're well outside the
  // window (hysteresis), so sliding the camera never re-creates visible tiles.
  pruneFar(tilePool, m.cx, m.cy, m.radius + 4);
  // Statics: seen-based — a roof the server stops sending (player under cover)
  // must be removed so the interior shows.
  prune(staticPool, seenS, (e) => e);
  diag.tiles = tilePool.size + staticPool.size;
  // Stamp EVERY texture a live sprite/anim-part could currently be showing as
  // "just used", once per poll — not only the ones this function's own diff
  // loops happen to touch. Two real escapes found live, both live sprites the
  // window loops above never revisit: (1) pruneFar's hysteresis-ring tiles
  // (kept in tilePool at Chebyshev radius+1..+4 — on stage/screen for camera-
  // slide hysteresis — but outside the span×span window loop that walks
  // m.tiles); (2) a mobile's st.partTex last-good fallback (drawMobs reuses an
  // old, already-drawn texture the instant a frame's current url hasn't
  // resolved yet, without going through texFor/touchTex for THAT texture's own
  // url). Left stale past TEX_IDLE_MS, either could get destroyed out from
  // under an on-stage sprite by sweepTexCache → app.render() throws → the rAF
  // loop dies (see frame()'s own resilience fix). ~1.3k Map.set calls at a
  // typical view radius — trivial next to the rest of this function.
  forEachLiveTexUrl(touchTex);
}
// Every texture url currently referenced by a live, on-stage sprite or a
// mobile's per-part fallback (drawMobs's st.partTex — see part() below for why
// it stores {tex,url} pairs, not just the texture). Two call sites: (1) a
// per-poll touchTex sweep (syncWorld, above) so none of these ever look "idle"
// to sweepTexCache's LRU scan; (2) sweepTexCache's own belt-and-braces
// exclude-list, so even a url this pass fails to touch can never be evicted
// while still referenced live.
function forEachLiveTexUrl(fn) {
  for (const e of tilePool.values()) fn(e.url);
  for (const sp of staticPool.values()) {
    fn(sp._texUrl);
    if (sp._frameUrls) for (const u of sp._frameUrls) fn(u);
  }
  for (const e of itemPool.values()) {
    fn(e.url);
    if (e.sp._frameUrls) for (const u of e.sp._frameUrls) fn(u); // animated item's other frames
  }
  for (const st of anim.values()) {
    if (st.partTex) for (const e of st.partTex.values()) fn(e.url);
  }
}
function prune(pool, seen, getSp) {
  for (const [key, e] of pool) {
    if (!seen.has(key)) {
      const sp = getSp(e);
      animatedStatics.delete(sp); // drop from the animation set before destroy
      world.removeChild(sp); sp.destroy(); pool.delete(key);
    }
  }
}
function pruneFar(pool, cx, cy, maxDist) {
  for (const [key, e] of pool) {
    const i = key.indexOf(",");
    const x = +key.slice(0, i), y = +key.slice(i + 1);
    if (Math.abs(x - cx) > maxDist || Math.abs(y - cy) > maxDist) {
      world.removeChild(e.sp); e.sp.destroy(); pool.delete(key);
    }
  }
}

// A flat land tile: the 44×44 diamond art as a centered sprite.
function makeFlatTile(x, y, z, tex) {
  const sp = new PIXI.Sprite(tex);
  sp.anchor.set(0.5, 0.5);
  sp.x = isoX(x, y); sp.y = isoY(x, y, z); sp.zIndex = depthZ(x, y, z - 2, 0);
  return sp;
}

// A sloped land tile: a 4-corner quad whose vertices follow the corner heights
// (top=this, right=(x+1,y), bottom=(x+1,y+1), left=(x,y+1)), per ClassicUO.
//
// The texture is always a seamless texmap, so the UVs are the unit square
// mapped corner to corner — ClassicUO's `_cornerOffsetX/Y` identity in
// `DrawStretchedLand`. There is no land-art variant: a tile without a texmap is
// never stretched at all (see the caller), which is exactly why one texture
// source suffices here.
//
// No half-texel inset on those UVs, deliberately. The inset ClassicUO applies
// in `CalculateHalfPixelUVs` exists because its terrain lives in an atlas, so a
// vertex at the region's edge samples the first texel of whatever was packed
// next door — a fringe of foreign terrain. Every texmap here is loaded as its
// own standalone texture (`PIXI.Assets.load` per URL, no Spritesheet anywhere),
// so there is no neighbour to bleed in and clamping handles the edge.
function makeStretchedTile(x, y, z0, z1, z2, z3, tex) {
  const Bx = (x - y) * HALF, By = (x + y) * HALF;
  const aPosition = [
    Bx,        By - HALF - z0 * ZSTEP, // top
    Bx + HALF, By        - z1 * ZSTEP, // right
    Bx,        By + HALF - z2 * ZSTEP, // bottom
    Bx - HALF, By        - z3 * ZSTEP, // left
  ];
  const aUV = [0, 0, 1, 0, 1, 1, 0, 1];
  const geometry = new PIXI.Geometry({ attributes: { aPosition, aUV }, indexBuffer: [0, 1, 2, 0, 2, 3] });
  const mesh = new PIXI.Mesh({ geometry, texture: tex });
  // Sort a sloped tile by its AverageZ (ClassicUO Land.CalculateAverageZ: the mean
  // of whichever diagonal corner-pair differs less), NOT the top corner — otherwise
  // a slope whose top corner is high sorts in front of taller statics (e.g. stairs)
  // that sit behind it and wrongly covers them. (top=z0, right=z1, bottom=z2, left=z3)
  const avgZ = Math.abs(z0 - z2) <= Math.abs(z3 - z1) ? (z0 + z2) >> 1 : (z3 + z1) >> 1;
  mesh.zIndex = depthZ(x, y, avgZ - 2, 0);
  return mesh;
}

// A flat-colour diamond placeholder for a land tile whose art/texmap PNG hasn't
// streamed in yet (or a window-edge tile with no neighbour to slope against). Uses
// the server-provided average colour `c` so terrain never flashes the black page
// background during load/scroll; replaced by the textured tile on a later poll once
// texFor() resolves. Corner heights follow the same layout as makeStretchedTile.
function makeColorTile(x, y, z0, z1, z2, z3, c) {
  const Bx = (x - y) * HALF, By = (x + y) * HALF;
  const g = new PIXI.Graphics();
  g.poly([
    Bx,        By - HALF - z0 * ZSTEP, // top
    Bx + HALF, By        - z1 * ZSTEP, // right
    Bx,        By + HALF - z2 * ZSTEP, // bottom
    Bx - HALF, By        - z3 * ZSTEP, // left
  ]).fill(Array.isArray(c) && c.length === 3 ? ((c[0] << 16) | (c[1] << 8) | c[2]) : 0x101418);
  const avgZ = Math.abs(z0 - z2) <= Math.abs(z3 - z1) ? (z0 + z2) >> 1 : (z3 + z1) >> 1;
  g.zIndex = depthZ(x, y, avgZ - 2, 0);
  return g;
}

// Is world tile (x,y) walkable per the latest scene? (outside the window → assume yes)
function tileWalkable(x, y) {
  const m = scene && scene.map;
  if (!m) return true;
  const span = 2 * m.radius + 1;
  const col = x - m.cx + m.radius, row = y - m.cy + m.radius;
  if (col < 0 || col >= span || row < 0 || row >= span) return true;
  const t = m.tiles[row * span + col];
  return t ? t.w === 1 : true;
}

// Predicted standing Z for stepping onto (x,y) — the server's per-tile `sz`
// (CalculateNewZ). null when outside the window. Lets prediction raise/lower Z
// together with the step so stairs glide instead of popping.
function tileSZ(x, y) {
  const m = scene && scene.map;
  if (!m) return null;
  const span = 2 * m.radius + 1;
  const col = x - m.cx + m.radius, row = y - m.cy + m.radius;
  if (col < 0 || col >= span || row < 0 || row >= span) return null;
  const t = m.tiles[row * span + col];
  if (!t) return null;
  return t.sz !== undefined ? (t.sz | 0) : (t.z | 0);
}

// Serial of the closed door blocking (x,y), if any — the optional `dr` field the
// server attaches to a land tile ONLY when it's blocked SOLELY by an openable
// closed door (a tile blocked by a door AND e.g. a crate does not get it). null
// when there's no door, (x,y) is outside the client's map window, or the server
// predates this field — all three cases are indistinguishable and all correctly
// mean "nothing to auto-open here". Feeds tryAutoOpenDoor() near canWalk().
function tileDoor(x, y) {
  const m = scene && scene.map;
  if (!m) return null;
  const span = 2 * m.radius + 1;
  const col = x - m.cx + m.radius, row = y - m.cy + m.radius;
  if (col < 0 || col >= span || row < 0 || row >= span) return null;
  const t = m.tiles[row * span + col];
  return t && t.dr !== undefined ? (t.dr >>> 0) : null;
}

