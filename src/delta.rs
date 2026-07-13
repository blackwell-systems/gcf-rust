use crate::packroot::pack_root;
use crate::types::{DeltaPayload, Edge, Symbol};
use crate::{kind_abbrev, kind_expand};
use std::collections::HashMap;
use std::fmt::Write;

/// Encode a DeltaPayload into GCF delta format.
pub fn encode_delta(d: &DeltaPayload) -> String {
    let mut b = String::new();

    // Header.
    let savings = if d.full_tokens > 0 {
        100.0 * (1.0 - d.delta_tokens as f64 / d.full_tokens as f64)
    } else {
        0.0
    };
    writeln!(
        b,
        "GCF profile=graph tool={} delta=true base_root={} new_root={} tokens={} savings={:.0}%",
        d.tool, d.base_root, d.new_root, d.delta_tokens, savings
    )
    .unwrap();

    // Removed symbols.
    if !d.removed.is_empty() {
        b.push_str("## removed\n");
        for s in &d.removed {
            let kind = kind_abbrev(&s.kind);
            writeln!(b, "{} {}", kind, s.qualified_name).unwrap();
        }
    }

    // Added symbols.
    if !d.added.is_empty() {
        b.push_str("## added\n");
        for (i, s) in d.added.iter().enumerate() {
            let kind = kind_abbrev(&s.kind);
            writeln!(
                b,
                "@{} {} {} {:.2} {} {}",
                i, kind, s.qualified_name, s.score, s.provenance, s.distance
            )
            .unwrap();
        }
    }

    // Removed edges.
    if !d.removed_edges.is_empty() {
        b.push_str("## edges_removed\n");
        for e in &d.removed_edges {
            writeln!(b, "{} -> {} {}", e.source, e.target, e.edge_type).unwrap();
        }
    }

    // Added edges.
    if !d.added_edges.is_empty() {
        b.push_str("## edges_added\n");
        for e in &d.added_edges {
            writeln!(b, "{} -> {} {}", e.source, e.target, e.edge_type).unwrap();
        }
    }

    b
}

/// Parse a `source -> target type` delta edge line.
fn parse_delta_edge(line: &str) -> Result<Edge, String> {
    let idx = match line.find(" -> ") {
        Some(i) => i,
        None => {
            return Err(format!(
                "malformed_delta: edge line missing ' -> ': {:?}",
                line
            ))
        }
    };
    let source = &line[..idx];
    let rest: Vec<&str> = line[idx + 4..].split_whitespace().collect();
    if rest.len() != 2 {
        return Err(format!(
            "malformed_delta: edge line {:?} must be 'source -> target type'",
            line
        ));
    }
    Ok(Edge {
        source: source.to_string(),
        target: rest[0].to_string(),
        edge_type: rest[1].to_string(),
        status: String::new(),
    })
}

/// Decode a GCF graph delta wire payload (as produced by `encode_delta`) back
/// into a `DeltaPayload`. Kind abbreviations on removed/added lines are expanded
/// to their full form so the result matches a base snapshot's symbol identities.
pub fn decode_delta(input: &str) -> Result<DeltaPayload, String> {
    let trimmed = input.trim_end_matches('\n');
    let lines: Vec<&str> = trimmed.split('\n').collect();
    if lines.is_empty() || lines[0].is_empty() {
        return Err("missing_header: empty delta payload".to_string());
    }
    let header = lines[0].trim_end_matches('\r');
    if !header.starts_with("GCF profile=graph") {
        return Err(
            "missing_profile: delta header must begin with 'GCF profile=graph'".to_string(),
        );
    }

    let mut d = DeltaPayload {
        tool: String::new(),
        base_root: String::new(),
        new_root: String::new(),
        removed: Vec::new(),
        added: Vec::new(),
        removed_edges: Vec::new(),
        added_edges: Vec::new(),
        delta_tokens: 0,
        full_tokens: 0,
    };
    for field in header.split_whitespace() {
        if let Some((k, v)) = field.split_once('=') {
            match k {
                "tool" => d.tool = v.to_string(),
                "base_root" => d.base_root = v.to_string(),
                "new_root" => d.new_root = v.to_string(),
                _ => {}
            }
        }
    }

    let mut section = "";
    for raw in &lines[1..] {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("## ") {
            section = rest.trim();
            match section {
                "removed" | "added" | "edges_removed" | "edges_added" => {}
                other => {
                    return Err(format!("malformed_delta: unknown section {:?}", other));
                }
            }
            continue;
        }
        match section {
            "removed" => {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() != 2 {
                    return Err(format!(
                        "malformed_delta: removed line {:?} must be 'kind qname'",
                        line
                    ));
                }
                d.removed.push(Symbol {
                    kind: kind_expand(parts[0]),
                    qualified_name: parts[1].to_string(),
                    score: 0.0,
                    provenance: String::new(),
                    distance: 0,
                    signature: String::new(),
                    components: Default::default(),
                });
            }
            "added" => {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() != 6 {
                    return Err(format!(
                        "malformed_delta: added line {:?} must be '@id kind qname score provenance distance'",
                        line
                    ));
                }
                let score: f64 = parts[3]
                    .parse()
                    .map_err(|_| format!("malformed_delta: invalid added score {:?}", parts[3]))?;
                let dist: i32 = parts[5].parse().map_err(|_| {
                    format!("malformed_delta: invalid added distance {:?}", parts[5])
                })?;
                d.added.push(Symbol {
                    kind: kind_expand(parts[1]),
                    qualified_name: parts[2].to_string(),
                    score,
                    provenance: parts[4].to_string(),
                    distance: dist,
                    signature: String::new(),
                    components: Default::default(),
                });
            }
            "edges_removed" => {
                d.removed_edges.push(parse_delta_edge(line)?);
            }
            "edges_added" => {
                d.added_edges.push(parse_delta_edge(line)?);
            }
            _ => {
                return Err(format!(
                    "malformed_delta: data line {:?} before any section header",
                    line
                ));
            }
        }
    }
    Ok(d)
}

