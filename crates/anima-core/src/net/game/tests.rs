//! Golden/regression tests for the game-phase codec.
//!
//! Split out of `mod.rs` purely for size: this is a `mod tests` like any
//! other (`use super::*` reaches the handlers, including the private ones),
//! it just lives in its own file because it outgrew the module it tests.

use super::*;
use crate::net::packet::PacketWriter;
use crate::types::Position;

fn target_packet(target_type: u8, cursor_id: u32, flag: u8) -> Vec<u8> {
    let mut p = PacketWriter::new();
    p.u8(0x6C).u8(target_type).u32(cursor_id).u8(flag);
    p.zeros(12); // serial+x+y+z+graphic fields (server sends zero on request)
    p.into_vec()
}

#[test]
fn mega_cliloc_parses_property_lines() {
    // Two property lines for serial 0xDEADBEEF, revision 0x12345678.
    // Line 0: cliloc 1050045 with args "\t\tLongsword" (a name template).
    // Line 1: cliloc 1060403 with args "15" (e.g. "physical damage 15%").
    let mut p = PacketWriter::new();
    p.u8(0xD6).u16(0); // id + length placeholder
    p.u16(0x0001) // unknown
        .u32(0xDEAD_BEEF) // serial
        .u8(0)
        .u8(0) // two zero bytes
        .u32(0x1234_5678); // revision
    let put_line = |p: &mut PacketWriter, cliloc: u32, args: &str| {
        let units: Vec<u8> = args
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes()) // UTF-16 LE args
            .collect();
        p.u32(cliloc).u16(units.len() as u16).bytes(&units);
    };
    put_line(&mut p, 1_050_045, "\t\tLongsword");
    put_line(&mut p, 1_060_403, "15");
    p.u32(0); // terminator
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;

    let mut w = World::new();
    assert!(apply_packet(&mut w, &frame));
    assert_eq!(w.opl_revision.get(&0xDEAD_BEEF), Some(&0x1234_5678));
    let lines = w.opl.get(&0xDEAD_BEEF).expect("opl stored");
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], (1_050_045, "\t\tLongsword".to_string()));
    assert_eq!(lines[1], (1_060_403, "15".to_string()));

    // 0xDC OPLInfo carrying the same revision, with ServUO's 0x40000000 bit
    // set the way `ObjectPropertyList.Hash` sets it: nothing to do.
    let opl_info = |revision: u32| {
        let mut q = PacketWriter::new();
        q.u8(0xDC).u32(0xDEAD_BEEF).u32(revision);
        q.into_vec()
    };
    assert!(apply_packet(&mut w, &opl_info(0x4000_0000 | 0x1234_5678)));
    assert!(w.pending_opl_requests.is_empty());
    // A different revision means our lines are stale: queue a refetch, once,
    // without disturbing the revision the lines we hold actually came with.
    assert!(apply_packet(&mut w, &opl_info(0x4000_0000 | 0x9999_0000)));
    assert!(apply_packet(&mut w, &opl_info(0x4000_0000 | 0x9999_0000)));
    assert_eq!(w.pending_opl_requests, vec![0xDEAD_BEEF]);
    assert_eq!(w.opl_revision.get(&0xDEAD_BEEF), Some(&0x1234_5678));
    // The answering 0xD6 satisfies the queued request.
    assert!(apply_packet(&mut w, &frame));
    assert!(w.pending_opl_requests.is_empty());
}

#[test]
fn opl_info_for_an_unfetched_serial_is_ignored() {
    // Hover-driven only: a notice for a serial we hold no OPL for must not
    // turn into a request (ServUO sends 0xDC for everything entering view).
    let mut w = World::new();
    let mut q = PacketWriter::new();
    q.u8(0xDC).u32(0x4001_0203).u32(0x4000_0001);
    assert!(apply_packet(&mut w, &q.into_vec()));
    assert!(w.pending_opl_requests.is_empty());
    assert!(w.opl_revision.is_empty());
}

#[test]
fn popup_menu_modern_and_legacy() {
    // Modern (version 2): [cliloc u32][index u16][flags u16] per entry.
    let mut p = PacketWriter::new();
    p.u8(0xBF).u16(0); // id + len placeholder
    p.u16(0x0014) // subcommand
        .u16(0x0002) // version 2
        .u32(0xDEAD_BEEF) // serial
        .u8(2); // count
    p.u32(3_000_122).u16(0).u16(0x0000); // entry 0
    p.u32(3_006_111).u16(1).u16(0x0001); // entry 1 (flag 0x01)
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;

    let mut w = World::new();
    assert!(apply_packet(&mut w, &frame));
    let menu = w.popup.as_ref().expect("popup set");
    assert_eq!(menu.serial, 0xDEAD_BEEF);
    assert_eq!(menu.entries.len(), 2);
    assert_eq!(
        menu.entries[0],
        PopupEntry {
            index: 0,
            cliloc: 3_000_122,
            flags: 0
        }
    );
    assert_eq!(
        menu.entries[1],
        PopupEntry {
            index: 1,
            cliloc: 3_006_111,
            flags: 1
        }
    );

    // Legacy (version 1): [index u16][cliloc-3000000 u16][flags u16].
    let mut p = PacketWriter::new();
    p.u8(0xBF).u16(0);
    p.u16(0x0014).u16(0x0001).u32(0x0102_0304).u8(1);
    p.u16(7).u16(122).u16(0x0000); // index 7, cliloc 3000122
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;
    assert!(apply_packet(&mut w, &frame));
    let menu = w.popup.as_ref().expect("popup set");
    assert_eq!(menu.serial, 0x0102_0304);
    assert_eq!(
        menu.entries,
        vec![PopupEntry {
            index: 7,
            cliloc: 3_000_122,
            flags: 0
        }]
    );
}

#[test]
fn spellbook_content_parses_and_prunes_on_delete() {
    let mut w = World::new();
    // 0xBF/0x1B NewSpellbookContent: magery book (graphic 0x0EFA, ServUO
    // BookOffset 0 -> offset 1) knows spells 1 (Clumsy) and 4 (Heal) — bits
    // 0 and 3 of the mask, content = 0b1001 = 0x9. `content` is written
    // byte-by-byte LSB-first (see `parse_spellbook_content`'s doc), unlike
    // the rest of the wire (big-endian).
    let mut p = PacketWriter::new();
    p.u8(0xBF).u16(0); // id + len placeholder
    p.u16(0x001B) // subcommand
        .u16(0x0001) // unknown, always 1
        .u32(0x4000_0010) // book serial
        .u16(0x0EFA) // graphic (magery book ItemID)
        .u16(1) // offset = BookOffset(0) + 1
        .bytes(&[0x09, 0, 0, 0, 0, 0, 0, 0]); // content mask, LSB-first
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;
    assert_eq!(frame.len(), 23); // ServUO NewSpellbookContent EnsureCapacity(23)
    assert!(apply_packet(&mut w, &frame));

    let sb = w
        .spellbooks
        .get(&0x4000_0010)
        .expect("spellbook content stored");
    assert_eq!(sb.graphic, 0x0EFA);
    assert_eq!(sb.offset, 1);
    assert_eq!(sb.content, 0x9);

    // The book is destroyed/despawned (0x1D Delete) — the entry is pruned with it.
    let mut d = PacketWriter::new();
    d.u8(0x1D).u32(0x4000_0010);
    assert!(apply_packet(&mut w, &d.into_vec()));
    assert!(!w.spellbooks.contains_key(&0x4000_0010));
}

#[test]
fn custom_house_parses_and_decodes_planes() {
    // Plane A: mode 0 (a "stair section" header, per real ServUO plane
    // numbering — see the module's `custom_house` doc), one 5-byte entry:
    // graphic 0x0063, dx=-3, dy=3, dz=0.
    let raw_a: Vec<u8> = vec![0x00, 0x63, 0xFD /* -3 */, 0x03, 0x00];
    let comp_a = miniz_oxide::deflate::compress_to_vec_zlib(&raw_a, 6);
    let dlen_a = raw_a.len() as u32;
    let clen_a = comp_a.len() as u32;
    let header_a: u32 = (9 << 24)
        | ((dlen_a & 0xFF) << 16)
        | ((clen_a & 0xFF) << 8)
        | (((dlen_a >> 8) & 0xF) << 4)
        | ((clen_a >> 8) & 0xF);

    // Plane B: mode 2 (grid), plane_z 0 — six graphic-only entries, one a
    // graphic-0 hole. With bounds min_x=min_y=-1, max_y=1 (a 3x3 footprint),
    // `mh = max_y - min_y + 2 = 4` (the ground plane's extra south stair
    // row) and y is the fast axis: i=0->(-1,-1), 1->(-1,0), 2->(-1,1)
    // (the hole), 3->(-1,2), 4->(0,-1), 5->(0,0).
    let raw_b: Vec<u8> = vec![
        0x00, 0x10, // i=0 -> (-1,-1)
        0x00, 0x11, // i=1 -> (-1,0)
        0x00, 0x00, // i=2 -> (-1,1) hole: graphic 0
        0x00, 0x12, // i=3 -> (-1,2)
        0x00, 0x13, // i=4 -> (0,-1)
        0x00, 0x14, // i=5 -> (0,0)
    ];
    let comp_b = miniz_oxide::deflate::compress_to_vec_zlib(&raw_b, 6);
    let dlen_b = raw_b.len() as u32;
    let clen_b = comp_b.len() as u32;
    let header_b: u32 = (2 << 28)
        | ((dlen_b & 0xFF) << 16)
        | ((clen_b & 0xFF) << 8)
        | (((dlen_b >> 8) & 0xF) << 4)
        | ((clen_b >> 8) & 0xF);

    // Plane C: clen == 0 — a skipped plane must consume no payload bytes.
    let header_c: u32 = 1 << 28;

    // Plane D: mode 1, plane_z 1, 76 identical 4-byte entries = 304 bytes —
    // dlen 0x130 exercises the 12-bit length's HIGH NIBBLE through the real
    // parser (real ground planes are width*height*2 > 255 bytes, so a swapped
    // nibble split corrupts most actual houses while every <256-byte test
    // plane still passes). dz = ((1-1)%4)*20+7 = 7.
    let raw_d: Vec<u8> = [0x0B, 0xBB, 0x05, 0xFA /* -6 */].repeat(76);
    let comp_d = miniz_oxide::deflate::compress_to_vec_zlib(&raw_d, 6);
    let dlen_d = raw_d.len() as u32; // 304 = 0x130: high nibble is non-zero
    let clen_d = comp_d.len() as u32;
    let header_d: u32 = (1 << 28)
        | (1 << 24)
        | ((dlen_d & 0xFF) << 16)
        | ((clen_d & 0xFF) << 8)
        | (((dlen_d >> 8) & 0xF) << 4)
        | ((clen_d >> 8) & 0xF);

    let serial = 0x4001_0001u32;
    let mut p = PacketWriter::new();
    p.u8(0xD8).u16(0); // id + length placeholder
    p.u8(0x03); // compression flag (ServUO always writes 3)
    p.u8(0x01); // enable-response flag
    p.u32(serial).u32(1); // revision 1
    p.u16(0).u16(0); // advisory tile count / buffer length — untrusted, ignored
    p.u8(4); // plane_count
    p.u32(header_a).bytes(&comp_a);
    p.u32(header_b).bytes(&comp_b);
    p.u32(header_c); // clen == 0: no payload bytes follow
    p.u32(header_d).bytes(&comp_d);
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;

    let mut w = World::new();
    assert!(apply_packet(&mut w, &frame));

    let design = w.house_designs.get(&serial).expect("design stored");
    assert_eq!(design.revision, 1);
    assert_eq!(
        design.planes.len(),
        3,
        "the clen==0 plane must be skipped, not stored"
    );
    assert_eq!(design.planes[0].mode, 0);
    assert_eq!(
        design.planes[0].data, raw_a,
        "inflate must reproduce the exact original bytes"
    );
    assert_eq!(design.planes[1].mode, 2);
    assert_eq!(design.planes[1].data, raw_b);
    assert_eq!(
        design.planes[2].data, raw_d,
        ">255-byte plane pins the split 12-bit dlen"
    );

    // Exercise the pure decoder directly, adding a mode-1 floor plane plus
    // mode-2 planes for BOTH remaining grid branches, so every offset
    // asymmetry is pinned in one call. plane_z 2 -> dz = ((2-1)%4)*20+7 = 27.
    let mut planes = design.planes.clone();
    planes.push(HousePlane {
        mode: 1,
        plane_z: 2,
        data: vec![0x00, 0xAA, 0x02, 0xFE /* -2 */],
    });
    // Plane E: mode 2, plane_z 1 — floors 1-4 use INSET offsets (min+1) and
    // mh = max_y - min_y = 2: i=1 -> (0,1), i=2 -> (1,0); i=0 is a hole.
    // dz = z_of(1) = 7. The ground-plane branch would land these on B's keys.
    planes.push(HousePlane {
        mode: 2,
        plane_z: 1,
        data: vec![0, 0, 0, 0x21, 0, 0x22],
    });
    // Plane F: mode 2, plane_z 5 — roof grids use UN-inset offsets with
    // mh = max_y - min_y + 1 = 3, and z_of's %4 wraps plane 5 back to dz 7
    // (not 87). i=6 -> (1,-1); i=0..5 are holes.
    planes.push(HousePlane {
        mode: 2,
        plane_z: 5,
        data: vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x00, 0x31],
    });
    let tiles = crate::world::decode_house_planes(&planes, -1, -1, 1); // 3x3 footprint bounds

    assert_eq!(tiles.get(&(-3, 3)), Some(&vec![(0x0063, 0)])); // mode 0
    assert_eq!(tiles.get(&(2, -2)), Some(&vec![(0x00AA, 27)])); // mode 1, plane_z 2 -> dz 27
    assert_eq!(tiles.get(&(-1, -1)), Some(&vec![(0x0010, 0)])); // mode 2, i=0
                                                                // i=3 -> (-1,2) pins Y as the FAST axis (a transposed decode puts it at
                                                                // (2,-1)) AND the ground plane's mh = max_y-min_y+2 south stair row.
    assert_eq!(tiles.get(&(-1, 2)), Some(&vec![(0x0012, 0)]));
    assert_eq!(tiles.get(&(0, 0)), Some(&vec![(0x0014, 0)])); // mode 2, i=5
    assert_eq!(tiles.get(&(-1, 1)), None, "graphic 0 is a hole, not a tile");
    // Plane D: 76 stacked entries at (5,-6), all dz 7 (plane_z 1).
    let d_tiles = tiles.get(&(5, -6)).expect("mode-1 >255-byte plane decoded");
    assert_eq!(d_tiles.len(), 76);
    assert_eq!(d_tiles[0], (0x0BBB, 7));
    // Plane E: inset floor-grid branch (off = min+1, mh = 2).
    assert_eq!(tiles.get(&(0, 1)), Some(&vec![(0x0021, 7)]));
    assert_eq!(tiles.get(&(1, 0)), Some(&vec![(0x0022, 7)]));
    // Plane F: roof-grid branch (off = min, mh = 3) + the %4 height wrap.
    assert_eq!(tiles.get(&(1, -1)), Some(&vec![(0x0031, 7)]));
}

#[test]
fn house_revision_notice_queues_request_once() {
    let mut w = World::new();
    let serial = 0x4001_0002u32;
    w.item_mut(serial).is_multi = true; // the foundation must be a known item

    let notice = |revision: u32| {
        let mut p = PacketWriter::new();
        p.u8(0xBF).u16(0).u16(0x001D).u32(serial).u32(revision);
        let mut frame = p.into_vec();
        let len = frame.len() as u16;
        frame[1] = (len >> 8) as u8;
        frame[2] = (len & 0xFF) as u8;
        frame
    };

    assert!(apply_packet(&mut w, &notice(5)));
    assert_eq!(w.pending_house_design_requests, vec![serial]);

    // A repeat notice (ServUO resends on every SendInfoTo) must not
    // duplicate the queued request.
    assert!(apply_packet(&mut w, &notice(5)));
    assert_eq!(w.pending_house_design_requests, vec![serial]);

    // Once the design itself arrives at that revision, the request drains.
    let mut d = PacketWriter::new();
    d.u8(0xD8).u16(0);
    d.u8(0x03).u8(0x01).u32(serial).u32(5);
    d.u16(0).u16(0);
    d.u8(0); // zero planes
    let mut dframe = d.into_vec();
    let dlen = dframe.len() as u16;
    dframe[1] = (dlen >> 8) as u8;
    dframe[2] = (dlen & 0xFF) as u8;
    assert!(apply_packet(&mut w, &dframe));
    assert!(w.pending_house_design_requests.is_empty());
    assert_eq!(w.house_designs.get(&serial).map(|d| d.revision), Some(5));

    // A further notice at the SAME (now-current) revision must not
    // re-trigger a request — otherwise every keepalive loops forever.
    assert!(apply_packet(&mut w, &notice(5)));
    assert!(w.pending_house_design_requests.is_empty());

    assert_eq!(
        crate::net::outgoing::build_house_design_request(0x4001_0001),
        vec![0xBF, 0, 9, 0, 0x1E, 0x40, 0x01, 0x00, 0x01]
    );
}

/// Builds a 0xBF/0x20 BeginHouseCustomization/EndHouseCustomization frame
/// with the full wire tail (`[0x0000][0xFFFF][0xFFFF][0xFF]`) even though
/// the handler only reads `serial`/`state` — so the test frame is the real
/// packet shape, not just the prefix the parser happens to consume.
fn house_customization_frame(serial: u32, state: u8) -> Vec<u8> {
    let mut p = PacketWriter::new();
    p.u8(0xBF).u16(0).u16(0x0020);
    p.u32(serial).u8(state);
    p.u16(0x0000).u16(0xFFFF).u16(0xFFFF).u8(0xFF);
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;
    frame
}

#[test]
fn general_info_house_customization_enters_and_leaves() {
    let mut w = World::new();
    let serial = 0x4002_0001u32;

    assert!(w.customizing_house.is_none());
    assert!(apply_packet(
        &mut w,
        &house_customization_frame(serial, 0x04)
    ));
    assert_eq!(w.customizing_house, Some(serial));

    assert!(apply_packet(
        &mut w,
        &house_customization_frame(serial, 0x05)
    ));
    assert!(w.customizing_house.is_none());
}

#[test]
fn general_info_house_customization_ignores_unknown_state() {
    let mut w = World::new();
    w.customizing_house = Some(0x4002_0002); // already customizing some other foundation

    assert!(apply_packet(
        &mut w,
        &house_customization_frame(0x4002_0003, 0x99) // reserved/unknown state byte
    ));
    assert_eq!(
        w.customizing_house,
        Some(0x4002_0002),
        "an unrecognized state byte must be a no-op, not clear or overwrite existing state"
    );
}

fn party_frame(body: &[u8]) -> Vec<u8> {
    let mut p = PacketWriter::new();
    p.u8(0xBF).u16(0).u16(0x0006);
    for &b in body {
        p.u8(b);
    }
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;
    frame
}

#[test]
fn party_list_remove_invite_and_chat() {
    let mut w = World::new();
    w.party.pending_invite = Some(0xAAAA);

    // 0x01 member list: count 2, leader then member. Clears pending invite.
    let list = party_frame(&[0x01, 2, 0, 0, 0x11, 0x11, 0, 0, 0x22, 0x22]);
    assert!(apply_packet(&mut w, &list));
    assert_eq!(w.party.members, vec![0x0000_1111, 0x0000_2222]);
    assert_eq!(w.party.leader, 0x0000_1111);
    assert_eq!(w.party.pending_invite, None);

    // 0x02 remove: count 1, removed serial, then 1 remaining member.
    let remove = party_frame(&[0x02, 1, 0, 0, 0x22, 0x22, 0, 0, 0x11, 0x11]);
    assert!(apply_packet(&mut w, &remove));
    assert_eq!(w.party.members, vec![0x0000_1111]);
    assert_eq!(w.party.leader, 0x0000_1111);

    // 0x02 disband: count 0, removed serial, no members.
    let disband = party_frame(&[0x02, 0, 0, 0, 0x11, 0x11]);
    assert!(apply_packet(&mut w, &disband));
    assert!(w.party.members.is_empty());
    assert_eq!(w.party.leader, 0);

    // 0x07 invitation: leader serial → pending invite.
    let invite = party_frame(&[0x07, 0, 0, 0x33, 0x33]);
    assert!(apply_packet(&mut w, &invite));
    assert_eq!(w.party.pending_invite, Some(0x0000_3333));

    // 0x04 chat-to-all: from serial + UTF-16 BE text → journal.
    let mut body = vec![0x04, 0, 0, 0x11, 0x11];
    for u in "hi".encode_utf16() {
        body.extend_from_slice(&u.to_be_bytes());
    }
    let chat = party_frame(&body);
    assert!(apply_packet(&mut w, &chat));
    assert_eq!(w.journal.last().expect("party line").text, "hi");
}

#[test]
fn container_content_refresh_and_stale_drop() {
    let mut w = World::new();
    // Pre-existing item in container 0xBAG that the refresh will NOT include.
    let old = w.item_mut(0x111);
    old.container = Some(0x4000_0BA6);

    // 0x3C: one item (a pickaxe, graphic 0x0E86) in container 0xBAG.
    let mut p = PacketWriter::new();
    p.u8(0x3C).u16(0).u16(1); // id, len, count
    p.u32(0x222) // serial
        .u16(0x0E86)
        .u8(0) // graphic + inc
        .u16(1) // amount
        .u16(3)
        .u16(4) // slot x,y
        .u8(0) // grid
        .u32(0x4000_0BA6) // container
        .u16(0); // hue
    apply_packet(&mut w, &p.into_vec());

    let pick = w.items.get(&0x222).expect("pickaxe added to bag");
    assert_eq!(pick.graphic, 0x0E86);
    assert_eq!(pick.container, Some(0x4000_0BA6));
    // The stale item (not in the refresh) is dropped.
    assert!(!w.items.contains_key(&0x111));
}

#[test]
fn world_item_hs_parsed_as_ground_item() {
    let mut w = World::new();
    // 0xF3: a forge (graphic 0x0FB1) on the ground at (2566, 493, 19).
    let mut p = PacketWriter::new();
    p.u8(0xF3).u16(0x0001).u8(0x00); // id, unk, data_type=item
    p.u32(0x4000_1000).u16(0x0FB1).u8(0); // serial, graphic, inc
    p.u16(1).u16(1); // amount, amount2
    p.u16(2566).u16(493).u8(19i8 as u8); // x, y, z
    p.u8(0).u16(0).u8(0); // light, hue, flags
    apply_packet(&mut w, &p.into_vec());
    let it = w.items.get(&0x4000_1000).expect("ground item added");
    assert_eq!(it.graphic, 0x0FB1);
    assert_eq!((it.pos.x, it.pos.y), (2566, 493));
    assert_eq!(it.container, None);
}

#[test]
fn world_item_hs_multi_populates_is_multi_and_strips_bank_bit() {
    let mut w = World::new();
    // 0xF3 type==2: a SmallBoat placed at (1492, 1760, 0) — multi id 2
    // (ServUO `SmallBoat.SouthID`). Real wire shape (verified against
    // ServUO `Server/Network/Packets.cs` `WorldItemHS`): the server masks
    // `itemID &= 0x3FFF` BEFORE writing a `BaseMulti`'s graphic, so this
    // NEVER carries the 0x4000 bank bit on the wire — `type == 2` alone is
    // what tells the client it's a multi. `inc` is always written as a
    // literal 0 for both branches (ServUO never increments here).
    let mut p = PacketWriter::new();
    p.u8(0xF3).u16(0x0001).u8(0x02); // id, unk, data_type=multi
    p.u32(0x4001_2345).u16(0x0002).u8(0); // serial, graphic (plain multi id 2, no bank bit), inc
    p.u16(1).u16(1); // amount, amount2
    p.u16(1492).u16(1760).u8(0); // x, y, z
    p.u8(2).u16(0).u8(0); // direction (south), hue, flags
    apply_packet(&mut w, &p.into_vec());
    let it = w
        .items
        .get(&0x4001_2345)
        .expect("multi added to World.items");
    assert!(it.is_multi, "type==2 must set is_multi");
    assert_eq!(
        it.graphic, 0x0002,
        "graphic is the plain multi id (never had a bank bit to strip)"
    );
    assert_eq!((it.pos.x, it.pos.y, it.pos.z), (1492, 1760, 0));

    // Despawns (0x1D) exactly like any other item/mobile.
    let mut d = PacketWriter::new();
    d.u8(0x1D).u32(0x4001_2345);
    assert!(apply_packet(&mut w, &d.into_vec()));
    assert!(
        !w.items.contains_key(&0x4001_2345),
        "0x1D must remove the multi like a normal item"
    );
}

