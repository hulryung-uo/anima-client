//! World model: the single source of truth for "what the game looks like now".
//!
//! Incoming packets mutate this (see [`crate::net::game`]); consumers (AI brain,
//! renderer) only read it. Mirrors ClassicUO's `World` — player plus mobiles and
//! items keyed by serial.

use std::collections::{HashMap, HashSet};

use crate::net::login::LoginResult;
use crate::types::{Position, Serial};

/// A creature/character (NPC or player) in the world.
#[derive(Debug, Clone, Default)]
pub struct Mobile {
    pub serial: u32,
    pub name: String,
    pub pos: Position,
    pub body: u16,
    /// Facing, low 3 bits (0..7).
    pub direction: u8,
    pub hue: u16,
    /// Notoriety byte (1=innocent, 3=gray/criminal, 6=murderer/red, ...).
    pub notoriety: u8,
    pub hits: u16,
    pub hits_max: u16,
    pub mana: u16,
    pub mana_max: u16,
    pub stam: u16,
    pub stam_max: u16,
    /// Hidden status — the mobile-update status-flags 0x80 bit (ServUO
    /// `Mobile.cs GetPacketFlags`: 0x04 Poisoned, 0x08 YellowHealth, 0x40
    /// WarMode, 0x80 Hidden). Set by the Hiding/stealth skills or a GM `[set
    /// Hidden true`; the server only describes a hidden mobile to a client that
    /// can actually perceive it (self, or an ally within Detect Hidden range),
    /// so seeing this flag at all means we're allowed to see them — the
    /// renderer draws them semi-transparent as feedback. Re-derived from the
    /// flags byte on every 0x20/0x77/0x78 (not sticky): a later update that
    /// omits the bit clears it back to `false`.
    pub hidden: bool,
    /// Poisoned status — the mobile-update status-flags 0x04 bit (see
    /// [`Mobile::hidden`]'s doc for the full bit layout). In UO the health bar
    /// turns green while this is set, independent of the actual HP fraction —
    /// it's how you tell a mobile is poisoned at a glance. Re-derived from the
    /// flags byte on every 0x20/0x77/0x78 (not sticky): a later update that
    /// omits the bit clears it back to `false` (e.g. Cure Poison).
    pub poisoned: bool,
}

/// An item — on the ground, in a container, or equipped.
#[derive(Debug, Clone, Default)]
pub struct Item {
    pub serial: u32,
    pub graphic: u16,
    /// Stack count for a normal item; for a corpse (`graphic == 0x2006`) the server
    /// overloads this field with the dead creature's BODY id instead (ServUO
    /// `Corpse.Amount = owner.Body`; ClassicUO `Item.GetGraphicForAnimation`
    /// special-cases `IsCorpse` to return `Amount`). The renderer, not this crate,
    /// interprets which meaning applies.
    pub amount: u16,
    pub pos: Position,
    /// Container serial, or `None` when lying on the ground.
    pub container: Option<u32>,
    /// Worn layer (0 when not equipped).
    pub layer: u8,
    pub hue: u16,
    pub name: String,
    /// Facing (low 3 bits, 0..7), sent as a per-item byte on 0x1A/0xF3 — only
    /// meaningful for a corpse (graphic `0x2006`), where it orients the death-pose
    /// sprite (ClassicUO stores this same wire byte as both `Item.Direction` and,
    /// reused, `Item.LightID`/`Layer`; we only need the facing).
    pub direction: u8,
    /// Is this a **multi** (a placed boat or house), not a normal pickable item?
    /// Set when `type == 2` on 0x1A/0xF3 (ClassicUO `UpdateGameObject`'s
    /// `item.IsMulti`); `graphic` is then a *multi id* (an index into
    /// `multi.idx`/`multi.mul`, resolved via `anima_assets::Multis`), not an ART
    /// graphic. Reuses the ordinary `Item`/`World::items` machinery (get-or-create,
    /// 0x1D delete/prune, facet purge) instead of a separate map — a multi is a
    /// world *entity* like any other, just one the renderer expands into many
    /// static-like components instead of drawing directly.
    pub is_multi: bool,
}

/// Self-only fields that don't live on the player's [`Mobile`].
#[derive(Debug, Clone, Default)]
pub struct PlayerStats {
    pub is_female: bool,
    pub strength: u16,
    pub dexterity: u16,
    pub intelligence: u16,
    pub gold: u32,
    pub armor: i16,
    pub weight: u16,
    pub weight_max: u16,
}

/// One journal (chat/system) line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalEntry {
    pub serial: u32,
    pub name: String,
    pub text: String,
    pub msg_type: u8,
    pub hue: u16,
    /// Cliloc id for localized messages (0xC1/0xCC); 0 for plain speech. For a
    /// cliloc line, `text` holds the raw tab-separated args — the brain resolves
    /// `(cliloc, text)` to display text via the client's Cliloc table.
    pub cliloc: u32,
}

/// One skill's standing. Values are in **tenths** (wire units): 500 == 50.0.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Skill {
    pub id: u16,
    /// Effective value (base + transient item/buff bonuses), tenths.
    pub value: u16,
    /// Trainable base — skill *progression* registers here, tenths.
    pub base: u16,
    /// Cap, tenths (default 1000 == 100.0).
    pub cap: u16,
    /// 0 = up, 1 = down, 2 = locked.
    pub lock: u8,
}

/// A graphical effect event (0x70 GraphicalEffect / 0xC0 HuedEffect / 0xC7
/// ParticleEffect): a spell bolt, hit sparkle, explosion, or field visual.
/// Mirrors `recent_sounds`/`recent_damage` — a capped queue with a monotonic
/// `seq` so the renderer spawns each visual exactly once. `kind` is the wire
/// `GraphicEffectType`: 0 = Moving (a projectile src→tgt), 1 = Lightning (a bolt
/// at the target), 2 = FixedXYZ (stays at src x/y/z), 3 = FixedFrom (stays on the
/// src/tgt serial). Positions are tiles; `graphic` is the ART tile id (animated
/// via animdata.mul); `hue` is 0 for 0x70 (0xC0/0xC7 carry one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Effect {
    pub seq: u64,
    pub kind: u8,
    pub src_serial: u32,
    pub tgt_serial: u32,
    pub graphic: u16,
    pub sx: u16,
    pub sy: u16,
    pub sz: i8,
    pub tx: u16,
    pub ty: u16,
    pub tz: i8,
    pub speed: u8,
    pub duration: u8,
    pub hue: u16,
}

/// Current weather state (from 0x65). `kind` reuses the wire type byte:
/// 0 = rain, 1 = fierce storm, 2 = snow, 3 = storm; 0xFE/0xFF = none/reset.
/// `intensity` is the particle count (0..70). The renderer only animates the
/// kinds it knows (rain/snow) and ignores the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Weather {
    pub kind: u8,
    pub intensity: u8,
}

impl Default for Weather {
    fn default() -> Self {
        // 0xFF = no weather until the server says otherwise.
        Self { kind: 0xFF, intensity: 0 }
    }
}

/// One active buff or debuff icon on the player (from 0xDF). `icon` is the raw
/// `BuffIconType` id off the wire (used as the upsert key); `name` is a short
/// human label resolved from a hardcoded icon→name table (the real name comes
/// from a cliloc we don't have a table for, so it's approximated). `dur` is the
/// duration in seconds the server sent (0 = no timer / permanent); the client
/// counts down from when it first saw the icon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Buff {
    pub icon: u16,
    pub name: String,
    pub dur: u32,
}

/// A vendor's BUY window (from 0x74 OpenBuyWindow). `container` is the vendor's
/// for-sale container (its items already live in [`World::items`] with
/// `container == this`); `entries` are `(price, name)` in **packet order**, which
/// the renderer matches to those container items by index (see ClassicUO
/// `PacketHandlers.BuyList`). `vendor` is the vendor mobile serial — the serial a
/// BUY request (0x3B) is addressed to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShopBuy {
    pub vendor: u32,
    pub container: u32,
    pub entries: Vec<(u32, String)>,
}

/// One line of a vendor's SELL list (from 0x9E SellList): an item in our pack the
/// vendor will buy, with the price it pays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShopSellItem {
    pub serial: u32,
    pub graphic: u16,
    pub hue: u16,
    pub amount: u16,
    pub price: u16,
    pub name: String,
}

/// A vendor's SELL window (from 0x9E SellList). `vendor` is the vendor mobile
/// serial a SELL request (0x9F) is addressed to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShopSell {
    pub vendor: u32,
    pub items: Vec<ShopSellItem>,
}

/// A server-sent generic gump / dialog (from 0xB0 DisplayGump or 0xDD
/// DisplayGumpPacked): a quest dialog, NPC menu, confirmation box, etc. `layout`
/// is the raw UO gump command string (`{ resizepic 0 0 5054 400 300 }{ button …
/// }…`); `text` holds the referenced text lines (referenced by index from
/// `text`/`croppedtext`/`htmlgump` commands). The renderer (anima-net scene
/// bridge) parses `layout` into elements; the brain answers via
/// [`crate::agent::Action::GumpResponse`] (packet 0xB1). Keyed by `serial`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Gump {
    pub serial: u32,
    pub gump_id: u32,
    pub x: i32,
    pub y: i32,
    pub layout: String,
    pub text: Vec<String>,
}

