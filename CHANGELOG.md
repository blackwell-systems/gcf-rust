# Changelog

## v2.4.0 (2026-07-12)

### Fixes

- The conformance runner now hard-fails on any unhandled operation (instead of silently skipping it) and exercises session, delta, roundtrip, and pack-root fixtures end to end; the graph delta wire decode and verify path is now covered, so no operations remain allow-listed.
- Implemented the graph delta wire decoder and verifier (`decode_delta` / `verify_delta`): parse a `GCF profile=graph delta=true` wire back into removed/added symbols and edge changes, apply them atomically to a base snapshot, recompute `pack_root`, and reject a wrong `new_root` with `root_mismatch` (SPEC 10.4). The `## added` encoder now emits the trailing `distance` field (SPEC 3.4.1, Section 10.1). The shared `graph-delta` fixtures now run end to end: 001 (encode, gains the trailing distance), 002 (verified apply), 003 (`root_mismatch` rejection).
- Added the graph-profile PackRoot (`pack_root`, gcf-pack-root-v1, SPEC 10.2): rust previously lacked it entirely. Byte-identical to the other SDKs (verified against the shared graph-pack-root fixtures' canonical bytes).
- **Session encoding correctness fix.** `encodeWithSession` assigned per-response local IDs instead of stable session-global IDs, so the cross-call dedup references (`@N  # previously transmitted`) pointed at the wrong symbols, and the header emitted zero-valued `budget`/`tokens`/`edges`. Both are fixed to match the reference; graph session output is now byte-identical across all six SDKs. This had gone undetected because the conformance runner skipped the shared graph-session fixtures (now wired).
- Buffered graph encoder: order edges by source ID, then target ID, then edge type (SPEC 16.1), instead of emitting them in input order. Decode-invariant (edges are a set) and does not affect `pack_root` (which sorts edge records independently), so no content addresses change. Pinned by shared fixture `graph-encode/003`. Streaming edges remain in producer-arrival order.
- Decoder: reject an orphan `.field` attachment (a `.field` whose name is neither a `^`-marked column of its row nor a `>`-containing field name, SPEC 7.4.6.1.4) instead of silently absorbing it as an undeclared extra field. Such a stray attachment previously decoded to a record no encoder produces, silently injecting a field onto the last-parsed row (a lossless round-trip hole); now rejected per SPEC 16.5 (`orphan_attachment`).
- Decoder: reject an orphan positional inline body (a pipe-delimited line with no eligible `^{}` attachment-marker cell) instead of silently dropping it. The object-body parser previously skipped any unrecognized line, so a stray positional body (e.g. a second `Bob|b@t.com` after a row's one inline cell was filled) vanished with no error (silent data loss); now rejected per SPEC 16.5 (`orphan_inline_attachment`).
- Graph streaming trailer: the edge count is now always the last `counts` entry, even when the stream has no edges (positional `counts=2,1,0`; labeled `counts=…,edges:0`). A zero-edge stream previously dropped it, violating the SPEC §8.4 / §8.4.1 rule that the edge count is always present and last (the invariant that keeps the positional form unambiguous). The graph trailer is decoder-ignored, so this changes producer output only.

### Streaming: opt-in labeled trailer counts (SPEC §8.4.1)

- New `StreamOptions.labeled_trailer_counts`. When set, the `##! summary` graph streaming trailer emits `counts=` in the labeled form `label:count` per group (e.g. `counts=targets:2,related:1,edges:3`) instead of the default positional values-only form (`counts=2,1,3`). Default false is byte-identical to prior output.
- Opt-in and non-breaking: a producer-side comprehension aid for known weak consumers. The trailer counts remain informational (decoder-ignored) in both forms; neither changes the decoded payload. Mirrors the `gcf-go` reference.

### Conformance and docs

- The conformance runner now executes the `graph-stream-encode` fixtures (streaming-encode parity, previously decode-only): fixture 004 (positional trailer) and 005 (labeled trailer).
- README: corrected the streaming example trailer from the defunct `## _summary … sections=` to the real `##! summary … counts=`; README now leads with the project diagram.

## v2.3.0 (2026-07-12)

### Generic-profile delta encoding (SPEC §10a)

- Full producer + consumer implementation of generic-profile delta, byte-for-byte interoperable with `gcf-go`, `gcf-python`, and `gcf-typescript`:
  - `GenericSet` (keyed record set), `GenericDeltaPayload`
  - `generic_pack_root` (`gcf-pack-root-v1`, generic profile) with a purpose-built cell canonicalization (`canonical_cell`) decoupled from the wire cell encoder: collision-free (null/bool/number bare, strings always quoted) and record-safe. Fields and records sort by UTF-8 byte order (Rust `str` ordering is byte-wise), matching Go's `sort.Strings`.
  - `diff_generic_sets` (the blessed producer path; centralizes the keyed-diff invariants), `encode_generic_full`, `encode_generic_delta`
  - `decode_generic_full`, `decode_generic_delta` (consumer wire parsing)
  - `verify_generic_delta` (atomic apply + `new_root` verification)
- Delta is opt-in and bilateral; the existing `encode_generic` path is unchanged (backward compatible).
- `GenericDeltaSession` producer-side re-anchor helper (SPEC §10a.8, non-normative producer policy): manages the delta/full re-anchor cadence over a stream of generic-profile updates, emitting either a compact delta or, on its chosen cadence, a full re-anchor. Introduces no new wire syntax (every payload is byte-identical to `encode_generic_full` / `encode_generic_delta`). `ReanchorPolicy::fixed_n(n)` (re-anchor every N turns; `DEFAULT_REANCHOR_N = 15`) and `ReanchorPolicy::size_guard()` (re-anchor once cumulative delta bytes reach the current full-payload size); a schema change forces a full (§10a.7). Byte-for-byte identical to the `gcf-go` reference and verified by the shared `generic-delta-session` conformance fixtures.
- SHA-256 is implemented locally (no new dependency; `regex`/`serde` remain the only crates), verified against NIST known-answer vectors and the shared conformance fixtures (which are generated by Go's `crypto/sha256`, so an identical pack root is a cross-implementation correctness proof).

### Fixes

- **Losslessness (multi-byte UTF-8 in quoted strings):** `parse_quoted_string` reconstructed literal characters with `bytes[i] as char`, which reinterprets each UTF-8 byte as Latin-1 and corrupts any multi-byte character inside a quoted string (e.g. `中`/emoji came back as their raw bytes). It only surfaced when a multi-byte character shared a cell with a character that forces quoting (a control char or delimiter), since plain unicode strings are emitted unquoted; every prior test missed that combination. Literal characters are now copied whole (advancing by `char::len_utf8`). Found by the new generic-delta round-trip fuzz.

### Tests

- Generic-delta fuzz (`tests/generic_delta_fuzz.rs`), mirroring `gcf-go`: the decoder never panics on arbitrary/mutated input, and arbitrary UTF-8 string cells survive the full-wire round-trip with the pack root preserved (the property that caught the quoted-string bug above).

- Unit suite mirroring the other SDKs: self-proving round-trip (diff -> encode -> apply -> recomputed root), determinism / row-order invariance, no-type-collision canonicalization, every invariant/error path, full-payload wire round-trip, the complete server -> wire -> consumer end-to-end loop, and malformed-wire-fails-closed.
- Conformance runner support for `generic-pack-root`, `generic-delta`, `generic-delta-verify`, `generic-delta-decode`, and `generic-delta-session` (shared fixtures); produces identical pack roots, delta wire, and re-anchor emissions to `gcf-go`, `gcf-python`, and `gcf-typescript`.
- Session-helper unit suite mirroring `gcf-go`: FixedN cadence pattern, SizeGuard trigger, schema-change-forces-full, exactly-two-fulls over 30 same-schema turns (N=15), and a load-bearing consumer-stays-in-sync check (apply each emission, assert `generic_pack_root` matches the producer state every turn) under both policies.

## v2.2.2 (2026-07-10)

### Fixes

- **Losslessness (nested null):** a nested object that is null at an intermediate level (e.g. `{"meta":{"owner":null}}`) is no longer flattened. Previously its leaves encoded as absent (`~`) and unflattened to a missing key, silently dropping the null. Such fields now fall back to the attachment mechanism; a top-level null still flattens losslessly (emits `-`, reconstructs via the all-null rule). Enforced by the shared conformance fixtures `flatten/017`–`019`. Prototype pollution does not affect Rust (maps have no mutable prototype).

### Tests

- `test_flatten_roundtrip`: aligned arrays whose shared fields are fixed-shape nested objects, with a field or an intermediate nested level sometimes null/absent — the shape the prior scalar-only generator never produced, leaving the flatten/unflatten path unexercised. Verified to fail on the pre-fix encoder and pass on the fix.

## v2.2.1 (2026-06-23)

### Flatten Opt-Out

- Added `GenericOptions` struct with `no_flatten` field to disable nested object flattening
- `encode_generic_with_options(data, &GenericOptions { no_flatten: true })` produces attachment syntax instead of path columns
- Backward compatible: `encode_generic(data)` behavior unchanged (flatten on by default)
- Fixed: field names containing `>` no longer appear as tabular columns (spec rule 7.4.6.1.4)
- Fixed: field names containing `>` no longer eligible for flattening analysis
- Fixed: decoder no longer treats literal `>` in key names as a path separator
- Fixed: decoder accepts orphan attachments (fields excluded from column list)
- Fuzz key generator now includes `>` for adversarial testing

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
