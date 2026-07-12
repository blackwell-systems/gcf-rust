//! Generic-profile delta encoding (SPEC Section 10a).
//!
//! Full producer + consumer for keyed-row deltas over the generic profile,
//! byte-for-byte interoperable with gcf-go, gcf-python, and gcf-typescript.
//! Delta is opt-in and bilateral; the existing `encode_generic` path is unchanged.
//!
//! SHA-256 is implemented locally (no new dependency); the shared conformance
//! fixtures verify it end to end.

use crate::scalar::{
    format_key, format_number, format_scalar, parse_quoted_string, parse_scalar,
    split_respecting_quotes, quote_string, ScalarValue,
};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fmt::Write;

const NULL: Value = Value::Null;

/// A keyed record set: the unit generic-profile delta operates on (Section 10a).
/// Rows are order-agnostic (set semantics); `fields` carries the declared column
/// order for the wire form; `key` names the identity column (the `@id` / `key=`);
/// `name` is the tabular section name for a full payload.
#[derive(Debug, Clone, PartialEq)]
pub struct GenericSet {
    pub name: String,
    pub key: String,
    pub fields: Vec<String>,
    pub rows: Vec<Map<String, Value>>,
}

/// A diff between two `GenericSet`s (computed by `diff_generic_sets` or supplied
/// directly and serialized by `encode_generic_delta`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GenericDeltaPayload {
    pub tool: String,
    pub key: String,
    pub fields: Vec<String>,
    pub base_root: String,
    pub new_root: String,
    pub added: Vec<Map<String, Value>>,
    pub changed: Vec<Map<String, Value>>,
    pub removed: Vec<Value>,
    pub delta_tokens: u64,
    pub full_tokens: u64,
}

/// Canonicalize one value for the pack-root record (Section 10a.3). Purpose-built
/// and deliberately decoupled from the wire cell encoder (`format_scalar`): it must
/// be collision-free and record-safe, not round-trippable.
///   - Typed literals stay bare so they never collide with the strings that spell
///     them: null is `-` (never a string), booleans are `true`/`false`, numbers are
///     canonical (Section 2.3.1).
///   - Strings are ALWAYS quoted, so (a) they can't collide with a typed literal
///     (`-`, `true`, `123` all become quoted), and (b) a tab or newline inside a
///     value is escaped and cannot break the tab/newline-delimited record.
pub fn canonical_cell(v: &Value) -> String {
    match v {
        Value::Null => "-".to_string(),
        Value::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        Value::Number(n) => format_number(n),
        Value::String(s) => quote_string(s),
        other => quote_string(&other.to_string()),
    }
}

/// Compute the canonical pack root for a keyed set using the gcf-pack-root-v1
/// algorithm, generic profile (Section 10a.3). Two implementations given the same
/// logical set MUST produce the same result. Fields and records sort by UTF-8 byte
/// order (Rust `str` ordering is byte-wise), matching Go's `sort.Strings`.
pub fn generic_pack_root(s: &GenericSet) -> String {
    let mut sorted_fields = s.fields.clone();
    sorted_fields.sort();

    let mut records: Vec<String> = s
        .rows
        .iter()
        .map(|row| {
            let mut r = String::from("R");
            for f in &sorted_fields {
                r.push('\t');
                r.push_str(f);
                r.push('\t');
                r.push_str(&canonical_cell(row.get(f).unwrap_or(&NULL)));
            }
            r.push('\n');
            r
        })
        .collect();
    records.sort();

    format!("sha256:{}", sha256_hex(records.concat().as_bytes()))
}

/// Build an identity -> row map, rejecting duplicate identities (Section 10a.1).
fn index_by_key(s: &GenericSet) -> Result<HashMap<String, &Map<String, Value>>, String> {
    let mut m = HashMap::with_capacity(s.rows.len());
    for row in &s.rows {
        let id = canonical_cell(row.get(&s.key).unwrap_or(&NULL));
        if m.contains_key(&id) {
            return Err(format!(
                "delta_invalid: duplicate identity {} for key \"{}\"",
                id, s.key
            ));
        }
        m.insert(id, row);
    }
    Ok(m)
}