#[test]
fn world_item_legacy_multi_detected_by_graphic_bank_bit() {
    let mut w = World::new();
    // 0x1A: a multi's wire graphic is `>= 0x4000` (ClassicUO `UpdateItem`'s
    // `type = graphic >= 0x4000 ? 2 : 0`), here 0x4064 = bank bit | house
    // multi id 0x64 (StonePlasterHouse).
    let mut p = PacketWriter::new();
    p.u8(0x1A).u16(0); // id, len (unused — frame is read from offset 3)
    p.u32(0x4000_9999); // serial (no has_amount flag)
    p.u16(0x4064); // graphic: bank bit | multi id 0x64
    p.u16(1000).u16(1000); // x, y (no direction/hue flags)
    p.u8(0); // z
    apply_packet(&mut w, &p.into_vec());
    let it = w.items.get(&0x4000_9999).expect("legacy multi added");
    assert!(
        it.is_multi,
        "graphic >= 0x4000 must set is_multi on 0x1A too"
    );
    assert_eq!(it.graphic, 0x0064);
}

#[test]
fn world_item_legacy_multi_classified_before_graphic_inc_added() {
    let mut w = World::new();
    // 0x1A: ClassicUO's `UpdateItem` classifies `type = graphic >= 0x4000 ?
    // 2 : 0` from the graphic AS READ off the wire (after stripping the
    // 0x8000 extension bit, if set) — `graphicInc` is stored separately
    // and only added to `graphic` later, inside `UpdateGameObject`, well
    // after this classification already ran. Pick a wire graphic (0x3FFE,
    // extended) + inc (4) whose SUM crosses 0x4000 (0x4002) but whose
    // PRE-inc value (0x3FFE) does not: classifying post-inc (the bug)
    // would misread this as multi id 2; classifying pre-inc (correct)
    // leaves it an ordinary item — which ClassicUO then stores with its
    // full, unmasked incremented graphic (a non-multi item's graphic is
    // never masked).
    let mut p = PacketWriter::new();
    p.u8(0x1A).u16(0); // id, len (unused — frame is read from offset 3)
    p.u32(0x4000_AAAA); // serial (no has_amount flag)
    p.u16(0x8000 | 0x3FFE); // graphic: 0x8000 ext bit | pre-inc value 0x3FFE
    p.u8(4); // graphic_inc
    p.u16(1000).u16(1000); // x, y (no direction/hue/flags bits set)
    p.u8(0); // z
    apply_packet(&mut w, &p.into_vec());
    let it = w.items.get(&0x4000_AAAA).expect("item added");
    assert!(
        !it.is_multi,
        "pre-inc graphic 0x3FFE is below 0x4000 — must NOT classify as a multi"
    );
    assert_eq!(
        it.graphic, 0x4002,
        "non-multi keeps the full incremented graphic unmasked, like ClassicUO"
    );
}

#[test]
fn packet_list_dispatches_batched_0xf3_items() {
    // 0xF7 PacketList: [id][len:u16][count:u16] then count × 0xF3 sub-packets
    // (each fixed 26 bytes, no length prefix). Both items must land.
    fn f3_body(serial: u32, graphic: u16, x: u16, y: u16) -> Vec<u8> {
        let mut p = PacketWriter::new();
        p.u16(0x0001).u8(0x00); // unk, data_type=item
        p.u32(serial).u16(graphic).u8(0); // serial, graphic, inc
        p.u16(1).u16(1); // amount, amount2
        p.u16(x).u16(y).u8(0); // x, y, z
        p.u8(0).u16(0); // direction, hue
        let mut v = p.into_vec();
        v.resize(25, 0); // pad to the fixed 25-byte 0xF3 body
        v
    }
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0xF7).u16(0); // id + length placeholder
    p.u16(2); // count
    p.u8(0xF3).bytes(&f3_body(0x4000_2001, 0x0FB1, 100, 200));
    p.u8(0xF3).bytes(&f3_body(0x4000_2002, 0x0FB2, 101, 201));
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;
    assert!(apply_packet(&mut w, &frame));
    assert_eq!(w.items.get(&0x4000_2001).map(|i| i.graphic), Some(0x0FB1));
    assert_eq!(w.items.get(&0x4000_2002).map(|i| i.graphic), Some(0x0FB2));
}

#[test]
fn packet_list_stops_at_non_0xf3_subpacket() {
    // ClassicUO breaks on the first non-0xF3 sub-id; the batch after it is
    // dropped rather than mis-parsed.
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0xF7).u16(0);
    p.u16(2);
    p.u8(0x11).u32(0xDEAD); // a non-0xF3 sub-id -> stop immediately
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;
    assert!(apply_packet(&mut w, &frame));
    assert!(w.items.is_empty());
}

#[test]
fn world_item_hs_corpse_carries_body_and_direction() {
    let mut w = World::new();
    // 0xF3: a corpse (graphic 0x2006) — the dead creature's body (400 = human
    // male) rides in `amount`, its facing (south = 5) in the direction byte.
    let mut p = PacketWriter::new();
    p.u8(0xF3).u16(0x0001).u8(0x00); // id, unk, data_type=item
    p.u32(0x4000_2000).u16(0x2006).u8(0); // serial, graphic, inc
    p.u16(400).u16(400); // amount (body id), amount2 (repeated)
    p.u16(1500).u16(1600).u8(10i8 as u8); // x, y, z
    p.u8(5).u16(0x0044).u8(0); // direction, hue, flags
    apply_packet(&mut w, &p.into_vec());
    let it = w.items.get(&0x4000_2000).expect("corpse item added");
    assert_eq!(it.graphic, 0x2006);
    assert_eq!(it.amount, 400); // dead creature's body id
    assert_eq!(it.direction, 5);
    assert_eq!(it.hue, 0x0044);
}

#[test]
fn world_item_legacy_corpse_direction_only_when_flagged() {
    let mut w = World::new();
    // 0x1A: a corpse (graphic 0x2006), body 0x00EE in `amount`, direction byte
    // present (x's 0x8000 flag) and hue present (y's 0x8000 flag).
    let mut p = PacketWriter::new();
    p.u8(0x1A).u16(0); // id, len (unused — frame is read from offset 3)
    p.u32(0x8000_0000 | 0x4000_1234); // serial | has_amount flag
    p.u16(0x2006); // graphic (corpse, no inc-flag bit)
    p.u16(0x00EE); // amount = body id
    p.u16(0x8000 | 1234); // x | direction-present flag
    p.u16(0x8000 | 5678); // y | hue-present flag
    p.u8(5); // direction (present because the x flag was set)
    p.u8((-2i8) as u8); // z
    p.u16(0x0033); // hue (present because the y flag was set)
    apply_packet(&mut w, &p.into_vec());
    let it = w.items.get(&0x4000_1234).expect("corpse item added");
    assert_eq!(it.graphic, 0x2006);
    assert_eq!(it.amount, 0x00EE);
    assert_eq!(it.direction, 5);
    assert_eq!(it.hue, 0x0033);
    assert_eq!((it.pos.x, it.pos.y, it.pos.z), (1234, 5678, -2));

    // A plain item (no direction/hue flags) leaves direction at its default 0.
    let mut w2 = World::new();
    let mut p2 = PacketWriter::new();
    p2.u8(0x1A).u16(0);
    p2.u32(0x4000_5555); // no has_amount flag
    p2.u16(0x0EED); // gold graphic, no inc
    p2.u16(100).u16(200).u8(0i8 as u8); // x, y (no flags), z
    apply_packet(&mut w2, &p2.into_vec());
    let it2 = w2.items.get(&0x4000_5555).expect("plain item added");
    assert_eq!(it2.direction, 0);
}

#[test]
fn cliloc_message_keeps_id_and_args() {
    let mut w = World::new();
    // 0xC1: cliloc 1044625 ("You dig some ore...") with one LE-UTF16 arg "iron".
    let mut p = PacketWriter::new();
    p.u8(0xC1).u16(0); // id, len(placeholder)
    p.u32(0).u16(0).u8(0).u16(0).u16(3); // serial, graphic, type, hue, font
    p.u32(1044625); // cliloc
    p.zeros(30); // name (System)
    for ch in "iron".chars() {
        p.u16((ch as u16).swap_bytes()); // write LE by swapping (writer is BE)
    }
    apply_packet(&mut w, &p.into_vec());
    let e = w.journal.last().expect("cliloc journal line");
    assert_eq!(e.cliloc, 1044625);
    assert_eq!(e.text, "iron");
    assert_eq!(e.name, "System");
}

#[test]
fn single_click_label_sets_mobile_name_not_journal() {
    let mut w = World::new();
    w.mobiles.insert(
        0x1234,
        crate::world::Mobile {
            serial: 0x1234,
            ..Default::default()
        },
    );
    // 0xC1 MessageLocalized as ServUO sends a single-click name: type 6 (Label),
    // cliloc 1050045 (the OPL name header `~1_val~`), the name as the sole arg.
    let mut p = PacketWriter::new();
    p.u8(0xC1).u16(0);
    p.u32(0x1234).u16(0).u8(6).u16(946).u16(3); // serial, graphic, type=6, hue, font
    p.u32(1050045); // cliloc = name header
    p.zeros(30); // name column (unused here)
    for ch in "Zurghed".chars() {
        p.u16((ch as u16).swap_bytes()); // LE-UTF16 arg
    }
    apply_packet(&mut w, &p.into_vec());
    // Stored on the mobile (drives the overhead label / hover), NOT scrolled in chat.
    assert_eq!(w.mobiles.get(&0x1234).unwrap().name, "Zurghed");
    assert!(
        w.journal.is_empty(),
        "a single-click name must not scroll in the journal"
    );
}

#[test]
fn regular_speech_still_journals_and_leaves_name_untouched() {
    let mut w = World::new();
    w.mobiles.insert(
        0x55,
        crate::world::Mobile {
            serial: 0x55,
            name: "Guard".into(),
            ..Default::default()
        },
    );
    // A normal (type 0) ascii talk from the same serial is chat, not a name.
    let mut p = PacketWriter::new();
    p.u8(0x1C).u16(0);
    p.u32(0x55).u16(0).u8(0).u16(0).u16(3); // serial, graphic, type=0, hue, font
    p.zeros(30); // name column
    p.bytes(b"halt!\0");
    apply_packet(&mut w, &p.into_vec());
    assert_eq!(w.mobiles.get(&0x55).unwrap().name, "Guard"); // name unchanged
    assert!(
        w.journal.iter().any(|e| e.text == "halt!"),
        "speech still journals"
    );
}

#[test]
fn skills_full_list_and_single_update() {
    let mut w = World::new();
    // Type 0x02: full list, 1-based ids, with caps, terminated by id 0.
    // Entry: Mining (45 → wire 46), value 500, base 480, lock 0, cap 1000.
    let mut p = PacketWriter::new();
    p.u8(0x3A).u16(0).u8(0x02); // id, len(placeholder), type
    p.u16(46).u16(500).u16(480).u8(0).u16(1000); // Mining
    p.u16(0); // terminator
    apply_packet(&mut w, &p.into_vec());
    let mining = w.skills.get(&45).expect("mining stored at 0-based id");
    assert_eq!((mining.value, mining.base, mining.cap), (500, 480, 1000));

    // Single update 0xDF (has cap, NOT 1-based): Mining base ticks to 482.
    let mut s = PacketWriter::new();
    s.u8(0x3A).u16(0).u8(0xDF);
    s.u16(45).u16(502).u16(482).u8(0).u16(1000);
    apply_packet(&mut w, &s.into_vec());
    assert_eq!(w.skills.get(&45).unwrap().base, 482);
}

#[test]
fn target_cursor_sets_and_cancels() {
    let mut w = World::new();
    apply_packet(&mut w, &target_packet(1, 0xDEAD_BEEF, 0));
    let t = w.pending_target.expect("cursor stored");
    assert_eq!(
        (t.target_type, t.cursor_id, t.cursor_flag),
        (1, 0xDEAD_BEEF, 0)
    );

    // flag == 3 is a withdrawal: it clears any pending cursor.
    apply_packet(&mut w, &target_packet(1, 0xDEAD_BEEF, 3));
    assert!(w.pending_target.is_none());
}

/// Fixed 30-byte 0x99 TargetMultiPlacement: `[id][allowGround][cursorId:u32]
/// [flags:u8]` then 11 unused bytes up to absolute offset 18, then
/// `[multiId][xOff][yOff][zOff][hue]` (all u16), then 2 trailing pad bytes
/// — see `multi_placement_tail`'s doc for why those offsets are absolute.
fn multi_placement_packet(
    cursor_id: u32,
    multi_id: u16,
    x_off: u16,
    y_off: u16,
    z_off: u16,
    hue: u16,
) -> Vec<u8> {
    let mut p = PacketWriter::new();
    p.u8(0x99).u8(1).u32(cursor_id).u8(0);
    p.zeros(11); // bytes 7..18, unused here
    p.u16(multi_id).u16(x_off).u16(y_off).u16(z_off).u16(hue);
    p.zeros(2); // pad to the fixed 30-byte packet
    p.into_vec()
}

#[test]
fn multi_target_cursor_stores_ground_target_and_placement_footprint() {
    let mut w = World::new();
    apply_packet(
        &mut w,
        &multi_placement_packet(0xCAFE_BABE, 0x64, 1, 2, 3, 0x0044),
    );
    // Same reply path as always: a plain ground target so the brain
    // answers with an ordinary `Action::TargetGround`.
    let t = w.pending_target.expect("ground cursor stored");
    assert_eq!(
        (t.target_type, t.cursor_id, t.cursor_flag),
        (1, 0xCAFE_BABE, 0)
    );
    let mp = w
        .pending_multi_placement
        .expect("placement footprint stored");
    assert_eq!(
        (mp.multi_id, mp.x_off, mp.y_off, mp.z_off, mp.hue),
        (0x64, 1, 2, 3, 0x0044)
    );

    // A fresh plain 0x6C is not a multi placement — any earlier footprint
    // must not linger past it.
    apply_packet(&mut w, &target_packet(0, 0x1234, 0));
    assert!(w.pending_multi_placement.is_none());
}

#[test]
fn multi_target_cursor_short_frame_skips_placement_not_target() {
    let mut w = World::new();
    // Only the header ServUO always sends (id+allowGround+cursorId+flags),
    // truncated well before the multi-id/offset/hue tail: must still store
    // the plain ground target (the reply path must not change) but leave
    // the footprint absent rather than erroring.
    let mut p = PacketWriter::new();
    p.u8(0x99).u8(1).u32(0xFEED).u8(0);
    apply_packet(&mut w, &p.into_vec());
    let t = w.pending_target.expect("ground cursor stored");
    assert_eq!((t.target_type, t.cursor_id), (1, 0xFEED));
    assert!(w.pending_multi_placement.is_none());
}

#[test]
fn multi_target_cursor_withdrawal_clears_placement() {
    let mut w = World::new();
    apply_packet(&mut w, &multi_placement_packet(0xAAAA, 0x64, 0, 0, 0, 0));
    assert!(w.pending_multi_placement.is_some());

    // flag == 3 on a plain 0x6C withdraws the cursor the multi placement
    // also set — the footprint must disappear with it.
    apply_packet(&mut w, &target_packet(1, 0xAAAA, 3));
    assert!(w.pending_target.is_none());
    assert!(w.pending_multi_placement.is_none());
}

#[test]
fn mobile_moving_updates_world() {
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0x77)
        .u32(0xABCD)
        .u16(0x0190) // body
        .u16(100) // x
        .u16(200) // y
        .u8(5i8 as u8) // z
        .u8(0x03) // dir
        .u16(0) // hue
        .u8(0) // flags
        .u8(1); // notoriety
    assert!(apply_packet(&mut w, &p.into_vec()));
    let m = &w.mobiles[&0xABCD];
    assert_eq!((m.pos.x, m.pos.y, m.pos.z), (100, 200, 5));
    assert_eq!(m.body, 0x0190);
    assert_eq!(m.notoriety, 1);
}

/// A fixed 0x77 MobileMoving frame with a chosen status-flags byte
/// (`flags`), otherwise identical to `mobile_moving_updates_world`.
fn mobile_moving_frame(serial: u32, flags: u8) -> Vec<u8> {
    let mut p = PacketWriter::new();
    p.u8(0x77)
        .u32(serial)
        .u16(0x0190) // body
        .u16(100) // x
        .u16(200) // y
        .u8(5i8 as u8) // z
        .u8(0x03) // dir
        .u16(0) // hue
        .u8(flags)
        .u8(1); // notoriety
    p.into_vec()
}

#[test]
fn mobile_moving_hidden_flag_sets_and_clears() {
    let mut w = World::new();
    // Bit 0x80 set → hidden.
    assert!(apply_packet(
        &mut w,
        &mobile_moving_frame(0xBEEF, FLAG_HIDDEN)
    ));
    assert!(w.mobiles[&0xBEEF].hidden);

    // A later update that omits the bit clears it back — not sticky.
    assert!(apply_packet(&mut w, &mobile_moving_frame(0xBEEF, 0x00)));
    assert!(!w.mobiles[&0xBEEF].hidden);
}

#[test]
fn mobile_moving_no_hidden_flag_stays_false() {
    let mut w = World::new();
    assert!(apply_packet(&mut w, &mobile_moving_frame(0xCAFE, 0x00)));
    assert!(!w.mobiles[&0xCAFE].hidden);
}

/// A variable-length 0x78 MobileIncoming frame (id + u16 length + fixed
/// fields, no worn-item records) with a chosen status-flags byte.
fn mobile_incoming_frame(serial: u32, flags: u8) -> Vec<u8> {
    let mut p = PacketWriter::new();
    p.u8(0x78).u16(0); // id + length placeholder
    p.u32(serial)
        .u16(0x0190) // body
        .u16(100) // x
        .u16(200) // y
        .u8(5i8 as u8) // z
        .u8(0x03) // dir
        .u16(0) // hue
        .u8(flags)
        .u8(1); // notoriety
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;
    frame
}

/// The same frame plus a worn-item list — `(serial, graphic, layer, hue)` each,
/// terminated by the zero serial the wire uses.
fn mobile_incoming_frame_with_equipment(serial: u32, worn: &[(u32, u16, u8, u16)]) -> Vec<u8> {
    let mut p = PacketWriter::new();
    p.bytes(&mobile_incoming_frame(serial, 0x00));
    for &(item, graphic, layer, hue) in worn {
        p.u32(item).u16(graphic).u8(layer).u16(hue);
    }
    p.u32(0); // end-of-list terminator
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;
    frame
}

#[test]
fn mobile_incoming_drops_equipment_the_new_list_omits() {
    // The list is the mobile's COMPLETE visible equipment, so gear it no longer
    // names has been taken off. Before this, an unequipped item kept
    // `container == Some(mobile)` forever and every consumer that asks "what is
    // this mobile wearing" kept seeing it — a dismount out of view left the
    // mount drawn under its rider.
    const HELMET: u32 = 0x4000_0001;
    const SWORD: u32 = 0x4000_0002;
    const MOUNT: u32 = 0x4000_0003;
    let mut w = World::new();
    assert!(apply_packet(
        &mut w,
        &mobile_incoming_frame_with_equipment(
            0xABCD,
            &[
                (HELMET, 0x140A, 0x06, 0),   // Helmet
                (SWORD, 0x0F5E, 0x01, 0),    // OneHanded
                (MOUNT, 0x3E9F, 0x19, 0x84), // Mount
            ],
        )
    ));
    assert_eq!(w.items[&HELMET].container, Some(0xABCD));
    assert_eq!(w.items[&MOUNT].hue, 0x84);

    // The rider dismounts and sheathes: a fresh 0x78 lists only the helmet.
    assert!(apply_packet(
        &mut w,
        &mobile_incoming_frame_with_equipment(0xABCD, &[(HELMET, 0x140A, 0x06, 0)])
    ));
    assert_eq!(w.items[&HELMET].container, Some(0xABCD));
    assert!(!w.items.contains_key(&SWORD), "sheathed weapon lingered");
    assert!(!w.items.contains_key(&MOUNT), "ghost mount lingered");
}

#[test]
fn mobile_incoming_keeps_the_backpack_and_the_container_layers() {
    // The sweep must skip the layers a mobile does not *wear*. The backpack is
    // never in the incoming list yet obviously survives; a vendor's shop
    // containers (0x1A-0x1C) and the bank box (0x1D) hang off the mobile the
    // same way, and dropping those would empty the buy window — see
    // `recorrelate_shop_buy`, which resolves prices through exactly that link.
    const BACKPACK: u32 = 0x4000_0010;
    const SHOP_BUY: u32 = 0x4000_0011;
    const BANK: u32 = 0x4000_0012;
    const CLOAK: u32 = 0x4000_0013;
    let mut w = World::new();
    for (serial, layer) in [
        (BACKPACK, 0x15),
        (SHOP_BUY, 0x1B),
        (BANK, 0x1D),
        (CLOAK, 0x14),
    ] {
        let it = w.item_mut(serial);
        it.layer = layer;
        it.container = Some(0xABCD);
    }
    // A 0x78 naming no equipment at all — the strongest form of the sweep.
    assert!(apply_packet(
        &mut w,
        &mobile_incoming_frame_with_equipment(0xABCD, &[])
    ));
    assert!(w.items.contains_key(&BACKPACK), "backpack was swept");
    assert!(w.items.contains_key(&SHOP_BUY), "shop container was swept");
    assert!(w.items.contains_key(&BANK), "bank box was swept");
    assert!(
        !w.items.contains_key(&CLOAK),
        "worn cloak survived the sweep"
    );
}

#[test]
fn mobile_incoming_leaves_other_mobiles_equipment_alone() {
    // The sweep is scoped to the mobile the packet is about. Two characters
    // standing together must not undress each other.
    const MINE: u32 = 0x4000_0020;
    const THEIRS: u32 = 0x4000_0021;
    let mut w = World::new();
    for (serial, owner) in [(MINE, 0xABCD), (THEIRS, 0xBEEF)] {
        let it = w.item_mut(serial);
        it.layer = 0x14; // Cloak
        it.container = Some(owner);
    }
    assert!(apply_packet(
        &mut w,
        &mobile_incoming_frame_with_equipment(0xABCD, &[])
    ));
    assert!(!w.items.contains_key(&MINE));
    assert!(w.items.contains_key(&THEIRS));
}