/// One entry of a server-sent context (popup) menu (0xBF/0x14). `cliloc` is the
/// localized-string id for the label (resolve via the Cliloc table); `index` is
/// echoed back when the entry is chosen (0xBF/0x15); `flags` carry attributes
/// (e.g. `0x01` = disabled/colored, `0x02` = arrow/submenu).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PopupEntry {
    pub index: u16,
    pub cliloc: u32,
    pub flags: u16,
}

/// A server-sent context menu (right-click popup, 0xBF/0x14) for `serial`.
/// Replaced when a new one arrives; cleared on selection. The brain/renderer
/// reads it; selecting an entry sends [`crate::agent::Action::PopupSelect`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PopupMenu {
    pub serial: u32,
    pub entries: Vec<PopupEntry>,
}

/// A book opened by the player (from 0x93 OpenBook / 0xD4 OpenBookNew). The header
/// packet sets `serial`/`title`/`author`/`writable`/`page_count` and sizes `pages`
/// to `page_count` empty pages; the page content arrives separately in 0x66
/// BookData and fills `pages` (each page is its lines). The client requests the
/// pages (outgoing 0x66) once the header lands. The renderer reads it; the brain
/// never parses bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Book {
    pub serial: u32,
    pub title: String,
    pub author: String,
    pub writable: bool,
    pub page_count: u16,
    /// One `Vec<String>` of lines per page, indexed `page - 1`. Sized to
    /// `page_count` (empty until 0x66 fills each page).
    pub pages: Vec<Vec<String>>,
}

/// One school spellbook's known contents (0xBF/0x1B NewSpellbookContent), sent
/// only when the book is actually opened (double-click) — ServUO
/// `Spellbook.DisplayTo`, gated on `NetState.NewSpellbook` (negotiated for any
/// client version >= 4.0.0a, which anima-client's reported version always
/// satisfies — see `anima_net::CLIENT_VERSION`). An owned-but-unopened book
/// simply has no entry in [`World::spellbooks`] yet.
///
/// `graphic` is the book's ItemID (school identifier — ServUO `Spellbook`
/// subclass constructors): 0x0EFA magery, 0x2253 necromancy, 0x2252 chivalry,
/// 0x238C bushido, 0x23A0 ninjitsu, 0x2D50 spellweaving, 0x2D9D mysticism.
/// `offset` is the wire-sent `BookOffset + 1` (ServUO `Spellbook.BookOffset`):
/// the absolute spell id of `content`'s bit 0 — magery 1, necromancy 101,
/// chivalry 201, bushido 401, ninjitsu 501, spellweaving 601, mysticism 678.
/// `content` is a 64-bit mask; bit N set iff spell `offset + N` is in the book.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpellbookContent {
    pub graphic: u16,
    pub offset: u16,
    pub content: u64,
}

/// A treasure/decoration map item's window (0x90 DisplayMap / 0xF5
/// DisplayMapNew + 0x56 MapCommand — ServUO `Scripts/Items/Tools/MapItem.cs`,
/// cross-checked against ClassicUO `PacketHandlers.DisplayMap`/`MapData` and
/// `Game/UI/Gumps/MapGump.cs`). Keyed by the map item's own serial in
/// [`World::map_gumps`].
///
/// `gump_art` is a constant `0x139D` at every real ServUO call site (the
/// blank aged-parchment map background — ClassicUO's `MapGump` only ever
/// uses this id for a small decorative corner icon and instead renders a
/// custom terrain-snippet texture for the real background, but we don't have
/// that asset pipeline; the renderer stretches the plain `0x139D` gump art to
/// `width`×`height` instead, which needs no further pin rescale — see below).
/// `facet` mirrors [`World::map_index`]'s encoding (0=Felucca, 1=Trammel,
/// 2=Ilshenar, 3=Malas, 4=Tokuno, 5=TerMur); a legacy 0x90 carries no facet at
/// all, so it defaults to 0 (Felucca) for one.
///
/// `min_x`/`min_y`/`max_x`/`max_y` are the WORLD tile-coordinate bounds this
/// map covers (`MapItem.Bounds`); `width`/`height` are the RENDERED art size
/// in pixels. Critically, [`MapView::pins`] are already expressed in that
/// same `width`×`height` PIXEL space, not world tiles — ServUO's
/// `MapItem.ConvertToWorld`/`ConvertToMap` do the bounds↔pixel conversion
/// server-side before a pin ever hits the wire, and ClassicUO's own
/// `MapGump.PinControl` positions a pin at its raw wire `(x, y)` with no
/// client-side rescale — so a renderer draws each pin straight onto the
/// `width`×`height` art, no math needed (only rescale if you choose to
/// display the background at something OTHER than its native `width`×
/// `height`, e.g. stretching gump art `0x139D` to fill that same box already
/// accounts for it).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MapView {
    /// Monotonic tag bumped every 0x90/0xF5 for ANY map serial (shared ring
    /// like [`World::container_open_seq`], not a per-serial counter) — ServUO
    /// resends full `MapDetails`/`NewMapDetails` on EVERY double-click/decode
    /// (`MapItem.OnDoubleClick`/`TreasureMap.Decode` always call `DisplayTo`),
    /// even for byte-identical content, and real UO reopens the window every
    /// time. The renderer must treat each `open_seq` as its own "please open"
    /// request rather than deduping purely on `serial` (mirrors
    /// [`Paperdoll::seq`]).
    pub open_seq: u64,
    pub gump_art: u16,
    pub facet: u8,
    pub min_x: u16,
    pub min_y: u16,
    pub max_x: u16,
    pub max_y: u16,
    pub width: u16,
    pub height: u16,
    /// Pins in `width`×`height` pixel space, in server order. Index 0 is the
    /// treasure/chest pin on a decoded treasure map — ServUO's `MapItem.
    /// RemovePin` refuses to remove index 0 (see [`World::apply_map_command`]),
    /// so a renderer may want to draw it distinctly from player-added pins.
    pub pins: Vec<(u16, u16)>,
    /// Whether the player may currently edit pins (0x56 command 7
    /// MapSetEditable — ServUO `MapItem.ValidateEdit`/`OnToggleEditable`).
    pub editable: bool,
}

/// An outstanding target cursor the server is waiting on (from a 0x6C request).
/// The brain answers it with `Action::TargetObject`/`TargetGround`; the response
/// must echo `cursor_id`, `cursor_flag`, and `target_type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetCursor {
    /// 0 = object/serial target, 1 = ground/location target.
    pub target_type: u8,
    /// Cursor id assigned by the server; echoed back in the response.
    pub cursor_id: u32,
    /// 0 neutral, 1 harmful, 2 helpful. (3 = cancel; never stored — it clears.)
    pub cursor_flag: u8,
}

/// A server-initiated paperdoll open/refresh (0x88 DisplayPaperdoll) — sent
/// whenever we double-click a mobile, ours or another's (ServUO
/// `Scripts/Misc/Paperdoll.cs`, off `Mobile.OnDoubleClick`). `title` is the
/// server-precomputed name+title line (`Titles.ComputeTitle`, e.g. "Anima the
/// Adventurer") — plain text, no cliloc to resolve. `warmode` mirrors the
/// target's combat stance; `can_lift` is whether WE'RE allowed to lift/equip
/// items on this doll (`Mobile.AllowEquipFrom` — true for our own paperdoll,
/// false for a stranger's). `seq` is a monotonic tag (like [`Effect::seq`]):
/// ServUO resends this on EVERY double-click, even re-clicking the same
/// `serial` after we've closed its window, and real UO reopens it every time
/// — so the renderer must treat each `seq` as its own "please open" request
/// rather than deduping purely on `serial`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Paperdoll {
    pub seq: u64,
    pub serial: u32,
    pub title: String,
    pub warmode: bool,
    pub can_lift: bool,
}

/// An outstanding server text prompt (from a 0xC2 UnicodePrompt request) — the
/// mechanism behind ~38 ServUO flows (pet rename, house sign, guild abbreviation,
/// …). The actual question text is *not* carried on this packet (ServUO sends it
/// separately as a cliloc/system message just before opening the prompt — it
/// already lands in [`World::journal`]); this only carries the two ids the
/// response must echo. Cleared when we answer (see
/// [`crate::agent::Action::PromptResponse`]/[`crate::agent::Action::PromptCancel`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptState {
    /// The prompt's sender serial (usually our own) — echoed back verbatim.
    pub sender_serial: u32,
    /// Opaque id identifying which `Prompt` subclass the server is waiting on
    /// (ServUO `Prompt.TypeId`); echoed back verbatim so it can resume the right one.
    pub prompt_id: u32,
}

