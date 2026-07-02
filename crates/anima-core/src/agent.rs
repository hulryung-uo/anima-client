//! The Observation/Action contract — the stable seam between the world and a
//! brain (AI or a human's input) or a renderer.
//!
//! - [`Observation`] is a read-only snapshot the brain consumes; build it with
//!   [`World::observe`].
//! - [`Action`] is a high-level intent the brain emits; a driver
//!   ([`anima-net`]'s `Session`) turns it into packets.
//!
//! Keeping this schema stable lets scripted / RL / LLM brains and the
//! native/WASM backends all plug into the same interface (see DESIGN.md §3).

use crate::gump_layout::GumpElement;
use crate::types::Position;
use crate::world::{
    Book, Buff, JournalEntry, Party, PopupMenu, PromptState, ShopBuy, ShopSell, TargetCursor,
    TradeState, Weather, World,
};

/// A skill value, in human units (50.0 == GM-half). Derived from [`crate::world::Skill`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkillView {
    pub id: u16,
    pub value: f32,
    pub base: f32,
    pub cap: f32,
    pub lock: u8,
}

/// A read-only view of our own character.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlayerView {
    pub serial: u32,
    pub name: String,
    pub pos: Position,
    pub direction: u8,
    pub hits: u16,
    pub hits_max: u16,
    pub mana: u16,
    pub mana_max: u16,
    pub stam: u16,
    pub stam_max: u16,
    pub strength: u16,
    pub dexterity: u16,
    pub intelligence: u16,
    pub gold: u32,
    pub weight: u16,
    /// Carry-weight cap ([`crate::world::PlayerStats::weight_max`]) — the
    /// natural companion to `weight` for "can I still pick this up".
    pub weight_max: u16,
    /// Armor rating (AR), [`crate::world::PlayerStats::armor`].
    pub armor: i16,
}

/// A nearby creature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobileView {
    pub serial: u32,
    pub name: String,
    pub pos: Position,
    pub body: u16,
    pub notoriety: u8,
    pub hits: u16,
    pub hits_max: u16,
    /// Chebyshev distance from the player.
    pub distance: u32,
}

/// A nearby item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemView {
    pub serial: u32,
    pub graphic: u16,
    pub amount: u16,
    pub pos: Position,
    pub container: Option<u32>,
    /// Worn layer (0 if not equipped). 0x15 (21) = backpack.
    pub layer: u8,
    pub distance: u32,
}

