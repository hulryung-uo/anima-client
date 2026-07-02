//! UOP (Ultima Online Patch) container reader.
//!
//! UOP files store entries keyed by a Jenkins `HashLittle2` of their virtual
//! path. Ported from `anima/anima/uop.py`.
//!
//! Two readers share the same block-chain table parser ([`parse_uop_table`]):
//! [`UopReader`] loads the whole file into memory (fine for small containers —
//! gump/map/art chunks, `AnimationSequence.uop`'s ~113KB); [`LazyUopReader`]
//! parses only the entry table and reads+decompresses ONE entry at a time from
//! a shared file handle, for containers too big to load whole (the
//! `AnimationFrame{1..4}.uop` set used by [`crate::anim`]'s UOP animation
//! path, ~509MB combined).

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Mutex;

use flate2::read::ZlibDecoder;

/// Jenkins HashLittle2 of a UOP entry path, returned as a 64-bit value.
pub fn uop_hash(s: &str) -> u64 {
    let b = s.as_bytes();
    let length = b.len();
    let seed = (length as u32).wrapping_add(0xDEAD_BEEF);
    let (mut ebx, mut edi, mut esi) = (seed, seed, seed);
    let w = |x: &[u8], i: usize| -> u32 {
        (x[i] as u32) | ((x[i + 1] as u32) << 8) | ((x[i + 2] as u32) << 16) | ((x[i + 3] as u32) << 24)
    };

    let mut i = 0usize;
    while i + 12 < length {
        edi = edi.wrapping_add(w(b, i + 4));
        esi = esi.wrapping_add(w(b, i + 8));
        let mut edx = w(b, i).wrapping_sub(esi);
        edx = edx.wrapping_add(ebx) ^ esi.rotate_left(4);
        esi = esi.wrapping_add(edi);
        edi = edi.wrapping_sub(edx) ^ edx.rotate_left(6);
        edx = edx.wrapping_add(esi);
        esi = esi.wrapping_sub(edi) ^ edi.rotate_left(8);
        edi = edi.wrapping_add(edx);
        ebx = edx.wrapping_sub(esi) ^ esi.rotate_left(16);
        esi = esi.wrapping_add(edi);
        edi = edi.wrapping_sub(ebx) ^ ebx.rotate_left(19);
        ebx = ebx.wrapping_add(esi);
        esi = esi.wrapping_sub(edi) ^ edi.rotate_left(4);
        edi = edi.wrapping_add(ebx);
        i += 12;
    }

    let remaining = length - i;
    if remaining > 0 {
        let byte = |k: usize| b[i + k] as u32;
        if remaining >= 12 { esi = esi.wrapping_add(byte(11) << 24); }
        if remaining >= 11 { esi = esi.wrapping_add(byte(10) << 16); }
        if remaining >= 10 { esi = esi.wrapping_add(byte(9) << 8); }
        if remaining >= 9 { esi = esi.wrapping_add(byte(8)); }
        if remaining >= 8 { edi = edi.wrapping_add(byte(7) << 24); }
        if remaining >= 7 { edi = edi.wrapping_add(byte(6) << 16); }
        if remaining >= 6 { edi = edi.wrapping_add(byte(5) << 8); }
        if remaining >= 5 { edi = edi.wrapping_add(byte(4)); }
        if remaining >= 4 { ebx = ebx.wrapping_add(byte(3) << 24); }
        if remaining >= 3 { ebx = ebx.wrapping_add(byte(2) << 16); }
        if remaining >= 2 { ebx = ebx.wrapping_add(byte(1) << 8); }
        if remaining >= 1 { ebx = ebx.wrapping_add(byte(0)); }

        esi = (esi ^ edi).wrapping_sub(edi.rotate_left(14));
        let ecx = (esi ^ ebx).wrapping_sub(esi.rotate_left(11));
        edi = (edi ^ ecx).wrapping_sub(ecx.rotate_left(25));
        esi = (esi ^ edi).wrapping_sub(edi.rotate_left(16));
        let edx = (esi ^ ecx).wrapping_sub(esi.rotate_left(4));
        edi = (edi ^ edx).wrapping_sub(edx.rotate_left(14));
        let eax = (esi ^ edi).wrapping_sub(edi.rotate_left(24));
        ((edi as u64) << 32) | eax as u64
    } else {
        (esi as u64) << 32
    }
}

