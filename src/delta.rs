use crate::types::DeltaPayload;
use crate::kind_abbrev;
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
    write!(
        b,
        "GCF tool={} delta=true base_root={} new_root={} tokens={} savings={:.0}%\n",
        d.tool, d.base_root, d.new_root, d.delta_tokens, savings
    )
    .unwrap();

    // Removed symbols.
    if !d.removed.is_empty() {
        b.push_str("## removed\n");
        for s in &d.removed {
            let kind = kind_abbrev(&s.kind);
            write!(b, "{} {}\n", kind, s.qualified_name).unwrap();
        }
    }

    // Added symbols.
    if !d.added.is_empty() {
        b.push_str("## added\n");
        for (i, s) in d.added.iter().enumerate() {
            let kind = kind_abbrev(&s.kind);
            write!(
                b,
                "@{} {} {} {:.2} {}\n",
                i, kind, s.qualified_name, s.score, s.provenance
            )
            .unwrap();
        }
    }

    // Removed edges.
    if !d.removed_edges.is_empty() {
        b.push_str("## edges_removed\n");
        for e in &d.removed_edges {
            write!(b, "{} -> {} {}\n", e.source, e.target, e.edge_type).unwrap();
        }
    }

    // Added edges.
    if !d.added_edges.is_empty() {
        b.push_str("## edges_added\n");
        for e in &d.added_edges {
            write!(b, "{} -> {} {}\n", e.source, e.target, e.edge_type).unwrap();
        }
    }

    b
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
        assert!(output.contains("@0 fn pkg.NewFunc 0.85 rwr"));
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