#[test]
fn mobile_incoming_withholds_the_sweep_on_a_truncated_list() {
    // A record cut off mid-way is not the server saying "everything else came
    // off" — it's a frame we could not finish reading. Before the sweep existed
    // that distinction cost nothing (a short tail just meant some items went
    // un-updated); now it decides whether real gear gets deleted.
    const HELMET: u32 = 0x4000_0040;
    const CLOAK: u32 = 0x4000_0041;
    let mut w = World::new();
    assert!(apply_packet(
        &mut w,
        &mobile_incoming_frame_with_equipment(
            0xABCD,
            &[(HELMET, 0x140A, 0x06, 0), (CLOAK, 0x1515, 0x14, 0)],
        )
    ));

    // Same packet, but the second record is chopped after its serial.
    let mut p = PacketWriter::new();
    p.bytes(&mobile_incoming_frame(0xABCD, 0x00));
    p.u32(HELMET).u16(0x140A).u8(0x06).u16(0);
    p.u32(CLOAK).u16(0x1515); // …and then the frame ends mid-record
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;
    assert!(apply_packet(&mut w, &frame));
    assert!(
        w.items.contains_key(&CLOAK),
        "a truncated list must not be read as a removal"
    );
}

#[test]
fn mobile_incoming_keeps_opl_for_gear_that_never_came_off() {
    // Why this departs from ClassicUO, which removes every non-backpack child
    // and recreates the listed ones: recreating an item that never left drops
    // its OPL, so ClassicUO silently invalidates the tooltip of every piece of
    // gear each time its wearer walks back into view.
    const SWORD: u32 = 0x4000_0030;
    let mut w = World::new();
    let worn = [(SWORD, 0x0F5E, 0x01, 0)];
    assert!(apply_packet(
        &mut w,
        &mobile_incoming_frame_with_equipment(0xABCD, &worn)
    ));
    w.opl
        .insert(SWORD, vec![(1_050_045, "\t\tLongsword".into())]);
    w.opl_revision.insert(SWORD, 42);
    assert!(apply_packet(
        &mut w,
        &mobile_incoming_frame_with_equipment(0xABCD, &worn)
    ));
    assert!(w.opl.contains_key(&SWORD), "tooltip was needlessly dropped");
    assert_eq!(w.opl_revision.get(&SWORD), Some(&42));
}

#[test]
fn mobile_incoming_hidden_flag_sets_and_clears() {
    let mut w = World::new();
    assert!(apply_packet(
        &mut w,
        &mobile_incoming_frame(0xABCD, FLAG_HIDDEN)
    ));
    assert!(w.mobiles[&0xABCD].hidden);

    // A fresh 0x78 without the bit flips it back — proves it's not sticky.
    assert!(apply_packet(&mut w, &mobile_incoming_frame(0xABCD, 0x00)));
    assert!(!w.mobiles[&0xABCD].hidden);
}

#[test]
fn mobile_incoming_war_mode_and_paralyzed_flags_decode() {
    // 0x78 MobileIncoming: war-mode (0x40) and paralyzed (0x01) both ride
    // the same status-flags byte as hidden — for another mobile, this
    // byte is the only wire source for either.
    let mut w = World::new();
    assert!(apply_packet(
        &mut w,
        &mobile_incoming_frame(0xF00D, FLAG_WARMODE | FLAG_PARALYZED)
    ));
    let m = &w.mobiles[&0xF00D];
    assert!(m.war_mode);
    assert!(m.paralyzed);
    assert!(!m.hidden, "unrelated bit must stay false");

    // A fresh 0x78 that omits both bits clears them back — not sticky.
    assert!(apply_packet(&mut w, &mobile_incoming_frame(0xF00D, 0x00)));
    let m = &w.mobiles[&0xF00D];
    assert!(!m.war_mode);
    assert!(!m.paralyzed);
}

#[test]
fn mobile_update_hidden_flag_is_the_self_feedback_path() {
    // 0x20 MobileUpdate is fixed-length, no length prefix: serial, body,
    // graphic_inc, hue, flags, x, y, server_id, dir, z.
    let mut w = World::new();
    let build = |flags: u8| {
        let mut p = PacketWriter::new();
        p.u32(0x1001) // serial
            .u16(0x0190) // body
            .u8(0) // graphic_inc
            .u16(0) // hue
            .u8(flags)
            .u16(100) // x
            .u16(200) // y
            .u16(0) // server_id
            .u8(0x03) // dir
            .u8(5i8 as u8); // z
        let mut frame = p.into_vec();
        frame.insert(0, 0x20);
        frame
    };
    assert!(apply_packet(&mut w, &build(FLAG_HIDDEN)));
    assert!(w.mobiles[&0x1001].hidden);

    assert!(apply_packet(&mut w, &build(0x00)));
    assert!(!w.mobiles[&0x1001].hidden, "hidden must not be sticky");
}

#[test]
fn health_bar_status_poison_sets_and_clears() {
    // 0x16/0x17 NewHealthbarUpdate/MobileHealthbarStatus:
    // [id][len:u16][serial:u32][count:u16] then count × [type:u16][flag:u8].
    // type 1 = poison bar (ServUO HealthbarPoison writes `p.Level + 1`, i.e.
    // > 0 while poisoned); type 2 = yellow/blessed bar.
    let build = |id: u8, type_: u16, flag: u8| {
        let mut p = PacketWriter::new();
        p.u8(id).u16(0); // id + length placeholder
        p.u32(0x0BAD).u16(1).u16(type_).u8(flag);
        let mut v = p.into_vec();
        let len = v.len() as u16;
        v[1] = (len >> 8) as u8;
        v[2] = (len & 0xFF) as u8;
        v
    };
    let mut w = World::new();
    w.mobile_mut(0x0BAD); // existing-only: pre-create, no phantom spawn
                          // Poison level 2 → flag byte 3 (>0) → poisoned, level = 3 - 1 = 2.
    assert!(apply_packet(&mut w, &build(0x17, 1, 3)));
    assert!(w.mobiles[&0x0BAD].poisoned);
    assert_eq!(w.mobiles[&0x0BAD].poison_level, 2);
    // Cured → flag 0 → not poisoned (not sticky), level -1.
    assert!(apply_packet(&mut w, &build(0x17, 1, 0)));
    assert!(!w.mobiles[&0x0BAD].poisoned);
    assert_eq!(w.mobiles[&0x0BAD].poison_level, -1);
    // A yellow-healthbar update (type 2) must NOT touch the poison flag,
    // and must work identically via 0x16.
    assert!(apply_packet(&mut w, &build(0x17, 1, 2))); // re-poison
    assert!(apply_packet(&mut w, &build(0x16, 2, 1))); // blessed/yellow, type 2, via 0x16
    assert!(
        w.mobiles[&0x0BAD].poisoned,
        "type-2 update left poison alone"
    );
    assert!(
        w.mobiles[&0x0BAD].yellow_health,
        "type-2 sets yellow_health"
    );
    // A poison-only packet must not disturb yellow_health.
    assert!(apply_packet(&mut w, &build(0x17, 1, 0))); // cure, type 1 only
    assert!(
        w.mobiles[&0x0BAD].yellow_health,
        "type-1 update left yellow_health alone"
    );
}

#[test]
fn health_bar_status_does_not_spawn_phantom_mobile() {
    // Unlike the old mobile_mut-based implementation, a status packet for
    // a serial we don't already know must be a no-op (ClassicUO's
    // NewHealthbarUpdate returns early when Mobiles.Get(serial) is null).
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0x17).u16(0);
    p.u32(0xDEAD).u16(1).u16(1).u8(3);
    let mut v = p.into_vec();
    let len = v.len() as u16;
    v[1] = (len >> 8) as u8;
    v[2] = (len & 0xFF) as u8;
    assert!(apply_packet(&mut w, &v));
    assert!(!w.mobiles.contains_key(&0xDEAD));
}

#[test]
fn update_mobile_status_recognized_but_applies_no_state() {
    // 0xDE UpdateMobileStatus: ClassicUO's handler applies no state — we
    // match that (recognized + parsed, no World mutation, no phantom
    // mobile created).
    let mut w = World::new();
    // status == 0: no trailing attacker serial.
    let mut p = PacketWriter::new();
    p.u8(0xDE).u16(0).u32(0x1234).u8(0);
    let mut v = p.into_vec();
    let len = v.len() as u16;
    v[1] = (len >> 8) as u8;
    v[2] = (len & 0xFF) as u8;
    assert!(apply_packet(&mut w, &v));
    assert!(w.mobiles.is_empty(), "must not create a phantom mobile");

    // status == 1: trailing attacker serial present.
    let mut q = PacketWriter::new();
    q.u8(0xDE).u16(0).u32(0x1234).u8(1).u32(0x5678);
    let mut v = q.into_vec();
    let len = v.len() as u16;
    v[1] = (len >> 8) as u8;
    v[2] = (len & 0xFF) as u8;
    assert!(apply_packet(&mut w, &v));
    assert!(w.mobiles.is_empty());
}

#[test]
fn semivisible_recognized_but_applies_no_state() {
    // 0xC4 Semivisible: ClassicUO's handler is an empty no-op; we parse it
    // for recognition only.
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0xC4).u32(0xABCD).u8(1);
    assert!(apply_packet(&mut w, &p.into_vec()));
    assert!(w.mobiles.is_empty(), "must not create a phantom mobile");
}

#[test]
fn hidden_and_poison_are_independent() {
    // Hidden rides the mobile-flags byte (0x80); poison rides the 0x17
    // health-bar packet — setting one must not disturb the other.
    let mut w = World::new();
    assert!(apply_packet(
        &mut w,
        &mobile_moving_frame(0xF00D, FLAG_HIDDEN)
    ));
    assert!(w.mobiles[&0xF00D].hidden);
    assert!(!w.mobiles[&0xF00D].poisoned);
    let mut p = PacketWriter::new();
    p.u8(0x17).u16(0);
    p.u32(0xF00D).u16(1).u16(1).u8(2); // poison bar, level 1
    let mut v = p.into_vec();
    let len = v.len() as u16;
    v[1] = (len >> 8) as u8;
    v[2] = (len & 0xFF) as u8;
    assert!(apply_packet(&mut w, &v));
    assert!(w.mobiles[&0xF00D].poisoned);
    assert!(w.mobiles[&0xF00D].hidden, "poison update kept hidden");
}

#[test]
fn delete_removes_entity() {
    let mut w = World::new();
    w.mobile_mut(0x55);
    let mut p = PacketWriter::new();
    p.u8(0x1D).u32(0x55);
    assert!(apply_packet(&mut w, &p.into_vec()));
    assert!(!w.mobiles.contains_key(&0x55));
}

#[test]
fn vital_hits_update() {
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0xA1).u32(0x77).u16(120).u16(95); // max, cur
    assert!(apply_packet(&mut w, &p.into_vec()));
    let m = &w.mobiles[&0x77];
    assert_eq!((m.hits, m.hits_max), (95, 120));
}

#[test]
fn mobile_attributes_updates_all_vitals() {
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0x2D)
        .u32(0x77)
        .u16(120)
        .u16(95)
        .u16(80)
        .u16(61)
        .u16(110)
        .u16(87);
    let frame = p.into_vec();
    assert_eq!(frame.len(), 17);
    assert!(apply_packet(&mut w, &frame));
    let m = &w.mobiles[&0x77];
    assert_eq!((m.hits, m.hits_max), (95, 120));
    assert_eq!((m.mana, m.mana_max), (61, 80));
    assert_eq!((m.stam, m.stam_max), (87, 110));
}

#[test]
fn ascii_talk_to_journal() {
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0x1C)
        .u16(0) // length placeholder
        .u32(0x01)
        .u16(0) // graphic
        .u8(0) // type (regular)
        .u16(33) // hue
        .u16(3) // font
        .fixed_ascii("Hastin", 30)
        .bytes(b"hello there\0");
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;
    assert!(apply_packet(&mut w, &frame));
    assert_eq!(w.journal.len(), 1);
    assert_eq!(w.journal[0].name, "Hastin");
    assert_eq!(w.journal[0].text, "hello there");
}

#[test]
fn update_name_sets_name_when_present() {
    // 0x98 UpdateName: [id][len:u16][serial:u32][name ASCII to end of frame].
    // Existing-only (like ClassicUO): the mobile must already exist.
    let mut w = World::new();
    w.mobile_mut(0x1001); // pre-create, as a spatial packet (0x78/0x20) would
    let mut p = PacketWriter::new();
    p.u8(0x98).u16(0).u32(0x1001).bytes(b"Hastin");
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;
    assert!(apply_packet(&mut w, &frame));
    assert_eq!(w.mobiles[&0x1001].name, "Hastin");
}

#[test]
fn update_name_does_not_spawn_phantom_for_unknown_serial() {
    // A 0x98 for a serial we don't already track (a stray/late name reply,
    // or an item serial) must NOT create a phantom mobile at (0,0).
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0x98).u16(0).u32(0xDEAD).bytes(b"Ghost");
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;
    assert!(apply_packet(&mut w, &frame));
    assert!(!w.mobiles.contains_key(&0xDEAD));
}

#[test]
fn health_bar_status_high_poison_flag_does_not_panic() {
    // A malformed poison flag >= 0x80 must not underflow-panic poison_level.
    let mut w = World::new();
    w.mobile_mut(0x0BAD);
    let mut p = PacketWriter::new();
    p.u8(0x17).u16(0).u32(0x0BAD).u16(1).u16(1).u8(0xFF);
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;
    assert!(apply_packet(&mut w, &frame));
    assert!(w.mobiles[&0x0BAD].poisoned);
}

#[test]
fn update_name_skips_empty_name() {
    // ClassicUO's UpdateName skips an empty name rather than blanking
    // whatever name it already has; an empty name also must not create a
    // phantom mobile entry.
    let mut w = World::new();
    w.mobile_mut(0x1001).name = "Hastin".to_string();
    let mut p = PacketWriter::new();
    p.u8(0x98).u16(0).u32(0x1001).bytes(b"\0");
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;
    assert!(apply_packet(&mut w, &frame));
    assert_eq!(w.mobiles[&0x1001].name, "Hastin", "must not blank the name");

    let mut w2 = World::new();
    let mut q = PacketWriter::new();
    q.u8(0x98).u16(0).u32(0x2002).bytes(b"\0");
    let mut frame2 = q.into_vec();
    let len2 = frame2.len() as u16;
    frame2[1] = (len2 >> 8) as u8;
    frame2[2] = (len2 & 0xFF) as u8;
    assert!(apply_packet(&mut w2, &frame2));
    assert!(
        !w2.mobiles.contains_key(&0x2002),
        "must not create a phantom mobile"
    );
}

#[test]
fn damage_queues_event() {
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0x0B).u32(0x0000_1234).u16(17);
    assert!(apply_packet(&mut w, &p.into_vec()));
    assert_eq!(w.recent_damage.last(), Some(&(1, 0x0000_1234, 17)));
    assert_eq!(w.damage_seq, 1);
}

#[test]
fn general_info_new_damage_pushes_damage_event() {
    // 0xBF/0x22 New Damage — the AOS-era twin of 0x0B: [unk:u8][serial:u32][damage:u8].
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0xBF).u16(0).u16(0x0022);
    p.u8(0).u32(0x0000_1234).u8(17);
    assert!(apply_packet(&mut w, &patch_len(p.into_vec())));
    assert_eq!(w.recent_damage.last(), Some(&(1, 0x0000_1234, 17)));
    assert_eq!(w.damage_seq, 1);
}

#[test]
fn general_info_clear_weapon_ability_disarms_the_armed_move() {
    // The defect this closes: an arm was write-only. We set it optimistically
    // when sending 0xD7 (nothing else can — the server never confirms an arm),
    // and 0xBF/0x21 is the only message that takes it back, so without this
    // handler the bar stayed highlighted for the rest of the session.
    let mut w = World::new();
    w.arm_ability(7); // Double Strike
    assert_eq!(w.armed_ability, 7);
    let mut p = PacketWriter::new();
    p.u8(0xBF).u16(0).u16(0x0021); // payload-free (ServUO `ClearWeaponAbility`)
    assert!(apply_packet(&mut w, &patch_len(p.into_vec())));
    assert_eq!(w.armed_ability, 0);
}

#[test]
fn arm_ability_toggles_off_when_the_same_move_is_re_armed() {
    // ClassicUO `UsePrimaryAbility`: re-arming the armed move disarms it
    // (`ability ^= 0x80`), and arming the other slot replaces rather than adds.
    let mut w = World::new();
    w.arm_ability(7);
    w.arm_ability(7);
    assert_eq!(w.armed_ability, 0, "re-arming the same move disarms");
    w.arm_ability(7);
    w.arm_ability(9);
    assert_eq!(w.armed_ability, 9, "arming another move replaces it");
    w.arm_ability(0);
    assert_eq!(w.armed_ability, 0, "0 always disarms");
}

#[test]
fn general_info_toggle_special_ability_tracks_active_spells() {
    // 0xBF/0x25 — [abilityID:u16][active:u8]. ServUO sends `moveID + 1` /
    // `spellID + 1`, so these are SPELL ids, not weapon-ability ids.
    let toggle = |spell: u16, active: u8| {
        let mut p = PacketWriter::new();
        p.u8(0xBF).u16(0).u16(0x0025).u16(spell).u8(active);
        patch_len(p.into_vec())
    };
    let mut w = World::new();
    assert!(apply_packet(&mut w, &toggle(401, 1)));
    assert!(apply_packet(&mut w, &toggle(402, 1)));
    assert_eq!(w.active_spell_icons, vec![401, 402]);
    // Re-asserting an already-active stance must not duplicate it.
    assert!(apply_packet(&mut w, &toggle(401, 1)));
    assert_eq!(w.active_spell_icons, vec![401, 402]);
    assert!(apply_packet(&mut w, &toggle(401, 0)));
    assert_eq!(w.active_spell_icons, vec![402]);
    // Deactivating something that was never active is a harmless no-op.
    assert!(apply_packet(&mut w, &toggle(999, 0)));
    assert_eq!(w.active_spell_icons, vec![402]);
}

#[test]
fn bulletin_summary_upserts_instead_of_duplicating() {
    // A header can be asked for twice — a re-open clears the list and any
    // consumer re-asks — so a blind push showed the same message twice in the
    // board listing.
    let mut w = World::new();
    let board = |name: &str| {
        let mut p = PacketWriter::new();
        p.u8(0x71).u16(0).u8(0).u32(0x4000_0001);
        p.zeros(4); // unused
        p.bytes(name.as_bytes()).zeros(30 - name.len());
        patch_len(p.into_vec())
    };
    assert!(apply_packet(&mut w, &board("a board")));
    let summary = |subject: &str| {
        let mut p = PacketWriter::new();
        p.u8(0x71)
            .u16(0)
            .u8(1)
            .u32(0x4000_0001)
            .u32(0x4000_0002)
            .u32(0);
        p.u8(3).bytes(b"Bob");
        p.u8(subject.len() as u8).bytes(subject.as_bytes());
        p.u8(3).bytes(b"now");
        patch_len(p.into_vec())
    };
    assert!(apply_packet(&mut w, &summary("first")));
    assert!(apply_packet(&mut w, &summary("edited")));
    let b = w.bulletin_board.as_ref().unwrap();
    assert_eq!(b.summaries.len(), 1, "same serial must not duplicate");
    assert_eq!(b.summaries[0].subject, "edited");
}

#[test]
fn play_sound_queues_event() {
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0x54).u8(0).u16(0x0145).u16(0).u16(100).u16(200).u16(0);
    assert!(apply_packet(&mut w, &p.into_vec()));
    assert_eq!(w.recent_sounds.last(), Some(&(1, 0x0145, 100, 200)));
    assert_eq!(w.sound_seq, 1);
}

#[test]
fn graphic_effect_0x70_parsed() {
    let mut w = World::new();
    // 0x70: a Moving fireball (graphic 0x36D4) from 0xAAAA at (100,200,5)
    // to 0xBBBB at (110,210,5), speed 7, duration 30. Hue must be 0 for 0x70.
    let mut p = PacketWriter::new();
    p.u8(0x70)
        .u8(0) // type = Moving
        .u32(0xAAAA) // src serial
        .u32(0xBBBB) // tgt serial
        .u16(0x36D4) // graphic
        .u16(100)
        .u16(200)
        .u8(5i8 as u8) // src x,y,z
        .u16(110)
        .u16(210)
        .u8(5i8 as u8) // tgt x,y,z
        .u8(7) // speed
        .u8(30) // duration
        .u16(0) // unknown
        .u8(0) // fixed direction
        .u8(0); // explode
    let frame = p.into_vec();
    assert_eq!(frame.len(), 28); // 0x70 is 28 bytes
    assert!(apply_packet(&mut w, &frame));
    let e = w.recent_effects.last().expect("effect queued");
    assert_eq!(e.seq, 1);
    assert_eq!(e.kind, 0);
    assert_eq!((e.src_serial, e.tgt_serial), (0xAAAA, 0xBBBB));
    assert_eq!(e.graphic, 0x36D4);
    assert_eq!((e.sx, e.sy, e.sz), (100, 200, 5));
    assert_eq!((e.tx, e.ty, e.tz), (110, 210, 5));
    assert_eq!((e.speed, e.duration, e.hue, e.blend), (7, 30, 0, 0));
    assert!(!e.explodes); // the byte was 0 above
}

/// The `explode` byte decides whether a projectile bursts where it lands
/// (ClassicUO `MovingEffect::RemoveMe` → `FixedEffect(0x36CB)`). It sits right
/// after `fixedDir`, so reading it also pins that neighbour's offset.
#[test]
fn graphic_effect_explode_byte_is_retained() {
    for (byte, want) in [(0u8, false), (1, true), (0xFF, true)] {
        let mut w = World::new();
        let mut p = PacketWriter::new();
        p.u8(0x70)
            .u8(0) // type = Moving
            .u32(0xAAAA)
            .u32(0xBBBB)
            .u16(0x36D4)
            .u16(100)
            .u16(200)
            .u8(5i8 as u8)
            .u16(110)
            .u16(210)
            .u8(5i8 as u8)
            .u8(7) // speed
            .u8(30) // duration
            .u16(0) // unknown
            .u8(3) // fixed direction — deliberately non-zero, must not be read
            .u8(byte); // explode
        assert!(apply_packet(&mut w, &p.into_vec()));
        let e = w.recent_effects.last().expect("effect queued");
        assert_eq!(e.explodes, want, "explode byte {byte:#04X}");
        // The neighbouring bytes must be unaffected by the shifted read.
        assert_eq!((e.speed, e.duration), (7, 30));
    }
}

#[test]
fn hued_effect_0xc0_carries_hue() {
    let mut w = World::new();
    // 0xC0: a FixedFrom effect on serial 0xCAFE with hue 0x0021 (low 16 bits
    // of the u32) and renderMode 2 (Screen).
    let mut p = PacketWriter::new();
    p.u8(0xC0)
        .u8(3) // type = FixedFrom
        .u32(0xCAFE)
        .u32(0xCAFE)
        .u16(0x3728) // graphic
        .u16(50)
        .u16(60)
        .u8(0)
        .u16(50)
        .u16(60)
        .u8(0)
        .u8(10)
        .u8(20)
        .u16(0)
        .u8(0)
        .u8(0)
        .u32(0x0000_0021) // hue u32
        .u32(2); // renderMode = Screen
    let frame = p.into_vec();
    assert_eq!(frame.len(), 36); // 0xC0 is 36 bytes
    assert!(apply_packet(&mut w, &frame));
    let e = w.recent_effects.last().expect("effect queued");
    assert_eq!(e.kind, 3);
    assert_eq!(e.hue, 0x0021);
    assert_eq!(e.blend, 2);
}

#[test]
fn hued_effect_0xc0_blend_wraps_mod_7() {
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0xC0)
        .u8(0)
        .u32(1)
        .u32(2)
        .u16(0x36D4)
        .u16(1)
        .u16(1)
        .u8(0)
        .u16(2)
        .u16(2)
        .u8(0)
        .u8(1)
        .u8(1)
        .u16(0)
        .u8(0)
        .u8(0)
        .u32(0)
        .u32(9); // 9 % 7 = 2
    assert!(apply_packet(&mut w, &p.into_vec()));
    assert_eq!(w.recent_effects.last().unwrap().blend, 2);
}

