# ClassicUO compatibility gap inventory

Working inventory for bringing `anima-client` toward ClassicUO feature coverage.
A feature counts as complete only when its packet/state, native driver contract,
scene/agent exposure, and user-facing behavior (where applicable) all exist.

Closed since the audit (2026-08-02/03): the five Tier 1 rows marked **CLOSED**
below (speech modes, stat locks, auto-walk speed tiers, shard list + relay
address, login/character rejection reasons) and three of Tier 4's four —
terrain perception (the row the audit called out as cutting against the
project's own thesis), cliloc-resolved journal text, the 0xCC affix bug, and
the gump layout grammar — plus both Tier 2 rows (status sheet, buff names).
Tier 4 is now closed out entirely.

Closed 2026-08-06 (the combat batch): three more Tier 1 rows — armed weapon
ability state, pre-AOS stun/disarm, targeted bandage use — and the stale
worn-item divergence recorded at the bottom of this file. Contract schema
v18 → v19. That batch also found a real disagreement between the two reference
implementations (ClassicUO swaps the stun and disarm subcommands relative to
ServUO and Razor); see the Tier 1 row for which side we took and why.

Closed 2026-08-07 (the party batch): both remaining party rows — loot flag /
private message / leader kick, and non-self status request. Contract schema
v19 → v20. Live-verified end to end by running **two `play` instances** against
the shard (a GM leader on 8788, an ordinary account on 8789) and actually
forming a party, which is the only way to test any of it. That pass also found
the second row's stated premise wrong in two ways — see the row.

Closed 2026-08-07 (the menus batch): the last "S each" Tier 1 row — guild/quest
menus, help+GM page, rename, quest-arrow click. Contract schema v20 → v21.
`build_house_design` was renamed `build_encoded_command`: the guild and quest
gump requests want byte-identical framing, so the house designer is the
biggest user of that shape, not the only one.

Closed 2026-08-07 (the chat batch): the chat-channels Tier 1 row. Contract
schema v21 → v22. Two quirks worth knowing before touching it again, both
faithful ports rather than defects:

- **`chat.current_channel` is "the channel the server last named".** The
  advertisement burst after `ChatOpen` is a run of create-conference commands
  (0x03E8) and each overwrites it, so before any join it holds whichever
  channel came last — measured as `Looking For Group` against ServUO. A real
  join (0x03F1) sets it too, so it becomes truthful from then on. ClassicUO's
  `ChatManager.CurrentChannelName` does exactly the same.
- **A chat sender arrives as `<serial>Name`** — ServUO builds it that way in
  `ChatUser.Username` — and the text keeps a leading space where the colour tag
  `{...}` was stripped. ClassicUO prints both warts verbatim. `scene.chat`
  keeps the raw values (the serial is how a consumer links a line to a mobile)
  and only the journal tidies them, which is the one deliberate departure here.

Closed 2026-08-07 (map pins): the map-pin-editing Tier 1 row. Contract schema
v22 → v23. Three things the wire does not tell you, all measured live:

- **A pin edit is never acknowledged**, so the client must apply it locally —
  and therefore must apply the server's *gate* locally too. The first cut
  applied unconditionally and invented pins: against a decoded treasure map
  still in view mode the window showed `[[169,184],[50,50]]` where the server
  held only `[[169,184]]`. `Session::apply_map_edit` now mirrors the
  `m_Editable` half of `ValidateEdit`. The rest of that check (in reach, not
  protected, not someone else's) is invisible from the client, so a refusal for
  one of those still desyncs — self-healing on the next display, because
  `DisplayTo` re-sends the whole pin list.
- **`RemovePin` refuses index 0 but `ClearPins` does not.** "Clear" is not
  "remove each in turn"; the guard that protects a treasure map's chest pin
  exists on only one of the two.
- **…and yet a treasure map cannot be left pinless.** `TreasureMap.DisplayTo`
  ends with `if (Pins.Count == 0) AddWorldPin(ChestLocation)`, so the chest pin
  regrows on the next open. Measured: `[]` immediately after the clear,
  `[[169,184]]` after reopening. An ordinary `MapItem` stays empty. This one
  refuted a conclusion drawn from the first read of the live output — the clear
  *had* worked, and the reopen put the pin back.

Closed 2026-08-08 (book authoring): the book-authoring Tier 1 row. Contract
schema v23 → v24. Two things to carry forward:

- **This row was *overstated*, not understated.** It claimed "0x93/0xD4 header
  edit and page write builders exist as round-trip stubs"; only
  `build_book_page_request` existed. Rename and chat were the opposite error
  ("absent" for code that was written but uncalled), so the audit's two failure
  modes both show up in this file — check the tree, not the row.
- **0x66 means two different things by shape alone**, and the *request* form is
  inert against ServUO: it registers 0x66 to `ContentChange` only, where the
  request's `0xFFFF` line count fails `lineCount <= 8` and the packet is
  dropped. Nothing is lost, because `BaseBook.OnDoubleClick` sends the header
  *and* every page unprompted — but `Action::BookRequest` does nothing on this
  server, which is now said on the builder.

Closed 2026-08-08 (boat helm): the boat-helm Tier 1 row. Contract schema
v24 → v25. Two findings worth keeping:

- **0xBF/0x33 is not in ServUO's core handler table.** It is registered by the
  High Seas *scripts*
  (`Scripts/Services/Expansions/High Seas/Network/PacketHandlers.cs`), which is
  why grepping `Server/Network/PacketHandlers.cs` says the subcommand does not
  exist. It works on this T2A shard anyway — the script registers
  unconditionally — so this is one expansion-flavoured feature the shard *can*
  prove, unlike the three listed below.
- **The other boat-control path is tiller-man speech**, which dispatches on
  `e.Keywords`. That is now implemented (`speech.mul` + encoded 0xAD); mouse
  piloting remains the held-key path. See the closed Tier 5 speech row.

Closed 2026-08-08 (bulletin boards): the **last** Tier 1 row. Contract schema
v25 → v26. Tier 1 is now closed out entirely. Three findings:

- **ClassicUO corrupts any non-ASCII bulletin subject, and we no longer copy
  it.** `Send_BulletinBoardPostMessage` writes .NET's char count as the length
  prefix ahead of UTF-8 bytes; ServUO reads it with
  `ReadUTF8StringSafe(byteCount)`. They agree only for ASCII. Measured live
  before the fix: `항해일지` (4 characters, 12 bytes, declared as 5) came back
  from the board as `항` plus half a character, and every field after the
  subject shifted. Our builder already used the byte count for each *line*, so
  following ClassicUO made one field disagree with its own neighbours. Now the
  byte count throughout, with a test that walks ASCII/Latin-1/Hangul/mixed.
- **A board's messages arrive as container contents, not as summaries.**
  `OnDoubleClick` sends `BBDisplayBoard` then a `ContainerContent`; each
  message's header must be requested individually (sub 4). Miss that and the
  board is silently empty.
- **The header request has to be time-throttled, not asked-once.** Re-opening a
  board resends sub 0, which our decoder treats as a fresh board and clears the
  summaries — so a once-per-serial guard deadlocked into a permanently empty
  list. It also surfaced that the summary decode appended blindly, showing a
  message twice when two consumers asked; it upserts by serial now.

Closed 2026-08-08 (container fidelity): the first Tier 2 row — authentic
container layout. Contract unchanged (no new actions; `contItems` already
carried the positions).

Two things it surfaced rather than caused:

- **The drop clamp was written for the grid.** Dropping into a container
  measured the whole window, subtracted a hardcoded 20px title bar and clamped
  to a flat 150x120. Harmless while the body was a uniform grid; wrong the
  moment the art defines the coordinate space. It measures the body and its
  real size now.
- **"Return to backpack" sent (0, 0)**, piling anything put back that way into
  the bag's top-left corner — invisible in a grid, obvious once the window drew
  the real bag. ~~Left alone because there is no wire value that asks the
  server to choose.~~ **That was wrong, and the grid-loot batch found the
  value:** `Item.DropToItem` routes `x == -1 && y == -1` — 0xFFFF on the wire,
  read as `Int16` — to `OnDroppedOnto` → `Container.DropItem`, which is exactly
  the randomiser inside the bag's bounds. ServUO uses the same sentinel itself
  (`DropToItem(from, pack, new Point3D(-1, -1, 0))`) and ClassicUO's
  `GrabItem` sends it. Fixed; measured live, a return now lands at a
  server-chosen (98, 66) instead of (0, 0). No bounds table needed after all.

Tier 2 recap at this point (historical — every item named here closed in the
notes that follow): the 0x11 `type >= 6` combat tail, journal
filters/tabs/timestamps, grid loot, the info bar, the counter bar and friends,
and window management.

Closed 2026-08-08 (journal): the second Tier 2 row. No contract change — `type`,
`hue` and `serial` were all already on the wire and in `scene.journal`. Two
things worth keeping:

- **Filtering the journal by message type alone is wrong**, and looks right
  until you try it. ServUO sends "Welcome, …" and "The page queue is empty." as
  MessageType **Regular** with serial `0xFFFFFFFF` and the name "System" — so a
  type-only filter files them under Speech and leaves the System tab empty,
  which is exactly what the first cut did. ClassicUO does not filter on the
  type either: it derives a separate `TextType`, SYSTEM when
  `type == System || serial == 0xFFFFFFFF || serial == 0 || (name == "system"
  && no entity)` and OBJECT only when a real speaker exists. That is what is
  ported.
- **A signature-gated render and a lazily-fetched hue table do not mix.**
  `hueHex` fetches one hue at a time and returns null until it lands, so every
  line was painted with its per-type fallback and never repainted — speech that
  should have been `#9c9c00` stayed white. The hue callback now invalidates the
  journal's signature, the same way it already refreshed the equip tip and the
  dye swatches.

Tier 2 recap at this point (historical — closed below): the 0x11 `type >= 6`
combat tail, grid loot, the info bar, the counter bar and friends, and window
management.

Closed 2026-08-09 (grid loot): the third Tier 2 row. No contract change — the
one-click take is two packets we already had. Two notes:

- **`0xFFFF/0xFFFF` on a drop means "you place it".** Written up under the
  container row above, because it corrects a conclusion recorded there.
- **A refused loot looks identical to a broken one.** The first live click on a
  war hammer left it on the corpse with nothing in the journal; the cause was
  carry weight (99/75 on a STR-10 character), and ServUO simply bounces the
  item back. Raising STR made the same click work. Worth knowing before
  debugging the client: the server refuses silently here too.

Tier 2 recap at this point (historical — closed below): the 0x11 `type >= 6`
combat tail, the info bar, the counter bar and friends, and window management.

Closed 2026-08-09 (window management): the fourth Tier 2 row. Purely client
work — no packet is involved. One trap, hit twice:

- **A hidden panel measures as all zeros.** The static panels are
  `display: none` until their key is pressed, so `getBoundingClientRect()`
  reports (0, 0, 0, 0) and both halves of the geometry code got it wrong in
  turn: clamping the zero-rect was a no-op that left a planted (5000, 4000)
  intact in a 756-wide viewport, and saving from it then stored (0, 0) for a
  panel visibly at (716, 445). Both now go through one `winPos`, which falls
  back to the inline style — the position the element *will* take when shown.

Closed 2026-08-09 (info bar): the fifth Tier 2 row. Client-only; every value
was already in `scene.player`. Worth noting what it *proved* about the row
below it: enumerating ClassicUO's `InfoBarVars` against our own player view
turns the "0x11 `type >= 6` combat tail" row from an abstract parsing gap into
a concrete list of nine readouts a player can see are missing. Those two rows
are the same gap seen from either end.

Closed 2026-08-09 (counter bar): the sixth Tier 2 row, and the first of the
grab-bag row above (the other five closed in the notes that follow). Client-only again — the bar
sends nothing but the `use` behind a double-click. Two things worth recording:

- **The client already knows everything it needs to count.** The obvious worry
  about a carried-item counter is that it reads zero until you have opened
  every bag, since it can only count what the server has told us about.
  Instrumenting the 0x25 handler settles it: ServUO pushes one
  add-to-container per carried item at login, at **every depth** — 16 for the
  worn pack, and a stack of 20 sitting inside a nested bag nobody had ever
  opened. The planned "ask for the pack contents when the bar opens" was
  written, found to be provably dead here, and deleted rather than shipped as
  insurance against a server we cannot point at.
- **Binding a slot must not cost you the item.** Dropping a held item on a cell
  sends it back to the exact `(container, x, y)` it was lifted from — captured
  before the pickup, since the server drops it out of the container the moment
  it accepts one. A *partly* lifted stack goes back through `0xFFFF/0xFFFF`
  instead, because that is `Container.TryDropItem`, which stacks it onto the
  pile it was split from; an exact-coordinate drop is `OnDroppedInto` and would
  leave a second little pile on top of the first. Verified live both ways.

Tier 2 recap at this point (historical — closed below): the 0x11 `type >= 6`
combat tail, and the ignore list / combat book / racial-abilities book /
network stats / inspector remainder of the grab-bag row.

Closed 2026-08-09 (ignore list): the second entry of the grab-bag row (four
left). One scene field, no packet, and one ClassicUO bug found by trying to
match it:

- **ClassicUO's ignore list half-works on a titled name.** 0x98 UpdateName
  carries the title with the name — this shard answers a single-click on a
  young player with `"Anima\t (Young)"`, and an NPC with `"Carl\tthe tailor"` —
  while the 30-byte name field on a speech packet carries the bare `"Anima"`.
  `IgnoreManager.AddIgnoredTarget` stores the former (`m.Name`), and then the
  two filter sites disagree about what to compare it against:
  `MessageManager` tests the *entity's* name and matches, `JournalGump` tests
  the *packet's* name (`entry.Name`) and does not. So ClassicUO silences a
  titled player's floating text while still printing their journal lines, and a
  player who ages out of Young leaves the list silently, since the stored
  string no longer describes anyone. Both strings were read off this client's
  own scene, live. Keying on the part before the tab makes the two sites agree
  and survives the title changing.
- **The filter is retroactive here.** ClassicUO drops an ignored line as it
  arrives, so everything said before you hit ignore stays in the log; this
  journal re-renders under the filter, so ignoring someone also clears what
  they already said (and un-ignoring brings it back — verified both ways). A
  deliberate difference: the point of the button is to stop reading them.
- **Spell text is exempt**, ClassicUO's rule, and worth keeping for a reason
  that is not politeness: the mantra tells you what is about to hit you.
  Verified with a real cast — the ignored player's `Uus Jux` (type 10) reaches
  both the journal and the overhead while their ordinary speech reaches
  neither.

Closed 2026-08-10 (combat book): the third entry of the grab-bag row (three
left). The gump itself is a reference window, but building it meant checking the
data behind it, and the data was wrong.

- **ClassicUO's weapon→ability table disagrees with the server on 19 of the 202
  graphics they share.** The server is the half that decides: ServUO's
  `WeaponAbility.SetCurrentAbility` refuses anything that is not the equipped
  weapon's `PrimaryAbility`/`SecondaryAbility` and clears the arm. Extracting
  those two properties from every `Scripts/Items/Equipment/Weapons/*.cs` and
  diffing found eight entries with **the pair in the wrong order** (dagger,
  gargish dagger, viking sword) and eleven naming **a move the weapon does not
  have at all** — club Shadow Strike (really Crushing Blow), sledge and smithy
  hammer Shadow Strike (Paralyzing Blow), tessen and gargish tessen Block (Dual
  Wield), nunchaku Feint (Double Strike), sai Block (Dual Wield), gargish
  gnarled staff Paralyzing Blow (Force of Nature). Each was confirmed by reading
  the class, not just by the extractor. The eleven are buttons the server
  answers by disarming them.
- **It is also missing 21 graphics ServUO knows**, nearly all the second art of
  a `[Flipable]` pair — katana `0x13FE`, kryss `0x1400`, scimitar `0x13B5`.
  ClassicUO papers over these at lookup time by retrying `graphic ± 1` when the
  tiledata AnimID matches; with the real entries present that hack is not needed
  here. The 21 the ClassicUO table has and ServUO does not are kept — another
  shard may define them.
- **Unarmed was offering ids 0 and 1**, on the belief that they meant "server
  picks". They do not: ServUO's `EventSink_SetAbility` reads 0 as *clear the
  arm* and 1 as *Armor Ignore*, which fists do not have. Bare hands now use
  ServUO's `Fists` — Disarm, then Paralyzing Blow. (ClassicUO's own default is
  the same pair in the opposite order.)
- **Cliloc has no text for the last three moves** on this shard's data files —
  Infused Throw, Mystic Arc and Disrobe come back empty and fall back to the
  client's English names. Cliloc also calls ability 8 *Infecting* where ServUO's
  class is `InfectiousStrike`; the book shows the cliloc name, since that is
  what the game itself calls it.

Verified live: the endpoint returns all 32 with real descriptions; a katana
reads Double Strike / Armor Ignore, matching `Katana.cs`; a club reads Crushing
Blow / Dismount, which is the corrected entry and not ClassicUO's Shadow Strike;
bare hands read Disarm / Paralyzing Blow; and the weapon list under Crushing
Blow draws 37 weapons with their tiledata names. What is **not** verifiable here
is arming a move — `SetCurrentAbility` returns early on `!Core.AOS`, so on this
T2A shard no arm ever sticks. That was already true of the ability bar.

Closed 2026-08-10 (racial abilities book): the fourth entry of the grab-bag row
(two left). Contract v27 — `player.race` and `ToggleFlying`.

- **Race has two sources and this shard has only the second.** ClassicUO reads
  it from 0x11's ML tail (`type >= 5`) and, separately, from the body graphic in
  `Mobile.CheckGraphicChange`. The core has parsed the ML byte since the
  extended-status batch; it had simply never left the server, so it now rides in
  the scene and in `Observation`. Live it reads **0** — a T2A shard sends
  `type < 5` and never mentions race — so the body is doing all the work, and
  the window says which of the two answered rather than implying the server told
  us. A ghost body is in neither table, hence the last-known-race stickiness
  ClassicUO gets for free by never clearing the field.
- **One racial ability is a packet.** Every human and elf trait is passive, and
  of the gargoyle's five only Flying is used rather than possessed — which is
  why `RacialAbilitiesBookGump` hangs a double-click on that icon alone.
  `build_toggle_flying` is ClassicUO's `Send_ToggleGargoyleFlying` byte for
  byte, including the constant `1` and four zeros ServUO's handler never reads.
  ServUO gates it on race and nothing else (`PlayerMobile.ToggleFlying` returns
  unless `Race == Race.Gargoyle`) — there is no expansion check on the handler.
  Sent as a human it is accepted and ignored, which is the useful negative
  test: a wrong length would have desynced the stream instead.
- **The elf and gargoyle pages are rendering, not behaviour.** No gargoyle can
  exist on a pre-SA shard, so those two were checked by forcing the race
  client-side: the counts, names, icons, cliloc text and the single non-passive
  entry all come out right. Flying actually flying is not something this shard
  can show.
- **A cache header bit back.** `/abilities.json` shipped with
  `Cache-Control: max-age=3600`, copied from `/pois.json`. After a rebuild the
  browser kept serving the previous binary's answer to the same URL, so the new
  racial block silently arrived empty. The header is gone: the renderer fetches
  this once per page load anyway, so an hour of cache bought one request and hid
  a data change.

Closed 2026-08-10 (network stats): the fifth entry of the grab-bag row. One
left, the inspector.

- **ClassicUO's ping cannot see a fast link.** `NetStatistics` stores round
  trips in milliseconds and uses **0 as "this slot has not answered"**, so its
  `Ping` getter skips exactly the samples a sub-millisecond link produces.
  Against a shard on the same machine — the normal case here — its network gump
  reads 0 ms forever, and there is no way to tell that from a server that never
  replied. The ring here holds `Option<u32>` microseconds: absent means absent,
  and the live reading is **270–340 µs**.
- **The keepalive was not a measurement.** 0x73 already went out, but every 30
  seconds, purely to stop the server dropping an idle connection. ClassicUO
  pings once a second (`GameScene.Update`) and that is what makes the number
  worth showing, so the interval now matches. Two bytes a second each way.
- **Both hops are shown.** ClassicUO has one link to report; this client has
  browser ⇄ play server ⇄ game server, and the page already timed the first for
  its diag line. Showing only the UO half would misattribute a slow poll to the
  shard.
- **Bytes are counted before decompression**, which is what actually crossed the
  wire — the game phase is Huffman-coded server→client, so the decoded stream is
  much larger than the traffic. Three game-phase writes that went straight to
  the socket (the walk packet, the walk resync, the 0xBD version answer) now go
  through `Session::send`, which is where the counter lives; behaviour is
  unchanged, the counts are no longer short.

Verified live: idle sits at ~25 B/s in and ~2 B/s out (the ping and little
else); a GM `[go` teleport, which makes ServUO resend the whole view, spikes to
**3.25 KB/s in** and settles back within two seconds. Totals and packet counts
climb monotonically across both.

Closed 2026-08-10 (inspector): the last entry of the grab-bag row, and with it
everything in Tier 2 except the 0x11 `type >= 6` combat tail, which this shard
could not show until a later ML-flipped session closed that row too. Client-only — the inspector reads the scene and sends
nothing.

- **The picking is ours, the dump is ClassicUO's.** Its inspector opens from a
  per-object hit test (and from several context menus); ours arms a local mode
  and spends it on the next click, the same shape the ignore list uses. That
  reaches mobiles, world items, container items and land tiles. A ground click
  resolves to a *tile* rather than to one static, so the tile view lists what
  stands on it — four stacked wall/roof pieces on the test tile, at z 20, 40,
  60 and 63.
- **The raw record is the addition worth having.** ClassicUO's dump is a
  hand-written list per object type and silently omits anything nobody thought
  to print. Here the readable table sits above the scene record itself, which is
  the whole of what the server told us.
- **A browser cannot just take the clipboard.** ClassicUO copies a clicked value
  with one SDL call; the async Clipboard API needs a permission and a trusted
  gesture, and `execCommand` needs the gesture too. Three tiers, ending in
  "select the text on the page so ⌘C can have it", which always works. Verified
  both ways: a real click copies, a scripted `.click()` (untrusted) falls
  through the tiers instead of silently doing nothing.
- **Found while testing: two windows opening in the same corner.** The network
  window and the inspector both defaulted to `top: 60px; left: 210px`, so the
  second one to open was invisible under the first — the click that was supposed
  to hit the inspector landed on the network readout. Staggered. The geometry
  memory from the window-management row only helps *after* a window has been
  dragged once.

Closed 2026-08-10 (0x11 `type >= 6`): the last Tier 2 row, and the only one
closed without a live confirmation — because this shard cannot produce the
packet, which is worth being precise about.

- **Measured, not assumed: this ServUO sends `type = 3`.** A one-line probe on
  the 0x11 handler reports type 3 in a 70-byte frame, every time. `type` is
  chosen by `MobileStatus` in `Packets.cs`: 6 needs `Core.ML` *and* a client
  that asked for the extended status, 5 needs `Core.ML`, 4 needs `Core.AOS`,
  else 3. So it is not only the combat tail that never arrives — the whole AOS
  block above it (resistances, luck, damage range, tithing) is absent too, which
  is why those read 0 in the status sheet and always have. The `Core.ML`/
  `Core.AOS` gates are the server's expansion setting, so no client-side version
  bump can unlock them.
- **The layout is still pinned by two independent implementations.** ServUO
  writes `(short)AOS.GetStatus(i)` for `i in 0..=14`, and ClassicUO reads
  fifteen shorts back in exactly that order, field for field — the same
  agreement that settled the stun/disarm subcommands. The golden-byte test
  encodes that order with a distinct value per slot, so a transposition cannot
  pass.
- **Missing values read as 0, and never cost the packet.** ClassicUO guards
  every read (`p.Position + 2 > p.Length ? 0 : …`) and so does this. It is not
  hypothetical: an Enhanced-Client session is sent **29** of these values rather
  than 15, so a mixed shard is exactly how a tail of the wrong length shows up,
  and failing the packet would lose the name and HP with it.
- **What is still ambiguous, in both clients.** The wire has no "absent"
  marker, so a shard that never sends the block and a character with no bonuses
  are indistinguishable — everything reads 0 either way. The info bar's picker
  says so rather than letting a row of zeroes imply a measurement.

Confirmed 2026-08-10, after the fact, by flipping `Config/Expansion.cfg` to ML
for one session (backed up, restored, and the shard restarted on T2A
afterwards — its world, including the fixtures earlier batches left in
FoundryGM's pack, came through both restarts intact). What that session showed:

- **`type = 6`, frame length 121** on the probe — precisely ServUO's
  `EnsureCapacity(isEnhancedClient ? 151 : 121)` for a classic client.
- **`defenseChanceMax` = 45**, which is `45 + BaseArmor.GetRefinedDefenseChance`
  with no refinement — and slot 6 is the *only* one of the fifteen whose
  expected value is 45. That single number confirms the field alignment far
  better than the run of zeros a bonus-less character produces elsewhere.
- **The five resistance caps = 100**, which is `Mobile.GetMaxResistance`'s
  non-player branch; the 70 a plain player gets does not apply to a staff
  mobile. The status sheet duly read "0 / 100" per resist and the info bar's
  nine new fields rendered.
- **Two other rows got their missing confirmation for free.** The ML block
  arrived as well, so `player.race` came off the wire as **1** instead of being
  inferred, and the racial-abilities book switched its own label from "inferred
  from body 400" to "from the 0x11 race byte" — the path that row had to ship
  untested. `weightMax` likewise came from the server (450) rather than from
  our `7*(str>>1)+40` fallback (390).

That was the one row where the shard's own configuration was the obstacle, and
a reversible half-hour settled it — worth remembering the next time a row reads
"unobservable here".

**Know what the local shard can and cannot prove.** `Config/Expansion.cfg` on
the ServUO at `127.0.0.1:2594` says `CurrentExpansion=T2A`, so `Core.AOS` is
**false** there. That single fact splits this batch in half, and it is worth
checking before writing off a live test as a failed feature:

- It makes the shard the *ideal* rig for the pre-AOS rows. `Fists.cs`'s
  disarm/stun handlers open with `if (Core.AOS) return;`, so on T2A they
  actually run — which is how the ClassicUO subcommand divergence got settled
  empirically rather than by reading three sources and picking a majority.
- It makes 0xBF/0x21 and 0xBF/0x25 **unobservable** there. Both are gated:
  `WeaponAbility.ClearCurrentAbility` sends its packet only
  `if (Core.AOS && m.NetState != null)`, and 0x25's senders (`SpecialMove`,
  `SamuraiSpell`) are AOS+/SE content that cannot be invoked at all. A live
  session will therefore show an arm that never clears no matter how long you
  swing — **that is the shard, not the client**, and it cost a confused
  detour to work out. Those two rows rest on unit tests plus the ServUO
  source; confirming them live needs a shard at AOS or later.
- The same gate has now caught three separate features, so check it *first*
  when something reaches ServUO and nothing happens: `0x11 type >= 6` needs
  `Core.ML` (the shard sends type 3), and the guild menu needs
  `Guild.NewGuildSystem => Core.SE`. In each case the client is fine and the
  expansion is the reason — a live test that produces silence here is not
  evidence of a bug.

## Cross-check against OpenShard's `docs/findings.md` (2026-08-07)

A second opinion from a *server* that speaks the same protocol (see DESIGN.md
§7). Twelve of its claims are checkable against this tree; ten confirmed what
we already do, and the two that did not were both real defects.

**Confirmed, no change needed:** `Equipconv.def` is an override table and a
worn item with no entry draws from its own tiledata `AnimID` (`Look::equip_conv`
already falls back — this is the defect that silently dropped every piece of
plain clothing over there, and we also read the `mobtypes.txt` they note they
do not); the texmap id sits in the two bytes after the land entry's 8-byte flags
and its size is `len == 0x2000 ? 64 : 128`; land is preferred out of the `.uop`;
distance is Chebyshev; `0x8C`/`0xA8` carry the address in opposite orders; a
declared frame length is validated before anything is reserved; `0x22` means
opposite things per direction and our incoming table / outgoing builders
already keep the two apart; sloped land is drawn as a four-corner quad whose
vertices come from the neighbours' z, not as a flat diamond at its own.

**Fixed — land art read a black pixel as a hole.** Land and statics decode the
same 16-bit colour and disagree about zero: a static carries transparency as
`0x0000`, but a land tile's shape is the diamond and nothing else, so a zero
inside it is a genuinely black pixel. ClassicUO makes the split explicit —
`LoadLand` writes `Color16To32(...) | 0xFF_00_00_00` with no test at all, while
`LoadArt` guards every run with `if (val != 0)`. Ours ran both through the
transparent-on-zero path. Measured against the real `artLegacyMUL.uop`:
**361 of 851 land tiles (42.4%) are affected, 25,877 pixels**, three to six on
an ordinary grass tile — small enough to read as dark speckle in the texture
rather than as holes, which is why it survived every screenshot. Now
`argb1555_land` vs `argb1555`, pinned by a test on an all-zero tile.

**Fixed — one bad walk confirm froze movement for the whole session.** The
repair leg is a request/response: a confirm we cannot place sets
`walking_failed` (gating every further step) and sends one 0x22 Resync, and
ServUO's `Resynchronize` answers with **0x20 MobileUpdate** — not a 0x21 deny.
Only `Walker::reset` cleared the gate, and the driver called it solely on a
position jump greater than one tile. But a sequence desync is a disagreement
about *count*, not *place*: the resync answer normally carries the position we
already hold, so the jump is zero tiles, the reset never fired, and
`resend_resync` stayed latched so we never asked again. `Walker::on_player_update`
now performs the repair on any self-0x20, gated on `walking_failed` so an
unrelated 0x20 (the hiding-flag path) cannot drop live pending steps. A full
reset is correct there because `Resynchronize` also does `state.Sequence = 0`.

**Fixed — a tile with no texmap was stretched anyway.** ClassicUO refuses:
`Land.ApplyStretch` bails the moment the texmap entry is empty and sets
`AverageZ = MinZ = z`, so the tile is drawn as a flat diamond however the ground
around it stands. We stretched the tile's own 44x44 art onto the quad instead —
seamless, but smearing the diamond over a steep slope, which is the very thing
texmaps exist to avoid. Measured: **23 of 2724 land graphics have `TexID == 0`
and all 23 are `Wet`**, so the whole footprint is water at a shoreline —
precisely what ClassicUO pre-seeds `IsStretched = TexID == 0 && IsWet` to
refuse. Aligned, which also made `makeStretchedTile`'s land-art UV branch dead
code and removed it: the quad now has exactly one texture source.

**Do not oversell this one.** Sampling the live shard at open ocean
(1250,3780), coast (1420,1720) and inland (1500,1560), the tiles whose drawing
actually changes number **zero** in all three: water without a texmap turns out
to be uniformly flat, so it already took the `!sloped` path. The value here is
parity plus a deleted branch, not a visible fix — worth having if a facet or a
custom map ever does slope water, and worth writing down so nobody re-measures
it hoping for a screenshot difference.

**Checked and does not apply: the half-texel UV inset.** ClassicUO's
`CalculateHalfPixelUVs` exists because its terrain lives in a texture atlas, so
a vertex at a region's edge samples the first texel of whatever was packed next
door. Every texmap here is loaded as its own standalone texture
(`PIXI.Assets.load` per URL; there is no `Spritesheet` in the tree), so there is
no neighbour to bleed in. Recorded because the symptom — a one-texel fringe of
foreign terrain along two edges of every stretched tile — would be baffling
without the atlas half of the explanation.

## Audit baseline

- Audited: **2026-07-30** (supersedes the 2026-07-22 pass; 60 commits landed between
  them, including the whole custom-house **editing** vertical the old file listed as
  missing)
- ClassicUO source: local upstream checkout, `src/ClassicUO.Client` + `src/ClassicUO.Assets`
- anima source: whole workspace — `crates/anima-{core,assets,net,agent,wasm,desktop}`
  and `web/{main.js,dialogs.js,index.html}`
- Method: 14 subsystem sweeps (packets in/out, gumps, managers, rendering, assets,
  input/macros, audio, config, world/map/houses, text/journal/chat,
  combat/targeting/spells, login/character, network robustness, plus a
  "what did we miss" critic), each followed by an adversarial pass whose job was to
  *refute* the sweep.

### Reliability note — read this before trusting a row

The first adversarial pass **rubber-stamped**: it confirmed essentially every claim.
An independent re-test of 31 claims then found **12 outright wrong and 11
overstated** (~39%). The false positives clustered in one place: surveyors read the
Rust crates and never properly read `web/main.js`, which is where most player-facing
UI actually lives. All remaining claims were then re-run through skeptics primed with
that failure rate; that pass refuted 0 of 112 and downgraded 10, which is the reason
the list below is worth acting on.

**Wrongly reported as missing — these are implemented, do not re-do them:**
user-definable macros + hotkeys (`web/index.html:1092-1120`, `web/main.js:9041-9210`,
localStorage `anima.macros`); health bars over other mobiles and party
(`web/main.js:5028-5140`); overhead name plates (`web/main.js:5087-5136`); world-map
user markers (`web/main.js:4261-4264`, `:4475-4487`); spell definitions for all eight
schools plus casting from the spellbook (`web/main.js:5422-5574`); the weapon
special-ability bar (`web/main.js:5306-5416`); lightning/moving/fixed graphic effects
(`crates/anima-core/src/net/game.rs:899-925`); animated statics
(`crates/anima-assets/src/animdata.rs` + `crates/anima-net/src/scene.rs:839-857`);
distance-based sound falloff (`web/main.js:333-364`); the music playlist config
(`crates/anima-net/src/play_server.rs:2387-2441`); per-sound volume
(`web/main.js:231-232`, `:376-381`). "Looping ambient sound" is not a ClassicUO
feature at all (zero hits across its tree).

Rows marked **[verified]** were re-checked by hand against the code, not just by an
agent.

---

## Tier 0 — defects in shipped behavior

These are not missing features; they are things that are wrong today.

### T0.1 Facets 1–5 load no terrain at all — **CLOSED** (`d4cd6d0`)

`crates/anima-assets/src/uop.rs:286` hardcodes the UOP virtual path to
`build/map0legacymul/{index:08}.dat` regardless of which facet the reader was opened
for. `MapData::open_facet` (`crates/anima-assets/src/map.rs:198`) correctly opens
`map{facet}LegacyMUL.uop`, so there is no error — and `load_land_block`
(`map.rs:232-243`) silently fills 64 cells with `(0, 0)` on a miss.

Measured against the real data files:

```
facet 0 (Felucca)  : terrain present 4/4 samples   g=0x0006 z=10 …
facet 1 (Trammel)  : terrain present 0/4 samples   g=0x0000 z=0
facet 2..5         : 0/4
```

The moment the server moves the player to Trammel (a normal ServUO destination),
Ilshenar, Malas, Tokuno or Ter Mur, the ground vanishes and every land Z reads 0, so
`CalculateNewZ`, walkability and step prediction all desync from the server.
ClassicUO uses the per-index pattern (`src/ClassicUO.Assets/MapLoader.cs:149`).
Closed by threading the facet into `by_map_chunk` and making `open_facet` refuse
a container whose chunk 0 is unreachable — a silent void is indistinguishable from
real flat terrain. All six facets verified against the real data files.

### T0.2 An unrecognized opcode kills the session — **CLOSED** (`d4cd6d0`)

`crates/anima-core/src/net/lengths.rs` is a sparse 198-entry table; anything absent
returns `PacketLength::Unknown` → `FramingError::UnknownPacket` →
`DriverError::Framing` at `crates/anima-net/src/lib.rs:1095`, which ends the session.
ClassicUO's `Network/PacketsTable.cs` is a complete 256-slot array (`-1` = read the
u16 length), so it can frame and ignore anything.

