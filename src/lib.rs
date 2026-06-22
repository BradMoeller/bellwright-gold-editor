//! Core logic for reading and editing player stats in Bellwright `.sav` files.
//!
//! Format: a UE `FArchive::SerializeCompressed` container ("VSWB") whose payload
//! is a custom protobuf blob.
//!
//! **Gold** is protobuf field 6 inside a record matched by the globally-unique
//! signature `2a <len> <id> 30 <varint> 3a` (the `<id>` length is read as a
//! varint: 4 bytes in save format v7, 6 bytes in v8).
//!
//! **Renown** is one entry in a large list of identically-shaped reputation
//! records `20 89 c8 9b dd 02 22 <len> 08 <id1> 10 <value> 28 bb 0d`; nothing
//! structural is unique to it, so the record is located by matching `<value>`
//! against the player's known current renown (see `find_renown`).
//!
//! This module has no GUI dependencies so it can be unit-tested directly.

use std::path::Path;

const BLOCK: usize = 131072; // 128 KiB uncompressed block size

/// UE `PACKAGE_FILE_TAG` that begins the `SerializeCompressed` container.
/// Everything structural (Summary.CompressedSize, the byte-shifted total-size
/// copy, and the chunk table) sits at fixed offsets relative to this tag.
const PACKAGE_FILE_TAG: [u8; 4] = [0xc1, 0x83, 0x2a, 0x9e];

#[derive(Debug)]
pub enum Error {
    Io(String),
    BadFile(String),
    Decompress(String),
    GoldNotFound(usize),   // number of candidate matches found (0 or >1)
    RenownNotFound(usize), // number of candidate matches found (0 or >1)
    ParseError(String),
    TooLarge(String),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(s) => write!(f, "File error: {s}"),
            Error::BadFile(s) => write!(f, "Not a recognizable Bellwright save: {s}"),
            Error::Decompress(s) => write!(f, "Decompression failed: {s}"),
            Error::GoldNotFound(0) => write!(f, "Could not locate the gold field in this save."),
            Error::GoldNotFound(n) => write!(f, "Ambiguous: found {n} possible gold fields; refusing to edit."),
            Error::RenownNotFound(0) => write!(f, "Could not locate the renown field in this save."),
            Error::RenownNotFound(n) => write!(f, "Ambiguous: found {n} possible renown fields; refusing to edit."),
            Error::ParseError(s) => write!(f, "Save structure error: {s}"),
            Error::TooLarge(s) => write!(f, "Unsupported edit: {s}"),
        }
    }
}
impl std::error::Error for Error {}

/// Read a little-endian C-string of fixed width, trimming at the first NUL.
fn read_str(buf: &[u8], off: usize, max: usize) -> String {
    let slice = &buf[off..(off + max).min(buf.len())];
    let end = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
    String::from_utf8_lossy(&slice[..end]).into_owned()
}

/// Decode a protobuf varint at `i`. Returns (value, new_index, byte_len).
fn read_varint(buf: &[u8], mut i: usize) -> Option<(u64, usize, usize)> {
    let start = i;
    let mut shift = 0u32;
    let mut val = 0u64;
    loop {
        if i >= buf.len() || shift > 63 {
            return None;
        }
        let b = buf[i];
        val |= ((b & 0x7f) as u64) << shift;
        i += 1;
        if b & 0x80 == 0 {
            return Some((val, i, i - start));
        }
        shift += 7;
    }
}

/// Encode a value as a (minimal) protobuf varint.
fn encode_varint(mut v: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            out.push(b | 0x80);
        } else {
            out.push(b);
            break;
        }
    }
    out
}