fn rows_equal(a: &Map<String, Value>, b: &Map<String, Value>, fields: &[String]) -> bool {
    fields.iter().all(|f| {
        canonical_cell(a.get(f).unwrap_or(&NULL)) == canonical_cell(b.get(f).unwrap_or(&NULL))
    })
}

fn key_of(row: &Map<String, Value>, key: &str) -> String {
    canonical_cell(row.get(key).unwrap_or(&NULL))
}

/// Compute the delta from `base` to `next`. This is the blessed producer path: it
/// is the single place that enforces the keyed-diff invariants (identity
/// uniqueness, added-not-in-base, changed-must-exist, whole-row replacement,
/// unchanged rows omitted). Added/changed/removed are sorted by identity for
/// reproducible output (Section 10a.6). Schema change or a missing key returns an
/// error: the caller must then send a full payload (Section 10a.7).
pub fn diff_generic_sets(
    base: &GenericSet,
    next: &GenericSet,
) -> Result<GenericDeltaPayload, String> {
    if next.key.is_empty() {
        return Err("delta_invalid: no identity key".to_string());
    }
    if next.key != base.key || base.fields != next.fields {
        return Err("delta_invalid: schema change (send full)".to_string());
    }
    let base_idx = index_by_key(base)?;
    let next_idx = index_by_key(next)?;

    let mut added: Vec<Map<String, Value>> = Vec::new();
    let mut changed: Vec<Map<String, Value>> = Vec::new();
    let mut removed: Vec<Value> = Vec::new();

    for (id, row) in &next_idx {
        match base_idx.get(id) {
            None => added.push((*row).clone()),
            Some(brow) => {
                if !rows_equal(brow, row, &next.fields) {
                    changed.push((*row).clone());
                }
            }
        }
        // equal rows are omitted (silence = "keep it", Section 10a.5)
    }
    for (id, brow) in &base_idx {
        if !next_idx.contains_key(id) {
            removed.push(brow.get(&next.key).cloned().unwrap_or(Value::Null));
        }
    }

    added.sort_by_key(|r| key_of(r, &next.key));
    changed.sort_by_key(|r| key_of(r, &next.key));
    removed.sort_by_key(canonical_cell);

    Ok(GenericDeltaPayload {
        tool: String::new(),
        key: next.key.clone(),
        fields: next.fields.clone(),
        base_root: generic_pack_root(base),
        new_root: generic_pack_root(next),
        added,
        changed,
        removed,
        delta_tokens: 0,
        full_tokens: 0,
    })
}

// --- producer-side wire encoding ---