#[derive(Debug)]
struct Entry {
    offset: usize,
    compressed_size: usize,
    decompressed_size: usize,
    compression: i16,
}

/// Parse a UOP file's block-chain entry table (magic + the linked list of
/// fixed-size directory blocks starting at the `i64` pointer at byte 12) into
/// `path hash -> Entry`, PLUS the hashes in file/table parse order (see the
/// second return value's doc below). Shared by [`UopReader`] (an in-memory
/// `Cursor`) and [`LazyUopReader`] (a `File`, seeking block-to-block) — this
/// is the ONLY thing either reader reads eagerly; entry payloads are read on
/// demand.
///
/// Returns `(hash -> Entry, hashes in file order)`. The order vec matters for
/// [`UopReader::all_entries`]/`AnimationSequence.uop`: ClassicUO's
/// `_uopInfos[animID] = uopInfo` makes a LATER entry (in file/table order)
/// for a duplicate animID replace the whole earlier one, so callers that scan
/// by content (rather than by hash) must walk entries in the SAME order the
/// file lists them, not HashMap iteration order (which is unrelated to file
/// order and not even stable across runs).
fn parse_uop_table<R: Read + Seek>(r: &mut R) -> std::io::Result<(HashMap<u64, Entry>, Vec<u64>)> {
    let mut hdr = [0u8; 20];
    r.read_exact(&mut hdr)?;
    let magic = u32::from_le_bytes(hdr[0..4].try_into().unwrap());
    if magic != 0x0050_594D {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("not a UOP file: bad magic 0x{magic:X}"),
        ));
    }
    let mut next_block = i64::from_le_bytes(hdr[12..20].try_into().unwrap());

    let mut entries = HashMap::new();
    let mut order = Vec::new();
    // A malformed/absent next block (bad pointer, truncated file) just fails
    // the `read_exact` below and we stop, same as the old bounds check against
    // `data.len()` on the in-memory reader.
    while next_block != 0 {
        if r.seek(SeekFrom::Start(next_block as u64)).is_err() {
            break;
        }
        let mut blk = [0u8; 12];
        if r.read_exact(&mut blk).is_err() {
            break;
        }
        let count = i32::from_le_bytes(blk[0..4].try_into().unwrap());
        next_block = i64::from_le_bytes(blk[4..12].try_into().unwrap());
        if count <= 0 {
            continue;
        }

        // Validate `count` against what the stream can actually hold BEFORE
        // allocating `count * 34` bytes for the record table: a corrupt/
        // malicious header claiming ~i32::MAX records would otherwise request
        // a ~68GB allocation (and on a 32-bit target like wasm32, `count as
        // usize * 34` can silently wrap instead of even getting that far).
        // `checked_mul` + a remaining-bytes bound catches both.
        let cur = r.stream_position()?;
        let end = r.seek(SeekFrom::End(0))?;
        r.seek(SeekFrom::Start(cur))?;
        let remaining = end.saturating_sub(cur);
        let need = (count as u64)
            .checked_mul(34)
            .filter(|&n| n <= remaining)
            .and_then(|n| usize::try_from(n).ok());
        let Some(need) = need else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("uop: directory block claims {count} records but only {remaining} bytes remain"),
            ));
        };

        let mut rec = vec![0u8; need];
        if r.read_exact(&mut rec).is_err() {
            break;
        }
        for i in 0..count as usize {
            let o = i * 34;
            let offset = i64::from_le_bytes(rec[o..o + 8].try_into().unwrap()) as usize;
            let header_len = u32::from_le_bytes(rec[o + 8..o + 12].try_into().unwrap()) as usize;
            let compressed = u32::from_le_bytes(rec[o + 12..o + 16].try_into().unwrap()) as usize;
            let decompressed = u32::from_le_bytes(rec[o + 16..o + 20].try_into().unwrap()) as usize;
            let file_hash = u64::from_le_bytes(rec[o + 20..o + 28].try_into().unwrap());
            let compression = i16::from_le_bytes(rec[o + 32..o + 34].try_into().unwrap());

            if offset == 0 || compressed == 0 {
                continue;
            }
            order.push(file_hash);
            entries.insert(
                file_hash,
                Entry {
                    offset: offset + header_len,
                    compressed_size: compressed,
                    decompressed_size: decompressed,
                    compression,
                },
            );
        }
    }
    Ok((entries, order))
}

