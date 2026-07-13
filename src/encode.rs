use crate::kind_abbrev;
use crate::types::{Payload, Symbol};
use std::collections::HashMap;
use std::fmt::Write;

struct DistanceGroup {
    distance: i32,
    symbols: Vec<Symbol>,
}

fn group_by_distance(symbols: &[Symbol]) -> Vec<DistanceGroup> {
    if symbols.is_empty() {
        return Vec::new();
    }
    // Sort by distance ascending, then score descending within each group
    // (stable sort preserves input order for equal keys), mirroring the Go
    // reference so symbol local IDs are assigned in canonical output order.
    let mut sorted: Vec<Symbol> = symbols.to_vec();
    sorted.sort_by(|a, b| {
        a.distance.cmp(&b.distance).then_with(|| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });
    let mut groups: Vec<DistanceGroup> = Vec::new();
    for s in sorted {
        if groups.is_empty() || groups.last().unwrap().distance != s.distance {
            groups.push(DistanceGroup {
                distance: s.distance,
                symbols: Vec::new(),
            });
        }
        groups.last_mut().unwrap().symbols.push(s);
    }
    groups
}

/// Encode serializes a Payload into GCF text format.
pub fn encode(p: &Payload) -> String {
    let mut b = String::new();

    // Group symbols by distance (sorted by score descending within each group).
    let groups = group_by_distance(&p.symbols);

    // Build symbol index AFTER sorting, so IDs are sequential in output order.
    let mut sym_index: HashMap<&str, usize> = HashMap::new();
    let mut next_id = 0usize;
    for g in &groups {
        for s in &g.symbols {
            sym_index.insert(&s.qualified_name, next_id);
            next_id += 1;
        }
    }

    // Count valid edges (both endpoints in symbol index).
    let valid_edges = p
        .edges
        .iter()
        .filter(|e| {
            sym_index.contains_key(e.source.as_str()) && sym_index.contains_key(e.target.as_str())
        })
        .count();

    // Header line.
    write!(b, "GCF profile=graph tool={}", p.tool).unwrap();
    if p.token_budget > 0 {
        write!(b, " budget={}", p.token_budget).unwrap();
    }
    if p.tokens_used > 0 {
        write!(b, " tokens={}", p.tokens_used).unwrap();
    }
    write!(b, " symbols={}", p.symbols.len()).unwrap();
    if valid_edges > 0 {
        write!(b, " edges={}", valid_edges).unwrap();
    }
    if !p.pack_root.is_empty() {
        write!(b, " pack_root={}", p.pack_root).unwrap();
    }
    b.push('\n');

    let group_names = ["targets", "related", "extended"];

    for g in &groups {
        if g.symbols.is_empty() {
            continue;
        }
        let name = if (g.distance as usize) < group_names.len() {
            group_names[g.distance as usize].to_string()
        } else {
            format!("distance_{}", g.distance)
        };
        writeln!(b, "## {}", name).unwrap();

        for s in &g.symbols {
            let idx = sym_index[s.qualified_name.as_str()];
            let kind = kind_abbrev(&s.kind);
            writeln!(
                b,
                "@{} {} {} {:.2} {}",
                idx, kind, s.qualified_name, s.score, s.provenance
            )
            .unwrap();
        }
    }

    // Edges section.
    if !p.edges.is_empty() {
        writeln!(b, "## edges [{}]", valid_edges).unwrap();
        // Resolve valid edges (both endpoints in symbol index), then order by
        // source ID then target ID (SPEC 16.1), with edge type breaking ties for
        // parallel edges. Reordering is decode-invariant (edges are a set) and does
        // not affect pack_root (which sorts edge records independently). Stable sort
        // preserves input order for fully-equal edges.
        let mut resolved: Vec<(usize, usize, &crate::types::Edge)> = p
            .edges
            .iter()
            .filter_map(|e| {
                match (
                    sym_index.get(e.source.as_str()),
                    sym_index.get(e.target.as_str()),
                ) {
                    (Some(&si), Some(&ti)) => Some((si, ti, e)),
                    _ => None,
                }
            })
            .collect();
        resolved.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.1.cmp(&b.1))
                .then(a.2.edge_type.cmp(&b.2.edge_type))
        });
        for (si, ti, e) in &resolved {
            write!(b, "@{}<@{} {}", ti, si, e.edge_type).unwrap();
            if !e.status.is_empty() && e.status != "unchanged" {
                write!(b, " {}", e.status).unwrap();
            }
            b.push('\n');
        }
    }

    b
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Edge;

    #[test]
    fn test_encode_basic() {
        let p = Payload {
            tool: "context_for_task".to_string(),
            token_budget: 5000,
            tokens_used: 1847,
            pack_root: String::new(),
            symbols: vec![
                Symbol {
                    qualified_name: "pkg.AuthMiddleware".to_string(),
                    kind: "function".to_string(),
                    score: 0.78,
                    provenance: "lsp_resolved".to_string(),
                    distance: 0,
                    signature: String::new(),
                    components: Default::default(),
                },
                Symbol {
                    qualified_name: "pkg.NewServer".to_string(),
                    kind: "function".to_string(),
                    score: 0.54,
                    provenance: "lsp_resolved".to_string(),
                    distance: 1,
                    signature: String::new(),
                    components: Default::default(),
                },
            ],
            edges: vec![Edge {
                source: "pkg.NewServer".to_string(),
                target: "pkg.AuthMiddleware".to_string(),
                edge_type: "calls".to_string(),
                status: String::new(),
            }],
        };

        let output = encode(&p);
        let expected = "\
GCF profile=graph tool=context_for_task budget=5000 tokens=1847 symbols=2 edges=1
## targets
@0 fn pkg.AuthMiddleware 0.78 lsp_resolved
## related
@1 fn pkg.NewServer 0.54 lsp_resolved
## edges [1]
@0<@1 calls
";
        assert_eq!(output, expected);
    }

    #[test]
    fn test_encode_with_pack_root() {
        let p = Payload {
            tool: "test".to_string(),
            token_budget: 100,
            tokens_used: 50,
            pack_root: "abc123".to_string(),
            symbols: vec![Symbol {
                qualified_name: "x.Y".to_string(),
                kind: "type".to_string(),
                score: 0.90,
                provenance: "ast".to_string(),
                distance: 0,
                signature: String::new(),
                components: Default::default(),
            }],
            edges: vec![],
        };
        let output = encode(&p);
        assert!(output.contains("pack_root=abc123"));
    }

    #[test]
    fn test_encode_kind_abbreviations() {
        let p = Payload {
            tool: "t".to_string(),
            token_budget: 0,
            tokens_used: 0,
            pack_root: String::new(),
            symbols: vec![
                Symbol {
                    qualified_name: "a.B".to_string(),
                    kind: "interface".to_string(),
                    score: 0.50,
                    provenance: "p".to_string(),
                    distance: 0,
                    signature: String::new(),
                    components: Default::default(),
                },
                Symbol {
                    qualified_name: "a.C".to_string(),
                    kind: "route_handler".to_string(),
                    score: 0.40,
                    provenance: "p".to_string(),
                    distance: 0,
                    signature: String::new(),
                    components: Default::default(),
                },
            ],
            edges: vec![],
        };
        let output = encode(&p);
        assert!(output.contains("iface a.B"));
        assert!(output.contains("route a.C"));
    }

    #[test]
    fn test_encode_distance_groups() {
        let p = Payload {
            tool: "t".to_string(),
            token_budget: 0,
            tokens_used: 0,
            pack_root: String::new(),
            symbols: vec![
                Symbol {
                    qualified_name: "a".to_string(),
                    kind: "function".to_string(),
                    score: 1.0,
                    provenance: "p".to_string(),
                    distance: 0,
                    signature: String::new(),
                    components: Default::default(),
                },
                Symbol {
                    qualified_name: "b".to_string(),
                    kind: "function".to_string(),
                    score: 0.5,
                    provenance: "p".to_string(),
                    distance: 5,
                    signature: String::new(),
                    components: Default::default(),
                },
            ],
            edges: vec![],
        };
        let output = encode(&p);
        assert!(output.contains("## targets"));
        assert!(output.contains("## distance_5"));
    }
}
