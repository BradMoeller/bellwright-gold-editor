# Bellwright Save File Format

How to edit a Bellwright save file — specifically, the player's **gold**.

Gold is stored as **protobuf field 6** (`30 <varint>`) at **decompressed offset `0xa6c7f`** (chunk 5, in-chunk `0x6c7f`). Change that varint, fix up the surrounding sizes (below), and the game loads the new amount. Verified working: 316 → 2000 → 20000.

---

## Overview

Bellwright uses a proprietary save format called **VSWB**. Save files live at:
```
/mnt/storage/Games/SteamLibrary/steamapps/compatdata/1812450/pfx/drive_c/users/steamuser/
AppData/Local/Bellwright/Saved/SaveGames/76561197963223709/
```

Files are named `Klint_<slot>.sav` where slot is `0`, `1`, `2`, `quick`, `auto`, `auto_today`, `auto_yesterday`. The in-game display name and village are stored **inside** the file, not in the filename.

The save being edited: `Klint_1.sav` — in-game name "TEST!", village "Krakon", character "Klint". `Klint_1.sav.bak` is the pristine original (patch from this for clean lineage).

---

## File Structure

```
Offset    Size    Description
──────────────────────────────────────────────────────
0x0000    4       Magic: "VSWB"
0x0004    4       Version: 7 (LE uint32)
0x0008    8       Timestamp (Unix LE uint64)
0x0010    8       Another timestamp (Unix LE uint64)
0x0018    8       Total uncompressed size (LE uint64)        ← update if size changes
0x0020    256     Character name ("Klint", null-padded)
0x0120    256     Map name ("Karvenia_08", null-padded)
0x0220    256     Save slot display name ("TEST!", null-padded)
0x0320    256     Village name ("Krakon", null-padded)
0x0420    8       Some data (non-zero)
0x0428    56      Zeros
0x0460    8       Zeros
0x0468    8       Value = 4 (LE uint64)
0x0470    16      Compression header (constant; UE PACKAGE_FILE_TAG + 131072 block size):
                    c1 83 2a 9e 22 22 22 22 00 00 02 00 00 00 00 00
0x0480    1       Prefix byte: 0x02
0x0481    4       Summary.CompressedSize (LE uint32)          ← update if any comp_size changes
0x0485    3       Zeros
0x0488    8       Total uncompressed size, byte-shifted copy  ← update if size changes
                    (i.e. u32 total at 0x0489)
0x0490    N*16    Chunk table: N entries × 16 bytes (N = chunk count, see below)
...       1       Global prefix byte: 0x00 (immediately after the table)
...       ~6.2MB  Compressed chunk data
```

This is UE's standard `FArchive::SerializeCompressed` container (hence `PACKAGE_FILE_TAG` `c1 83 2a 9e` and the load-time check below).

### Chunk count is variable per save

Do **not** assume a fixed count or data offset. Derive both from the header:
```
nchunks    = ceil( u64@0x18 / 131072 )
data_start = 0x490 + nchunks*16 + 1     # +1 for the global prefix byte
```
(e.g. 227 chunks for the big saves, 225 for smaller ones.) Hardcoding `0x12c1` silently produces garbage.

### Summary.CompressedSize (0x481) — CRITICAL

LE uint32 = **sum of all chunk compressed sizes**. The game verifies it on load and crashes otherwise:
```
Fatal error: Archive SerializedCompressed TotalChunkCompressedSize (X) != Summary.CompressedSize (Y)
```
Update it whenever any chunk's compressed size changes.

---

## Chunk Table (0x490)

N entries × 16 bytes. Each entry = two 8-byte slots:
```
Bytes 0–7:  [0x00] [comp_size:   3 bytes LE] [0x00 0x00 0x00 0x00]
Bytes 8–15: [0x00] [uncomp_size: 3 bytes LE] [0x00 0x00 0x00 0x00]
```
- All chunks except the last are 128 KB blocks (`uncomp_size = 131072`).
- Last chunk is the partial remainder.
- The loader decompresses each chunk into its own table-declared `uncomp_size` (advancing the destination pointer per chunk), so a chunk does **not** have to be exactly 131072 — see the "grow" case below.

---

## Compression: Oodle Kraken

Each chunk is independently compressed with **Oodle Kraken** (statically linked in `BellwrightGame-Win64-Shipping.exe`; no separate DLL). A chunk's data starts at the file offset found by summing `comp_size` from `data_start`.

### Compressed block format
```
Byte 0:    0x8C        Kraken, restart_decoder=1, uncompressed=0
Byte 1:    0x06        decoder_type=6 (Kraken), use_checksums=0
Bytes 2–4: 3-byte big-endian quantum header
           compressed_quantum_size = (uint24_be & 0x3FFFF) + 1
Bytes 5+:  compressed_quantum_size bytes of Kraken data
```

