# Changelog

## v2.2.0 (2026-06-22)

### Spec v3.2: Nested Object Flattening

- Encoder automatically flattens fixed-shape nested objects into `>` path column names (e.g., `"customer>name"` instead of `^` + `.customer {}` attachment)
- Decoder reconstructs nested objects from `>` path columns
- 20-48% fewer tokens on deeply nested API data (Jira, Stripe, K8s, calendar events)
- 100% comprehension on every frontier model (validated across 9 models, 7 providers)
- Zero regression on lossless round-trips (50M across JSON, YAML, MessagePack, CSV)
- Falls back to attachment mechanism for: variable-length arrays, objects with different keys across rows, objects with `>` in key names, empty nested objects
- Configurable fuzz iterations via `FUZZ_ITERATIONS` env var

## v2.1.0 (2026-06-14)

### Spec v3.1

- `tool` field in graph profile header is now optional (SHOULD be present for MCP, not required)
- Removed `MissingTool` error variant from `DecodeError` enum

### Bug Fixes

- Quote strings containing commas (conformance: `inline-schema/006_inline_with_quoted_values`)

## v2.0.0 (2026-06-12)

### Breaking Changes

- `encode_generic` now produces inline schema format (not backwards compatible with v1.x decoders)
- Attachment lines no longer indented (same depth as parent row)
- Inline object fields use positional encoding without field-name prefix

### New Features

- Inline object schema: objects with 3+ scalar fields encoded positionally with `^{fields}` header
- Shared array schemas: identical nested arrays omit `{fields}` after first row
- 472M+ fuzz iterations across all 6 implementations, zero failures

### Bug Fixes

- Quote strings starting with `.` (dot prefix)
- Quote C1 control characters (U+0080-U+009F)
- Quote Unicode whitespace (NBSP, hair space, etc.)

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