/// The player's party (0xBF/0x06). `members` are the current member serials in
/// server order — `members[0]` is the leader (`leader` mirrors it for convenience).
/// `pending_invite` is the serial of a party leader who invited us (sub-sub 0x07)
/// and is awaiting our accept/decline; cleared once we join or decline. An empty
/// `members` means we are not in a party. Member names/hits are *not* stored here —
/// they are resolved from the [`Mobile`]s in view when building a scene.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Party {
    pub members: Vec<u32>,
    pub leader: u32,
    pub pending_invite: Option<u32>,
}

/// An active secure trade session (player-to-player trade window, 0x6F).
/// Trading is peer-initiated with no consent required (dropping an item on a
/// player opens a trade with them, ServUO `Mobile.OpenTrade`/`OnDragDrop`,
/// `Mobile.cs` ~10830), and ServUO tracks trades as a `List<SecureTrade>` per
/// client (`NetState.Trades`) — nothing stops two DIFFERENT strangers from
/// each opening a session with us at once. So [`World::trades`] is a `Vec`,
/// one entry per opponent, not a single slot; see [`World::open_trade`]/
/// [`World::trade_mut`]/[`World::close_trade`] for how sessions are
/// found/updated/removed instead of assigning the field directly.
///
/// Each side of the trade is backed by an in-world container item — its
/// contents arrive over the ordinary 0x25 AddToContainer / 0x3C
/// ContainerContent path (ServUO's `SecureTradeEquip` packet literally *is*
/// 0x25 with the same layout; see [`crate::net::game::secure_trade`]'s doc) —
/// nothing filters them out, so [`World::items`] already has both sides'
/// items keyed by `container == my_container` / `container ==
/// their_container`, exactly like a normal backpack. `my_container` is the key
/// every wire exchange addresses: ServUO always sends US our OWN side's
/// container serial on every action (`SecureTrade.Close`/`Update` send
/// `m_From.Container` to `m_From.Mobile`, never the opponent's), and
/// ClassicUO's `TradingGump` only ever sends its own `ID1` outbound, never the
/// opponent's `ID2` (`Game/UI/Gumps/TradingGump.cs`) — so it's also what every
/// outgoing action (cancel/accept/gold) addresses.
///
/// Gold/platinum come in three independent flavors (ClassicUO
/// `TradingGump.Gold`/`.Platinum` vs `.HisGold`/`.HisPlatinum` vs the local
/// entry variable its text field sends from — `TradingGump.OnTextChanged`):
/// - `my_offer_gold`/`my_offer_platinum` — what *we've* put up. The server
///   never echoes our own offer back to us as such, so this is tracked
///   optimistically the moment we send [`crate::agent::Action::TradeGold`]
///   (mirrors the `SkillLock` action's optimistic local update) — it's our
///   only record of it.
/// - `their_offer_gold`/`their_offer_platinum` — the OPPONENT's offer, pushed
///   to us as 0x6F action `3` UpdateGold (ClassicUO `HisGold`/`HisPlatinum`).
/// - `balance_gold`/`balance_platinum` — OUR account's total available
///   currency (ServUO `Account.TotalGold`/`TotalPlat`), pushed to us as action
///   `4` UpdateLedger. This is an input CAP for our own offer, not a trade
///   amount at all (ClassicUO clamps `my_gold_entry` to `Gold` before ever
///   sending it) — the renderer should show it next to the entry field and
///   clamp client-side, same as the reference client.
///
/// This whole AOS/TOL "virtual gold" feature only activates once both the
/// client version and the server's `AccountGold.Enabled` (ServUO gates it on
/// `Core.TOL`) agree; on a server/client pair that never negotiates it, all
/// three gold flavors simply stay 0 and only items change hands.
///
/// Removed from [`World::trades`] when the session closes (0x6F action 1) —
/// cancelled by either side or completed (both accepted) — which also purges
/// its leftover container contents; see [`World::close_trade`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TradeState {
    pub opponent_serial: u32,
    pub opponent_name: String,
    pub my_container: u32,
    pub their_container: u32,
    pub my_accept: bool,
    pub their_accept: bool,
    pub my_offer_gold: u32,
    pub my_offer_platinum: u32,
    pub their_offer_gold: u32,
    pub their_offer_platinum: u32,
    pub balance_gold: u32,
    pub balance_platinum: u32,
}

