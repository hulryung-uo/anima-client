//! Server-driven windows, as the JSON the browser renders.
//!
//! Gumps and everything gump-shaped: menus, prompts, tips, profiles, books,
//! trades, vendor windows, popups, maps. The layout *grammar* is parsed in
//! `anima_core::gump_layout` (it is protocol data a brain can read too); this
//! module is only the projection of that into the renderer's shape.

use super::*;

/// Convert a core-parsed [`GumpElement`] into the renderer's positioned JSON
/// shape (`t`/`x`/`y`/…). The grammar itself now lives in
/// [`anima_core::gump_layout`] (it's protocol data, not rendering); this is
/// just the JSON shaping plus cliloc_table resolution (which needs
/// `anima_assets::Cliloc`, unavailable to the zero-dep core) — ported
/// unchanged from the old inline `parse_gump_layout` so the scene JSON this
/// produces is byte-for-byte identical to before the split.
pub(super) fn gump_element_json(e: &GumpElement, cliloc_table: Option<&Cliloc>) -> Value {
    match e {
        GumpElement::Background { x, y, w, h, page } => {
            json!({"t":"bg","x":x,"y":y,"w":w,"h":h,"page":page})
        }
        // Decorative art — we draw a plain marker, so the gump id isn't needed.
        GumpElement::Image { x, y, page, .. } => json!({"t":"bg","x":x,"y":y,"page":page}),
        // `graphic` (the normal-state art) lets the client draw the real button
        // art (a small gump) instead of the raw reply id as text.
        GumpElement::Button {
            x,
            y,
            graphic,
            reply_id,
            pageflag,
            param,
            page,
        } => json!({
            "t":"button","x":x,"y":y,"g":graphic,"id":reply_id,"page":page,
            "pageflag":pageflag,"param":param,
        }),
        GumpElement::Text {
            x,
            y,
            w: None,
            s,
            page,
        } => {
            json!({"t":"text","x":x,"y":y,"s":s,"page":page})
        }
        GumpElement::Text {
            x,
            y,
            w: Some(w),
            s,
            page,
        } => {
            json!({"t":"text","x":x,"y":y,"w":w,"s":s,"page":page})
        }
        // Resolve against the Cliloc table so NPC dialogs show real text, not
        // #ids. Shaped as the SAME "t":"text" JSON as a plain Text element
        // (deliberately — `w` is always `Some` for an html block, so the
        // client's one `e.t === "text"` branch in `renderGumpHtml` handles
        // both). Any UO gump-HTML tags/entities in `s` (`<CENTER>`, `&amp;`,
        // …) are left as-is for the client to interpret — see
        // `GumpElement::Html`'s doc.
        GumpElement::Html {
            x,
            y,
            w,
            text,
            page,
            ..
        } => {
            let s = match text {
                HtmlText::Literal(s) => s.clone(),
                HtmlText::Cliloc {
                    id,
                    args: Some(args),
                } => cliloc_table
                    .and_then(|c| c.format(*id, args))
                    .unwrap_or_else(|| format!("#{id}")),
                HtmlText::Cliloc { id, args: None } => cliloc_table
                    .and_then(|c| c.get(*id).map(str::to_string))
                    .unwrap_or_else(|| format!("#{id}")),
            };
            json!({"t":"text","x":x,"y":y,"w":w,"s":s,"page":page})
        }
        GumpElement::Check { x, y, id, on, page } => {
            json!({"t":"check","x":x,"y":y,"id":id,"on":on,"page":page})
        }
        // `g` (group): radios are mutually exclusive only within one, so the
        // renderer needs it to keep two answers to one question apart.
        GumpElement::Radio {
            x,
            y,
            id,
            on,
            group,
            page,
        } => {
            json!({"t":"radio","x":x,"y":y,"id":id,"on":on,"g":group,"page":page})
        }
        GumpElement::Entry {
            x,
            y,
            w,
            id,
            s,
            limit,
            page,
        } => {
            json!({"t":"entry","x":x,"y":y,"w":w,"id":id,"s":s,"lim":limit,"page":page})
        }
        GumpElement::TilePic {
            x,
            y,
            graphic,
            hue,
            page,
        } => {
            json!({"t":"tilepic","x":x,"y":y,"g":graphic,"hue":hue,"page":page})
        }
        GumpElement::TiledImage {
            x,
            y,
            w,
            h,
            graphic,
            page,
        } => {
            json!({"t":"tiled","x":x,"y":y,"w":w,"h":h,"g":graphic,"page":page})
        }
        GumpElement::Translucent { x, y, w, h, page } => {
            json!({"t":"trans","x":x,"y":y,"w":w,"h":h,"page":page})
        }
        GumpElement::PicInPic {
            x,
            y,
            graphic,
            sx,
            sy,
            w,
            h,
            page,
        } => {
            json!({"t":"picinpic","x":x,"y":y,"g":graphic,"sx":sx,"sy":sy,
                   "w":w,"h":h,"page":page})
        }
        GumpElement::ButtonTileArt {
            x,
            y,
            graphic,
            reply_id,
            pageflag,
            param,
            art,
            hue,
            tile_x,
            tile_y,
            page,
        } => {
            json!({"t":"btnart","x":x,"y":y,"g":graphic,"id":reply_id,
                   "pf":pageflag,"param":param,"art":art,"hue":hue,
                   "tx":tile_x,"ty":tile_y,"page":page})
        }
        // Both decorate the element before them — no position of their own.
        GumpElement::Tooltip { cliloc, args, page } => {
            let text = cliloc_text(cliloc, args.as_deref(), cliloc_table);
            json!({"t":"tip","text":text,"page":page})
        }
        GumpElement::ItemProperty { serial, page } => {
            json!({"t":"oplTip","serial":serial,"page":page})
        }
    }
}

