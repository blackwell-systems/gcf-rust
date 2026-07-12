//! GCF (Graph Compact Format) encoder and decoder for Rust.
//!
//! GCF is a compact, text-only, graph-native wire format designed for MCP tool
//! responses. It exploits referential identity (local IDs), graph topology
//! (edges as references), and hierarchical grouping (distance-based sections)
//! to achieve significant token savings over JSON while remaining human-readable.
//!
//! Specification: <https://github.com/blackwell-systems/gcf>
//!
//! # Quick Start
//!
//! ```
//! use gcf::{Payload, Symbol, Edge, encode, decode};
//!
//! let p = Payload {
//!     tool: "context_for_task".to_string(),
//!     token_budget: 5000,
//!     tokens_used: 1847,
//!     pack_root: String::new(),
//!     symbols: vec![
//!         Symbol {
//!             qualified_name: "pkg.AuthMiddleware".to_string(),
//!             kind: "function".to_string(),
//!             score: 0.78,
//!             provenance: "lsp_resolved".to_string(),
//!             distance: 0,
//!             signature: String::new(),
//!             components: Default::default(),
//!         },
//!     ],
//!     edges: vec![],
//! };
//!
//! let output = encode(&p);
//! let decoded = decode(&output).unwrap();
//! assert_eq!(decoded.tool, "context_for_task");
//! ```

pub mod decode;
pub mod decode_generic;
pub mod delta;
pub mod encode;
pub mod generic;
pub mod generic_delta;
pub mod scalar;
pub mod session;
pub mod stream;
pub mod stream_generic;
pub mod types;

pub use decode::{decode, DecodeError};
pub use decode_generic::decode_generic;
pub use delta::encode_delta;
pub use encode::encode;
pub use generic::{encode_generic, encode_generic_with_options, GenericOptions};
pub use generic_delta::{
    canonical_cell, decode_generic_delta, decode_generic_full, diff_generic_sets,
    encode_generic_delta, encode_generic_full, generic_pack_root, verify_generic_delta,
    GenericDeltaPayload, GenericSet,
};
pub use session::{encode_with_session, Session};
pub use stream::{StreamEncoder, StreamOptions};
pub use stream_generic::GenericStreamEncoder;
pub use types::{Components, DeltaPayload, Edge, Payload, Symbol};

use std::collections::HashMap;
use std::sync::LazyLock;

/// Map from full kind names to short GCF abbreviations.
static KIND_ABBREV_MAP: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    m.insert("function", "fn");
    m.insert("type", "type");
    m.insert("method", "method");
    m.insert("interface", "iface");
    m.insert("var", "var");
    m.insert("const", "const");
    m.insert("resource", "resource");
    m.insert("table", "table");
    m.insert("class", "class");
    m.insert("selector", "selector");
    m.insert("field", "field");
    m.insert("route_handler", "route");
    m.insert("external", "ext");
    m.insert("file", "file");
    m.insert("package", "pkg");
    m.insert("service", "svc");
    m
});

/// Map from short GCF abbreviations to full kind names.
static KIND_EXPAND_MAP: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    m.insert("fn", "function");
    m.insert("type", "type");
    m.insert("method", "method");
    m.insert("iface", "interface");
    m.insert("var", "var");
    m.insert("const", "const");
    m.insert("resource", "resource");
    m.insert("table", "table");
    m.insert("class", "class");
    m.insert("selector", "selector");
    m.insert("field", "field");
    m.insert("route", "route_handler");
    m.insert("ext", "external");
    m.insert("file", "file");
    m.insert("pkg", "package");
    m.insert("svc", "service");
    m
});

/// Returns the GCF abbreviation for a kind, or the original string if no abbreviation exists.
pub fn kind_abbrev(kind: &str) -> String {
    KIND_ABBREV_MAP
        .get(kind)
        .map(|s| s.to_string())
        .unwrap_or_else(|| kind.to_string())
}

/// Returns the expanded kind for a GCF abbreviation, or the original string if not recognized.
pub fn kind_expand(abbrev: &str) -> String {
    KIND_EXPAND_MAP
        .get(abbrev)
        .map(|s| s.to_string())
        .unwrap_or_else(|| abbrev.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kind_abbrev() {
        assert_eq!(kind_abbrev("function"), "fn");
        assert_eq!(kind_abbrev("interface"), "iface");
        assert_eq!(kind_abbrev("route_handler"), "route");
        assert_eq!(kind_abbrev("external"), "ext");
        assert_eq!(kind_abbrev("package"), "pkg");
        assert_eq!(kind_abbrev("service"), "svc");
        assert_eq!(kind_abbrev("unknown_kind"), "unknown_kind");
    }

    #[test]
    fn test_kind_expand() {
        assert_eq!(kind_expand("fn"), "function");
        assert_eq!(kind_expand("iface"), "interface");
        assert_eq!(kind_expand("route"), "route_handler");
        assert_eq!(kind_expand("ext"), "external");
        assert_eq!(kind_expand("pkg"), "package");
        assert_eq!(kind_expand("svc"), "service");
        assert_eq!(kind_expand("unknown_abbrev"), "unknown_abbrev");
    }

    #[test]
    fn test_abbrev_expand_roundtrip() {
        for (full, abbrev) in KIND_ABBREV_MAP.iter() {
            assert_eq!(kind_expand(abbrev), *full);
            assert_eq!(kind_abbrev(full), *abbrev);
        }
    }
}