/// The whole observable game state.
#[derive(Debug, Default)]
pub struct World {
    /// Our own character's serial, once we've entered the world.
    pub player: Option<Serial>,
    /// Server advertised the AOS expansion via SupportedFeatures `0xB9` during
    /// login. Gates AOS-only UI (e.g. the weapon special-ability bar). T2A = false.
    pub aos: bool,
    pub player_stats: PlayerStats,
    pub mobiles: HashMap<u32, Mobile>,
    pub items: HashMap<u32, Item>,
    /// Chat / system message log (journal), newest last.
    pub journal: Vec<JournalEntry>,
    /// Server-issued fast-walk prevention keys (FIFO), consumed one per step.
    pub fast_walk: Vec<u32>,
    /// An outstanding target cursor (0x6C), if the server is waiting on one.
    pub pending_target: Option<TargetCursor>,
    /// Our skills by id (0x3A), values in tenths. See [`Skill`].
    pub skills: HashMap<u16, Skill>,
    /// Recent sound-effect events (0x54), each `(seq, sound_id)`, newest last,
    /// capped to the most recent few. The renderer plays only events with a
    /// `seq` it hasn't played yet (like the journal's `seq`).
    pub recent_sounds: Vec<(u64, u16, u16, u16)>,
    /// Monotonic counter assigning each sound event a unique `seq`.
    pub sound_seq: u64,
    /// Recent damage events (0x0B), each `(seq, serial, amount)`, newest last,
    /// capped to the most recent few. `serial` took `amount` HP. The renderer
    /// floats each `seq` it hasn't shown yet (like sounds/journal).
    pub recent_damage: Vec<(u64, u32, u16)>,
    /// Monotonic counter assigning each damage event a unique `seq`.
    pub damage_seq: u64,
    /// Recent character-animation events (0x6E): `(seq, serial, action, frame_count,
    /// forward, delay)`. `serial` should play animation group `action` once (combat
    /// swings, bows, get-hit, …). The renderer plays each `seq` it hasn't yet.
    pub recent_anims: Vec<(u64, u32, u16, u16, bool, u8)>,
    /// Monotonic counter assigning each animation event a unique `seq`.
    pub anim_seq: u64,
    /// Recent *typed* animation events (0xE2): `(seq, serial, kind, action, mode)`.
    /// `kind` is the wire `AnimationType` (0-15: Attack/Parry/.../Spawn — see
    /// [`crate::net::game`]'s `typed_anim`), not a raw group like 0x6E's `action`.
    /// `mode` is the wire "delay" byte, repurposed by the renderer only to pick a
    /// cosmetic variant. The renderer resolves the real per-body animation group
    /// (ClassicUO `GetObjectNewAnimation`) before playing it.
    pub recent_typed_anims: Vec<(u64, u32, u16, u16, u8)>,
    /// Monotonic counter assigning each typed-animation event a unique `seq`.
    pub typed_anim_seq: u64,
    /// The current background music track id (0x6D), or `None` if stopped.
    pub current_music: Option<u16>,
    /// Overall light level (0x4F): 0 = brightest day, ~0x1F darkest night.
    pub light_level: u8,
    /// The player's personal light level (0x4E), if the server has sent one.
    /// Combined with `light_level` via [`World::effective_light`].
    pub personal_light: Option<u8>,
    /// Current weather (0x65). See [`Weather`].
    pub weather: Weather,
    /// Current season (0xBC): 0=Spring, 1=Summer, 2=Fall, 3=Winter, 4=Desolation.
    /// Defaults to 0 (Spring). The renderer may tint the scene to match; we do not
    /// remap tree/foliage graphics (that's a much larger change).
    pub season: u8,
    /// The player's active buffs/debuffs (0xDF), keyed by `icon`. See [`Buff`].
    pub buffs: Vec<Buff>,
    /// The current vendor BUY window (0x74), if one is open. See [`ShopBuy`].
    pub shop_buy: Option<ShopBuy>,
    /// The current vendor SELL window (0x9E), if one is open. See [`ShopSell`].
    pub shop_sell: Option<ShopSell>,
    /// Open server-sent gumps/dialogs (0xB0/0xDD), keyed by serial. See [`Gump`].
    pub gumps: Vec<Gump>,
    /// The current context (popup) menu (0xBF/0x14), if one is open. See [`PopupMenu`].
    pub popup: Option<PopupMenu>,
    /// The currently open book (0x93/0xD4 header + 0x66 page content), or `None`.
    /// See [`Book`].
    pub book: Option<Book>,
    /// Spellbook contents we've been told about (0xBF/0x1B), keyed by the book
    /// item's serial. Only ever populated for a book that's actually been
    /// opened this session (see [`SpellbookContent`]'s doc) — the K-key
    /// spellbook UI dims spells for a school only when it has an entry here,
    /// and otherwise renders that school as if every spell were owned (same
    /// as before this field existed). Pruned like [`World::opl`]: dropped on
    /// delete ([`World::remove`]) and on a facet purge ([`World::on_map_change`]).
    /// Set via [`World::set_spellbook_content`].
    pub spellbooks: HashMap<u32, SpellbookContent>,
    /// An on-screen quest arrow (0xBA) pointing at tile `(x, y)`, or `None` when
    /// hidden. The renderer draws an arrow toward this tile.
    pub quest_arrow: Option<(u16, u16)>,
    /// Recent graphical-effect events (0x70/0xC0/0xC7), newest last, capped to the
    /// most recent few. The renderer spawns a visual for each `seq` it hasn't seen
    /// yet (like sounds/damage). See [`Effect`].
    pub recent_effects: Vec<Effect>,
    /// Monotonic counter assigning each effect event a unique `seq`.
    pub effect_seq: u64,
    /// Object Property Lists (OPL / tooltips), keyed by serial (0xD6 MegaCliloc).
    /// Each entry is the raw property lines `(cliloc id, tab-separated args)` in the
    /// order the server sent them (line 0 is the name, the rest are magical mods).
    /// Stored raw because the core has no Cliloc table; the renderer/scene resolves
    /// `cliloc.format(id, args)` for display. See [`World::set_opl`].
    pub opl: HashMap<u32, Vec<(u32, String)>>,
    /// The OPL revision hash last seen per serial (0xD6 header / 0xDC OPLInfo).
    /// Lets a consumer detect a stale tooltip and re-request; not acted on in core.
    pub opl_revision: HashMap<u32, u32>,
    /// The player's party state (0xBF/0x06). See [`Party`].
    pub party: Party,
    /// Whether the player is in war mode (combat stance). Authoritatively set by
    /// the server's 0x72 SetWarMode echo (see [`crate::net::game`]); the renderer
    /// reflects it and the war-mode toggle reads it.
    pub war: bool,
    /// The serial last sent in an Attack (0x05) — UO's "last target" for the
    /// auto-attack / attack-last combat loop. `None` until the player attacks.
    pub last_attack: Option<u32>,
    /// The server's authoritative current combat opponent (0xAA ChangeCombatant,
    /// `Mobile.Combatant` — Mobile.cs ~2213), distinct from [`World::last_attack`]
    /// (which is just the last serial *we* sent an Attack request for): the server
    /// can also change combatant on its own (e.g. it retargets who's actually
    /// swinging at us). `None` when combat has ended (wire serial 0).
    pub combatant: Option<u32>,
    /// Maps a corpse item's serial to the mobile that died to create it (0xAF
    /// DisplayDeath). AI-facing only ("is this the corpse of what I killed") — no
    /// renderer change needed (a corpse already carries its own body/hue/direction).
    /// Pruned when the corpse item is removed (0x1D → [`World::remove`]) and capped
    /// defensively by [`World::set_corpse_of`] (a delete we somehow missed must
    /// never pin this map's growth for the rest of the session).
    pub corpse_of: HashMap<u32, u32>,
    /// A corpse's worn-item layout (0x89 CorpseEquip): `(layer, item serial)` pairs,
    /// keyed by corpse serial. `layer` is the real (un-shifted) wear layer — the
    /// wire format sends `layer + 1` with a `0` terminator (ServUO
    /// `Scripts/Items/Corpses/Packets.cs` `CorpseEquip`); we undo that shift so it
    /// matches [`Item::layer`]'s convention everywhere else. No renderer change
    /// needed — the loot window already lists a corpse's contents flatly via 0x3C.
    /// Pruned/capped the same way as [`World::corpse_of`].
    pub corpse_equip: HashMap<u32, Vec<(u8, u32)>>,
    /// Recent lift-rejection events (0x27 LiftRej), each `(seq, reason)`, newest
    /// last, capped to the most recent few (like `recent_damage`/`recent_sounds`).
    /// The server refused our last pickup (0x07) — the item never left its source.
    /// `reason` (ClassicUO `ServerErrorMessages`, indexed by this packet id; any
    /// code 5 or higher reads the same message as 4): 0 CannotLift, 1 OutOfRange,
    /// 2 OutOfSight, 3 BelongsToAnother, 4 AlreadyHolding, 5 generic/Inspecific.
    /// The renderer clears the drag-ghost (without sending a drop — nothing ever
    /// moved) and shows the reason as a system journal line for each `seq` it
    /// hasn't shown yet.
    pub recent_lift_rejects: Vec<(u64, u8)>,
    /// Monotonic counter assigning each lift-rejection event a unique `seq`.
    pub lift_reject_seq: u64,
    /// An outstanding server text prompt (0xC2 UnicodePrompt), if one is pending.
    /// See [`PromptState`].
    pub prompt: Option<PromptState>,
    /// The latest server-initiated paperdoll open/refresh (0x88), if any has
    /// arrived this session. See [`Paperdoll`] — note its `seq`, not just
    /// `serial`, is what tells a renderer this is a fresh open request.
    pub paperdoll: Option<Paperdoll>,
    /// Monotonic counter assigning each [`World::paperdoll`] update a unique `seq`.
    pub paperdoll_seq: u64,
    /// Recent 0x24 (DrawContainer/ContainerDisplay(HS)) events: each `(seq,
    /// serial, gump_id)`, newest last, capped like `recent_lift_rejects`. This
    /// is deliberately a **raw, unfiltered data log** — every `gump_id` ServUO
    /// ever sends on 0x24 gets recorded, per D3 (core = data; renderer =
    /// policy). That matters because ServUO reuses 0x24 for two things that
    /// are NOT a container window (see [`crate::net::game::draw_container`]'s
    /// doc for the exact ServUO/ClassicUO cites): `DisplayBuyList` (vendor
    /// "Buy" window, `gump_id` 0x30, `serial` = the vendor MOBILE) and
    /// `DisplaySpellbook` (`gump_id` 0xFFFF, `serial` = the spellbook ITEM).
    /// Deciding those two shouldn't pop a generic container window is a
    /// renderer/UI call, not a `World` one — `anima_net::scene`'s bridge to
    /// the web client is what filters them out of the "open a window" signal
    /// it emits; a future consumer of this same ring is free to make a
    /// different call. Fires for a container we did NOT ourselves
    /// double-click — a banker's "bank" speech, a GM `[bank`, a snoop menu
    /// pick, … — the ordinary client-initiated open (our own double-click)
    /// already opens its window locally and doesn't need this.
    pub recent_container_opens: Vec<(u64, u32, u16)>,
    /// Monotonic counter assigning each container-open event a unique `seq`.
    pub container_open_seq: u64,
    /// Recent Swing events (0x2F): each `(seq, attacker, defender)`, newest
    /// last, capped like `recent_lift_rejects`. ServUO only ever sends this to
    /// the ATTACKING player's own client (`attacker.Send(...)` — an NPC
    /// attacker has no `NetState` to receive it), so `attacker` is normally our
    /// own serial; carried generically anyway since nothing about the wire
    /// format assumes that. Purely cosmetic feedback (face the defender) — no
    /// gameplay state depends on it.
    pub recent_swings: Vec<(u64, u32, u32)>,
    /// Monotonic counter assigning each swing event a unique `seq`.
    pub swing_seq: u64,
    /// Current facet/map index (0xBF/0x08 MapChange): 0=Felucca, 1=Trammel,
    /// 2=Ilshenar, 3=Malas, 4=Tokuno, 5=TerMur (ServUO `Map.MapID`). The play server
    /// watches this and reloads `anima_assets::MapData` for the matching facet via
    /// `MapData::open_facet` (per-facet `map{N}`/`staidx{N}`/`statics{N}` files +
    /// ClassicUO `MapsDefaultSize` dimensions: Felucca/Trammel 7168×4096, Ilshenar
    /// 2304×1600, Malas 2560×2048, Tokuno 1448×1448, TerMur 1280×4096), so
    /// terrain/statics follow the player across facets. (The facet-0 world-map
    /// overlay in `anima_net::scene::render_worldmap` still uses the fixed
    /// `MAP_WIDTH`/`MAP_HEIGHT` consts — it's only ever rendered for Felucca.) Set
    /// via [`World::on_map_change`] (never assign directly — that's what purges the
    /// old facet's entities).
    pub map_index: u8,
    /// Active player-to-player secure trade sessions (0x6F) — normally 0 or 1,
    /// but see [`TradeState`]'s doc for why concurrent sessions with
    /// different opponents are possible. Use [`World::open_trade`]/
    /// [`World::trade_mut`]/[`World::close_trade`] rather than indexing
    /// directly.
    pub trades: Vec<TradeState>,
    /// Open treasure/decoration map windows (0x90/0xF5 + 0x56), keyed by the
    /// map item's own serial. See [`MapView`]. Pruned like [`World::spellbooks`]:
    /// dropped on delete ([`World::remove`]) and on a facet purge
    /// ([`World::on_map_change`]). Set via [`World::set_map_view`]/
    /// [`World::apply_map_command`] — never insert directly (that's what
    /// assigns a correctly-ordered [`MapView::open_seq`]).
    pub map_gumps: HashMap<u32, MapView>,
    /// Monotonic counter assigning each [`World::set_map_view`] call (any
    /// serial) its [`MapView::open_seq`] — a single shared ring, like
    /// [`World::container_open_seq`], not one counter per map.
    pub map_open_seq: u64,
}