/// Resolve a tooltip's cliloc_table the same way an `html` element's is — with args
/// when the server sent them, plain otherwise (see [`HtmlText::Cliloc`]'s doc
/// for why the distinction matters).
fn cliloc_text(id: &u32, args: Option<&str>, table: Option<&Cliloc>) -> String {
    let resolved = match args {
        Some(a) => table.and_then(|c| c.format(*id, a)),
        None => table.and_then(|c| c.get(*id)).map(str::to_string),
    };
    resolved.unwrap_or_else(|| format!("#{id}"))
}

/// Build the `gumps` array for the scene: each open server gump (0xB0/0xDD),
/// its layout parsed by [`gump_layout::parse`] into positioned elements (see
/// [`gump_element_json`]).
pub(super) fn gumps_json(world: &World, cliloc: Option<&Cliloc>) -> String {
    let gumps: Vec<Value> = world
        .gumps
        .iter()
        .map(|g| {
            let layout = gump_layout::parse(&g.layout, &g.text);
            let elements: Vec<Value> = layout
                .elements
                .iter()
                .map(|e| gump_element_json(e, cliloc))
                .collect();
            json!({
                "serial": g.serial, "gumpId": g.gump_id,
                "x": g.x, "y": g.y, "w": layout.width, "h": layout.height,
                "elements": elements,
            })
        })
        .collect();
    json_array(&gumps)
}

/// Build the `popup` object for the scene: the open context menu (0xBF/0x14), or
/// `null` when none. Each entry's `text` is resolved from the Cliloc table (falls
/// back to `#<id>` when the table is missing or the id is unknown).
pub(super) fn popup_json(world: &World, cliloc: Option<&Cliloc>) -> Value {
    match &world.popup {
        None => Value::Null,
        Some(menu) => {
            let entries: Vec<Value> = menu
                .entries
                .iter()
                .map(|e| {
                    let text = cliloc
                        .and_then(|c| c.get(e.cliloc))
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("#{}", e.cliloc));
                    // `hl`: ClassicUO shows entries flagged 0x01 in a highlight hue
                    // (0x0386) — the menu's default/primary action. Pass it through so
                    // the renderer can accent it. (0x02 = submenu arrow; unsupported.)
                    json!({ "index": e.index, "text": text, "hl": e.flags & 0x01 != 0 })
                })
                .collect();
            json!({ "serial": menu.serial, "entries": entries })
        }
    }
}

/// Build every open legacy 0x7C menu in stable serial order. Entry indices are
/// one-based because that is what the matching 0x7D response echoes; zero is
/// reserved for cancel.
pub(super) fn legacy_menus_json(world: &World) -> Value {
    let mut menus: Vec<_> = world.legacy_menus.iter().collect();
    menus.sort_by_key(|menu| menu.serial);
    Value::Array(
        menus
            .into_iter()
            .map(|menu| {
                let kind = match menu.kind {
                    anima_core::world::LegacyMenuKind::Items => "items",
                    anima_core::world::LegacyMenuKind::Question => "question",
                };
                let entries: Vec<Value> = menu
                    .entries
                    .iter()
                    .enumerate()
                    .map(|(index, entry)| {
                        json!({
                            "index": index + 1,
                            "graphic": entry.graphic,
                            "hue": entry.hue,
                            "text": entry.text,
                        })
                    })
                    .collect();
                json!({
                    "serial": menu.serial,
                    "menuId": menu.menu_id,
                    "question": menu.question,
                    "kind": kind,
                    "entries": entries,
                })
            })
            .collect(),
    )
}

