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
- **The other boat-control path is unreachable from this client, and that is a
  Tier 5 gap, not a boat gap.** `BaseBoat.OnSpeech` dispatches the tiller-man
  commands ("forward", "stop") on `e.Keywords`, which needs the `speech.mul`
  keyword encoding this project does not implement. The same wall stops
  "vendor buy" from working. Mouse piloting is therefore the whole of our boat
  control rather than a convenience on top of speech.

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

Tier 2 is **not** finished — still open: the 0x11 `type >= 6` combat tail
(unobservable on this shard, see the expansion note below), journal
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

Tier 2 still open: the 0x11 `type >= 6` combat tail (unobservable here), grid
loot, the info bar, the counter bar and friends, and window management.

Closed 2026-08-09 (grid loot): the third Tier 2 row. No contract change — the
one-click take is two packets we already had. Two notes:

- **`0xFFFF/0xFFFF` on a drop means "you place it".** Written up under the
  container row above, because it corrects a conclusion recorded there.
- **A refused loot looks identical to a broken one.** The first live click on a
  war hammer left it on the corpse with nothing in the journal; the cause was
  carry weight (99/75 on a STR-10 character), and ServUO simply bounces the
  item back. Raising STR made the same click work. Worth knowing before
  debugging the client: the server refuses silently here too.

Tier 2 still open: the 0x11 `type >= 6` combat tail (unobservable here), the
info bar, the counter bar and friends, and window management.

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
grab-bag row above (the other five are still open). Client-only again — the bar
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

Tier 2 still open: the 0x11 `type >= 6` combat tail (unobservable here), and the
ignore list / combat book / racial-abilities book / network stats / inspector
remainder of the grab-bag row.

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
cannot show at all. Client-only — the inspector reads the scene and sends
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
| ~~Targeted use (0xBF/0x2C) — bandage self/target in one packet~~ — **CLOSED, live-verified** | `build_bandage_target` + `Action::BandageTarget { bandage, target }` + `bandage:<serial>[:<target>]`. `target` 0 = self (the `PartyAccept` sentinel convention), which is the case worth the shortcut. Skips the double-click → 0x6C cursor → reply round-trip, which is what makes reliable self-healing under pressure possible. Live: `bandage:<serial>` with no target healed the player, with `scene.target.active == 0` throughout — **no cursor is ever raised**. Note ServUO still emits cliloc 500948 "Who will you use the bandages on?" from inside the 0x2C handler before invoking the target itself (`Bandage.cs BandageTargetRequest`), so that line in the journal is not evidence of a cursor. ServUO's siblings 0x2D TargetedSpell / 0x2E TargetedSkillUse / 0x30 TargetByResourceMacro are the same idea and still absent | S |
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

---

## Tier 2 — UI for state we already decode

| Gap | Note | Effort |
|---|---|---|
| ~~**Extended status sheet**~~ — **CLOSED** | all of it (armor + the four resistances, weight/max, stats-cap, followers/max, damage range, luck, tithing, and the three stat locks) now leaves `build_scene` and shows in the status panel. It had been parsed into `World` all along and simply never sent | M |
| **0x11 `type >= 6` combat tail** | max resists, HCI/DCI/SSI/DI/LRC/SDI/FCR/FC/LMC are explicitly not parsed (`game.rs:2757`); no field of that family exists anywhere | M |
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
| ~~Window management~~ — **CLOSED, live-verified** | Every draggable panel — the dynamic windows from `makeWindowFrame` *and* the static ones (paperdoll, spellbook, skills, options, macros) — now **remembers where it was left** and comes back inside the viewport, because the persistence lives in `makeDraggable`, the one function they all already went through. Keyed by element id or class, never by serial: a serial is per-corpse/per-bag and never returns, so "the next container opens where I put the last one" is the only memory worth having (ClassicUO's per-type defaults do the same). Windows are clamped on restore and on browser resize, and a position saved on a bigger screen is **written back clamped**, so it heals instead of being re-clamped forever. `resize: both` is opt-in per window (bulletin board, server gumps, plus the journal from the row above) — deliberately not on the map or an authentic container, whose layout is in the server's own pixel space | M |

---

## Tier 3 — rendering fidelity

Shadows; corpse equipment layers (corpses render as a bare body); death animation
(mobiles do not fall); `light.mul` light shapes/colours (a radius-only light system
exists — `build_scene`'s `lights` list in `scene/mod.rs`); directional lighting on stretched land; seasonal
land/static graphic remap (the season *system* exists, the remap does not);
`TileFlag.Translucent` statics drawn opaque; static hue from `statics.mul` discarded
at decode; mount rider vertical offset; seated-character deformation; roof/ceiling
fading; 0x23 DragEffect decoded but never drawn; GameEffect blend modes and
projectile rotation; StaticFilters (tree→stumps, hide vegetation).

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

Beyond T0.1 and T0.7 above: `verdata.mul` patching; `mapdif`/`stadif` map-diff files
and 0xBF/0x18; `speech.mul` keyword encoding (so NPC keyword commands can never be
sent); `Multimap.rle`/`facet0N.mul` (the map window shows only its gump frame and
pins, no terrain raster); `fonts.mul` + `unifont*.mul`; `art.def`/`TexTerr.def` alias
tables; `gump.def` alias table; `skills.mul` (skill names + HasAction);
`Prof.txt`/`Professn.enu` professions (0xF8's profession byte is hardcoded 0);
`tileart.uop`; `Anim1.def`/`Anim2.def` group-replacement tables **and** the missing
`% AnimationCount` clamp for animals/monsters; `light.mul`; cliloc language selection
and BWT-compressed clilocs; `Client.Version`-driven format selection; MUL fallback
when a UOP is absent; and `hues.mul` ramp-index selection (we pick the ramp by pixel
brightness where ClassicUO uses the red channel).

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
