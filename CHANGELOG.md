# Changelog

All notable changes to this project are documented here. This project adheres to
[Semantic Versioning](https://semver.org/).

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