/// Notoriety values treated as hostile for auto-attack selection:
/// 3 = gray, 4 = criminal, 5 = enemy (orange), 6 = murderer (red).
fn is_hostile_noto(noto: u8) -> bool {
    matches!(noto, 3..=6)
}

/// How many recent sound events [`World::push_sound`] keeps.
const MAX_RECENT_ANIMS: usize = 16;
const MAX_RECENT_SOUNDS: usize = 16;
/// How many recent damage events [`World::push_damage`] keeps.
const MAX_RECENT_DAMAGE: usize = 16;
/// How many recent effect events [`World::push_effect`] keeps.
const MAX_RECENT_EFFECTS: usize = 32;
/// How many recent lift-rejection events [`World::push_lift_reject`] keeps.
const MAX_RECENT_LIFT_REJECTS: usize = 16;
/// How many recent container-open events [`World::push_container_open`] keeps.
const MAX_RECENT_CONTAINER_OPENS: usize = 16;
/// How many recent swing events [`World::push_swing`] keeps.
const MAX_RECENT_SWINGS: usize = 16;
/// Defensive cap on [`World::corpse_of`]/[`World::corpse_equip`] — both are pruned
/// on delete (0x1D), so this only guards against a delete we somehow missed
/// pinning the map's growth for the rest of a long session.
const MAX_CORPSE_LINKS: usize = 256;

impl World {
    pub fn new() -> Self {
        Self::default()
    }

    /// Our own character, if we've entered the world and it's known.
    pub fn player_mobile(&self) -> Option<&Mobile> {
        self.player.and_then(|s| self.mobiles.get(&s.0))
    }

    pub fn player_mobile_mut(&mut self) -> Option<&mut Mobile> {
        match self.player {
            Some(s) => self.mobiles.get_mut(&s.0),
            None => None,
        }
    }

    /// Whether the player is mounted — an item on the Mount layer (0x19) with a
    /// real graphic. Mounted halves the step time (ClassicUO `Mobile.IsMounted`).
    pub fn player_mounted(&self) -> bool {
        match self.player {
            Some(s) => self
                .items
                .values()
                .any(|it| it.container == Some(s.0) && it.layer == 0x19 && it.graphic != 0),
            None => false,
        }
    }

    /// Set the player and seed its mobile from a completed login.
    pub fn enter_world(&mut self, r: &LoginResult) {
        self.player = Some(Serial(r.serial));
        self.aos = r.aos;
        let m = self.mobiles.entry(r.serial).or_default();
        m.serial = r.serial;
        m.body = r.body;
        m.pos = Position {
            x: r.x,
            y: r.y,
            z: r.z,
        };
        m.direction = r.direction;
    }

    pub fn is_player(&self, serial: u32) -> bool {
        self.player == Some(Serial(serial))
    }

    /// Is `serial` a currently in-view, hostile mobile (and not ourself)? A mobile
    /// is "in view" iff it's in [`World::mobiles`]; hostile per [`is_hostile_noto`].
    pub fn is_hostile_mobile(&self, serial: u32) -> bool {
        !self.is_player(serial)
            && self
                .mobiles
                .get(&serial)
                .is_some_and(|m| is_hostile_noto(m.notoriety))
    }

    /// Pick the best auto-attack target (UO "last target" combat loop): the current
    /// [`World::last_attack`] if it's still a live in-view hostile mobile, otherwise
    /// the NEAREST in-view hostile mobile (Chebyshev distance to the player).
    /// `None` if no hostile mobile is in view. In-view only; filters notoriety to
    /// {gray, criminal, enemy, murderer}.
    pub fn auto_attack_target(&self) -> Option<u32> {
        if let Some(prev) = self.last_attack {
            if self.is_hostile_mobile(prev) {
                return Some(prev);
            }
        }
        let p = self.player_mobile()?.pos;
        self.mobiles
            .values()
            .filter(|m| !self.is_player(m.serial) && is_hostile_noto(m.notoriety))
            .min_by_key(|m| (m.pos.x.abs_diff(p.x)).max(m.pos.y.abs_diff(p.y)))
            .map(|m| m.serial)
    }

    /// Get-or-create a mobile by serial.
    pub fn mobile_mut(&mut self, serial: u32) -> &mut Mobile {
        let m = self.mobiles.entry(serial).or_default();
        m.serial = serial;
        m
    }

    /// Get-or-create an item by serial.
    pub fn item_mut(&mut self, serial: u32) -> &mut Item {
        let it = self.items.entry(serial).or_default();
        it.serial = serial;
        it
    }

    /// Record a sound-effect event (0x54). Assigns the next monotonic `seq` and
    /// keeps only the most recent [`MAX_RECENT_SOUNDS`].
    pub fn push_sound(&mut self, sound_id: u16, x: u16, y: u16) {
        self.sound_seq += 1;
        self.recent_sounds.push((self.sound_seq, sound_id, x, y));
        let overflow = self.recent_sounds.len().saturating_sub(MAX_RECENT_SOUNDS);
        if overflow > 0 {
            self.recent_sounds.drain(0..overflow);
        }
    }

    /// Record a character-animation event (0x6E): `serial` should play animation
    /// `action` once. Assigns the next monotonic `seq`; keeps the most recent
    /// [`MAX_RECENT_ANIMS`].
    pub fn push_anim(&mut self, serial: u32, action: u16, frames: u16, forward: bool, delay: u8) {
        self.anim_seq += 1;
        self.recent_anims
            .push((self.anim_seq, serial, action, frames, forward, delay));
        let overflow = self.recent_anims.len().saturating_sub(MAX_RECENT_ANIMS);
        if overflow > 0 {
            self.recent_anims.drain(0..overflow);
        }
    }

    /// Record a typed-animation event (0xE2): `serial` was told to play
    /// `AnimationType` `kind`'s `action` (an emote, gesture, alert, …). Assigns the
    /// next monotonic `seq` and keeps only the most recent [`MAX_RECENT_ANIMS`].
    pub fn push_typed_anim(&mut self, serial: u32, kind: u16, action: u16, mode: u8) {
        self.typed_anim_seq += 1;
        self.recent_typed_anims
            .push((self.typed_anim_seq, serial, kind, action, mode));
        let overflow = self.recent_typed_anims.len().saturating_sub(MAX_RECENT_ANIMS);
        if overflow > 0 {
            self.recent_typed_anims.drain(0..overflow);
        }
    }

    /// Record a damage event (0x0B): `serial` took `amount` HP. Assigns the next
    /// monotonic `seq` and keeps only the most recent [`MAX_RECENT_DAMAGE`].
    pub fn push_damage(&mut self, serial: u32, amount: u16) {
        self.damage_seq += 1;
        self.recent_damage.push((self.damage_seq, serial, amount));
        let overflow = self.recent_damage.len().saturating_sub(MAX_RECENT_DAMAGE);
        if overflow > 0 {
            self.recent_damage.drain(0..overflow);
        }
    }

    /// Record a graphical-effect event (0x70/0xC0/0xC7). Assigns the next monotonic
    /// `seq` (the caller may leave `effect.seq` at 0) and keeps only the most recent
    /// [`MAX_RECENT_EFFECTS`].
    pub fn push_effect(&mut self, mut effect: Effect) {
        self.effect_seq += 1;
        effect.seq = self.effect_seq;
        self.recent_effects.push(effect);
        let overflow = self.recent_effects.len().saturating_sub(MAX_RECENT_EFFECTS);
        if overflow > 0 {
            self.recent_effects.drain(0..overflow);
        }
    }

    /// Record a lift-rejection event (0x27 LiftRej): the server refused our last
    /// pickup with the given `reason` byte. Assigns the next monotonic `seq` and
    /// keeps only the most recent [`MAX_RECENT_LIFT_REJECTS`].
    pub fn push_lift_reject(&mut self, reason: u8) {
        self.lift_reject_seq += 1;
        self.recent_lift_rejects.push((self.lift_reject_seq, reason));
        let overflow = self.recent_lift_rejects.len().saturating_sub(MAX_RECENT_LIFT_REJECTS);
        if overflow > 0 {
            self.recent_lift_rejects.drain(0..overflow);
        }
    }

