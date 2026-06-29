# Movement, Animation & Tiles — ClassicUO model → anima-client port

> Authoritative reference for how movement/animation/tiles work in ClassicUO and
> how we port them to anima-client's three layers. Implement against this doc;
> when reality disagrees, fix the code to match ClassicUO (cite file:line).

ClassicUO source: `~/dev/uo/classicuo/src/ClassicUO.Client`.

---

## 0. Our three layers (where each piece lives)

ClassicUO is a single process: client = renderer = predictor, talking straight to
the server. We are split:

```
browser (web/main.js)            ── prediction + smooth pixel interpolation + animation frames
   │  HTTP: GET /scene.json (poll ~150ms) · POST /input (intent)
native play server (anima-net)   ── one live Session; PACES walk steps to UO cadence
   │  TCP (sans-IO core drives it)
anima-core::net::movement Walker ── sequence/fastwalk, ConfirmWalk/DenyWalk, predicts the next tile
   │  TCP
ServUO
```

> **Loop-tick precision (learned the hard way).** ClassicUO checks its pacing gate
> (`LastStepRequestTime`) **every frame (~16ms)**, so a 200ms run cadence fires at
> ~200ms. Our play-server loop must likewise tick fast: the socket read timeout is
> **20ms** (`PUMP_READ_TIMEOUT`), not 400ms — a long timeout stalls the loop waiting
> for a packet, so the cadence gate is only checked every ~400ms and **running gets
> throttled down to walk speed** (measured: run 423ms → after fix 208ms/tile).
>
> **Cadence accuracy = no stutter (learned harder).** Even at 20ms socket timeout, the
> play loop period is `observe(20ms)` + scene build. Cold-cache builds (decoding new
> land art for `land_avg_color`) spike to ~40ms, so the gate fired late and **run paced
> at ~256ms while the browser predicts 200ms → the prediction outran the server →
> `LEAD_CAP` stall (stutter) + `SNAP_DIST` (jump)**. Two fixes: (1) `observe` 60→20ms;
> (2) a **fixed-timestep accumulator** — on each step set `last_step = now - overshoot`
> (overshoot past the gate, capped at one step) instead of `= now`, so the *average*
> cadence is exact despite a coarse loop. Result: run true **median 200ms**, walk 400ms,
> no spikes. Client `LEAD_CAP`/`SNAP_DIST` raised to 3.5/4.5 as jitter headroom (denies
> still snap immediately). The committed base (server) and the prediction now advance at
> the same rate, so the lead stays bounded → smooth.

> **Pacing model — the browser IS the pacer (ClassicUO-faithful, fixes tap overshoot).**
> ClassicUO calls `PlayerMobile.Walk` every frame while a key is held; `Walk` self-gates
> on `LastStepRequestTime` (queues ONE step per walkTime — turn=100ms, move=cadence) and
> sends one `WalkRequest` per step. Release just stops calling Walk → the queue drains.
> We now do the same: the browser prediction (`enqueueSteps`) is gated by `lastStepReq =
> now + walkTime` and queues one step per cadence; `processSteps` renders it and sends one
> `walk` per **committed** step; the play-server executes each once (no `desired_until`
> pacing). So a key *tap* = exactly one tile (was two — the old ungated buffer queued two
> steps, and a separate server `desired_until` kept pacing after release), the move right
> after a turn fires in 100ms, and release stops immediately. There is **no gate
> lookahead** — `enqueueSteps` queues a step only once `now >= lastStepReq`, exactly
> one per walkTime. (An earlier ~28ms lookahead pre-queued the *next* tile a frame
> early; if the key released in that window the pre-queued step still committed → a
> "한 발자국 더" overshoot. Removing it costs at most a 1-frame boundary gap, far less
> bad than an extra tile.) On release `enqueueSteps` finishes the in-progress front
> step (it commits → one `walk` = the tile you were already walking into) and drops
> any buffered step — ClassicUO-faithful: queued steps complete forward, no new one
> starts, never a backward "round-down" (an earlier rounding-to-nearest version eased
> the render *backward* when `prog < 0.5`, which read as a correction the real client
> never shows — reverted).
>
> **Reconcile hold-off (kills the "뒤로갔다 앞으로" yank).** The soft at-rest
> reconcile (`!moveIntent && queue empty && serverStable && off>0 → pred.x = p.x`) is
> meant for genuine divergence (shove, short teleport, drift). But right after you
> stop, the *last* walk's confirm can lag one poll, so the server momentarily looks
> "settled" one tile back; the reconcile then snaps the base backward, and when the
> confirm lands it snaps forward again — a visible backward-then-forward slide on
> ~¼–½ of stops. Fix: only soft-reconcile once `performance.now() - lastWalkSentAt >
> RECONCILE_HOLDOFF` (500 ms), past the confirm window. Prediction is 1:1 with the
> server (verified: browser commits == server MOVED, denies==0), so inside the window
> we just trust it. Measured: with the hold-off the post-stop backward slide is 0.000
> tiles across all directions; without it the test reproduces a 0.9-tile yank.
>
> **Gate anchoring (fixes a mid-walk micro-stutter).** `processSteps` commits on a
> *rigid* grid (`t0 += dur`), but a naïve gate `lastStepReq = now + walkTime` is set
> from jittery wall-clock: each enqueue fires at `now >= prev gate` (only ever later),
> so the gate creeps forward relative to the commit grid until, on the very frame a
> step commits, enqueue is still blocked → the queue drains for one frame → the walk
> visibly hitches (`q=1` every step in the trace, cadence wandering 400→900ms). Fix:
> while movement is *continuous* (this enqueue is within one cadence of the last) we
> advance the gate from the **previous gate**, not `now`, so it stays locked to the
> grid and the next step is always queued the same frame the current one commits
> (`q` oscillates 1↔2, never 0). After an idle/release gap we restart from `now` so
> taps and resume are unchanged. Verified: held walk/run `starvation=0` (was ~5/170),
> taps still exactly 1 tile, `denies=0`.