/// Build pending server 0x95 hue pickers in stable callback-serial order.
pub(super) fn hue_pickers_json(world: &World) -> Value {
    let mut pickers = world.hue_pickers.clone();
    pickers.sort_by_key(|picker| picker.serial);
    Value::Array(
        pickers
            .into_iter()
            .map(|picker| json!({ "serial": picker.serial, "graphic": picker.graphic }))
            .collect(),
    )
}

/// Open 0xA6 Tip/Notice windows. Arrival order and `seq` are preserved because
/// repeated packets with the same wire tip id are distinct ClassicUO gumps.
pub(super) fn tips_json(world: &World) -> Value {
    Value::Array(
        world
            .tips
            .iter()
            .map(|tip| {
                json!({
                    "seq": tip.seq,
                    "tip": tip.tip,
                    "kind": tip.kind.as_str(),
                    "text": tip.text,
                })
            })
            .collect(),
    )
}

/// Open 0xAB modal text-entry dialogs. Arrival order and local `seq` preserve
/// repeated callback tuples as distinct ClassicUO windows.
pub(super) fn text_entry_dialogs_json(world: &World) -> Value {
    Value::Array(
        world
            .text_entry_dialogs
            .iter()
            .map(|dialog| {
                json!({
                    "seq": dialog.seq,
                    "serial": dialog.serial,
                    "parentId": dialog.parent_id,
                    "buttonId": dialog.button_id,
                    "text": dialog.text,
                    "canClose": dialog.can_close,
                    "variant": dialog.variant,
                    "maxLength": dialog.max_length,
                    "description": dialog.description,
                })
            })
            .collect(),
    )
}

/// Open 0xB8 character profiles. These are persistent windows rather than an
/// event ring: an exact `seq` remains present until the user closes/saves it,
/// and a newer response for the same serial replaces it with a fresh identity.
pub(super) fn character_profiles_json(world: &World) -> Value {
    Value::Array(
        world
            .character_profiles
            .iter()
            .map(|profile| {
                json!({
                    "seq": profile.seq,
                    "serial": profile.serial,
                    "header": profile.header,
                    "footer": profile.footer,
                    "body": profile.body,
                    "canEdit": profile.can_edit,
                })
            })
            .collect(),
    )
}

/// Latest 0xD1 logout permission reply. The play loop, not the renderer,
/// performs an accepted disconnect; a denied reply remains visible long enough
/// for the browser to restore its logout button and explain the refusal.
pub(super) fn logout_ack_json(world: &World) -> Value {
    match world.logout_ack {
        Some(ack) => json!({ "seq": ack.seq, "allowed": ack.allowed }),
        None => Value::Null,
    }
}

/// Bounded 0xF6 history used by the browser to interpolate every member of a
/// boat step from the same source, destination, and server speed.
pub(super) fn boat_movements_json(world: &World) -> Value {
    Value::Array(
        world
            .recent_boat_movements
            .iter()
            .map(|movement| {
                let mut entities = Vec::with_capacity(movement.entities.len() + 1);
                entities.push(json!({
                    "serial": movement.boat_serial,
                    "from": { "x": movement.from.x, "y": movement.from.y, "z": movement.from.z },
                    "to": { "x": movement.to.x, "y": movement.to.y, "z": movement.to.z },
                }));
                entities.extend(movement.entities.iter().map(|entity| {
                    json!({
                        "serial": entity.serial,
                        "from": { "x": entity.from.x, "y": entity.from.y, "z": entity.from.z },
                        "to": { "x": entity.to.x, "y": entity.to.y, "z": entity.to.z },
                    })
                }));
                json!({
                    "seq": movement.seq,
                    "boat": movement.boat_serial,
                    "speed": movement.speed,
                    "dir": movement.moving_direction,
                    "facing": movement.facing_direction,
                    "entities": entities,
                })
            })
            .collect(),
    )
}

