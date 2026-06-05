use crate::kind_abbrev;
use crate::types::{Payload, Symbol};
use std::collections::HashMap;
use std::fmt::Write;
use std::sync::Mutex;

/// Session tracks symbols that have been transmitted to a client, enabling
/// subsequent responses to reference them by ID without full retransmission.
/// Thread-safe: multiple tool handlers may encode concurrently within a session.
pub struct Session {
    inner: Mutex<SessionInner>,
}

struct SessionInner {
    symbols: HashMap<String, usize>,
    next_id: usize,
}

impl Session {
    /// Create a new empty session.
    pub fn new() -> Self {
        Session {
            inner: Mutex::new(SessionInner {
                symbols: HashMap::new(),
                next_id: 0,
            }),
        }
    }

    /// Returns true if the symbol has been sent in a previous response.
    pub fn transmitted(&self, qname: &str) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.symbols.contains_key(qname)
    }

    /// Returns the session-global ID for a previously transmitted symbol.
    /// Returns None if not found.
    pub fn get_id(&self, qname: &str) -> Option<usize> {
        let inner = self.inner.lock().unwrap();
        inner.symbols.get(qname).copied()
    }

    /// Record marks symbols as transmitted and assigns session-global IDs.
    pub fn record(&self, symbols: &[Symbol]) {
        let mut inner = self.inner.lock().unwrap();
        for sym in symbols {
            if !inner.symbols.contains_key(&sym.qualified_name) {
                let id = inner.next_id;
                inner.next_id += 1;
                inner.symbols.insert(sym.qualified_name.clone(), id);
            }
        }
    }

    /// Returns the number of symbols tracked in this session.
    pub fn size(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner.symbols.len()
    }

    /// Clears the session state.
    pub fn reset(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.symbols.clear();
        inner.next_id = 0;
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

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

/// Encode a payload with session deduplication. Symbols that were already
/// transmitted in prior responses are emitted as bare references
/// (`@N  # previously transmitted`) instead of full declarations.
pub fn encode_with_session(p: &Payload, sess: &Session) -> String {
    let mut b = String::new();

    // Build local ID mapping for this response.
    let mut local_index: HashMap<&str, usize> = HashMap::new();
    for (i, s) in p.symbols.iter().enumerate() {
        local_index.insert(&s.qualified_name, i);
    }

    // Count valid edges.
    let valid_edges = p
        .edges
        .iter()
        .filter(|e| {
            local_index.contains_key(e.source.as_str())
                && local_index.contains_key(e.target.as_str())
        })
        .count();

    // Header with session=true marker.
    write!(
        b,
        "GCF tool={} budget={} tokens={} symbols={} edges={} session=true",
        p.tool,
        p.token_budget,
        p.tokens_used,
        p.symbols.len(),
        valid_edges
    )
    .unwrap();
    if !p.pack_root.is_empty() {
        write!(b, " pack_root={}", p.pack_root).unwrap();
    }
    b.push('\n');

    // Track which symbols are new.
    let is_new: Vec<bool> = p
        .symbols
        .iter()
        .map(|s| !sess.transmitted(&s.qualified_name))
        .collect();

    // Group by distance.
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
        writeln!(b, "## {}", name).unwrap();

        for s in &g.symbols {
            let idx = local_index[s.qualified_name.as_str()];
            if sess.transmitted(&s.qualified_name) {
                writeln!(b, "@{}  # previously transmitted", idx).unwrap();
            } else {
                let kind = kind_abbrev(&s.kind);
                writeln!(
                    b,
                    "@{} {} {} {:.2} {}",
                    idx, kind, s.qualified_name, s.score, s.provenance
                )
                .unwrap();
            }
        }
    }

    // Edges section.
    if !p.edges.is_empty() {
        writeln!(b, "## edges [{}]", valid_edges).unwrap();
        for e in &p.edges {
            let src_idx = local_index.get(e.source.as_str());
            let tgt_idx = local_index.get(e.target.as_str());
            if let (Some(&si), Some(&ti)) = (src_idx, tgt_idx) {
                write!(b, "@{}<@{} {}", ti, si, e.edge_type).unwrap();
                if !e.status.is_empty() && e.status != "unchanged" {
                    write!(b, " {}", e.status).unwrap();
                }
                b.push('\n');
            }
        }
    }

    // Record new symbols in the session.
    let new_symbols: Vec<Symbol> = p
        .symbols
        .iter()
        .enumerate()
        .filter(|(i, _)| is_new[*i])
        .map(|(_, s)| s.clone())
        .collect();
    sess.record(&new_symbols);

    b
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Edge;

    fn make_symbol(qname: &str, distance: i32) -> Symbol {
        Symbol {
            qualified_name: qname.to_string(),
            kind: "function".to_string(),
            score: 0.80,
            provenance: "lsp".to_string(),
            distance,
            signature: String::new(),
            components: Default::default(),
        }
    }

    #[test]
    fn test_session_dedup() {
        let sess = Session::new();

        let p1 = Payload {
            tool: "test".to_string(),
            token_budget: 1000,
            tokens_used: 100,
            pack_root: String::new(),
            symbols: vec![make_symbol("a.Func1", 0), make_symbol("a.Func2", 1)],
            edges: vec![],
        };

        let out1 = encode_with_session(&p1, &sess);
        assert!(out1.contains("session=true"));
        assert!(out1.contains("fn a.Func1"));
        assert!(out1.contains("fn a.Func2"));
        assert!(!out1.contains("previously transmitted"));

        // Second call with overlapping symbol.
        let p2 = Payload {
            tool: "test".to_string(),
            token_budget: 1000,
            tokens_used: 50,
            pack_root: String::new(),
            symbols: vec![
                make_symbol("a.Func1", 0), // already transmitted
                make_symbol("a.Func3", 1), // new
            ],
            edges: vec![],
        };

        let out2 = encode_with_session(&p2, &sess);
        assert!(out2.contains("# previously transmitted"));
        assert!(out2.contains("fn a.Func3"));
        // a.Func1 should NOT have a full declaration in out2
        assert!(!out2.contains("fn a.Func1"));
    }

    #[test]
    fn test_session_size_and_reset() {
        let sess = Session::new();
        assert_eq!(sess.size(), 0);

        sess.record(&[make_symbol("x.Y", 0)]);
        assert_eq!(sess.size(), 1);
        assert!(sess.transmitted("x.Y"));

        sess.reset();
        assert_eq!(sess.size(), 0);
        assert!(!sess.transmitted("x.Y"));
    }

    #[test]
    fn test_session_edges_preserved() {
        let sess = Session::new();
        let p = Payload {
            tool: "test".to_string(),
            token_budget: 0,
            tokens_used: 0,
            pack_root: String::new(),
            symbols: vec![make_symbol("a.X", 0), make_symbol("a.Y", 1)],
            edges: vec![Edge {
                source: "a.Y".to_string(),
                target: "a.X".to_string(),
                edge_type: "calls".to_string(),
                status: String::new(),
            }],
        };
        let out = encode_with_session(&p, &sess);
        assert!(out.contains("## edges"));
        assert!(out.contains("@0<@1 calls"));
    }
}