#[test]
fn play_music_sets_and_stops() {
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0x6D).u16(0x0009);
    assert!(apply_packet(&mut w, &p.into_vec()));
    assert_eq!(w.current_music, Some(0x0009));

    let mut s = PacketWriter::new();
    s.u8(0x6D).u16(0xFFFF);
    assert!(apply_packet(&mut w, &s.into_vec()));
    assert_eq!(w.current_music, None);
}

#[test]
fn overall_light_level_stored() {
    let mut w = World::new();
    assert_eq!(w.light_level, 0); // default = brightest day
    let mut p = PacketWriter::new();
    p.u8(0x4F).u8(0x18); // dusk
    assert!(apply_packet(&mut w, &p.into_vec()));
    assert_eq!(w.light_level, 0x18);
    assert_eq!(w.effective_light(), 0x18);
}

#[test]
fn personal_light_combines_with_overall() {
    let mut w = World::new();
    w.player = Some(crate::types::Serial(0x42));
    w.light_level = 0x18; // dusk
                          // Night Sight: a personal light bright enough to beat `32 - overall`
                          // lifts the scene. 32 - max(25, 32 - 24) = 7.
    let mut p = PacketWriter::new();
    p.u8(0x4E).u32(0x42).u8(25);
    assert!(apply_packet(&mut w, &p.into_vec()));
    assert_eq!(w.personal_light, Some(25));
    assert_eq!(w.effective_light(), 7);

    // A personal light for someone else is ignored.
    let mut q = PacketWriter::new();
    q.u8(0x4E).u32(0x99).u8(0x00);
    assert!(apply_packet(&mut w, &q.into_vec()));
    assert_eq!(w.personal_light, Some(25));
}

/// Regression: ServUO pairs 0x4E with every 0x4F and sends personal = 0 for a
/// character with no Night Sight. Combining the two with `min` pinned the
/// result to 0, so night never darkened; combining them as ClassicUO does
/// leaves the overall level untouched.
#[test]
fn zero_personal_light_does_not_cancel_the_night() {
    let mut w = World::new();
    w.player = Some(crate::types::Serial(0x42));
    for overall in [0x00u8, 0x0C, 0x18, 0x1A] {
        let mut p = PacketWriter::new();
        p.u8(0x4F).u8(overall);
        assert!(apply_packet(&mut w, &p.into_vec()));
        let mut q = PacketWriter::new();
        q.u8(0x4E).u32(0x42).u8(0x00);
        assert!(apply_packet(&mut w, &q.into_vec()));
        assert_eq!(w.effective_light(), overall, "overall {overall:#04X}");
    }

    // A personal light weaker than the ambient brightness changes nothing.
    w.light_level = 0x0C;
    w.personal_light = Some(4);
    assert_eq!(w.effective_light(), 0x0C);
}

#[test]
fn weather_sets_and_resets() {
    let mut w = World::new();
    assert_eq!(w.weather.kind, 0xFF); // default = none
                                      // Rain, 40 particles.
    let mut p = PacketWriter::new();
    p.u8(0x65).u8(0).u8(40).u8(70);
    assert!(apply_packet(&mut w, &p.into_vec()));
    assert_eq!((w.weather.kind, w.weather.intensity), (0, 40));

    // Reset to none.
    let mut q = PacketWriter::new();
    q.u8(0x65).u8(0xFE).u8(0).u8(0);
    assert!(apply_packet(&mut w, &q.into_vec()));
    assert_eq!(w.weather.kind, 0xFE);
}

#[test]
fn season_sets_field() {
    let mut w = World::new();
    assert_eq!(w.season, 0); // default = Spring
                             // 0xBC: Winter (3), playMusic = 1.
    let mut p = PacketWriter::new();
    p.u8(0xBC).u8(3).u8(1);
    assert!(apply_packet(&mut w, &p.into_vec()));
    assert_eq!(w.season, 3);
}

#[test]
fn client_view_range_sets_field() {
    let mut w = World::new();
    assert_eq!(w.client_view_range, 18); // UO's stock default
    let mut p = PacketWriter::new();
    p.u8(0xC8).u8(24);
    assert!(apply_packet(&mut w, &p.into_vec()));
    assert_eq!(w.client_view_range, 24);
}

#[test]
fn set_time_sets_game_time() {
    let mut w = World::new();
    assert_eq!(w.game_time, None);
    let mut p = PacketWriter::new();
    p.u8(0x5B).u8(13).u8(45).u8(9);
    assert!(apply_packet(&mut w, &p.into_vec()));
    assert_eq!(
        w.game_time,
        Some(GameTime {
            hour: 13,
            minute: 45,
            second: 9,
        })
    );
}

#[test]
fn war_mode_sets_field() {
    let mut w = World::new();
    assert!(!w.war); // default = peace
                     // 0x72: war on, trailing fixed padding 0x00 0x32 0x00.
    let mut on = PacketWriter::new();
    on.u8(0x72).u8(1).u8(0x00).u8(0x32).u8(0x00);
    assert!(apply_packet(&mut w, &on.into_vec()));
    assert!(w.war);
    // 0x72: war off.
    let mut off = PacketWriter::new();
    off.u8(0x72).u8(0).u8(0x00).u8(0x32).u8(0x00);
    assert!(apply_packet(&mut w, &off.into_vec()));
    assert!(!w.war);
}

#[test]
fn buff_add_and_remove() {
    let mut w = World::new();
    // 0xDF add: Bless (icon 0x0418), 3600s duration, for our serial.
    let mut p = PacketWriter::new();
    p.u8(0xDF).u16(0); // id, len placeholder
    p.u32(0x42).u16(0x0418).u16(1); // serial, icon, count=1 (add)
    p.u16(0).u16(0).u16(0x0418).u16(0).u32(0); // source, pad, icon, queue, pad
    p.u16(3600); // timer (seconds)
    p.zeros(3).u32(0).u32(0).u32(0); // pad + 3 clilocs (parser stops at timer)
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;
    assert!(apply_packet(&mut w, &frame));
    assert_eq!(w.buffs.len(), 1);
    assert_eq!(w.buffs[0].icon, 0x0418);
    assert_eq!(w.buffs[0].name, "Bless");
    assert_eq!(w.buffs[0].dur, 3600);

    // Re-add same icon → upsert (no duplicate), new duration.
    let mut p2 = PacketWriter::new();
    p2.u8(0xDF).u16(0).u32(0x42).u16(0x0418).u16(1);
    p2.u16(0).u16(0).u16(0x0418).u16(0).u32(0).u16(120);
    apply_packet(&mut w, &p2.into_vec());
    assert_eq!(w.buffs.len(), 1);
    assert_eq!(w.buffs[0].dur, 120);

    // 0xDF remove: count=0 drops the icon.
    let mut q = PacketWriter::new();
    q.u8(0xDF).u16(0).u32(0x42).u16(0x0418).u16(0); // count=0
    assert!(apply_packet(&mut w, &q.into_vec()));
    assert!(w.buffs.is_empty());
}

#[test]
fn buff_unknown_icon_falls_back() {
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0xDF).u16(0).u32(1).u16(0x0999).u16(1);
    p.u16(0).u16(0).u16(0x0999).u16(0).u32(0).u16(0);
    apply_packet(&mut w, &p.into_vec());
    assert_eq!(w.buffs[0].name, "#2457"); // 0x0999 = 2457, no table entry
    assert_eq!(w.buffs[0].dur, 0); // dur 0 = permanent / no timer
}

fn legacy_menu_packet(
    serial: u32,
    menu_id: u16,
    question: &str,
    entries: &[(u16, u16, &str)],
) -> Vec<u8> {
    let mut p = PacketWriter::new();
    p.u8(0x7C)
        .u16(0)
        .u32(serial)
        .u16(menu_id)
        .u8(question.len() as u8)
        .bytes(question.as_bytes())
        .u8(entries.len() as u8);
    for &(graphic, hue, text) in entries {
        p.u16(graphic)
            .u16(hue)
            .u8(text.len() as u8)
            .bytes(text.as_bytes());
    }
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1..3].copy_from_slice(&len.to_be_bytes());
    frame
}

#[test]
fn legacy_item_menu_parses_and_same_serial_replaces_atomically() {
    let serial = 0x0102_0304;
    let first = legacy_menu_packet(
        serial,
        7,
        "Choose an item",
        &[(0x0F5E, 0x0481, "Sword"), (0x0EED, 0, "Gold")],
    );
    let mut w = World::new();
    assert!(apply_packet(&mut w, &first));
    let menu = w.legacy_menu(serial).unwrap();
    assert_eq!(menu.menu_id, 7);
    assert_eq!(menu.question, "Choose an item");
    assert_eq!(menu.kind, LegacyMenuKind::Items);
    assert_eq!(menu.entries.len(), 2);
    assert_eq!(menu.entries[0].graphic, 0x0F5E);
    assert_eq!(menu.entries[0].hue, 0x0481);
    assert_eq!(menu.entries[0].text, "Sword");

    // A malformed resend is handled but cannot partially replace the menu.
    let mut truncated = legacy_menu_packet(serial, 9, "Broken", &[(1, 2, "replacement")]);
    truncated.pop();
    assert!(apply_packet(&mut w, &truncated));
    assert_eq!(w.legacy_menu(serial).unwrap().menu_id, 7);

    let replacement = legacy_menu_packet(serial, 9, "Again", &[(0x2006, 3, "Corpse")]);
    assert!(apply_packet(&mut w, &replacement));
    assert_eq!(w.legacy_menus.len(), 1);
    assert_eq!(w.legacy_menu(serial).unwrap().menu_id, 9);
}

#[test]
fn legacy_question_menus_use_zero_item_fields_and_can_coexist() {
    let mut w = World::new();
    let first = legacy_menu_packet(
        10,
        0,
        "Where do you wish to go?",
        &[(0, 0, "Britain"), (0, 0, "Minoc")],
    );
    let second = legacy_menu_packet(20, 3, "Continue?", &[(0, 0, "Yes")]);
    assert!(apply_packet(&mut w, &first));
    assert!(apply_packet(&mut w, &second));
    assert_eq!(w.legacy_menus.len(), 2);
    assert_eq!(w.legacy_menu(10).unwrap().kind, LegacyMenuKind::Question);
    assert_eq!(w.legacy_menu(10).unwrap().entries[1].text, "Minoc");
    assert_eq!(w.legacy_menu(20).unwrap().question, "Continue?");
}

#[test]
fn hue_picker_parses_fixed_packet_upserts_and_coexists() {
    let mut w = World::new();
    assert!(apply_packet(
        &mut w,
        &[0x95, 0x01, 0x02, 0x03, 0x04, 0, 0, 0x0F, 0xAB]
    ));
    assert_eq!(
        w.hue_picker(0x0102_0304),
        Some(&HuePicker {
            serial: 0x0102_0304,
            graphic: 0x0FAB,
        })
    );

    assert!(apply_packet(
        &mut w,
        &[0x95, 0x00, 0x00, 0x00, 0x09, 0, 0, 0x20, 0x06]
    ));
    assert_eq!(w.hue_pickers.len(), 2);

    // A retransmit with the same callback serial refreshes in place.
    assert!(apply_packet(
        &mut w,
        &[0x95, 0x01, 0x02, 0x03, 0x04, 0, 0, 0x0E, 0xED]
    ));
    assert_eq!(w.hue_pickers.len(), 2);
    assert_eq!(w.hue_picker(0x0102_0304).unwrap().graphic, 0x0EED);
}

#[test]
fn truncated_hue_picker_is_ignored_without_partial_state() {
    let mut w = World::new();
    assert!(apply_packet(
        &mut w,
        &[0x95, 0x01, 0x02, 0x03, 0x04, 0, 0, 0x0F]
    ));
    assert!(w.hue_pickers.is_empty());
}

#[test]
fn open_buy_window_parses_prices_and_vendor() {
    let mut w = World::new();
    // The for-sale container (0x4000_0001) is worn by vendor 0xAABB.
    let cont = w.item_mut(0x4000_0001);
    cont.container = Some(0xAABB);

    // 0x3C VendorBuyContent sends the for-sale items **reversed** but with
    // each item's correct buy-list index+1 in its `x` (ServUO overloads x so
    // the client can re-sort). So send them egg-first (x=2) then loaf (x=1) —
    // arrival order is the REVERSE of the buy-list order below — and the
    // recorrelation must still pair price[0] (bread) with the x=1 loaf.
    let mut c = PacketWriter::new();
    c.u8(0x3C).u16(0).u16(2);
    // read_container_item: serial, graphic(u16)+inc(u8), amount, x, y, grid, cont, hue
    c.u32(0x102)
        .u16(0x09B5)
        .u8(0)
        .u16(7)
        .u16(2)
        .u16(1)
        .u8(0)
        .u32(0x4000_0001)
        .u16(0);
    c.u32(0x101)
        .u16(0x103B)
        .u8(0)
        .u16(20)
        .u16(1)
        .u16(1)
        .u8(0)
        .u32(0x4000_0001)
        .u16(0);
    let mut cframe = c.into_vec();
    let clen = cframe.len() as u16;
    cframe[1] = (clen >> 8) as u8;
    cframe[2] = (clen & 0xFF) as u8;
    assert!(apply_packet(&mut w, &cframe));

    // 0x74: container, count=2, two (price, name) entries in the same order.
    let mut p = PacketWriter::new();
    p.u8(0x74).u16(0); // id, len placeholder
    p.u32(0x4000_0001).u8(2);
    p.u32(15).u8(5).bytes(b"bread");
    p.u32(3).u8(3).bytes(b"egg");
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;
    assert!(apply_packet(&mut w, &frame));

    let sb = w.shop_buy.as_ref().expect("buy window stored");
    assert_eq!(sb.vendor, 0xAABB);
    assert_eq!(sb.container, 0x4000_0001);
    assert_eq!(sb.entries.len(), 2);
    // Each price is now paired with its concrete container item by 0x3C order.
    assert_eq!(sb.entries[0].price, 15);
    assert_eq!(sb.entries[0].name, "bread");
    assert_eq!(sb.entries[0].serial, 0x101);
    assert_eq!(sb.entries[0].graphic, 0x103B);
    assert_eq!(sb.entries[0].amount, 20);
    assert_eq!(sb.entries[1].price, 3);
    assert_eq!(sb.entries[1].name, "egg");
    assert_eq!(sb.entries[1].serial, 0x102);
    assert_eq!(sb.entries[1].graphic, 0x09B5);
    assert_eq!(sb.entries[1].amount, 7);
}

#[test]
fn sell_list_parses_items() {
    let mut w = World::new();
    // 0x9E: vendor 0xAABB will buy one item from our pack.
    let mut p = PacketWriter::new();
    p.u8(0x9E).u16(0); // id, len placeholder
    p.u32(0xAABB).u16(1);
    p.u32(0x4000_0009) // serial
        .u16(0x0EED) // graphic (gold-ish)
        .u16(0) // hue
        .u16(7) // amount
        .u16(12) // price
        .u16(6)
        .bytes(b"dagger"); // nameLen + name
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;
    assert!(apply_packet(&mut w, &frame));

    let ss = w.shop_sell.as_ref().expect("sell list stored");
    assert_eq!(ss.vendor, 0xAABB);
    assert_eq!(ss.items.len(), 1);
    let it = &ss.items[0];
    assert_eq!(it.serial, 0x4000_0009);
    assert_eq!((it.graphic, it.amount, it.price), (0x0EED, 7, 12));
    assert_eq!(it.name, "dagger");
}

#[test]
fn display_gump_parses_layout_and_text() {
    let mut w = World::new();
    // 0xB0: a tiny dialog — one button + one text line ("Hi").
    let layout = "{ resizepic 0 0 5054 200 100 }{ button 20 70 247 248 1 0 1 }{ text 20 20 0 0 }";
    let mut p = PacketWriter::new();
    p.u8(0xB0).u16(0); // id, len placeholder
    p.u32(0xDEAD_BEEF) // serial
        .u32(0x0000_002A) // gumpId
        .u32(100) // x
        .u32(50); // y
    p.u16(layout.len() as u16).bytes(layout.as_bytes());
    p.u16(1); // textLinesCount
    p.u16(2); // charLen for "Hi"
    p.u16(b'H' as u16).u16(b'i' as u16); // UTF-16 BE (writer is BE)
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;
    assert!(apply_packet(&mut w, &frame));

    assert_eq!(w.gumps.len(), 1);
    let g = &w.gumps[0];
    assert_eq!(g.serial, 0xDEAD_BEEF);
    assert_eq!(g.gump_id, 0x2A);
    assert_eq!((g.x, g.y), (100, 50));
    assert_eq!(g.layout, layout);
    assert_eq!(g.text, vec!["Hi".to_string()]);

    // A re-send with the same serial upserts in place (no duplicate).
    apply_packet(&mut w, &frame);
    assert_eq!(w.gumps.len(), 1);

    // close_gump drops it.
    w.close_gump(0xDEAD_BEEF);
    assert!(w.gumps.is_empty());
}

#[test]
fn quest_arrow_show_and_hide() {
    let mut w = World::new();
    // 0xBA: show an arrow pointing at (1234, 5678), with a trailing serial (HS
    // form) the handler should read past and ignore.
    let mut p = PacketWriter::new();
    p.u8(0xBA).u8(1).u16(1234).u16(5678).u32(0xDEAD_BEEF);
    assert!(apply_packet(&mut w, &p.into_vec()));
    assert_eq!(w.quest_arrow, Some((1234, 5678)));

    // active = 0 hides it.
    let mut q = PacketWriter::new();
    q.u8(0xBA).u8(0).u16(0).u16(0).u32(0);
    assert!(apply_packet(&mut w, &q.into_vec()));
    assert_eq!(w.quest_arrow, None);
}

#[test]
fn open_book_header_parsed() {
    let mut w = World::new();
    // 0x93: a 2-page writable book "My Diary" by "Anima".
    let mut p = PacketWriter::new();
    p.u8(0x93).u32(0x4000_0001).u8(1).u8(0).u16(2);
    p.fixed_ascii("My Diary", 60).fixed_ascii("Anima", 30);
    let frame = p.into_vec();
    assert_eq!(frame.len(), 99); // 0x93 is fixed 99 bytes
    assert!(apply_packet(&mut w, &frame));
    let b = w.book.as_ref().expect("book opened");
    assert_eq!(b.serial, 0x4000_0001);
    assert_eq!(b.title, "My Diary");
    assert_eq!(b.author, "Anima");
    assert!(b.writable);
    assert_eq!(b.page_count, 2);
    assert_eq!(b.pages.len(), 2);
    assert!(b.pages[0].is_empty());
}

#[test]
fn book_data_fills_pages() {
    let mut w = World::new();
    // Open a 2-page book first (so book_data has somewhere to write).
    let mut h = PacketWriter::new();
    h.u8(0x93).u32(0x55).u8(0).u8(0).u16(2);
    h.fixed_ascii("Tome", 60).fixed_ascii("Sage", 30);
    apply_packet(&mut w, &h.into_vec());

    // 0x66: page 1 has two lines, page 2 has one line.
    let mut p = PacketWriter::new();
    p.u8(0x66).u16(0); // id + length placeholder
    p.u32(0x55).u16(2); // serial, page count
    p.u16(1).u16(2).bytes(b"line one\0").bytes(b"line two\0");
    p.u16(2).u16(1).bytes(b"page two\0");
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;
    assert!(apply_packet(&mut w, &frame));

    let b = w.book.as_ref().expect("book present");
    assert_eq!(
        b.pages[0],
        vec!["line one".to_string(), "line two".to_string()]
    );
    assert_eq!(b.pages[1], vec!["page two".to_string()]);
}

#[test]
fn typed_animation_stores_kind_action_and_mode() {
    let mut w = World::new();
    // 0xE2 NewMobileAnimation: serial 0xDEAD_BEEF, AnimationType::Emote (7),
    // action 1 ("salute"), mode/delay 42 — matches ServUO's `.salute` emote.
    let mut p = PacketWriter::new();
    p.u8(0xE2).u32(0xDEAD_BEEF).u16(7).u16(1).u8(42);
    let frame = p.into_vec();
    assert_eq!(frame.len(), 10); // ServUO NewMobileAnimation : base(0xE2, 10)
    assert!(apply_packet(&mut w, &frame));
    let (seq, serial, kind, action, mode) =
        *w.recent_typed_anims.last().expect("typed anim recorded");
    assert_eq!(seq, 1);
    assert_eq!(serial, 0xDEAD_BEEF);
    assert_eq!(kind, 7); // Emote
    assert_eq!(action, 1); // salute
    assert_eq!(mode, 42);
}

#[test]
fn unknown_packet_ignored() {
    let mut w = World::new();
    // 0x9B is fixed-len but not handled → recognized=false
    assert!(!apply_packet(&mut w, &[0x9B, 0, 0]));
}

#[test]
fn display_death_links_corpse_and_prunes_on_delete() {
    let mut w = World::new();
    // 0xAF: killed mobile 0xAAAA's corpse is item 0x4000_0001.
    let mut p = PacketWriter::new();
    p.u8(0xAF).u32(0xAAAA).u32(0x4000_0001).u32(0);
    let frame = p.into_vec();
    assert_eq!(frame.len(), 13); // ServUO DeathAnimation : base(0xAF, 13)
    assert!(apply_packet(&mut w, &frame));
    assert_eq!(w.corpse_of.get(&0x4000_0001), Some(&0xAAAA));

    // The corpse item despawns (0x1D Delete) — the link is pruned with it.
    let mut d = PacketWriter::new();
    d.u8(0x1D).u32(0x4000_0001);
    assert!(apply_packet(&mut w, &d.into_vec()));
    assert!(!w.corpse_of.contains_key(&0x4000_0001));
}

#[test]
fn change_combatant_sets_and_clears() {
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0xAA).u32(0xDEAD_BEEF);
    let frame = p.into_vec();
    assert_eq!(frame.len(), 5); // ServUO ChangeCombatant : base(0xAA, 5)
    assert!(apply_packet(&mut w, &frame));
    assert_eq!(w.combatant, Some(0xDEAD_BEEF));

    // serial 0 = combat ended.
    let mut q = PacketWriter::new();
    q.u8(0xAA).u32(0);
    assert!(apply_packet(&mut w, &q.into_vec()));
    assert_eq!(w.combatant, None);
}

#[test]
fn follow_r_sets_and_clears_target() {
    let mut w = World::new();
    // 0x15 FollowR: [id][follower:u32][followed:u32] (9 bytes).
    let mut p = PacketWriter::new();
    p.u8(0x15).u32(0x1111).u32(0x2222);
    let frame = p.into_vec();
    assert_eq!(frame.len(), 9); // ServUO FollowMessage : base(0x15, 9)
    assert!(apply_packet(&mut w, &frame));
    assert_eq!(w.follow_target, Some(0x2222));

    // followed serial 0 clears the target.
    let mut q = PacketWriter::new();
    q.u8(0x15).u32(0x1111).u32(0);
    assert!(apply_packet(&mut w, &q.into_vec()));
    assert_eq!(w.follow_target, None);
}

#[test]
fn lift_reject_queues_event() {
    let mut w = World::new();
    // 0x27: reason 3 = BelongsToAnother.
    let mut p = PacketWriter::new();
    p.u8(0x27).u8(3);
    let frame = p.into_vec();
    assert_eq!(frame.len(), 2); // ServUO LiftRej : base(0x27, 2)
    assert!(apply_packet(&mut w, &frame));
    assert_eq!(w.recent_lift_rejects.last(), Some(&(1, 3)));
    assert_eq!(w.lift_reject_seq, 1);
}