/// Build the `party` object for the scene (0xBF/0x06). `leader` is the party
/// leader's serial (0 = none), `members` lists each member `{serial, name, hits,
/// hitsMax}`, and `invite` is the serial of a leader who invited us (0 = none).
/// Member name/hits are resolved from the [`Mobile`] in view — falling back to
/// "Member"/0 when that member isn't currently in range. Always emitted; an empty
/// `members` means we're not in a party.
pub(super) fn party_json(world: &World) -> Value {
    let members: Vec<Value> = world
        .party
        .members
        .iter()
        .map(|&serial| {
            let m = world.mobiles.get(&serial);
            let name = m
                .map(|m| m.name.clone())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| "Member".to_string());
            json!({
                "serial": serial,
                "name": name,
                "hits": m.map_or(0, |m| m.hits),
                "hitsMax": m.map_or(0, |m| m.hits_max),
            })
        })
        .collect();
    json!({
        "leader": world.party.leader,
        "members": members,
        "invite": world.party.pending_invite.unwrap_or(0),
    })
}

/// Maximum number of serials whose OPL (tooltip) lines are emitted per scene, to
/// keep the payload bounded.
pub(super) const OPL_CAP: usize = 64;

/// Build the `opl` object for the scene: each entity's resolved Object Property
/// List (0xD6 MegaCliloc) as an array of display lines `{ "<serial>": ["name",
/// "mod1", …], … }`. Each line is `cliloc.format(id, args)` (falls back to `#<id>`
/// when the table is missing or the id is unknown); empty lines are skipped.
/// Resolved here because the scene has the Cliloc table (the core stores raw ids).
/// Capped at [`OPL_CAP`] serials to keep the scene bounded — preferring serials
/// currently in view (mobiles/ground items near the player).
pub(super) fn opl_json(world: &World, cliloc: Option<&Cliloc>) -> Value {
    let mut map = serde_json::Map::new();
    // Prefer entities the player can actually see: mobiles and ground items.
    let in_view = |s: u32| {
        world.mobiles.contains_key(&s)
            || world.items.get(&s).is_some_and(|it| it.container.is_none())
    };
    let resolve = |lines: &Vec<(u32, String)>| -> Vec<Value> {
        lines
            .iter()
            .filter_map(|(id, args)| {
                let text = cliloc
                    .and_then(|c| c.format(*id, args))
                    .unwrap_or_else(|| format!("#{id}"));
                let text = text.trim();
                if text.is_empty() {
                    None
                } else {
                    Some(Value::String(text.to_string()))
                }
            })
            .collect()
    };
    // Visible serials first, then any remaining, until the cap.
    for (&serial, lines) in world.opl.iter().filter(|(&s, _)| in_view(s)) {
        if map.len() >= OPL_CAP {
            break;
        }
        let resolved = resolve(lines);
        if !resolved.is_empty() {
            map.insert(serial.to_string(), Value::Array(resolved));
        }
    }
    for (&serial, lines) in world.opl.iter().filter(|(&s, _)| !in_view(s)) {
        if map.len() >= OPL_CAP {
            break;
        }
        let resolved = resolve(lines);
        if !resolved.is_empty() {
            map.insert(serial.to_string(), Value::Array(resolved));
        }
    }
    Value::Object(map)
}

/// Build the `trades` array for the scene: every open secure-trade session
/// (0x6F), or `[]` when none — see [`World::trades`]'s doc for why more than
/// one can be open at once (concurrent sessions with different opponents).
/// Items on each side are NOT duplicated here — the client already gets them
/// from `contItems`, filtered by `myCont`/`theirCont` (the trade containers
/// are ordinary container serials), exactly like a normal container window.
/// `myOfferGold`/`myOfferPlat` is what we've offered, `theirOfferGold`/
/// `theirOfferPlat` is the opponent's offer, and `balanceGold`/`balancePlat`
/// is our account balance (an input cap for our own offer, not a trade
/// amount) — see [`crate::world::TradeState`]'s doc for why these three are
/// distinct.
pub(super) fn trades_json(world: &World) -> Value {
    let trades: Vec<Value> = world
        .trades
        .iter()
        .map(|t| {
            json!({
                "opponent": t.opponent_name,
                "opponentSerial": t.opponent_serial,
                "myCont": t.my_container,
                "theirCont": t.their_container,
                "myAccept": t.my_accept,
                "theirAccept": t.their_accept,
                "myOfferGold": t.my_offer_gold,
                "myOfferPlat": t.my_offer_platinum,
                "theirOfferGold": t.their_offer_gold,
                "theirOfferPlat": t.their_offer_platinum,
                "balanceGold": t.balance_gold,
                "balancePlat": t.balance_platinum,
            })
        })
        .collect();
    Value::Array(trades)
}

