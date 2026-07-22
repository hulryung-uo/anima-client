//! Versioned JSON wire format for the brain↔body contract.
//!
//! `anima-core` stays serde-free, so the JSON shapes for its
//! [`Observation`]/[`Action`] live in this small adapter crate. Native,
//! out-of-process, and WASM consumers share it instead of mirroring schemas.
//! The shapes are also mirrored by `anima2/anima2/contract.py` — keep that
//! consumer in lockstep.
//!
//! ## Schema (v14 — snake_case, versioned)
//!
//! [`observation_to_json`] emits one object with these top-level keys, one per
//! [`Observation`] field: `player`, `mobiles`, `items`, `new_journal`,
//! `pending_target`, `skills`, `gumps`, `prompt`, `trades`, `buffs`,
//! `shop_buy`, `shop_sell`, `popup`, `legacy_menus`, `hue_pickers`, `open_urls`, `tips`,
//! `text_entry_dialogs`, `book`, `party`, `quest_arrow`, `waypoints`,
//! `weather`, `season`, `light`, `war`, `last_attack`, `combatant`, `corpse_of`,
//! `corpse_equip`, `map_index`, `aos`, `opl`, `recent_damage`, `spellbooks`,
//! `map_gumps`. A
//! gump's `elements` are the structured [`anima_core::gump_layout::GumpElement`]s
//! (see [`gump_element_json`]) — a cliloc-driven `html` element carries the
//! raw `{"id":..,"args":..}` reference *unresolved* (no Cliloc table is
//! threaded through this bridge; see that function's doc). `opl`'s property
//! lines are unresolved the same way — `{"cliloc": id, "args": ".."}` per
//! line, name first. `spellbooks` entries carry `content` split into `lo`/`hi`
//! u32 halves (see [`spellbook_json`]) rather than one 64-bit number, for the
//! same reason the scene bridge splits it: a JS-side (or any float-backed
//! JSON) consumer only keeps 53 bits of integer precision, which a full
//! 64-spell Magery book's mask can exceed. `map_gumps` entries (see
//! [`map_view_json`]) carry a treasure/decoration map's pixel-space pins
//! straight through — no Cliloc/rescale involved (ServUO already converts
//! bounds↔pixel server-side; see [`anima_core::world::MapView`]'s doc).
//!
//! Deliberately **excluded** (renderer/audio-only playback queues with no
//! decision-relevant signal, and each would just add per-tick serialization
//! cost to `observe()` for something a brain can't act on): `current_music`,
//! `recent_sounds`, `recent_anims`/`recent_typed_anims`, `recent_effects`,
//! `recent_lift_rejects`, `recent_container_opens` (0x24 — a window-opening UI
//! signal), `recent_swings` (0x2F — cosmetic facing feedback), `paperdoll`
//! (0x88 — a UI-open signal + display title, not something a brain decides
//! from; equipment state is already in `items`/worn `layer`). `PlayerStats::
//! is_female` is also excluded — purely cosmetic (paperdoll gump selection),
//! never gameplay-relevant. `recent_damage` is *not* excluded — combat brains
//! need per-hit attribution (another mobile's HP otherwise only arrives as a
//! coarse scaled percentage); dedupe on `seq` across polls like the
//! renderer's scene bridge does.
//!
//! Bump the schema marker above and [`SCHEMA_VERSION`] (and note the change here)
//! whenever a key is renamed or removed — the Python side keys off these names verbatim. (v3:
//! added `opl`, `recent_damage`. v4: added `spellbooks`. v5: added `items[].
//! is_multi` — a placed boat/house leaks into `items` as an ordinary-looking
//! entry, but its `graphic` is a *multi id* (an index into
//! `multi.idx`/`multi.mul`), not an ART graphic, and small multi ids collide
//! with real ART ids (e.g. multi id `0x0002`). A brain must check `is_multi`
//! before treating `graphic` as an ART id — see [`ItemView`]'s doc. v6: added
//! `map_gumps` — treasure/decoration map windows (0x90/0xF5 DisplayMap(New) +
//! 0x56 MapCommand), so a brain can locate a decoded treasure map's pins
//! without a human reading the parchment. v7: added `player.body`,
//! `player.poisoned`, and `player.dead` so survival brains receive status that
//! the core world already tracks. v8: added `waypoints`, the server's 0xE5
//! corpse/resurrection/quest markers with 0xE6 removal semantics. v9: added
//! `legacy_menus`, the server's concurrent 0x7C item/question menus. v10:
//! added `hue_pickers`, the server's concurrent 0x95 dye color pickers. v11:
//! added `prompt.kind` (`"ascii"` or `"unicode"`) so consumers can preserve
//! the identity of legacy 0x9A prompts separately from 0xC2 prompts. v12:
//! added `open_urls`, validated 0xA5 HTTP(S) navigation requests carrying a
//! monotonic `seq`; consumers must still ask the user before opening one. v13:
//! added `tips`, concurrent 0xA6 Tip/Notice windows, plus `TipNavigate` and
//! `TipClose` actions. v14: added concurrent 0xAB `text_entry_dialogs` plus
//! `TextEntryResponse` and `TextEntryClose` actions.)
//!
//! [`Observation`]: anima_core::agent::Observation
//! [`Action`]: anima_core::agent::Action

/// Current Observation/Action JSON schema version documented above.
pub const SCHEMA_VERSION: u32 = 14;

use anima_core::agent::{
    Action, GumpView, ItemView, MobileView, Observation, PlayerView, SkillView, WaypointView,
};
use anima_core::gump_layout::{GumpElement, HtmlText};
use anima_core::types::Position;
use anima_core::world::{
    Book, Buff, HuePicker, JournalEntry, LegacyMenu, LegacyMenuEntry, LegacyMenuKind, MapView,
    OpenUrlRequest, Party, PopupEntry, PopupMenu, PromptState, ShopBuy, ShopSell, ShopSellItem,
    SpellbookContent, TargetCursor, TextEntryDialog, TipNotice, TradeState, Weather,
};
use serde_json::{json, Value};

fn pos_json(p: &Position) -> Value {
    json!({ "x": p.x, "y": p.y, "z": p.z })
}