So ClassicUO's single Walk loop maps to **three** cooperating pieces:
- **core `Walker`** = ClassicUO's `WalkerManager` (sequence, fastwalk, confirm/deny, the authoritative client position).
- **play-server pacing loop** = ClassicUO's `PlayerMobile.Walk` *gate* (`LastStepRequestTime`, turn-vs-move timing, mounted/run speed).
- **browser prediction** = ClassicUO's `Mobile.ProcessSteps` smooth interpolation + the *instant* turn/first-step feel (because of the HTTP round-trip we predict locally and reconcile).

---

## 1. Timing constants (ClassicUO)

`Game/Constants.cs`, `Game/Data/MovementSpeed.cs`:

| const | ms | meaning |
|-------|----|---------|
| `TURN_DELAY` | 100 | a pure facing change (turn) |
| `TURN_DELAY_FAST` | 45 | (unused) |
| `PLAYER_WALKING_DELAY` | 150 | `IsWalking` window |
| `CHARACTER_ANIMATION_DELAY` | 80 | render frame interval → frames-per-tile |
| `MAX_STEP_COUNT` | 5 | queued/pending steps cap |
| `MAX_FAST_WALK_STACK_SIZE` | 5 | fastwalk key buffer |
| `STEP_DELAY_WALK` | 400 | foot walk / tile |
| `STEP_DELAY_RUN` | 200 | foot run / tile |
| `STEP_DELAY_MOUNT_WALK` | 200 | mounted walk / tile |
| `STEP_DELAY_MOUNT_RUN` | 100 | mounted run / tile |

```
TimeToCompleteMovement(run, mounted) =
    mounted ? (run ? 100 : 200)
            : (run ? 200 : 400)
```
"mounted" for timing = `IsMounted || SpeedMode∈{FastUnmount, FastUnmountAndCantRun} || IsFlying`.

Pixel distance per tile: **cardinal (N/E/S/W) = 22px, diagonal = 44px** (iso). Z: 4px per z.

---

## 2. Walk decision — turn vs move (`PlayerMobile.Walk`, lines 510-674)