/// A loaded save: raw bytes, header-derived layout, chunk table, and the
/// fully decompressed protobuf payload.
pub struct SaveFile {
    pub raw: Vec<u8>,
    pub total: usize,
    pub nchunks: usize,
    pub data_start: usize,
    /// Offset of the chunk table (tag + 0x20). Differs by format version
    /// (0x490 in v7, 0xc90 in v8), so it's derived, not hardcoded.
    pub table_off: usize,
    /// Offset of Summary.CompressedSize (u32 LE, tag + 0x11).
    pub summary_off: usize,
    /// Offset of the byte-shifted total-uncompressed-size copy (u32 LE, tag + 0x19).
    pub totalcopy_off: usize,
    /// (comp_size, uncomp_size, file_offset_of_compressed_data)
    pub chunks: Vec<(usize, usize, usize)>,
    pub decompressed: Vec<u8>,
    pub display_name: String,
    pub village: String,
    pub character: String,
}

/// Location of the gold field within the decompressed payload.
pub struct GoldLoc {
    pub tag_off: usize,   // offset of the field-6 tag byte (0x30)
    pub value_off: usize, // offset of the first varint byte
    pub byte_len: usize,  // current varint byte length
    pub value: u64,
}

/// Location of the renown field within the decompressed payload.
pub struct RenownLoc {
    pub value_off: usize, // offset of the first varint byte of the renown value
    pub byte_len: usize,  // current varint byte length
    pub value: u64,
}

impl SaveFile {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let raw = std::fs::read(path).map_err(|e| Error::Io(e.to_string()))?;
        if raw.len() < 0x500 || &raw[0..4] != b"VSWB" {
            return Err(Error::BadFile("missing VSWB magic".into()));
        }
        let total = u64::from_le_bytes(raw[0x18..0x20].try_into().unwrap()) as usize;
        if total == 0 || total > 1 << 30 {
            return Err(Error::BadFile("implausible uncompressed size".into()));
        }
        // Locate the SerializeCompressed container by its tag. Its position
        // moved between format versions (v7: 0x470, v8: 0xc70), so search the
        // header region rather than assuming a fixed offset.
        const SEARCH_LIMIT: usize = 0x4000;
        let tag = raw[..SEARCH_LIMIT.min(raw.len())]
            .windows(4)
            .position(|w| w == PACKAGE_FILE_TAG)
            .ok_or_else(|| Error::BadFile("compression container tag not found".into()))?;
        let table_off = tag + 0x20;
        let summary_off = tag + 0x11;
        let totalcopy_off = tag + 0x19;

        let nchunks = total.div_ceil(BLOCK);
        let data_start = table_off + nchunks * 16 + 1;
        if data_start >= raw.len() {
            return Err(Error::BadFile("chunk table overruns file".into()));
        }

        // Parse chunk table + decompress every chunk into one contiguous blob.
        let mut chunks = Vec::with_capacity(nchunks);
        let mut decompressed = Vec::with_capacity(total);
        let mut ext = oozextract::Extractor::new();
        let mut pos = data_start;
        for i in 0..nchunks {
            let b = table_off + i * 16;
            let comp = (raw[b + 1] as usize)
                | ((raw[b + 2] as usize) << 8)
                | ((raw[b + 3] as usize) << 16);
            let uncomp = (raw[b + 9] as usize)
                | ((raw[b + 10] as usize) << 8)
                | ((raw[b + 11] as usize) << 16);
            if comp == 0 || pos + comp > raw.len() {
                return Err(Error::BadFile(format!("chunk {i} out of bounds")));
            }
            let cd = &raw[pos..pos + comp];
            if cd[0] == 0xCC && comp == uncomp + 2 {
                // already-uncompressed block
                decompressed.extend_from_slice(&cd[2..2 + uncomp]);
            } else {
                let mut d = vec![0u8; uncomp];
                ext.read_from_slice(cd, &mut d)
                    .map_err(|e| Error::Decompress(format!("chunk {i}: {e:?}")))?;
                decompressed.extend_from_slice(&d);
            }
            chunks.push((comp, uncomp, pos));
            pos += comp;
        }
        if decompressed.len() != total {
            return Err(Error::Decompress(format!(
                "reassembled {} bytes, header says {}",
                decompressed.len(),
                total
            )));
        }