/// A parsed UOP container, fully loaded into memory (both the entry table AND
/// every entry's raw bytes) — fine for small-to-medium containers; see
/// [`LazyUopReader`] for the ~509MB `AnimationFrame*.uop` set.
pub struct UopReader {
    data: Vec<u8>,
    entries: HashMap<u64, Entry>,
    /// Hashes in file/table parse order — see [`parse_uop_table`]'s doc
    /// comment on why [`Self::all_entries`] must walk this instead of
    /// `entries.values()` (whose order is unrelated to file order).
    order: Vec<u64>,
}

impl UopReader {
    pub fn open(path: &Path) -> std::io::Result<UopReader> {
        let data = std::fs::read(path)?;
        let mut cursor = std::io::Cursor::new(&data);
        let (entries, order) = parse_uop_table(&mut cursor)?;
        Ok(UopReader { data, entries, order })
    }

    /// Number of parsed entries.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Decompressed bytes for one entry, bounds-checked against `data`: a
    /// truncated/corrupt file can claim an `offset`/`compressed_size` that
    /// runs past the end of what was actually read (e.g. a partial download),
    /// and unlike the old direct-slice version this must degrade to `None`
    /// (letting the caller fall back to an identity table) rather than panic
    /// — [`crate::anim::Anim::open`] scans every `AnimationSequence.uop` entry
    /// via [`Self::all_entries`] at open time, so a panic here used to take
    /// the whole `Anim::open` down with it.
    fn decode(&self, e: &Entry) -> Option<Vec<u8>> {
        let end = e.offset.checked_add(e.compressed_size)?;
        let raw = self.data.get(e.offset..end)?;
        if e.compression == 1 {
            let mut out = Vec::new();
            ZlibDecoder::new(raw).read_to_end(&mut out).ok()?;
            Some(out)
        } else {
            Some(raw.to_vec())
        }
    }

    /// Decompressed bytes for an entry identified by its path hash.
    pub fn by_hash(&self, hash: u64) -> Option<Vec<u8>> {
        self.decode(self.entries.get(&hash)?)
    }