    /// Record a 0x24 (DrawContainer/ContainerDisplay) event verbatim: the
    /// server sent `gump_id` for `serial`, unfiltered — see
    /// [`World::recent_container_opens`]'s doc for why this stays a raw data
    /// log (including the non-container `gump_id`s 0x30/0xFFFF) rather than
    /// deciding here whether it's "really" a container open. Assigns the next
    /// monotonic `seq` and keeps only the most recent
    /// [`MAX_RECENT_CONTAINER_OPENS`].
    pub fn push_container_open(&mut self, serial: u32, gump_id: u16) {
        self.container_open_seq += 1;
        self.recent_container_opens.push((self.container_open_seq, serial, gump_id));
        let overflow = self.recent_container_opens.len().saturating_sub(MAX_RECENT_CONTAINER_OPENS);
        if overflow > 0 {
            self.recent_container_opens.drain(0..overflow);
        }
    }

    /// Record a Swing event (0x2F): `attacker` just swung at `defender`.
    /// Assigns the next monotonic `seq` and keeps only the most recent
    /// [`MAX_RECENT_SWINGS`].
    pub fn push_swing(&mut self, attacker: u32, defender: u32) {
        self.swing_seq += 1;
        self.recent_swings.push((self.swing_seq, attacker, defender));
        let overflow = self.recent_swings.len().saturating_sub(MAX_RECENT_SWINGS);
        if overflow > 0 {
            self.recent_swings.drain(0..overflow);
        }
    }

    /// Record a server-initiated paperdoll open/refresh (0x88 DisplayPaperdoll).
    /// Assigns the next monotonic `seq` (see [`Paperdoll::seq`]'s doc for why a
    /// repeat of the same `serial` still needs a fresh one) and replaces any
    /// prior state.
    pub fn set_paperdoll(&mut self, serial: u32, title: String, warmode: bool, can_lift: bool) {
        self.paperdoll_seq += 1;
        self.paperdoll = Some(Paperdoll { seq: self.paperdoll_seq, serial, title, warmode, can_lift });
    }

    /// Push a client-synthesized "System" journal line — no packet caused this; it's
    /// the client informing the player of something the server never says itself
    /// (e.g. a WalkTo the local pathfinder rejected). Reuses the exact mechanism a
    /// real 0x1C/0xC1 system message uses (see `push_journal_cliloc` in
    /// net/game.rs), so it renders identically in the client. Deliberate, narrow
    /// exception to "packet handlers mutate World" — this is UX feedback about a
    /// purely local (client-side) decision, not gameplay state.
    pub fn push_system_note(&mut self, text: impl Into<String>) {
        self.journal.push(JournalEntry {
            serial: 0,
            name: "System".to_string(),
            text: text.into(),
            msg_type: 0,
            hue: 0,
            cliloc: 0,
        });
    }

    /// Record a corpse→killed-mobile link (0xAF DisplayDeath). Upserts by
    /// `corpse_serial`; defensively evicts an arbitrary entry (not LRU — this is a
    /// rare safety net, not a hot path) once at [`MAX_CORPSE_LINKS`], since a
    /// missed 0x1D delete would otherwise pin this map's growth forever.
    pub fn set_corpse_of(&mut self, corpse_serial: u32, killed_serial: u32) {
        if self.corpse_of.len() >= MAX_CORPSE_LINKS && !self.corpse_of.contains_key(&corpse_serial) {
            if let Some(&k) = self.corpse_of.keys().next() {
                self.corpse_of.remove(&k);
            }
        }
        self.corpse_of.insert(corpse_serial, killed_serial);
    }

    /// Record a corpse's worn-item layout (0x89 CorpseEquip). Replaces any prior
    /// entries for `corpse` (the server sends the full list each time); capped the
    /// same defensive way as [`World::set_corpse_of`].
    pub fn set_corpse_equip(&mut self, corpse: u32, entries: Vec<(u8, u32)>) {
        if self.corpse_equip.len() >= MAX_CORPSE_LINKS && !self.corpse_equip.contains_key(&corpse) {
            if let Some(&k) = self.corpse_equip.keys().next() {
                self.corpse_equip.remove(&k);
            }
        }
        self.corpse_equip.insert(corpse, entries);
    }

    /// The light level the renderer should use: the brighter (lower) of the
    /// overall level and the player's personal light, when a personal light is
    /// active. Lower = brighter, so `min` picks the brighter of the two.
    pub fn effective_light(&self) -> u8 {
        match self.personal_light {
            Some(p) => self.light_level.min(p),
            None => self.light_level,
        }
    }

    /// Add or refresh a buff icon (0xDF action=add). Upserts by `icon`.
    pub fn add_buff(&mut self, icon: u16, name: String, dur: u32) {
        if let Some(b) = self.buffs.iter_mut().find(|b| b.icon == icon) {
            b.name = name;
            b.dur = dur;
        } else {
            self.buffs.push(Buff { icon, name, dur });
        }
    }

    /// Drop a buff icon (0xDF action=remove). No-op if not present.
    pub fn remove_buff(&mut self, icon: u16) {
        self.buffs.retain(|b| b.icon != icon);
    }

    /// Add or replace a gump (0xB0/0xDD). Upserts by `serial` so a re-sent gump
    /// (same serial) refreshes in place rather than stacking duplicates.
    pub fn add_gump(&mut self, gump: Gump) {
        if let Some(g) = self.gumps.iter_mut().find(|g| g.serial == gump.serial) {
            *g = gump;
        } else {
            self.gumps.push(gump);
        }
    }

    /// Drop a gump by serial (the player answered/closed it). No-op if absent.
    pub fn close_gump(&mut self, serial: u32) {
        self.gumps.retain(|g| g.serial != serial);
    }

    /// Close every open gump of a given KIND (0xBF/0x04 CloseGump). ServUO
    /// addresses this by `Gump.TypeID` — a hash of the C# gump class — which is
    /// the SAME value the ordinary open packet (0xB0/0xDD) calls `gumpId`, i.e.
    /// this matches [`Gump::gump_id`], not [`Gump::serial`] (one specific open
    /// instance). Real call sites (`Mobile.CloseGump`, `BaseGump.Refresh`/
    /// `.Cancel`) only ever have one gump of a kind open at a time, but nothing
    /// stops more from accumulating, so this drops every matching one — mirrors
    /// ClassicUO's own handler, which walks every open gump and disposes any
    /// whose `ServerSerial` (its name for this same TypeID value) matches.
    pub fn close_gump_by_type(&mut self, type_id: u32) {
        self.gumps.retain(|g| g.gump_id != type_id);
    }

    /// Drop the vendor SELL window (0x9E), if any. The window is consumed
    /// once we answer it (or abandon it) — clearing it locally keeps a stale
    /// list from being answered a second time by a later, unrelated sale
    /// (its serials no longer refer to what's actually in the pack). Mirrors
    /// [`World::close_gump`]; see [`crate::agent::Action::SellItems`].
    pub fn close_shop_sell(&mut self) {
        self.shop_sell = None;
    }

    /// Drop the vendor BUY window (0x74), if any. Mirrors [`World::close_shop_sell`];
    /// see [`crate::net::game::end_vendor`] (0x3B EndVendorBuy/EndVendorSell), which
    /// is the actual server-driven close for this window (unlike the sell side,
    /// nothing locally/optimistically cleared this before — see DESIGN history).
    pub fn close_shop_buy(&mut self) {
        self.shop_buy = None;
    }

    /// Open (or refresh) a secure trade session (0x6F action 0 Display).
    /// Upserts by `opponent_serial` — ServUO allows only one open trade per
    /// mobile pair (`NetState.FindTradeContainer`/`AddTrade`, `Mobile.OpenTrade`
    /// reuses the existing container instead of starting a second one), so a
    /// repeat Display for the same opponent replaces rather than duplicates.
    /// A *different* opponent is a genuinely separate concurrent session —
    /// see [`TradeState`]'s doc.
    pub fn open_trade(&mut self, trade: TradeState) {
        if let Some(existing) = self.trades.iter_mut().find(|t| t.opponent_serial == trade.opponent_serial) {
            *existing = trade;
        } else {
            self.trades.push(trade);
        }
    }

    /// Look up an active trade session by our own container serial — every
    /// client-bound 0x6F action (Update/UpdateGold/UpdateLedger/Close) and
    /// every outgoing action we send addresses a session this way (see
    /// [`TradeState`]'s doc). `None` if no session has that container (the
    /// caller raced the session away — treat as a no-op).
    pub fn trade_mut(&mut self, my_container: u32) -> Option<&mut TradeState> {
        self.trades.iter_mut().find(|t| t.my_container == my_container)
    }

    /// Close a trade session by our own container serial (0x6F action 1, or a
    /// locally-initiated cancel) and purge its leftover container contents.
    /// ServUO bounces both sides' items back to their owners' backpacks on
    /// close but sends NO removal packets for the opponent's side (our own
    /// bounced items come back via ordinary 0x25 AddToContainer traffic
    /// instead) — without this, anything still sitting in either trade
    /// container at close time would linger in [`World::items`] forever,
    /// keyed by a container serial nothing will ever reference again. No-op
    /// if no session has that container.
    pub fn close_trade(&mut self, my_container: u32) {
        let Some(pos) = self.trades.iter().position(|t| t.my_container == my_container) else {
            return;
        };
        let t = self.trades.remove(pos);
        self.items.retain(|serial, item| {
            *serial != t.my_container
                && *serial != t.their_container
                && item.container != Some(t.my_container)
                && item.container != Some(t.their_container)
        });
    }