        Ok(SaveFile {
            display_name: read_str(&raw, 0x220, 256),
            village: read_str(&raw, 0x320, 256),
            character: read_str(&raw, 0x20, 256),
            raw,
            total,
            nchunks,
            data_start,
            table_off,
            summary_off,
            totalcopy_off,
            chunks,
            decompressed,
        })
    }

    /// Locate the (unique) gold field. The record has the shape
    /// `2a <len> <id bytes> 30 <gold varint> 3a` — a length-delimited field 5
    /// (an id) immediately followed by varint field 6 (gold) and the start of
    /// field 7 (`3a`). This combination is globally unique across observed saves;
    /// we require exactly one match.
    ///
    /// The id's length is read as a varint rather than hardcoded: it was 4 bytes
    /// in save format v7 and grew to 6 bytes in v8 (`2a 04 …` → `2a 06 …`).
    pub fn find_gold(&self) -> Result<GoldLoc, Error> {
        let d = &self.decompressed;
        let mut hits = Vec::new();
        let mut i = 0usize;
        while i + 2 < d.len() {
            if d[i] == 0x2a {
                if let Some((id_len, id_start, _)) = read_varint(d, i + 1) {
                    let tag_off = id_start + id_len as usize;
                    if tag_off < d.len() && d[tag_off] == 0x30 {
                        if let Some((val, j, ln)) = read_varint(d, tag_off + 1) {
                            if j < d.len() && d[j] == 0x3a {
                                hits.push(GoldLoc {
                                    tag_off,
                                    value_off: tag_off + 1,
                                    byte_len: ln,
                                    value: val,
                                });
                            }
                        }
                    }
                }
            }
            i += 1;
        }
        match hits.len() {
            1 => Ok(hits.pop().unwrap()),
            n => Err(Error::GoldNotFound(n)),
        }
    }

    /// Locate the renown record by its **current value**.
    ///
    /// Renown is one entry in a large list of identically-shaped reputation
    /// records; nothing structural is unique to it (the surrounding id, UUID and
    /// timestamps all vary or repeat — see `bellwright_renown/FINDINGS.md`). The
    /// only reliable discriminator is the value, which the player reads off the
    /// character sheet. We look for records of the shape
    ///
    /// `20 89 C8 9B DD 02  22 <len>  08 <id1>  10 <value>  28 BB 0D`
    ///
    /// whose `value == current_value` and require exactly one match. If two
    /// records share the value (possible for round numbers) we refuse rather
    /// than guess; the player can nudge renown in-game to make it unique.
    pub fn find_renown(&self, current_value: u64) -> Result<RenownLoc, Error> {
        let d = &self.decompressed;
        // Constant `record-type` prefix, up to and including the `22` submsg tag.
        const ANCHOR: &[u8] = &[0x20, 0x89, 0xC8, 0x9B, 0xDD, 0x02, 0x22];
        // Constant id2 trailer (field 5 = 1723) immediately after the submessage.
        const ID2: &[u8] = &[0x28, 0xBB, 0x0D];

        let mut hits = Vec::new();
        let mut i = 0usize;
        while i + ANCHOR.len() < d.len() {
            if &d[i..i + ANCHOR.len()] != ANCHOR {
                i += 1;
                continue;
            }
            i += 1; // advance past this anchor for the next search regardless
            let q = i - 1 + ANCHOR.len();
            // Submessage length, then its bounds.
            let Some((sublen, sub_start, _)) = read_varint(d, q) else { continue };
            let sub_end = sub_start + sublen as usize;
            if sub_end + ID2.len() > d.len() {
                continue;
            }
            // Submessage must be exactly `08 <id1> 10 <value>`.
            if d[sub_start] != 0x08 {
                continue;
            }
            let Some((_id1, after_id1, _)) = read_varint(d, sub_start + 1) else { continue };
            if after_id1 >= d.len() || d[after_id1] != 0x10 {
                continue;
            }
            let Some((val, after_val, vl)) = read_varint(d, after_id1 + 1) else { continue };
            // The value must end exactly at the submessage boundary, and the
            // constant id2 trailer must follow.
            if after_val != sub_end || &d[sub_end..sub_end + ID2.len()] != ID2 {
                continue;
            }
            if val == current_value {
                hits.push(RenownLoc {
                    value_off: after_id1 + 1,
                    byte_len: vl,
                    value: val,
                });
            }
        }
        match hits.len() {
            1 => Ok(hits.pop().unwrap()),
            n => Err(Error::RenownNotFound(n)),
        }
    }

    /// Produce a new save (raw bytes) with renown changed from `current_value`
    /// to `new_value`. `current_value` is needed to locate the record (see
    /// [`find_renown`]). Does not write to disk.
    pub fn with_renown(&self, current_value: u64, new_value: u64) -> Result<Vec<u8>, Error> {
        let loc = self.find_renown(current_value)?;
        let new_bytes = encode_varint(new_value);
        let delta_inner = new_bytes.len() as isize - loc.byte_len as isize;

        let mut edits: Vec<(usize, usize, Vec<u8>)> = Vec::new();
        if delta_inner != 0 {
            // `ancestor_chain` keys off the field's *tag* offset; the renown
            // value's `0x10` tag is the byte just before the value varint.
            let chain = self.ancestor_chain(loc.value_off - 1)?;
            let mut delta = delta_inner;
            for &(lp_off, length, lp_len) in chain.iter().rev() {
                let new_len = length as isize + delta;
                if new_len < 0 {
                    return Err(Error::ParseError("negative container length".into()));
                }
                let nb = encode_varint(new_len as u64);
                let lp_delta = nb.len() as isize - lp_len as isize;
                edits.push((lp_off, lp_len, nb));
                delta += lp_delta;
            }
        }
        edits.push((loc.value_off, loc.byte_len, new_bytes));
        edits.sort_by_key(|e| e.0);

        let mut nd = Vec::with_capacity(self.decompressed.len() + 8);
        let mut cur = 0usize;
        for (off, old_len, bytes) in &edits {
            nd.extend_from_slice(&self.decompressed[cur..*off]);
            nd.extend_from_slice(bytes);
            cur = off + old_len;
        }
        nd.extend_from_slice(&self.decompressed[cur..]);

        let total_delta = nd.len() as isize - self.decompressed.len() as isize;
        let new_total = (self.total as isize + total_delta) as usize;
        let emin = edits.first().unwrap().0;
        let emax = edits.iter().map(|(o, l, _)| o + l).max().unwrap();
        let c0 = emin / BLOCK;
        let c1 = (emax - 1) / BLOCK;

        if new_total.div_ceil(BLOCK) != self.nchunks {
            return Err(Error::TooLarge(
                "edit changes chunk count (near a block boundary)".into(),
            ));
        }

        let mut out = self.raw[..self.data_start].to_vec();
        let mut new_table = self.chunks.clone();
        #[allow(clippy::needless_range_loop)]
        for i in c0..=c1 {
            let mut uncomp = self.chunks[i].1;
            if i == c1 { uncomp = (uncomp as isize + total_delta) as usize; }
            new_table[i].1 = uncomp;
        }

        let mut nd_pos = c0 * BLOCK;
        #[allow(clippy::needless_range_loop)]
        for i in 0..self.nchunks {
            if i >= c0 && i <= c1 {
                let uncomp = new_table[i].1;
                let comp = uncomp + 2;
                out.push(0xCC); out.push(0x06);
                out.extend_from_slice(&nd[nd_pos..nd_pos + uncomp]);
                nd_pos += uncomp;
                new_table[i].0 = comp;
                let b = self.table_off + i * 16;
                out[b+1] = (comp & 0xFF) as u8;
                out[b+2] = ((comp >> 8) & 0xFF) as u8;
                out[b+3] = ((comp >> 16) & 0xFF) as u8;
                out[b+9]  = (uncomp & 0xFF) as u8;
                out[b+10] = ((uncomp >> 8) & 0xFF) as u8;
                out[b+11] = ((uncomp >> 16) & 0xFF) as u8;
            } else {
                let (comp, _, pos) = self.chunks[i];
                out.extend_from_slice(&self.raw[pos..pos + comp]);
            }
        }

        out[0x18..0x20].copy_from_slice(&(new_total as u64).to_le_bytes());
        out[self.totalcopy_off..self.totalcopy_off + 4].copy_from_slice(&(new_total as u32).to_le_bytes());
        let sum_comp: usize = new_table.iter().map(|c| c.0).sum();
        out[self.summary_off..self.summary_off + 4].copy_from_slice(&(sum_comp as u32).to_le_bytes());

        verify_renown(&out, &nd)?;
        Ok(out)
    }

    /// Walk the protobuf tree from the root and return the chain of
    /// length-delimited containers (innermost last) whose value range contains
    /// `target` (the gold tag offset). Each item is (len_prefix_offset,
    /// length_value, len_prefix_byte_len).
    fn ancestor_chain(&self, target: usize) -> Result<Vec<(usize, u64, usize)>, Error> {
        let d = &self.decompressed;
        let mut chain = Vec::new();
        // Iterative descent: at each level scan fields until we find the
        // length-delimited field whose value range contains `target`.
        let mut start = 0usize;
        let mut end = d.len();
        'outer: loop {
            let mut i = start;
            while i < end {
                let (tag, after_tag, _) = read_varint(d, i)
                    .ok_or_else(|| Error::ParseError(format!("bad tag @0x{i:x}")))?;
                let wire = (tag & 7) as u8;
                if i == target {
                    // reached the target field's tag; chain complete
                    return Ok(chain);
                }
                match wire {
                    0 => {
                        let (_, ni, _) = read_varint(d, after_tag).ok_or_else(|| {
                            Error::ParseError(format!("bad varint @0x{after_tag:x}"))
                        })?;
                        i = ni;
                    }
                    2 => {
                        let (len, vstart, lblen) = read_varint(d, after_tag).ok_or_else(|| {
                            Error::ParseError(format!("bad len @0x{after_tag:x}"))
                        })?;
                        let vend = vstart + len as usize;
                        if vstart <= target && target < vend {
                            chain.push((vstart - lblen, len, lblen));
                            start = vstart;
                            end = vend;
                            continue 'outer;
                        }
                        i = vend;
                    }
                    5 => i = after_tag + 4,
                    1 => i = after_tag + 8,
                    _ => {
                        return Err(Error::ParseError(format!(
                            "unsupported wire {wire} @0x{i:x}"
                        )))
                    }
                }
                if i > end {
                    return Err(Error::ParseError(format!("overran container @0x{i:x}")));
                }
            }
            // target not found inside any nested container at this level
            return Err(Error::ParseError(
                "target field not reachable by parse".into(),
            ));
        }
    }

    /// Produce a new save (raw bytes) with gold set to `new_value`.
    /// Does not write to disk. Validates internal consistency.
    pub fn with_gold(&self, new_value: u64) -> Result<Vec<u8>, Error> {
        let gold = self.find_gold()?;
        let new_gold_bytes = encode_varint(new_value);
        let delta_inner = new_gold_bytes.len() as isize - gold.byte_len as isize;

        // Build the edited decompressed payload.
        // Collect edits as (offset, old_len, new_bytes). Non-overlapping.
        let mut edits: Vec<(usize, usize, Vec<u8>)> = Vec::new();

        if delta_inner != 0 {
            // Field grew/shrank: fix every enclosing container's length prefix.
            let chain = self.ancestor_chain(gold.tag_off)?;
            // Process innermost -> outermost, accumulating size delta.
            let mut delta = delta_inner;
            for &(lp_off, length, lp_len) in chain.iter().rev() {
                let new_len = length as isize + delta;
                if new_len < 0 {
                    return Err(Error::ParseError("negative container length".into()));
                }
                let nb = encode_varint(new_len as u64);
                let lp_delta = nb.len() as isize - lp_len as isize;
                edits.push((lp_off, lp_len, nb));
                delta += lp_delta;
            }
        }
        edits.push((gold.value_off, gold.byte_len, new_gold_bytes));
        edits.sort_by_key(|e| e.0);

        // Splice edits into a fresh decompressed buffer.
        let mut nd = Vec::with_capacity(self.decompressed.len() + 8);
        let mut cur = 0usize;
        for (off, old_len, bytes) in &edits {
            nd.extend_from_slice(&self.decompressed[cur..*off]);
            nd.extend_from_slice(bytes);
            cur = off + old_len;
        }
        nd.extend_from_slice(&self.decompressed[cur..]);

        let total_delta = nd.len() as isize - self.decompressed.len() as isize;
        let new_total = (self.total as isize + total_delta) as usize;

        // Which original chunks does the edited region touch?
        let emin = edits.first().unwrap().0;
        let emax = edits
            .iter()
            .map(|(off, old_len, _)| off + old_len)
            .max()
            .unwrap();
        let c0 = emin / BLOCK;
        let c1 = (emax - 1) / BLOCK;

        // Keep chunk count stable (true for realistic gold edits). If the last
        // touched chunk would overflow a block, bail rather than corrupt.
        if new_total.div_ceil(BLOCK) != self.nchunks {
            return Err(Error::TooLarge(
                "edit changes chunk count (near a block boundary)".into(),
            ));
        }

        // Rebuild the file: header + table + chunk data.
        let mut out = self.raw[..self.data_start].to_vec();

        // New per-chunk uncompressed sizes: chunks [c0,c1) keep their original
        // size; chunk c1 absorbs the whole delta so trailing chunks stay aligned.
        let mut new_table = self.chunks.clone(); // (comp, uncomp, _)
        #[allow(clippy::needless_range_loop)]
        for i in c0..=c1 {
            let mut uncomp = self.chunks[i].1;
            if i == c1 {
                uncomp = (uncomp as isize + total_delta) as usize;
            }
            new_table[i].1 = uncomp;
        }

        // Emit chunk data. Unaffected chunks: copy original compressed bytes.
        // Affected chunks [c0,c1]: emit uncompressed (0xCC 0x06 + raw) from `nd`.
        let mut nd_pos = c0 * BLOCK; // start of affected region in edited payload
        #[allow(clippy::needless_range_loop)]
        for i in 0..self.nchunks {
            if i >= c0 && i <= c1 {
                let uncomp = new_table[i].1;
                let comp = uncomp + 2;
                out.push(0xCC);
                out.push(0x06);
                out.extend_from_slice(&nd[nd_pos..nd_pos + uncomp]);
                nd_pos += uncomp;
                new_table[i].0 = comp;
                let b = self.table_off + i * 16;
                out[b + 1] = (comp & 0xFF) as u8;
                out[b + 2] = ((comp >> 8) & 0xFF) as u8;
                out[b + 3] = ((comp >> 16) & 0xFF) as u8;
                out[b + 9] = (uncomp & 0xFF) as u8;
                out[b + 10] = ((uncomp >> 8) & 0xFF) as u8;
                out[b + 11] = ((uncomp >> 16) & 0xFF) as u8;
            } else {
                let (comp, _, pos) = self.chunks[i];
                out.extend_from_slice(&self.raw[pos..pos + comp]);
            }
        }

        // Update total uncompressed size: 0x18 (u64) and byte-shifted copy at 0x489 (u32).
        out[0x18..0x20].copy_from_slice(&(new_total as u64).to_le_bytes());
        out[self.totalcopy_off..self.totalcopy_off + 4].copy_from_slice(&(new_total as u32).to_le_bytes());

        // Update Summary.CompressedSize (0x481) = sum of all chunk comp sizes.
        let sum_comp: usize = new_table.iter().map(|c| c.0).sum();
        out[self.summary_off..self.summary_off + 4].copy_from_slice(&(sum_comp as u32).to_le_bytes());

        // Self-check: reload and verify gold reads back correctly.
        verify(&out, new_value)?;
        Ok(out)
    }
}