Diffing the two tables, **44 opcodes ClassicUO can frame and we cannot**:

```
0x3D 0x4A 0x4B 0x4C 0x4D 0x50 0x52 0x59 0x5A 0x5C 0x5E 0x5F 0x60 0x61 0x62 0x63
0x64 0x67 0x68 0x69 0x6A 0x6B 0x79 0x7A 0x7E 0x7F 0x84 0x87 0x8A 0x8D 0x8E 0x8F
0x92 0x94 0x96 0x9C 0x9D 0xAC 0xB3 0xB4 0xC5 0xCD 0xCE 0xD5
```

Closed: the table now covers all 256 ids. Two values deliberately depart from
ClassicUO — 0xD5 stays variable (its fixed 9 comes from the CV_7010400 / 7.0.104.0
branch, above the 7.0.102.3 we advertise) and 0xCF keeps ClassicUO's 78 (ServUO's
registration for it is client→server, the opposite of the direction this table
frames). An unknown id stays fatal by design, since a packet whose length you do
not know cannot be skipped.

### T0.3 The packaged desktop app forgets every preference — **CLOSED** (`d4cd6d0`)

`crates/anima-desktop/src/main.rs:145` binds the play server with `http_port: 0`
(OS-assigned) and points the webview at `http://127.0.0.1:<random>/`. `localStorage`
is keyed by origin *including the port*, so every launch of the shipped app gets a
brand-new, empty store: settings, world-map markers, POI filters, macros, and all
remembered panel positions are lost each time. The `play` dev binary defaults to
8090, which is why this never showed up in development.