/// A perception snapshot for the brain. Nearby lists are sorted by distance.
#[derive(Debug, Clone, Default)]
pub struct Observation {
    pub player: PlayerView,
    pub mobiles: Vec<MobileView>,
    pub items: Vec<ItemView>,
    /// Journal lines since the last observation (see [`World::observe`]).
    pub new_journal: Vec<JournalEntry>,
    /// Set when the server is waiting for us to pick a target (answer with
    /// [`Action::TargetObject`] / [`Action::TargetGround`]).
    pub pending_target: Option<TargetCursor>,
    /// Our skills, sorted by id (values in human units, e.g. 50.0).
    pub skills: Vec<SkillView>,
    /// Open server gumps/dialogs (0xB0/0xDD) — e.g. a craft menu. Answer with
    /// [`Action::GumpResponse`].
    pub gumps: Vec<GumpView>,
    /// An outstanding server text prompt (0xC2 UnicodePrompt — pet rename, house
    /// sign, guild abbreviation, …), if one is pending. Answer with
    /// [`Action::PromptResponse`]/[`Action::PromptCancel`].
    pub prompt: Option<PromptState>,
    /// Active player-to-player secure trade sessions (0x6F), if any — normally
    /// 0 or 1, but see [`crate::world::TradeState`]'s doc for why concurrent
    /// sessions with different opponents are possible. Items on each side are
    /// the [`ItemView`]s whose `container` matches a session's
    /// `my_container`/`their_container`. Answer with
    /// [`Action::TradeAccept`]/[`Action::TradeCancel`]/[`Action::TradeGold`],
    /// each addressed to a specific session via its `my_container`.
    pub trades: Vec<TradeState>,
    /// The player's active buffs/debuffs (0xDF). See [`Buff`].
    pub buffs: Vec<Buff>,
    /// The open vendor BUY window (0x74), if any. See [`ShopBuy`]. Answer with
    /// [`Action::BuyItems`].
    pub shop_buy: Option<ShopBuy>,
    /// The open vendor SELL window (0x9E), if any. See [`ShopSell`]. Answer
    /// with [`Action::SellItems`].
    pub shop_sell: Option<ShopSell>,
    /// The open context (right-click popup) menu (0xBF/0x14), if any. See
    /// [`PopupMenu`]. Answer with [`Action::PopupSelect`].
    pub popup: Option<PopupMenu>,
    /// The currently open book (0x93/0xD4 + 0x66), if any. See [`Book`].
    /// Request more pages with [`Action::BookRequest`].
    pub book: Option<Book>,
    /// The player's party (0xBF/0x06). See [`Party`]. An empty `members` means
    /// we're not in a party. Answer a pending invite with
    /// [`Action::PartyAccept`]/[`Action::PartyDecline`].
    pub party: Party,
    /// An on-screen quest arrow (0xBA) pointing at world tile `(x, y)`, or
    /// `None` when hidden.
    pub quest_arrow: Option<(u16, u16)>,
    /// Current weather (0x65). See [`Weather`].
    pub weather: Weather,
    /// Current season (0xBC): 0=Spring, 1=Summer, 2=Fall, 3=Winter, 4=Desolation.
    pub season: u8,
    /// Effective light level a renderer would use (brighter of the overall and
    /// personal light — see [`World::effective_light`]); 0 = brightest day,
    /// ~0x1F darkest night.
    pub light: u8,
    /// Whether the player is in war mode (combat stance). Toggle with
    /// [`Action::WarMode`].
    pub war: bool,
    /// The serial we last sent an Attack (0x05) request for — UO's "last
    /// target" for the auto-attack loop ([`Action::AttackLast`]/
    /// [`Action::AutoAttack`]). `None` until the player attacks.
    pub last_attack: Option<u32>,
    /// The server's authoritative current combat opponent (0xAA
    /// ChangeCombatant), distinct from `last_attack` (which is only the last
    /// serial *we* asked to attack — the server can retarget on its own).
    /// `None` when combat has ended.
    pub combatant: Option<u32>,
    /// Corpse→killed-mobile links (0xAF DisplayDeath), each `(corpse_serial,
    /// killed_mobile_serial)`, sorted by corpse serial. Lets a brain confirm
    /// "this is the corpse of what I killed" before looting.
    pub corpse_of: Vec<(u32, u32)>,
    /// A corpse's worn-item layout (0x89 CorpseEquip), each `(corpse_serial,
    /// [(layer, item_serial), …])`, sorted by corpse serial.
    pub corpse_equip: Vec<(u32, Vec<(u8, u32)>)>,
    /// Current facet/map index (0xBF/0x08 MapChange): 0=Felucca, 1=Trammel,
    /// 2=Ilshenar, 3=Malas, 4=Tokuno, 5=TerMur.
    pub map_index: u8,
    /// Whether the server advertised the AOS expansion during login
    /// ([`World::aos`]) — gates AOS-only mechanics (e.g. weapon special moves
    /// via [`Action::UseAbility`]).
    pub aos: bool,
    /// Object Property Lists (0xD6 MegaCliloc) answering an [`Action::OplRequest`],
    /// each `(serial, [(cliloc id, args), …])`, sorted by serial. Raw — the core
    /// has no Cliloc table, so a brain wanting display text resolves it itself
    /// (mirrors [`GumpView::layout`]'s cliloc-driven `html` elements). Line 0 is
    /// the name; the rest are magic properties, in the order the server sent them.
    pub opl: Vec<(u32, Vec<(u32, String)>)>,
    /// Recent per-hit damage events (0x0B), each `(seq, serial, amount)`, oldest
    /// first, capped to the most recent few — `serial` took `amount` HP. A combat
    /// brain wants this: other mobiles' HP otherwise only arrives as a coarse
    /// scaled percentage (0x17/0x77's damage bar). Dedupe on `seq` across polls
    /// (like the renderer's scene bridge does) — this always carries the full
    /// capped buffer, not just what's new since the last observation.
    pub recent_damage: Vec<(u64, u32, u16)>,
}

/// A read-only view of an open server gump/dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GumpView {
    pub serial: u32,
    pub gump_id: u32,
    /// The raw UO gump layout string (`{ button … }{ gumppic … }…`), kept as a
    /// fallback for a brain that wants to parse it itself.
    pub layout: String,
    /// `layout` parsed into typed elements (see [`crate::gump_layout`]) — the
    /// normal way a brain reads a gump instead of re-implementing the grammar.
    /// A cliloc-driven [`GumpElement::Html`] is left unresolved (the core has
    /// no Cliloc table); a driver with one (`anima-net`) resolves it.
    pub elements: Vec<GumpElement>,
}

