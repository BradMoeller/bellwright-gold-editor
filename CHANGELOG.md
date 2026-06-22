# Changelog

All notable changes to this project are documented here. This project adheres to
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Fixed
- **Save format v8 support** (post-patch saves). A game patch bumped the `VSWB`
  version 7→8 and shifted the entire `SerializeCompressed` container +0x800 bytes
  (chunk table `0x490`→`0xc90`, Summary.CompressedSize `0x481`→`0xc81`, total-size
  copy `0x489`→`0xc89`). The loader now locates the container by its
  `PACKAGE_FILE_TAG` and derives every offset relative to it, so it loads v7 and
  v8 alike. Previously v8 saves failed with "chunk 0 out of bounds".
- **Relocated gold field.** The gold record's field-5 id grew from 4 to 6 bytes
  (`2a 04 …` → `2a 06 …`), so the old fixed-offset signature missed it ("Could not
  locate the gold field"). The id length is now read as a varint, matching both
  formats.

### Added
- **Renown editing** (GUI + `bellwright-gold-cli set-renown <save> <current> <new>`).
  Renown is one of thousands of identically-shaped reputation records with no
  unique structural marker, so it's located by matching the current value the
  player supplies; ambiguous matches are refused rather than guessed.
- Investigation notes in `bellwright_renown/FINDINGS.md` (outside this repo).

### Changed
- Renown writes verify by reloading and comparing the full decompressed payload
  (value-independent), instead of re-finding by value.

## [1.0.0] - 2026-06-04

### Added
- Cross-platform desktop GUI (egui/eframe) to view and edit player gold.
- Headless CLI (`bellwright-gold-cli info` / `set`).
- Automatic gold location via a globally-unique protobuf signature — no offsets.
- One-time `<file>.bak` backup on first edit.
- In-memory verification of every write before the file is replaced.
- Handles values of any varint width (1–5 bytes), fixing enclosing protobuf
  length-prefixes and container/`Summary.CompressedSize` fields as needed.
- Reverse-engineered save-format documentation (`docs/save_format.md`).