fn player_json(p: &PlayerView) -> Value {
    json!({
        "serial": p.serial, "name": p.name, "pos": pos_json(&p.pos),
        "direction": p.direction, "hits": p.hits, "hits_max": p.hits_max,
        "mana": p.mana, "mana_max": p.mana_max, "stam": p.stam, "stam_max": p.stam_max,
        "strength": p.strength, "dexterity": p.dexterity, "intelligence": p.intelligence,
        "gold": p.gold, "weight": p.weight, "weight_max": p.weight_max, "armor": p.armor,
        "body": p.body, "poisoned": p.poisoned, "dead": p.dead,
    })
}

fn mobile_json(m: &MobileView) -> Value {
    json!({
        "serial": m.serial, "name": m.name, "pos": pos_json(&m.pos), "body": m.body,
        "notoriety": m.notoriety, "hits": m.hits, "hits_max": m.hits_max, "distance": m.distance,
    })
}

fn item_json(i: &ItemView) -> Value {
    json!({
        "serial": i.serial, "graphic": i.graphic, "amount": i.amount,
        "pos": pos_json(&i.pos), "container": i.container, "layer": i.layer,
        "distance": i.distance, "is_multi": i.is_multi,
    })
}

fn skill_json(s: &SkillView) -> Value {
    json!({ "id": s.id, "value": s.value, "base": s.base, "cap": s.cap, "lock": s.lock })
}

/// Serialize a parsed gump element. Positions/ids/pages mirror
/// [`GumpElement`]'s fields directly; an `html` element's `text` is
/// `{"literal": "..."}` when already resolved, or `{"cliloc": {"id":..,
/// "args":..}}` (unresolved — `args` is `null` for a plain `Cliloc::get`
/// lookup, a string for a `Cliloc::format` substitution) when it references a
/// cliloc the core can't look up itself.
fn gump_element_json(e: &GumpElement) -> Value {
    match e {
        GumpElement::Background { x, y, w, h, page } => {
            json!({"type": "background", "x": x, "y": y, "w": w, "h": h, "page": page})
        }
        GumpElement::Image {
            x,
            y,
            graphic,
            page,
        } => {
            json!({"type": "image", "x": x, "y": y, "graphic": graphic, "page": page})
        }
        GumpElement::Button {
            x,
            y,
            graphic,
            reply_id,
            pageflag,
            param,
            page,
        } => json!({
            "type": "button", "x": x, "y": y, "graphic": graphic, "reply_id": reply_id,
            "pageflag": pageflag, "param": param, "page": page,
        }),
        GumpElement::Text { x, y, w, s, page } => {
            json!({"type": "text", "x": x, "y": y, "w": w, "s": s, "page": page})
        }
        GumpElement::Html {
            x,
            y,
            w,
            h,
            text,
            page,
        } => {
            let text = match text {
                HtmlText::Literal(s) => json!({ "literal": s }),
                HtmlText::Cliloc { id, args } => json!({ "cliloc": { "id": id, "args": args } }),
            };
            json!({"type": "html", "x": x, "y": y, "w": w, "h": h, "text": text, "page": page})
        }
        GumpElement::Check { x, y, id, on, page } => {
            json!({"type": "check", "x": x, "y": y, "id": id, "on": on, "page": page})
        }
        GumpElement::Radio { x, y, id, on, page } => {
            json!({"type": "radio", "x": x, "y": y, "id": id, "on": on, "page": page})
        }
        GumpElement::Entry {
            x,
            y,
            w,
            id,
            s,
            page,
        } => {
            json!({"type": "entry", "x": x, "y": y, "w": w, "id": id, "s": s, "page": page})
        }
    }
}

fn gump_json(g: &GumpView) -> Value {
    json!({
        "serial": g.serial, "gump_id": g.gump_id, "layout": g.layout,
        "elements": g.elements.iter().map(gump_element_json).collect::<Vec<_>>(),
    })
}

fn journal_json(j: &JournalEntry) -> Value {
    json!({
        "serial": j.serial, "name": j.name, "text": j.text,
        "msg_type": j.msg_type, "hue": j.hue, "cliloc": j.cliloc,
    })
}

fn target_json(t: &TargetCursor) -> Value {
    json!({ "target_type": t.target_type, "cursor_id": t.cursor_id, "cursor_flag": t.cursor_flag })
}

fn prompt_json(p: &PromptState) -> Value {
    json!({
        "sender_serial": p.sender_serial,
        "prompt_id": p.prompt_id,
        "kind": p.kind.as_str(),
    })
}

fn trade_json(t: &TradeState) -> Value {
    json!({
        "opponent_serial": t.opponent_serial, "opponent_name": t.opponent_name,
        "my_container": t.my_container, "their_container": t.their_container,
        "my_accept": t.my_accept, "their_accept": t.their_accept,
        "my_offer_gold": t.my_offer_gold, "my_offer_platinum": t.my_offer_platinum,
        "their_offer_gold": t.their_offer_gold, "their_offer_platinum": t.their_offer_platinum,
        "balance_gold": t.balance_gold, "balance_platinum": t.balance_platinum,
    })
}

fn buff_json(b: &Buff) -> Value {
    json!({ "icon": b.icon, "name": b.name, "dur": b.dur })
}

fn shop_buy_json(s: &ShopBuy) -> Value {
    let entries: Vec<Value> = s
        .entries
        .iter()
        .map(|(price, name)| json!({ "price": price, "name": name }))
        .collect();
    json!({ "vendor": s.vendor, "container": s.container, "entries": entries })
}

fn shop_sell_item_json(i: &ShopSellItem) -> Value {
    json!({
        "serial": i.serial, "graphic": i.graphic, "hue": i.hue,
        "amount": i.amount, "price": i.price, "name": i.name,
    })
}

fn shop_sell_json(s: &ShopSell) -> Value {
    json!({
        "vendor": s.vendor,
        "items": s.items.iter().map(shop_sell_item_json).collect::<Vec<_>>(),
    })
}

/// Serialize one spellbook's known-contents mask (0xBF/0x1B), keyed by its own
/// serial. `content` is split into two u32 halves (`lo` = bits 0..31, `hi` =
/// bits 32..63) rather than sent whole — see this module's top doc for why.
fn spellbook_json((serial, sb): &(u32, SpellbookContent)) -> Value {
    json!({
        "serial": serial, "graphic": sb.graphic, "offset": sb.offset,
        "lo": (sb.content & 0xFFFF_FFFF) as u32, "hi": (sb.content >> 32) as u32,
    })
}