#[test]
fn drag_completion_packets_queue_bounded_events() {
    let mut w = World::new();

    let mut p = PacketWriter::new();
    p.u8(0x28).u32(0x4000_1234);
    let frame = p.into_vec();
    assert_eq!(frame.len(), 5);
    assert!(apply_packet(&mut w, &frame));
    assert_eq!(w.recent_drag_completions.len(), 1);
    assert_eq!(w.recent_drag_completions[0].seq, 1);
    assert_eq!(w.recent_drag_completions[0].packet, 0x28);
    assert_eq!(w.recent_drag_completions[0].token, Some(0x4000_1234));

    assert!(apply_packet(&mut w, &[0x29]));
    assert_eq!(w.recent_drag_completions[1].seq, 2);
    assert_eq!(w.recent_drag_completions[1].packet, 0x29);
    assert_eq!(w.recent_drag_completions[1].token, None);

    for _ in 0..20 {
        assert!(apply_packet(&mut w, &[0x29]));
    }
    assert_eq!(w.drag_completion_seq, 22);
    assert_eq!(w.recent_drag_completions.len(), 16);
    assert_eq!(w.recent_drag_completions.first().unwrap().seq, 7);
    assert_eq!(w.recent_drag_completions.last().unwrap().seq, 22);
}

#[test]
fn death_status_applies_classicuo_effects_without_guessing_alive_state() {
    let mut w = World::new();
    w.player = Some(crate::types::Serial(0x1001));
    w.mobile_mut(0x1001).body = 0x0190;
    w.season = 3;
    w.current_music = Some(12);
    w.weather = crate::world::Weather {
        kind: 2,
        intensity: 30,
    };

    assert!(apply_packet(&mut w, &[0x2C, 0]));
    assert_eq!(w.weather, crate::world::Weather::default());
    assert_eq!(w.current_music, Some(42));
    assert_eq!(w.pre_death_season, Some(3));
    assert_eq!(w.pre_death_music, Some(Some(12)));
    assert_eq!(w.death_screen.unwrap().action, 0);
    assert_eq!(w.pending_war_mode_requests, vec![false]);
    // Action 2 is the end of ServUO's same death sequence, not resurrection.
    assert!(apply_packet(&mut w, &[0x2C, 2]));
    assert_eq!(w.death_screen.unwrap().seq, 2);
    assert_eq!(w.pre_death_music, Some(Some(12)));
    assert_eq!(w.pending_war_mode_requests, vec![false, false]);

    // ClassicUO's sole excluded action has no side effects.
    assert!(apply_packet(&mut w, &[0x2C, 1]));
    assert_eq!(w.death_screen.unwrap().seq, 2);
    assert_eq!(w.pending_war_mode_requests, vec![false, false]);
}

#[test]
fn player_body_transition_sets_and_restores_death_environment() {
    let mut w = World::new();
    w.player = Some(crate::types::Serial(0x1001));
    w.mobile_mut(0x1001).body = 0x0190;
    w.season = 2;
    w.current_music = None;

    let update = |body: u16| {
        let mut p = PacketWriter::new();
        p.u8(0x20)
            .u32(0x1001)
            .u16(body)
            .u8(0)
            .u16(0)
            .u8(0)
            .u16(100)
            .u16(200)
            .u16(0)
            .u8(3)
            .u8(0);
        p.into_vec()
    };

    assert!(apply_packet(&mut w, &update(0x0192)));
    assert_eq!(w.season, 4);
    assert_eq!(w.current_music, Some(42));
    assert_eq!(w.pre_death_music, Some(None));

    assert!(apply_packet(&mut w, &update(0x0190)));
    assert_eq!(w.season, 2);
    assert_eq!(w.current_music, None);
    assert_eq!(w.pre_death_season, None);
    assert_eq!(w.pre_death_music, None);
}

#[test]
fn pathfinding_records_every_server_walkto_request() {
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0x38).u16(0x1234).u16(0x5678).u16((-7i16) as u16);
    let frame = p.into_vec();
    assert_eq!(frame.len(), 7);
    assert!(apply_packet(&mut w, &frame));
    assert_eq!(
        w.server_pathfind,
        Some(crate::world::ServerPathfindRequest {
            seq: 1,
            x: 0x1234,
            y: 0x5678,
            z: (-7i16) as u16,
        })
    );

    // A byte-identical resend is a fresh command and must replace/restart
    // any active route, so only its seq changes.
    assert!(apply_packet(&mut w, &frame));
    assert_eq!(w.server_pathfind.unwrap().seq, 2);
}

#[test]
fn corpse_equip_parses_entries_and_terminator() {
    let mut w = World::new();
    // 0x89: corpse 0x4000_0002 wearing a shirt (layer 5 → wire 6, serial
    // 0x4000_0003) and a hat (layer 7 → wire 8, serial 0x4000_0004), terminated
    // by the layer==0 (Layer.Invalid) sentinel.
    let mut p = PacketWriter::new();
    p.u8(0x89).u16(0); // id, len placeholder
    p.u32(0x4000_0002);
    p.u8(6).u32(0x4000_0003);
    p.u8(8).u32(0x4000_0004);
    p.u8(0); // terminator
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;
    assert!(apply_packet(&mut w, &frame));
    let entries = w
        .corpse_equip
        .get(&0x4000_0002)
        .expect("corpse equip stored");
    assert_eq!(entries, &vec![(5, 0x4000_0003), (7, 0x4000_0004)]);
}

#[test]
fn corpse_equip_truncated_frame_keeps_what_parsed() {
    let mut w = World::new();
    // 0x89: corpse 0x55, one full entry, then a dangling layer byte with no
    // serial behind it (truncated mid-stream) — must not panic, and the
    // complete entry before it is kept.
    let mut p = PacketWriter::new();
    p.u8(0x89).u16(0);
    p.u32(0x55);
    p.u8(3).u32(0x4000_0009); // one complete entry (real layer 2)
    p.u8(4); // dangling layer byte, no serial follows
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;
    assert!(apply_packet(&mut w, &frame));
    let entries = w.corpse_equip.get(&0x55).expect("corpse equip stored");
    assert_eq!(entries, &vec![(2, 0x4000_0009)]);
}

#[test]
fn unicode_prompt_sets_pending_state() {
    let mut w = World::new();
    // 0xC2 UnicodePrompt (server→client): serial 0x0102_0304, promptId
    // 0xDEAD_BEEF, plus the type/language/textLen fields ServUO always zeros.
    let mut p = PacketWriter::new();
    p.u8(0xC2).u16(0); // id, len placeholder
    p.u32(0x0102_0304).u32(0xDEAD_BEEF).u32(0).u32(0).u16(0);
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    assert_eq!(len, 21); // ServUO UnicodePrompt EnsureCapacity(21)
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;
    assert!(apply_packet(&mut w, &frame));
    let p = w.prompt.expect("prompt pending");
    assert_eq!((p.sender_serial, p.prompt_id), (0x0102_0304, 0xDEAD_BEEF));
    assert_eq!(p.kind, PromptKind::Unicode);
}

#[test]
fn ascii_prompt_sets_kind_and_replaces_the_pending_prompt() {
    let mut w = World::new();
    w.prompt = Some(PromptState {
        sender_serial: 1,
        prompt_id: 2,
        kind: PromptKind::Unicode,
    });

    let mut p = PacketWriter::new();
    p.u8(0x9A).u16(11).u32(0x0102_0304).u32(0xDEAD_BEEF);
    assert!(apply_packet(&mut w, &p.into_vec()));
    assert_eq!(
        w.prompt,
        Some(PromptState {
            sender_serial: 0x0102_0304,
            prompt_id: 0xDEAD_BEEF,
            kind: PromptKind::Ascii,
        })
    );

    // A malformed resend is recognized but cannot erase/partially replace
    // the complete prompt already being shown.
    assert!(apply_packet(&mut w, &[0x9A, 0, 10, 0, 0, 0, 9]));
    assert_eq!(
        w.prompt.expect("original prompt retained").kind,
        PromptKind::Ascii
    );
}

fn open_url_packet(url: &[u8]) -> Vec<u8> {
    let mut p = PacketWriter::new();
    p.u8(0xA5).u16(0).bytes(url).u8(0);
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1..3].copy_from_slice(&len.to_be_bytes());
    frame
}

#[test]
fn open_url_accepts_only_valid_http_urls_and_preserves_repeat_events() {
    let mut w = World::new();
    for url in [
        "https://uo.com/ultima-store/?item=1#buy",
        "HTTP://ServUO.craftuo.com:8080/",
        "https://[2001:db8::1]/news",
        "https://uo.com/ultima-store/?item=1#buy",
    ] {
        assert!(apply_packet(&mut w, &open_url_packet(url.as_bytes())));
    }
    assert_eq!(w.recent_open_urls.len(), 4);
    assert_eq!(w.recent_open_urls[0].seq, 1);
    assert_eq!(w.recent_open_urls[3].seq, 4);
    assert_eq!(w.recent_open_urls[1].url, "HTTP://ServUO.craftuo.com:8080/");

    let rejected: &[&[u8]] = &[
        b"javascript:alert(1)",
        b"file:///etc/passwd",
        b"//uo.com/path",
        b"https://user:pass@uo.com/",
        b"https:///missing-host",
        b"https://uo.com:99999/",
        b"https://bad..host/",
        b"https://uo.com\\@evil.example/",
        b"https://uo.com/<script>",
        b"https://uo.com/space here",
        b"https://uo.com/\x80",
        b"",
    ];
    for &url in rejected {
        assert!(apply_packet(&mut w, &open_url_packet(url)));
    }
    let mut oversized = b"https://example.com/".to_vec();
    oversized.resize(MAX_OPEN_URL_BYTES + 1, b'a');
    assert!(apply_packet(&mut w, &open_url_packet(&oversized)));
    assert_eq!(w.recent_open_urls.len(), 4, "invalid URLs emit no event");

    assert!(apply_packet(&mut w, &[0xA5, 0, 3]));
    assert_eq!(w.recent_open_urls.len(), 4, "truncated packet is inert");
}

#[test]
fn open_url_event_ring_is_bounded() {
    let mut w = World::new();
    for i in 0..20 {
        assert!(apply_packet(
            &mut w,
            &open_url_packet(format!("https://example.com/{i}").as_bytes())
        ));
    }
    assert_eq!(w.recent_open_urls.len(), 16);
    assert_eq!(w.recent_open_urls.first().map(|e| e.seq), Some(5));
    assert_eq!(w.recent_open_urls.last().map(|e| e.seq), Some(20));
}

fn tip_packet(flag: u8, tip: u32, text: &[u8]) -> Vec<u8> {
    let mut p = PacketWriter::new();
    p.u8(0xA6)
        .u16(0)
        .u8(flag)
        .u32(tip)
        .u16(text.len() as u16)
        .bytes(text);
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1..3].copy_from_slice(&len.to_be_bytes());
    frame
}

#[test]
fn tip_window_parses_pageable_tip_notice_and_cp1252() {
    let mut w = World::new();
    assert!(apply_packet(
        &mut w,
        &tip_packet(0, 0x1234_5678, b"First\rSecond \x80")
    ));
    assert!(apply_packet(&mut w, &tip_packet(2, 9, b"Maintenance")));
    assert!(apply_packet(&mut w, &tip_packet(0, 0x1234_5678, b"Repeat")));

    assert_eq!(w.tips.len(), 3);
    assert_eq!((w.tips[0].seq, w.tips[0].tip), (1, 0x1234_5678));
    assert_eq!(w.tips[0].kind, TipKind::Tip);
    assert_eq!(w.tips[0].text, "First\nSecond €");
    assert_eq!(w.tips[1].kind, TipKind::Notice);
    assert_eq!(w.tips[1].text, "Maintenance");
    assert_eq!(w.tips[2].seq, 3, "repeat packet opens a distinct gump");

    w.close_tip(1);
    assert!(w.tip(1).is_none());
    assert_eq!(w.tips.len(), 2, "close removes exactly one window");
}

#[test]
fn tip_window_ignores_flag_one_and_truncated_text_atomically() {
    let mut w = World::new();
    assert!(apply_packet(&mut w, &[0xA6, 0, 4, 1]));
    assert!(w.tips.is_empty(), "ClassicUO treats flag 1 as a no-op");

    let mut truncated = tip_packet(0, 7, b"abc");
    truncated[8..10].copy_from_slice(&10u16.to_be_bytes());
    assert!(apply_packet(&mut w, &truncated));
    assert!(w.tips.is_empty());
}

#[test]
fn tip_window_ring_is_bounded() {
    let mut w = World::new();
    for tip in 0..20 {
        assert!(apply_packet(&mut w, &tip_packet(2, tip, b"notice")));
    }
    assert_eq!(w.tips.len(), 16);
    assert_eq!(w.tips.first().map(|tip| tip.seq), Some(5));
    assert_eq!(w.tips.last().map(|tip| tip.seq), Some(20));
}

#[allow(clippy::too_many_arguments)]
fn text_entry_dialog_packet(
    serial: u32,
    parent_id: u8,
    button_id: u8,
    text: &[u8],
    can_close: bool,
    variant: u8,
    max_length: u32,
    description: &[u8],
) -> Vec<u8> {
    let mut p = PacketWriter::new();
    p.u8(0xAB)
        .u16(0)
        .u32(serial)
        .u8(parent_id)
        .u8(button_id)
        .u16(text.len() as u16)
        .bytes(text)
        .u8(can_close as u8)
        .u8(variant)
        .u32(max_length)
        .u16(description.len() as u16)
        .bytes(description);
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1..3].copy_from_slice(&len.to_be_bytes());
    frame
}

#[test]
fn text_entry_dialog_preserves_callbacks_constraints_and_cp1252() {
    let mut w = World::new();
    let packet = text_entry_dialog_packet(
        0x0102_0304,
        5,
        6,
        b"Account \x80",
        true,
        2,
        12,
        b"Digits only \x99",
    );
    assert!(apply_packet(&mut w, &packet));
    assert!(apply_packet(&mut w, &packet));

    assert_eq!(w.text_entry_dialogs.len(), 2);
    let dialog = &w.text_entry_dialogs[0];
    assert_eq!(dialog.seq, 1);
    assert_eq!(dialog.serial, 0x0102_0304);
    assert_eq!((dialog.parent_id, dialog.button_id), (5, 6));
    assert_eq!(dialog.text, "Account €");
    assert!(dialog.can_close);
    assert_eq!(dialog.variant, 2);
    assert_eq!(dialog.max_length, 12);
    assert_eq!(dialog.description, "Digits only ™");
    assert_eq!(w.text_entry_dialogs[1].seq, 2);

    w.close_text_entry_dialog(1);
    assert!(w.text_entry_dialog(1).is_none());
    assert_eq!(w.text_entry_dialogs.len(), 1);
}

#[test]
fn text_entry_dialog_is_atomic_and_bounded() {
    let mut w = World::new();
    let mut truncated = text_entry_dialog_packet(1, 2, 3, b"Title", false, 0, 20, b"Description");
    truncated.pop();
    assert!(apply_packet(&mut w, &truncated));
    assert!(w.text_entry_dialogs.is_empty());

    for serial in 0..20 {
        assert!(apply_packet(
            &mut w,
            &text_entry_dialog_packet(serial, 0, 0, b"T", false, 0, 8, b"D")
        ));
    }
    assert_eq!(w.text_entry_dialogs.len(), 16);
    assert_eq!(
        w.text_entry_dialogs.first().map(|dialog| dialog.seq),
        Some(5)
    );
    assert_eq!(
        w.text_entry_dialogs.last().map(|dialog| dialog.seq),
        Some(20)
    );
}

fn character_profile_packet(serial: u32, header: &[u8], footer: &str, body: &str) -> Vec<u8> {
    let mut p = PacketWriter::new();
    p.u8(0xB8).u16(0).u32(serial).bytes(header).u8(0);
    for unit in footer.encode_utf16() {
        p.u16(unit);
    }
    p.u16(0);
    for unit in body.encode_utf16() {
        p.u16(unit);
    }
    p.u16(0);
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1..3].copy_from_slice(&len.to_be_bytes());
    frame
}

#[test]
fn character_profile_decodes_strings_editability_and_replacement() {
    let mut w = World::new();
    w.player = Some(crate::types::Serial(0x0102_0304));
    assert!(apply_packet(
        &mut w,
        &character_profile_packet(
            0x0102_0304,
            b"Anima \x80",
            "Account 😀",
            "Original biography",
        )
    ));
    assert!(apply_packet(
        &mut w,
        &character_profile_packet(9, b"Visitor", "Friend", "Read only")
    ));

    assert_eq!(w.character_profiles.len(), 2);
    let own = &w.character_profiles[0];
    assert_eq!(own.seq, 1);
    assert_eq!(own.header, "Anima €");
    assert_eq!(own.footer, "Account 😀");
    assert_eq!(own.body, "Original biography");
    assert!(own.can_edit);
    assert!(!w.character_profiles[1].can_edit);

    assert!(apply_packet(
        &mut w,
        &character_profile_packet(0x0102_0304, b"Anima", "Updated", "New body")
    ));
    assert_eq!(
        w.character_profiles.len(),
        2,
        "same serial replaces its gump"
    );
    assert_eq!(w.character_profiles[0].serial, 9);
    assert_eq!(w.character_profiles[1].seq, 3);
    assert_eq!(w.character_profiles[1].body, "New body");

    // ServUO hides the real serial for a locked self profile, so ClassicUO
    // treats the serial-zero response as read-only.
    assert!(apply_packet(
        &mut w,
        &character_profile_packet(0, b"Anima", "Locked", "No edits")
    ));
    assert!(!w.character_profiles.last().unwrap().can_edit);
}

#[test]
fn character_profile_is_atomic_bounded_and_exactly_closed() {
    let mut w = World::new();
    assert!(apply_packet(
        &mut w,
        &character_profile_packet(7, b"Valid", "Footer", "Body")
    ));
    let mut truncated = character_profile_packet(7, b"Replacement", "Footer", "Body");
    truncated.pop();
    assert!(apply_packet(&mut w, &truncated));
    assert_eq!(w.character_profiles.len(), 1);
    assert_eq!(w.character_profiles[0].header, "Valid");

    for serial in 100..120 {
        assert!(apply_packet(
            &mut w,
            &character_profile_packet(serial, b"P", "", "")
        ));
    }
    assert_eq!(w.character_profiles.len(), 16);
    assert_eq!(w.character_profiles.first().map(|p| p.serial), Some(104));
    let seq = w.character_profiles[3].seq;
    w.close_character_profile(seq);
    assert!(w.character_profile(seq).is_none());
    assert_eq!(w.character_profiles.len(), 15);
}

#[test]
fn logout_ack_preserves_allow_deny_and_monotonic_identity() {
    let mut w = World::new();
    assert!(apply_packet(&mut w, &[0xD1, 0x00]));
    assert_eq!(
        w.logout_ack,
        Some(crate::world::LogoutAck {
            seq: 1,
            allowed: false,
        })
    );
    assert!(apply_packet(&mut w, &[0xD1, 0x01]));
    assert_eq!(
        w.logout_ack,
        Some(crate::world::LogoutAck {
            seq: 2,
            allowed: true,
        })
    );
}

#[test]
fn boat_moving_commits_boat_and_known_passengers_as_one_event() {
    let boat_serial = 0x4000_1000;
    let player_serial = 0x0000_0042;
    let item_serial = 0x4000_2000;
    let mut w = World::new();
    w.item_mut(boat_serial).pos = Position {
        x: 100,
        y: 200,
        z: -5,
    };
    w.item_mut(boat_serial).is_multi = true;
    w.mobile_mut(player_serial).pos = Position {
        x: 101,
        y: 200,
        z: -4,
    };
    w.item_mut(item_serial).pos = Position {
        x: 99,
        y: 200,
        z: -5,
    };

    let mut p = PacketWriter::new();
    p.u8(0xF6)
        .u16(0)
        .u32(boat_serial)
        .u8(4)
        .u8(2)
        .u8(6)
        .u16(101)
        .u16(200)
        .u16((-5i16) as u16)
        .u16(3)
        .u32(player_serial)
        .u16(102)
        .u16(200)
        .u16((-4i16) as u16)
        .u32(item_serial)
        .u16(100)
        .u16(200)
        .u16((-5i16) as u16)
        .u32(0x0000_9999)
        .u16(103)
        .u16(200)
        .u16(0);
    let frame = patch_len(p.into_vec());
    assert!(apply_packet(&mut w, &frame));

    assert_eq!(
        w.items[&boat_serial].pos,
        Position {
            x: 101,
            y: 200,
            z: -5
        }
    );
    assert_eq!(w.items[&boat_serial].direction, 6);
    assert_eq!(
        w.mobiles[&player_serial].pos,
        Position {
            x: 102,
            y: 200,
            z: -4
        }
    );
    assert_eq!(
        w.items[&item_serial].pos,
        Position {
            x: 100,
            y: 200,
            z: -5
        }
    );
    let movement = &w.recent_boat_movements[0];
    assert_eq!(movement.seq, 1);
    assert_eq!(movement.boat_serial, boat_serial);
    assert_eq!(movement.speed, 4);
    assert_eq!(movement.moving_direction, 2);
    assert_eq!(movement.facing_direction, 6);
    assert_eq!(
        movement.entities.len(),
        2,
        "unknown passenger stays unknown"
    );
}

#[test]
fn truncated_boat_moving_is_atomic() {
    let boat_serial = 0x4000_1000;
    let mut w = World::new();
    w.item_mut(boat_serial).pos = Position { x: 10, y: 20, z: 0 };
    let frame = [
        0xF6, 0, 18, 0x40, 0, 0x10, 0, 4, 2, 2, 0, 11, 0, 20, 0, 0, 0, 1,
    ];
    assert!(apply_packet(&mut w, &frame));
    assert!(apply_packet(&mut w, &[0xF6]));
    assert_eq!(w.items[&boat_serial].pos, Position { x: 10, y: 20, z: 0 });
    assert!(w.recent_boat_movements.is_empty());
}

/// Patch the big-endian length word at `[1..3]` of a variable-framed test packet.
fn patch_len(mut frame: Vec<u8>) -> Vec<u8> {
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;
    frame
}

#[test]
fn secure_trade_display_opens_session() {
    let mut w = World::new();
    assert!(w.trades.is_empty());
    // 0x6F action 0 (Display): opponent 0xBEEF, my container 0x4000_0001,
    // their container 0x4000_0002, hasName=true, name "Bob" (NUL-padded to 30).
    let mut p = PacketWriter::new();
    p.u8(0x6F).u16(0); // id, len placeholder
    p.u8(0x00).u32(0xBEEF).u32(0x4000_0001).u32(0x4000_0002);
    p.u8(1).fixed_ascii("Bob", 30);
    let frame = patch_len(p.into_vec());
    assert_eq!(frame.len(), 47); // 3 header + 1 action + 3×4 serials + 1 bool + 30 name
    assert!(apply_packet(&mut w, &frame));
    assert_eq!(w.trades.len(), 1);
    let t = &w.trades[0];
    assert_eq!(t.opponent_serial, 0xBEEF);
    assert_eq!(t.my_container, 0x4000_0001);
    assert_eq!(t.their_container, 0x4000_0002);
    assert_eq!(t.opponent_name, "Bob");
    assert!(!t.my_accept && !t.their_accept);
}

#[test]
fn secure_trade_display_same_opponent_replaces_not_duplicates() {
    let mut w = World::new();
    w.open_trade(TradeState {
        opponent_serial: 0xBEEF,
        my_container: 0x4000_0001,
        ..Default::default()
    });
    // A second Display for the SAME opponent (ServUO's FindTradeContainer
    // dedupe: only one session per mobile pair) must replace, not append.
    let mut p = PacketWriter::new();
    p.u8(0x6F)
        .u16(0)
        .u8(0x00)
        .u32(0xBEEF)
        .u32(0x4000_0003)
        .u32(0x4000_0004);
    p.u8(1).fixed_ascii("Bob", 30);
    assert!(apply_packet(&mut w, &patch_len(p.into_vec())));
    assert_eq!(w.trades.len(), 1);
    assert_eq!(w.trades[0].my_container, 0x4000_0003);
}

