# Changelog

## v1.0.1 (2026-06-10)

- CLI: `encode`, `decode`, `encode-generic`, `decode-generic` subcommands
- Both graph and generic profiles supported from the command line

## v1.0.0 (2026-06-10)

SPEC v2.0 implementation. 125/133 conformance fixtures passing (8 skipped: session, delta, binary UTF-8, negative zero, graph encode). 40M property-based round-trips with zero failures.

### Breaking changes from v0.5.0

- `encode_generic` emits `GCF profile=generic` header
- `decode_generic` requires `GCF profile=` header
- Strings colliding with typed literals are quoted
- Full JSON string escaping and number grammar
- `-` for null, `~` for absent, `^` for nested attachments
- `##! summary` trailer replaces `## _summary`
- Graph encoder emits `profile=graph`
- `serde_json` now uses `preserve_order` feature for insertion-order keys
- Added `regex` dependency for scalar grammar

### New

- `scalar.rs`: common scalar grammar (quoting, escaping, parsing, number formatting)
- Conformance test runner (133 fixtures)
- Property-based round-trip tests (40M verified, configurable via `GCF_ITERATIONS`)

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