    /// Decompressed bytes of every entry, in FILE/TABLE PARSE ORDER — for
    /// small containers that must be scanned by CONTENT rather than looked up
    /// by a known virtual path. `AnimationSequence.uop` is the motivating
    /// case: the body id each entry describes lives inside the (compressed)
    /// payload itself, not in a path we could hash up front, and a later
    /// duplicate-body entry must be seen AFTER an earlier one (see
    /// [`parse_uop_table`]'s doc comment) — hence walking `order`, not
    /// `entries.values()`.
    pub fn all_entries(&self) -> impl Iterator<Item = Vec<u8>> + '_ {
        self.order.iter().filter_map(move |h| self.decode(self.entries.get(h)?))
    }

    /// Decompressed bytes for `pattern.format(index)`, e.g.
    /// `"build/map0legacymul/{:08}.dat"` with `index`.
    pub fn by_map_chunk(&self, index: usize) -> Option<Vec<u8>> {
        let path = format!("build/map0legacymul/{index:08}.dat");
        self.by_hash(uop_hash(&path))
    }

    /// Decompressed bytes for an art entry (`build/artlegacymul/{:08}.tga` — the
    /// art UOP uses a legacy `.tga` extension in its virtual paths, unlike the
    /// map's `.dat`). `index` = land graphic (0..0x3FFF) or `0x4000 + static`.
    pub fn by_art(&self, index: usize) -> Option<Vec<u8>> {
        let path = format!("build/artlegacymul/{index:08}.tga");
        self.by_hash(uop_hash(&path))
    }

    /// Decompressed bytes for a sound entry (`build/soundlegacymul/{:08}.dat`).
    /// `index` = the sound id used by the 0x54 PlaySoundEffect packet.
    pub fn by_sound(&self, index: usize) -> Option<Vec<u8>> {
        let path = format!("build/soundlegacymul/{index:08}.dat");
        self.by_hash(uop_hash(&path))
    }

    /// Gump entry (`build/gumpartlegacymul/{:08}.tga`). Gump UOP entries carry an
    /// 8-byte "extra" (width:i32, height:i32) ahead of the payload (ClassicUO opens
    /// this file with `hasExtra=true`). Returns `(decompressed payload, width, height)`
    /// where the payload is the row-lookup table + RLE pixel runs.
    pub fn by_gump(&self, index: usize) -> Option<(Vec<u8>, u32, u32)> {
        let path = format!("build/gumpartlegacymul/{index:08}.tga");
        let e = self.entries.get(&uop_hash(&path))?;
        let d = &self.data;
        if e.offset + 8 > d.len() {
            return None;
        }
        let le32 = |o: usize| u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]);
        let mut w = le32(e.offset);
        let mut h = le32(e.offset + 4);
        let end = (e.offset + 8 + e.compressed_size).min(d.len());
        let payload = &d[e.offset + 8..end];
        let mut out = if e.compression == 1 {
            let mut out = Vec::with_capacity(e.decompressed_size);
            ZlibDecoder::new(payload).read_to_end(&mut out).ok()?;
            out
        } else {
            payload.to_vec()
        };
        // Fallback: some entries store width/height as the first two u32 of the
        // (decompressed) payload instead of the extra field.
        if (w == 0 || h == 0 || w > 4096 || h > 4096) && out.len() >= 8 {
            w = u32::from_le_bytes([out[0], out[1], out[2], out[3]]);
            h = u32::from_le_bytes([out[4], out[5], out[6], out[7]]);
            out.drain(0..8);
        }
        if w == 0 || h == 0 || w > 4096 || h > 4096 {
            return None;
        }
        Some((out, w, h))
    }
}

/// Lazy UOP reader for containers too big to load whole: parses only the
/// entry table (the same [`parse_uop_table`] as [`UopReader`], but seeking a
/// `File` instead of loading it) and keeps a `Mutex<File>` to read+decompress
/// ONE entry at a time on demand. Opening never reads a single entry's
/// payload, so opening all four `AnimationFrame{1..4}.uop` (~509MB combined)
/// only touches their small block-chain tables, not their animation data.
pub struct LazyUopReader {
    file: Mutex<File>,
    entries: HashMap<u64, Entry>,
}

impl LazyUopReader {
    pub fn open(path: &Path) -> std::io::Result<LazyUopReader> {
        let mut file = File::open(path)?;
        // Only `entries` (hash -> Entry) is needed here — the file/parse-order
        // vec is for `UopReader::all_entries`'s content scan, which
        // `LazyUopReader` doesn't offer (it's only ever looked up by hash).
        let (entries, _order) = parse_uop_table(&mut file)?;
        Ok(LazyUopReader { file: Mutex::new(file), entries })
    }

