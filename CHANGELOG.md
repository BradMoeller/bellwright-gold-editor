# Changelog

All notable changes to this project are documented here. This project adheres to
[Semantic Versioning](https://semver.org/).

## [Unreleased]

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
