//! Game-phase packet codec → [`World`] mutation.
//!
//! [`apply_packet`] decodes a single framed game packet and updates the world
//! state, which is the single source of truth. The brain/renderer read `World`;
//! they never parse bytes. Ported from `anima/anima/perception/handlers.py`.
//!
//! Only perception-relevant packets are handled so far; unrecognized ids are
//! ignored (returns `false`). Movement confirm/deny (0x21/0x22) are owned by
//! [`crate::net::movement`].

use super::packet::{PacketError, PacketReader, Result as PResult};
use crate::world::{
    BoatMovedEntity, BoatMovement, BulletinBoard, BulletinMessage, BulletinSummary, ChatChannel,
    ChatStatus, DragAnimation, Effect, GameTime, Gump, HouseDesign, HousePlane, HuePicker,
    JournalEntry, LegacyMenu, LegacyMenuEntry, LegacyMenuKind, MultiPlacement, PopupEntry,
    PopupMenu, PromptKind, PromptState, Skill, TargetCursor, TipKind, TradeState, Waypoint, World,
};

// The handlers themselves, grouped by what they are about rather than by
// packet id — `dispatch` below is the id-ordered index into them. Each module
// opens with `use super::*`, which reaches this module's imports and its
// siblings' handlers alike, so a handler can be moved between modules without
// touching anything but the two files.
//
// `text` is the exception: shared decoding primitives that touch no `World`.
mod text;
use text::*;

mod chat;
mod combat;
mod effects;
mod general_info;
mod items;
mod mobiles;
mod session;
mod ui;
use chat::*;
use combat::*;
use effects::*;
use general_info::*;
use items::*;
use mobiles::*;
use session::*;
use ui::*;

/// Decode one framed game packet (id byte included) into `world`.
/// Returns `true` if the packet id was recognized.
pub fn apply_packet(world: &mut World, frame: &[u8]) -> bool {
    if frame.is_empty() {
        return false;
    }
    // A malformed/truncated packet must never crash the session — swallow parse
    // errors and treat the packet as handled-but-skipped.
    dispatch(world, frame[0], frame).unwrap_or(true)
}

fn dispatch(world: &mut World, id: u8, frame: &[u8]) -> PResult<bool> {
    match id {
        0x20 => mobile_update(world, frame)?,
        0x77 => mobile_moving(world, frame)?,
        0x78 => mobile_incoming(world, frame)?,
        0x2E => equip_update(world, frame)?,
        0x1A => world_item(world, frame)?,
        0xF3 => world_item_hs(world, frame)?,
        0xF7 => packet_list(world, frame)?,
        0x1D => delete(world, frame)?,
        0x11 => char_status(world, frame)?,
        0x98 => update_name(world, frame)?,
        0x16 => health_bar_status(world, frame)?,
        0x17 => health_bar_status(world, frame)?,
        0xDE => update_mobile_status(world, frame)?,
        0xC4 => semivisible(world, frame)?,
        0xA1 => vital(world, frame, Vital::Hits)?,
        0xA2 => vital(world, frame, Vital::Mana)?,
        0xA3 => vital(world, frame, Vital::Stam)?,
        0x1C => ascii_talk(world, frame)?,
        0xAE => unicode_talk(world, frame)?,
        0xBF => general_info(world, frame)?,
        0x6C => target_cursor(world, frame)?,
        0x3A => skills(world, frame)?,
        0x3C => container_content(world, frame)?,
        0x25 => add_to_container(world, frame)?,
        0xC1 => cliloc_message(world, frame)?,
        0xCC => cliloc_affix(world, frame)?,
        0x0B => damage(world, frame)?,
        0x70 => graphic_effect(world, frame, false)?,
        0xC0 => graphic_effect(world, frame, true)?,
        0xC7 => graphic_effect(world, frame, true)?,
        0x54 => play_sound(world, frame)?,
        0x6E => character_anim(world, frame)?,
        0xE2 => typed_anim(world, frame)?,
        0x6D => play_music(world, frame)?,
        0x71 => bulletin_board_data(world, frame)?,
        0x72 => war_mode(world, frame)?,
        0x4F => overall_light(world, frame)?,
        0x4E => personal_light(world, frame)?,
        0x65 => weather(world, frame)?,
        0xBC => season(world, frame)?,
        0xC8 => client_view_range(world, frame)?,
        0x5B => set_time(world, frame)?,
        0x74 => open_buy_window(world, frame)?,
        0x7C => open_legacy_menu(world, frame)?,
        0x95 => open_hue_picker(world, frame)?,
        0x9E => sell_list(world, frame)?,
        0xDF => buff(world, frame)?,
        0xB0 => display_gump(world, frame)?,
        0xB2 => chat_message(world, frame)?,
        0xDD => display_gump_packed(world, frame)?,
        0xBA => quest_arrow(world, frame)?,
        0xD6 => mega_cliloc(world, frame)?,
        0xDC => opl_info(world, frame)?,
        0x93 => open_book(world, frame)?,
        0xD4 => open_book_new(world, frame)?,
        0x66 => book_data(world, frame)?,
        0xAF => display_death(world, frame)?,
        0xAA => change_combatant(world, frame)?,
        0x15 => follow_r(world, frame)?,
        0x23 => drag_animation(world, frame)?,
        0x27 => lift_reject(world, frame)?,
        0x28 => end_dragging_item(world, frame)?,
        0x29 => drop_item_accepted(world)?,
        0x2C => death_status(world, frame)?,
        0x2D => mobile_attributes(world, frame)?,
        0x38 => pathfinding(world, frame)?,
        0x89 => corpse_equip(world, frame)?,
        0x9A => ascii_prompt(world, frame)?,
        0xA5 => open_url(world, frame)?,
        0xA6 => tip_window(world, frame)?,
        0xAB => text_entry_dialog(world, frame)?,
        0xB8 => character_profile(world, frame)?,
        0xC2 => unicode_prompt(world, frame)?,
        0xD1 => logout_ack(world, frame)?,
        0x6F => secure_trade(world, frame)?,
        0x3B => end_vendor(world, frame)?,
        0x24 => draw_container(world, frame)?,
        0x88 => open_paperdoll(world, frame)?,
        0x2F => swing(world, frame)?,
        0x90 => display_map(world, frame, false)?,
        0xF5 => display_map(world, frame, true)?,
        0xF6 => boat_moving(world, frame)?,
        0x56 => map_command(world, frame)?,
        0x99 => multi_target_cursor(world, frame)?,
        0xD8 => custom_house(world, frame)?,
        0xE5 => display_waypoint(world, frame)?,
        0xE6 => remove_waypoint(world, frame)?,
        0x97 => move_player(world, frame)?,
        0xD2 => update_character(world, frame)?,
        0xD3 => update_object(world, frame)?,
        _ => return Ok(false),
    }
    Ok(true)
}

#[cfg(test)]
mod tests;
