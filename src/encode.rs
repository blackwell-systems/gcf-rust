use crate::types::{Payload, Symbol};
use crate::kind_abbrev;
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
    let mut groups: Vec<DistanceGroup> = Vec::new();
    for s in symbols {
        if groups.is_empty() || groups.last().unwrap().distance != s.distance {
            groups.push(DistanceGroup {
                distance: s.distance,
                symbols: Vec::new(),
            });
        }
        groups.last_mut().unwrap().symbols.push(s.clone());
    }
    groups
}

/// Encode serializes a Payload into GCF text format.
pub fn encode(p: &Payload) -> String {
    let mut b = String::new();

    // Header line.
    write!(
        b,
        "GCF tool={} budget={} tokens={} symbols={}",
        p.tool,
        p.token_budget,
        p.tokens_used,
        p.symbols.len()
    )
    .unwrap();
    if !p.pack_root.is_empty() {
        write!(b, " pack_root={}", p.pack_root).unwrap();
    }
    b.push('\n');

    // Build symbol index for edge references.
    let mut sym_index: HashMap<&str, usize> = HashMap::new();
    for (i, s) in p.symbols.iter().enumerate() {
        sym_index.insert(&s.qualified_name, i);
    }

    // Group symbols by distance.
    let groups = group_by_distance(&p.symbols);
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
        write!(b, "## {}\n", name).unwrap();

        for s in &g.symbols {
            let idx = sym_index[s.qualified_name.as_str()];
            let kind = kind_abbrev(&s.kind);
            write!(
                b,
                "@{} {} {} {:.2} {}\n",
                idx, kind, s.qualified_name, s.score, s.provenance
            )
            .unwrap();
        }
    }

    // Edges section.
    if !p.edges.is_empty() {
        b.push_str("## edges\n");
        for e in &p.edges {
            let src_idx = sym_index.get(e.source.as_str());
            let tgt_idx = sym_index.get(e.target.as_str());
            if let (Some(&si), Some(&ti)) = (src_idx, tgt_idx) {
                write!(b, "@{}<@{} {}", ti, si, e.edge_type).unwrap();
                if !e.status.is_empty() && e.status != "unchanged" {
                    write!(b, " {}", e.status).unwrap();
                }
                b.push('\n');
            }
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
GCF tool=context_for_task budget=5000 tokens=1847 symbols=2
## targets
@0 fn pkg.AuthMiddleware 0.78 lsp_resolved
## related
@1 fn pkg.NewServer 0.54 lsp_resolved
## edges
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