/// Serialize one open map window (0x90/0xF5 + 0x56), keyed by its own serial.
/// `pins` are `[x, y]` pairs in `width`×`height` pixel space (see [`MapView`]'s
/// doc — already server-converted, no rescale needed).
fn map_view_json((serial, mv): &(u32, MapView)) -> Value {
    json!({
        "serial": serial, "open_seq": mv.open_seq, "gump_art": mv.gump_art, "facet": mv.facet,
        "bounds": { "min_x": mv.min_x, "min_y": mv.min_y, "max_x": mv.max_x, "max_y": mv.max_y },
        "width": mv.width, "height": mv.height,
        "pins": mv.pins.iter().map(|&(x, y)| json!([x, y])).collect::<Vec<_>>(),
        "editable": mv.editable,
    })
}

fn popup_entry_json(e: &PopupEntry) -> Value {
    json!({ "index": e.index, "cliloc": e.cliloc, "flags": e.flags })
}

fn popup_json(p: &PopupMenu) -> Value {
    json!({
        "serial": p.serial,
        "entries": p.entries.iter().map(popup_entry_json).collect::<Vec<_>>(),
    })
}

fn legacy_menu_entry_json(index: usize, entry: &LegacyMenuEntry) -> Value {
    json!({
        "index": index + 1,
        "graphic": entry.graphic,
        "hue": entry.hue,
        "text": entry.text,
    })
}

fn legacy_menu_json(menu: &LegacyMenu) -> Value {
    let kind = match menu.kind {
        LegacyMenuKind::Items => "items",
        LegacyMenuKind::Question => "question",
    };
    json!({
        "serial": menu.serial,
        "menu_id": menu.menu_id,
        "question": menu.question,
        "kind": kind,
        "entries": menu.entries.iter().enumerate()
            .map(|(index, entry)| legacy_menu_entry_json(index, entry))
            .collect::<Vec<_>>(),
    })
}

fn hue_picker_json(picker: &HuePicker) -> Value {
    json!({ "serial": picker.serial, "graphic": picker.graphic })
}

fn open_url_json(request: &OpenUrlRequest) -> Value {
    json!({ "seq": request.seq, "url": request.url })
}

fn tip_json(tip: &TipNotice) -> Value {
    json!({
        "seq": tip.seq,
        "tip": tip.tip,
        "kind": tip.kind.as_str(),
        "text": tip.text,
    })
}

fn text_entry_dialog_json(dialog: &TextEntryDialog) -> Value {
    json!({
        "seq": dialog.seq,
        "serial": dialog.serial,
        "parent_id": dialog.parent_id,
        "button_id": dialog.button_id,
        "text": dialog.text,
        "can_close": dialog.can_close,
        "variant": dialog.variant,
        "max_length": dialog.max_length,
        "description": dialog.description,
    })
}

fn book_json(b: &Book) -> Value {
    json!({
        "serial": b.serial, "title": b.title, "author": b.author,
        "writable": b.writable, "page_count": b.page_count, "pages": b.pages,
    })
}

fn party_json(p: &Party) -> Value {
    json!({ "members": p.members, "leader": p.leader, "pending_invite": p.pending_invite })
}

fn weather_json(w: &Weather) -> Value {
    json!({ "kind": w.kind, "intensity": w.intensity })
}

fn waypoint_json(w: &WaypointView) -> Value {
    json!({
        "serial": w.serial,
        "pos": pos_json(&w.pos),
        "map": w.map,
        "kind": w.kind,
        "ignore_object": w.ignore_object,
        "cliloc": w.cliloc,
        "name": w.name,
        "distance": w.distance,
    })
}

/// A raw OPL property line: `{"cliloc": id, "args": "tab\tseparated"}` — left
/// unresolved (no Cliloc table in this bridge; see the module doc).
fn opl_line_json((cliloc, args): &(u32, String)) -> Value {
    json!({ "cliloc": cliloc, "args": args })
}

/// Serialize an [`Observation`] to the brain-facing JSON shape — see this
/// module's top doc comment for the full key list + versioning note.
pub fn observation_to_json(obs: &Observation) -> Value {
    let corpse_of: Vec<Value> = obs
        .corpse_of
        .iter()
        .map(|(corpse, killed)| json!({ "corpse": corpse, "killed": killed }))
        .collect();
    let corpse_equip: Vec<Value> = obs
        .corpse_equip
        .iter()
        .map(|(corpse, entries)| {
            let entries: Vec<Value> = entries
                .iter()
                .map(|(layer, serial)| json!({ "layer": layer, "serial": serial }))
                .collect();
            json!({ "corpse": corpse, "entries": entries })
        })
        .collect();
    let opl: Vec<Value> = obs
        .opl
        .iter()
        .map(|(serial, lines)| {
            json!({ "serial": serial, "lines": lines.iter().map(opl_line_json).collect::<Vec<_>>() })
        })
        .collect();
    let recent_damage: Vec<Value> = obs
        .recent_damage
        .iter()
        .map(|&(seq, serial, amount)| json!({ "seq": seq, "serial": serial, "amount": amount }))
        .collect();
    json!({
        "player": player_json(&obs.player),
        "mobiles": obs.mobiles.iter().map(mobile_json).collect::<Vec<_>>(),
        "items": obs.items.iter().map(item_json).collect::<Vec<_>>(),
        "new_journal": obs.new_journal.iter().map(journal_json).collect::<Vec<_>>(),
        "pending_target": obs.pending_target.as_ref().map(target_json),
        "skills": obs.skills.iter().map(skill_json).collect::<Vec<_>>(),
        "gumps": obs.gumps.iter().map(gump_json).collect::<Vec<_>>(),
        "prompt": obs.prompt.as_ref().map(prompt_json),
        "trades": obs.trades.iter().map(trade_json).collect::<Vec<_>>(),
        "buffs": obs.buffs.iter().map(buff_json).collect::<Vec<_>>(),
        "shop_buy": obs.shop_buy.as_ref().map(shop_buy_json),
        "shop_sell": obs.shop_sell.as_ref().map(shop_sell_json),
        "popup": obs.popup.as_ref().map(popup_json),
        "legacy_menus": obs.legacy_menus.iter().map(legacy_menu_json).collect::<Vec<_>>(),
        "hue_pickers": obs.hue_pickers.iter().map(hue_picker_json).collect::<Vec<_>>(),
        "open_urls": obs.open_urls.iter().map(open_url_json).collect::<Vec<_>>(),
        "tips": obs.tips.iter().map(tip_json).collect::<Vec<_>>(),
        "text_entry_dialogs": obs.text_entry_dialogs.iter().map(text_entry_dialog_json).collect::<Vec<_>>(),
        "book": obs.book.as_ref().map(book_json),
        "party": party_json(&obs.party),
        "quest_arrow": obs.quest_arrow.map(|(x, y)| json!({ "x": x, "y": y })),
        "waypoints": obs.waypoints.iter().map(waypoint_json).collect::<Vec<_>>(),
        "weather": weather_json(&obs.weather),
        "season": obs.season,
        "light": obs.light,
        "war": obs.war,
        "last_attack": obs.last_attack,
        "combatant": obs.combatant,
        "corpse_of": corpse_of,
        "corpse_equip": corpse_equip,
        "map_index": obs.map_index,
        "aos": obs.aos,
        "opl": opl,
        "recent_damage": recent_damage,
        "spellbooks": obs.spellbooks.iter().map(spellbook_json).collect::<Vec<_>>(),
        "map_gumps": obs.map_gumps.iter().map(map_view_json).collect::<Vec<_>>(),
    })
}