Closed by serving from the first free port in `8190..=8199` and remembering the
port actually bound, which keeps the original reason for an ephemeral port (never
collide with the `play` bin, or with a second copy) while giving a stable origin.

### T0.4 Dyed items render in their default colour — **CLOSED**

A ground item's scene entry (`crates/anima-net/src/scene.rs:2589`) carries
`x,y,z,g,serial,pz` and **no hue** — `Item::hue` is only used for corpses
(`scene.rs:2625`). The browser then requests art with no hue query in all three
places items appear: world (`web/main.js:2650`), container/backpack grids
(`:6922`) and the vendor window (`:7755`, `:7770`). Only worn equipment on the
paperdoll is hued (`:6830`).

Closed, and PartialHue-correct: `tiledata`'s `TileFlag.PartialHue` (0x40000) is
folded into bit `0x8000` of the shipped hue — ClassicUO's own encoding, which
`anima_assets::apply_hue` already read — so only the gray pixels are recoloured on
the 533 graphics that ask for it. Applied to every item surface including worn
equipment, which the first pass had deferred (a full hue on a partial-hue item
looks worse than no hue at all, and the doll and the backpack would have shown the
same item in two different colours).

### T0.5 Animated dynamic items never animate — **CLOSED**

`anim_suffix` (`crates/anima-net/src/scene.rs:839`) is applied to map statics
(`:3090`) and multi components (`:2370`) but not to the dynamic-item loop
(`:2586`), so server-spawned animated items — spell fields, campfires, some
braziers — render as a single frozen frame.

Closed: the item loop now emits the same animdata frame list statics get, and the
renderer registers those sprites into the existing animation tick. The per-pixel
hit area follows the drawn frame rather than frame 0, so a click still lands where
the flame actually is.

### T0.6 Item tooltips (OPL) never refresh — **CLOSED**

`opl_info` (`crates/anima-core/src/net/game.rs:1769`) records the 0xDC revision and
does nothing with it; `World::opl_revision` is written in four places and compared
nowhere. The only 0xD6 request path is `Action::OplRequest`, driven solely by a
browser hover, and `web/main.js:726/769/6821` gate on `!oplReq.has(serial)` so a
serial is fetched at most once. An item whose properties change keeps its stale
tooltip forever. The identical staleness pattern *is* handled correctly for
custom-house designs (`game.rs:3068-3085`), which is the model this followed.

Closed: a 0xDC naming a revision we do not hold queues a re-request, drained by
the driver (core still never touches a socket), batched by `OPL_REQUEST_BATCH`
next to the builder so both drivers and any future caller share the cap.

### T0.7 62 multi ids are invisible and non-solid — **CLOSED**

`crates/anima-assets/src/multis.rs` reads only `multi.idx`/`multi.mul`;
`MultiCollection.uop` is unread. 62 ids exist only in the UOP — `0x50-0x53`,
`0x147C-0x14A1`, `0x177C`, `0x1DF4-0x1DFB`, `0x2120-0x212A` — which covers ServUO's
pre-built castles. For those, `placement_json` (`scene.rs:2166`) bails and the
walkability fold gets nothing, so both the placement preview and collision are
silently absent.

Closed by reading `MultiCollection.uop` and merging it under `multi.mul`. The
audit missed the more important half, which the review surfaced: a component's
**visible** set is not its **solid** set. ServUO maps UOP flag `0x101` to
`TileFlag.Generic` and keeps it in the collision grid while ClassicUO does not
draw it — a nodraw-but-solid component, and both references are right. Walkability
now folds on a separate `server_keeps`, and because our ServUO reads the UOP
(`Config/DataPath.cfg` → the same directory this reader opens) while our merge
takes the MUL for shared ids, the UOP's keep answer is overlaid onto MUL
components. That corrects 907 components across 70 multis — 281 of them on
ordinary shared ids — which the client had called open where the server blocks.

---

## Tier 1 — capabilities the player/brain cannot express

The receive side is generally complete; there is no way to *act*.