#[test]
fn secure_trade_close_clears_only_matching_session() {
    let mut w = World::new();
    w.open_trade(TradeState {
        opponent_serial: 1,
        my_container: 0x4000_0001,
        ..Default::default()
    });
    w.open_trade(TradeState {
        opponent_serial: 2,
        my_container: 0x4000_0002,
        ..Default::default()
    });
    // 0x6F action 1 (Close): container 0x4000_0001 — only that session drops.
    let mut p = PacketWriter::new();
    p.u8(0x6F).u16(0).u8(0x01).u32(0x4000_0001);
    assert!(apply_packet(&mut w, &patch_len(p.into_vec())));
    assert_eq!(w.trades.len(), 1);
    assert_eq!(w.trades[0].my_container, 0x4000_0002);
}

#[test]
fn secure_trade_close_purges_leftover_container_items() {
    let mut w = World::new();
    w.open_trade(TradeState {
        opponent_serial: 1,
        my_container: 0x4000_0001,
        their_container: 0x4000_0002,
        ..Default::default()
    });
    // Items sitting in either trade container at close time (ServUO sends no
    // removal packet for the opponent's side — see `World::close_trade`'s doc).
    w.item_mut(0x5000_0001).container = Some(0x4000_0001); // mine
    w.item_mut(0x5000_0002).container = Some(0x4000_0002); // theirs
                                                           // An unrelated item elsewhere must survive the purge.
    w.item_mut(0x5000_0003).container = Some(0x9999_0000);
    let mut p = PacketWriter::new();
    p.u8(0x6F).u16(0).u8(0x01).u32(0x4000_0001);
    assert!(apply_packet(&mut w, &patch_len(p.into_vec())));
    assert!(!w.items.contains_key(&0x5000_0001));
    assert!(!w.items.contains_key(&0x5000_0002));
    assert!(w.items.contains_key(&0x5000_0003));
}

#[test]
fn secure_trade_interleaved_two_sessions() {
    let mut w = World::new();
    // Open a session with B, then a second with C — two strangers can each
    // open a trade with us concurrently (no consent required).
    let mut open_b = PacketWriter::new();
    open_b
        .u8(0x6F)
        .u16(0)
        .u8(0x00)
        .u32(0xB0B)
        .u32(0x4000_0001)
        .u32(0x4000_0002);
    open_b.u8(1).fixed_ascii("Bob", 30);
    assert!(apply_packet(&mut w, &patch_len(open_b.into_vec())));

    let mut open_c = PacketWriter::new();
    open_c
        .u8(0x6F)
        .u16(0)
        .u8(0x00)
        .u32(0xC0C)
        .u32(0x4000_0003)
        .u32(0x4000_0004);
    open_c.u8(1).fixed_ascii("Carol", 30);
    assert!(apply_packet(&mut w, &patch_len(open_c.into_vec())));
    assert_eq!(w.trades.len(), 2);

    // C accepts and offers gold — must land on C's session only.
    let mut c_accept = PacketWriter::new();
    c_accept
        .u8(0x6F)
        .u16(0)
        .u8(0x02)
        .u32(0x4000_0003)
        .u32(0)
        .u32(1);
    assert!(apply_packet(&mut w, &patch_len(c_accept.into_vec())));
    let mut c_gold = PacketWriter::new();
    c_gold
        .u8(0x6F)
        .u16(0)
        .u8(0x03)
        .u32(0x4000_0003)
        .u32(777)
        .u32(3);
    assert!(apply_packet(&mut w, &patch_len(c_gold.into_vec())));

    // Close B (container 0x4000_0001) — C must survive untouched.
    let mut close_b = PacketWriter::new();
    close_b.u8(0x6F).u16(0).u8(0x01).u32(0x4000_0001);
    assert!(apply_packet(&mut w, &patch_len(close_b.into_vec())));

    assert_eq!(w.trades.len(), 1);
    let c = &w.trades[0];
    assert_eq!(c.opponent_serial, 0xC0C);
    assert_eq!(c.my_container, 0x4000_0003);
    assert!(c.their_accept);
    assert_eq!((c.their_offer_gold, c.their_offer_platinum), (777, 3));
}

#[test]
fn secure_trade_update_accept_flags() {
    let mut w = World::new();
    w.open_trade(TradeState {
        my_container: 0x4000_0001,
        their_container: 0x4000_0002,
        ..Default::default()
    });
    // 0x6F action 2 (Update): I accepted (1), they haven't (0).
    let mut p = PacketWriter::new();
    p.u8(0x6F).u16(0).u8(0x02).u32(0x4000_0001).u32(1).u32(0);
    assert!(apply_packet(&mut w, &patch_len(p.into_vec())));
    let t = &w.trades[0];
    assert!(t.my_accept);
    assert!(!t.their_accept);

    // Both accept → both flags flip.
    let mut q = PacketWriter::new();
    q.u8(0x6F).u16(0).u8(0x02).u32(0x4000_0001).u32(1).u32(1);
    assert!(apply_packet(&mut w, &patch_len(q.into_vec())));
    let t = &w.trades[0];
    assert!(t.my_accept && t.their_accept);
}

#[test]
fn secure_trade_update_gold_and_ledger() {
    let mut w = World::new();
    w.open_trade(TradeState {
        my_container: 0x4000_0001,
        their_container: 0x4000_0002,
        ..Default::default()
    });
    // 0x6F action 3 (UpdateGold): the OPPONENT offered 500 gold / 2 plat.
    let mut p = PacketWriter::new();
    p.u8(0x6F).u16(0).u8(0x03).u32(0x4000_0001).u32(500).u32(2);
    assert!(apply_packet(&mut w, &patch_len(p.into_vec())));
    let t = &w.trades[0];
    assert_eq!((t.their_offer_gold, t.their_offer_platinum), (500, 2));
    assert_eq!((t.balance_gold, t.balance_platinum), (0, 0)); // untouched

    // 0x6F action 4 (UpdateLedger): OUR account balance is 1000 gold / 5 plat
    // (an input cap, not an offer — see `TradeState`'s doc).
    let mut q = PacketWriter::new();
    q.u8(0x6F).u16(0).u8(0x04).u32(0x4000_0001).u32(1000).u32(5);
    assert!(apply_packet(&mut w, &patch_len(q.into_vec())));
    let t = &w.trades[0];
    assert_eq!((t.balance_gold, t.balance_platinum), (1000, 5));
    assert_eq!((t.their_offer_gold, t.their_offer_platinum), (500, 2)); // untouched
    assert_eq!((t.my_offer_gold, t.my_offer_platinum), (0, 0)); // untouched — we never sent one
}

#[test]
fn secure_trade_unrecognized_action_is_a_noop() {
    let mut w = World::new();
    w.open_trade(TradeState {
        my_container: 0x4000_0001,
        ..Default::default()
    });
    let mut p = PacketWriter::new();
    p.u8(0x6F).u16(0).u8(0xFF); // no such action — must not panic or touch state
    assert!(apply_packet(&mut w, &patch_len(p.into_vec())));
    assert_eq!(w.trades.len(), 1);
}

#[test]
fn general_info_map_change_sets_facet() {
    let mut w = World::new();
    assert_eq!(w.map_index, 0);
    // 0xBF/0x08 MapChange: switch to facet 1 (Trammel).
    let mut p = PacketWriter::new();
    p.u8(0xBF).u16(0).u16(0x0008).u8(1);
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;
    assert_eq!(frame.len(), 6); // ServUO MapChange EnsureCapacity(6)
    assert!(apply_packet(&mut w, &frame));
    assert_eq!(w.map_index, 1);
}

#[test]
fn general_info_map_patches_stores_counts_in_classicuo_order() {
    let mut w = World::new();
    // 0xBF/0x18: count=2 facets, (map=3, statics=7) then (map=1, statics=0).
    let mut p = PacketWriter::new();
    p.u8(0xBF)
        .u16(0)
        .u16(0x0018)
        .u32(2)
        .u32(3)
        .u32(7)
        .u32(1)
        .u32(0);
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;
    assert!(apply_packet(&mut w, &frame));
    assert_eq!(w.map_patch_counts, vec![(3, 7), (1, 0)]);
    assert_eq!(w.map_patches_gen, 1);
}

#[test]
fn map_change_purges_old_facet_but_keeps_player_and_holdings() {
    let mut w = World::new();
    let player = 0x1000_0001;
    w.player = Some(crate::types::Serial(player));
    w.mobile_mut(player).name = "Anima".into();

    // Worn equip: container == the player's own serial directly.
    let backpack = 0x4000_0001;
    w.item_mut(backpack).container = Some(player);
    // Backpack'd item: container == the backpack's serial (nested one level).
    let potion = 0x4000_0002;
    w.item_mut(potion).container = Some(backpack);

    // A stranger mobile and a loose ground item from the OLD facet —
    // neither is the player nor rooted at them, so both must be purged.
    let stranger = 0x1000_0002;
    w.mobile_mut(stranger).name = "Rat".into();
    let ground_item = 0x4000_0003;
    w.item_mut(ground_item);

    // A corpse (and its worn-item layout) from the old facet — purged along
    // with the links that index it, so nothing dangles afterward.
    let corpse = 0x4000_0004;
    w.item_mut(corpse);
    w.set_corpse_of(corpse, stranger);
    w.set_corpse_equip(corpse, vec![(1, 0x4000_0005)]);

    // 0xBF/0x08 MapChange: switch facet 0 → 1 (Trammel).
    let mut p = PacketWriter::new();
    p.u8(0xBF).u16(0).u16(0x0008).u8(1);
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;
    assert!(apply_packet(&mut w, &frame));
    assert_eq!(w.map_index, 1);

    // Survivors: only the player mobile, and only what's rooted at them.
    assert_eq!(w.mobiles.keys().copied().collect::<Vec<_>>(), vec![player]);
    let mut kept: Vec<u32> = w.items.keys().copied().collect();
    kept.sort();
    assert_eq!(kept, vec![backpack, potion]);

    // Purged: the stranger, the ground item, and the now-dangling corpse links.
    assert!(!w.items.contains_key(&ground_item));
    assert!(!w.items.contains_key(&corpse));
    assert!(w.corpse_of.is_empty());
    assert!(w.corpse_equip.is_empty());
}

#[test]
fn map_change_same_facet_is_a_noop() {
    let mut w = World::new();
    let player = 0x1000_0001;
    w.player = Some(crate::types::Serial(player));
    w.mobile_mut(player);
    let stranger = 0x1000_0002;
    w.mobile_mut(stranger);
    let ground_item = 0x4000_0003;
    w.item_mut(ground_item);

    // 0xBF/0x08 MapChange re-affirming the CURRENT facet (0) — must not
    // purge anything (only an actual facet change does).
    let mut p = PacketWriter::new();
    p.u8(0xBF).u16(0).u16(0x0008).u8(0);
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;
    assert!(apply_packet(&mut w, &frame));
    assert_eq!(w.map_index, 0);
    assert!(w.mobiles.contains_key(&stranger));
    assert!(w.items.contains_key(&ground_item));
}

#[test]
fn end_vendor_closes_matching_buy_and_sell_windows() {
    let mut w = World::new();
    w.shop_buy = Some(crate::world::ShopBuy {
        vendor: 0xAABB,
        container: 0x1,
        entries: vec![],
    });
    // 0x3B: EndVendorBuy/EndVendorSell, vendor 0xAABB, trailing unused byte.
    let mut p = PacketWriter::new();
    p.u8(0x3B).u16(0).u32(0xAABB).u8(0);
    let frame = patch_len(p.into_vec());
    assert_eq!(frame.len(), 8); // ServUO EndVendorBuy/EndVendorSell : base(0x3B, 8)
    assert!(apply_packet(&mut w, &frame));
    assert!(w.shop_buy.is_none());

    // A 0x3B for a DIFFERENT vendor must not touch an unrelated open window.
    w.shop_sell = Some(crate::world::ShopSell {
        vendor: 0xCCDD,
        items: vec![],
    });
    let mut q = PacketWriter::new();
    q.u8(0x3B).u16(0).u32(0xAABB).u8(0);
    assert!(apply_packet(&mut w, &patch_len(q.into_vec())));
    assert!(
        w.shop_sell.is_some(),
        "unrelated vendor's sell window must survive"
    );

    // The matching vendor DOES close the sell window too (same opcode
    // closes whichever of buy/sell is actually open for that vendor).
    let mut r = PacketWriter::new();
    r.u8(0x3B).u16(0).u32(0xCCDD).u8(0);
    assert!(apply_packet(&mut w, &patch_len(r.into_vec())));
    assert!(w.shop_sell.is_none());
}

#[test]
fn draw_container_queues_open_event() {
    let mut w = World::new();
    // 0x24 ContainerDisplayHS: serial, gumpId, trailing HS word (ignored).
    let mut p = PacketWriter::new();
    p.u8(0x24).u32(0x4000_0009).u16(0x003C).u16(0x007D);
    let frame = p.into_vec();
    assert_eq!(frame.len(), 9); // ServUO ContainerDisplayHS : base(0x24, 9)
    assert!(apply_packet(&mut w, &frame));
    assert_eq!(
        w.recent_container_opens.last(),
        Some(&(1, 0x4000_0009, 0x003C))
    );
}

#[test]
fn draw_container_records_vendor_buy_and_spellbook_gump_ids_too() {
    // `World` is a pure data log for 0x24 (see `recent_container_opens`'s
    // doc) — it must NOT filter DisplayBuyList's gumpId 0x30 or
    // DisplaySpellbook's 0xFFFF; that's the renderer's (anima-net scene
    // bridge's) job, tested at that layer. Here we just confirm the raw
    // gump_id survives into the ring for whatever consumer wants it.
    let mut w = World::new();
    // DisplayBuyListHS: vendor mobile serial, gumpId 0x30 ("buy window id").
    let mut p = PacketWriter::new();
    p.u8(0x24).u32(0x1000_0055).u16(0x0030).u16(0x0000);
    assert!(apply_packet(&mut w, &p.into_vec()));
    assert_eq!(
        w.recent_container_opens.last(),
        Some(&(1, 0x1000_0055, 0x0030))
    );

    // DisplaySpellbookHS: spellbook item serial, gumpId 0xFFFF (-1).
    let mut q = PacketWriter::new();
    q.u8(0x24).u32(0x4000_0066).u16(0xFFFF).u16(0x007D);
    assert!(apply_packet(&mut w, &q.into_vec()));
    assert_eq!(
        w.recent_container_opens.last(),
        Some(&(2, 0x4000_0066, 0xFFFF))
    );
}

#[test]
fn draw_container_retains_the_art_a_server_opened_window_draws_from() {
    // A container the server opens on its own (a banker's box: `BankBox.Open`
    // → `Container.DisplayTo`, ServUO `Server/Items/Containers.cs`) is the case
    // the capped event ring alone cannot serve. Nothing local asked for that
    // window, so it is built from the 0x24 itself — and it has to keep drawing
    // for as long as it stays open, long after the event has aged out. That is
    // what `container_gumps` is for; assert the SAME packet fills both.
    let mut w = World::new();
    const BANK: u32 = 0x4000_0123;
    // ServUO gives a bank box (item 0xE7C) gump 0x4A — `Data/containers.cfg`.
    let mut p = PacketWriter::new();
    p.u8(0x24).u32(BANK).u16(0x004A).u16(0x007D);
    assert!(apply_packet(&mut w, &p.into_vec()));
    assert_eq!(w.recent_container_opens.last(), Some(&(1, BANK, 0x004A)));
    assert_eq!(w.container_gumps.get(&BANK), Some(&0x004A));

    // Push the open out of the ring — a busy bag-of-bags session does this in
    // seconds — and the window must still know what to draw. 32 is comfortably
    // past `MAX_RECENT_CONTAINER_OPENS`, which is private to `world`.
    for i in 0..32u32 {
        let mut q = PacketWriter::new();
        q.u8(0x24).u32(0x4000_1000 + i).u16(0x003C).u16(0x007D);
        assert!(apply_packet(&mut w, &q.into_vec()));
    }
    assert!(
        !w.recent_container_opens
            .iter()
            .any(|&(_, serial, _)| serial == BANK),
        "the event ring is expected to age out — that is why the id is retained"
    );
    assert_eq!(
        w.container_gumps.get(&BANK),
        Some(&0x004A),
        "the still-open window would go blank without this"
    );
}

#[test]
fn open_paperdoll_parses_title_and_flags() {
    let mut w = World::new();
    // 0x88 DisplayPaperdoll: serial, 60-byte title, flags (warmode + canLift).
    let mut p = PacketWriter::new();
    p.u8(0x88)
        .u32(0xDEAD_BEEF)
        .fixed_ascii("Anima the Adventurer", 60)
        .u8(0x03);
    let frame = p.into_vec();
    assert_eq!(frame.len(), 66); // ServUO DisplayPaperdoll : base(0x88, 66)
    assert!(apply_packet(&mut w, &frame));
    let pd = w.paperdoll.as_ref().expect("paperdoll set");
    assert_eq!(pd.serial, 0xDEAD_BEEF);
    assert_eq!(pd.title, "Anima the Adventurer");
    assert!(pd.warmode);
    assert!(pd.can_lift);
    assert_eq!(pd.seq, 1);

    // A second request for the SAME serial still bumps `seq` (real UO
    // reopens on every double-click, even a repeat one).
    let mut q = PacketWriter::new();
    q.u8(0x88)
        .u32(0xDEAD_BEEF)
        .fixed_ascii("Anima the Adventurer", 60)
        .u8(0x00);
    assert!(apply_packet(&mut w, &q.into_vec()));
    let pd2 = w.paperdoll.as_ref().expect("paperdoll set");
    assert_eq!(pd2.seq, 2);
    assert!(!pd2.warmode);
}

#[test]
fn swing_queues_attacker_and_defender() {
    let mut w = World::new();
    // 0x2F Swing: flag (always 0, unused), attacker, defender.
    let mut p = PacketWriter::new();
    p.u8(0x2F).u8(0).u32(0x1000_0001).u32(0x1000_0002);
    let frame = p.into_vec();
    assert_eq!(frame.len(), 10); // ServUO Swing : base(0x2F, 10)
    assert!(apply_packet(&mut w, &frame));
    assert_eq!(w.recent_swings.last(), Some(&(1, 0x1000_0001, 0x1000_0002)));
}

#[test]
fn general_info_close_gump_by_type_drops_matching_kind() {
    let mut w = World::new();
    w.add_gump(Gump {
        serial: 1,
        gump_id: 0x2A,
        ..Default::default()
    });
    w.add_gump(Gump {
        serial: 2,
        gump_id: 0x2A,
        ..Default::default()
    });
    w.add_gump(Gump {
        serial: 3,
        gump_id: 0x5B,
        ..Default::default()
    });
    // 0xBF/0x04 CloseGump: typeID 0x2A, buttonID 0 (every real ServUO call
    // site sends 0 — see `general_info`'s doc).
    let mut p = PacketWriter::new();
    p.u8(0xBF).u16(0).u16(0x0004).u32(0x2A).u32(0);
    let frame = patch_len(p.into_vec());
    assert_eq!(frame.len(), 13); // ServUO CloseGump EnsureCapacity(13)
    assert!(apply_packet(&mut w, &frame));
    assert_eq!(w.gumps.len(), 1);
    assert_eq!(w.gumps[0].serial, 3);
}

#[test]
fn general_info_display_equipment_info_journals_name_crafter_and_attrs() {
    // ServUO DisplayEquipmentInfo: name cliloc, crafted-by, unidentified,
    // one flag attr (charges -1) and one charged attr, then terminator -1.
    let mut w = World::new();
    w.item_mut(0x4000_0001).name = "katana".into();
    let mut p = PacketWriter::new();
    p.u8(0xBF).u16(0).u16(0x0010).u32(0x4000_0001).u32(1026638);
    p.u32(0xFFFF_FFFD).u16(3).bytes(b"Bob");
    p.u32(0xFFFF_FFFC);
    p.u32(1_061_170).u16((-1i16) as u16); // exceptional (flag)
    p.u32(1_061_179).u16(50); // uses remaining
    p.u32(0xFFFF_FFFF);
    assert!(apply_packet(&mut w, &patch_len(p.into_vec())));
    assert_eq!(w.journal.len(), 4);
    assert_eq!(w.journal[0].cliloc, 1026638);
    assert_eq!(w.journal[0].hue, 0x3B2);
    assert_eq!(w.journal[0].serial, 0x4000_0001);
    assert_eq!(w.journal[1].text, "Crafted by Bob[Unidentified]");
    assert_eq!(w.journal[2].cliloc, 1_061_170);
    assert!(w.journal[2].affix.is_empty());
    assert_eq!(w.journal[3].cliloc, 1_061_179);
    assert_eq!(w.journal[3].affix, " : 50");
}

#[test]
fn general_info_display_equipment_info_drops_unknown_item() {
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0xBF).u16(0).u16(0x0010).u32(0x4000_0099).u32(1026638);
    p.u32(0xFFFF_FFFF);
    assert!(apply_packet(&mut w, &patch_len(p.into_vec())));
    assert!(w.journal.is_empty());
}

#[test]
fn general_info_heritage_opens_and_close_sentinel_clears() {
    let mut w = World::new();
    let mut open = PacketWriter::new();
    open.u8(0xBF).u16(0).u16(0x002A).u8(1).u8(2); // female elf
    assert!(apply_packet(&mut w, &patch_len(open.into_vec())));
    let prompt = w.race_change.expect("dialog open");
    assert!(prompt.female);
    assert_eq!(prompt.race, 2);

    let mut close = PacketWriter::new();
    close.u8(0xBF).u16(0).u16(0x002A).u8(0).u8(0xFF);
    assert!(apply_packet(&mut w, &patch_len(close.into_vec())));
    assert!(w.race_change.is_none());
}

#[test]
fn general_info_forced_anim_matches_low_16_bits() {
    let mut w = World::new();
    w.mobile_mut(0x1000_00AB);
    let mut p = PacketWriter::new();
    p.u8(0xBF).u16(0).u16(0x002B).u16(0x00AB).u8(4).u8(3);
    assert!(apply_packet(&mut w, &patch_len(p.into_vec())));
    assert_eq!(
        w.recent_anims.last(),
        Some(&(1, 0x1000_00AB, 4, 3, true, 0))
    );
}

#[test]
fn general_info_extended_stats_v0_is_existing_only() {
    // 0xBF/0x19 ExtendedStats version 0 (bonded-pet death flag):
    // [version:u8=0][serial:u32][isDead:u8]. Existing-mobile-only — a
    // serial we don't already know must not spawn a phantom mobile.
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0xBF).u16(0).u16(0x0019).u8(0).u32(0xDEAD).u8(1);
    assert!(apply_packet(&mut w, &patch_len(p.into_vec())));
    assert!(
        !w.mobiles.contains_key(&0xDEAD),
        "must not create a phantom mobile"
    );

    // A pre-created mobile does get its is_dead flag set.
    w.mobile_mut(0xBEEF);
    let mut q = PacketWriter::new();
    q.u8(0xBF).u16(0).u16(0x0019).u8(0).u32(0xBEEF).u8(1);
    assert!(apply_packet(&mut w, &patch_len(q.into_vec())));
    assert!(w.mobiles[&0xBEEF].is_dead);
}

#[test]
fn general_info_extended_stats_v2_sets_player_stat_locks() {
    // 0xBF/0x19 ExtendedStats version 2 (stat-training locks), only
    // meaningful for the player: [version:u8=2][serial:u32][updateGump:u8]
    // [state:u8]. state packs str/dex/int locks as 2-bit fields
    // (str<<4 | dex<<2 | int); here Str=Down(1), Dex=Locked(2), Int=Up(0).
    let mut w = World::new();
    w.player = Some(crate::types::Serial(0x1001));
    let state: u8 = (1 << 4) | (2 << 2); // Int=Up(0) contributes nothing
    let mut p = PacketWriter::new();
    p.u8(0xBF)
        .u16(0)
        .u16(0x0019)
        .u8(2)
        .u32(0x1001)
        .u8(0)
        .u8(state);
    assert!(apply_packet(&mut w, &patch_len(p.into_vec())));
    assert_eq!(w.player_stats.str_lock, 1);
    assert_eq!(w.player_stats.dex_lock, 2);
    assert_eq!(w.player_stats.int_lock, 0);
}

