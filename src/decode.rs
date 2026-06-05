use crate::types::{Edge, Payload, Symbol};
use crate::kind_expand;
use std::collections::HashMap;
use std::fmt;

/// Error type for GCF decoding failures.
#[derive(Debug, Clone, PartialEq)]
pub enum DecodeError {
    EmptyInput,
    InvalidHeader(String),
    MissingTool,
    InvalidField(String),
    InvalidSymbolLine(String),
    InvalidEdgeLine(String),
    UnknownEdgeId(String),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::EmptyInput => write!(f, "gcf: empty input"),
            DecodeError::InvalidHeader(h) => {
                write!(f, "gcf: invalid header, expected 'GCF ...' got {:?}", h)
            }
            DecodeError::MissingTool => write!(f, "gcf: header missing required 'tool' field"),
            DecodeError::InvalidField(msg) => write!(f, "gcf: {}", msg),
            DecodeError::InvalidSymbolLine(msg) => write!(f, "gcf: {}", msg),
            DecodeError::InvalidEdgeLine(msg) => write!(f, "gcf: {}", msg),
            DecodeError::UnknownEdgeId(msg) => write!(f, "gcf: {}", msg),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Decode parses GCF text back into a Payload.
pub fn decode(input: &str) -> Result<Payload, DecodeError> {
    let lines: Vec<&str> = input.split('\n').collect();
    if lines.is_empty() || (lines.len() == 1 && lines[0].is_empty()) {
        return Err(DecodeError::EmptyInput);
    }

    let header = lines[0];
    if !header.starts_with("GCF ") {
        return Err(DecodeError::InvalidHeader(header.to_string()));
    }

    let mut p = Payload {
        tool: String::new(),
        tokens_used: 0,
        token_budget: 0,
        pack_root: String::new(),
        symbols: Vec::new(),
        edges: Vec::new(),
    };

    parse_header(&header[4..], &mut p)?;

    if p.tool.is_empty() {
        return Err(DecodeError::MissingTool);
    }

    let mut symbols: Vec<Symbol> = Vec::new();
    let mut sym_by_id: HashMap<usize, usize> = HashMap::new(); // id -> index in symbols vec
    let mut current_distance: i32 = 0;
    let mut in_edges = false;

    for &line in &lines[1..] {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }

        // Group header.
        if let Some(group) = line.strip_prefix("## ") {
            if group == "edges" {
                in_edges = true;
            } else {
                in_edges = false;
                match group {
                    "targets" => current_distance = 0,
                    "related" => current_distance = 1,
                    "extended" => current_distance = 2,
                    _ => {
                        if let Some(d) = group.strip_prefix("distance_") {
                            if let Ok(n) = d.parse::<i32>() {
                                current_distance = n;
                            }
                        }
                    }
                }
            }
            continue;
        }

        // Comment.
        if line.starts_with("# ") {
            continue;
        }

        if in_edges {
            let edge = parse_edge_line(line, &sym_by_id, &symbols)?;
            p.edges.push(edge);
        } else {
            let (sym, id) = parse_symbol_line(line, current_distance)?;
            symbols.push(sym);
            sym_by_id.insert(id, symbols.len() - 1);
        }
    }

    p.symbols = symbols;
    Ok(p)
}

fn parse_header(fields: &str, p: &mut Payload) -> Result<(), DecodeError> {
    for part in fields.split_whitespace() {
        let mut kv = part.splitn(2, '=');
        let key = match kv.next() {
            Some(k) => k,
            None => continue,
        };
        let val = match kv.next() {
            Some(v) => v,
            None => continue,
        };
        match key {
            "tool" => p.tool = val.to_string(),
            "budget" => {
                p.token_budget = val.parse::<i64>().map_err(|_| {
                    DecodeError::InvalidField(format!("invalid budget {:?}", val))
                })?;
            }
            "tokens" => {
                p.tokens_used = val.parse::<i64>().map_err(|_| {
                    DecodeError::InvalidField(format!("invalid tokens {:?}", val))
                })?;
            }
            "pack_root" => p.pack_root = val.to_string(),
            "symbols" => { /* informational, reconstructed from parsed symbols */ }
            _ => { /* ignore unknown fields like session=true */ }
        }
    }
    Ok(())
}