| Gap | Where it stands | Effort |
|---|---|---|
| ~~**Speech modes** (whisper / yell / emote / guild / alliance)~~ — **CLOSED** | `Action::Say` now carries a `SpeechMode` (`agent.rs`), the driver writes its `MessageType` byte, `play_server` gained `whisper:`/`yell:`/`emote:`/`guild:`/`alliance:` commands, and the chat bar routes `/w`, `/y`, `/e`, `/g`, `/a` prefixes. JSON `Say.mode` is optional, so pre-mode brains are unaffected | S |
| ~~**Stat locks** (0xBF/0x1A)~~ — **CLOSED** | `build_stat_lock` + `Action::StatLock` + `statlock:<stat>:<lock>`, with the same optimistic local update the skill-lock twin does | S |
| ~~**Armed weapon ability state** (0xBF/0x21 clear, 0xBF/0x25 toggle)~~ — **CLOSED** (unit-tested; not live-verifiable on a T2A shard — see the note under "Audit baseline") | Both directions now land. `World::armed_ability` is written optimistically by the driver on 0xD7 (an arm is *never* acknowledged — the only message about it is the one revoking it) and cleared by 0xBF/0x21; 0xBF/0x25 fills `World::active_spell_icons`, which despite the packet's name carries **spell** ids (ServUO sends `moveID + 1` / `spellID + 1`), so it lights up the spellbook, not the bar. Both reach `Observation` and `scene.json`. The renderer keeps a 500ms optimistic window before believing the server, because `scene.json` is polled every 150ms and the snapshot answering a click predates it (the D11 hazard); outside that window the server simply wins, since a swing can resolve before its own echo arrives | S |
| ~~Pre-AOS stun / disarm (0xBF/0x09, 0x0A)~~ — **CLOSED, live-verified** | `build_disarm_request`/`build_stun_request` + `Action::DisarmRequest`/`StunRequest` + `disarm`/`stun` commands. **Watch the subcommand numbering:** ClassicUO has the two swapped — its `Send_StunRequest` writes 0x09 and `Send_DisarmRequest` writes 0x0A, while ServUO registers `RegisterExtended(0x09, DisarmRequest)`/`(0x0A, StunRequest)` and Razor (`Razor/Network/Packets.cs`) writes the same pairing as ServUO. **Settled empirically, not by majority vote:** the two handlers gate on *different* skills, so skills alone identify which one a subcommand reached. With hands free, ArmsLore+Wrestling 100 / Anatomy 0 → `disarm` answered "You get yourself ready to disarm your opponent" and `stun` answered "You are not skilled enough to stun your opponent"; inverting to ArmsLore 0 / Anatomy 100 inverted both replies exactly. ClassicUO's numbering would have swapped them. A byte-exact test pins it so the divergence can't be "corrected" back into a silent wrong-move bug | S |
| ~~Targeted use (0xBF/0x2C) — bandage self/target in one packet~~ — **CLOSED, live-verified** | `build_bandage_target` + `Action::BandageTarget { bandage, target }` + `bandage:<serial>[:<target>]`. `target` 0 = self (the `PartyAccept` sentinel convention), which is the case worth the shortcut. Skips the double-click → 0x6C cursor → reply round-trip, which is what makes reliable self-healing under pressure possible. Live: `bandage:<serial>` with no target healed the player, with `scene.target.active == 0` throughout — **no cursor is ever raised**. Note ServUO still emits cliloc 500948 "Who will you use the bandages on?" from inside the 0x2C handler before invoking the target itself (`Bandage.cs BandageTargetRequest`), so that line in the journal is not evidence of a cursor. ServUO's siblings 0x2D/0x2E/0x30 closed in the next row | S |
| ~~Targeted spell / skill / harvest (0xBF/0x2D, 0x2E, 0x30)~~ — **CLOSED** | Same one-packet shortcut as bandage. `build_targeted_spell`/`build_targeted_skill`/`build_target_by_resource` + `Action::TargetedSpell`/`TargetedSkill`/`TargetByResource` + `tspell:<id>:<target>` / `tskill:<id>:<target>` / `tharvest:<tool>:<resource>`. Spell ids are 1-based (ServUO subtracts 1, matching `cast:`); skill ids are 0-based (no subtract, matching `useskill:`); resource is ServUO's 0 ore / 1 sand / 2 wood / 3 grave / 4 mushrooms. `target` 0 = self. Contract schema v28 → v29 | S |
| ~~**Auto-walk always runs at unmounted-walk speed**~~ — **CLOSED** | new `movement::walk_pacing(world, want_run)` returns `(run, ms)` from live state — ClassicUO `PlayerMobile.Walk`'s rules, including `SpeedMode >= CantRun`, spent stamina (ghosts exempt), and `FastUnmount` taking the mounted tier without a mount. Both auto-walkers (`Route::step_delay`, `play_server`'s loop) consult it per tick and now run | S |
| ~~**Shard list**~~ — **CLOSED** | `parse_server_list` (0xA8) + `LoginMachine::servers()`; `cfg.server_index` names the shard's *own* index and an unlisted one fails with the shard names instead of hanging. 0x8C now yields a `GameServerAddress`, which the native driver dials first (5s timeout) before falling back to the login endpoint — ClassicUO's `IgnoreRelayIp`/`ip == 0` case, available as `Endpoint::ignore_relay_ip`. **Watch the byte order:** 0xA8's address is reversed, 0x8C's is not | M |
| ~~**Login/character rejection reasons**~~ — **CLOSED** | 0x82 gained `account_denied_text`, 0x53 became `LoginError::CharacterLoginRejected` with `character_login_rejected_text`, and 0xFD's queue window is stored and quoted by 0x53 codes 13/14. `LoginError` now implements `Display`, so the browser login page and CLI show the server's stated reason instead of a Debug dump | S |
| ~~Book authoring (title/author + page text)~~ — **CLOSED, live-verified** | **The row was overstated**: no write builder existed at all, only the 0x66 page *request*. Added `build_book_header_change` (0xD4) and `build_book_page_write` (0x66) + `BookHeaderChange`/`BookPageWrite` + `bookhdr`/`bookpage`, and the reader now turns into an editor on a writable book (title/author inputs, a per-page textarea, Save, and a page turn that saves first). Both writes are **silently all-or-nothing** server-side — a title over 60 UTF-8 bytes, a ninth line, or a line reaching 80 characters makes ServUO discard the whole packet, so the builders clamp. Verified live: title/author and two pages persisted and read back exactly, and the three clamps were accepted at 60 / 8 / 79 where unclamped input would have lost everything | M |
| ~~Map pin editing (0x56)~~ — **CLOSED, live-verified** | `MapToggleEditable`/`MapAddPin`/`MapInsertPin`/`MapChangePin`/`MapRemovePin`/`MapClearPins` + `mapedit`/`mappin`/`mappinins`/`mappinmv`/`mappindel`/`mappinclr`, and the map window gained an Edit toggle, a Clear button, click-to-add and click-a-pin-to-remove. **`MapToggleEditable` must come first** — ServUO gates every mutator on `ValidateEdit` = `m_Editable && Validate(from)` and drops the rest silently, the same shape of trap as `ChatOpen`. Note `0x56` is bidirectional with *different* command meanings per direction (client `5` = ClearPins, server `5` = display), the `0x22` trap from OpenShard's findings | S |
| ~~Boat helm control (0xBF/0x33)~~ — **CLOSED, live-verified** | `build_boat_move_request` + `BoatMove { dir, run }`/`BoatStop` + `boat:<dir>[:<0|1>]`/`boatstop`, and the renderer steers the ship from the same held-key intent that would otherwise walk (`pilotingBoat()` branches before the walk predictor, as ClassicUO does). **The serial on the wire is the PLAYER's, not the boat's** — ServUO finds the mobile and reaches the ship through `mob.Mount`. Piloting must be taken first by double-clicking the tiller man; `player.mounted` plus a Mount-layer item of graphic `0x3E96` (`BoatMountItem`) is how the client tells a helm from a horse. Speed is 1 walk / 2 run and **every other value is a stop** (`GetMovementInterval`'s default yields `clientSpeed = 0`, which `StartMove` refuses), so there is no speed 3. Verified live on a SmallBoat in open ocean: sailed, stopped, and measured 10 tiles/3s running against 2 walking | M |
| ~~Bulletin board post/read/reply~~ — **CLOSED, live-verified** | All four builders already existed with no caller (the third row where "absent" meant "written but unreachable"). Added the four actions, `Observation::bulletin_board`/`bulletin_message`, a `bboard` scene object, `bbmsg`/`bbsum`/`bbpost`/`bbdel`, and a board window with a thread list, body pane and a compose box that posts, replies and deletes. **The read flow is not obvious from the packet:** opening a board sends `BBDisplayBoard` *and a container-contents packet* (0x3C) listing the messages as items — the subject/poster/date of each arrives only in answer to a per-message header request (sub 4), so a client that does not ask shows an empty board however many threads it holds. Verified live: post, reply (threaded under its parent), fetch body, delete, and a Korean subject round-tripping | M |
| ~~Chat channels (0xB2/0xB3/0xB5)~~ — **CLOSED, live-verified** | The receive half *and* every outgoing builder already existed with no caller — only the Action/scene/UI layer was missing, which is the second time this audit's "absent" has meant "written but unreachable" (see rename). Added `ChatOpen`/`ChatJoin`/`ChatCreate`/`ChatLeave`/`ChatSay`, a `chat` scene object (status, channel, channels, and a seq-stamped `lines` ring), `Observation::chat`/`chat_messages`, `chatopen`/`chatjoin`/`chatcreate`/`chatleave`/`chatsay` commands, and `/c`, `/cjoin`, `/ccreate`, `/chatopen`, `/cleave` in the chat bar; lines print into the journal. **`ChatOpen` must come first** — ServUO's `ChatAction` drops any action from a sender `ChatUser.GetChatUser` does not know, silently, so a brain that jumps straight to joining sees nothing and gets no error. Verified with two live sessions: register → 4 channels advertised → both join `General` → messages cross in both directions | M |
| ~~Guild / quest menus (0xD7 sub 0x28 / 0x32), help+GM page (0x9B), rename (0x75), quest-arrow click (0xBF/0x07)~~ — **CLOSED** (4 of 5 live-verified) | `Action::GuildMenu`/`QuestMenu`/`HelpRequest`/`Rename`/`QuestArrowClick` + `guildmenu`/`questmenu`/`help`/`rename:<serial>:<name>`/`questarrow[:<0\|1>]`. All five answer with an ordinary gump or journal line, so nothing new was needed to *read* the result — these were purely missing outgoing verbs. **"Absent" was wrong about rename:** `build_rename_request` already existed, CP1252-encoded and correct, with no caller anywhere — only the Action and command were missing. Live: `help` → the "Ultima Online Help Menu" gump; `questmenu` → the "Quest Log" gump; `rename` turned "a horse" into "Bucephalus", cross-confirmed by the Tracking skill's own list showing the new name; `questarrow` discriminated by button — a **left** click left the arrow up and a **right** click cleared it, which is exactly `TrackArrow.OnClick`'s asymmetry, so the boolean byte demonstrably arrives intact. Only `guildmenu` is unverified here: ServUO gates it on `Guild.NewGuildSystem => Core.SE`, above this shard's T2A (see the shard note above) | S each |
| ~~Party: loot flag, private message, leader kick~~ — **CLOSED, live-verified** | `build_party_can_loot` (0xBF/0x06/0x06), `build_party_private_message` (0x06/0x03) and `build_party_remove` (0x06/0x02) + `Action::PartySetCanLoot`/`PartyPrivateMessage`/`PartyKick` + `partyloot:<0\|1>`/`partytell:<member>:<text>`/`partykick:<member>`. **Kick and leave are one packet**, distinguished only by whose serial it names; ServUO's `PartyCommands.OnRemove` gates it on `p.Leader == from \|\| from == target` and ignores a non-leader silently — verified both ways live (a member's kick of the leader changed nothing; the leader's kick of the member disbanded the party on both clients). Loot flag answers cliloc 1005447/1005448 and is **never sent back**, so a UI must remember what it asked for. Private messages are clamped to 128 chars because ServUO *drops* longer ones rather than truncating | S |
| ~~Request another entity's status (0x34 non-self) → party mana/stam bars~~ — **CLOSED, live-verified — and the row's premise was wrong twice** | `Action::StatusRequest { serial }` (0 = self) → 0x34 type 4; `party[].mana/manaMax/stam/stamMax` now leave `build_scene`. Two corrections worth keeping: **(a) it is a resync, not the only source.** ServUO pushes a member's mana/stam changes unprompted (`Party.OnManaChanged`/`OnStamChanged` → 0xA2/0xA3) — but only while they are in update range and visible, so a member you lost sight of freezes at the last value with nothing scheduled to fix it. That is the real gap this fills. **(b) The values are percentages, not points.** Every party-facing vitals packet goes through `AttributeNormalizer` (max written as a fixed 25, current as `cur * 25 / max`). Measured live: a member at a real 7/10 mana reached the leader as 17/25. Never compare another member's mana against a spell cost — our own vitals are un-normalized, theirs are not | S |
| ~~Open door / last weapon / virtue / animate / cast-from-book / all-names~~ — **CLOSED** | ClassicUO `GameActions` verbs that had no Action. `OpenDoor` (0x12/0x58) opens the door on the facing tile — auto-walk and keyboard bump now send this instead of double-clicking a serial, matching `PlayerMobile.TryOpenDoors`. `EquipLastWeapon` (0xD7/0x1E), `InvokeVirtue` (0x12/0xF4, 1-based Honor…Spirituality), `EmoteAction` (0x12/0xC7 bow/salute; command `animate:` so it does not collide with speech `emote:`), `CastSpellFromBook` (0x12/0x27; the spellbook UI uses it when the book's serial is known), `AllNames` (single-click every other mobile and every corpse, cap 60). Contract schema v29 → v30. Macros gained the same verbs; Options gained ContainerScale (50–200%) and authentic corpse gumps blink the 0x45/0x46 eye. Container minimize/iconize is closed on the container-windows row below | S |
| ~~**Pre-OPL equipment info** (0xBF/0x10)~~ — **CLOSED** | ServUO still sends `DisplayEquipmentInfo` on equipment `OnSingleClick` (crafted-by, unidentified, charges, exceptional). ClassicUO paints it as overhead text (`0x3B2`) then requests MegaCliloc. anima-core journals the name cliloc plus crafter/unidentified ASCII and per-attr clilocs (charges as affix), so the play-server resolver and `ingestSpeech` float the same lines. Unknown serials are dropped, matching ClassicUO `Items.Get == null` | S |
| ~~**Heritage / race-change** (0xBF/0x2A)~~ — **CLOSED** | Incoming `HeritagePacket` (`female`, `race` 1/2/3) fills `World::race_change` / `Observation::race_change` / `scene.raceChange`; race 0 or > 3 (ServUO close sentinel `0xFF`) clears it. Confirm is `Action::ChangeRace` (five `u16`s, ClassicUO `Send_ChangeRaceRequest`, ServUO size-15 `HeritageTransform`); cancel is the 5-byte subcommand-only packet (cliloc 1073645). Play commands `racechange:` / `racechangecancel`; the renderer opens a small confirm dialog. Schema v30 → v31 | S |
| ~~**Forced mobile animation** (0xBF/0x2B)~~ — **CLOSED** | ClassicUO matches `(mobile.serial & 0xFFFF)` and `SetAnimation`/`AnimIndex` with `ExecuteAnimation = false`. We resolve the same low-16 serial and queue a 0x6E-style `recent_anims` event so the renderer plays the group; freeze-on-frame is approximated by playing once | S |
| ~~**Open UO Store** (0xFA)~~ — **CLOSED** | ClassicUO `Send_OpenUOStore` is a 1-byte packet. `Action::OpenUOStore` + `uostore`. ServUO registers it in `UltimaStore.UOStoreRequest` | S |

Closed 2026-08-23 (AlwaysRun): ClassicUO `AlwaysRun` / `AlwaysRunUnlessHidden` are now Options (default off / on). Keyboard walk runs without Shift when Always run is set, unless the player is hidden and Walk while hidden is on. Mouse-steer still uses distance-to-run.

Remaining ClassicUO Options / visual QoL (not protocol holes): AutoOpenCorpses, SmoothDoors, NameOverHeadHandler Ctrl+Shift filters, aura under feet, drag-select health bars, anchored gump groups, MacroButtonGump on the hotbar. Circle of Transparency is a deliberate occupancy-fade instead (`06-movement.js`). Screenshot / Credits / LocationGo are client chrome.

---

## Tier 2 — UI for state we already decode

| Gap | Note | Effort |
|---|---|---|
| ~~**Extended status sheet**~~ — **CLOSED** | all of it (armor + the four resistances, weight/max, stats-cap, followers/max, damage range, luck, tithing, and the three stat locks) now leaves `build_scene` and shows in the status panel. It had been parsed into `World` all along and simply never sent | M |
| ~~0x11 `type >= 6` combat tail~~ — **CLOSED, live-verified** (on a temporarily ML-flipped shard) | All fifteen values now parse into `PlayerStats::aos` and reach the scene, `Observation` (schema v28), the status sheet and the info bar — which is where the nine fields the info-bar row had to leave out finally appear. The five resistance **caps** arrive in the same block, so the sheet reads "60 / 70" instead of a bare 60. Confirmed against a real server by flipping the shard to ML for one session: **`type = 6`, 121 bytes**, with `defenseChanceMax` = 45 — a value only slot 6 can produce. Back on T2A it answers **`type = 3`** and every field returns to 0 | M |
| ~~**Buff names**~~ — **CLOSED** | 0xDF's title/description clilocs and their (little-endian) argument blocks are parsed into `Buff`; the renderer resolves the title for the bar and shows the description on hover, and `anima_net::localize` fills `display`/`display_desc` for brains. The 35-entry English table survives as the fallback when a shard sends no title cliloc. The debuff tint is still a regex over the name — UO carries no buff/debuff flag on 0xDF (ClassicUO hardcodes the split by icon id) | M |
| ~~Journal~~ — **CLOSED, live-verified** | All five: lines take the **server hue** through the same `msgColor` the floating overheads already used (per-type colour only as the fallback, ClassicUO's rule) plus per-type styling — a yell bold, a whisper dim, an emote italic; **All/Speech/Guild/System tabs**; an arrival **timestamp**, stamped client-side because UO sends none; and the log is `resize: vertical` with its height remembered. The tab rule is a port of ClassicUO's `TextType` decision, **not** a filter on message type — see the note above for why the obvious version puts every system line under Speech | M |
| ~~Grid loot~~ — **CLOSED, live-verified** | A corpse (identified by its gump id `9`, which `scene.contGumps` already carries) opens the uniform grid rather than the authentic corpse art, with **click-an-item-to-take** and **Loot all** — the split ClassicUO's separate `GridLootGump` exists to make, since a body's scattered layout is the wrong shape for looting. Off in Options for anyone who wants the real corpse gump. One click is ClassicUO's `GameActions.GrabItem`: a lift, then a drop with **no position**. Verified live on an orc corpse | M |
| ~~Info bar~~ — **CLOSED, live-verified** | A draggable bar of user-chosen readouts with a ⚙ field picker, persisted, opened from Options. Fields are ClassicUO's `InfoBarVars` restricted to what `scene.player` actually carries: HP, mana, stamina, weight, followers, gold, damage, armour, luck, the four resistances, stat cap, tithing and notoriety. **The nine it is missing are exactly the `0x11 type >= 6` row below** — LowerReagentCost, SpellDamageInc, FasterCasting/Recovery, Hit/Defense/DamageChanceInc, LowerManaCost, SwingSpeedInc — so the picker says so in place of leaving the absence to read as an oversight. Weight turns red over capacity, which is the one field whose colour carries information: past `weightMax` ServUO refuses a pickup in silence (see the grid-loot row) | M |
| ~~Counter bar~~ — **CLOSED, live-verified** | Cells pinned to an item graphic (optionally to one hue), counting what you carry and using one on double-click. The count is a port of ClassicUO's `GetTotalAmountOfItem` + `Item.GetTotalAmount` — worn items on layers 1..0x17, recursing through nested bags — and its display rules: no number on a lone item, a signed distance from a per-slot *compare to* with `±` on target, a red cell below a *warn below* threshold, and the change flash (green up, red down, fading over 5s). Bind a cell by dragging an item onto it, which puts the item straight back where it came from, as ClassicUO does. Reachable ClassicUO settings that live in its right-click context menu — ignore hue, compare to, remove — sit in a strip under the bar instead, since this client has no context menus | S–M |
| ~~Ignore list~~ — **CLOSED, live-verified** | A set of character names whose talk this client drops, in both places ClassicUO drops it — the floating overhead and the journal — with ClassicUO's Spell exemption, its guards (a mobile, not yourself, not one with a yellow health bar) and its messages. Added by ClassicUO's own client-side pick (arm, then click a player; no packet leaves) or by typing a name, which ClassicUO cannot do. The yellow-bar guard needed `Mobile::yellow_health` — parsed since the health-bar batch, never sent — to reach the renderer as `yellow`. **Keyed on the bare name**, which is where ClassicUO gets it wrong: see the note below | S–M |
| ~~Combat book~~ — **CLOSED, live-verified** | All 32 weapon moves with the client's own icons (gump `0x5200 + id - 1`), their names and descriptions read from **cliloc** at runtime (`1028838 + i` / `1061693 + i`, ClassicUO's ids) through a new `/abilities.json`, and the weapons that grant each — the inverse of the `WEAPON_ABILITIES` table the ability bar already had, so the two halves cannot drift (ClassicUO keeps a second hand-written copy for this gump, a 400-line `GetItemsList` switch). The equipped weapon's two moves head the window, armed state and click-to-arm included. Reconciling that table against ServUO's own weapon classes corrected 19 entries and added 21 — see the note below | S–M |
| ~~Racial-abilities book~~ — **CLOSED, live-verified** | What your race gives you: four entries for a human, six for an elf, five for a gargoyle, with ClassicUO's icons (`0x5DD0`/`0x5DD4`/`0x5DDA` + i) and its names, and the descriptions read from cliloc (`1112198`/`1112202`/`1112208` + i) through the same `/abilities.json` the combat book fetches. Race comes from 0x11's ML byte when the shard sends one and otherwise from the body graphic, ClassicUO's other rule — which is the only one that fires here. All entries are passive except the gargoyle's Flying, now `Action::ToggleFlying` (0xBF/0x32) | S–M |
| ~~Network stats~~ — **CLOSED, live-verified** | Ping, in/out rate, totals and packet counts for the link to the game server, plus this page's own HTTP poll — two hops, because this client has two and a lag reading that blamed the wrong one would be worse than none. The ping is a real 0x73 round trip (ServUO's `PingAck` echoes the sequence byte), averaged over five as ClassicUO does, measured in the driver that owns the socket since the core has no clock by design. Kept in **microseconds** — see the note below | S–M |
| ~~Inspector~~ — **CLOSED, live-verified** | Arm it, click a mobile, an item or the ground, and read back everything this client believes about it: ClassicUO's key/value dump with its field names, sorted by key, click-a-value-to-copy. Plus the **raw scene record** underneath — ClassicUO's list only prints what someone hardcoded, and the question worth asking in this repo is what the server actually sent. A ground click resolves to a tile, so that view also lists the statics standing on it | S–M |
| ~~Container gumps ignore real container art and each item's stored (x,y)~~ — **CLOSED, live-verified** | Both halves were already on the wire and discarded: 0x3C carries a position per item, and 0x24 names the container's gump. The gump id is now **retained** (`World::container_gumps`) rather than read from the open-event ring, which ages out while the window stays open, and reaches the renderer as `scene.contGumps`. The window draws that art and places each item at its raw (x, y), read **signed** as ClassicUO does (`(short)item.X`). ClassicUO additionally clamps into a per-gump bounds table (78 entries); we clip with `overflow: hidden` instead, which needs no table and cannot disagree with the server about where an item actually is. Verified live on a real backpack (gump 60, 230×204) with a dozen items at their stored positions | M |
| ~~Container windows: sized wrong, no type, one-mode~~ — **CLOSED, live-verified** | Follow-up to the row above, all three verified live. (1) The window was a fixed 252px, clipping a wide gump (a 282px game board) and guttering a narrow one; it now sizes to the gump texture (learned from the loaded `<img>`, cached per gump id), like ClassicUO's `ContainerGump`. (2) A late 0x24 (gump id arriving after the item set was stable) never re-rendered because `containerSignature` hashed only items; it now includes the gump id, the container graphic/name, and the view toggles. (3) A new **grid mode** (`settings.gridContainers`, default off = authentic) shows the container's own icon + tiledata name in the title bar so a pouch reads differently from a backpack — which the authentic mode *cannot* distinguish because ServUO gives both gump 0x3C. The name/graphic are resolved server-side into a new `scene.contInfo` (serial→{g,name,hue}) from `world.items` + tiledata, since the browser has neither. **Deliberate divergences:** we keep a slim title bar in both modes (ClassicUO's authentic gump is chromeless, but our DOM window needs a drag/close surface); the general grid mode is our own (ClassicUO has only the corpse `GridLootGump`); still no per-gump bounds clamp. `ContainerScale` (Options 50–200%, chess/backgammon gumps `0x091A`/`0x092E` stay 1×) and the authentic corpse blink-eye (gump `0x45`/`0x46`, 750ms) landed with the GameActions batch. Minimize/iconize landed later — see the closed note after this table. The line this row used to carry — "a server-only 0x24 with no local `openContainer` (a banker's box) still opens no window" — was **wrong**; see the note below | M |
| ~~Window management~~ — **CLOSED, live-verified** | Every draggable panel — the dynamic windows from `makeWindowFrame` *and* the static ones (paperdoll, spellbook, skills, options, macros) — now **remembers where it was left** and comes back inside the viewport, because the persistence lives in `makeDraggable`, the one function they all already went through. Keyed by element id or class, never by serial: a serial is per-corpse/per-bag and never returns, so "the next container opens where I put the last one" is the only memory worth having (ClassicUO's per-type defaults do the same). Windows are clamped on restore and on browser resize, and a position saved on a bigger screen is **written back clamped**, so it heals instead of being re-clamped forever. `resize: both` is opt-in per window (bulletin board, server gumps, plus the journal from the row above) — deliberately not on the map or an authentic container, whose layout is in the server's own pixel space | M |

Closed 2026-08-20 (container minimize/iconize): the leftover on the container-windows
row. ClassicUO's `ContainerGump` collapses a bag to its `IconizedGraphic` on a
left-click of `MinimizerArea` (always 16×16) and restores on double-click of the
collapsed pic (`ContainerGump.cs` `HitBoxOnMouseUp` / `GumpPicContainerOnMouseDoubleClick`).
The data is **not** a `containers.cfg` in the UO install — it is
`ContainerManager.MakeDefault()`, persisted as `Data/Client/containers.txt`. Only
four gumps ship an icon (`0x003C`→`0x0050` at 105,162; `0x775E`/`0x7760`/`0x7762`
at 105,178); `IconizedGraphic == 0` means no pin. Grid loot and `gridContainers`
stay uncollapsed (ClassicUO only does this on authentic `ContainerGump`). Chess/
backgammon have no icon. A press that moved ≥5 px is a window drag, not a
minimize, matching `MIN_PICKUP_DRAG_DISTANCE_PIXELS`; a held cursor item blocks
it (`ItemHold.Enabled`). Minimized state lives on the window object, not in
`containerSignature`, so the poll cannot fight the player.

Closed 2026-08-24 (server-opened containers): the "a banker's box still opens no
window" line on the row above was a **misdiagnosis**, and this is the correction
rather than a fix for what it claimed. The 0x24 path has been whole since the
vendor/container packet pack (2026-07-11): `draw_container` records the open,
`container_opens_json` filters the two overloads out of it, and the browser's
`ingestContainerOpens` (`web/js/08-overlays.js`) calls the same `openContainer`
a double-click does. Driving the real `web/js/*.js` through a fake DOM with a
scene carrying only a server 0x24 — no local open anywhere — builds the window,
titles it, draws gump `0x4A` and fills it, and it survives the next two polls.
What was actually broken in August was one layer earlier: ServUO's banker keys
entirely off `e.Keywords` (`Scripts/Mobiles/NPCs/Banker.cs:323`, a `foreach` over
the keyword array — it never string-matches "bank"), and this client only started
encoding `speech.mul` keywords onto 0xAD three days after that row was written
(9f4773d, 2026-08-17). Until then no keyword reached the banker, `BankBox.Open`
never ran, and **no 0x24 was ever sent** — so the absent window was blamed on the
window layer. Pinned offline now at both testable seams:
`draw_container_retains_the_art_a_server_opened_window_draws_from` (anima-core)
and `a_server_only_container_open_carries_both_halves_the_window_needs`
(anima-net), the second because it takes *two* fields to make that window
visible — `containerOpens` to open it and `contGumps` to give it art — and losing
either reads to a player as "nothing happened".

One real hole did come out of the investigation and is fixed: `refreshContainer`
read a missing `contGumps` entry and an entry of **0** as the same thing, and
parked both in the chromeless "0x24 hasn't landed yet" placeholder — which is
transparent, so invisible. For a server-initiated open there is by definition
nothing left to wait for, so a shard that answers `Container.GumpID` 0 hung an
invisible window forever. Presence now decides the wait (and `containerSignature`
distinguishes the two states, or the window could never re-render out of it); a
named gump of 0 falls through to the grid, the same fallback a missing gump
texture already took. Still open, deliberately: the WASM page (`/?wasm=1`) opens
no server-initiated container at all — `Observation` **excludes**
`recent_container_opens` on purpose ("a window-opening UI signal",
`anima-contract-json/src/lib.rs`), so `14-wasm.js` hardcodes `containerOpens: []`
and `contGumps: {}`. Reversing that is a contract schema decision, not a renderer
one. ClassicUO's own chain, for the record: `PacketHandlers.cs:206`
`Handler.Add(0x24, OpenContainer)` → `OpenContainer` (:1305) → `0xFFFF`
`SpellbookGump` / `0x0030` `ShopGump` / else `new ContainerGump(world, item,
graphic, playsound)` (:1516) — and note ClassicUO is **stricter** than we are
there, dropping the packet outright when `world.Items.Get(serial) == null`
("[OpenContainer]: item not found", :1540).

---

## Tier 3 — rendering fidelity

~~Shadows~~, ~~corpse equipment layers~~ and ~~death animation~~ (**CLOSED,
live-verified** — see below); ~~`light.mul` light shapes and colours~~ and ~~directional lighting on stretched
land~~ (**CLOSED, live-verified** — see below); ~~seasonal
land/static graphic remap~~ (**CLOSED, live-verified** — see below);
~~`TileFlag.Translucent` statics drawn opaque~~ (**CLOSED, live-verified** — see
below); ~~static hue from `statics.mul` discarded
at decode~~ (**CLOSED** — see below); ~~mount rider vertical offset~~ (**CLOSED** —
see below); ~~seated-character deformation~~ (**CLOSED** — see below); ~~roof/ceiling
fading~~ and ~~the pathing half of the ceiling rule~~ (**both CLOSED,
live-verified** — see below);
~~0x23 DragEffect decoded but never drawn~~ (**CLOSED** — see below);
~~GameEffect blend modes and projectile rotation~~ (**CLOSED** — see below);
~~StaticFilters (tree→stumps, hide vegetation)~~
(**CLOSED, live-verified** — see below).
~~HasSurfaceOverhead~~ (**CLOSED** — see below).

Closed 2026-08-22 (`HasSurfaceOverhead`): the leftover on the roof-fading row.
ClassicUO `GameSceneDrawingSorting.HasSurfaceOverhead` (`:511`) sets
`AllowedToDraw = false` on every mobile but the player when a 4×4 of Static/Multi
tiles around them (`dx,dy ∈ -1..=2`) each has a NoShoot or Window piece above the
body, close enough to the current draw ceiling (`_maxZ - tile.Z + 5 >= tile.Z -
obj.Z`). From outside a house (`maxZ = 127`) that hides people deep inside it;
walking in lowers `maxZ` to the eave and the inequality fails, so they reappear.
A vendor in a doorway stays visible because one of the 16 cells has no cover.
Emitted as `"so":1` (omitted when false); the renderer skips the sprite, its
name/HP bar, and the click target, matching `MobileView.Draw`'s `!AllowedToDraw`
early return. The `_maxGroundZ` drawing gate stays the land-overhang `t.h` path
it already was — after `UpdateMaxDrawZ` it is 127 except in a cave, where it
equals `maxZ`, and `obj.Z` is an `sbyte` so `Z <= 127` is a no-op outdoors.

Closed 2026-08-17 (static hue): the `statics.mul` record is 7 bytes —
graphic, in-block x/y, z, **hue** — and the last two were consumed only by
`pos += 7`. `StaticTile` now keeps the dye; the scene emits it PartialHue-encoded
the same way items are (`item_art_hue` on the *drawn* graphic's flags, so a
desolation mushroom→blood remap recolors only gray pixels); the renderer asks
`?hue=` and keys the static pool by dye so two same-graphic panes on one tile
do not collapse. Pathing never sees it. Most statics are undyed, so the field
is omitted when 0.

Closed 2026-08-17 (mount rider offset): `mount_body` already returned
`(body, OffsetY)` and `mount_anim_for` threw the second half away. ClassicUO
draws the mount at the original `drawY`, then `drawY += OffsetY` for the rider
and every worn layer (`MobileView.cs:256`). Wired through as `mountOff`; the
renderer applies it to body and equipment only, never the mount or the shadow
(the shadow is drawn *before* that add). Ethereal horse −9, cu sidhe +18 —
the two signs a rider would sit inside or float above the animal without.

Closed 2026-08-17 (0x23 DragEffect): the packet was parsed and queued
(`World::recent_drag_anims`) with no consumer. It now leaves as `scene.dragAnims`
and the renderer interpolates the item graphic source→dest the way a kind-0
moving effect does. Gold/gem flight remaps were already in the decoder.

Closed 2026-08-17 (GameEffect blend + rotation): 0xC0/0xC7's `renderMode` u32
was left unread. ClassicUO takes it `% 7` as `GraphicEffectBlendMode`. Stored
on `Effect::blend`, shipped, and mapped onto PIXI blend modes (Normal /
Multiply / Screen+ScreenMore / approximations for ScreenLess, half-transparent,
and ShadowBlue). Moving projectiles (kind 0) now rotate with ClassicUO's
`AngleToTarget = atan2(-dY, -dX)` on the iso delta, pivoted at the art center.
Lightning stays additive regardless — it is a gump flash, not this table.
0x70 has no renderMode and stays blend 0.

Closed 2026-08-17 (seated-character deformation): E/S chairs still hold the
Stand frame (people have no dedicated sit art for those facings), but the
renderer now runs ClassicUO `DrawCharacterSitted` — three quads at 0.35 / 0.60
/ 0.94 of the frame, upper band shifted `flip ? -8 : 8`, mid a trapezoid,
lower unshifted. Shadow and mount stay undeformed. N/W keep the ONMOUNT_STAND
group they already used. Any human standing on a chair static/item
(`TryGetSittingInfo`: same tile, `|Z| ≤ 1`, not mounted, not mid-step) sits,
not just the local double-click overlay — so an NPC on an east-facing chair
leans too. Gargoyle bodies (666/667/0x02B6/0x02B7) are skipped; ClassicUO uses
group 42 for those rather than the three-band lean.

Closed 2026-08-17 (`speech.mul` keywords): `SpeechesLoader` records are
big-endian `id`/`length` + UTF-8. Matching ports `IsMatch` (`*`-split,
CheckStart/CheckEnd, word-bounded, case-insensitive). A hit forces Unicode
0xAD with `MessageType.Encoded` (0xC0) and ClassicUO's nibble-packed ids +
UTF-8 text — ServUO's 0x03 path never fills `e.Keywords`, so "vendor buy" and
tiller "forward"/"stop" must take this route even for ASCII. The core stays
file-free: the driver loads `speech.mul` and hands ids to `build_unicode_say`.

Closed 2026-08-17 (skills.mul / art.def / gump.def / TexTerr.def / Anim1.def /
Anim2.def / MUL fallback / hues ramp): `skills.mul` names + HasAction are
served at `/skillinfo.json` and replace the hardcoded skill table. Missing
art/gump/texmap indexes follow the first present `{group}` member and optional
hue. `Anim1.def`/`Anim2.def` remap then `% AnimationCount` (13/35/22) so an
out-of-range animal group cannot walk into the next body's idx block. Art and
gumps open `*.mul`+`*.idx` when the UOP is absent. Hue ramp index is the
pixel's 5-bit red field (`GetColor16`'s `(c >> 10) & 0x1F`; after 5→8
expansion that is `px[0] >> 3`), not `max(r,g,b)` and not `* 31 / 255`.

Closed 2026-08-10 (corpse equipment layers): the first Tier 3 row. A corpse
already drew the dead thing's held death-pose frame; now it wears what it died
in.

- **The data was already in the world, twice over.** 0x89 CorpseEquip has been
  parsed into `World::corpse_equip` all along with no consumer, and the items
  themselves arrive beside it: ServUO's `Corpse.SendInfoTo` sends
  `CorpseContent` *and* `CorpseEquip` unprompted — but only when
  `((Body)Amount).IsHuman`, which is the server half of the same `ishuman` test
  ClassicUO applies before drawing any layer. So a non-human corpse stays bare
  on both sides, without either needing a rule for it.
- **The layers reuse the living-mobile path**: the per-direction
  `LAYER_ORDER_DIR` (ClassicUO's `LayerOrder.UsedLayers`), the `isCovered`
  suppression rules, and `centerFor`'s per-frame draw-centers. Each layer is a
  child of the corpse's body sprite, so it inherits position, depth and the
  boat/transparency passes; only the offset between the two draw-centers is
  computed here. A layer with fewer frames than the death pose is clamped to its
  own last frame, as ClassicUO does.
- **One divergence found by looting.** 0x89 is a one-shot snapshot and nothing
  on the wire retracts it, so the first cut left a corpse still wearing the
  sword you had just taken off it. ClassicUO never faces this: it stores the
  layer on the item and asks `FindItemByLayer`, a search of the corpse's *live*
  contents. The scene now asks the same question — an entry is drawn only while
  the item's container is still the corpse. Verified by taking a helm: five
  layers to four, in the scene and in the sprite tree, within one poll.

Verified live on a killed brigand (body 401, death group 21): six layers
resolved with their AnimIDs and hues, and the rendered corpse shows tunic,
skirt, shoes and the spear lying across it where the same corpse with the layer
pass disabled is a bare body.

Closed 2026-08-10 (death animation): the second Tier 3 row. Mobiles fall over
now instead of blinking into a corpse.

- **0xAF was parsed and half-used.** `display_death` already recorded which
  corpse belongs to which killer; the animation cue itself — the reason the
  packet exists — went nowhere. It is now a bounded `recent_deaths` ring beside
  the other event feeds, carrying the death group the server resolves (the same
  mobtypes-aware call a corpse's held frame uses, which the browser cannot make
  for itself).
- **The dying body has to outlive the mobile.** ServUO deletes the mobile and
  drops the corpse in the same breath as the cue, so there is nothing left to
  animate by the time the poll lands. ClassicUO keeps the entity alive locally
  under `serial | 0x80000000` and holds the corpse's own sprite back
  (`CorpseManager`) until the animation ends; this does the same with a
  `dyingMobs` map and a skip in the item loop.
- **The trap was ordering.** The first cut looked up the dying mobile in
  `scene.mobiles` and its animation state in `anim` — and found neither, because
  the cue arrives in the poll *after* it left the scene and `updateAnimStates`
  had already pruned it. Both now happen while it still exists: the ingest runs
  before the prune, and each mobile's scene record is stashed on its animation
  state as it is touched, so the falling body keeps its hue and equipment.
- **ServUO cannot show the second death pose.** ClassicUO reads 0xAF's third
  field as `running` and passes it to `GetDeathAction` to pick Die2 over Die1;
  ServUO's `DeathAnimation` writes a literal 0, so Die1 is the only reachable
  pose here. Carried through anyway — nothing in the wire format promises that.

Verified live by logging the body sprite's texture every 60ms across a kill:
`17/1/0/0.png` (orc, stand) → the cue lands → `17/2/0/0,1,2,3` (MONSTER_DIE1,
each frame in order) → frame 3 held → the entity is released and the corpse
sprite appears, with the corpse absent from the item pool for exactly that
window. A canvas capture taken at frame ≥ 1 shows the orc mid-fall.

Closed 2026-08-10 (shadows): the third Tier 3 row. Characters cast one.

- **The transform is ClassicUO's, exactly.** `Batcher2D.DrawShadow` builds a
  parallelogram from the sprite: half height, top edge pushed right by that same
  half-height, and the whole thing lifted 10px so it starts at the feet. As a
  matrix that is `[1, 0, -0.5, 0.5]`, which PIXI expresses as a **-45° x-skew
  with `scaleY = cos 45°`** — worth writing down, because "squash and lean" has
  a dozen plausible parameterisations and only one of them matches.
- **The colour is the shader's, exactly.** `IsometricWorld.fx`'s `SHADOW` branch
  is `color.rgb = 0; alpha = 0.4` over the sprite's own alpha mask — a flat
  black silhouette, not a dimmed copy of the character.
- **One per mobile, from the body frame only.** ClassicUO draws it as a separate
  pass with `entity = null` before the layered draw, never per worn layer, so
  overlapping clothes do not stack up into a darker patch. Its gates come along
  too: not while dead, not while hidden, and an Options toggle
  (`ShadowsEnabled`, on by default there and here). A mounted rider's shadow
  drops 10px, matching the `drawY + 10` in that branch.
- **Static shadows followed** once `StaticFilters` landed (the row below):
  ClassicUO also shadows trees, foliage and rocks behind a second toggle, and
  "is this graphic a tree or a rock" is exactly what that row answers.

Verified live. The transform, read off the sprite: skew -0.785 rad, scaleY
0.707, alpha 0.4, tint 0x000000, y offset = height/2 - 10, z just under the
body. The hidden gate proved itself by accident — the test character was hidden,
so no shadow appeared until `Set Hidden false`. And a pixel diff of the same
frame with the pass on and off shows **2572 pixels darkened by ~15/255** in a
right-leaning parallelogram at the character's feet, and nothing else changed.

Closed 2026-08-10 (StaticFilters): the fourth Tier 3 row — and with it the half
of the shadows row that was waiting on it.

- **The split is data, not a list.** ClassicUO's `StaticFilters.Load` looks like
  three hardcoded arrays written out to text files, and the interesting rule is
  buried in the writing: a "tree" seed that is **not** `IsImpassable` is filed
  under *vegetation* instead, and a vegetation seed that is impassable is
  dropped. So the classification depends on tiledata, which the browser has no
  access to — hence `/staticfilters.json`, resolved server-side. Against this
  install: 24 cave tiles, 46 trees and 194 vegetation, the last figure being the
  178 surviving seeds plus the 16 tree seeds that turned out to be passable.
- **Both toggles are ClassicUO's, applied where it applies them** — at draw
  time, never in the scene: which trees a player wants to see is not the
  server's business. `TreeToStumps` drops every foliage static (that is what
  removes the canopy) and swaps a tree's graphic for `0x0E59`; `HideVegetation`
  drops vegetation outright. Both off by default, as there.
- **Static shadows, finally.** The shadows row shipped without them because
  "is this a tree or a rock" had no answer yet. It does now, so trees, foliage
  and rocks cast the same parallelogram mobiles do, behind ClassicUO's second
  toggle (`ShadowsStatics`, on by default).

Verified live in the woods south-east of Britain (1402 statics in view): trees
as stumps redraws 32 trees as stumps and removes 29 canopies, hiding vegetation
drops 96 statics, and 62 of the 1402 qualify for a shadow — a pixel diff of the
same frame with static shadows on and off shows **85,356 pixels darkened by
~14.5/255** under the canopies, and nothing else changed.

Closed 2026-08-10 (light.mul shapes): the fifth Tier 3 row. Lights had a radius;
now they have their real shapes.

- **A UO light is not a circle.** `light.mul` holds about a hundred hand-drawn
  greyscale masks of different sizes — a wall torch throws a lopsided cone, a
  campfire a wide oval, a mobile a 225×225 soft disc — and every light-emitting
  graphic points at one through its tiledata **`Quality` byte**, the same field
  wearables use as their layer (ClassicUO `AddLight`: `light.ID = data.Layer`).
  A new `anima-assets::Lights` reads `lightidx.mul`/`light.mul`; the play server
  serves each shape at `/light/<id>.png` and the scene now carries the id per
  light source.
- **Additive there, subtractive here.** ClassicUO draws the greys into a light
  buffer with additive blending; this renderer erases holes in a darkness
  overlay instead. So the decoder hands back **white pixels with the intensity
  in the alpha channel** — the same mask and falloff, expressed for a
  compositor that subtracts. The pixel rule is otherwise ClassicUO's, negative
  lights (`val > 0x1F` → `~val & 0x1F`) included.
- **The colours followed the same day.** ClassicUO's `LightColors` is a
  graphic→colour table (a switch, then two nested range chains evaluated in
  order — a later group overwrites an earlier one, which the port preserves)
  plus the `ID > 200 → colour = ID - 200` convention, feeding a 64-row texture
  built by scaling a base RGB through one of six intensity **curves** per
  channel. All of that is ported: `light_colored(id, colour)` bakes the exact
  ramp into the mask's RGB, so `/light/<id>.png?c=<n>` is the same shape in the
  right colour. Its `IsHue` flag is not — that is only ever set by a
  user-supplied `lightshaders.txt`, which nothing here reads.

One thing the overlay had to translate rather than copy: ClassicUO *adds* its
coloured light buffer onto the world, and adding colour to a veil this pass has
just erased changes almost nothing — measured at 2/255 on a full-strength
brazier. What works here is to erase *to a colour* instead of to nothing: the
same mask is painted back over the hole, so the world shows through a thin wash
of the light's own colour.

Verified live at `globallight 28`: seven distinct shape ids in view
(1, 2, 26, 27, 29, 40, 42) decoding to 110×110 up to 300×300 masks, and a pixel
diff of the same night frame drawn with the shapes versus the old radial
fallback differs across **124,521 pixels**. The difference is not subtle in
kind: the circle lifts the whole neighbourhood evenly, while the real masks
leave the cobbles beyond a lamp dark and light the building faces beside it.
For colour, a GM-spawned brazier (`0x0E31`, which the table puts at colour 40,
red) tints its own glow: 68,975 pixels shift against a white-light control,
green and blue down ~23 each while red barely moves.

**A stale cache cost an hour, for the second time.** The coloured PNG kept
coming back white after the ramp landed, because Chrome had heuristically
cached `light/2.png?c=40` from the window when the server still ignored `?c=`.
`/abilities.json` had the same bite. Light shapes now answer `no-cache` — the
bytes are a pure function of the data files, but their *URL* does not change
when the server's answer does, and that is the case where a heuristic cache is
simply wrong.

Closed 2026-08-12 (directional lighting on stretched land): the sixth Tier 3
row, and the first picked by surveying all nine remaining rows in parallel
rather than taking the next one down the list. It won because it is the only one
that shares no edit site with any other — four of the others all want the same
statics loop.

- **The light belongs to map CORNERS, not tiles.** `Land.ApplyStretch`
  (`Land.cs:158-161`) calls `CalculateNormal` four times, once per corner of the
  diamond, each with that corner's own Z and its four axis neighbours — so
  neighbouring tiles *share* a corner's value and the shading is continuous
  across the terrain instead of faceted per tile. The renderer interpolates the
  light across the quad, which is the same gouraud result ClassicUO gets by
  interpolating the normal, with the trigonometry done once per corner on the
  CPU instead of once per fragment.
- **The four cross products have a closed form.** `CalculateNormal`
  (`Land.cs:164-238`) sums four cross products of `(±22, ±22, (nz - z) * 4)`.
  That reduces to `n ∝ (L + B - R - T, L + T - B - R, 22)` — **the corner's own
  Z cancels out entirely**. Checked against a literal port of the C# over
  200,005 cases including the all-equal early-out and ±127 extremes: worst
  component difference **2.2e-16**.
- **`IsStretched` is wider than "this tile's corners differ".** It is the OR of
  those same four calls, and each looks one tile *further out* than the diamond
  — so a tile whose own corners are level but which sits beside a step is
  stretched and shaded by ClassicUO, and was drawn flat here. Measured live in
  the client: **612 tiles by the old test, 963 by ClassicUO's**, in one 49×49
  window at (1420, 1702). The test and the light are two readings of the same
  four calls, which is why they land together.
- **Only stretched land is lit**, which is ClassicUO's rule and not an
  oversight: `LandView.cs:41-56` picks `SHADER_LAND` when `IsStretched` and
  `SHADER_NONE` otherwise, so a flat tile is drawn from its own 44×44 art with
  no shading. A level *stretched* corner still darkens to 0.854 — the fixed
  point of `get_light` — so the two classes of tile genuinely differ in
  brightness there too.
- **The custom shader was free.** These tiles are built from a plain
  `PIXI.Geometry`, not a `MeshGeometry`, and PIXI only batches the latter — so
  every stretched tile was already its own draw call. Confirmed at runtime:
  `mesh.batched === false` with the shader attached, 92 shaders for a whole view
  (one per texmap texture, not one per tile), fps unchanged at ~125.

Verified live at (1420, 1702). The light range came out **0.3232 .. 1.0708**,
matching the range the maths predicts. A level corner reads **0.8535534 at
terrain-shading 5, 10, 15 and 25** — `get_light`'s fixed point, which must not
move when the slider does — while a sloped corner beside it fans 0.884 → 1.006
over the same range. One tile's four vertex lights read
`0.854 / 0.854 / 0.653 / 0.854`, i.e. real per-vertex variation and not a flat
tint. Against a control with the light forced to 1, **206,006 pixels changed and
205,987 of them got darker**; the water in the same frame is untouched, because
water with no texmap is the one thing ClassicUO refuses to stretch.

Closed 2026-08-12 (seasonal land/static graphic remap): the seventh Tier 3 row,
and the one the inventory had backwards — "the season *system* exists, the remap
does not" was true, but the reason it was hard is that the remap is one line of
lookup wrapped in a trap that a naive implementation walks straight into.

- **UO ships one map, not four.** Winter is not separate art; it is
  `Data/Client/seasons.txt`, a 498-row table of `<season>,<kind>,<from>,<to>`
  substitutions the *client* applies — grass 573 becomes snow 1861, a green tree
  becomes a bare one — while the server never learns about it. Before writing a
  single row I parsed that file two independent ways (the shipped text, and the
  498 `WriteLine` literals in `SeasonManager.CreateDefaultSeasonsFile` that
  regenerate it) and reconciled them byte-for-byte: 312 winter land rows, and
  17/27/70/72 static rows for spring/fall/winter/desolation. Summer has none — it
  *is* the art as shipped.
- **The trap is that graphic and walkability share a struct.** ClassicUO stores
  the substitution *in* the object's `Graphic` field and then every tiledata
  accessor indexes the remapped id — including the ones `Pathfinder.CreateItemList`
  reads. So ClassicUO genuinely pathfinds on substituted flags, and **29 of the
  312 winter land rows flip IMPASSABLE** (27 impassable → passable). That is fine
  in ClassicUO because it *is* the authority; against a ServUO shard, whose
  `MovementImpl` reads the raw ids and never consults `Map.Season`, it would be a
  client that thinks it can walk onto tiles the server denies. So the remap here
  is computed as a **local `draw_g`** and emitted as a *sibling* field (`dg`),
  never rebound into the tile struct — `World` keeps the server's own graphics,
  and every `w`/`sz`/`li`/`h`/`pf` byte with them. A `#[ignore]`d regression test
  builds the same scene at summer and winter and asserts every pathing field is
  byte-identical; fault-injecting the "obvious" rebind (writing `draw_g` into
  `g`) makes it fail on field `g`, and the impassable-from-`draw_g` variant fails
  to compile because `MapData` deliberately exposes no by-graphic land-flags
  accessor. This is a **deliberate divergence from ClassicUO**, recorded in
  DESIGN.md so a future reader doesn't file it as a porting bug.
- **The texmap follows the drawn graphic.** All 312 winter land rows change
  `TexID`, and a stretched snow tile must be drawn with snow's seamless texture —
  so `dg`'s `tx` comes from a new by-graphic `land_tex_id`, verified live
  (dg 285 → tx 285, not the original tile's texmap).
- **One hop, never a chain.** The winter statics bucket has three rows whose `to`
  is itself a `from` (3245/3246/3253 → 3379, and 3379 → 6093). ClassicUO reads
  the array once (`arr[g] == 0 ? g : arr[g]`), so 3245 must land on 3379 and
  stop; a fixed-point loop would over-substitute to 6093. Pinned by test.
- **Foliage in winter/desolation is deleted, not recoloured.**
  `IsFoliageVisibleAtSeason` skips drawing `IsFoliage && !IsMultiMovable &&
  season >= Winter`. That is a *draw* skip (`fh` flag), not a removal — the static
  stays in the stream so it still blocks and still feeds `calculate_new_z` — and
  its scope is the easy thing to get wrong: it applies to real statics and to
  non-multi dynamic *items* (a GM-placed shrub vanishes) but **not** to multi
  components, so a boat or house keeps its own greenery. The idea of "remap the
  foliage flag too" was checked and dropped: 0 of the 70 winter and 0 of the 72
  desolation rows change the FOLIAGE bit.
- **The old colour wash was wrong and is gone.** `04-boot.js` used to paint a
  faint amber/blue/grey `fillRect` per season as a stand-in for the real remap. A
  grep of ClassicUO's whole `src/` for `Season` reaches SeasonManager / World /
  Land / Static / Multi / GameSceneDrawingSorting and not one of them touches
  `Hue` or `AlphaHue`. Worse, on this shard Felucca registers as **season 4**, so
  the wash meant every living player was permanently greyed — now the world is
  the substituted art with no tint.

Verified live four ways, all against the running ServUO. **Desolation (native** —
Felucca is season 4, and the core also flips to it on player death): 303 statics
remapped, every value matching the table, 65 foliage statics culled, the
substituted burnt-tree art loading and rendering; no grey wash where there used
to be one. **Winter, reachable no other way** (a shard's season is fixed at
`RegisterMap` and no GM command changes it — so a client-side `ANIMA_SEASON`
override was added, which doubles as the sharpest fault-injection rig there is,
since it *guarantees* the client disagrees with the server): 2027 land tiles
remapped to snow and 297 statics to their bare variants, all matching the table,
snow rendering, bare trees rendering — and **zero** of the ~1200 in-view pathing
fields moved versus the season-4 baseline at the same tile, the whole design
proven in the running client and not just the unit test. **Spring (180) and
fall (271)** statics remapped via the same override, zero mismatches. Summer is
identity by construction. The shard was left on its native season and the
override defaults off.

Closed 2026-08-12 (`TileFlag.Translucent` statics): the eighth Tier 3 row, taken
ahead of roof/ceiling fading on purpose — translucency forces a single-owner
alpha refactor in the renderer that roof fading needs too, so doing it first pays
for both. The flag turned out to be *already in the tree*, under the wrong name.

- **`WET` was Translucent all along.** `anima-assets`'s tiledata module declared
  `WET = 0x0000_0008`. ClassicUO's `Wet` is **0x80** (`TileDataLoader.cs:441`);
  **0x8 is `Translucent`** (`:425`), confirmed independently by ServUO
  (`Server/TileData.cs:135`) and by the data — classic water statics 0x1796+
  carry 0x80 and not 0x8, and **0 of 16384 land graphics** carry 0x8 at all.
  The constant had exactly one occurrence repo-wide (its own declaration), so
  nothing ever misbehaved; it was a trap, not a bug. Renamed rather than deleted,
  and the doc now records that `Wet` is a bit we will genuinely want later —
  ServUO gates *movement onto water* on it (`Server/Map.cs:1480`) — and that 109
  of the 272 translucent graphics carry both, which is how they get confused.
  Every other flag constant was audited against ClassicUO's enum and is correct;
  a unit test now pins all nine.
- **The alpha is 178/255 = 0.698, not a half.** `ProcessAlpha` eases `AlphaHue`
  to 178 (`GameSceneDrawingSorting.cs:371`) and the shader consumes it as
  `AlphaHue / 255f` (`StaticView.cs:63`, `ItemView.cs:49`). ClassicUO's `0.5`
  lives at `GameEffectView.cs:118` — a different mechanism on the effects path,
  and the easy wrong answer.
- **One alpha owner, sources that each carry their own membership.** The old
  transparency pass hardcoded `1` as every pooled sprite's resting alpha in three
  places, so the first time the player stepped behind a spiderweb it would have
  been walked to *opaque* and left there. Sprites now have a `_baseAlpha` they
  rest at plus named fade sources; `easeAlphas` composes `min(resting, …sources)`
  and is the only thing that assigns `.alpha`. **Minimum, not a priority chain**,
  because ClassicUO's chain is monotone in its shipped targets (0 above maxZ, 0
  roof, 178 translucent, 255 default, 76 foliage-behind-tree) — a priority order
  whose targets descend with priority *is* a minimum — and its one non-monotone
  branch, the circle of transparency, ships disabled (`Profile.cs:126`). `min`
  therefore agrees with a default-profile ClassicUO on every branch that fires,
  and is order-free so the next source is one call. `drawMobs` was rewritten to
  set a resting alpha and compose too — a literal no-op today (nothing fades
  mobiles yet), but it means the claim "one owner" is true rather than nearly true.
- **Honest about what this does NOT buy.** `min` never actually binds in this
  row: `A_OCC` 0.35 and `A_OCC_FOLIAGE` 0.45 are both below 0.698, so an
  occluding translucent static lands on the occlusion value either way. And roof
  fading will need more than one extra term, because the occlusion pass only ever
  *visits* sprites that overlap the avatar — a ceiling occludes nothing. It needs
  its own collector (hence "each source owns its membership"), plus a server
  change: statics at/above `max_z` are currently culled out of the stream
  entirely, so there is no sprite left to fade. `docs/RENDERING.md`'s deferred
  `(z-maxZ)<height` wall-top exception is a third prerequisite.
- **Three emit sites, and the graphic each reads.** Statics and multi components
  classify off the SEASONAL `dflags`; dynamic items off the raw graphic, because
  ClassicUO overrides `UpdateGraphicBySeason` on `Land`/`Static`/`Multi` only and
  an `Item` never gets substituted. That is not a formality: desolation remaps
  mushrooms 3345/3348/3351 → blood 4651, and blood *is* translucent while a
  mushroom is not, so reading `s.flags` would draw exactly those three opaque in
  the one season that produces them. A `#[ignore]`d test pins it at a real
  mushroom on the map, and fault-injecting `s.flags` makes it fail.
- **Not done, deliberately.** Land (structurally exempt — ClassicUO's
  `case Land land:` never calls `ProcessAlpha`, `:624`), mobiles (`:838` passes an
  *empty* `StaticTiles`, so `IsTranslucent` is unconditionally false for them),
  container/paperdoll/vendor art (world render only — a spiderweb in your backpack
  is opaque in ClassicUO too), and `GameEffect` — a fifth `ProcessAlpha` call site
  (`:967`) that belongs to the still-open *GameEffect blend modes* row. We also do
  not reproduce the 0→178 fade-IN (`CalculateAlpha`, ±25/20ms), which would make
  translucent statics the only objects in the game that ease in. One more
  divergence worth knowing: ClassicUO gates picking on `AlphaHue >= 127`
  (`:423`), so 178 stays clickable but an occlusion-faded 89 does not; our statics
  set no hit area at all, so there is nothing to diverge from yet.

Verified live against the running ServUO, at a blood-soaked cultist chamber
(956, 700) chosen because every dense spiderweb cluster sits at x > 5120, i.e. in
Lost Lands a shard may not have loaded. **89 translucent statics in view, every
one rendering at exactly 0.698**, 1838 opaque beside them, and `alphaActive`
empty — a translucent sprite that never occludes is never even tracked. Spawning
a web with `[add static 4282` in front of the avatar exercised the *item* path
(`tr:1` on a dynamic item) and it genuinely occluded: base 0.698, pulled to 0.35.
Walking three tiles away, it settled back at **0.698, not 1.0** — the exact
regression the old hardcoded resting value would have produced. Against a control
with every `_baseAlpha` forced to 1, **52,641 pixels changed**, and the
translucent frame is *less red and greyer* over them, i.e. blood blending toward
the stone floor beneath it. The spawned web was removed afterwards.

Closed 2026-08-12 (roof/ceiling fading): the ninth Tier 3 row, and the one the
previous row's alpha refactor was built for. It turned out to be a row about
**what the server refuses to send**, not about easing.

- **The fade IS the cull — our own doc had it backwards.** `RENDERING.md` §3
  described ClassicUO as hard-culling above the ceiling with a fade "bolted on as
  cosmetic polish". In fact `AddTileToRenderList`'s `maxZ` parameter is the
  literal **`150`** passed by its only call site (`GameScene.cs:629-635`), which
  also discards the `bool` it returns — so every `if (maxObjectZ > maxZ)` branch
  inside it is dead twice over. The live rule is `ProcessAlpha` easing `AlphaHue`
  to 0, and the object is only skipped once it *reaches* 0. ClassicUO never
  removes it from the tile's object chain, which is exactly why its own
  `Pathfinder.CreateItemList` is untouched by the ceiling — `Pathfinder.cs`
  contains no `_maxZ`, `AlphaHue` or `AllowedToDraw` anywhere.
- **So OUR architecture had inverted the reference's guarantee.** ClassicUO holds
  one world model and filters only in the renderer; we made the draw filter the
  *transport* filter, `continue`-ing ceiling-hidden objects out of a stream the
  browser also uses for walk math. Sending them back does not violate the house
  rule — it restores ClassicUO's invariant.
- **The scope was wrong as this file stated it.** `z >= max_z` withholds not just
  roofs but indoor **staircases and upper-floor slabs**: at a Britain house 381 of
  381 dropped statics inside `PATH_RADIUS` carried real pathing bits. "Send the
  roof back" would have been the wrong fix; "stop dropping anything, flag it
  instead" is the right one.
- **Two predicates, deliberately not one.** `hz` is the DRAW decision; a separate
  `path_withheld` is byte-for-byte the old cull and alone gates the `h`/`pf`
  suffix. That split is the whole safety argument, and it is what lets a
  ceiling-hidden object be *emitted* while remaining provably inert: the browser
  builds its path list with `tiledataPathObj(z, h|0, pf|0)`, which returns `null`
  whenever `pf == 0`, so a record without `pf` contributes **zero elements** to
  `createItemList` — `calculate_new_z` is identical element-for-element. Not
  measured: proved at the source, then pinned by a golden.
- **A faded object must also stop lighting, animating and evicting.** Three traps,
  all of which "flag instead of continue" walks straight into, because the old
  `continue` sat *above* them. (1) The light push: ceiling-hidden lamps began
  lighting the room from the storey you just hid, and since `LIGHT_CAP` is a hard
  64 with static lights appended last, they evicted the nearest real ones —
  regressing two closed rows. ClassicUO calls `AddLight` only from
  `StaticView.Draw`, which a faded object never reaches. (2) Animated statics kept
  the on-demand renderer repainting forever for invisible sprites. (3) Hidden
  objects shared the 4000-entry emit budget and truncated the DRAWN set — measured
  live at Blackthorn, 4000 exactly, silently restoring the very pop this row
  removes. They now have their own `HIDDEN_STATIC_CAP`.
- **`easeAlphas` was frame-rate dependent** (a fixed per-frame factor, no `dt`) —
  ~400 ms at 60 Hz, ~200 ms at 120 Hz. Occlusion hid that; a whole roof fading
  makes it obvious, so the ease is now `dt`-anchored to 60 Hz. It also now owns
  `visible`, written *unconditionally*: a ceiling-hidden sprite is created at
  alpha 0 and stays there, so folding that write into the "alpha changed" branch
  would have meant it never ran and hundreds of alpha-0 sprites stayed in the batch.
- **Deliberately NOT done at the time, each recorded rather than silently skipped.
  Later closed where noted.** The tall-wall `(z-maxZ)<height` exception is part of
  the dead-against-`150` code — porting it would ADD a divergence (**won't-port**).
  `_noDrawRoofs` is now a real bool on `DrawCeiling` / `map.noDrawRoofs` (no longer
  inferred from `max_z < 127`). `HasSurfaceOverhead` ships as `"so":1`. `_maxGroundZ`
  is a picking gate (`map.maxGroundZ`). Land still pops (its sprite is destroyed
  outright) — **won't-port**. The **pathing half** closed the same day — see the
  next entry.

Verified live against the running ServUO at a Britain house (1618,1556,30 → walk
south). The discriminating measurement is the transition, since a fade and a pop
share an end state: a per-frame sampler recorded `maxZ` dropping 127 → 67 and 352
statics becoming ceiling-hidden, then alpha decaying **0.905 → 0.820 → 0.743 → … →
0.011** across ~47 distinct intermediate values over ~390 ms, at a constant 0.905
ratio per frame — exactly the `dt`-anchored law, where a pop emits no intermediates
at all. They settled at alpha 0 with `visible: 0` (352 sprites out of the batch),
and walking back out returned every one of them: `invisible: 0`, with the only
sub-1.0 sprites left at **0.698** and **0.35** — translucent and occluding — i.e.
the shared alpha model composing three sources correctly. Payload **+5.4 KB
(+2.8%)**, scene build 9.0–9.4 ms, fps 120 / worst 9 ms. Zero hidden statics carried
a pathing field on the wire, and walking E,E,W,W under the roof returned to the
exact start tile with no denials. The pathing golden is captured from the
*pre-change* emitter (159/157/32 path-bearing and 437/1567/471 drawn statics at
three under-cover centers) so it cannot certify itself — a first attempt passed
vacuously at `max_z = 127` until its positive control caught it.

Closed 2026-08-12 (the pathing half of the ceiling rule): opened by the row above
and closed within the day, because the justification I recorded for deferring it
turned out to be false in all three of its clauses.

- **What the deferral claimed.** That withholding `h`/`pf` from ceiling-hidden
  objects was safe: the browser-vs-server disagreement was "always one-directional
  (browser more permissive, never a shifted Z) and never on a tile the server marks
  walkable". Three independent sweeps had put it at 0.019–0.064% of step
  resolutions, and the reasoning was that walk *denial* comes from the
  server-computed `w`, so a blind browser could never walk anywhere forbidden.
- **What it actually is.** Measured live in the shipping browser code, not in a
  Rust twin: at (6318,1688,-35) with `maxZ = 0`, the page's own
  `calculateNewZ(6317,1689,-35,5)` returned **0** while `tileSZ` returned **5** and
  `tileAt(...).w` was **1**. Both answers non-null, different, on a tile the server
  marks walkable — falsifying every clause at once. `createItemList` for that tile
  returned a single element (the land) because the surface it should have stood on
  had been stripped of its `pf`. Offline witnesses reach **20 Z units — 80 px** at
  Trinsic (1900,2680) and a Britain house (1617,1560), both on walkable tiles, both
  with the discarded `sz` correct.
- **Why the old measurements missed it.** They swept single hops from a *land*-derived
  starting Z. A real prediction chains, carrying the previous tile's `sz` forward;
  taking `currentZ` from the wire's own `sz` for the tile behind surfaces 79
  both-resolved-but-different cases, 8 on walkable tiles. A depth-5 cap in one sweep
  also stopped exactly one hop short of the 20-unit cases at depth 6.
- **The fix is a deletion.** `path_withheld` — the predicate the previous row
  introduced so this half *could* be deferred — is retired at all three emit sites.
  `hz` stays exactly as it was and remains purely a draw decision. This is also the
  more faithful port: ClassicUO's `Pathfinder.CreateItemList` reads the raw object
  chain and never consults the draw ceiling, so an invisible object still blocks and
  still contributes a standing surface there too.
- **The split was still worth building.** It is what made the previous row provably
  inert while this question was open, and what made this fix a one-line deletion per
  site rather than an untangling.

Verified live at the witness tile with the fix in: `own` **5**, `sz` **5**, `agree:
true`, and `createItemList` back to 3 elements including the restored
`{flags: SURFACE|BRIDGE, avgZ: 5, height: 10}` — the object whose absence produced
the 0. The pre-change golden is kept and its assertion inverted from equality to
**superset**: every pathing entry blessed before this change must still be present
and unchanged (nothing lost or altered), while the `draw:` half must still match
exactly, which is what would catch hidden objects starting to evict drawn ones from
the emit budget. At the three under-cover centers 410 of 1751 ceiling-hidden statics
now carry `pf` (the rest are outside `PATH_RADIUS`, as for any static).

Closed 2026-08-13 (**the draw-predicate class**, six emit sites): the previous
three rows each fixed one instance of the same defect and each turned out to be
incomplete — the fourth site sat fifteen lines from three that had just been
fixed. So this row censused the *class* instead of chasing the next instance.

**The class.** `scene.statics` is both the draw list and the browser's pathing
input, so any emitter that `continue`s an object out of the stream for a DRAWING
reason also deletes its `h`/`pf`, blinding the browser's `calculate_new_z` to
something the server still sees. ClassicUO has no such coupling: it holds one
world model and filters only in the renderer, which is why
`Pathfinder.CreateItemList` contains no `AllowedToDraw`/`AlphaHue`/`_maxZ`
anywhere.

**The mechanism**, one for all six: a never-drawn record, `"nd":1`, carrying
`x,y,z,g[,ms]` plus `h`/`pf` and *nothing a sprite would need* — no `pz`, `f`,
`tr`, `dg`, `a`/`ai`. The browser skips it in one line beside the existing `fh`
guard. It is the third sibling of `fh` and `hz` rather than a new concept.

Sites, with what each was withholding:

| site | was | population |
|---|---|---|
| multi `!visible` (`terrain.rs`) | dropped | 1284 authoritative components / 378 ids, 542 path-bearing — incl. every boat's origin hull (0x58A5 `MainHull`, pf=2 h=18 → 18 Z / 72 px on the ship's own tile) |
| tiledata `nodraw` statics | dropped | 6,751 path-bearing on Felucca; at (5575,810) **327 of 760** inside `PATH_RADIUS` |
| `nodraw` in `emit_multi_component` | dropped | 16 components, 6 path-bearing |
| `item_nodraw` filter (items) | dropped | ServUO ships path-bearing ones: `HouseLadder`'s climbable rung (0x3F28, SURFACE\|BRIDGE h=3), `InvisibleTile`/`ShipCannon` (0x2198) |
| `n_statics < 4000` draw budget | dropped whole records | 294 path-bearing inside `PATH_RADIUS` at map1 (1499,1455), 3538 at map5 (750,3365) |
| `*n_statics += 1` in multis | charged the drawn budget | the eviction trap acf1b2e fixed for statics and never here; roofs are 34% of some multis' components |

Budgets now **degrade rather than drop**: an over-budget object falls through to a
never-drawn record. A budget may stop us drawing something; it may never stop us
shipping its pathing bits.

**The review caught two FATALs, and the second is the interesting one.** The
plan's multi fix made the emit gate `server_keeps || is_origin` — which promotes a
*walkability* field into a **draw gate**. It holds today (`visible ⟹ server_keeps`,
0 violations over every merged component, asserted by a new test) but
`apply_uop_keep_overlay` assigns `server_keeps` unconditionally, so one
geometry-key collision would silently delete a visible wall from the screen. The
gate is therefore the UNION (`visible || server_keeps || is_origin`) with pathing
gated separately on the authority's own predicate — draw and transport decoupled,
which is this row's whole thesis. The first FATAL: the existing ceiling golden
could not see *any* of the six sites (its three centers emit zero `nd` records,
pass `multis: None`, and never reach the 4000 gate), so a new drawn-set golden was
blessed from **pre-change** code at centers that do. A third finding, the radar at
`07-hud.js`, drew a dot for every item with no `fh`/`nd` guard — it would have
turned invisible tiles into phantom minimap dots, and was already leaking
winter-culled foliage.

Verified live at (5575,810): **327 never-drawn records** on the wire, every one
carrying `pf`, none carrying a draw field; the browser's own `createItemList`
returns 3 elements where it returned 1; **zero sprites** built for the never-drawn
graphic; and `staticPool.size` **1018**, exactly the pre-change golden's drawn
count — the drawn set is byte-identical. On real boat data the origin hull now
reaches the wire as a never-drawn record (34 components, 1 never-drawn), and 649
authoritative-but-invisible components exist across the containers.

**Honest limit.** A full-window sweep at (5575,810) — 3,528 step resolutions —
found **zero** changed answers. The 327 records were genuinely being deleted, but
at that location they were redundant with other blockers. The census measured
*records withheld*, not *answers changed*; the witnesses with measured
behavioural impact are the multi origin (18 Z) and the ladder rung, not the
nodraw statics. This row restores the data; it does not claim a visible fix at
that site.

After it, "no draw predicate deletes a pathing byte" is true of the emitters.
`PATH_ONLY_CAP` and `PATH_RADIUS` are transport budgets this architecture needs
because `scene.statics` is a windowed poll, not ClassicUO's full object chain —
they are not draw predicates and are **won't-port** (ClassicUO has no emit cap).

Closed 2026-08-22 (the three leftovers of the draw-predicate row):

- ~~`HIDDEN_STATIC_CAP`'s value~~ — **CLOSED.** The two Blackthorn numbers were
  different measurements, not a contradiction: **4000 exactly** is
  `DRAWN_STATIC_CAP` filling (the pop the split prevents); **2616** is
  ceiling-hidden statics in one 49×49 window, which `HIDDEN_STATIC_CAP = 3200`
  sits past. The Britain-house live check was 352 hidden. Comment at
  `terrain.rs` pins both.
- ~~Invisible index-0 drawn as the parent Item's sprite~~ — **CLOSED.**
  ClassicUO `Item.LoadMulti` (`:256-284`): a visible component is a `Multi`; an
  invisible index-0 is not, and the parent Item draws it via
  `DisplayedGraphic = MultiGraphic` iff `MultiGraphic > 2`
  (`CheckGraphicChange` `:358`). We skip drawing the parent (`items_json`
  filters `is_multi`), so `multi_component_never_draw` emits that origin
  graphic as a sprite rather than an `nd` record. Tests:
  `invisible_origin_is_drawn_when_graphic_exceeds_two`.
- ~~`near_multis` bounds for custom-house designs~~ — **CLOSED.** Per-house
  `MultiDistanceBonus` (`Item.cs:305-308`) plus 0xD8 design-tile extents;
  include when Chebyshev(origin, player) ≤ `RADIUS + bonus`
  (`HouseManager.IsHouseInRange` `:75-77`). Replaces the global `MULTI_MARGIN
  = 32`. Tests: `multi_distance_bonus_matches_classicuo_and_grows_for_design_tiles`,
  `multi_origin_in_view_uses_view_plus_bonus`.

---

## Tier 4 — the AI contract

This is the one that cuts against the project's own thesis.

- ~~**Brains have no terrain perception.**~~ — **CLOSED.** `Observation.terrain` is
  an optional `TerrainView`: a square walkability window (`walkable`, standing `z`,
  and the serial of a closed door that would have to be opened first) filled by
  `agent::survey_terrain` over the existing `path::Terrain` trait. The core still
  reads no map files — `World::observe` leaves it `None` and a driver holding
  `MapData` fills it (`Session::observation_with_terrain`, surveyed through the same
  `scene::MapTerrain` the auto-walk route uses, so brain and driver cannot disagree
  about what is walkable). Wired through the JSON contract (schema **v17**; per-tile
  data packed as parallel `walk` string + `z` array + sparse `doors`, since this is
  the largest field in an observation and is re-sent every poll), the NDJSON bridge
  (`{"cmd":"observe","terrain_radius":N}`), and both agent runners. `WanderBrain`
  now turns before hitting a wall it can see and opens a door instead of walking
  into it, with tests for both — and, with no window supplied, behaves exactly as
  it did before.
  **Known limit, by design:** every tile is judged as a step from the player's
  current Z, so the far edge of the window is approximate on sharply sloped
  ground; exact multi-step reachability still means `find_path`.
- ~~Cliloc-localized messages reach brains as an unresolved id plus raw args~~ —
  **CLOSED.** `JournalEntry::display` carries the resolved line, filled by
  `anima_net::resolve_journal` — the driver, because the core has no Cliloc table
  (the same split as `Observation::terrain`). Wired into both agent runners and the
  JSON contract (schema **v18**); `text` still carries the raw args. Verified live
  through the NDJSON bridge: a party invite now reaches the brain as
  `display: "Who would you like to add to your party?"` instead of `cliloc 1005454`
  with empty args.
- ~~0xCC ClilocMessageAffix drops the affix-flags byte and concatenates the affix
  into the cliloc *args*~~ — **CLOSED.** The affix is kept beside the arguments
  (`JournalEntry::affix`/`affix_prepend`) and joined to the *resolved* text on the
  side flag `0x01` asks for, matching ClassicUO `DisplayClilocString`/`AffixType`.
  Folding it into the args had corrupted the argument list and lost the affix
  entirely on any template without a placeholder.
- ~~Server-gump layout grammar~~ — **CLOSED.** All of `tilepic`/`tilepichue`,
  `gumppictiled`, `checkertrans`, `tooltip`, `itemproperty`, `buttontileart`,
  `picinpic`, `textentrylimited` and `noclose`/`nodispose`/`nomove` now parse into
  typed elements (the window flags onto `GumpLayout` rather than the element list),
  reach both the JSON contract and the renderer, and draw. `{ group N }` is tracked
  so `Radio` carries its group — the renderer names each group separately, without
  which a two-question gump could only ever hold one answer. `tooltip`/`itemproperty`
  decorate the **preceding** element, as UO attaches them, rather than taking a slot.
  Verified by parser tests plus a synthetic gump injected into the live renderer:
  two radio groups each hold their own selection, `textentrylimited` caps input, and
  the art/tiled/crop/translucent elements draw.

---

## Tier 5 — assets (`crates/anima-assets`)

Beyond T0.1 and T0.7 above. Closed 2026-08-22:

- ~~`verdata.mul` patching~~ — **CLOSED.** `Verdata::open` + `TileData::apply_verdata`
  (file id 30: land groups 836/964 bytes, static 1188/1316, `block_id - 0x200` for
  statics). Missing file → empty table, matching ClassicUO's skip.
  `TileDataLoader` / `VerdataLoader`.
- ~~`mapdif`/`stadif` and 0xBF/0x18~~ — **CLOSED.** `MapDiffs` per facet;
  `MapData::apply_patches(land, sta)` applies the first N `mapdifl`/`stadifl`
  entries (ClassicUO `MapLoader.ApplyPatches`). GeneralInfo 0x18 stores
  `(mapPatches, staticPatches)` per facet in ClassicUO order (cap 6). The play
  loop reapplies on `map_patches_gen`. Test:
  `general_info_map_patches_stores_counts_in_classicuo_order`.
- ~~`speech.mul` keyword encoding~~ (**CLOSED** — see above);
- ~~`Multimap.rle`/`facet0N.mul`~~ — **CLOSED.** `MultiMap` RLE → greyscale `Image`;
  `GET /multimap.png`. The isometric world map still rasters from map+radarcol
  (`/worldmap.png`); this is the classic client's own facet bitmap
  (`MultiMapLoader`).
- ~~`fonts.mul` + `unifont*.mul`~~ — **CLOSED.** `Fonts` decoder (`FontsLoader`);
  `GET /font/text.png?font=&uni=&t=` and `/font/ascii|uni/{font}/{ch}.png`. Overhead
  names use the unicode font via CSS mask (ClassicUO's 1-bit glyphs, hued by
  notoriety/speech).
- ~~`art.def`/`TexTerr.def` alias tables~~ (**CLOSED**);
- ~~`gump.def` alias table~~ (**CLOSED**);
- ~~`skills.mul`~~ (**CLOSED**);
- ~~`Prof.txt`/`Professn.enu` professions~~ — **CLOSED.** `ProfessionLoader` parse;
  0xF8 profession byte from `Desc` (Warrior=1 … Ninja=7, Advanced leftover = 0).
  Wizard fetches `/professions.json` (skills 30×4 = 120, stats sum 90).
- ~~`tileart.uop`~~ — **CLOSED.** Stack-amount aliases rewrite scene `g`;
  `TryGetAppearance` (type 0, only when more than one appearance type) feeds
  paperdoll gump ids as `MALE_GUMP_OFFSET + appearanceId`
  (`PaperDollInteractable.GetAnimID` `:427-436`, `TileArt.TryGetAppearance`).
- ~~`Anim1.def`/`Anim2.def` + `% AnimationCount`~~ (**CLOSED**);
- ~~`light.mul`~~ (**CLOSED**, Tier 3);
- ~~cliloc language selection and BWT-compressed clilocs~~ — **CLOSED.**
  `Cliloc::open_lang` loads ENU then overlays `ANIMA_LANG` (default `enu`).
  Fourth byte `0x8E` → BWT (`ClilocLoader.cs:77`).
- ~~`Client.Version`-driven format selection~~ — **CLOSED** as file-length
  detection: ClassicUO `TileDataLoader.cs:35` uses `Version < CV_7090`; we have
  no client-version string in `anima-assets`, and a HS `tiledata.mul` is always
  ≥ 493_568 bytes (the HS land section), which is the same fork. Multis already
  pick 12- vs 16-byte stride the same way.
- ~~MUL fallback when a UOP is absent~~ (**CLOSED**);
- ~~`hues.mul` ramp-index (red channel)~~ (**CLOSED**).

---

## Deliberately not doing

Login/game-stream encryption (this client targets shards that accept unencrypted
connections); ClassicUO's assist/plugin API; Enhanced-Client-only packets (0xE3,
0xEC); ClassicUO's internal plumbing with no player- or agent-visible effect.