### Uncompressed block format (used when patching)
```
Byte 0:    0xCC        (= 0x8C | 0x40, uncompressed flag)
Byte 1:    0x06
Bytes 2+:  raw uncompressed data (uncomp_size bytes, no quantum header)
```
`comp_size = 2 + uncomp_size`. The game's Oodle decoder handles these natively.

### Tooling
```bash
cargo install oozextract   # v0.5.4 — used as a Rust library:
                           # oozextract::Extractor::new().read_from_slice(src, &mut dst)
```
Working Rust decompressor/patcher: `/tmp/bellwright_decomp/` (Cargo project).

---

## Decompressed Data Format

The ~29.7 MB decompressed blob is **binary protobuf** — custom to Bellwright (no GVAS magic; not UE GVAS).

Top level:
```
Field 1 (string):  "Klint"         player name
Field 2 (string):  "Karvenia_08"   map name
Field 3+ (bytes):  large nested messages = full game state
```

### Varint encoding
```
varint(n) = [n]                              if n ≤ 127
          = [(n & 0x7F)|0x80, n >> 7]        if 128 ≤ n ≤ 16383   (2 bytes)
          = 3 bytes                          if n ≥ 16384
```
| Value | Varint |
|-------|--------|
| 316   | `bc 02` |
| 2000  | `d0 0f` |
| 16383 | `ff 7f` (max 2-byte) |
| 20000 | `a0 9c 01` |

---

## Editing Gold

Gold = protobuf field 6 at decompressed offset **`0xa6c7f`** = chunk **5**, in-chunk **`0x6c7f`**:
```
... 2a 04 e9 52 dd 57 | 30 <gold varint> | 3a 18 08 ...
                         ^tag field6, wire0
```

### Case A — new value fits in the same number of varint bytes (≤ 16383)

e.g. 316 → 2000 (both 2-byte). No size change; edit is trivial:
1. Derive `nchunks` / `data_start` from the header.
2. Decompress chunk 5.
3. Overwrite the 2 varint bytes at in-chunk `0x6c7f`.
4. Re-emit chunk 5 as an uncompressed block (`CC 06` + 131072 bytes); `comp_size = 131074`.
5. Update chunk 5's table entry `comp_size`.
6. Update `Summary.CompressedSize` (0x481): `new = old − old_comp5 + 131074`.
7. Write the file.

### Case B — new value needs MORE varint bytes (≥ 16384)

e.g. 2000 → 20000 (2-byte → 3-byte): the field grows by 1 byte, so the enclosing protobuf must stay consistent. The gold field is nested **5 length-delimited containers deep**, and **all five ancestor length-prefixes sit inside chunk 5** alongside the gold byte, so the whole +1 edit is contained in one chunk.

Steps (in addition to Case A):
1. In chunk 5, increment each ancestor length-prefix varint by the byte delta (+1). For TEST! these are at in-chunk offsets and current values:

   | in-chunk offset | length |
   |-----------------|--------|
   | `0x6b5e` | 6437 |
   | `0x6b83` | 6400 |
   | `0x6b88` | 6393 |
   | `0x6b8d` | 6388 |
   | `0x6b92` | 6383 |

   All are 2-byte varints far below 16383, so +1 keeps them 2 bytes (no cascade). To find this chain generically: recursive-descent the protobuf and record every wiretype-2 container whose value range includes the gold tag offset.
2. Splice the new varint in at `0x6c7f` (chunk 5 content becomes 131073 bytes).
3. Re-emit chunk 5 uncompressed (`CC 06` + 131073 bytes); `comp_size = 131075`, `uncomp_size = 131073`. Update both fields in chunk 5's table entry.
4. Update total uncompressed size **in both places**: `0x18` (u64) and the byte-shifted copy at `0x489` (u32 LE), each +1.
5. Update `Summary.CompressedSize` (0x481).
6. Write the file.

### Validation (do this before loading the game)

- Reassemble using each chunk's declared `uncomp_size`; total must equal `u64@0x18`.
- Σ(chunk `comp_size`) must equal `Summary.CompressedSize` (0x481).
- Parse the protobuf top-down from offset 0; it should reach the gold field reading the new value, with every container length consistent and no overruns.

---

## Notes

- The game verifies `Summary.CompressedSize` but does **not** verify per-chunk checksums or any hash of the decompressed data.
- Patch from the pristine `Klint_1.sav.bak` and back up the current `Klint_1.sav` first.
- Saves under this directory share character "Klint" but may be different playthroughs (e.g. "SDF"/Bradford vs "TEST!"/Krakon) — check the display name (0x220) and village (0x320).