fn field_decl(fields: &[String], key: &str) -> String {
    fields
        .iter()
        .map(|f| {
            if f == key {
                format!("@{}", format_key(f))
            } else {
                format_key(f)
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn encode_row(row: &Map<String, Value>, fields: &[String]) -> String {
    fields
        .iter()
        .map(|f| format_scalar(row.get(f).unwrap_or(&NULL), '|'))
        .collect::<Vec<_>>()
        .join("|")
}

/// Emit a delta-participating full base payload: `key=` in the header, an
/// `@`-prefixed identity field in the declaration, pipe-separated rows.
pub fn encode_generic_full(s: &GenericSet, tool: &str) -> String {
    let name = if s.name.is_empty() { "rows" } else { &s.name };
    let mut b = String::from("GCF profile=generic");
    if !tool.is_empty() {
        write!(b, " tool={}", tool).unwrap();
    }
    writeln!(b, " pack_root={} key={}", generic_pack_root(s), s.key).unwrap();
    writeln!(
        b,
        "## {} [{}]{{{}}}",
        name,
        s.rows.len(),
        field_decl(&s.fields, &s.key)
    )
    .unwrap();
    for row in &s.rows {
        b.push_str(&encode_row(row, &s.fields));
        b.push('\n');
    }
    b
}

/// Serialize a delta payload (Section 10a.2). Sections are emitted in the
/// deterministic order added / changed / removed (Section 10a.6).
pub fn encode_generic_delta(d: &GenericDeltaPayload) -> String {
    let mut b = String::from("GCF profile=generic");
    if !d.tool.is_empty() {
        write!(b, " tool={}", d.tool).unwrap();
    }
    write!(
        b,
        " delta=true base_root={} new_root={} key={}",
        d.base_root, d.new_root, d.key
    )
    .unwrap();
    if d.full_tokens > 0 {
        let savings = 100.0 * (1.0 - d.delta_tokens as f64 / d.full_tokens as f64);
        write!(b, " savings={:.0}%", savings).unwrap();
    }
    b.push('\n');

    if !d.added.is_empty() {
        writeln!(b, "## added [{}]{{{}}}", d.added.len(), field_decl(&d.fields, &d.key)).unwrap();
        for row in &d.added {
            b.push_str(&encode_row(row, &d.fields));
            b.push('\n');
        }
    }
    if !d.changed.is_empty() {
        writeln!(b, "## changed [{}]{{{}}}", d.changed.len(), field_decl(&d.fields, &d.key)).unwrap();
        for row in &d.changed {
            b.push_str(&encode_row(row, &d.fields));
            b.push('\n');
        }
    }
    if !d.removed.is_empty() {
        writeln!(b, "## removed [{}]{{@{}}}", d.removed.len(), d.key).unwrap();
        for idv in &d.removed {
            b.push_str(&format_scalar(idv, '|'));
            b.push('\n');
        }
    }
    b
}

/// Apply a delta to a base set and verify the result hashes to `expected_new_root`
/// (Section 10a.5). Atomic: the whole payload is validated before any state
/// changes, and a mismatch leaves the base untouched.
pub fn verify_generic_delta(
    base: &GenericSet,
    d: &GenericDeltaPayload,
    expected_new_root: &str,
) -> Result<GenericSet, String> {
    if generic_pack_root(base) != d.base_root {
        return Err("base_mismatch: base root does not equal delta base_root".to_string());
    }
    let base_idx = index_by_key(base)?;

    // Validate the entire payload against the original base before mutating.
    for idv in &d.removed {
        if !base_idx.contains_key(&canonical_cell(idv)) {
            return Err(format!(
                "delta_invalid: removing identity {} not in base",
                canonical_cell(idv)
            ));
        }
    }
    for row in &d.added {
        if base_idx.contains_key(&key_of(row, &d.key)) {
            return Err(format!(
                "delta_invalid: adding identity {} that already exists",
                key_of(row, &d.key)
            ));
        }
    }
    for row in &d.changed {
        if !base_idx.contains_key(&key_of(row, &d.key)) {
            return Err(format!(
                "delta_invalid: changing identity {} not in base",
                key_of(row, &d.key)
            ));
        }
    }

    // Apply to a working copy.
    let mut work: HashMap<String, Map<String, Value>> = base_idx
        .iter()
        .map(|(k, v)| (k.clone(), (*v).clone()))
        .collect();
    for idv in &d.removed {
        work.remove(&canonical_cell(idv));
    }
    for row in &d.added {
        work.insert(key_of(row, &d.key), row.clone());
    }
    for row in &d.changed {
        work.insert(key_of(row, &d.key), row.clone());
    }

    let result = GenericSet {
        name: base.name.clone(),
        key: base.key.clone(),
        fields: base.fields.clone(),
        rows: work.into_values().collect(),
    };
    let got = generic_pack_root(&result);
    if got != expected_new_root {
        return Err(format!(
            "root_mismatch: computed {}, expected {}",
            got, expected_new_root
        ));
    }
    Ok(result)
}

// --- consumer-side wire parsing (Section 10a) ---

fn scalar_to_value(sv: ScalarValue) -> Result<Value, String> {
    match sv {
        ScalarValue::Null => Ok(Value::Null),
        ScalarValue::Bool(b) => Ok(Value::Bool(b)),
        ScalarValue::Int(i) => Ok(Value::Number(i.into())),
        ScalarValue::Float(f) => serde_json::Number::from_f64(f)
            .map(Value::Number)
            .ok_or_else(|| "delta_invalid: non-finite number".to_string()),
        ScalarValue::Str(s) => Ok(Value::String(s)),
        ScalarValue::Missing => {
            Err("delta_invalid: missing (~) not allowed in delta row".to_string())
        }
        ScalarValue::Attachment => {
            Err("delta_invalid: attachment (^) not allowed in delta row".to_string())
        }
    }
}

fn parse_header_fields(header: &str) -> HashMap<String, String> {
    let mut m = HashMap::new();
    for tok in header.split_whitespace() {
        if let Some(i) = tok.find('=') {
            if i > 0 {
                m.insert(tok[..i].to_string(), tok[i + 1..].to_string());
            }
        }
    }
    m
}

fn parse_count(s: &str) -> Result<usize, String> {
    if s == "0" {
        return Ok(0);
    }
    if s.is_empty() || s.starts_with('0') {
        return Err(format!("delta_invalid: invalid count {}", s));
    }
    s.parse::<usize>()
        .map_err(|_| format!("delta_invalid: invalid count {}", s))
}

/// Find the byte index of the first `[` not inside a quoted string.
fn find_bracket_start(s: &str) -> Option<usize> {
    let mut in_quote = false;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' && in_quote {
            escaped = true;
            continue;
        }
        if c == '"' {
            in_quote = !in_quote;
            continue;
        }
        if c == '[' && !in_quote {
            return Some(i);
        }
    }
    None
}

/// Parse a delta/full field declaration `{@id,total,...}`, returning the ordered
/// fields and the key field (the one that was `@`-marked) (Section 10a.1).
fn split_delta_field_decl(decl: &str) -> Result<(Vec<String>, String), String> {
    if decl.len() < 2 || !decl.starts_with('{') || !decl.ends_with('}') {
        return Err(format!("delta_invalid: invalid field declaration: {}", decl));
    }
    let inner = &decl[1..decl.len() - 1];
    if inner.is_empty() {
        return Ok((Vec::new(), String::new()));
    }
    let mut fields = Vec::new();
    let mut key_field = String::new();
    for raw in split_respecting_quotes(inner, ',') {
        let mut f = raw.trim().to_string();
        let mut is_key = false;
        if let Some(rest) = f.strip_prefix('@') {
            f = rest.to_string();
            is_key = true;
        }
        if f.len() >= 2 && f.starts_with('"') && f.ends_with('"') {
            f = parse_quoted_string(&f)?;
        }
        if is_key {
            key_field = f.clone();
        }
        fields.push(f);
    }
    Ok((fields, key_field))
}

/// Parse the content after `## ` of a delta/full section, e.g.
/// `added [1]{@id,total,status,customer}` or `orders [3]{@id,...}` or `removed [1]{@id}`.
fn parse_section_header(content: &str) -> Result<(String, usize, Vec<String>, String), String> {
    let bi = find_bracket_start(content)
        .ok_or_else(|| format!("delta_invalid: section header without count: {}", content))?;
    let name = content[..bi].trim().to_string();
    let rest = &content[bi..]; // "[N]{...}"
    if !rest.starts_with('[') {
        return Err(format!("delta_invalid: malformed section header: {}", content));
    }
    let close = rest
        .find(']')
        .ok_or_else(|| format!("delta_invalid: unterminated count: {}", content))?;
    let count = parse_count(&rest[1..close])?;
    let (fields, key_field) = split_delta_field_decl(&rest[close + 1..])?;
    Ok((name, count, fields, key_field))
}

fn parse_row(line: &str, fields: &[String]) -> Result<Map<String, Value>, String> {
    let cells = split_respecting_quotes(line, '|');
    if cells.len() != fields.len() {
        return Err(format!(
            "delta_invalid: row has {} cells, expected {}: {}",
            cells.len(),
            fields.len(),
            line
        ));
    }
    let mut row = Map::new();
    for (i, f) in fields.iter().enumerate() {
        row.insert(f.clone(), scalar_to_value(parse_scalar(&cells[i], true)?)?);
    }
    Ok(row)
}

/// Parse a delta-participating full base payload into a `GenericSet`, and return
/// the declared `pack_root` (Section 10a).
pub fn decode_generic_full(text: &str) -> Result<(GenericSet, String), String> {
    let trimmed = text.trim_end_matches('\n');
    let lines: Vec<&str> = trimmed.split('\n').collect();
    let hdr = parse_header_fields(lines[0]);
    if hdr.get("profile").map(String::as_str) != Some("generic") {
        return Err("not a generic payload".to_string());
    }
    let mut set = GenericSet {
        name: String::new(),
        key: hdr.get("key").cloned().unwrap_or_default(),
        fields: Vec::new(),
        rows: Vec::new(),
    };
    let mut i = 1;
    while i < lines.len() {
        let line = lines[i];
        if !line.starts_with("## ") {
            i += 1;
            continue;
        }
        let (name, count, fields, key_field) = parse_section_header(&line[3..])?;
        set.name = name;
        set.fields = fields.clone();
        if set.key.is_empty() {
            set.key = key_field;
        }
        i += 1;
        for _ in 0..count {
            if i >= lines.len() {
                return Err("delta_invalid: fewer rows than declared count".to_string());
            }
            set.rows.push(parse_row(lines[i], &fields)?);
            i += 1;
        }
    }
    Ok((set, hdr.get("pack_root").cloned().unwrap_or_default()))
}

/// Parse a delta payload into a `GenericDeltaPayload` (Section 10a.2). The result
/// can be applied with `verify_generic_delta`.
pub fn decode_generic_delta(text: &str) -> Result<GenericDeltaPayload, String> {
    let trimmed = text.trim_end_matches('\n');
    let lines: Vec<&str> = trimmed.split('\n').collect();
    let hdr = parse_header_fields(lines[0]);
    if hdr.get("profile").map(String::as_str) != Some("generic") {
        return Err("not a generic payload".to_string());
    }
    if hdr.get("delta").map(String::as_str) != Some("true") {
        return Err("not a delta payload".to_string());
    }
    let mut d = GenericDeltaPayload {
        tool: hdr.get("tool").cloned().unwrap_or_default(),
        key: hdr.get("key").cloned().unwrap_or_default(),
        base_root: hdr.get("base_root").cloned().unwrap_or_default(),
        new_root: hdr.get("new_root").cloned().unwrap_or_default(),
        ..Default::default()
    };
    let mut fields_set = false;
    let mut i = 1;
    while i < lines.len() {
        let line = lines[i];
        if !line.starts_with("## ") {
            i += 1;
            continue;
        }
        let (name, count, fields, key_field) = parse_section_header(&line[3..])?;
        if d.key.is_empty() && !key_field.is_empty() {
            d.key = key_field;
        }
        if !fields_set && (name == "added" || name == "changed") {
            d.fields = fields.clone();
            fields_set = true;
        }
        i += 1;
        match name.as_str() {
            "added" | "changed" => {
                let mut rows = Vec::with_capacity(count);
                for _ in 0..count {
                    if i >= lines.len() {
                        return Err(format!(
                            "delta_invalid: fewer rows than declared count in ## {}",
                            name
                        ));
                    }
                    rows.push(parse_row(lines[i], &fields)?);
                    i += 1;
                }
                if name == "added" {
                    d.added = rows;
                } else {
                    d.changed = rows;
                }
            }
            "removed" => {
                for _ in 0..count {
                    if i >= lines.len() {
                        return Err(
                            "delta_invalid: fewer identities than declared count in ## removed"
                                .to_string(),
                        );
                    }
                    d.removed.push(scalar_to_value(parse_scalar(lines[i], true)?)?);
                    i += 1;
                }
            }
            other => {
                return Err(format!("delta_invalid: unknown delta section {}", other));
            }
        }
    }
    Ok(d)
}

// --- SHA-256 (local, no dependency) ---

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

fn sha256_hex(data: &[u8]) -> String {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA256_K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = String::with_capacity(64);
    for v in h {
        write!(out, "{:08x}", v).unwrap();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    fn row(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    fn orders_base() -> GenericSet {
        GenericSet {
            name: "orders".into(),
            key: "id".into(),
            fields: vec!["id".into(), "total".into(), "status".into(), "customer".into()],
            rows: vec![
                row(json!({"id": 1001, "total": 59.98, "status": "shipped", "customer": "Alice"})),
                row(json!({"id": 1002, "total": 29.99, "status": "pending", "customer": "Bob"})),
                row(json!({"id": 1003, "total": 129.50, "status": "shipped", "customer": "Carol"})),
            ],
        }
    }

    fn orders_next() -> GenericSet {
        GenericSet {
            name: "orders".into(),
            key: "id".into(),
            fields: vec!["id".into(), "total".into(), "status".into(), "customer".into()],
            rows: vec![
                row(json!({"id": 1002, "total": 29.99, "status": "shipped", "customer": "Bob"})),
                row(json!({"id": 1003, "total": 129.50, "status": "shipped", "customer": "Carol"})),
                row(json!({"id": 1004, "total": 75.00, "status": "pending", "customer": "Dave"})),
            ],
        }
    }

    #[test]
    fn roundtrip_by_root() {
        let base = orders_base();
        let next = orders_next();
        let d = diff_generic_sets(&base, &next).unwrap();
        assert_eq!((d.added.len(), d.changed.len(), d.removed.len()), (1, 1, 1));
        assert_eq!(d.new_root, generic_pack_root(&next));
        let result = verify_generic_delta(&base, &d, &generic_pack_root(&next)).unwrap();
        assert_eq!(generic_pack_root(&result), generic_pack_root(&next));
    }

    #[test]
    fn pack_root_row_order_invariant() {
        let a = orders_base();
        let mut b = orders_base();
        b.rows.swap(0, 2);
        assert_eq!(generic_pack_root(&a), generic_pack_root(&b));
    }

    #[test]
    fn canonical_cell_no_collision() {
        assert_eq!(canonical_cell(&Value::Null), "-");
        assert_eq!(canonical_cell(&json!(true)), "true");
        assert_eq!(canonical_cell(&json!("true")), "\"true\"");
        assert_eq!(canonical_cell(&json!("-")), "\"-\"");
        assert_eq!(canonical_cell(&json!(59.98)), "59.98");
        assert_eq!(canonical_cell(&json!("59.98")), "\"59.98\"");
        assert_eq!(canonical_cell(&json!("a\tb")), "\"a\\tb\"");
    }

    #[test]
    fn invariants() {
        let base = orders_base();
        let base_root = generic_pack_root(&base);

        let mut dup = orders_base();
        dup.rows
            .push(row(json!({"id": 1001, "total": 1.0, "status": "x", "customer": "y"})));
        assert!(diff_generic_sets(&dup, &orders_next())
            .unwrap_err()
            .contains("duplicate identity"));

        let mut sc = orders_next();
        sc.fields = vec!["id".into(), "total".into(), "status".into()];
        assert!(diff_generic_sets(&base, &sc)
            .unwrap_err()
            .contains("schema change"));

        let add_existing = GenericDeltaPayload {
            key: "id".into(),
            fields: base.fields.clone(),
            base_root: base_root.clone(),
            added: vec![row(json!({"id": 1001, "total": 1.0, "status": "s", "customer": "c"}))],
            ..Default::default()
        };
        assert!(verify_generic_delta(&base, &add_existing, "sha256:x")
            .unwrap_err()
            .contains("already exists"));

        let change_missing = GenericDeltaPayload {
            key: "id".into(),
            fields: base.fields.clone(),
            base_root: base_root.clone(),
            changed: vec![row(json!({"id": 9999, "total": 1.0, "status": "s", "customer": "c"}))],
            ..Default::default()
        };
        assert!(verify_generic_delta(&base, &change_missing, "sha256:x")
            .unwrap_err()
            .contains("not in base"));

        let remove_missing = GenericDeltaPayload {
            key: "id".into(),
            fields: base.fields.clone(),
            base_root: base_root.clone(),
            removed: vec![json!(9999)],
            ..Default::default()
        };
        assert!(verify_generic_delta(&base, &remove_missing, "sha256:x")
            .unwrap_err()
            .contains("not in base"));

        let wrong_base = GenericDeltaPayload {
            key: "id".into(),
            fields: base.fields.clone(),
            base_root: "sha256:wrong".into(),
            ..Default::default()
        };
        assert!(verify_generic_delta(&base, &wrong_base, &base_root)
            .unwrap_err()
            .contains("base_mismatch"));

        let d = diff_generic_sets(&base, &orders_next()).unwrap();
        assert!(verify_generic_delta(&base, &d, "sha256:deadbeef")
            .unwrap_err()
            .contains("root_mismatch"));
    }

    #[test]
    fn full_wire_roundtrip() {
        let base = orders_base();
        let (got, pr) = decode_generic_full(&encode_generic_full(&base, "orders_query")).unwrap();
        assert_eq!(generic_pack_root(&got), generic_pack_root(&base));
        assert_eq!(pr, generic_pack_root(&base));
    }

    #[test]
    fn end_to_end() {
        let base = orders_base();
        let next = orders_next();
        let (held, _) =
            decode_generic_full(&encode_generic_full(&base, "orders_query")).unwrap();
        let d = diff_generic_sets(&base, &next).unwrap();
        let parsed = decode_generic_delta(&encode_generic_delta(&d)).unwrap();
        let result = verify_generic_delta(&held, &parsed, &generic_pack_root(&next)).unwrap();
        assert_eq!(generic_pack_root(&result), generic_pack_root(&next));
    }

    #[test]
    fn nulls_and_string_keys() {
        let nulls = GenericSet {
            name: "items".into(),
            key: "id".into(),
            fields: vec!["id".into(), "total".into(), "status".into(), "customer".into()],
            rows: vec![
                row(json!({"id": 2001, "total": 10.0, "status": null, "customer": "Amy"})),
                row(json!({"id": 2002, "total": null, "status": "open", "customer": null})),
            ],
        };
        let (got, _) = decode_generic_full(&encode_generic_full(&nulls, "")).unwrap();
        assert_eq!(generic_pack_root(&got), generic_pack_root(&nulls));

        let sku = GenericSet {
            name: "parts".into(),
            key: "sku".into(),
            fields: vec!["sku".into(), "name".into(), "qty".into()],
            rows: vec![
                row(json!({"sku": "1001", "name": "Widget", "qty": 5})),
                row(json!({"sku": "A-200", "name": "Gadget", "qty": 3})),
            ],
        };
        let (got2, _) = decode_generic_full(&encode_generic_full(&sku, "")).unwrap();
        assert_eq!(generic_pack_root(&got2), generic_pack_root(&sku));
    }

    #[test]
    fn decode_malformed_fails_closed() {
        let cases = [
            "",
            "GCF profile=graph delta=true base_root=a new_root=b key=id\n",
            "GCF profile=generic pack_root=r key=id\n## t [1]{@id}\n1\n",
            "GCF profile=generic delta=true base_root=a new_root=b key=id\n## added [2]{@id,x}\n1|2\n",
            "GCF profile=generic delta=true base_root=a new_root=b key=id\n## added [1]{@id,x}\n1\n",
            "GCF profile=generic delta=true base_root=a new_root=b key=id\n## bogus [1]{@id}\n1\n",
            "GCF profile=generic delta=true base_root=a new_root=b key=id\n## added [01]{@id,x}\n1|2\n",
        ];
        for wire in cases {
            assert!(decode_generic_delta(wire).is_err(), "expected error for {:?}", wire);
        }
    }
}