/// Reload a freshly built save from memory and confirm gold == expected.
fn verify(bytes: &[u8], expected: u64) -> Result<(), Error> {
    // Write to a temp-free in-memory reparse by reusing load logic on a slice.
    let tmp = std::env::temp_dir().join(format!("bw_verify_{}.tmp", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(|e| Error::Io(e.to_string()))?;
    let res = SaveFile::load(&tmp).and_then(|s| s.find_gold().map(|g| g.value));
    let _ = std::fs::remove_file(&tmp);
    match res {
        Ok(v) if v == expected => Ok(()),
        Ok(v) => Err(Error::ParseError(format!(
            "post-write verify read {v}, expected {expected}"
        ))),
        Err(e) => Err(e),
    }
}

/// Verify a freshly built renown save by reloading it and confirming the
/// decompressed payload exactly matches what we spliced. This is
/// value-independent (the renown finder needs a value to locate the record, so
/// we can't re-find by value robustly — and don't need to: the new value is
/// guaranteed correct by construction of `expected`).
fn verify_renown(bytes: &[u8], expected: &[u8]) -> Result<(), Error> {
    let tmp = std::env::temp_dir().join(format!("bw_verify_renown_{}.tmp", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(|e| Error::Io(e.to_string()))?;
    let res = SaveFile::load(&tmp);
    let _ = std::fs::remove_file(&tmp);
    let s = res?;
    if s.decompressed == expected {
        Ok(())
    } else {
        Err(Error::ParseError(
            "post-write verify: reloaded payload differs from edit".into(),
        ))
    }
}

fn bak_path(path: &Path) -> std::path::PathBuf {
    path.with_extension(format!(
        "{}.bak",
        path.extension().and_then(|e| e.to_str()).unwrap_or("sav")
    ))
}

fn ensure_backup(path: &Path) -> Result<(), Error> {
    let bak = bak_path(path);
    if !bak.exists() {
        std::fs::copy(path, &bak).map_err(|e| Error::Io(e.to_string()))?;
    }
    Ok(())
}

/// Convenience: write `new_value` into the save at `path`, creating a one-time
/// `<path>.bak` backup of the original if one doesn't already exist.
pub fn set_gold_on_disk<P: AsRef<Path>>(path: P, new_value: u64) -> Result<(), Error> {
    let path = path.as_ref();
    let out = SaveFile::load(path)?.with_gold(new_value)?;
    ensure_backup(path)?;
    std::fs::write(path, &out).map_err(|e| Error::Io(e.to_string()))?;
    Ok(())
}

/// Convenience: change renown from `current_value` to `new_value` in the save
/// at `path` (current value is needed to locate the record), creating a one-time
/// `<path>.bak` backup if one doesn't already exist.
pub fn set_renown_on_disk<P: AsRef<Path>>(
    path: P,
    current_value: u64,
    new_value: u64,
) -> Result<(), Error> {
    let path = path.as_ref();
    let out = SaveFile::load(path)?.with_renown(current_value, new_value)?;
    ensure_backup(path)?;
    std::fs::write(path, &out).map_err(|e| Error::Io(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{encode_varint, read_varint};

    #[test]
    fn varint_roundtrip() {
        for &v in &[
            0u64,
            1,
            127,
            128,
            316,
            2000,
            16383,
            16384,
            20000,
            999999,
            123456,
            u32::MAX as u64,
        ] {
            let bytes = encode_varint(v);
            let (decoded, consumed, len) = read_varint(&bytes, 0).expect("decode");
            assert_eq!(decoded, v, "value {v}");
            assert_eq!(consumed, bytes.len());
            assert_eq!(len, bytes.len());
        }
    }

    #[test]
    fn varint_byte_widths() {
        assert_eq!(encode_varint(316), vec![0xbc, 0x02]); // 2 bytes
        assert_eq!(encode_varint(2000), vec![0xd0, 0x0f]); // 2 bytes
        assert_eq!(encode_varint(20000), vec![0xa0, 0x9c, 0x01]); // 3 bytes
        assert_eq!(encode_varint(50), vec![0x32]); // 1 byte
    }

    #[test]
    fn truncated_varint_is_none() {
        assert!(read_varint(&[0x80], 0).is_none()); // continuation bit set, no more bytes
    }
}