/// A decision-maker that turns perception into intent. Scripted, RL, or LLM
/// brains all implement this; a driver feeds it [`Observation`]s and executes the
/// [`Action`]s it returns. This is the top of the Interface⊥Brain split.
pub trait Brain {
    /// Decide what to do given the current perception. May return zero or more
    /// actions (typically one step + the occasional speech/use).
    fn decide(&mut self, obs: &Observation) -> Vec<Action>;
}

/// The UO direction (0..7) that heads from the player toward a `(dx, dy)` offset
/// (each component reduced to its sign). Returns `None` for a zero offset.
pub fn dir_toward(dx: i32, dy: i32) -> Option<u8> {
    use crate::net::movement::direction_delta;
    let sx = dx.signum();
    let sy = dy.signum();
    if (sx, sy) == (0, 0) {
        return None;
    }
    (0u8..8).find(|&d| direction_delta(d) == (sx, sy))
}

/// A high-level intent emitted by the brain. The driver executes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Step one tile in UO direction 0..7 (running optional).
    Walk { dir: u8, run: bool },
    /// Auto-walk (click-to-walk): pathfind to world tile `(x, y)` and drive the
    /// player there step-by-step. A new `WalkTo` or any manual [`Action::Walk`]
    /// cancels an active route. The driver owns the route + pacing — the
    /// play-server paces it in its own loop; the headless `anima-net::Session`
    /// does the same non-blockingly via `Session::advance_route` (call it once
    /// per tick; `Session::navigate_to` remains for a blocking one-shot walk).
    WalkTo { x: u16, y: u16 },
    /// Speak in-game.
    Say { text: String },
    /// Send a message to the player's party (all members).
    PartySay { text: String },
    /// Begin attacking a target. The driver remembers `serial` as the "last
    /// target" (see [`crate::World::last_attack`]) for the auto-attack loop.
    Attack { serial: u32 },
    /// Auto-attack the best target (UO "last target" combat loop): the current
    /// last target if it's still a live in-view hostile, otherwise the nearest
    /// in-view hostile mobile. The driver picks the serial from the world.
    AutoAttack,
    /// Re-attack the current "last target" ([`crate::World::last_attack`]); a no-op
    /// if nothing has been attacked yet.
    AttackLast,
    /// Double-click ("use") an item or mobile.
    Use { serial: u32 },
    /// Single-click (request the name/label).
    Click { serial: u32 },
    /// Lift `amount` from a stack/item.
    PickUp { serial: u32, amount: u16 },
    /// Drop a held item at `(x, y, z)` into `container` (0xFFFFFFFF = ground).
    Drop { serial: u32, x: u16, y: u16, z: i16, container: u32 },
    /// Equip a held item onto the player at `layer` (UO 0x13 EquipRequest).
    Equip { serial: u32, layer: u8 },
    /// Toggle war mode.
    WarMode { on: bool },
    /// Cast a Magery spell by its spell id (1..64). If the spell needs a target,
    /// the server replies with a target cursor (answer via `TargetObject`/`TargetGround`).
    CastSpell { spell: u16 },
    /// Answer a pending target cursor by selecting an object/mobile.
    TargetObject { serial: u32 },
    /// Answer a pending target cursor by selecting a ground location.
    TargetGround { x: u16, y: u16, z: i16, graphic: u16 },
    /// Cancel a pending target cursor (Esc): the server stops waiting for a target
    /// (the spell/skill awaiting one is aborted) instead of hanging.
    TargetCancel,
    /// Buy `items` (each `(item serial, amount)`) from a vendor mobile (UO 0x3B).
    BuyItems { vendor: u32, items: Vec<(u32, u16)> },
    /// Sell `items` (each `(item serial, amount)`) to a vendor mobile (UO 0x9F).
    SellItems { vendor: u32, items: Vec<(u32, u16)> },
    /// Answer a server gump/dialog (0xB0/0xDD) with packet 0xB1 GumpResponse.
    /// `button` is the clicked reply button id (0 = close/cancel); `switches` are
    /// the ids of all checked checkboxes/selected radios; `entries` are
    /// `(textEntryId, value)` for each text field. The driver also closes the gump
    /// locally once the response is sent.
    GumpResponse {
        serial: u32,
        gump_id: u32,
        button: u32,
        switches: Vec<u32>,
        entries: Vec<(u16, String)>,
    },
    /// Request the right-click context (popup) menu for an entity (0xBF/0x13).
    /// The server answers with 0xBF/0x14, stored in `World::popup`.
    PopupRequest { serial: u32 },
    /// Choose entry `index` from the open context menu for `serial` (0xBF/0x15).
    PopupSelect { serial: u32, index: u16 },
    /// Request the content of all `pages` of the open book `serial` (outgoing 0x66).
    /// The server replies with 0x66 BookData, filling `World::book`.
    BookRequest { serial: u32, pages: u16 },
    /// Arm a weapon special move (UO 0xD7 UseCombatAbility). `ability` is the
    /// `Ability` enum id (the specific move, 1..=32); `0` disarms. The driver fills
    /// the player's serial. Which moves a weapon offers depends on its graphic
    /// (see ClassicUO `Abilities.cs` weapon→ability table).
    UseAbility { ability: u8 },
    /// Change a skill's lock state (UO 0x3A SkillStatusChangeRequest). `lock` is
    /// 0 = up (raise on gain), 1 = down (lower on gain), 2 = locked. The driver
    /// optimistically updates the world's skill lock so the UI reflects it at once.
    SkillLock { skill: u16, lock: u8 },
    /// Invoke an active skill by id (UO 0x12 ActionRequest, type 0x24 "use skill").
    /// Works for active skills (Hiding, Meditation, Anatomy, Animal Lore, …);
    /// passive skills are a no-op server-side.
    UseSkill { skill: u16 },
    /// Request an entity's Object Property List / tooltip (UO 0xD6 MegaClilocRequest).
    /// The server replies with a 0xD6 MegaCliloc stored in `World::opl`. Sent on
    /// hover so the client can show the item/mobile's full properties.
    OplRequest { serial: u32 },
    /// Invite a player to the party (0xBF/0x06/0x01). The server opens a target
    /// cursor; the player to invite is chosen via the normal target flow.
    PartyInvite,
    /// Accept a pending party invitation from `leader` (0xBF/0x06/0x08).
    PartyAccept { leader: u32 },
    /// Decline a pending party invitation from `leader` (0xBF/0x06/0x09).
    PartyDecline { leader: u32 },
    /// Leave the current party (0xBF/0x06/0x02); the driver fills our own serial.
    PartyLeave,
    /// Answer a pending server text prompt (0xC2 UnicodePrompt — pet rename, house
    /// sign, guild abbreviation, …) with typed `text`. The driver echoes the
    /// prompt's `sender_serial`/`prompt_id` from [`crate::world::World::prompt`]
    /// (cleared once answered); a no-op if nothing is pending.
    PromptResponse { text: String },
    /// Cancel a pending server text prompt (Esc): the server aborts whatever
    /// needed the response instead of leaving it dangling; a no-op if nothing is
    /// pending.
    PromptCancel,
    /// Toggle our side's accept checkbox on a secure trade (0x6F action 2).
    /// `container` selects which session (its `my_container`, from
    /// [`crate::world::World::trades`] — multiple can be open at once with
    /// different opponents); a no-op if no session has that container (the
    /// brain raced the session away). Both sides accepting completes the
    /// trade server-side.
    TradeAccept { container: u32, accept: bool },
    /// Cancel a secure trade (0x6F action 1): items on both sides return to
    /// their owners. `container` selects which session; the driver clears
    /// just that session locally; a no-op if no session has that container.
    TradeCancel { container: u32 },
    /// Set the virtual gold/platinum amount we're offering on a secure trade
    /// (0x6F action 3 UpdateGold). `container` selects which session; a no-op
    /// if no session has that container. Only takes effect on a server/client
    /// pair that negotiated the AOS/TOL "account gold" feature (see
    /// [`crate::world::TradeState`]'s doc).
    TradeGold { container: u32, gold: u32, platinum: u32 },
}