## Keeping this file honest

Add a ClassicUO source location and an end-to-end acceptance test when closing a row.
When a row is closed, say so here rather than deleting it — the "wrongly reported as
missing" list above exists because that history was not written down.

~~Known minor divergence carried over from the previous audit: the mobile-incoming
family (`0x78`, `0xD3`) does not clear a mobile's *stale* worn items…~~ —
**CLOSED.** Both handlers now share one `apply_worn_items`, which treats the
list as authoritative and drops equipment it omits (symptom: a mount stayed
drawn under a rider who dismounted out of view). Two deliberate departures from
ClassicUO's `UpdateObject`, which removes every non-backpack child and recreates
the listed ones: we remove only what the list omits, because recreating an item
that never left drops its name and OPL — ClassicUO silently invalidates the
tooltip of every worn piece each time its wearer walks back into view; and we
skip the container pseudo-layers (0x1A-0x1D) rather than guard on ClassicUO's
"is this container's gump open" flag, which is renderer state the core does not
have (D3). That second point is load-bearing rather than cosmetic: a vendor's
shop containers hang off the vendor mobile on 0x1A-0x1C (see
`recorrelate_shop_buy`, which resolves buy-list prices through exactly that
link) and the bank box on 0x1D, so a layer-blind sweep would empty the buy
window. Five tests cover it, including the vendor/bank case, the OPL one, and a
truncated-frame guard (a record cut off mid-way must not read as "everything
else came off" now that omission deletes).

**Live A/B against ServUO, same experiment both ways.** Spawn a vendor, note
its worn layers, teleport out of view, `[Remove` one worn item server-side (so
no 0x1D ever reaches us), teleport back and let its 0x78 resend:
*old code* — the deleted item was still listed as worn. *new code* — gone,
while layers 0x03/0x05/0x0B/0x15 and the shop containers 0x1A/0x1B survived,
and the Buy window still opened with all four prices resolved to concrete
serials through `cont: 0x400177C3` — the very layer-0x1A container the sweep
had to leave alone.

---

Closed 2026-08-24 (night, and the projectile that never burst): two defects
found by re-reading ClassicUO against shipped code rather than against the
backlog, which was empty. Neither is a missing feature; both are wire fields we
already had and used wrongly.

**1. The two light levels are on opposite scales, and we compared them.**
`World::effective_light` combined 0x4F and 0x4E as `min(overall, personal)` on
the reasoning that "lower = brighter, so `min` picks the brighter". That is true
of 0x4F alone and false of the pair. ClassicUO's `IsometricLight::Recalculate`
is explicit, comment included: `int reverted = 32 - Overall; // if overall is 0,
we have MAXIMUM light`, then `current = Personal > reverted ? Personal :
reverted` and `IsometricLevel = current * 0.03125f`. So **0x4F is a darkness and
0x4E is a brightness floor** — the thing Night Sight raises — and the client
combines them as a *brightness*, `max(personal, 32 - overall) / 32`. We now
compute `32 - max(personal, 32 - overall)` and keep one "higher = darker" scale
end to end, so nothing downstream had to change.

What that cost: ServUO's `PlayerMobile::CheckLightLevels` sends 0x4E paired with
**every** 0x4F, and `ComputeBaseLightLevels` sets `personal = m_LightLevel`,
which is 0 for any character without Night Sight. Once one such packet was
accepted, `min(overall, 0)` pinned the scene to 0 forever: no dusk, no night, no
dungeon, and the whole `light.mul` shape-and-colour pass — which only runs
behind `if (fxTint > 0.05)` — never executed again. And where it did fire it was
backwards: Night Sight *lowered* the result. Note this does not retract the
`light.mul` row above; that measurement was real. It means the darkness it
depends on had been dying as soon as the server resent a personal light.

Live A/B on the shard, at night (`overall` 12). ServUO's `[light N` sets
`e.Mobile.LightLevel` (`Handlers.cs:506`) — the **personal** level, not the
global, which is what makes it the right probe here:

| probe | `scene.light` now | under `min(...)` |
|---|---|---|
| no personal light (plain night) | **12** — dark | 0 — full daylight |
| `[light 25` (Night Sight) | **7** — brighter | 12 — no change, or darker |
| `[light 0` (dispelled) | **12** — dark again | 0 |

`32 - max(25, 32 - 12) = 7` and `32 - max(0, 20) = 12`, matching ClassicUO
exactly. The browser's tint was on the wrong denominator too (`light / 0x1F`
against a 32-step scale) and its coefficient was never exercised; it is now
`min(FX_MAX_NIGHT, light / 32)`, because a near-black overlay at alpha `a`
approximates ClassicUO's multiply-by-`1 - a`. `FX_MAX_NIGHT` survives as a
**cap**, not a scale: we draw far fewer light sources than ClassicUO, so an
authentic 0.97 at the darkest dungeon level would be unreadable.

