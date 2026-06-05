# Changelog

## v0.2.0 (2026-06-05)

- **Breaking**: `encode()` now emits `edges=N` in header line
- **Breaking**: `encode()` now emits `## edges [N]` section header (was `## edges`)
- `decode()` updated to parse `## edges [N]` format (strips bracket suffix)
- Session encoder updated to emit new edge count format

## v0.1.0 (2026-06-04)

- Initial release
- `encode` / `decode`: full GCF round-trip
- `encode_with_session`: session deduplication
- `encode_delta`: delta encoding
- `encode_generic`: tabular profile encoding
- Thread-safe `Session` type
- 16 kind abbreviations
- Zero dependencies, published to crates.io as `gcf`