/// Parse a vendor transaction's `items` array into `(serial, amount)` pairs.
/// Accepts either `[{"serial": .., "amount": ..}, ..]` or `[[serial, amount], ..]`.
fn shop_items_from_json(v: &Value) -> Vec<(u32, u16)> {
    let Some(arr) = v.get("items").and_then(Value::as_array) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|e| {
            if let Some(o) = e.as_object() {
                let serial = o.get("serial").and_then(Value::as_u64)? as u32;
                let amount = o.get("amount").and_then(Value::as_u64).unwrap_or(1) as u16;
                Some((serial, amount))
            } else if let Some(pair) = e.as_array() {
                let serial = pair.first().and_then(Value::as_u64)? as u32;
                let amount = pair.get(1).and_then(Value::as_u64).unwrap_or(1) as u16;
                Some((serial, amount))
            } else {
                None
            }
        })
        .collect()
}

/// Parse a gump response's `entries` array into `(entryId, text)` pairs. Accepts
/// either `[{"id": .., "text": ".."}, ..]` or `[[id, "text"], ..]`.
fn gump_entries_from_json(v: &Value) -> Vec<(u16, String)> {
    let Some(arr) = v.get("entries").and_then(Value::as_array) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|e| {
            if let Some(o) = e.as_object() {
                let id = o.get("id").and_then(Value::as_u64)? as u16;
                let text = o
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                Some((id, text))
            } else if let Some(pair) = e.as_array() {
                let id = pair.first().and_then(Value::as_u64)? as u16;
                let text = pair
                    .get(1)
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                Some((id, text))
            } else {
                None
            }
        })
        .collect()
}