#[test]
fn general_info_extended_stats_v2_ignores_non_player_serial() {
    let mut w = World::new();
    w.player = Some(crate::types::Serial(0x1001));
    let state: u8 = (1 << 4) | (2 << 2) | 3;
    let mut p = PacketWriter::new();
    p.u8(0xBF)
        .u16(0)
        .u16(0x0019)
        .u8(2)
        .u32(0x9999) // not the player
        .u8(0)
        .u8(state);
    assert!(apply_packet(&mut w, &patch_len(p.into_vec())));
    assert_eq!(w.player_stats.str_lock, 0, "must stay default");
    assert_eq!(w.player_stats.dex_lock, 0, "must stay default");
    assert_eq!(w.player_stats.int_lock, 0, "must stay default");
}

#[test]
fn general_info_speed_mode_sets_and_clamps() {
    // 0xBF/0x26 SpeedMode: [val:u8]. Values > 3 must clamp back to 0.
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0xBF).u16(0).u16(0x0026).u8(2); // CantRun
    assert!(apply_packet(&mut w, &patch_len(p.into_vec())));
    assert_eq!(w.player_stats.speed_mode, 2);

    let mut q = PacketWriter::new();
    q.u8(0xBF).u16(0).u16(0x0026).u8(0xFF); // out of range
    assert!(apply_packet(&mut w, &patch_len(q.into_vec())));
    assert_eq!(w.player_stats.speed_mode, 0);
}

#[test]
fn char_status_flag5_reads_ml_ren_aos_tail_in_wire_order() {
    // 0x11 CharacterStatus, flag=5 (ML): ClassicUO reads the ML tail,
    // then unconditionally checks (and reads) Renaissance, then AOS —
    // all three gated on the same flag/`type` byte, in that wire order.
    let mut w = World::new();
    w.player = Some(crate::types::Serial(0x1001));

    let mut p = PacketWriter::new();
    p.u8(0x11).u16(0); // id + length placeholder
    p.u32(0x1001) // serial
        .fixed_ascii("Anima", 30)
        .u16(90) // hits
        .u16(100) // hits_max
        .u8(0) // name_change_flag
        .u8(5); // flag
    p.u8(0) // is_female
        .u16(60) // strength
        .u16(70) // dexterity
        .u16(80) // intelligence
        .u16(50) // stam
        .u16(55) // stam_max
        .u16(40) // mana
        .u16(45) // mana_max
        .u32(12_345) // gold
        .u16(25u16) // armor (i16 25)
        .u16(400); // weight

    // ML tail (flag >= 5): weight_max, race.
    p.u16(500).u8(2);
    // Renaissance tail (flag >= 3): stats_cap, followers, followers_max.
    p.u16(225u16).u8(3).u8(5);
    // AOS tail (flag >= 4): resistances, luck, damage range, tithing.
    p.u16(10u16) // fire_resistance
        .u16(20u16) // cold_resistance
        .u16(30u16) // poison_resistance
        .u16(40u16) // energy_resistance
        .u16(77) // luck
        .u16(5u16) // damage_min
        .u16(12u16) // damage_max
        .u32(999); // tithing_points

    assert!(apply_packet(&mut w, &patch_len(p.into_vec())));
    let s = &w.player_stats;
    assert_eq!(s.weight_max, 500);
    assert_eq!(s.race, 2);
    assert_eq!(s.stats_cap, 225);
    assert_eq!(s.followers, 3);
    assert_eq!(s.followers_max, 5);
    assert_eq!(s.fire_resistance, 10);
    assert_eq!(s.cold_resistance, 20);
    assert_eq!(s.poison_resistance, 30);
    assert_eq!(s.energy_resistance, 40);
    assert_eq!(s.luck, 77);
    assert_eq!(s.damage_min, 5);
    assert_eq!(s.damage_max, 12);
    assert_eq!(s.tithing_points, 999);
}

/// Build a 0x11 CharacterStatus with `flag`, everything up to the AOS block
/// filled in, and `tail` appended raw. Shared by the `type >= 6` tests.
fn char_status_flag6(tail: &[u8]) -> Vec<u8> {
    let mut p = PacketWriter::new();
    p.u8(0x11).u16(0);
    p.u32(0x1003)
        .fixed_ascii("Anima", 30)
        .u16(90)
        .u16(100)
        .u8(0)
        .u8(6); // flag/type
    p.u8(0)
        .u16(60)
        .u16(70)
        .u16(80)
        .u16(50)
        .u16(55)
        .u16(40)
        .u16(45)
        .u32(12_345)
        .u16(25u16)
        .u16(400);
    p.u16(500).u8(1); // ML: weight_max, race
    p.u16(225u16).u8(3).u8(5); // Renaissance
    p.u16(10u16) // AOS: the four *current* resists, luck, damage, tithing
        .u16(20u16)
        .u16(30u16)
        .u16(40u16)
        .u16(77)
        .u16(5u16)
        .u16(12u16)
        .u32(999);
    let mut bytes = p.into_vec();
    bytes.extend_from_slice(tail);
    patch_len(bytes)
}

#[test]
fn char_status_flag6_reads_the_aos_combat_tail_in_servuo_order() {
    // The 15 shorts ServUO writes as `(short)AOS.GetStatus(i)` for i in 0..=14
    // (`Packets.cs` MobileStatus), which is the order ClassicUO reads them back
    // in. Distinct values per slot so a transposition cannot pass.
    let mut w = World::new();
    w.player = Some(crate::types::Serial(0x1003));

    let mut t = PacketWriter::new();
    t.u16(70u16) // 0 max physical resist
        .u16(71u16) // 1 max fire
        .u16(72u16) // 2 max cold
        .u16(73u16) // 3 max poison
        .u16(74u16) // 4 max energy
        .u16(35u16) // 5 defense chance increase
        .u16(45u16) // 6 max defense chance increase
        .u16(15u16) // 7 hit chance increase
        .u16(30u16) // 8 swing speed increase
        .u16(50u16) // 9 damage increase
        .u16(20u16) // 10 lower reagent cost
        .u16(12u16) // 11 spell damage increase
        .u16(4u16) // 12 faster cast recovery
        .u16(2u16) // 13 faster casting
        .u16(8u16); // 14 lower mana cost

    assert!(apply_packet(&mut w, &char_status_flag6(&t.into_vec())));
    let a = w.player_stats.aos;
    assert_eq!(a.max_physical_resistance, 70);
    assert_eq!(a.max_fire_resistance, 71);
    assert_eq!(a.max_cold_resistance, 72);
    assert_eq!(a.max_poison_resistance, 73);
    assert_eq!(a.max_energy_resistance, 74);
    assert_eq!(a.defense_chance_increase, 35);
    assert_eq!(a.max_defense_chance_increase, 45);
    assert_eq!(a.hit_chance_increase, 15);
    assert_eq!(a.swing_speed_increase, 30);
    assert_eq!(a.damage_increase, 50);
    assert_eq!(a.lower_reagent_cost, 20);
    assert_eq!(a.spell_damage_increase, 12);
    assert_eq!(a.faster_cast_recovery, 4);
    assert_eq!(a.faster_casting, 2);
    assert_eq!(a.lower_mana_cost, 8);
    // The blocks before it must still land — the tail is additive, not a
    // replacement for the AOS resist block.
    assert_eq!(w.player_stats.fire_resistance, 10);
    assert_eq!(w.player_stats.tithing_points, 999);
}

#[test]
fn char_status_flag6_survives_a_short_tail() {
    // ClassicUO guards every read (`p.Position + 2 > p.Length ? 0 : …`), so a
    // truncated tail costs those fields and nothing else. Ours must not fail
    // the packet and lose the name/HP with it — an Enhanced-Client session is
    // sent 29 of these values, and a shard mixing the two is exactly how a
    // short tail arrives.
    let mut w = World::new();
    w.player = Some(crate::types::Serial(0x1003));

    let mut t = PacketWriter::new();
    t.u16(70u16).u16(71u16).u16(72u16); // three of fifteen, then nothing

    assert!(apply_packet(&mut w, &char_status_flag6(&t.into_vec())));
    let a = w.player_stats.aos;
    assert_eq!(a.max_physical_resistance, 70);
    assert_eq!(a.max_cold_resistance, 72);
    assert_eq!(a.max_poison_resistance, 0, "missing values read as zero");
    assert_eq!(a.lower_mana_cost, 0);
    // And the rest of the packet still applied.
    assert_eq!(w.player_stats.stats_cap, 225);
    assert_eq!(w.mobiles[&0x1003].hits, 90);
}

#[test]
fn char_status_flag5_leaves_the_aos_combat_tail_zeroed() {
    // A pre-ML/pre-extended shard sends `type < 6` — this one sends 5 — and the
    // combat tail simply is not on the wire. Nothing may invent values for it.
    let mut w = World::new();
    w.player = Some(crate::types::Serial(0x1004));
    let mut p = PacketWriter::new();
    p.u8(0x11).u16(0);
    p.u32(0x1004)
        .fixed_ascii("Anima", 30)
        .u16(90)
        .u16(100)
        .u8(0)
        .u8(5);
    p.u8(0)
        .u16(60)
        .u16(70)
        .u16(80)
        .u16(50)
        .u16(55)
        .u16(40)
        .u16(45)
        .u32(1)
        .u16(0u16)
        .u16(0);
    p.u16(500).u8(1);
    p.u16(225u16).u8(0).u8(0);
    p.u16(0u16)
        .u16(0u16)
        .u16(0u16)
        .u16(0u16)
        .u16(0)
        .u16(0u16)
        .u16(0u16)
        .u32(0);
    assert!(apply_packet(&mut w, &patch_len(p.into_vec())));
    assert_eq!(w.player_stats.aos, crate::world::AosStatus::default());
}

#[test]
fn char_status_flag3_reads_only_renaissance_tail() {
    // flag=3: only the Renaissance block is present on the wire — the ML
    // and AOS blocks must not be read (there's nothing left to read).
    let mut w = World::new();
    w.player = Some(crate::types::Serial(0x1002));

    let mut p = PacketWriter::new();
    p.u8(0x11).u16(0);
    p.u32(0x1002)
        .fixed_ascii("Anima", 30)
        .u16(90)
        .u16(100)
        .u8(0)
        .u8(3); // flag
    p.u8(0)
        .u16(60)
        .u16(70)
        .u16(80)
        .u16(50)
        .u16(55)
        .u16(40)
        .u16(45)
        .u32(12_345)
        .u16(25u16)
        .u16(400);
    // Renaissance tail only.
    p.u16(200u16).u8(1).u8(4);

    assert!(apply_packet(&mut w, &patch_len(p.into_vec())));
    let s = &w.player_stats;
    assert_eq!(s.stats_cap, 200);
    assert_eq!(s.followers, 1);
    assert_eq!(s.followers_max, 4);
    // flag < 5 omits the weight cap on the wire; it is derived from strength
    // (7*(60>>1)+40 = 250), matching ClassicUO's non-ML fallback.
    assert_eq!(s.weight_max, 250);
    // No ML/AOS bytes were on the wire — these stay at defaults.
    assert_eq!(s.race, 0);
    assert_eq!(s.fire_resistance, 0);
    assert_eq!(s.tithing_points, 0);
}

#[test]
fn display_map_legacy_0x90_parses_bounds_no_facet() {
    let mut w = World::new();
    // 0x90 MapDetails: serial, gumpArt (always 0x139D), bounds, size. No facet.
    let mut p = PacketWriter::new();
    p.u8(0x90).u32(0x4000_7777).u16(0x139D);
    p.u16(0).u16(0).u16(400).u16(400); // minX, minY, maxX, maxY
    p.u16(200).u16(200); // width, height
    let frame = p.into_vec();
    assert_eq!(frame.len(), 19); // ServUO MapDetails : base(0x90, 19)
    assert!(apply_packet(&mut w, &frame));

    let mv = w.map_gumps.get(&0x4000_7777).expect("map view set");
    assert_eq!(mv.open_seq, 1);
    assert_eq!(mv.gump_art, 0x139D);
    assert_eq!(
        mv.facet, 0,
        "legacy 0x90 carries no facet — defaults to Felucca"
    );
    assert_eq!((mv.min_x, mv.min_y, mv.max_x, mv.max_y), (0, 0, 400, 400));
    assert_eq!((mv.width, mv.height), (200, 200));
    assert!(mv.pins.is_empty());
}

#[test]
fn display_map_new_0xf5_parses_trailing_facet() {
    let mut w = World::new();
    // 0xF5 NewMapDetails: identical body to 0x90, PLUS a trailing facet u16
    // (verified against ServUO's `NewMapDetails` ctor — appended at the very
    // end, not interleaved before width/height). facet=3 (Malas).
    let mut p = PacketWriter::new();
    p.u8(0xF5).u32(0x4000_8888).u16(0x139D);
    p.u16(520).u16(0).u16(2580).u16(2050);
    p.u16(400).u16(400);
    p.u16(3); // facet: Malas
    let frame = p.into_vec();
    assert_eq!(frame.len(), 21); // ServUO NewMapDetails : base(0xF5, 21)
    assert!(apply_packet(&mut w, &frame));

    let mv = w.map_gumps.get(&0x4000_8888).expect("map view set");
    assert_eq!(mv.facet, 3);
    assert_eq!(
        (mv.min_x, mv.min_y, mv.max_x, mv.max_y),
        (520, 0, 2580, 2050)
    );
    assert_eq!((mv.width, mv.height), (400, 400));
}

#[test]
fn display_map_resend_bumps_open_seq_and_resets_pins() {
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0x90)
        .u32(0x4000_9999)
        .u16(0x139D)
        .u16(0)
        .u16(0)
        .u16(400)
        .u16(400)
        .u16(200)
        .u16(200);
    assert!(apply_packet(&mut w, &p.into_vec()));
    assert_eq!(w.map_gumps[&0x4000_9999].open_seq, 1);

    // Simulate the pin arriving via 0x56, then a re-decode/re-click resending
    // 0x90 — real ServUO `MapItem.DisplayTo` always resends the bounds packet
    // first (about to be followed by a fresh Clear+re-Add over 0x56).
    let mut add = PacketWriter::new();
    add.u8(0x56).u32(0x4000_9999).u8(1).u8(0).u16(50).u16(60);
    assert!(apply_packet(&mut w, &add.into_vec()));
    assert_eq!(w.map_gumps[&0x4000_9999].pins.len(), 1);

    let mut q = PacketWriter::new();
    q.u8(0x90)
        .u32(0x4000_9999)
        .u16(0x139D)
        .u16(0)
        .u16(0)
        .u16(400)
        .u16(400)
        .u16(200)
        .u16(200);
    assert!(apply_packet(&mut w, &q.into_vec()));
    let mv = &w.map_gumps[&0x4000_9999];
    assert_eq!(
        mv.open_seq, 2,
        "a resend must bump open_seq even with identical bounds"
    );
    assert!(
        mv.pins.is_empty(),
        "a resend resets pins — the real wire flow re-adds them over 0x56"
    );
}

#[test]
fn map_command_add_pin() {
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0x90)
        .u32(0x4000_AAAA)
        .u16(0x139D)
        .u16(0)
        .u16(0)
        .u16(400)
        .u16(400)
        .u16(200)
        .u16(200);
    assert!(apply_packet(&mut w, &p.into_vec()));

    // 0x56 MapCommand command=1 (Add): the chest pin lands at index 0.
    let mut add = PacketWriter::new();
    add.u8(0x56).u32(0x4000_AAAA).u8(1).u8(0).u16(100).u16(120);
    let frame = add.into_vec();
    assert_eq!(frame.len(), 11); // ServUO MapCommand : base(0x56, 11)
    assert!(apply_packet(&mut w, &frame));
    assert_eq!(w.map_gumps[&0x4000_AAAA].pins, vec![(100, 120)]);
}

#[test]
fn map_command_clear_drops_every_pin() {
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0x90)
        .u32(0x4000_BBBB)
        .u16(0x139D)
        .u16(0)
        .u16(0)
        .u16(400)
        .u16(400)
        .u16(200)
        .u16(200);
    assert!(apply_packet(&mut w, &p.into_vec()));
    for (x, y) in [(10u16, 10u16), (20, 20), (30, 30)] {
        let mut add = PacketWriter::new();
        add.u8(0x56).u32(0x4000_BBBB).u8(1).u8(0).u16(x).u16(y);
        apply_packet(&mut w, &add.into_vec());
    }
    assert_eq!(w.map_gumps[&0x4000_BBBB].pins.len(), 3);

    // command=5 (Clear).
    let mut clear = PacketWriter::new();
    clear.u8(0x56).u32(0x4000_BBBB).u8(5).u8(0).u16(0).u16(0);
    assert!(apply_packet(&mut w, &clear.into_vec()));
    assert!(w.map_gumps[&0x4000_BBBB].pins.is_empty());
}

#[test]
fn map_command_remove_refuses_index_zero() {
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0x90)
        .u32(0x4000_CCCC)
        .u16(0x139D)
        .u16(0)
        .u16(0)
        .u16(400)
        .u16(400)
        .u16(200)
        .u16(200);
    assert!(apply_packet(&mut w, &p.into_vec()));
    for (x, y) in [(10u16, 10u16), (20, 20)] {
        let mut add = PacketWriter::new();
        add.u8(0x56).u32(0x4000_CCCC).u8(1).u8(0).u16(x).u16(y);
        apply_packet(&mut w, &add.into_vec());
    }

    // command=4 (Remove) index 0 — the treasure/chest pin — is refused
    // (ServUO `MapItem.RemovePin`'s `index > 0` guard).
    let mut rm0 = PacketWriter::new();
    rm0.u8(0x56).u32(0x4000_CCCC).u8(4).u8(0).u16(0).u16(0);
    assert!(apply_packet(&mut w, &rm0.into_vec()));
    assert_eq!(
        w.map_gumps[&0x4000_CCCC].pins.len(),
        2,
        "index 0 must survive"
    );

    // command=4 index 1 succeeds.
    let mut rm1 = PacketWriter::new();
    rm1.u8(0x56).u32(0x4000_CCCC).u8(4).u8(1).u16(0).u16(0);
    assert!(apply_packet(&mut w, &rm1.into_vec()));
    assert_eq!(w.map_gumps[&0x4000_CCCC].pins, vec![(10, 10)]);
}

#[test]
fn map_command_unknown_serial_is_ignored() {
    let mut w = World::new();
    // No 0x90/0xF5 was ever sent for this serial — must not panic or create one.
    let mut p = PacketWriter::new();
    p.u8(0x56).u32(0xDEAD_0000).u8(1).u8(0).u16(1).u16(1);
    assert!(apply_packet(&mut w, &p.into_vec()));
    assert!(w.map_gumps.is_empty());
}

#[allow(clippy::too_many_arguments)]
fn waypoint_frame(
    serial: u32,
    x: u16,
    y: u16,
    z: i8,
    map: u8,
    kind: u16,
    ignore_object: bool,
    cliloc: u32,
    name: &str,
) -> Vec<u8> {
    let mut p = PacketWriter::new();
    p.u8(0xE5)
        .u16(0)
        .u32(serial)
        .u16(x)
        .u16(y)
        .u8(z as u8)
        .u8(map)
        .u16(kind)
        .u16(u16::from(ignore_object))
        .u32(cliloc);
    let mut frame = p.into_vec();
    for unit in name.encode_utf16() {
        frame.extend_from_slice(&unit.to_le_bytes());
    }
    frame.extend_from_slice(&[0, 0]); // LittleUniNull terminator
    frame.extend_from_slice(&[0, 0]); // ServUO trailing short
    let len = frame.len() as u16;
    frame[1..3].copy_from_slice(&len.to_be_bytes());
    frame
}

#[test]
fn display_waypoint_parses_servuo_shape_and_utf16le_name() {
    let mut w = World::new();
    let frame = waypoint_frame(
        0x1234_5678,
        2551,
        420,
        -17,
        1,
        6,
        true,
        1_062_613,
        "Éowyn the healer",
    );

    assert!(apply_packet(&mut w, &frame));
    let waypoint = &w.waypoints[&0x1234_5678];
    assert_eq!(
        waypoint.pos,
        crate::types::Position {
            x: 2551,
            y: 420,
            z: -17
        }
    );
    assert_eq!(waypoint.map, 1);
    assert_eq!(waypoint.kind, 6);
    assert!(waypoint.ignore_object);
    assert_eq!(waypoint.cliloc, 1_062_613);
    assert_eq!(waypoint.name, "Éowyn the healer");
}

#[test]
fn waypoint_same_serial_upserts_and_e6_removes_exactly_one() {
    let mut w = World::new();
    assert!(apply_packet(
        &mut w,
        &waypoint_frame(10, 100, 100, 0, 0, 1, false, 1_046_414, "corpse")
    ));
    assert!(apply_packet(
        &mut w,
        &waypoint_frame(20, 200, 200, 5, 0, 6, false, 1_062_613, "healer")
    ));
    assert!(apply_packet(
        &mut w,
        &waypoint_frame(20, 201, 202, 6, 0, 6, false, 1_062_613, "moved")
    ));
    assert_eq!(w.waypoints.len(), 2);
    assert_eq!(w.waypoints[&20].pos.x, 201);
    assert_eq!(w.waypoints[&20].name, "moved");

    let mut remove = PacketWriter::new();
    remove.u8(0xE6).u32(20);
    assert!(apply_packet(&mut w, &remove.into_vec()));
    assert!(w.waypoints.contains_key(&10));
    assert!(!w.waypoints.contains_key(&20));

    let mut unknown = PacketWriter::new();
    unknown.u8(0xE6).u32(999);
    assert!(apply_packet(&mut w, &unknown.into_vec()));
    assert_eq!(w.waypoints.len(), 1);
}

#[test]
fn truncated_waypoint_is_ignored_without_partial_state() {
    let mut w = World::new();
    assert!(apply_packet(&mut w, &[0xE5, 0, 6, 0, 1, 2]));
    assert!(w.waypoints.is_empty());

    let valid = waypoint_frame(10, 100, 100, 0, 0, 6, false, 1_062_613, "healer");
    let mut malformed = Vec::new();

    let mut missing_trailing = valid[..valid.len() - 2].to_vec();
    let len = missing_trailing.len() as u16;
    missing_trailing[1..3].copy_from_slice(&len.to_be_bytes());
    malformed.push(missing_trailing);

    let mut odd_tail = valid[..valid.len() - 1].to_vec();
    let len = odd_tail.len() as u16;
    odd_tail[1..3].copy_from_slice(&len.to_be_bytes());
    malformed.push(odd_tail);

    let mut nonzero_trailing = valid;
    *nonzero_trailing.last_mut().unwrap() = 1;
    malformed.push(nonzero_trailing);

    for frame in malformed {
        assert!(apply_packet(&mut w, &frame));
        assert!(w.waypoints.is_empty());
    }
}

#[test]
fn waypoint_name_decodes_non_bmp_utf16() {
    let mut w = World::new();
    assert!(apply_packet(
        &mut w,
        &waypoint_frame(10, 100, 100, 0, 0, 6, false, 1_062_613, "Healer 🐉")
    ));
    assert_eq!(w.waypoints[&10].name, "Healer 🐉");
}

