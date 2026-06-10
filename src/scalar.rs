//! Common scalar grammar for GCF v2.0.

use regex::Regex;
use std::sync::LazyLock;

static JSON_NUMBER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$").unwrap());

static NUMERIC_LIKE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[+-]?\.?\d").unwrap());

static BARE_KEY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z_][a-zA-Z0-9_]*$").unwrap());

/// Sentinel for absent fields in tabular rows.
#[derive(Debug, Clone, PartialEq)]
pub enum ScalarValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Missing,
    Attachment,
}

pub fn needs_quote(s: &str) -> bool {
    if s.is_empty() { return true; }
    if matches!(s, "-" | "~" | "^" | "true" | "false") { return true; }
    if JSON_NUMBER_RE.is_match(s) { return true; }
    if NUMERIC_LIKE_RE.is_match(s) { return true; }
    let bytes = s.as_bytes();
    if bytes[0] == b' ' || bytes[bytes.len() - 1] == b' ' { return true; }
    if bytes[0] == b'#' || bytes[0] == b'@' { return true; }
    for c in s.chars() {
        if c == '"' || c == '\\' || c == '|' || c == ',' || (c as u32) < 0x20
            || c == '\n' || c == '\r'
        {
            return true;
        }
    }
    false
}

pub fn quote_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn format_scalar(v: &serde_json::Value, delimiter: char) -> String {
    match v {
        serde_json::Value::Null => "-".to_string(),
        serde_json::Value::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        serde_json::Value::Number(n) => format_number(n),
        serde_json::Value::String(s) => {
            if needs_quote(s) || (delimiter != '\0' && s.contains(delimiter)) {
                quote_string(s)
            } else {
                s.clone()
            }
        }
        _ => "-".to_string(),
    }
}

pub fn format_number(n: &serde_json::Number) -> String {
    if let Some(i) = n.as_i64() {
        return i.to_string();
    }
    if let Some(u) = n.as_u64() {
        return u.to_string();
    }
    if let Some(f) = n.as_f64() {
        if f == 0.0 {
            if f.is_sign_negative() { return "-0".to_string(); }
            return "0".to_string();
        }
        let abs = f.abs();
        if abs >= 1e-6 && abs < 1e21 {
            let s = format!("{}", f);
            // Strip trailing .0 for integer-valued floats.
            if s.ends_with(".0") && f == f.trunc() {
                return s[..s.len() - 2].to_string();
            }
            return s;
        }
        // Exponent notation.
        let s = format!("{:e}", f);
        // Normalize: lowercase e, explicit sign, no leading zeros.
        if let Some(pos) = s.find('e') {
            let mantissa = s[..pos].trim_end_matches('0').trim_end_matches('.');
            let exp_part = &s[pos + 1..];
            let (sign, digits) = if exp_part.starts_with('-') {
                ("-", exp_part[1..].trim_start_matches('0'))
            } else {
                ("+", exp_part.trim_start_matches('+').trim_start_matches('0'))
            };
            let digits = if digits.is_empty() { "0" } else { digits };
            return format!("{}e{}{}", mantissa, sign, digits);
        }
        return s;
    }
    n.to_string()
}

pub fn is_bare_key(s: &str) -> bool {
    BARE_KEY_RE.is_match(s)
}

pub fn format_key(s: &str) -> String {
    if is_bare_key(s) { s.to_string() } else { quote_string(s) }
}

pub fn parse_scalar(s: &str, tabular_context: bool) -> Result<ScalarValue, String> {
    if s.is_empty() { return Ok(ScalarValue::Str(String::new())); }
    if s.starts_with('"') { return parse_quoted_string(s).map(ScalarValue::Str); }
    if s == "-" { return Ok(ScalarValue::Null); }
    if s == "~" {
        if !tabular_context { return Err("invalid_missing: ~ outside tabular row cell".into()); }
        return Ok(ScalarValue::Missing);
    }
    if s == "^" {
        if !tabular_context { return Err("invalid_attachment_marker: ^ outside tabular row cell".into()); }
        return Ok(ScalarValue::Attachment);
    }
    if s == "true" { return Ok(ScalarValue::Bool(true)); }
    if s == "false" { return Ok(ScalarValue::Bool(false)); }
    if JSON_NUMBER_RE.is_match(s) {
        if let Ok(f) = s.parse::<f64>() {
            if !s.contains('.') && !s.contains('e') && !s.contains('E') {
                if f.abs() <= (1i64 << 53) as f64 {
                    return Ok(ScalarValue::Int(f as i64));
                }
            }
            return Ok(ScalarValue::Float(f));
        }
    }
    Ok(ScalarValue::Str(s.to_string()))
}

