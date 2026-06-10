use crate::kind_abbrev;
use crate::types::{Edge, Symbol};
use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use std::io::Write;
use std::sync::Mutex;

/// Options for the streaming encoder.
#[derive(Default)]
pub struct StreamOptions {
    pub token_budget: i64,
    pub tokens_used: i64,
    pub pack_root: String,
    pub session: bool,
}

/// StreamEncoder writes GCF output incrementally as symbols and edges arrive.
/// Zero buffering: each symbol/edge is written immediately. A trailer summary
/// is emitted on close() with the final counts.
///
/// Thread-safe via internal Mutex.
pub struct StreamEncoder<W: Write> {
    inner: Mutex<StreamEncoderInner<W>>,
}

struct StreamEncoderInner<W: Write> {
    w: W,
    sym_index: HashMap<String, usize>,
    next_id: usize,
    current_group: String,
    group_counts: Vec<(String, usize)>,
    edge_count: usize,
    edges_started: bool,
}

impl<W: Write> StreamEncoder<W> {
    /// Create a new streaming encoder writing to `w`. The header is emitted immediately.
    pub fn new(mut w: W, tool: &str, opts: StreamOptions) -> Self {
        let mut header = format!("GCF profile=graph tool={}", tool);
        if opts.token_budget > 0 {
            write!(header, " budget={}", opts.token_budget).unwrap();
        }
        if opts.tokens_used > 0 {
            write!(header, " tokens={}", opts.tokens_used).unwrap();
        }
        if !opts.pack_root.is_empty() {
            write!(header, " pack_root={}", opts.pack_root).unwrap();
        }
        if opts.session {
            header.push_str(" session=true");
        }
        header.push('\n');
        w.write_all(header.as_bytes()).unwrap();

        StreamEncoder {
            inner: Mutex::new(StreamEncoderInner {
                w,
                sym_index: HashMap::new(),
                next_id: 0,
                current_group: String::new(),
                group_counts: Vec::new(),
                edge_count: 0,
                edges_started: false,
            }),
        }
    }

    /// Emit a symbol line immediately. Group headers are auto-managed.
    pub fn write_symbol(&self, s: &Symbol) {
        let mut inner = self.inner.lock().unwrap();
        let group_names = ["targets", "related", "extended"];
        let group_name = if (s.distance as usize) < group_names.len() {
            group_names[s.distance as usize].to_string()
        } else {
            format!("distance_{}", s.distance)
        };

        if group_name != inner.current_group {
            writeln!(inner.w, "## {}", group_name).unwrap();
            inner.current_group = group_name.clone();
        }

        let id = inner.next_id;
        inner.sym_index.insert(s.qualified_name.clone(), id);
        inner.next_id += 1;

        let kind = kind_abbrev(&s.kind);
        writeln!(
            inner.w,
            "@{} {} {} {:.2} {}",
            id, kind, s.qualified_name, s.score, s.provenance
        )
        .unwrap();

        // Track group count.
        if let Some(entry) = inner
            .group_counts
            .iter_mut()
            .find(|(g, _)| g == &group_name)
        {
            entry.1 += 1;
        } else {
            inner.group_counts.push((group_name, 1));
        }
    }

    /// Emit an edge line immediately. Edges section header auto-emitted on first edge.
    pub fn write_edge(&self, e: &Edge) {
        let mut inner = self.inner.lock().unwrap();
        let src_idx = inner.sym_index.get(&e.source).copied();
        let tgt_idx = inner.sym_index.get(&e.target).copied();

        let (si, ti) = match (src_idx, tgt_idx) {
            (Some(s), Some(t)) => (s, t),
            _ => return,
        };

        if !inner.edges_started {
            writeln!(inner.w, "## edges [?]").unwrap();
            inner.edges_started = true;
        }

        let mut line = format!("@{}<@{} {}", ti, si, e.edge_type);
        if !e.status.is_empty() && e.status != "unchanged" {
            write!(line, " {}", e.status).unwrap();
        }
        writeln!(inner.w, "{}", line).unwrap();
        inner.edge_count += 1;
    }

    /// Emit a bare reference (session mode).
    pub fn write_bare_ref(&self, qname: &str, distance: i32) {
        let mut inner = self.inner.lock().unwrap();
        let group_names = ["targets", "related", "extended"];
        let group_name = if (distance as usize) < group_names.len() {
            group_names[distance as usize].to_string()
        } else {
            format!("distance_{}", distance)
        };

        if group_name != inner.current_group {
            writeln!(inner.w, "## {}", group_name).unwrap();
            inner.current_group = group_name.clone();
        }

        let id = inner.next_id;
        inner.sym_index.insert(qname.to_string(), id);
        inner.next_id += 1;
        writeln!(inner.w, "@{}  # previously transmitted", id).unwrap();

        if let Some(entry) = inner
            .group_counts
            .iter_mut()
            .find(|(g, _)| g == &group_name)
        {
            entry.1 += 1;
        } else {
            inner.group_counts.push((group_name, 1));
        }
    }

    /// Emit ##! summary trailer with final counts.
    pub fn close(&self) {
        let mut inner = self.inner.lock().unwrap();
        let mut counts: Vec<String> = Vec::new();

        for (_g, c) in &inner.group_counts {
            if *c > 0 {
                counts.push(c.to_string());
            }
        }
        if inner.edge_count > 0 {
            counts.push(inner.edge_count.to_string());
        }

        let symbol_count = inner.next_id;
        let edge_count = inner.edge_count;
        writeln!(
            inner.w,
            "##! summary symbols={} edges={} counts={}",
            symbol_count,
            edge_count,
            counts.join(",")
        )
        .unwrap();
    }

    /// Number of symbols written so far.
    pub fn symbol_count(&self) -> usize {
        self.inner.lock().unwrap().next_id
    }

    /// Number of edges written so far.
    pub fn edge_count(&self) -> usize {
        self.inner.lock().unwrap().edge_count
    }
}