**2. `doesExplode` was skipped, so nothing burst on impact.** 0x70/0xC0/0xC7
carry an explode byte that `effects.rs` dropped with `r.skip(1)`, so explosion
potions, explosive bolts and snowballs flew to the target and simply stopped —
losing the visual confirmation that the thing landed. ClassicUO keeps it as
`CanCreateExplosionEffect` and spawns the burst **client-side** in
`MovingEffect::RemoveMe` → `FixedEffect(0x36CB, Hue, 400, 0)` at the target,
inheriting the blend. So the core only carries the flag (`Effect::explodes`) and
the browser owns the spawn, which is the D3 line. One wrinkle: the browser has
no `animdata.mul` to animate `0x36CB` from, so the burst's frame list rides
along on the effect that causes it (`exFrames`/`exInterval`, emitted only when
`explodes` is set rather than on every effect in the feed).

The neighbouring `fixedDir` byte stays skipped **on purpose**: ClassicUO reads it
into `MovingEffect.FixedDir` and then never uses it anywhere in the client, so
our unconditional rotation already matches its real behaviour. The parser test
therefore sets `fixedDir` non-zero while varying explode, so a future off-by-one
between the two is caught.

**Verification, honestly.** The light fix is live-verified on the shard (table
above). The explode fix is **not**: it is pinned offline at both seams — the
parser (`graphic_effect_explode_byte_is_retained`, three byte values plus the
neighbouring fields) and the scene emitter
(`an_exploding_effect_carries_the_burst_the_browser_has_to_draw`, which also
asserts an ordinary effect ships neither key) — but the burst was not seen drawn
in a browser. ServUO has only three `explodes: true` call sites
(`SnowPile`, `PileOfGlacialSnow`, pet-training `SpecialAbility`), and all three
need a target that is itself carrying snow, which did not survive headless
driving. Worth a look the next time a real client session is up.