Gate (return false / can't step now) if any:
- `Walker.WalkingFailed` (server rejected a step → awaiting resync),
- `Walker.LastStepRequestTime > now` (cadence not elapsed),
- `Walker.StepsCount >= 5` (queue full),
- paralyzed (CV≥6.0.14.2).

Speed: `run |= AlwaysRun`; force walk if `SpeedMode>=CantRun` or `Stamina<=1 && !dead`.

Start position = the **last queued step's** end (predict ahead), else current tile.

`walkTime = TURN_DELAY (100)` by default. Then:
- **Same facing as requested dir:** `Pathfinder.CanWalk` → if it yields the same dir, it's a **MOVE**: advance x/y/z and `walkTime = TimeToCompleteMovement(run, mounted)`. If CanWalk redirected (diagonal blocked), it's a **TURN** only (walkTime stays 100).
- **Different facing:** first this only **TURNS** to the new dir (walkTime 100, no move). The **MOVE** happens on the *next* Walk call (now facing == dir). → classic **turn-then-move**.

After deciding: push a `StepInfo` (Sequence, Running, Old/Direction, X/Y/Z, Timer, NoRotation), `StepsCount++`, send `WalkRequest(dir|runbit, seq, fastwalkKey)`, advance `WalkSequence` (1..255, FF→1, never 0), and set `Walker.LastStepRequestTime = now + walkTime`.

---

## 3. Walker / sequence / fastwalk / confirm / deny (`WalkerManager.cs`)

- `WalkSequence` 0..255 (FF→1). After **DenyWalk**, reset to **0** (server resets too; the next request *must* be seq 0).
- `StepInfos[5]`: ring of pending steps with `Accepted` flags. `ConfirmWalk(seq)` marks the matching step accepted, advances the confirmed position; out-of-order/unknown seq → `Send_Resync()` + `WalkingFailed=true` (gates Walk until a DenyWalk resyncs).
- **DenyWalk(seq,x,y,z,dir)** (`0x21`): clear steps, `Reset()` (sequence→0, StepsCount→0, WalkingFailed=false), teleport to (x,y,z) + set facing. Clears render offset.
- **ConfirmWalk(seq)** (`0x22`): `[seq][notoriety&~0x40]`; sets player notoriety, accepts the step.
- **FastWalk**: server seeds a 5-key stack (via `0xBF`); each WalkRequest pops one key (`GetValue`) into the packet (anti-cheat). 0 when empty (stock shards accept 0).

### Our core `Walker` (anima-core/src/net/movement.rs) — now a faithful `WalkerManager` port
- ✅ sequence (0..255, FF→1, reset 0 on deny), fastwalk pop from `World.fast_walk`, optimistic facing on send.
- ✅ **5-slot `StepInfos` ring** (`MAX_STEP_COUNT`): each sent step is queued with its sequence; `step()` predicts from the queue **tail** (`Steps.Back()`), so multiple in-flight steps don't collapse onto the same tile.
- ✅ **ConfirmWalk** accepts the in-order front step and commits its tile to `World` (the headless equivalent of ClassicUO updating `RangeSize`; we have no separate render loop). A stray confirm with nothing pending is ignored.
- ✅ **bad/out-of-order confirm → Resync + `WalkingFailed`**: emits `Send_Resync` (`0x22`, via `Walker::take_resync` flushed in `Session::handle_frame`) and gates `step()` until a `DenyWalk` resyncs (ClassicUO `ConfirmWalk` isBadStep).
- ✅ **DenyWalk** → clear ring + `Reset` (seq→0) + teleport.
- In practice the queue stays depth ≤1 (the play-server paces one step per cadence and confirms are local/fast), but the ring + Resync now match ClassicUO under loss/latency.

---

## 4. Mounted (`Mobile.IsMounted`, lines 164-177)

`IsMounted` = there's an item on `Layer.Mount` (25 = 0x19), not driving a boat, valid graphic. Mounted ⇒ half the step time (see §1).

### Port
- `World`: `player_mounted()` = any item with `container==player && layer==0x19 && graphic!=0`. (We already keep worn items from the self `0x78` equipment loop.)
- scene JSON: send `player.mounted`.
- play-server pacing + browser prediction: pick cadence via `TimeToCompleteMovement(run, mounted)`.

---

## 5. Smooth interpolation (`Mobile.ProcessSteps`, lines 689-865)

Logical position jumps to the step target; a **render Offset** interpolates the
sprite from the previous tile over the step duration:
- `frames = maxDelay / CHARACTER_ANIMATION_DELAY(80)`  (maxDelay = step time − 1 frame)
- progress `x = elapsed / 80`
- `GetPixelOffset(dir, x, y, frames)`: cardinal scales by `22/frames`, diagonal by `44/frames`, per-direction sign; clamp to ±22 (cardinal) / ±44 (diagonal).
- `Offset.Z = (destZ−srcZ) * x * (4/frames)` (z eases over first 4 frames).
- when `elapsed >= maxDelay`: commit X/Y/Z, clear Offset, pop step; the walk **animation frame** also advances on this `CHARACTER_ANIMATION_DELAY` cadence.
- Other mobiles: if not flagged mounted but a step arrives in ≤ mount delay, treat as mounted (auto-detect speed from timing).

### Our browser port (web/main.js)
- We don't have the server's per-step timestamps (only ~150ms polls), so we **predict** the player locally (instant turn + step, `LEAD_CAP` ahead of server, reconcile/snap on `SNAP_DIST`) and **glide** every entity toward its target tile at constant velocity timed to the step cadence. Player cadence = `TimeToCompleteMovement(run, mounted)`; other mobiles = measured cadence.
- **Look-ahead buffer (= ClassicUO `Steps` queue / `MAX_STEP_COUNT`).** While a key is held, prediction keeps stepping ahead of the *known* (polled) server position so the avatar never pauses at a tile boundary waiting for the next poll. The known server pos lags by ~poll+confirm (~1.75 tiles at run), so `LEAD_CAP` must exceed that (**2.5**) or prediction stalls every step (the "멈칫" between inputs). The cap also bounds rubber-band if a step is later denied.
- **Deny→snap (= ClassicUO `DenyWalk`→`Reset`).** A bigger buffer means a rejected step would otherwise leave us predicting too far ahead. The browser can't see `DenyWalk` packets, so the scene exposes a `denies` counter; when it increments, prediction snaps straight to the server position (instead of waiting for `SNAP_DIST`).
- Walk-animation frame: group Walk(0) while moving / Stand(4) idle, frame = `floor(now/CHARACTER_ANIMATION_DELAY) % framecount` (see §6). Use mounted ride groups when mounted (§6 TODO).
- This is the pragmatic equivalent of ProcessSteps for a polled web client.

---

## 6. Animation groups (people) — `anima-assets::anim`

Legacy `anim.mul`: people base `(body-400)*175+35000` (35 groups × 5 dirs), monster `body*110` (22). Frame = palette(256×u16 ARGB1555) + RLE. 8→5 direction map + mirror (ClassicUO `GetAnimDirection`).

People groups we use: **WalkUnarmed=0, Stand=4**. Run uses the same group set; mounted uses the **OnmountRide** groups (`OnmountRideSlow=23`, `OnmountRideFast=24`, `OnmountStand=25`). 
- ✅ Stand/Walk frames decoded + served (`/anim/<body>/<group>/<dir>/<frame>.png`).
- TODO: pick RunUnarmed/RunArmed (2/3) when running, and Onmount groups (23-25) when mounted; render the mount sprite under the rider.

---

## 7. Tiles (land flat vs sloped, statics) — `web/main.js syncWorld`

ClassicUO draws a land tile **flat** (44×44 diamond art) when its 4 corner heights
are equal, else **stretched** (a 4-vertex quad following corner heights), using the
tile's **texmap** (seamless texture) when it has one, otherwise the stretched art.
Corners: top=(x,y), right=(x+1,y), bottom=(x+1,y+1), left=(x,y+1).
- ✅ implemented: flat sprite / stretched `PIXI.Mesh` with texmap (`/texmap/<id>.png`) or art; persistent tile pool (only edge tiles added/removed as the camera slides); statics anchored bottom-center, z-ordered by `(x+y)`.
- Z step = 4px. Static z-order/overlap follows `(x+y)*100 (+50 for statics)`.

---

## 8. Implementation checklist (this doc → code)

- [x] turn-then-move (core Walker + optimistic facing); self pos/facing owned by Walker (never overwritten by inbound `0x77`/`0x78` — that bug caused dir oscillation/stalls).
- [x] sequence/fastwalk, confirm/deny resync (single-pending).
- [x] foot walk/run cadence (400/200) server-paced; client prediction + glide; instant turn/first-step.
- [x] real tile art incl. sloped/texmap; persistent pool; statics; body sprites; walk/idle anim frames.
- [x] **mounted**: detect Layer.Mount (`World::player_mounted`) → mounted cadence
  (`movement::step_delay_ms`, 100/200) in play-server pacing *and* browser prediction;
  `player.mounted` sent in scene.
- [x] **animation groups** by state: Run=2 when running, Onmount 23/24/25 when mounted,
  Walk=0/Stand=4 otherwise; frame cadence = `CHAR_ANIM_DELAY` (80ms). (`web/main.js animGroup`)
- [ ] draw the **mount sprite** under the rider (rider now shows the riding pose, but
  the horse body isn't drawn yet — needs the mount item's body→anim).
- [ ] Walker resync on bad confirm; 5-slot StepInfos if we ever pipeline >1 step.
- [x] **turn-then-move timing**: the gate to the next step is the *previous* action's
  walkTime (ClassicUO `LastStepRequestTime = now + walkTime`). A turn costs only
  TURN_DELAY(100), so the first step right after a turn fires 100ms later — not a full
  step cadence. Fixed in both browser (`pred.walkTime`) and play-server pacing
  (`last_walk_time`); previously both waited a full 400/200ms after a turn.
- [x] **don't attempt impassable**: pacing checks `tile_walkable` (= `walkable_z` for
  land/statics **+ dynamic world items** via `MapData::item_blocks`) before a move;
  only turns are unconditional. The renderer's `w` flag uses the same fn, so prediction
  and server agree → no walk-into-wall → DenyWalk → snap. (ClassicUO `Pathfinder.CanWalk`.)
