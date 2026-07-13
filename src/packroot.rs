//! Graph pack root: content-addressed SHA-256 hash of a graph snapshot.
//!
//! Mirrors the gcf-pack-root-v1 algorithm (SPEC Section 10.2) exactly. Two
//! implementations given the same logical graph MUST produce the same result.

use crate::generic_delta::sha256_hex;
use crate::kind_abbrev;
use crate::scalar::format_number;
use crate::types::{Edge, Symbol};
use std::collections::HashMap;

/// Format a symbol score in the canonical shortest-decimal form (e.g. `0.9`,
/// `1`), reusing the same number formatter as the generic encoder and generic
/// pack root. This is the pre-hash canonical form, NOT the wire's 2-decimal form.
fn format_score(score: f64) -> String {
    match serde_json::Number::from_f64(score) {
        Some(n) => format_number(&n),
        // Non-finite scores are not valid GCF; fall back to a stable rendering.
        None => score.to_string(),
    }
}

/// Compute the canonical pack root hash for a graph snapshot.
///
/// Returns `"sha256:" + hex(sha256(canonical_bytes))` where the canonical bytes
/// are built from independently byte-sorted symbol and edge records.
pub fn pack_root(symbols: &[Symbol], edges: &[Edge]) -> String {
    // Build canonical symbol records:
    //   S\t{kindAbbrev}\t{qualifiedName}\t{score}\t{provenance}\t{distance}\n
    let mut sym_records: Vec<String> = Vec::with_capacity(symbols.len());
    for s in symbols {
        let kind = kind_abbrev(&s.kind);
        let score = format_score(s.score);
        sym_records.push(format!(
            "S\t{}\t{}\t{}\t{}\t{}\n",
            kind, s.qualified_name, score, s.provenance, s.distance
        ));
    }

    // Build a qualified-name -> kind-abbrev map so edge endpoints carry the
    // symbol kind (disambiguates same-qname / different-kind symbols).
    let mut sym_kind: HashMap<&str, String> = HashMap::with_capacity(symbols.len());
    for s in symbols {
        sym_kind.insert(s.qualified_name.as_str(), kind_abbrev(&s.kind));
    }

    // Build canonical edge records:
    //   E\t{srcKindAbbrev}\t{source}\t{tgtKindAbbrev}\t{target}\t{edgeType}\n
    let mut edge_records: Vec<String> = Vec::with_capacity(edges.len());
    for e in edges {
        let src_kind = sym_kind.get(e.source.as_str()).cloned().unwrap_or_default();
        let tgt_kind = sym_kind.get(e.target.as_str()).cloned().unwrap_or_default();
        edge_records.push(format!(
            "E\t{}\t{}\t{}\t{}\t{}\n",
            src_kind, e.source, tgt_kind, e.target, e.edge_type
        ));
    }

    // Sort symbol records and edge records INDEPENDENTLY by UTF-8 byte order.
    sym_records.sort();
    edge_records.sort();

    // Concatenate: all symbols then all edges.
    let mut canonical = String::new();
    for r in &sym_records {
        canonical.push_str(r);
    }
    for r in &edge_records {
        canonical.push_str(r);
    }

    format!("sha256:{}", sha256_hex(canonical.as_bytes()))
}