fn parse_symbol_line(line: &str, distance: i32) -> Result<(Symbol, usize), DecodeError> {
    if !line.starts_with('@') {
        return Err(DecodeError::InvalidSymbolLine(format!(
            "expected symbol line starting with @, got {:?}",
            line
        )));
    }

    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 5 {
        return Err(DecodeError::InvalidSymbolLine(format!(
            "symbol line needs at least 5 fields, got {} in {:?}",
            parts.len(),
            line
        )));
    }

    let id_str = &parts[0][1..]; // strip @
    let id: usize = id_str.parse().map_err(|_| {
        DecodeError::InvalidSymbolLine(format!("invalid symbol id {:?}", id_str))
    })?;

    let kind = kind_expand(parts[1]);
    let qname = parts[2].to_string();
    let score: f64 = parts[3].parse().map_err(|_| {
        DecodeError::InvalidSymbolLine(format!("invalid score {:?}", parts[3]))
    })?;
    let provenance = parts[4].to_string();

    Ok((
        Symbol {
            qualified_name: qname,
            kind,
            score,
            provenance,
            distance,
            signature: String::new(),
            components: Default::default(),
        },
        id,
    ))
}

fn parse_edge_line(
    line: &str,
    sym_by_id: &HashMap<usize, usize>,
    symbols: &[Symbol],
) -> Result<Edge, DecodeError> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(DecodeError::InvalidEdgeLine(format!(
            "edge line needs at least 2 fields, got {:?}",
            line
        )));
    }

    let reference = parts[0];
    let lt_idx = match reference.find('<') {
        Some(i) => i,
        None => {
            return Err(DecodeError::InvalidEdgeLine(format!(
                "edge line missing '<' separator in {:?}",
                reference
            )));
        }
    };

    let target_id_str = &reference[1..lt_idx]; // strip leading @
    let source_id_str = &reference[lt_idx + 2..]; // strip <@

    let target_id: usize = target_id_str.parse().map_err(|_| {
        DecodeError::InvalidEdgeLine(format!("invalid target id {:?}", target_id_str))
    })?;
    let source_id: usize = source_id_str.parse().map_err(|_| {
        DecodeError::InvalidEdgeLine(format!("invalid source id {:?}", source_id_str))
    })?;

    let target_idx = sym_by_id.get(&target_id).ok_or_else(|| {
        DecodeError::UnknownEdgeId(format!(
            "edge references unknown symbol id(s): target={} source={}",
            target_id, source_id
        ))
    })?;
    let source_idx = sym_by_id.get(&source_id).ok_or_else(|| {
        DecodeError::UnknownEdgeId(format!(
            "edge references unknown symbol id(s): target={} source={}",
            target_id, source_id
        ))
    })?;

    let edge_type = parts[1].to_string();
    let status = if parts.len() >= 3 {
        parts[2].to_string()
    } else {
        String::new()
    };

    Ok(Edge {
        source: symbols[*source_idx].qualified_name.clone(),
        target: symbols[*target_idx].qualified_name.clone(),
        edge_type,
        status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_basic() {
        let input = "\
GCF tool=context_for_task budget=5000 tokens=1847 symbols=2
## targets
@0 fn pkg.AuthMiddleware 0.78 lsp_resolved
## related
@1 fn pkg.NewServer 0.54 lsp_resolved
## edges
@0<@1 calls
";
        let p = decode(input).unwrap();
        assert_eq!(p.tool, "context_for_task");
        assert_eq!(p.token_budget, 5000);
        assert_eq!(p.tokens_used, 1847);
        assert_eq!(p.symbols.len(), 2);
        assert_eq!(p.symbols[0].qualified_name, "pkg.AuthMiddleware");
        assert_eq!(p.symbols[0].kind, "function"); // fn expanded
        assert_eq!(p.symbols[0].distance, 0);
        assert_eq!(p.symbols[1].distance, 1);
        assert_eq!(p.edges.len(), 1);
        assert_eq!(p.edges[0].source, "pkg.NewServer");
        assert_eq!(p.edges[0].target, "pkg.AuthMiddleware");
        assert_eq!(p.edges[0].edge_type, "calls");
    }

    #[test]
    fn test_decode_rejects_bad_header() {
        let result = decode("NOT_GCF tool=x");
        assert!(result.is_err());
        match result.unwrap_err() {
            DecodeError::InvalidHeader(_) => {}
            other => panic!("expected InvalidHeader, got {:?}", other),
        }
    }

    #[test]
    fn test_decode_rejects_missing_tool() {
        let result = decode("GCF budget=100 tokens=50 symbols=0");
        assert!(result.is_err());
        match result.unwrap_err() {
            DecodeError::MissingTool => {}
            other => panic!("expected MissingTool, got {:?}", other),
        }
    }

    #[test]
    fn test_decode_rejects_too_few_fields() {
        let input = "GCF tool=t budget=0 tokens=0 symbols=1\n## targets\n@0 fn pkg.X 0.5";
        let result = decode(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_rejects_unknown_edge_ids() {
        let input = "\
GCF tool=t budget=0 tokens=0 symbols=1
## targets
@0 fn a.B 0.50 p
## edges
@0<@99 calls
";
        let result = decode(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_tolerates_crlf() {
        let input = "GCF tool=t budget=0 tokens=0 symbols=1\r\n## targets\r\n@0 fn a.B 0.50 p\r\n";
        let p = decode(input).unwrap();
        assert_eq!(p.symbols.len(), 1);
        assert_eq!(p.symbols[0].qualified_name, "a.B");
    }

    #[test]
    fn test_decode_expands_kind() {
        let input = "GCF tool=t budget=0 tokens=0 symbols=1\n## targets\n@0 iface a.B 0.50 p\n";
        let p = decode(input).unwrap();
        assert_eq!(p.symbols[0].kind, "interface");
    }

    #[test]
    fn test_decode_empty_input() {
        let result = decode("");
        assert!(result.is_err());
    }

    #[test]
    fn test_roundtrip() {
        use crate::encode::encode;
        use crate::types::Edge;

        let p = Payload {
            tool: "roundtrip_test".to_string(),
            token_budget: 2000,
            tokens_used: 500,
            pack_root: String::new(),
            symbols: vec![
                Symbol {
                    qualified_name: "pkg.Func1".to_string(),
                    kind: "function".to_string(),
                    score: 0.95,
                    provenance: "lsp_resolved".to_string(),
                    distance: 0,
                    signature: String::new(),
                    components: Default::default(),
                },
                Symbol {
                    qualified_name: "pkg.Type1".to_string(),
                    kind: "type".to_string(),
                    score: 0.80,
                    provenance: "ast_inferred".to_string(),
                    distance: 1,
                    signature: String::new(),
                    components: Default::default(),
                },
                Symbol {
                    qualified_name: "pkg.Iface1".to_string(),
                    kind: "interface".to_string(),
                    score: 0.60,
                    provenance: "lsp_resolved".to_string(),
                    distance: 2,
                    signature: String::new(),
                    components: Default::default(),
                },
            ],
            edges: vec![Edge {
                source: "pkg.Func1".to_string(),
                target: "pkg.Type1".to_string(),
                edge_type: "uses".to_string(),
                status: String::new(),
            }],
        };

        let encoded = encode(&p);
        let decoded = decode(&encoded).unwrap();

        assert_eq!(decoded.tool, p.tool);
        assert_eq!(decoded.token_budget, p.token_budget);
        assert_eq!(decoded.tokens_used, p.tokens_used);
        assert_eq!(decoded.symbols.len(), p.symbols.len());
        for (i, s) in decoded.symbols.iter().enumerate() {
            assert_eq!(s.qualified_name, p.symbols[i].qualified_name);
            assert_eq!(s.kind, p.symbols[i].kind);
            assert!((s.score - p.symbols[i].score).abs() < 0.01);
            assert_eq!(s.provenance, p.symbols[i].provenance);
            assert_eq!(s.distance, p.symbols[i].distance);
        }
        assert_eq!(decoded.edges.len(), p.edges.len());
        assert_eq!(decoded.edges[0].source, p.edges[0].source);
        assert_eq!(decoded.edges[0].target, p.edges[0].target);
        assert_eq!(decoded.edges[0].edge_type, p.edges[0].edge_type);
    }
}