- [x] **animation continuity**: walk/run frame is driven by a per-entity phase that
  advances one full cycle **per tile** (so leg speed tracks step speed — walk vs run
  feet don't slide), the whole (group,dir) cycle is prefetched (no per-frame pop-in),
  and "moving" is held-key/glide based (no walk-in-place moonwalk after stopping;
  holding into a wall walks in place, like UO). (`web/main.js` `animPhase`/`animMoving`.)
- [x] **right-button mouse move** (ClassicUO `MoveCharacterByMouseInput` + `GetMouseDirection`):
  hold RMB → walk toward the cursor; ≥190px from the screen-center (the avatar) → run; an
  18px dead zone in the center. Screen→world dir uses the iso one-step rotation `(screenDir+7)%8`
  (ClassicUO's `facing-1`). Feeds the same `moveIntent` as the keyboard. (`web/main.js`)
- [x] **animation frame cadence**: leg cycle tied to **ground covered** (`cyclesPerTile`), not
  wall-clock. Walking = half a stride cycle per tile (one footstep) → 80ms/frame, matching CUO.
  Running takes *bigger strides* (`0.32` cycle/tile) → ~62ms/frame: faster than walk so it doesn't
  skate (CUO's fixed 80ms gives ~2.5 run frames/tile = skating), but not the 40ms full-coupling
  that looked too fast. `animPhase` is a 0..1 cycle fraction; tune `cyclesPerTile` to taste.
- [x] **Pathfinder.CanWalk** (`scene::can_walk` + JS `canWalk`): a diagonal step forbids
  corner-cutting (both flanking cardinals must be open) and, if blocked, **redirects to the
  first open flanking cardinal → slides along the wall**; a blocked cardinal just fails. The
  resolved direction drives both server pacing and browser prediction (turn-then-move), and a
  fully-blocked step stands (`pred.moving=false`) so running into a wall shows no run animation.
- [x] **walkable_z = ServUO MovementImpl** rule: a standable surface is `Surface && !Impassable`;
  head clearance (`IsOk`) treats anything `Impassable || Surface` as occupying the body span — so
  a **table** (Impassable+Surface) blocks the tile even though it's a surface. `item_blocks` (dynamic
  world items) likewise blocks any impassable item.
- [x] **continuous walk/run smoothness** (no stutter / no stop-jump). Three fixes, each
  found by measurement/simulation:
  - *Server cadence accuracy* — fixed-timestep accumulator + short `observe` so run paces a
    true **median 200ms** (was ~256ms, which the 200ms client prediction outran → stall+snap).
  - *Stop overshoot* — the browser sends an explicit **`stop`** on key release; the play loop
    clears `desired` at once instead of pacing ~2 extra tiles past the release (which made the
    prediction snap forward). Residual ≤1 tile is **eased** (processSteps) not snapped.
  - *Mid-run micro-stutter* — the prediction **enqueue fills the buffer (QBUF=2) ungated**;
    `processSteps` is the sole pacer (move=cadence, turn=TURN_DELAY). Previously enqueue was
    gated at the same cadence as consumption, so the queue sat at depth 1 and periodically
    drained to 0 for a frame (the "멈칫"). Validated with a headless sim (`scratchpad/predsim2.js`):
    queue-empty-while-held=0, jumps=0, backward=0 across poll 100–250ms and server 200–215ms.
- [ ] (later) faithful CalculateNewZ/StepHeight climb (currently MAX_STEP=16; ServUO StepHeight=2).