fn chebyshev(a: Position, b: Position) -> u32 {
    (a.x.abs_diff(b.x)).max(a.y.abs_diff(b.y)) as u32
}

impl World {
    /// Build an [`Observation`]. `journal_cursor` is the index into
    /// [`World::journal`] already seen; it is advanced to the current length and
    /// only newer lines are returned, so a brain sees each line once.
    pub fn observe(&self, journal_cursor: &mut usize) -> Observation {
        let pm = self.player_mobile().cloned().unwrap_or_default();
        let player = PlayerView {
            serial: pm.serial,
            name: pm.name.clone(),
            pos: pm.pos,
            direction: pm.direction,
            hits: pm.hits,
            hits_max: pm.hits_max,
            mana: pm.mana,
            mana_max: pm.mana_max,
            stam: pm.stam,
            stam_max: pm.stam_max,
            strength: self.player_stats.strength,
            dexterity: self.player_stats.dexterity,
            intelligence: self.player_stats.intelligence,
            gold: self.player_stats.gold,
            weight: self.player_stats.weight,
            weight_max: self.player_stats.weight_max,
            armor: self.player_stats.armor,
        };

        let mut mobiles: Vec<MobileView> = self
            .mobiles
            .values()
            .filter(|m| Some(m.serial) != self.player.map(|s| s.0))
            .map(|m| MobileView {
                serial: m.serial,
                name: m.name.clone(),
                pos: m.pos,
                body: m.body,
                notoriety: m.notoriety,
                hits: m.hits,
                hits_max: m.hits_max,
                distance: chebyshev(player.pos, m.pos),
            })
            .collect();
        mobiles.sort_by_key(|m| m.distance);

        let mut items: Vec<ItemView> = self
            .items
            .values()
            .map(|it| ItemView {
                serial: it.serial,
                graphic: it.graphic,
                amount: it.amount,
                pos: it.pos,
                container: it.container,
                layer: it.layer,
                distance: chebyshev(player.pos, it.pos),
            })
            .collect();
        items.sort_by_key(|it| it.distance);

        let start = (*journal_cursor).min(self.journal.len());
        let new_journal = self.journal[start..].to_vec();
        *journal_cursor = self.journal.len();

        let mut skills: Vec<SkillView> = self
            .skills
            .values()
            .map(|s| SkillView {
                id: s.id,
                value: s.value as f32 / 10.0,
                base: s.base as f32 / 10.0,
                cap: s.cap as f32 / 10.0,
                lock: s.lock,
            })
            .collect();
        skills.sort_by_key(|s| s.id);

        let gumps = self
            .gumps
            .iter()
            .map(|g| GumpView {
                serial: g.serial,
                gump_id: g.gump_id,
                layout: g.layout.clone(),
                elements: crate::gump_layout::parse(&g.layout, &g.text).elements,
            })
            .collect();

        // HashMap iteration order isn't stable — sort so a brain sees a
        // deterministic order run to run (like `skills`, sorted by id).
        let mut corpse_of: Vec<(u32, u32)> = self.corpse_of.iter().map(|(&c, &k)| (c, k)).collect();
        corpse_of.sort_by_key(|&(c, _)| c);
        let mut corpse_equip: Vec<(u32, Vec<(u8, u32)>)> =
            self.corpse_equip.iter().map(|(&c, v)| (c, v.clone())).collect();
        corpse_equip.sort_by_key(|&(c, _)| c);

        let mut opl: Vec<(u32, Vec<(u32, String)>)> =
            self.opl.iter().map(|(&s, v)| (s, v.clone())).collect();
        opl.sort_by_key(|&(s, _)| s);

        Observation {
            player,
            mobiles,
            items,
            new_journal,
            pending_target: self.pending_target,
            skills,
            gumps,
            prompt: self.prompt,
            trades: self.trades.clone(),
            buffs: self.buffs.clone(),
            shop_buy: self.shop_buy.clone(),
            shop_sell: self.shop_sell.clone(),
            popup: self.popup.clone(),
            book: self.book.clone(),
            party: self.party.clone(),
            quest_arrow: self.quest_arrow,
            weather: self.weather,
            season: self.season,
            light: self.effective_light(),
            war: self.war,
            last_attack: self.last_attack,
            combatant: self.combatant,
            corpse_of,
            corpse_equip,
            map_index: self.map_index,
            aos: self.aos,
            opl,
            recent_damage: self.recent_damage.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::login::LoginResult;

    #[test]
    fn observe_sorts_by_distance_and_advances_journal() {
        let mut w = World::new();
        w.enter_world(&LoginResult {
            serial: 0x311,
            x: 100,
            y: 100,
            z: 0,
            direction: 0,
            body: 0x190,
            aos: false,
        });
        // Two mobiles at different distances.
        let far = w.mobile_mut(0xAA);
        far.pos = Position { x: 110, y: 100, z: 0 };
        let near = w.mobile_mut(0xBB);
        near.pos = Position { x: 102, y: 100, z: 0 };

        w.journal.push(JournalEntry {
            serial: 0,
            name: "System".into(),
            text: "hello".into(),
            msg_type: 0,
            hue: 0,
            cliloc: 0,
        });

        let mut cursor = 0;
        let obs = w.observe(&mut cursor);
        assert_eq!(obs.mobiles.len(), 2);
        assert_eq!(obs.mobiles[0].serial, 0xBB); // nearest first
        assert_eq!(obs.mobiles[0].distance, 2);
        assert_eq!(obs.new_journal.len(), 1);

        // A second observe with the advanced cursor sees no repeat lines.
        let obs2 = w.observe(&mut cursor);
        assert!(obs2.new_journal.is_empty());
    }
}