/// Verify that applying a delta to a base snapshot produces the expected new_root.
/// Returns the resulting symbols and edges on success, or an error otherwise.
#[allow(clippy::too_many_arguments)]
pub fn verify_delta(
    base_symbols: &[Symbol],
    base_edges: &[Edge],
    removed_symbols: &[Symbol],
    added_symbols: &[Symbol],
    removed_edges: &[Edge],
    added_edges: &[Edge],
    expected_new_root: &str,
) -> Result<(Vec<Symbol>, Vec<Edge>), String> {
    // Index base symbols by identity (kind, qname).
    let mut sym_map: HashMap<(String, String), Symbol> = HashMap::with_capacity(base_symbols.len());
    for s in base_symbols {
        sym_map.insert((s.kind.clone(), s.qualified_name.clone()), s.clone());
    }

    // Apply symbol removals.
    for s in removed_symbols {
        let key = (s.kind.clone(), s.qualified_name.clone());
        if sym_map.remove(&key).is_none() {
            return Err(format!(
                "delta_invalid: removing symbol {} {} that does not exist in base",
                s.kind, s.qualified_name
            ));
        }
    }

    // Apply symbol additions.
    for s in added_symbols {
        let key = (s.kind.clone(), s.qualified_name.clone());
        if sym_map.contains_key(&key) {
            return Err(format!(
                "delta_invalid: adding symbol {} {} that already exists",
                s.kind, s.qualified_name
            ));
        }
        sym_map.insert(key, s.clone());
    }

    let result_symbols: Vec<Symbol> = sym_map.into_values().collect();

    // Index base edges by identity (source, target, type).
    let mut edge_map: HashMap<(String, String, String), Edge> =
        HashMap::with_capacity(base_edges.len());
    for e in base_edges {
        edge_map.insert(
            (e.source.clone(), e.target.clone(), e.edge_type.clone()),
            e.clone(),
        );
    }

    // Apply edge removals.
    for e in removed_edges {
        let key = (e.source.clone(), e.target.clone(), e.edge_type.clone());
        if edge_map.remove(&key).is_none() {
            return Err(format!(
                "delta_invalid: removing edge {} -> {} {} that does not exist",
                e.source, e.target, e.edge_type
            ));
        }
    }

    // Apply edge additions.
    for e in added_edges {
        let key = (e.source.clone(), e.target.clone(), e.edge_type.clone());
        if edge_map.contains_key(&key) {
            return Err(format!(
                "delta_invalid: adding edge {} -> {} {} that already exists",
                e.source, e.target, e.edge_type
            ));
        }
        edge_map.insert(key, e.clone());
    }

    let result_edges: Vec<Edge> = edge_map.into_values().collect();

    // Verify pack root.
    let computed_root = pack_root(&result_symbols, &result_edges);
    if computed_root != expected_new_root {
        return Err(format!(
            "root_mismatch: computed {}, expected {}",
            computed_root, expected_new_root
        ));
    }

    Ok((result_symbols, result_edges))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Edge, Symbol};

    #[test]
    fn test_encode_delta() {
        let d = DeltaPayload {
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
            removed_edges: vec![Edge {
                source: "a".to_string(),
                target: "b".to_string(),
                edge_type: "calls".to_string(),
                status: String::new(),
            }],
            added_edges: vec![Edge {
                source: "c".to_string(),
                target: "d".to_string(),
                edge_type: "uses".to_string(),
                status: String::new(),
            }],
            delta_tokens: 30,
            full_tokens: 200,
        };

        let output = encode_delta(&d);
        assert!(output.contains("delta=true"));
        assert!(output.contains("base_root=aaa111"));
        assert!(output.contains("new_root=bbb222"));
        assert!(output.contains("tokens=30"));
        assert!(output.contains("savings=85%"));
        assert!(output.contains("## removed"));
        assert!(output.contains("fn pkg.OldFunc"));
        assert!(output.contains("## added"));
        assert!(output.contains("@0 fn pkg.NewFunc 0.85 rwr 0"));
        assert!(output.contains("## edges_removed"));
        assert!(output.contains("a -> b calls"));
        assert!(output.contains("## edges_added"));
        assert!(output.contains("c -> d uses"));
    }

    #[test]
    fn test_delta_savings_zero_full() {
        let d = DeltaPayload {
            tool: "t".to_string(),
            base_root: "a".to_string(),
            new_root: "b".to_string(),
            removed: vec![],
            added: vec![],
            removed_edges: vec![],
            added_edges: vec![],
            delta_tokens: 0,
            full_tokens: 0,
        };
        let output = encode_delta(&d);
        assert!(output.contains("savings=0%"));
    }
}
