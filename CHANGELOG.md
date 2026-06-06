# Changelog

## v0.5.0 (2026-06-05)

- `GenericStreamEncoder`: zero-buffering tabular streaming encode (begin_array/write_row/end_array/write_kv/write_section/write_inline_array)
- `decode_generic`: parse GCF tabular text into `serde_json::Value` (tabular arrays, key-value, nested sections, inline arrays, nested row fields, empty arrays, graph fallback)

## v0.3.0 (2026-06-05)

- `encode_generic`: primitive arrays inlined as `name[N]: val1,val2,val3`

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