    /// Store an entity's Object Property List (0xD6 MegaCliloc): the raw property
    /// lines `(cliloc, args)` plus the `revision` hash. Replaces any prior list for
    /// the serial (the server sends the full list each time).
    pub fn set_opl(&mut self, serial: u32, revision: u32, lines: Vec<(u32, String)>) {
        self.opl_revision.insert(serial, revision);
        self.opl.insert(serial, lines);
    }

    /// Record a spellbook's known contents (0xBF/0x1B NewSpellbookContent).
    /// Upserts by the book's own serial — a re-opened book simply replaces its
    /// previous entry (the server always sends the current full mask, not a diff).
    pub fn set_spellbook_content(&mut self, serial: u32, graphic: u16, offset: u16, content: u64) {
        self.spellbooks.insert(serial, SpellbookContent { graphic, offset, content });
    }

    /// Open or refresh a map item's window (0x90 DisplayMap / 0xF5
    /// DisplayMapNew). Upserts by `serial`, resetting `pins` to empty and
    /// bumping [`MapView::open_seq`] UNCONDITIONALLY, even for byte-identical
    /// content — see that field's doc for why a repeat must still read as a
    /// fresh "please open". Resetting `pins` here (rather than keeping the
    /// old list) matches the real wire sequence: ServUO's `MapItem.DisplayTo`
    /// always follows this packet with a 0x56 command-5 (Clear) and then one
    /// command-1 (Add) per CURRENT pin, so the full list is about to be
    /// rebuilt from scratch by [`World::apply_map_command`] regardless.
    #[allow(clippy::too_many_arguments)]
    pub fn set_map_view(
        &mut self,
        serial: u32,
        gump_art: u16,
        facet: u8,
        min_x: u16,
        min_y: u16,
        max_x: u16,
        max_y: u16,
        width: u16,
        height: u16,
    ) {
        self.map_open_seq += 1;
        self.map_gumps.insert(
            serial,
            MapView {
                open_seq: self.map_open_seq,
                gump_art,
                facet,
                min_x,
                min_y,
                max_x,
                max_y,
                width,
                height,
                pins: Vec::new(),
                editable: false,
            },
        );
    }

    /// Apply a 0x56 MapCommand to the map window `serial` names — a no-op if
    /// we have no [`MapView`] for it (a command for a map we were never shown
    /// a 0x90/0xF5 for, or one already pruned). `command`/`number`/`x`/`y` are
    /// the raw wire fields; semantics cross-checked against ServUO `MapItem`'s
    /// `OnAddPin`/`OnInsertPin`/`OnChangePin`/`OnRemovePin`/`OnClearPins`/
    /// `OnToggleEditable` and its `MapSetEditable` reply, and ClassicUO's
    /// `MapMessageType` enum (`Add`=1, `Insert`=2, `Move`=3, `Remove`=4,
    /// `Clear`=5, `Edit`=6, `EditResponse`=7):
    /// - `1` Add — append `(x, y)` (`number` unused; ServUO's own `MapAddPin`
    ///   reply always writes 0 there).
    /// - `2` Insert — insert `(x, y)` at index `number`, clamped to the end
    ///   (ServUO `InsertPin`: an out-of-range index appends instead).
    /// - `3` Move/Change — replace the pin at index `number` with `(x, y)`;
    ///   no-op if `number` is out of range (ServUO `ChangePin`).
    /// - `4` Remove — remove the pin at index `number`; refuses index 0 (the
    ///   treasure/chest pin — see [`MapView::pins`]'s doc) same as ServUO's
    ///   `RemovePin` (`index > 0 && index < count`), and any other
    ///   out-of-range index.
    /// - `5` Clear — drop every pin. Also how ServUO's own `MapDisplay` "please
    ///   open" nudge rides this exact command (`number=x=y=0`) right after a
    ///   0x90/0xF5 — harmless here since [`World::set_map_view`] already
    ///   reset `pins` to empty.
    /// - `6` Edit (toggle request) — flips `editable`. Only ever observed as a
    ///   CLIENT→SERVER request in the reference sources (a real client's
    ///   "Plot Course"/"Stop Plotting" button); handled defensively for
    ///   protocol completeness in case a server ever echoes it back.
    /// - `7` EditResponse — the authoritative feedback: sets `editable` from
    ///   the bool ServUO packs into `number` (`MapSetEditable`).
    /// - Any other byte is ignored.
    pub fn apply_map_command(&mut self, serial: u32, command: u8, number: u8, x: u16, y: u16) {
        let Some(mv) = self.map_gumps.get_mut(&serial) else {
            return;
        };
        match command {
            1 => mv.pins.push((x, y)),
            2 => {
                let idx = (number as usize).min(mv.pins.len());
                mv.pins.insert(idx, (x, y));
            }
            3 => {
                if let Some(p) = mv.pins.get_mut(number as usize) {
                    *p = (x, y);
                }
            }
            4 => {
                if number > 0 && (number as usize) < mv.pins.len() {
                    mv.pins.remove(number as usize);
                }
            }
            5 => mv.pins.clear(),
            6 => mv.editable = !mv.editable,
            7 => mv.editable = number != 0,
            _ => {}
        }
    }

    /// Remove an entity (mobile or item) by serial. Returns true if it was a mobile.
    /// A deleted corpse item also drops its [`World::corpse_of`]/
    /// [`World::corpse_equip`] entries (corpses despawn, so these must not outlive
    /// the item they describe). A deleted spellbook likewise drops its
    /// [`World::spellbooks`] entry, and a deleted map item drops its
    /// [`World::map_gumps`] window.
    pub fn remove(&mut self, serial: u32) -> bool {
        let was_mobile = self.mobiles.remove(&serial).is_some();
        self.items.remove(&serial);
        self.opl.remove(&serial);
        self.opl_revision.remove(&serial);
        self.spellbooks.remove(&serial);
        self.corpse_of.remove(&serial);
        self.corpse_equip.remove(&serial);
        self.map_gumps.remove(&serial);
        was_mobile
    }

    /// Whether `serial`'s container chain (item → the container item holding it
    /// → …) ultimately bottoms out at `root` — i.e. `serial` is worn by `root`
    /// or sits somewhere inside a container `root` is holding (backpack
    /// contents, a nested pouch, …). Mirrors ClassicUO `Item.RootContainer`.
    /// Capped so a malformed/cyclic container chain can't loop forever.
    fn item_rooted_at(&self, serial: u32, root: u32) -> bool {
        let mut current = match self.items.get(&serial) {
            Some(it) => it.container,
            None => return false,
        };
        for _ in 0..32 {
            match current {
                Some(c) if c == root => return true,
                Some(c) => match self.items.get(&c) {
                    Some(parent) => current = parent.container,
                    None => return false, // container is a mobile (not `root`) or already gone
                },
                None => return false, // lying on the ground — no container at all
            }
        }
        false
    }

