<p align="center">
  <a href="https://github.com/blackwell-systems"><img src="https://raw.githubusercontent.com/blackwell-systems/blackwell-docs-theme/main/badge-trademark.svg" alt="Blackwell Systems"></a>
  <a href="https://github.com/blackwell-systems/gcf-rust/actions"><img src="https://github.com/blackwell-systems/gcf-rust/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License"></a>
  <a href="https://crates.io/crates/gcf"><img src="https://img.shields.io/crates/v/gcf.svg" alt="crates.io"></a>
</p>

# gcf-rust

Rust implementation of [GCF](https://gcformat.com/) -- the most token-efficient wire format for LLMs. A drop-in alternative to JSON and TOON for any structured data.

**100% comprehension on every frontier model tested. 25.5% fewer tokens than TOON, 53% fewer than JSON across 15 datasets. 90.7% on structurally complex code graphs (vs TOON 68.5%, JSON 53.6%). 1,700+ LLM evaluations. Zero training.**

Docs: [gcformat.com](https://gcformat.com/) | [Playground](https://gcformat.com/playground.html) | [GCF vs TOON](https://gcformat.com/guide/vs-toon.html)

## Install

```toml
[dependencies]
gcf = "0.1"
```

Zero-copy where possible. Minimal dependencies (serde, serde_json). Don't want to change code? Use the [MCP proxy](https://github.com/blackwell-systems/gcf-proxy) for zero-code adoption.

## Quick Start

```rust
use gcf::encode_generic;
use serde_json::json;

let data = json!({
    "employees": [
        {"id": 1, "name": "Alice", "department": "Engineering", "salary": 95000},
        {"id": 2, "name": "Bob", "department": "Sales", "salary": 72000},
    ],
});
let output = encode_generic(&data);
```

Output:
```
## employees [2]{department,id,name,salary}
Engineering|1|Alice|95000
Sales|2|Bob|72000
```

Works on any `serde_json::Value`. One header declares field names, rows are positional values.

## Graph Profile

For code graph data with symbols, edges, and distance groups:

```rust
use gcf::{Payload, Symbol, Edge, encode};

let p = Payload {
    tool: "context_for_task".into(), token_budget: 5000, tokens_used: 1847,
    symbols: vec![
        Symbol { qualified_name: "pkg.Auth".into(), kind: "function".into(), score: 0.78, provenance: "lsp".into(), distance: 0, ..Default::default() },
        Symbol { qualified_name: "pkg.Server".into(), kind: "function".into(), score: 0.54, provenance: "lsp".into(), distance: 1, ..Default::default() },
    ],
    edges: vec![Edge { source: "pkg.Server".into(), target: "pkg.Auth".into(), edge_type: "calls".into(), ..Default::default() }],
    ..Default::default()
};
let output = encode(&p);
```

Output:
```
GCF tool=context_for_task budget=5000 tokens=1847 symbols=2 edges=1
## targets
@0 fn pkg.Auth 0.78 lsp
## related
@1 fn pkg.Server 0.54 lsp
## edges [1]
@0<@1 calls
```

## Decode

```rust
use gcf::decode;

let p = decode(input).expect("valid GCF");
println!("{} {} symbols {} edges", p.tool, p.symbols.len(), p.edges.len());
```

## Session Deduplication

Track transmitted symbols across multiple tool responses. Previously-sent symbols become bare references instead of full declarations:

```rust
use gcf::{Session, encode_with_session};

let sess = Session::new();

let out1 = encode_with_session(&payload1, &sess); // full declarations
let out2 = encode_with_session(&payload2, &sess); // reused symbols as "@N  # previously transmitted"
```

By the 5th call in a session: 92.7% token savings vs JSON.

## Streaming Encode

Write GCF output incrementally as symbols and edges arrive. Zero buffering, O(1) memory per row:

```rust
use gcf::{StreamEncoder, StreamOptions, Symbol, Edge};

let enc = StreamEncoder::new(writer, "context_for_task", StreamOptions {
    token_budget: 5000,
    ..Default::default()
});

enc.write_symbol(&Symbol { qualified_name: "pkg.Auth".into(), kind: "function".into(), score: 0.95, provenance: "lsp".into(), distance: 0, ..Default::default() });
enc.write_edge(&Edge { source: "pkg.Server".into(), target: "pkg.Auth".into(), edge_type: "calls".into(), ..Default::default() });
enc.close();
```

Output uses `[?]` deferred counts and `## _summary` trailer. Standard `decode()` handles streaming output with no changes. Thread-safe via Mutex.

## Delta Encoding

When the consumer already has a prior context pack, send only what changed:

```rust
use gcf::{DeltaPayload, Symbol, encode_delta};

let delta = DeltaPayload {
    tool: "context_for_task".to_string(),
    base_root: "aaa111".to_string(),
    new_root: "bbb222".to_string(),
    removed: vec![Symbol {
        qualified_name: "pkg.OldFunc".to_string(),
        kind: "function".to_string(),
        score: 0.0,
        provenance: String::new(),
        distance: 0,
        signature: String::new(),
        components: Default::default(),
    }],
    added: vec![Symbol {
        qualified_name: "pkg.NewFunc".to_string(),
        kind: "function".to_string(),
        score: 0.85,
        provenance: "rwr".to_string(),
        distance: 0,
        signature: String::new(),
        components: Default::default(),
    }],
    removed_edges: vec![],
    added_edges: vec![],
    delta_tokens: 30,
    full_tokens: 200,
};

let output = encode_delta(&delta);
```

81.2% savings on re-queries where the pack changed slightly.

## Generic Encoding

Encode any `serde_json::Value` (not just graph payloads) into GCF tabular format:

```rust
use gcf::encode_generic;
use serde_json::json;

let data = json!({
    "employees": [
        {"id": 1, "name": "Alice", "department": "Engineering", "salary": 95000},
        {"id": 2, "name": "Bob", "department": "Sales", "salary": 72000},
    ],
});
let output = encode_generic(&data);
```

Output:
```
## employees [2]{department,id,name,salary}
Engineering|1|Alice|95000
Sales|2|Bob|72000
```

Works on objects, arrays, and primitives. Arrays of uniform objects get tabular rows. Nested objects use `## key` section headers.

## API

| Function | Description |
|----------|-------------|
| `encode(p: &Payload) -> String` | Encode a graph payload to GCF text |
| `encode_generic(data: &Value) -> String` | Encode any JSON value to GCF tabular format |
| `decode(input: &str) -> Result<Payload, DecodeError>` | Parse GCF text back to a Payload |
| `encode_with_session(p: &Payload, s: &Session) -> String` | Encode with session deduplication |
| `encode_delta(d: &DeltaPayload) -> String` | Encode a delta (added/removed only) |
| `Session::new() -> Session` | Create a new session tracker (thread-safe via Mutex) |

## Types

| Type | Purpose |
|------|---------|
| `Payload` | Full GCF payload: tool, budget, symbols, edges, pack root |
| `Symbol` | Graph node: qualified name, kind, score, provenance, distance |
| `Edge` | Directed relationship: source, target, edge type |
| `DeltaPayload` | Diff between two packs: added/removed symbols and edges |
| `Components` | Score breakdown: blast_radius, confidence, recency, distance |
| `Session` | Thread-safe tracker for multi-call deduplication |
| `DecodeError` | Enum of decode failure modes |

## Benchmarks

1,700+ LLM evaluations across 10 models, 3 providers, and 51 independent test runs.

| | GCF | TOON | JSON |
|---|---|---|---|
| **Comprehension** (23 runs, 10 models) | **90.7%** | 68.5% | 53.6% |
| **Generation** (28 runs, 9 models) | **5/5** | 1.0/5 | 5.0/5 |
| **Input tokens** (500 symbols) | **11,090** | 16,378 | 53,341 |
| **Output tokens** (100 symbols) | **5,976** | 8,937 | 16,121 |

GCF wins 13/15 datasets on the expanded [token efficiency benchmark](https://github.com/blackwell-systems/toon/tree/gcf-comparison). Full results: [gcformat.com/guide/benchmarks](https://gcformat.com/guide/benchmarks.html)

## Links

- [Documentation](https://gcformat.com/)
- [Playground](https://gcformat.com/playground.html)
- [Cost Calculator](https://gcformat.com/calculator.html)
- [Specification](https://github.com/blackwell-systems/gcf) (v3.1 Stable)
- [GCF vs TOON](https://gcformat.com/guide/vs-toon.html)
- [Benchmarks](https://gcformat.com/guide/benchmarks.html) (1,700+ evaluations)

### Ecosystem

| | |
|---|---|
| **Go** | [gcf-go](https://github.com/blackwell-systems/gcf-go) |
| **TypeScript** | [gcf-typescript](https://github.com/blackwell-systems/gcf-typescript) |
| **Python** | [gcf-python](https://github.com/blackwell-systems/gcf-python) |
| **Rust** | [gcf-rust](https://github.com/blackwell-systems/gcf-rust) |
| **Swift** | [gcf-swift](https://github.com/blackwell-systems/gcf-swift) |
| **Kotlin** | [gcf-kotlin](https://github.com/blackwell-systems/gcf-kotlin) |
| **MCP Proxy** | [gcf-proxy](https://github.com/blackwell-systems/gcf-proxy) |
| **VS Code** | [Marketplace](https://marketplace.visualstudio.com/items?itemName=blackwell-systems.gcf-vscode) |
| **Claude Code** | [gcf-claude-plugin](https://github.com/blackwell-systems/gcf-claude-plugin) |
| **Codex CLI** | [gcf-codex-plugin](https://github.com/blackwell-systems/gcf-codex-plugin) |
| **Tree-sitter** | [tree-sitter-gcf](https://github.com/blackwell-systems/tree-sitter-gcf) |

## License

MIT - [Dayna Blackwell](https://github.com/blackwell-systems)