pub fn parse_quoted_string(s: &str) -> Result<String, String> {
    let bytes = s.as_bytes();
    if bytes.len() < 2 || bytes[0] != b'"' {
        return Err("unterminated_quote".into());
    }
    let mut out = String::new();
    let mut i = 1;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            if i + 1 != bytes.len() {
                return Err("trailing_characters: after closing quote".into());
            }
            return Ok(out);
        }
        if bytes[i] == b'\\' {
            if i + 1 >= bytes.len() { return Err("unterminated_quote".into()); }
            i += 1;
            match bytes[i] {
                b'"' => out.push('"'),
                b'\\' => out.push('\\'),
                b'/' => out.push('/'),
                b'b' => out.push('\u{0008}'),
                b'f' => out.push('\u{000C}'),
                b'n' => out.push('\n'),
                b'r' => out.push('\r'),
                b't' => out.push('\t'),
                b'u' => {
                    if i + 4 >= bytes.len() { return Err("invalid_escape: incomplete unicode".into()); }
                    let hex = &s[i + 1..i + 5];
                    let code = u16::from_str_radix(hex, 16)
                        .map_err(|_| format!("invalid_escape: invalid unicode \\u{}", hex))?;
                    if (0xD800..=0xDBFF).contains(&code) {
                        if i + 10 >= bytes.len() || bytes[i + 5] != b'\\' || bytes[i + 6] != b'u' {
                            return Err("invalid_surrogate: isolated high surrogate".into());
                        }
                        let hex2 = &s[i + 7..i + 11];
                        let low = u16::from_str_radix(hex2, 16)
                            .map_err(|_| format!("invalid_surrogate: invalid low surrogate \\u{}", hex2))?;
                        if !(0xDC00..=0xDFFF).contains(&low) {
                            return Err("invalid_surrogate: expected low surrogate".into());
                        }
                        let combined = 0x10000 + (code as u32 - 0xD800) * 0x400 + (low as u32 - 0xDC00);
                        out.push(char::from_u32(combined).ok_or("invalid_surrogate")?);
                        i += 11;
                        continue;
                    }
                    if (0xDC00..=0xDFFF).contains(&code) {
                        return Err("invalid_surrogate: isolated low surrogate".into());
                    }
                    out.push(char::from_u32(code as u32).ok_or("invalid_escape")?);
                    i += 5;
                    continue;
                }
                c => return Err(format!("invalid_escape: unknown \\{}", c as char)),
            }
            i += 1;
            continue;
        }
        if bytes[i] < 0x20 {
            return Err(format!("invalid_escape: unescaped control U+{:04x}", bytes[i]));
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    Err("unterminated_quote".into())
}

pub fn split_respecting_quotes(s: &str, delim: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut escaped = false;
    for c in s.chars() {
        if escaped { current.push(c); escaped = false; continue; }
        if c == '\\' && in_quote { current.push(c); escaped = true; continue; }
        if c == '"' { in_quote = !in_quote; current.push(c); continue; }
        if c == delim && !in_quote { parts.push(current.clone()); current.clear(); continue; }
        current.push(c);
    }
    parts.push(current);
    parts
}

pub fn split_field_decl(s: &str) -> Result<Vec<String>, String> {
    if s.len() < 2 || !s.starts_with('{') {
        return Err(format!("invalid field declaration: {}", s));
    }
    let close = find_closing_brace(s).ok_or_else(|| format!("invalid field declaration: {}", s))?;
    let inner = &s[1..close];
    if inner.is_empty() { return Ok(Vec::new()); }
    let raw = split_respecting_quotes(inner, ',');
    let mut fields = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for f in raw {
        let f = f.trim();
        let name = if f.len() >= 2 && f.starts_with('"') && f.ends_with('"') {
            parse_quoted_string(f)?
        } else {
            if !is_bare_key(f) { return Err(format!("invalid field name: {}", f)); }
            f.to_string()
        };
        if !seen.insert(name.clone()) {
            return Err(format!("duplicate_field_name: {}", name));
        }
        fields.push(name);
    }
    Ok(fields)
}

pub fn find_closing_brace(s: &str) -> Option<usize> {
    let mut in_quote = false;
    let mut escaped = false;
    for (i, c) in s.chars().enumerate() {
        if escaped { escaped = false; continue; }
        if c == '\\' && in_quote { escaped = true; continue; }
        if c == '"' { in_quote = !in_quote; continue; }
        if c == '}' && !in_quote { return Some(i); }
    }
    None
}