/// Build the exact pre-hash canonical byte string for a graph snapshot. Exposed
/// for conformance verification against fixture `canonicalBytes`.
pub fn pack_root_canonical_bytes(symbols: &[Symbol], edges: &[Edge]) -> String {
    let mut sym_records: Vec<String> = Vec::with_capacity(symbols.len());
    for s in symbols {
        let kind = kind_abbrev(&s.kind);
        let score = format_score(s.score);
        sym_records.push(format!(
            "S\t{}\t{}\t{}\t{}\t{}\n",
            kind, s.qualified_name, score, s.provenance, s.distance
        ));
    }
    let mut sym_kind: HashMap<&str, String> = HashMap::with_capacity(symbols.len());
    for s in symbols {
        sym_kind.insert(s.qualified_name.as_str(), kind_abbrev(&s.kind));
    }
    let mut edge_records: Vec<String> = Vec::with_capacity(edges.len());
    for e in edges {
        let src_kind = sym_kind.get(e.source.as_str()).cloned().unwrap_or_default();
        let tgt_kind = sym_kind.get(e.target.as_str()).cloned().unwrap_or_default();
        edge_records.push(format!(
            "E\t{}\t{}\t{}\t{}\t{}\n",
            src_kind, e.source, tgt_kind, e.target, e.edge_type
        ));
    }
    sym_records.sort();
    edge_records.sort();
    let mut canonical = String::new();
    for r in &sym_records {
        canonical.push_str(r);
    }
    for r in &edge_records {
        canonical.push_str(r);
    }
    canonical
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym(qname: &str, kind: &str, score: f64, prov: &str, distance: i32) -> Symbol {
        Symbol {
            qualified_name: qname.to_string(),
            kind: kind.to_string(),
            score,
            provenance: prov.to_string(),
            distance,
            signature: String::new(),
            components: Default::default(),
        }
    }

    fn edge(source: &str, target: &str, edge_type: &str) -> Edge {
        Edge {
            source: source.to_string(),
            target: target.to_string(),
            edge_type: edge_type.to_string(),
            status: String::new(),
        }
    }

    #[test]
    fn empty_graph() {
        assert_eq!(
            pack_root(&[], &[]),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn single_symbol_integer_score() {
        let symbols = vec![sym("pkg.Main", "function", 1.0, "lsp_resolved", 0)];
        assert_eq!(
            pack_root_canonical_bytes(&symbols, &[]),
            "S\tfn\tpkg.Main\t1\tlsp_resolved\t0\n"
        );
        assert_eq!(
            pack_root(&symbols, &[]),
            "sha256:da39c1fb07913659a75865cf4afaa971683448625d19ac92c0303e90f678585f"
        );
    }

    #[test]
    fn basic_two_symbol_one_edge() {
        let symbols = vec![
            sym("pkg.Auth", "function", 0.78, "lsp_resolved", 0),
            sym("pkg.Server", "function", 0.54, "lsp_resolved", 1),
        ];
        let edges = vec![edge("pkg.Server", "pkg.Auth", "calls")];
        assert_eq!(
            pack_root_canonical_bytes(&symbols, &edges),
            "S\tfn\tpkg.Auth\t0.78\tlsp_resolved\t0\nS\tfn\tpkg.Server\t0.54\tlsp_resolved\t1\nE\tfn\tpkg.Server\tfn\tpkg.Auth\tcalls\n"
        );
        assert_eq!(
            pack_root(&symbols, &edges),
            "sha256:8e6d32973b4005c604399a14faa32799d54f427800008312a3357c349d41e572"
        );
    }

    #[test]
    fn sort_order() {
        let symbols = vec![
            sym("z.Foo", "function", 0.9, "lsp_resolved", 0),
            sym("a.Bar", "type", 0.5, "ast_inferred", 1),
        ];
        let edges = vec![
            edge("z.Foo", "a.Bar", "calls"),
            edge("a.Bar", "z.Foo", "imports"),
        ];
        assert_eq!(
            pack_root_canonical_bytes(&symbols, &edges),
            "S\tfn\tz.Foo\t0.9\tlsp_resolved\t0\nS\ttype\ta.Bar\t0.5\tast_inferred\t1\nE\tfn\tz.Foo\ttype\ta.Bar\tcalls\nE\ttype\ta.Bar\tfn\tz.Foo\timports\n"
        );
        assert_eq!(
            pack_root(&symbols, &edges),
            "sha256:fa9ffe4a5d09fc21e1e122000b269ac9098fbec8d21eee5665ed33711be8af94"
        );
    }
}