    /// Number of parsed entries.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Decompressed bytes for an entry, read from disk on demand (briefly
    /// locks the shared file handle). `None` on I/O error, a missing hash, or
    /// an unsupported compression flag — every entry we've inspected across
    /// the real `AnimationFrame{1..4}.uop`/`AnimationSequence.uop` files is
    /// flag `1` (Zlib); ClassicUO also defines a `ZlibBwt` flag (`3`) but we
    /// deliberately have NOT ported `BwtDecompress` since no real entry needs
    /// it — an entry with any other flag just logs and returns `None` instead
    /// of silently misdecoding.
    pub fn by_hash(&self, hash: u64) -> Option<Vec<u8>> {
        let e = self.entries.get(&hash)?;
        let mut f = self.file.lock().ok()?;
        f.seek(SeekFrom::Start(e.offset as u64)).ok()?;
        let mut raw = vec![0u8; e.compressed_size];
        f.read_exact(&mut raw).ok()?;
        match e.compression {
            0 => Some(raw),
            1 => {
                let mut out = Vec::with_capacity(e.decompressed_size);
                ZlibDecoder::new(&raw[..]).read_to_end(&mut out).ok()?;
                Some(out)
            }
            other => {
                eprintln!(
                    "uop: entry has unsupported compression flag {other} (only None/Zlib are \
                     ported — BwtDecompress was never needed by real data); skipping"
                );
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal one-entry UOP file: magic/header, a single directory
    /// block (1 record, no chaining), then the entry's raw payload right
    /// after the table. Compression flag `0` (None) — exercises the same
    /// table layout [`LazyUopReader`] and [`UopReader`] both parse, without
    /// needing a real zlib stream.
    fn build_single_entry_uop(path: &str, payload: &[u8]) -> Vec<u8> {
        let mut buf = vec![0u8; 20]; // magic(4) + version(4) + timestamp(4) + next_block(8)
        buf[0..4].copy_from_slice(&0x0050_594Du32.to_le_bytes());
        buf[12..20].copy_from_slice(&20i64.to_le_bytes()); // first (only) block starts right after

        // Block header: count=1, next_block=0 (end of chain).
        buf.extend_from_slice(&1i32.to_le_bytes());
        buf.extend_from_slice(&0i64.to_le_bytes());

        let hash = uop_hash(path);
        let payload_offset = buf.len() as i64 + 34; // right after this one 34-byte record
        buf.extend_from_slice(&payload_offset.to_le_bytes()); // offset
        buf.extend_from_slice(&0i32.to_le_bytes()); // header_len
        buf.extend_from_slice(&(payload.len() as i32).to_le_bytes()); // compressed_size
        buf.extend_from_slice(&(payload.len() as i32).to_le_bytes()); // decompressed_size
        buf.extend_from_slice(&hash.to_le_bytes()); // file_hash
        buf.extend_from_slice(&0u32.to_le_bytes()); // data_hash (unused)
        buf.extend_from_slice(&0i16.to_le_bytes()); // compression flag: None

        buf.extend_from_slice(payload);
        buf
    }

    #[test]
    fn parse_uop_table_reads_single_entry() {
        let path = "build/test/00000001.bin";
        let payload = b"hello uop";
        let data = build_single_entry_uop(path, payload);

        let mut cursor = std::io::Cursor::new(&data);
        let (entries, order) = parse_uop_table(&mut cursor).expect("parse");
        assert_eq!(entries.len(), 1);
        assert_eq!(order, vec![uop_hash(path)]);

        let e = entries.get(&uop_hash(path)).expect("entry present under its path hash");
        assert_eq!(e.compressed_size, payload.len());
        assert_eq!(e.compression, 0);
        assert_eq!(&data[e.offset..e.offset + e.compressed_size], payload);
    }

    #[test]
    fn parse_uop_table_rejects_block_count_exceeding_remaining_data() {
        // Corrupt directory block: claims i32::MAX records (~68GB of 34-byte
        // record table) but the stream has nothing left after the 12-byte
        // block header. Must fail cleanly with an io::Error, not attempt the
        // allocation (or, on a 32-bit target, wrap `count * 34` and read
        // garbage).
        let mut buf = vec![0u8; 20];
        buf[0..4].copy_from_slice(&0x0050_594Du32.to_le_bytes());
        buf[12..20].copy_from_slice(&20i64.to_le_bytes());
        buf.extend_from_slice(&i32::MAX.to_le_bytes()); // count
        buf.extend_from_slice(&0i64.to_le_bytes()); // next_block = 0

        let mut cursor = std::io::Cursor::new(&buf);
        let err = parse_uop_table(&mut cursor).expect_err("bogus count must be rejected");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn uop_reader_and_lazy_reader_agree_on_a_synthetic_file() {
        let path = "build/test/00000001.bin";
        let payload = b"hello lazy uop reader";
        let data = build_single_entry_uop(path, payload);

        let dir = std::env::temp_dir();
        let file_path = dir.join(format!("anima_uop_test_{}.uop", std::process::id()));
        std::fs::write(&file_path, &data).expect("write temp uop");

        let eager = UopReader::open(&file_path).expect("open eager");
        let lazy = LazyUopReader::open(&file_path).expect("open lazy");
        assert_eq!(eager.entry_count(), 1);
        assert_eq!(lazy.entry_count(), 1);

        let hash = uop_hash(path);
        assert_eq!(eager.by_hash(hash).as_deref(), Some(payload.as_slice()));
        assert_eq!(lazy.by_hash(hash).as_deref(), Some(payload.as_slice()));

        std::fs::remove_file(&file_path).ok();
    }

    #[test]
    fn decode_degrades_to_none_on_truncated_entry_instead_of_panicking() {
        // The directory table still declares the entry's real (untruncated)
        // offset/compressed_size, but the file itself got cut short (e.g. a
        // partial download of AnimationSequence.uop) — `decode` must return
        // `None` for it instead of panicking on an out-of-range slice, since
        // `Anim::open` scans every entry via `all_entries` at startup and a
        // panic there would take the whole open down instead of degrading to
        // an identity replace table (see module docs).
        let path = "build/test/00000001.bin";
        let payload = b"hello uop truncated test payload";
        let mut data = build_single_entry_uop(path, payload);
        data.truncate(data.len() - 5); // chop off the tail of the payload

        let dir = std::env::temp_dir();
        let file_path = dir.join(format!("anima_uop_truncated_test_{}.uop", std::process::id()));
        std::fs::write(&file_path, &data).expect("write temp uop");

        let reader = UopReader::open(&file_path).expect("the table itself still parses fine");
        assert_eq!(reader.by_hash(uop_hash(path)), None);
        assert_eq!(reader.all_entries().count(), 0);

        std::fs::remove_file(&file_path).ok();
    }

    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource (real AnimationFrame1.uop, ~100MB)
    fn lazy_reader_matches_eager_reader_on_real_file() {
        let path = format!("{}/dev/uo/uo-resource/AnimationFrame1.uop", std::env::var("HOME").unwrap());
        let eager = UopReader::open(std::path::Path::new(&path)).expect("open eager");
        let lazy = LazyUopReader::open(std::path::Path::new(&path)).expect("open lazy");
        assert_eq!(eager.entry_count(), lazy.entry_count());
        println!("AnimationFrame1.uop: {} entries", eager.entry_count());

        // Spot-check a handful of real entries decode identically both ways.
        let mut checked = 0;
        for body in 0u16..2000 {
            for action in 0u8..40 {
                let hash = uop_hash(&format!("build/animationlegacyframe/{body:06}/{action:02}.bin"));
                match (eager.by_hash(hash), lazy.by_hash(hash)) {
                    (Some(a), Some(b)) => {
                        assert_eq!(a, b, "body {body} action {action} decoded differently");
                        checked += 1;
                    }
                    (None, None) => {}
                    (a, b) => panic!("body {body} action {action}: mismatch presence eager={a:?} lazy_len={:?}", b.map(|v| v.len())),
                }
                if checked >= 20 {
                    break;
                }
            }
            if checked >= 20 {
                break;
            }
        }
        println!("spot-checked {checked} matching entries");
        assert!(checked > 0, "should find at least one real animation .bin to cross-check");
    }
}