/// Build the `book` object for the scene: the open book (0x93/0xD4 header + 0x66
/// pages), or `null` when none. `pages` is an array of pages, each an array of its
/// text lines (empty arrays until the page content arrives).
pub(super) fn book_json(world: &World) -> Value {
    match &world.book {
        None => Value::Null,
        Some(b) => json!({
            "serial": b.serial,
            "title": b.title,
            "author": b.author,
            "writable": b.writable,
            "pageCount": b.page_count,
            "pages": b.pages,
        }),
    }
}

/// Build the `prompt` object for the scene: an outstanding 0x9A ASCII or 0xC2
/// Unicode server text prompt, or `{"active":0}` when none. The question text itself
/// already arrived as a journal line (see `World::prompt`'s doc) — the client
/// just needs to know a response is due. `promptId` is included alongside
/// `serial` so the client can tell a fresh, server-chained prompt (ServUO
/// commonly sets the next `Prompt` right inside `OnResponse`) apart from a
/// re-poll of the one it's already showing — the two ids together are the
/// prompt's identity, not just `active`'s edge. Pure (no Session), so it's
/// unit-testable directly.
pub(super) fn prompt_json(world: &World) -> Value {
    match world.prompt {
        Some(p) => json!({
            "active": 1,
            "serial": p.sender_serial,
            "promptId": p.prompt_id,
            "kind": p.kind.as_str(),
        }),
        None => json!({ "active": 0 }),
    }
}

/// Resolve a vendor shop-list name that may be a literal display string OR a
/// bare cliloc id rendered as ASCII digits. ServUO's stock-item naming
/// (`IShopSellInfo.GetNameFor`, `Scripts/VendorInfo/GenericSell.cs`) falls
/// back to `item.LabelNumber.ToString()` — a plain decimal string, no `#` —
/// whenever the item has no explicit `Item.Name`, which is the common case
/// for ordinary stock; a leading `#` is stripped too before parsing, in case
/// some other server variant writes it that way. Cliloc ids for item names
/// run well above any count a real display name could plausibly be (`>=
/// 500_000` — the same heuristic the 0x74 buy-list side already used), so a
/// small numeric string (a name that just happens to be digits) is left
/// alone, and anything at/above it is looked up in the Cliloc table, falling
/// back to `name` **exactly as given** (any leading `#` included) if the
/// table doesn't know it (no table loaded, or an id it doesn't have) — the
/// `#`-stripped form is only used for the numeric parse, never invented into
/// the display text we can't actually confirm.
pub(super) fn resolve_shop_name(name: &str, cliloc: Option<&Cliloc>) -> String {
    name.strip_prefix('#')
        .unwrap_or(name)
        .parse::<u32>()
        .ok()
        .filter(|&id| id >= 500_000)
        .and_then(|id| cliloc.and_then(|c| c.get(id).map(str::to_string)))
        .unwrap_or_else(|| name.to_string())
}

/// Build the `paperdoll` object for the scene: the latest server-initiated
/// paperdoll open/refresh (0x88), or `null` when none has arrived this
/// session. `seq` lets the renderer treat a fresh request as a "please open"
/// event even for a `serial` it already has a (possibly since-closed) window
/// for — see [`crate::world::Paperdoll`]'s doc for why a repeat matters.
pub(super) fn paperdoll_json(world: &World) -> Value {
    match &world.paperdoll {
        None => Value::Null,
        Some(p) => json!({
            "seq": p.seq, "serial": p.serial, "title": p.title,
            "warmode": p.warmode, "canLift": p.can_lift,
        }),
    }
}

/// Validated 0xA5 HTTP(S) navigation requests. This bridge only transports the
/// request and its monotonic identity; the browser presents a consent dialog
/// and never navigates merely because an entry appears here.
pub(super) fn open_urls_json(world: &World) -> Value {
    Value::Array(
        world
            .recent_open_urls
            .iter()
            .map(|request| json!({ "seq": request.seq, "url": request.url }))
            .collect(),
    )
}