#[test]
fn move_player_records_direction_and_running_and_bumps_seq() {
    // 0x97 MovePlayer: [id][direction:u8] (2 bytes). ClassicUO forces
    // Player.Walk(dir & 7, running = dir & 0x80).
    let mut w = World::new();
    assert!(w.forced_walk.is_none());

    let mut p = PacketWriter::new();
    p.u8(0x97).u8(0x03); // dir 3 (south), not running
    assert!(apply_packet(&mut w, &p.into_vec()));
    assert_eq!(
        w.forced_walk,
        Some(crate::world::ForcedWalkRequest {
            dir: 3,
            run: false,
            seq: 1,
        })
    );

    let mut p2 = PacketWriter::new();
    p2.u8(0x97).u8(0x87); // dir 7 | running bit 0x80
    assert!(apply_packet(&mut w, &p2.into_vec()));
    assert_eq!(
        w.forced_walk,
        Some(crate::world::ForcedWalkRequest {
            dir: 7,
            run: true,
            seq: 2,
        })
    );
}

#[test]
fn update_character_is_existing_only_and_respects_self_guard() {
    // 0xD2 UpdateCharacter: [id][serial:u32][graphic:u16][x:u16][y:u16]
    // [z:i8][direction:u8][hue:u16][flags:u8][notoriety:u8].
    let build = |serial: u32,
                 graphic: u16,
                 x: u16,
                 y: u16,
                 z: i8,
                 dir: u8,
                 hue: u16,
                 flags: u8,
                 noto: u8| {
        let mut p = PacketWriter::new();
        p.u8(0xD2)
            .u32(serial)
            .u16(graphic)
            .u16(x)
            .u16(y)
            .u8(z as u8)
            .u8(dir)
            .u16(hue)
            .u8(flags)
            .u8(noto);
        p.into_vec()
    };

    let mut w = World::new();

    // Unknown serial: no phantom mobile is created.
    assert!(apply_packet(
        &mut w,
        &build(0x9999, 0x0190, 100, 200, 5, 3, 0, 0, 1)
    ));
    assert!(!w.mobiles.contains_key(&0x9999));

    // Pre-created non-self mobile: full update (pos/body/hue/notoriety).
    w.mobile_mut(0x1234);
    assert!(apply_packet(
        &mut w,
        &build(0x1234, 0x0190, 100, 200, 5, 0x83, 0x0022, FLAG_HIDDEN, 6)
    ));
    let m = &w.mobiles[&0x1234];
    assert_eq!(m.body, 0x0190);
    assert_eq!(m.pos.x, 100);
    assert_eq!(m.pos.y, 200);
    assert_eq!(m.pos.z, 5);
    assert_eq!(m.direction, 0x03); // low 3 bits only
    assert_eq!(m.hue, 0x0022);
    assert_eq!(m.notoriety, 6);
    assert!(m.hidden);

    // Self mobile: visual/flags only, position untouched.
    w.player = Some(crate::types::Serial(0x311));
    w.mobile_mut(0x311).pos = crate::types::Position { x: 50, y: 60, z: 7 };
    assert!(apply_packet(
        &mut w,
        &build(0x311, 0x0191, 999, 999, 9, 2, 0x0033, 0, 3)
    ));
    let me = &w.mobiles[&0x311];
    assert_eq!(me.body, 0x0191);
    assert_eq!(me.hue, 0x0033);
    assert_eq!(me.notoriety, 3);
    assert_eq!(
        me.pos,
        crate::types::Position { x: 50, y: 60, z: 7 },
        "self position must not move"
    );
}

#[test]
fn update_object_creates_mobile_and_parses_worn_item_past_padding() {
    // 0xD3 UpdateObject: same leading shape as 0x78 MobileIncoming plus 6
    // padding bytes before the worn-item list — assert those 6 bytes are
    // correctly skipped (not mistaken for part of the item list) by
    // checking the worn item parses with the right serial/graphic/layer/hue.
    let mut w = World::new();

    let mut p = PacketWriter::new();
    p.u8(0xD3).u16(0); // id + length placeholder
    p.u32(0xBEEF) // serial (new — not yet known)
        .u16(0x0190) // body
        .u16(100) // x
        .u16(200) // y
        .u8(5i8 as u8) // z
        .u8(0x03) // dir
        .u16(0x0044) // hue
        .u8(0) // flags
        .u8(2) // notoriety
        .zeros(6); // 0xD3-only padding, absent on 0x78
    p.u32(0xCAFE).u16(0x1234).u8(0x01).u16(0x0055); // worn item
    p.u32(0); // terminator
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;

    assert!(apply_packet(&mut w, &frame));

    let m = &w.mobiles[&0xBEEF];
    assert_eq!(m.body, 0x0190);
    assert_eq!(m.pos.x, 100);
    assert_eq!(m.pos.y, 200);
    assert_eq!(m.pos.z, 5);
    assert_eq!(m.direction, 0x03);
    assert_eq!(m.hue, 0x0044);
    assert_eq!(m.notoriety, 2);

    let it = &w.items[&0xCAFE];
    assert_eq!(it.graphic, 0x1234);
    assert_eq!(it.layer, 0x01);
    assert_eq!(it.hue, 0x0055);
    assert_eq!(it.container, Some(0xBEEF));
}

#[test]
fn update_object_self_guard_leaves_position_untouched() {
    // Like mobile_incoming, 0xD3 must never move the Walker-owned self
    // position/facing, but should still refresh visual attributes.
    let mut w = World::new();
    w.player = Some(crate::types::Serial(0x311));
    w.mobile_mut(0x311).pos = crate::types::Position { x: 50, y: 60, z: 7 };

    let mut p = PacketWriter::new();
    p.u8(0xD3).u16(0);
    p.u32(0x311)
        .u16(0x0192)
        .u16(999)
        .u16(999)
        .u8(9i8 as u8)
        .u8(2)
        .u16(0x0033)
        .u8(0)
        .u8(3)
        .zeros(6);
    p.u32(0); // no worn items
    let mut frame = p.into_vec();
    let len = frame.len() as u16;
    frame[1] = (len >> 8) as u8;
    frame[2] = (len & 0xFF) as u8;

    assert!(apply_packet(&mut w, &frame));

    let me = &w.mobiles[&0x311];
    assert_eq!(me.body, 0x0192);
    assert_eq!(me.hue, 0x0033);
    assert_eq!(me.notoriety, 3);
    assert_eq!(
        me.pos,
        crate::types::Position { x: 50, y: 60, z: 7 },
        "self position must not move"
    );
}

#[allow(clippy::too_many_arguments)] // mirrors 0x23's flat wire layout
fn drag_animation_frame(
    graphic: u16,
    inc: u8,
    hue: u16,
    count: u16,
    source: u32,
    source_x: u16,
    source_y: u16,
    source_z: i8,
    dest: u32,
    dest_x: u16,
    dest_y: u16,
    dest_z: i8,
) -> Vec<u8> {
    let mut p = PacketWriter::new();
    p.u8(0x23)
        .u16(graphic)
        .u8(inc)
        .u16(hue)
        .u16(count)
        .u32(source)
        .u16(source_x)
        .u16(source_y)
        .u8(source_z as u8)
        .u32(dest)
        .u16(dest_x)
        .u16(dest_y)
        .u8(dest_z as u8);
    p.into_vec()
}

#[test]
fn drag_animation_remaps_gold_and_substitutes_known_source() {
    let mut w = World::new();
    w.mobile_mut(0xAAAA).pos = crate::types::Position { x: 10, y: 20, z: 3 };

    // Gold's "in flight" graphic remap; dest is a serial we've never seen,
    // so its wire-supplied coordinates should pass through unchanged.
    assert!(apply_packet(
        &mut w,
        &drag_animation_frame(0x0EED, 0, 5, 1, 0xAAAA, 999, 999, 9, 0xDEAD, 111, 222, 4)
    ));

    let anim = w.recent_drag_anims.last().expect("event pushed");
    assert_eq!(anim.seq, 1);
    assert_eq!(anim.graphic, 0x0EEF, "gold remaps to its flight graphic");
    assert_eq!(anim.hue, 5);
    assert_eq!(anim.count, 1);
    assert_eq!(anim.source, 0xAAAA, "known source serial kept");
    assert_eq!(anim.source_x, 10, "position substituted from live mobile");
    assert_eq!(anim.source_y, 20);
    assert_eq!(anim.source_z, 3);
    assert_eq!(anim.dest, 0, "unknown dest serial is zeroed");
    assert_eq!(anim.dest_x, 111, "unknown dest keeps wire coordinates");
    assert_eq!(anim.dest_y, 222);
    assert_eq!(anim.dest_z, 4);
}

#[test]
fn drag_animation_zeroes_unknown_source_and_bumps_seq() {
    let mut w = World::new();

    assert!(apply_packet(
        &mut w,
        &drag_animation_frame(0x0EEA, 0, 0, 1, 0x1111, 5, 6, 1, 0x2222, 7, 8, 2)
    ));
    assert_eq!(w.recent_drag_anims.last().unwrap().seq, 1);
    assert_eq!(
        w.recent_drag_anims.last().unwrap().graphic,
        0x0EEC,
        "gem remaps to its flight graphic"
    );
    assert_eq!(
        w.recent_drag_anims.last().unwrap().source,
        0,
        "unknown source serial is zeroed"
    );

    // A second event bumps the monotonic seq.
    assert!(apply_packet(
        &mut w,
        &drag_animation_frame(0x0EF0, 0, 0, 1, 0x1111, 5, 6, 1, 0x2222, 7, 8, 2)
    ));
    let last = w.recent_drag_anims.last().unwrap();
    assert_eq!(last.seq, 2);
    assert_eq!(
        last.graphic, 0x0EF2,
        "gem stack remaps to its flight graphic"
    );
}

/// Push a length-prefixed (`u8` length) string field, as used by 0x71 subs 1/2.
fn push_str8(p: &mut PacketWriter, s: &str) {
    let bytes = s.as_bytes();
    p.u8(bytes.len() as u8).bytes(bytes);
}

#[test]
fn bulletin_board_open_sets_name_and_serial() {
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0x71).u16(0); // id + length placeholder
    p.u8(0).u32(0xB0A2D); // sub 0, board serial
    p.fixed_ascii("Trade Board", 22); // 22-byte name field, NUL-padded
    let frame = patch_len(p.into_vec());

    assert!(apply_packet(&mut w, &frame));
    let board = w.bulletin_board.as_ref().expect("board opened");
    assert_eq!(board.serial, 0xB0A2D);
    assert_eq!(board.name, "Trade Board");
    assert!(board.summaries.is_empty());
}

#[test]
fn bulletin_board_summary_appends_only_for_the_open_board() {
    let mut w = World::new();

    // sub 1 with no board open at all: no-op, no panic.
    let mut none_open = PacketWriter::new();
    none_open.u8(0x71).u16(0);
    none_open.u8(1).u32(0xB0A2D).u32(1).u32(0);
    push_str8(&mut none_open, "Alice");
    push_str8(&mut none_open, "Selling swords");
    push_str8(&mut none_open, "Jul 1 12:00");
    assert!(apply_packet(&mut w, &patch_len(none_open.into_vec())));
    assert!(w.bulletin_board.is_none());

    // Open board 0xB0A2D.
    let mut open = PacketWriter::new();
    open.u8(0x71).u16(0);
    open.u8(0).u32(0xB0A2D);
    open.fixed_ascii("Trade Board", 22);
    assert!(apply_packet(&mut w, &patch_len(open.into_vec())));

    // sub 1 for a DIFFERENT board serial: ignored (mirrors ClassicUO only
    // updating a `BulletinBoardGump` it can find open for that serial).
    let mut mismatched = PacketWriter::new();
    mismatched.u8(0x71).u16(0);
    mismatched.u8(1).u32(0xFFFFFF).u32(2).u32(0);
    push_str8(&mut mismatched, "Bob");
    push_str8(&mut mismatched, "Buying shields");
    push_str8(&mut mismatched, "Jul 2 09:00");
    assert!(apply_packet(&mut w, &patch_len(mismatched.into_vec())));
    assert!(w.bulletin_board.as_ref().unwrap().summaries.is_empty());

    // sub 1 for the matching board serial: appended.
    let mut matched = PacketWriter::new();
    matched.u8(0x71).u16(0);
    matched.u8(1).u32(0xB0A2D).u32(3).u32(0);
    push_str8(&mut matched, "Alice");
    push_str8(&mut matched, "Selling swords");
    push_str8(&mut matched, "Jul 1 12:00");
    assert!(apply_packet(&mut w, &patch_len(matched.into_vec())));

    let board = w.bulletin_board.as_ref().unwrap();
    assert_eq!(board.summaries.len(), 1);
    let summary = &board.summaries[0];
    assert_eq!(summary.serial, 3);
    assert_eq!(summary.parent, 0);
    assert_eq!(summary.poster, "Alice");
    assert_eq!(summary.subject, "Selling swords");
    assert_eq!(summary.datetime, "Jul 1 12:00");
}

#[test]
fn bulletin_board_full_message_reconstructs_multiline_body() {
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0x71).u16(0); // id + length placeholder
    p.u8(2).u32(0xB0A2D).u32(3); // sub 2, board serial, message serial
    push_str8(&mut p, "Alice"); // poster (ASCII)
    push_str8(&mut p, "Selling swords"); // subject (UTF-8)
    push_str8(&mut p, "Jul 1 12:00"); // datetime (ASCII)
    p.zeros(4); // unused
    p.u8(1); // unk count
    p.zeros(4); // unk*4 skipped bytes
    p.u8(3); // line count
    push_str8(&mut p, " Hello"); // leading space, trimmed only at body start
    push_str8(&mut p, ""); // empty line: dropped, not joined
    push_str8(&mut p, "World");
    let frame = patch_len(p.into_vec());

    assert!(apply_packet(&mut w, &frame));
    let msg = w.bulletin_message.as_ref().expect("message stored");
    assert_eq!(msg.board, 0xB0A2D);
    assert_eq!(msg.serial, 3);
    assert_eq!(msg.poster, "Alice");
    assert_eq!(msg.subject, "Selling swords");
    assert_eq!(msg.datetime, "Jul 1 12:00");
    assert_eq!(msg.body, "Hello\nWorld");
}

#[test]
fn bulletin_board_truncated_frame_is_swallowed_without_partial_state() {
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0x71).u16(0);
    p.u8(2).u32(0xB0A2D).u32(3);
    push_str8(&mut p, "Alice");
    // Cut off mid-subject: length byte says more follows than actually does.
    p.u8(200).bytes(b"short");
    let frame = patch_len(p.into_vec());

    assert!(apply_packet(&mut w, &frame), "swallowed, not fatal");
    assert!(w.bulletin_message.is_none());
}

/// Write a UTF-16 BE, NUL-terminated string (UO's `ReadUnicodeBE` field).
fn push_utf16be(p: &mut PacketWriter, s: &str) {
    for unit in s.encode_utf16() {
        p.u16(unit);
    }
    p.u16(0); // NUL terminator
}

#[test]
fn chat_create_conference_sets_channel_and_current() {
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0xB2).u16(0); // id + length placeholder
    p.u16(0x03E8); // create conference
    p.u32(0); // skipped header
    push_utf16be(&mut p, "General");
    p.u16(0x31); // has password
    let frame = patch_len(p.into_vec());

    assert!(apply_packet(&mut w, &frame));
    assert_eq!(w.chat.current_channel, "General");
    assert_eq!(w.chat.channels.len(), 1);
    assert_eq!(w.chat.channels[0].name, "General");
    assert!(w.chat.channels[0].has_password);
}

#[test]
fn chat_join_conference_sets_current_channel() {
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0xB2).u16(0);
    p.u16(0x03F1); // you have joined a conference
    p.u32(0);
    push_utf16be(&mut p, "Trade");
    let frame = patch_len(p.into_vec());

    assert!(apply_packet(&mut w, &frame));
    assert_eq!(w.chat.current_channel, "Trade");
    assert_eq!(w.chat.channels.len(), 1);
    assert_eq!(w.chat.channels[0].name, "Trade");
    assert!(!w.chat.channels[0].has_password);
}

#[test]
fn chat_message_pushes_line_and_strips_brace_span() {
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0xB2).u16(0);
    p.u16(0x0025); // chat message
    p.u32(0); // skipped header
    p.u16(0); // msg type
    push_utf16be(&mut p, "Alice");
    push_utf16be(&mut p, "hello {c:#FF0000}world");
    let frame = patch_len(p.into_vec());

    assert!(apply_packet(&mut w, &frame));
    assert_eq!(w.chat_messages.len(), 1);
    assert_eq!(w.chat_messages[0].sender, "Alice");
    assert_eq!(w.chat_messages[0].text, "hello world");
    assert_eq!(w.chat_messages[0].seq, 1);
}

#[test]
fn chat_destroy_conference_removes_only_the_named_channel() {
    let mut w = World::new();
    for ch in ["General", "Trade"] {
        let mut p = PacketWriter::new();
        p.u8(0xB2).u16(0);
        p.u16(0x03E8);
        p.u32(0);
        push_utf16be(&mut p, ch);
        p.u16(0x30); // no password
        assert!(apply_packet(&mut w, &patch_len(p.into_vec())));
    }
    assert_eq!(w.chat.channels.len(), 2);
    // Destroy just "General".
    let mut p = PacketWriter::new();
    p.u8(0xB2).u16(0);
    p.u16(0x03E9);
    p.u32(0);
    push_utf16be(&mut p, "General");
    assert!(apply_packet(&mut w, &patch_len(p.into_vec())));
    assert_eq!(w.chat.channels.len(), 1);
    assert_eq!(w.chat.channels[0].name, "Trade");
}

#[test]
fn chat_localized_system_message_stored_with_empty_sender() {
    // A localized cmd (0x0001..=0x0024) carries only a text payload; we have
    // no Chat.enu template table, so the raw payload is stored with no
    // sender. An out-of-range cmd is ignored.
    let mut w = World::new();
    let mut p = PacketWriter::new();
    p.u8(0xB2).u16(0);
    p.u16(0x0001); // localized system message
    p.u32(0); // skipped header
    push_utf16be(&mut p, "the system speaks");
    assert!(apply_packet(&mut w, &patch_len(p.into_vec())));
    assert_eq!(w.chat_messages.len(), 1);
    assert!(w.chat_messages[0].sender.is_empty());
    assert_eq!(w.chat_messages[0].text, "the system speaks");

    // Unknown/out-of-range cmd → ignored, no new line.
    let mut p = PacketWriter::new();
    p.u8(0xB2).u16(0);
    p.u16(0x0100);
    p.u32(0);
    assert!(apply_packet(&mut w, &patch_len(p.into_vec())));
    assert_eq!(w.chat_messages.len(), 1);
}

#[test]
fn chat_close_clears_state() {
    let mut w = World::new();
    // Seed some state first via a create-conference.
    let mut p = PacketWriter::new();
    p.u8(0xB2).u16(0);
    p.u16(0x03E8);
    p.u32(0);
    push_utf16be(&mut p, "General");
    p.u16(0);
    assert!(apply_packet(&mut w, &patch_len(p.into_vec())));
    assert!(!w.chat.channels.is_empty());

    // 0x03EC close chat clears everything back to default.
    let mut q = PacketWriter::new();
    q.u8(0xB2).u16(0);
    q.u16(0x03EC);
    assert!(apply_packet(&mut w, &patch_len(q.into_vec())));
    assert_eq!(w.chat, crate::world::ChatState::default());
    assert_eq!(w.chat.enabled, ChatStatus::Disabled);
    assert!(w.chat.channels.is_empty());
    assert!(w.chat.current_channel.is_empty());
}

#[test]
fn chat_enable_transitions() {
    let mut w = World::new();
    assert_eq!(w.chat.enabled, ChatStatus::Disabled);

    // 0x03EB display enter-username → EnabledUserRequest.
    let mut p = PacketWriter::new();
    p.u8(0xB2).u16(0);
    p.u16(0x03EB);
    assert!(apply_packet(&mut w, &patch_len(p.into_vec())));
    assert_eq!(w.chat.enabled, ChatStatus::EnabledUserRequest);

    // 0x03ED username accepted → Enabled.
    let mut q = PacketWriter::new();
    q.u8(0xB2).u16(0);
    q.u16(0x03ED);
    q.u32(0);
    push_utf16be(&mut q, "Bob");
    assert!(apply_packet(&mut w, &patch_len(q.into_vec())));
    assert_eq!(w.chat.enabled, ChatStatus::Enabled);
}

/// 0xCC's affix belongs to the *resolved* line, not to the argument list —
/// folding it into the args both corrupts them and loses the affix entirely on
/// a template with no placeholder. The prepend flag decides which side.
#[test]
fn cliloc_affix_is_kept_apart_from_the_arguments() {
    fn packet(flags: u8, affix: &[u8], args: &str) -> Vec<u8> {
        let mut p = PacketWriter::new();
        p.u8(0xCC).u16(0); // id + length placeholder
        p.u32(0x1234).u16(0x0190).u8(0).u16(0x3B2).u16(3); // serial, body, type, hue, font
        p.u32(1_005_445).u8(flags); // cliloc + affix flags
        p.fixed_ascii("System", 30);
        p.bytes(affix).u8(0); // NUL-terminated affix
        for u in args.encode_utf16() {
            p.u16(u); // 0xCC arguments are big-endian UTF-16
        }
        let mut f = p.into_vec();
        let n = f.len() as u16;
        f[1] = (n >> 8) as u8;
        f[2] = n as u8;
        f
    }

    let mut w = World::new();
    assert!(apply_packet(&mut w, &packet(0x00, b" (appended)", "Anima")));
    let j = w.journal.last().expect("journal line");
    assert_eq!(j.cliloc, 1_005_445);
    assert_eq!(j.text, "Anima", "arguments must stay pure");
    assert_eq!(j.affix, " (appended)");
    assert!(!j.affix_prepend);

    assert!(apply_packet(&mut w, &packet(0x01, b"[guild] ", "Anima")));
    let j = w.journal.last().expect("journal line");
    assert_eq!(j.text, "Anima");
    assert_eq!(j.affix, "[guild] ");
    assert!(j.affix_prepend, "flag 0x01 is AffixType.Prepend");
}

/// 0xDF carries the buff's real name as a cliloc with its own arguments; the
/// 35-entry English table keyed off the icon is only a fallback. The argument
/// blocks are length-prefixed UTF-16 **little**-endian here — 0xCC's are big.
#[test]
fn buff_carries_its_title_and_description_clilocs() {
    let mut p = PacketWriter::new();
    p.u8(0xDF).u16(0); // id + length placeholder
    p.u32(0x1001).u16(0x03ED).u16(1); // serial, icon (Night Sight), count
    p.u16(0).u16(0).u16(0x03ED).u16(0).u32(0); // source, pad, icon, queue, pad
    p.u16(600); // timer, seconds
    p.zeros(3);
    p.u32(1_075_628).u32(1_075_629).u32(0); // title, description, "wtf"
    let args: Vec<u8> = "Anima"
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .chain([0, 0])
        .collect();
    p.u16(args.len() as u16).bytes(&args);
    p.u16(0); // no description arguments — the title's are reused
    let mut f = p.into_vec();
    let n = f.len() as u16;
    f[1] = (n >> 8) as u8;
    f[2] = n as u8;

    let mut w = World::new();
    assert!(apply_packet(&mut w, &f));
    let b = w.buffs.first().expect("buff recorded");
    assert_eq!(b.icon, 0x03ED);
    assert_eq!(b.dur, 600);
    assert_eq!(b.title_cliloc, 1_075_628);
    assert_eq!(b.desc_cliloc, 1_075_629);
    assert_eq!(b.title_args, "Anima");
    assert_eq!(
        b.desc_args, "Anima",
        "an empty description argument block falls back to the title's"
    );
    assert_eq!(b.name, "Night Sight", "the English fallback still stands");
    assert!(
        b.display.is_empty(),
        "words are the driver's job, not the core's"
    );
}
