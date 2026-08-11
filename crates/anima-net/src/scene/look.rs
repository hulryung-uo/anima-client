//! Asset lookups shared by everything `build_scene` emits.
//!
//! These were fifteen closures at the top of `build_scene`, which is what tied
//! the whole function together: each captured `map`/`anim`/`animdata`, so
//! nothing that used one could be lifted out. As a struct they can be passed
//! around, and the emitters become ordinary functions.
//!
//! The borrow shape is load-bearing and unchanged. `Look` holds a **shared**
//! `&MapData`, because the tile loop later needs the `&mut` one; the original
//! comments said this closure-by-closure ("resolved through the shared borrow
//! before `map` is consumed by the tile loop below") and it is really one fact
//! about the whole struct. Build a `Look`, emit everything that needs asset
//! lookups, drop it, *then* walk the tiles.

use super::*;

pub(super) struct Look<'a> {
    pub(super) map: Option<&'a MapData>,
    pub(super) anim: Option<&'a Anim>,
    pub(super) animdata: Option<&'a AnimData>,
}

impl Look<'_> {
    /// `Body.def` remap (ClassicUO ReplaceBody): redirect an exotic body to its
    /// real animation body so the renderer picks the right group + resolves a
    /// sprite. The mobile's own hue wins; Body.def's hue is only a fallback for
    /// base creatures.
    pub(super) fn remap(&self, body: u16, hue: u16) -> (u16, u16) {
        let (rbody, rhue) = self.anim.map_or((body, 0), |a| a.remap(body));
        (rbody, if hue != 0 { hue } else { rhue })
    }

    /// Authoritative animation group kind (0 monster, 1 animal, 2 people) for
    /// the (already Body.def-remapped) body: `mobtypes.txt` via `Anim`, else the
    /// raw range heuristic. Sent as `at` so the renderer picks group numbers
    /// that match the file layout the reader uses (an animal's stand is group 2,
    /// a monster's is group 1).
    pub(super) fn atype(&self, body: u16) -> u8 {
        self.anim.map_or_else(
            || (body >= 200) as u8 + (body >= 400) as u8,
            |a| a.anim_type(body),
        )
    }

    /// `Corpse.def` remap (ClassicUO ReplaceCorpse): the SAME idea as
    /// [`Look::remap`], but a separate table applied to a corpse item's body
    /// (which travels in the item's `amount` field — ClassicUO
    /// `Item.GetGraphicForAnimation`'s `IsCorpse` special case). The corpse's
    /// own hue still wins over Corpse.def's fallback.
    pub(super) fn remap_corpse(&self, body: u16, hue: u16) -> (u16, u16) {
        let (rbody, rhue) = self.anim.map_or((body, 0), |a| a.remap_corpse(body));
        (rbody, if hue != 0 { hue } else { rhue })
    }

    /// Worn equipment's AnimID (the sprite to fetch via `/anim`), from tiledata.
    /// 0 when there's no map.
    pub(super) fn item_anim(&self, g: u16) -> u16 {
        self.map.map_or(0, |m| m.item_anim(g))
    }

    /// `Equipconv.def` override (ClassicUO `EquipConversions[body][item.AnimID]`,
    /// consulted by `MobileView`/`ItemView` for the world sprite and
    /// `PaperDollInteractable.GetAnimID` for the paperdoll): given the wearer's
    /// REMAPPED `body` and an item's tiledata `base_anim`, return the
    /// replacement `(anim graphic, paperdoll gump id, hue)`. `anim` is always
    /// overridden when a conversion exists (ClassicUO's unconditional `graphic =
    /// data.Graphic`); `gump` is `Some` only then (`None` ⇒ caller keeps its own
    /// `anim + gender offset` paperdoll convention); `hue` is the item's own hue,
    /// falling back to the conversion's hue only when the item has none
    /// (ClassicUO: `if (hue == 0 && _equipConvData.HasValue) hue =
    /// _equipConvData.Value.Color`).
    pub(super) fn equip_conv(
        &self,
        body: u16,
        base_anim: u16,
        item_hue: u16,
    ) -> (u16, Option<u16>, u16) {
        match self.anim.and_then(|a| a.equip_conv(body, base_anim)) {
            Some(ec) => (
                ec.graphic,
                Some(equip_conv_gump(body, ec.gump)),
                if item_hue != 0 { item_hue } else { ec.hue },
            ),
            None => (base_anim, None, item_hue),
        }
    }

    /// Does an item graphic emit light (torch/lamp/brazier)?
    pub(super) fn item_is_light(&self, g: u16) -> bool {
        self.map.is_some_and(|m| m.item_is_light(g))
    }

    /// The `light.mul` shape a light-emitting graphic casts (tiledata's
    /// `Quality`/layer byte). 0 when there's no map — the renderer falls back to
    /// its plain radial glow for that.
    pub(super) fn item_light_id(&self, g: u16) -> u8 {
        self.map.map_or(0, |m| m.item_light_id(g))
    }

    /// Does an item graphic carry the Foliage flag (tree/bush)? Used so the
    /// renderer can fade it when it would occlude the player.
    pub(super) fn item_foliage(&self, g: u16) -> bool {
        self.map
            .is_some_and(|m| m.item_flags(g) & FLAG_FOLIAGE != 0)
    }

    /// "nodraw" void-placeholder items (name starts "nodraw", e.g. graphic 0x1
    /// staff spawner/markers): ClassicUO culls these for items just like
    /// statics — without this the "NO DRAW" placeholder bitmap shows on the
    /// ground for GM characters.
    pub(super) fn item_nodraw(&self, g: u16) -> bool {
        self.map.is_some_and(|m| m.item_is_nodraw(g))
    }

    /// Container (chest/bag/corpse 0x2006) → the client opens a loot window on
    /// double-click; non-containers (doors, etc.) must NOT spawn an empty window.
    pub(super) fn item_is_cont(&self, g: u16) -> bool {
        g == 0x2006 || self.map.is_some_and(|m| m.item_is_container(g))
    }

    /// STACKABLE tiledata — the split-stack dialog should only ever offer to
    /// split an item the server would actually accept a partial amount from.
    pub(super) fn item_stackable(&self, g: u16) -> bool {
        self.map
            .is_some_and(|m| m.item_flags(g) & FLAG_STACKABLE != 0)
    }

    /// A dyed item's art hue, PartialHue-aware (see [`item_art_hue`]). Every
    /// surface that draws item art — ground sprite, container/trade grid, vendor
    /// row, drag ghost — asks the art endpoint for this exact value.
    pub(super) fn item_hue(&self, g: u16, hue: u16) -> u16 {
        item_art_hue(hue, self.map.map_or(0, |m| m.item_flags(g)))
    }

    /// Animated dynamic item (campfire, spell field, brazier): the same animdata
    /// frame sequence a static with that graphic gets.
    pub(super) fn item_frames(&self, g: u16) -> Option<(Vec<u16>, u32)> {
        self.map.and_then(|m| anim_frames(m, self.animdata, g))
    }

    /// Draw-sort priority for a dynamic item (same scheme as statics): base z,
    /// with a background tile under, and a tile with height (a wall/door) over,
    /// same-tile flats.
    pub(super) fn item_pz(&self, g: u16, z: i32) -> i32 {
        self.map.map_or(z, |m| {
            let mut pz = z;
            if m.item_flags(g) & 0x1 != 0 {
                pz -= 1; // Background
            }
            if m.item_height(g) != 0 {
                pz += 1; // has height (door/wall/solid)
            }
            pz
        })
    }

    /// `h`/`pf` for a dynamic item, `in_radius` being whether it is within
    /// [`PATH_RADIUS`] of the player: now that a dynamic item can contribute a
    /// standing surface (a boat deck — see `dynamic_statics_at`'s doc), the
    /// browser's own `calculateNewZ` port needs the SAME tiledata bits a nearby
    /// static already carries (see `path_suffix`'s doc), or it keeps resolving
    /// the water's Z under a deck instead of the deck's own. Shares
    /// [`path_bits`] with `path_suffix` so the two can never derive different
    /// bits for the same graphic.
    pub(super) fn item_path_bits(&self, g: u16, in_radius: bool) -> (Option<u8>, Option<u8>) {
        self.map.map_or((None, None), |m| {
            path_bits(in_radius, m.item_height(g), m.item_flags(g))
        })
    }
}