    /// Apply a facet switch (0xBF/0x08 MapChange). The server never sends 0x1D
    /// deletes for the facet we're leaving — a moongate/recall just drops us
    /// straight into a new world with a fresh view — so without this the old
    /// facet's mobiles/items would linger as phantoms forever. Mirrors
    /// ClassicUO `World.MapIndex`'s setter, which calls
    /// `InternalMapChangeClear(noplayer: true)`: keep the player mobile and
    /// everything the player is holding (worn equipment + backpack contents,
    /// however deeply nested — see [`World::item_rooted_at`]), drop every
    /// other mobile/item, and prune the serial-keyed state that would
    /// otherwise still point at a dropped entity. A same-index call (the
    /// server re-affirming the current facet) is a no-op — real 0x08 traffic
    /// can and does repeat the current facet.
    pub fn on_map_change(&mut self, new_index: u8) {
        if new_index == self.map_index {
            return;
        }
        self.map_index = new_index;
        let Some(player) = self.player.map(|s| s.0) else {
            // Not actually in the world yet — shouldn't happen for a real
            // MapChange, but with no player nothing has an owner either way.
            self.mobiles.clear();
            self.items.clear();
            self.corpse_of.clear();
            self.corpse_equip.clear();
            self.opl.clear();
            self.opl_revision.clear();
            self.spellbooks.clear();
            self.map_gumps.clear();
            self.popup = None;
            return;
        };
        self.mobiles.retain(|&serial, _| serial == player);
        let keep_items: HashSet<u32> = self
            .items
            .keys()
            .copied()
            .filter(|&serial| self.item_rooted_at(serial, player))
            .collect();
        self.items.retain(|serial, _| keep_items.contains(serial));
        // Anything still indexed by a serial that just got dropped above is now
        // dangling — prune it the same way `World::remove` does for a single
        // delete, just batched over every purged serial. A spellbook rooted at
        // the player (worn or in the backpack, the common case) survives here
        // same as its item entry does — only a stray entry for some OTHER
        // book gets dropped.
        let alive = |serial: &u32| *serial == player || keep_items.contains(serial);
        self.corpse_of.retain(|corpse, _| alive(corpse));
        self.corpse_equip.retain(|corpse, _| alive(corpse));
        self.opl.retain(|serial, _| alive(serial));
        self.opl_revision.retain(|serial, _| alive(serial));
        self.spellbooks.retain(|serial, _| alive(serial));
        self.map_gumps.retain(|serial, _| alive(serial));
        if self.popup.as_ref().is_some_and(|p| !alive(&p.serial)) {
            self.popup = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_resolves_from_serial() {
        let mut w = World::new();
        assert!(w.player_mobile().is_none());

        let me = Serial(0x0000_2A2A);
        w.mobile_mut(me.0).name = "Anima".into();
        w.player = Some(me);

        assert_eq!(w.player_mobile().unwrap().name, "Anima");
    }

    #[test]
    fn close_shop_buy_drops_a_stale_buy_window() {
        let mut w = World::new();
        w.shop_buy = Some(ShopBuy { vendor: 0x1234, container: 0x1, entries: vec![] });
        w.close_shop_buy();
        assert!(w.shop_buy.is_none());
    }

    #[test]
    fn close_gump_by_type_drops_matching_kind_keeps_others() {
        let mut w = World::new();
        w.add_gump(Gump { serial: 1, gump_id: 100, ..Default::default() });
        w.add_gump(Gump { serial: 2, gump_id: 100, ..Default::default() });
        w.add_gump(Gump { serial: 3, gump_id: 200, ..Default::default() });
        w.close_gump_by_type(100);
        assert_eq!(w.gumps.len(), 1);
        assert_eq!(w.gumps[0].serial, 3);
    }

    #[test]
    fn paperdoll_seq_increments_on_every_set_even_same_serial() {
        let mut w = World::new();
        w.set_paperdoll(0xAAAA, "Anima the Adventurer".into(), false, true);
        let first = w.paperdoll.clone().unwrap();
        assert_eq!(first.seq, 1);
        // A repeat request for the SAME serial (re-double-click) still bumps
        // `seq` — the renderer must reopen a window it had closed.
        w.set_paperdoll(0xAAAA, "Anima the Adventurer".into(), true, true);
        let second = w.paperdoll.clone().unwrap();
        assert_eq!(second.seq, 2);
        assert!(second.warmode);
    }

    #[test]
    fn close_shop_sell_drops_a_stale_sell_window() {
        // Regression: `Session::apply_action` (anima-net) used to send
        // `SellItems` without ever clearing `world.shop_sell` — a second sell
        // trip could then answer the *previous* SellList, whose serials no
        // longer match anything in the pack (a silent failed sale).
        let mut w = World::new();
        w.shop_sell = Some(ShopSell { vendor: 0x1234, items: vec![] });
        w.close_shop_sell();
        assert!(w.shop_sell.is_none());
    }

    #[test]
    fn set_map_view_upserts_and_bumps_open_seq_even_when_repeated() {
        let mut w = World::new();
        w.set_map_view(0x4000_1111, 0x139D, 0, 0, 0, 400, 400, 200, 200);
        let first = w.map_gumps.get(&0x4000_1111).cloned().unwrap();
        assert_eq!(first.open_seq, 1);
        assert_eq!((first.gump_art, first.width, first.height), (0x139D, 200, 200));
        assert!(first.pins.is_empty());

        // A re-decode/re-click resends byte-identical bounds — must still bump
        // `open_seq` (see `MapView::open_seq`'s doc) so the renderer reopens a
        // window the player closed.
        w.set_map_view(0x4000_1111, 0x139D, 0, 0, 0, 400, 400, 200, 200);
        let second = w.map_gumps.get(&0x4000_1111).unwrap();
        assert_eq!(second.open_seq, 2);
    }

    #[test]
    fn apply_map_command_add_and_clear_pins() {
        let mut w = World::new();
        w.set_map_view(0x4000_2222, 0x139D, 0, 0, 0, 400, 400, 200, 200);
        // command 1 = Add: the treasure/chest pin (index 0), then a player pin.
        w.apply_map_command(0x4000_2222, 1, 0, 100, 120);
        w.apply_map_command(0x4000_2222, 1, 0, 50, 60);
        assert_eq!(w.map_gumps[&0x4000_2222].pins, vec![(100, 120), (50, 60)]);

        // command 5 = Clear: drops every pin.
        w.apply_map_command(0x4000_2222, 5, 0, 0, 0);
        assert!(w.map_gumps[&0x4000_2222].pins.is_empty());
    }

    #[test]
    fn apply_map_command_remove_refuses_index_zero() {
        let mut w = World::new();
        w.set_map_view(0x4000_3333, 0x139D, 0, 0, 0, 400, 400, 200, 200);
        w.apply_map_command(0x4000_3333, 1, 0, 10, 10); // index 0 — the chest pin
        w.apply_map_command(0x4000_3333, 1, 0, 20, 20); // index 1 — a player pin

        // Removing index 0 (the chest pin) must be refused, mirroring ServUO
        // `MapItem.RemovePin`'s `index > 0` guard.
        w.apply_map_command(0x4000_3333, 4, 0, 0, 0);
        assert_eq!(w.map_gumps[&0x4000_3333].pins.len(), 2, "index 0 must survive a remove");

        // Removing index 1 is allowed.
        w.apply_map_command(0x4000_3333, 4, 1, 0, 0);
        assert_eq!(w.map_gumps[&0x4000_3333].pins, vec![(10, 10)]);

        // Out-of-range index is a no-op, not a panic.
        w.apply_map_command(0x4000_3333, 4, 9, 0, 0);
        assert_eq!(w.map_gumps[&0x4000_3333].pins.len(), 1);
    }

    #[test]
    fn apply_map_command_insert_move_and_editable() {
        let mut w = World::new();
        w.set_map_view(0x4000_4444, 0x139D, 0, 0, 0, 400, 400, 200, 200);
        w.apply_map_command(0x4000_4444, 1, 0, 10, 10); // [ (10,10) ]
        w.apply_map_command(0x4000_4444, 2, 0, 5, 5); // Insert at 0 -> [(5,5),(10,10)]
        assert_eq!(w.map_gumps[&0x4000_4444].pins, vec![(5, 5), (10, 10)]);

        // Insert at an out-of-range index appends (ServUO `InsertPin` behavior).
        w.apply_map_command(0x4000_4444, 2, 99, 30, 30);
        assert_eq!(w.map_gumps[&0x4000_4444].pins, vec![(5, 5), (10, 10), (30, 30)]);

        // Move/change index 1 in place.
        w.apply_map_command(0x4000_4444, 3, 1, 11, 12);
        assert_eq!(w.map_gumps[&0x4000_4444].pins[1], (11, 12));

        // command 7 = SetEditable: `number` carries the bool.
        assert!(!w.map_gumps[&0x4000_4444].editable);
        w.apply_map_command(0x4000_4444, 7, 1, 0, 0);
        assert!(w.map_gumps[&0x4000_4444].editable);
        w.apply_map_command(0x4000_4444, 7, 0, 0, 0);
        assert!(!w.map_gumps[&0x4000_4444].editable);
    }

    #[test]
    fn apply_map_command_unknown_serial_is_a_no_op() {
        let mut w = World::new();
        w.apply_map_command(0xDEAD_BEEF, 1, 0, 1, 1); // no MapView for this serial
        assert!(w.map_gumps.is_empty());
    }

    #[test]
    fn map_view_pruned_on_delete_and_facet_purge() {
        let mut w = World::new();
        w.enter_world(&LoginResult { serial: 0x1, x: 0, y: 0, z: 0, direction: 0, body: 0x190, aos: false });
        w.set_map_view(0x4000_5555, 0x139D, 0, 0, 0, 400, 400, 200, 200);
        assert!(w.map_gumps.contains_key(&0x4000_5555));

        w.remove(0x4000_5555);
        assert!(w.map_gumps.is_empty(), "0x1D delete must prune the map view");

        w.set_map_view(0x4000_6666, 0x139D, 0, 0, 0, 400, 400, 200, 200);
        w.on_map_change(1); // facet switch — not rooted at the player, so it's purged
        assert!(w.map_gumps.is_empty(), "a facet switch must purge a map view we're not carrying");
    }

    #[test]
    fn enter_world_seeds_player() {
        let mut w = World::new();
        w.enter_world(&LoginResult {
            serial: 0x311,
            x: 3503,
            y: 2574,
            z: 14,
            direction: 0,
            body: 0x0190,
            aos: false,
        });
        assert!(w.is_player(0x311));
        let p = w.player_mobile().unwrap();
        assert_eq!(p.pos.x, 3503);
        assert_eq!(p.body, 0x0190);
    }
}
