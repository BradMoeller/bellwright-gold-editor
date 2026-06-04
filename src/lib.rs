//! Core logic for reading and editing player gold in Bellwright `.sav` files.
//!
//! Format: a UE `FArchive::SerializeCompressed` container ("VSWB") whose payload
//! is a custom protobuf blob. Player gold is protobuf field 6 inside a uniquely
//! shaped record: `field5(len 4) -> field6(varint)=GOLD -> field7...`, i.e. the
//! byte signature `2a 04 ?? ?? ?? ?? 30 <varint> 3a`, which is globally unique.
//!
//! This module has no GUI dependencies so it can be unit-tested directly.

use std::path::Path;

const BLOCK: usize = 131072; // 128 KiB uncompressed block size
const TABLE_OFF: usize = 0x490;

#[derive(Debug)]
pub enum Error {
    Io(String),
    BadFile(String),
    Decompress(String),
    GoldNotFound(usize), // number of candidate matches found (0 or >1)
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
            Error::GoldNotFound(n) => write!(
                f,
                "Ambiguous: found {n} possible gold fields; refusing to edit."
            ),
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
        let nchunks = total.div_ceil(BLOCK);
        let data_start = TABLE_OFF + nchunks * 16 + 1;
        if data_start >= raw.len() {
            return Err(Error::BadFile("chunk table overruns file".into()));
        }

        // Parse chunk table + decompress every chunk into one contiguous blob.
        let mut chunks = Vec::with_capacity(nchunks);
        let mut decompressed = Vec::with_capacity(total);
        let mut ext = oozextract::Extractor::new();
        let mut pos = data_start;
        for i in 0..nchunks {
            let b = TABLE_OFF + i * 16;
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
            chunks,
            decompressed,
        })
    }

    /// Locate the (unique) gold field. The signature `2a 04 ?? ?? ?? ?? 30 <varint> 3a`
    /// is globally unique across observed saves; we require exactly one match.
    pub fn find_gold(&self) -> Result<GoldLoc, Error> {
        let d = &self.decompressed;
        let mut hits = Vec::new();
        let mut i = 0usize;
        while i + 8 < d.len() {
            if d[i] == 0x2a && d[i + 1] == 0x04 && d[i + 6] == 0x30 {
                if let Some((val, j, ln)) = read_varint(d, i + 7) {
                    if j < d.len() && d[j] == 0x3a {
                        hits.push(GoldLoc {
                            tag_off: i + 6,
                            value_off: i + 7,
                            byte_len: ln,
                            value: val,
                        });
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
                    // reached the gold field itself; chain complete
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
                "gold field not reachable by parse".into(),
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
                let b = TABLE_OFF + i * 16;
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
        out[0x489..0x48d].copy_from_slice(&(new_total as u32).to_le_bytes());

        // Update Summary.CompressedSize (0x481) = sum of all chunk comp sizes.
        let sum_comp: usize = new_table.iter().map(|c| c.0).sum();
        out[0x481..0x485].copy_from_slice(&(sum_comp as u32).to_le_bytes());

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

/// Convenience: write `new_value` into the save at `path`, creating a one-time
/// `<path>.bak` backup of the original if one doesn't already exist.
pub fn set_gold_on_disk<P: AsRef<Path>>(path: P, new_value: u64) -> Result<(), Error> {
    let path = path.as_ref();
    let save = SaveFile::load(path)?;
    let out = save.with_gold(new_value)?;
    let bak = path.with_extension(format!(
        "{}.bak",
        path.extension().and_then(|e| e.to_str()).unwrap_or("sav")
    ));
    if !bak.exists() {
        std::fs::copy(path, &bak).map_err(|e| Error::Io(e.to_string()))?;
    }
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