/// ServUO 0x24 `gumpId`s that are NOT a container (see
/// [`anima_core::net::game::draw_container`]'s doc for the ServUO/ClassicUO
/// cites): `DisplayBuyList`/`DisplayBuyListHS` (vendor "Buy" window) always
/// writes `0x30`; `DisplaySpellbook`/`DisplaySpellbookHS` always writes
/// `0xFFFF` (`-1` as the wire i16).
pub(super) const GUMP_ID_VENDOR_BUY: u16 = 0x0030;

pub(super) const GUMP_ID_SPELLBOOK: u16 = 0xFFFF;

/// Build the `dragCompletions` event ring consumed by the browser's held-item
/// cursor. `token` is null for payload-free 0x29 and the raw four-byte 0x28
/// value otherwise; keeping it raw lets the UI correlate serial-bearing legacy
/// packets without teaching the protocol layer an unverified interpretation.
pub(super) fn drag_completions_json(world: &World) -> Value {
    Value::Array(
        world
            .recent_drag_completions
            .iter()
            .map(|event| {
                json!({
                    "seq": event.seq,
                    "packet": event.packet,
                    "token": event.token,
                })
            })
            .collect(),
    )
}

/// Latest 0x2C death-screen trigger, separate from `player.dead` (which remains
/// body-derived). The browser uses `seq` to run ClassicUO's 1.5-second banner
/// once; action 2 therefore cannot be misread as a resurrection flag.
pub(super) fn death_screen_json(world: &World) -> Value {
    match world.death_screen {
        Some(event) => json!({ "seq": event.seq, "action": event.action }),
        None => Value::Null,
    }
}

/// Build the `containerOpens` array: [`World::recent_container_opens`] filtered
/// down to events that are actually a container window. `World` keeps that ring
/// as raw, unfiltered 0x24 data (every `gump_id` ServUO ever sent); deciding
/// which of those ids should make the web client pop a window is a renderer
/// policy call (D3: core = data, renderer = policy), so it happens here, not in
/// `World`. We skip `GUMP_ID_VENDOR_BUY`/`GUMP_ID_SPELLBOOK` — a vendor's Buy
/// list is already surfaced via `shop`/0x74/0x3B, and a spellbook via
/// `spellbooks`/0xBF/0x1B, so re-showing either as a bare generic Container
/// window here would be a spurious empty duplicate (live-reproduced: opening a
/// vendor's Buy list otherwise pushed the vendor's own MOBILE serial into this
/// ring as if it were a container).
pub(super) fn container_opens_json(world: &World) -> Value {
    let opens: Vec<Value> = world
        .recent_container_opens
        .iter()
        .filter(|&&(_, _, gump_id)| gump_id != GUMP_ID_VENDOR_BUY && gump_id != GUMP_ID_SPELLBOOK)
        .map(|&(seq, serial, _)| json!({ "seq": seq, "serial": serial }))
        .collect();
    Value::Array(opens)
}

/// Build the `maps` array: every open treasure/decoration map window
/// (0x90/0xF5 DisplayMap(New) + 0x56 MapCommand — [`anima_core::world::
/// MapView`]), sorted by serial for a stable order (the source is a
/// `HashMap`). `openSeq` bumps on every 0x90/0xF5 for that serial, even a
/// byte-identical resend (see [`anima_core::world::MapView::open_seq`]'s
/// doc) — the web client opens a window only when it sees a NEW `openSeq`
/// for a serial (mirrors `paperdoll`'s `seq`/`containerOpens`' ring), so a
/// user-closed map window doesn't pop back open on every poll. `pins` are
/// `[x, y]` pairs already in `w`×`h` PIXEL space (ServUO converts
/// bounds↔pixel server-side before a pin ever hits the wire — see
/// `MapView`'s doc) — the renderer draws each one straight onto the map art
/// with no rescale.
pub(super) fn maps_json(world: &World) -> Value {
    let mut maps: Vec<(&u32, &anima_core::world::MapView)> = world.map_gumps.iter().collect();
    maps.sort_by_key(|&(serial, _)| *serial);
    let maps: Vec<Value> = maps
        .iter()
        .map(|&(serial, mv)| {
            json!({
                "serial": serial, "openSeq": mv.open_seq, "gumpArt": mv.gump_art, "facet": mv.facet,
                "bounds": { "minX": mv.min_x, "minY": mv.min_y, "maxX": mv.max_x, "maxY": mv.max_y },
                "w": mv.width, "h": mv.height,
                "pins": mv.pins.iter().map(|&(x, y)| json!([x, y])).collect::<Vec<_>>(),
                "editable": mv.editable,
            })
        })
        .collect();
    Value::Array(maps)
}