/// Parse an [`Action`] from its JSON form (externally tagged by `"type"`).
/// Every [`Action`] variant round-trips through this (see the table-driven
/// test below) — keep this match exhaustive as the enum grows.
pub fn action_from_json(v: &Value) -> Result<Action, String> {
    let t = v
        .get("type")
        .and_then(Value::as_str)
        .ok_or("action missing 'type'")?;
    let u32f = |k: &str| v.get(k).and_then(Value::as_u64).map(|n| n as u32);
    let req_u32 = |k: &str| u32f(k).ok_or_else(|| format!("action {t} missing u32 '{k}'"));
    let req_u64 = |k: &str| {
        v.get(k)
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("action {t} missing u64 '{k}'"))
    };
    // A missing/mistyped (wrong key case, float) coordinate must error rather
    // than silently default to 0 — that would walk the player to the map
    // origin on a malformed request instead of surfacing the bad input.
    let req_u16 = |k: &str| {
        v.get(k)
            .and_then(Value::as_u64)
            .map(|n| n as u16)
            .ok_or_else(|| format!("action {t} missing u16 '{k}'"))
    };
    let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    match t {
        "Walk" => Ok(Action::Walk {
            dir: v.get("dir").and_then(Value::as_u64).unwrap_or(0) as u8,
            run: v.get("run").and_then(Value::as_bool).unwrap_or(false),
        }),
        "WalkTo" => Ok(Action::WalkTo {
            x: req_u16("x")?,
            y: req_u16("y")?,
        }),
        "Say" => Ok(Action::Say { text: text("text") }),
        "PartySay" => Ok(Action::PartySay { text: text("text") }),
        "Attack" => Ok(Action::Attack {
            serial: req_u32("serial")?,
        }),
        "AutoAttack" => Ok(Action::AutoAttack),
        "AttackLast" => Ok(Action::AttackLast),
        "Use" => Ok(Action::Use {
            serial: req_u32("serial")?,
        }),
        "Click" => Ok(Action::Click {
            serial: req_u32("serial")?,
        }),
        "PickUp" => Ok(Action::PickUp {
            serial: req_u32("serial")?,
            amount: v.get("amount").and_then(Value::as_u64).unwrap_or(1) as u16,
        }),
        "Drop" => Ok(Action::Drop {
            serial: req_u32("serial")?,
            x: v.get("x").and_then(Value::as_u64).unwrap_or(0) as u16,
            y: v.get("y").and_then(Value::as_u64).unwrap_or(0) as u16,
            z: v.get("z").and_then(Value::as_i64).unwrap_or(0) as i16,
            container: u32f("container").unwrap_or(0xFFFF_FFFF),
        }),
        "Equip" => Ok(Action::Equip {
            serial: req_u32("serial")?,
            layer: v.get("layer").and_then(Value::as_u64).unwrap_or(0) as u8,
        }),
        "WarMode" => Ok(Action::WarMode {
            on: v.get("on").and_then(Value::as_bool).unwrap_or(false),
        }),
        "CastSpell" => Ok(Action::CastSpell {
            spell: v.get("spell").and_then(Value::as_u64).unwrap_or(0) as u16,
        }),
        "TargetObject" => Ok(Action::TargetObject {
            serial: req_u32("serial")?,
        }),
        "TargetGround" => Ok(Action::TargetGround {
            x: v.get("x").and_then(Value::as_u64).unwrap_or(0) as u16,
            y: v.get("y").and_then(Value::as_u64).unwrap_or(0) as u16,
            z: v.get("z").and_then(Value::as_i64).unwrap_or(0) as i16,
            graphic: v.get("graphic").and_then(Value::as_u64).unwrap_or(0) as u16,
        }),
        "TargetCancel" => Ok(Action::TargetCancel),
        "BuyItems" => Ok(Action::BuyItems {
            vendor: req_u32("vendor")?,
            items: shop_items_from_json(v),
        }),
        "SellItems" => Ok(Action::SellItems {
            vendor: req_u32("vendor")?,
            items: shop_items_from_json(v),
        }),
        "BookRequest" => Ok(Action::BookRequest {
            serial: req_u32("serial")?,
            pages: v.get("pages").and_then(Value::as_u64).unwrap_or(0) as u16,
        }),
        "UseAbility" => Ok(Action::UseAbility {
            ability: v.get("ability").and_then(Value::as_u64).unwrap_or(0) as u8,
        }),
        "SkillLock" => Ok(Action::SkillLock {
            skill: v.get("skill").and_then(Value::as_u64).unwrap_or(0) as u16,
            lock: v.get("lock").and_then(Value::as_u64).unwrap_or(0) as u8,
        }),
        "UseSkill" => Ok(Action::UseSkill {
            skill: v.get("skill").and_then(Value::as_u64).unwrap_or(0) as u16,
        }),
        "OplRequest" => Ok(Action::OplRequest {
            serial: req_u32("serial")?,
        }),
        "PartyInvite" => Ok(Action::PartyInvite),
        "PartyLeave" => Ok(Action::PartyLeave),
        "PartyAccept" => Ok(Action::PartyAccept {
            leader: req_u32("leader")?,
        }),
        "PartyDecline" => Ok(Action::PartyDecline {
            leader: req_u32("leader")?,
        }),
        "PopupRequest" => Ok(Action::PopupRequest {
            serial: req_u32("serial")?,
        }),
        "PopupSelect" => Ok(Action::PopupSelect {
            serial: req_u32("serial")?,
            index: v.get("index").and_then(Value::as_u64).unwrap_or(0) as u16,
        }),
        "LegacyMenuSelect" => Ok(Action::LegacyMenuSelect {
            serial: req_u32("serial")?,
            index: req_u16("index")?,
        }),
        "HuePickerSelect" => Ok(Action::HuePickerSelect {
            serial: req_u32("serial")?,
            hue: req_u16("hue")?,
        }),
        "GumpResponse" => Ok(Action::GumpResponse {
            serial: req_u32("serial")?,
            gump_id: req_u32("gump_id")?,
            button: u32f("button").unwrap_or(0),
            switches: v
                .get("switches")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_u64().map(|n| n as u32))
                        .collect()
                })
                .unwrap_or_default(),
            entries: gump_entries_from_json(v),
        }),
        "PromptResponse" => Ok(Action::PromptResponse { text: text("text") }),
        "PromptCancel" => Ok(Action::PromptCancel),
        "TipNavigate" => Ok(Action::TipNavigate {
            seq: req_u64("seq")?,
            next: v.get("next").and_then(Value::as_bool).unwrap_or(false),
        }),
        "TipClose" => Ok(Action::TipClose {
            seq: req_u64("seq")?,
        }),
        "TextEntryResponse" => Ok(Action::TextEntryResponse {
            seq: req_u64("seq")?,
            text: text("text"),
            accepted: v.get("accepted").and_then(Value::as_bool).unwrap_or(true),
        }),
        "TextEntryClose" => Ok(Action::TextEntryClose {
            seq: req_u64("seq")?,
        }),
        "TradeAccept" => Ok(Action::TradeAccept {
            container: req_u32("container")?,
            accept: v.get("accept").and_then(Value::as_bool).unwrap_or(true),
        }),
        "TradeCancel" => Ok(Action::TradeCancel {
            container: req_u32("container")?,
        }),
        "TradeGold" => Ok(Action::TradeGold {
            container: req_u32("container")?,
            gold: u32f("gold").unwrap_or(0),
            platinum: u32f("platinum").unwrap_or(0),
        }),
        other => Err(format!("unknown action type: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anima_core::world::{PromptKind, TipKind};

    /// Table-driven: every [`Action`] variant round-trips through
    /// `{"type": ..} -> action_from_json -> Action`. Add a row here whenever
    /// the enum grows a variant.
    #[test]
    fn action_from_json_covers_every_variant() {
        let cases: Vec<(Value, Action)> = vec![
            (
                json!({"type": "Walk", "dir": 3, "run": true}),
                Action::Walk { dir: 3, run: true },
            ),
            (
                json!({"type": "WalkTo", "x": 1200, "y": 800}),
                Action::WalkTo { x: 1200, y: 800 },
            ),
            (
                json!({"type": "Say", "text": "hi"}),
                Action::Say { text: "hi".into() },
            ),
            (
                json!({"type": "PartySay", "text": "go"}),
                Action::PartySay { text: "go".into() },
            ),
            (
                json!({"type": "Attack", "serial": 42}),
                Action::Attack { serial: 42 },
            ),
            (json!({"type": "AutoAttack"}), Action::AutoAttack),
            (json!({"type": "AttackLast"}), Action::AttackLast),
            (
                json!({"type": "Use", "serial": 7}),
                Action::Use { serial: 7 },
            ),
            (
                json!({"type": "Click", "serial": 7}),
                Action::Click { serial: 7 },
            ),
            (
                json!({"type": "PickUp", "serial": 1073741825u64, "amount": 5}),
                Action::PickUp {
                    serial: 0x4000_0001,
                    amount: 5,
                },
            ),
            (
                json!({"type": "Drop", "serial": 9, "x": 10, "y": 20, "z": -3, "container": 99}),
                Action::Drop {
                    serial: 9,
                    x: 10,
                    y: 20,
                    z: -3,
                    container: 99,
                },
            ),
            (
                json!({"type": "Equip", "serial": 9, "layer": 2}),
                Action::Equip {
                    serial: 9,
                    layer: 2,
                },
            ),
            (
                json!({"type": "WarMode", "on": true}),
                Action::WarMode { on: true },
            ),
            (
                json!({"type": "CastSpell", "spell": 8}),
                Action::CastSpell { spell: 8 },
            ),
            (
                json!({"type": "TargetObject", "serial": 4242}),
                Action::TargetObject { serial: 4242 },
            ),
            (
                json!({"type": "TargetGround", "x": 1000, "y": 2000, "z": -5, "graphic": 420}),
                Action::TargetGround {
                    x: 1000,
                    y: 2000,
                    z: -5,
                    graphic: 420,
                },
            ),
            (json!({"type": "TargetCancel"}), Action::TargetCancel),
            (
                json!({"type": "BuyItems", "vendor": 5, "items": [[1, 2]]}),
                Action::BuyItems {
                    vendor: 5,
                    items: vec![(1, 2)],
                },
            ),
            (
                json!({"type": "SellItems", "vendor": 5, "items": [{"serial": 1, "amount": 3}]}),
                Action::SellItems {
                    vendor: 5,
                    items: vec![(1, 3)],
                },
            ),
            (
                json!({"type": "BookRequest", "serial": 3, "pages": 2}),
                Action::BookRequest {
                    serial: 3,
                    pages: 2,
                },
            ),
            (
                json!({"type": "UseAbility", "ability": 4}),
                Action::UseAbility { ability: 4 },
            ),
            (
                json!({"type": "SkillLock", "skill": 10, "lock": 2}),
                Action::SkillLock { skill: 10, lock: 2 },
            ),
            (
                json!({"type": "UseSkill", "skill": 21}),
                Action::UseSkill { skill: 21 },
            ),
            (
                json!({"type": "OplRequest", "serial": 8}),
                Action::OplRequest { serial: 8 },
            ),
            (json!({"type": "PartyInvite"}), Action::PartyInvite),
            (json!({"type": "PartyLeave"}), Action::PartyLeave),
            (
                json!({"type": "PartyAccept", "leader": 11}),
                Action::PartyAccept { leader: 11 },
            ),
            (
                json!({"type": "PartyDecline", "leader": 11}),
                Action::PartyDecline { leader: 11 },
            ),
            (
                json!({"type": "PopupRequest", "serial": 6}),
                Action::PopupRequest { serial: 6 },
            ),
            (
                json!({"type": "PopupSelect", "serial": 6, "index": 1}),
                Action::PopupSelect {
                    serial: 6,
                    index: 1,
                },
            ),
            (
                json!({"type": "LegacyMenuSelect", "serial": 9, "index": 2}),
                Action::LegacyMenuSelect {
                    serial: 9,
                    index: 2,
                },
            ),
            (
                json!({"type": "HuePickerSelect", "serial": 10, "hue": 902}),
                Action::HuePickerSelect {
                    serial: 10,
                    hue: 902,
                },
            ),
            (
                json!({"type": "GumpResponse", "serial": 1, "gump_id": 2, "button": 3,
                       "switches": [1, 2], "entries": [[4, "hi"]]}),
                Action::GumpResponse {
                    serial: 1,
                    gump_id: 2,
                    button: 3,
                    switches: vec![1, 2],
                    entries: vec![(4, "hi".into())],
                },
            ),
            (
                json!({"type": "PromptResponse", "text": "Fido"}),
                Action::PromptResponse {
                    text: "Fido".into(),
                },
            ),
            (json!({"type": "PromptCancel"}), Action::PromptCancel),
            (
                json!({"type": "TipNavigate", "seq": 9, "next": true}),
                Action::TipNavigate { seq: 9, next: true },
            ),
            (
                json!({"type": "TipClose", "seq": 10}),
                Action::TipClose { seq: 10 },
            ),
            (
                json!({"type": "TextEntryResponse", "seq": 11, "text": "123", "accepted": false}),
                Action::TextEntryResponse {
                    seq: 11,
                    text: "123".into(),
                    accepted: false,
                },
            ),
            (
                json!({"type": "TextEntryClose", "seq": 12}),
                Action::TextEntryClose { seq: 12 },
            ),
            (
                json!({"type": "TradeAccept", "container": 55, "accept": true}),
                Action::TradeAccept {
                    container: 55,
                    accept: true,
                },
            ),
            (
                json!({"type": "TradeCancel", "container": 55}),
                Action::TradeCancel { container: 55 },
            ),
            (
                json!({"type": "TradeGold", "container": 55, "gold": 100, "platinum": 1}),
                Action::TradeGold {
                    container: 55,
                    gold: 100,
                    platinum: 1,
                },
            ),
        ];
        for (json, expected) in cases {
            let got = action_from_json(&json).unwrap_or_else(|e| panic!("{json} -> err {e}"));
            assert_eq!(got, expected, "mismatch for {json}");
        }

        assert!(action_from_json(&json!({"type": "Nope"})).is_err());
        assert!(action_from_json(&json!({})).is_err());

        // A malformed WalkTo must error, not silently walk to the map origin.
        assert!(
            action_from_json(&json!({"type": "WalkTo", "y": 800})).is_err(),
            "WalkTo missing x must error"
        );
        assert!(
            action_from_json(&json!({"type": "WalkTo", "x": 12.5, "y": 800})).is_err(),
            "WalkTo with a non-integer x must error"
        );
    }

    #[test]
    fn observation_serializes_pending_target() {
        use anima_core::world::{TargetCursor, World};
        let mut w = World::default();
        w.pending_target = Some(TargetCursor {
            target_type: 1,
            cursor_id: 0xABCD,
            cursor_flag: 0,
        });
        let v = observation_to_json(&w.observe(&mut 0));
        assert_eq!(v["pending_target"]["cursor_id"], 0xABCD);
        assert_eq!(v["pending_target"]["target_type"], 1);
    }

    #[test]
    fn observation_serializes_player_survival_state_explicitly() {
        let obs = Observation {
            player: PlayerView {
                body: 0x192,
                poisoned: true,
                dead: true,
                ..PlayerView::default()
            },
            ..Observation::default()
        };
        let v = observation_to_json(&obs);
        assert_eq!(v["player"]["body"], 0x192);
        assert_eq!(v["player"]["poisoned"], true);
        assert_eq!(v["player"]["dead"], true);

        let default = observation_to_json(&Observation::default());
        assert_eq!(default["player"]["poisoned"], false);
        assert_eq!(default["player"]["dead"], false);
    }

    #[test]
    fn schema_v14_retains_waypoint_exact_shape() {
        assert_eq!(SCHEMA_VERSION, 14);
        let obs = Observation {
            waypoints: vec![WaypointView {
                serial: 0x1234_5678,
                pos: Position {
                    x: 2588,
                    y: 406,
                    z: -7,
                },
                map: 2,
                kind: 6,
                ignore_object: true,
                cliloc: 1_060_023,
                name: "Wandering Healer".into(),
                distance: 17,
            }],
            ..Observation::default()
        };

        assert_eq!(
            observation_to_json(&obs)["waypoints"],
            json!([{
                "serial": 0x1234_5678,
                "pos": {"x": 2588, "y": 406, "z": -7},
                "map": 2,
                "kind": 6,
                "ignore_object": true,
                "cliloc": 1_060_023,
                "name": "Wandering Healer",
                "distance": 17,
            }])
        );
    }

    #[test]
    fn schema_v9_serializes_legacy_menus_with_one_based_entries() {
        let obs = Observation {
            legacy_menus: vec![LegacyMenu {
                serial: 0x0102_0304,
                menu_id: 7,
                question: "Choose".into(),
                kind: LegacyMenuKind::Items,
                entries: vec![LegacyMenuEntry {
                    graphic: 0x0F5E,
                    hue: 0x0481,
                    text: "Sword".into(),
                }],
            }],
            ..Observation::default()
        };
        assert_eq!(
            observation_to_json(&obs)["legacy_menus"],
            json!([{
                "serial": 0x0102_0304u32,
                "menu_id": 7,
                "question": "Choose",
                "kind": "items",
                "entries": [{ "index": 1, "graphic": 0x0F5E, "hue": 0x0481, "text": "Sword" }],
            }])
        );
    }

    #[test]
    fn schema_v13_serializes_hue_pickers_exact_shape() {
        let obs = Observation {
            hue_pickers: vec![HuePicker {
                serial: 0x0102_0304,
                graphic: 0x0FAB,
            }],
            ..Observation::default()
        };
        assert_eq!(
            observation_to_json(&obs)["hue_pickers"],
            json!([{ "serial": 0x0102_0304u32, "graphic": 0x0FAB }])
        );
    }

    #[test]
    fn schema_v13_serializes_prompt_kind_exactly() {
        for (kind, expected) in [
            (PromptKind::Ascii, "ascii"),
            (PromptKind::Unicode, "unicode"),
        ] {
            let obs = Observation {
                prompt: Some(PromptState {
                    sender_serial: 0x0102_0304,
                    prompt_id: 0xDEAD_BEEF,
                    kind,
                }),
                ..Observation::default()
            };
            assert_eq!(
                observation_to_json(&obs)["prompt"],
                json!({
                    "sender_serial": 0x0102_0304u32,
                    "prompt_id": 0xDEAD_BEEFu32,
                    "kind": expected,
                })
            );
        }
    }

    #[test]
    fn schema_v13_serializes_validated_open_url_events_exactly() {
        let obs = Observation {
            open_urls: vec![
                OpenUrlRequest {
                    seq: 7,
                    url: "https://uo.com/news".into(),
                },
                OpenUrlRequest {
                    seq: 8,
                    url: "http://localhost:8080/help".into(),
                },
            ],
            ..Observation::default()
        };
        assert_eq!(
            observation_to_json(&obs)["open_urls"],
            json!([
                { "seq": 7, "url": "https://uo.com/news" },
                { "seq": 8, "url": "http://localhost:8080/help" },
            ])
        );
    }

    #[test]
    fn schema_v13_serializes_tip_and_notice_windows_exactly() {
        let obs = Observation {
            tips: vec![
                TipNotice {
                    seq: 3,
                    tip: 0x1234_5678,
                    kind: TipKind::Tip,
                    text: "First\nSecond €".into(),
                },
                TipNotice {
                    seq: 4,
                    tip: 9,
                    kind: TipKind::Notice,
                    text: "Maintenance".into(),
                },
            ],
            ..Observation::default()
        };
        assert_eq!(
            observation_to_json(&obs)["tips"],
            json!([
                { "seq": 3, "tip": 0x1234_5678u32, "kind": "tip", "text": "First\nSecond €" },
                { "seq": 4, "tip": 9, "kind": "notice", "text": "Maintenance" },
            ])
        );
    }

    #[test]
    fn schema_v14_serializes_text_entry_dialogs_exactly() {
        let obs = Observation {
            text_entry_dialogs: vec![TextEntryDialog {
                seq: 7,
                serial: 0x0102_0304,
                parent_id: 5,
                button_id: 6,
                text: "Account €".into(),
                can_close: true,
                variant: 2,
                max_length: 12,
                description: "Digits only".into(),
            }],
            ..Observation::default()
        };
        assert_eq!(
            observation_to_json(&obs)["text_entry_dialogs"],
            json!([{
                "seq": 7,
                "serial": 0x0102_0304u32,
                "parent_id": 5,
                "button_id": 6,
                "text": "Account €",
                "can_close": true,
                "variant": 2,
                "max_length": 12,
                "description": "Digits only",
            }])
        );
    }

    #[test]
    fn observation_json_has_expected_keys() {
        let obs = Observation::default();
        let v = observation_to_json(&obs);
        for k in [
            "player",
            "mobiles",
            "items",
            "new_journal",
            "pending_target",
            "skills",
            "gumps",
            "prompt",
            "trades",
            "buffs",
            "shop_buy",
            "shop_sell",
            "popup",
            "legacy_menus",
            "hue_pickers",
            "open_urls",
            "tips",
            "text_entry_dialogs",
            "book",
            "party",
            "quest_arrow",
            "waypoints",
            "weather",
            "season",
            "light",
            "war",
            "last_attack",
            "combatant",
            "corpse_of",
            "corpse_equip",
            "map_index",
            "aos",
            "opl",
            "recent_damage",
            "spellbooks",
            "map_gumps",
        ] {
            assert!(v.get(k).is_some(), "missing key {k}");
        }
        assert!(v["player"].get("hits_max").is_some());
        assert!(v["player"].get("weight_max").is_some());
        assert!(v["player"].get("body").is_some());
        assert!(v["player"].get("poisoned").is_some());
        assert!(v["player"].get("dead").is_some());
        // Nothing open by default: the Option-backed fields serialize to null.
        assert!(v["shop_buy"].is_null());
        assert!(v["book"].is_null());
        assert!(v["popup"].is_null());
        assert!(v["prompt"].is_null());
        assert_eq!(v["legacy_menus"], json!([]));
        assert_eq!(v["hue_pickers"], json!([]));
        assert_eq!(v["open_urls"], json!([]));
        assert_eq!(v["tips"], json!([]));
        assert_eq!(v["text_entry_dialogs"], json!([]));
    }

    /// Schema v5: `items[].is_multi` — a placed boat/house shows up in
    /// `items` (it's a `World::items` entry like any other) but its `graphic`
    /// is a multi id, not an ART id (see [`ItemView`]'s doc); a brain must be
    /// able to tell the two apart from the JSON alone.
    #[test]
    fn item_json_serializes_is_multi() {
        use anima_core::types::Position;

        let normal = ItemView {
            serial: 1,
            graphic: 0x0EED,
            amount: 1,
            pos: Position {
                x: 100,
                y: 100,
                z: 0,
            },
            container: None,
            layer: 0,
            distance: 3,
            is_multi: false,
        };
        let multi = ItemView {
            serial: 2,
            graphic: 0x0002, // a real ART id too — the collision `is_multi` guards against
            amount: 1,
            pos: Position {
                x: 1492,
                y: 1760,
                z: 0,
            },
            container: None,
            layer: 0,
            distance: 5,
            is_multi: true,
        };
        assert_eq!(item_json(&normal)["is_multi"], false);
        assert_eq!(item_json(&multi)["is_multi"], true);
        assert_eq!(
            item_json(&multi)["graphic"],
            2,
            "graphic is still emitted — just not an ART id here"
        );
    }

    #[test]
    fn observation_serializes_opl_and_recent_damage() {
        use anima_core::world::World;
        let mut w = World::default();
        w.set_opl(0xABCD, 1, vec![(1042971, "".into()), (1060451, "3".into())]);
        w.push_damage(0xABCD, 12);
        w.push_damage(0xABCD, 7);
        let v = observation_to_json(&w.observe(&mut 0));

        assert_eq!(v["opl"][0]["serial"], 0xABCD);
        assert_eq!(v["opl"][0]["lines"][0]["cliloc"], 1042971);
        assert_eq!(v["opl"][0]["lines"][1]["args"], "3");

        assert_eq!(v["recent_damage"].as_array().unwrap().len(), 2);
        assert_eq!(v["recent_damage"][0]["serial"], 0xABCD);
        assert_eq!(v["recent_damage"][0]["amount"], 12);
        assert_eq!(v["recent_damage"][1]["amount"], 7);
        // Monotonic seq lets a brain dedupe across polls.
        assert!(
            v["recent_damage"][1]["seq"].as_u64().unwrap()
                > v["recent_damage"][0]["seq"].as_u64().unwrap()
        );
    }

    #[test]
    fn observation_serializes_spellbooks_split_lo_hi() {
        use anima_core::world::World;
        let mut w = World::default();
        // Magery book (graphic 0x0EFA), offset 1 (BookOffset(0)+1), a mask with
        // bit 0 set (spell 1) and bit 40 set (well past JS's 32-bit range) — the
        // split lo/hi halves must each carry the right bits.
        let content: u64 = 1 | (1u64 << 40);
        w.set_spellbook_content(0x4000_0010, 0x0EFA, 1, content);
        let v = observation_to_json(&w.observe(&mut 0));

        assert_eq!(v["spellbooks"].as_array().unwrap().len(), 1);
        let sb = &v["spellbooks"][0];
        assert_eq!(sb["serial"], 0x4000_0010);
        assert_eq!(sb["graphic"], 0x0EFA);
        assert_eq!(sb["offset"], 1);
        assert_eq!(sb["lo"], 1u64);
        assert_eq!(sb["hi"], 1u64 << 8); // bit 40 overall == bit 8 of the high half
    }

    #[test]
    fn observation_json_spellbooks_empty_by_default() {
        let v = observation_to_json(&Observation::default());
        assert!(v["spellbooks"].as_array().unwrap().is_empty());
    }

    /// Schema v6: `map_gumps` — a treasure/decoration map window (0x90/0xF5 +
    /// 0x56), with pins carried straight through in pixel space.
    #[test]
    fn observation_serializes_map_gumps() {
        use anima_core::world::World;
        let mut w = World::default();
        w.set_map_view(0x4000_1234, 0x139D, 3, 520, 0, 2580, 2050, 400, 400);
        w.apply_map_command(0x4000_1234, 1, 0, 100, 120); // the chest pin
        w.apply_map_command(0x4000_1234, 7, 1, 0, 0); // editable = true
        let v = observation_to_json(&w.observe(&mut 0));

        assert_eq!(v["map_gumps"].as_array().unwrap().len(), 1);
        let mv = &v["map_gumps"][0];
        assert_eq!(mv["serial"], 0x4000_1234);
        assert_eq!(mv["open_seq"], 1);
        assert_eq!(mv["gump_art"], 0x139D);
        assert_eq!(mv["facet"], 3);
        assert_eq!(mv["bounds"]["min_x"], 520);
        assert_eq!(mv["bounds"]["max_y"], 2050);
        assert_eq!(
            (
                mv["width"].as_u64().unwrap(),
                mv["height"].as_u64().unwrap()
            ),
            (400, 400)
        );
        assert_eq!(mv["pins"][0][0], 100);
        assert_eq!(mv["pins"][0][1], 120);
        assert_eq!(mv["editable"], true);
    }

    #[test]
    fn observation_json_map_gumps_empty_by_default() {
        let v = observation_to_json(&Observation::default());
        assert!(v["map_gumps"].as_array().unwrap().is_empty());
    }

    #[test]
    fn gump_element_serializes_cliloc_reference_unresolved() {
        use anima_core::gump_layout::{GumpElement, HtmlText};
        let el = GumpElement::Html {
            x: 1,
            y: 2,
            w: 100,
            h: 20,
            text: HtmlText::Cliloc {
                id: 1042971,
                args: Some("a\tb".into()),
            },
            page: 0,
        };
        let v = gump_element_json(&el);
        assert_eq!(v["type"], "html");
        assert_eq!(v["text"]["cliloc"]["id"], 1042971);
        assert_eq!(v["text"]["cliloc"]["args"], "a\tb");
    }
}
