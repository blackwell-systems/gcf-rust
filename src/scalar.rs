//! Common scalar grammar for GCF v2.0.

use regex::Regex;
use std::sync::LazyLock;

// Digits are ASCII 0-9 (SPEC 2.3): \d is avoided because the regex crate's \d also
// matches Unicode decimal digits (\p{Nd}), which would accept e.g. "1.<U+0665>".
static JSON_NUMBER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?$").unwrap()
});

static NUMERIC_LIKE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[+-]\.?[0-9]|^\.[0-9]|^0[0-9]").unwrap());

static BARE_KEY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z_][a-zA-Z0-9_]*$").unwrap());

static INLINE_ARRAY_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[[^\]]*\]\s*:").unwrap());

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
    if s.is_empty() {
        return true;
    }
    if matches!(s, "-" | "~" | "^" | "true" | "false") {
        return true;
    }
    // A value shaped like an inline-schema attachment marker (^{...}) would decode
    // as an attachment and lose the string, so it must be quoted (SPEC 2.4).
    {
        let b = s.as_bytes();
        if b.len() >= 3 && b[0] == b'^' && b[1] == b'{' && b[b.len() - 1] == b'}' {
            return true;
        }
    }
    if JSON_NUMBER_RE.is_match(s) {
        return true;
    }
    if NUMERIC_LIKE_RE.is_match(s) {
        return true;
    }
    let bytes = s.as_bytes();
    if bytes[0] == b' ' || bytes[bytes.len() - 1] == b' ' {
        return true;
    }
    if bytes[0] == b'#' || bytes[0] == b'@' || bytes[0] == b'.' {
        return true;
    }
    if INLINE_ARRAY_RE.is_match(s) {
        return true;
    }
    for c in s.chars() {
        let code = c as u32;
        if c == '"'
            || c == '\\'
            || c == '|'
            || c == ','
            || code < 0x20
            || c == '\n'
            || c == '\r'
            || ((0x80..=0x9F).contains(&code)) // C1 controls
            || (code > 0x7F && matches!(code, 0xA0 | 0x1680 | 0x2028 | 0x2029 | 0x202F | 0x205F | 0x3000 | 0xFEFF))
            || (0x2000..=0x200A).contains(&code)
        // Unicode spaces
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
    match format_number_checked(n) {
        Ok(s) => s,
        // The generic encoder path is infallible (returns String), so an out-of-domain
        // host integer that reaches this formatter is emitted as its lossless decimal
        // digits rather than approximated through f64. A strict, fail-loud encode path
        // is enforced at the JSON->value bridge (SPEC 2.3.2); mirrors the Go stopgap in
        // encode_helpers.go (uint64 > int64 max -> string).
        Err((s, _)) => s,
    }
}

/// Format a number for the wire, returning an out-of-range error (as the offending
/// decimal string plus a message) when a host integer is outside the int64 domain.
/// The token shape follows the numeric domain (SPEC 2.3.1, 2.3.2): an integer value is
/// emitted as plain decimal digits across the whole closed interval [-2^63, 2^63-1]
/// (including the minimum -2^63, whose magnitude is exactly 2^63) and is never rendered
/// in exponent form; a double uses plain decimal iff 1e-6 <= abs < 2^63.
pub fn format_number_checked(n: &serde_json::Number) -> Result<String, (String, String)> {
    // Token shape follows the numeric domain (SPEC 2.3.2): inspect the lexeme FIRST. A
    // bare-integer lexeme (no '.'/'e'/'E') is an int64-domain integer; if it does not
    // parse to i64 it is outside [-2^63, 2^63-1] and is out of range. This precedes the
    // as_f64 branch because a Number backed by serde_json's arbitrary_precision returns
    // Some from as_f64 even for an over-long integer lexeme (e.g. 10^20 -> 1e20), which
    // would otherwise be misclassified as an in-range double. A u64 in [2^63, 2^64-1] is
    // likewise a bare integer over the edge and is caught here.
    let lexeme = n.to_string();
    if !lexeme.contains(['.', 'e', 'E']) {
        return match lexeme.parse::<i64>() {
            Ok(i) => Ok(i.to_string()),
            Err(_) => Err((lexeme.clone(), out_of_range_message(&lexeme))),
        };
    }
    if let Some(f) = n.as_f64() {
        if f == 0.0 {
            // Negative zero canonicalizes to 0 (SPEC 2.3.1): -0.0 equals 0.0 by value.
            return Ok("0".to_string());
        }
        let abs = f.abs();
        // Plain decimal iff 1e-6 <= abs < 2^53 (9007199254740992.0). Every double at or
        // above 2^53 is integer-valued, so a plain rendering would emit a bare-integer
        // token: indistinguishable from an int64 on the wire and beyond the binary64
        // safe-integer range (2^53-1), so a JavaScript decoder rejects it under its
        // default policy. Exponent shape keeps bare tokens int64 and decimal/exponent
        // tokens doubles (SPEC 2.3.1).
        if (1e-6..9007199254740992.0).contains(&abs) {
            let s = format!("{}", f);
            // Strip trailing .0 for integer-valued floats.
            if s.ends_with(".0") && f == f.trunc() {
                return Ok(s[..s.len() - 2].to_string());
            }
            return Ok(s);
        }
        // Exponent notation.
        let s = format!("{:e}", f);
        // Normalize: lowercase e, explicit sign, no leading zeros.
        if let Some(pos) = s.find('e') {
            let mantissa = s[..pos].trim_end_matches('0').trim_end_matches('.');
            let exp_part = &s[pos + 1..];
            let (sign, digits) = if let Some(rest) = exp_part.strip_prefix('-') {
                ("-", rest.trim_start_matches('0'))
            } else if let Some(rest) = exp_part.strip_prefix('+') {
                ("+", rest.trim_start_matches('0'))
            } else {
                ("+", exp_part.trim_start_matches('0'))
            };
            let digits = if digits.is_empty() { "0" } else { digits };
            return Ok(format!("{}e{}{}", mantissa, sign, digits));
        }
        return Ok(s);
    }
    // Reached only for a decimal/exponent lexeme that as_f64 cannot represent (bare
    // integers are handled by the lexeme guard above). It is in the double domain, so
    // emit its lexeme rather than erroring.
    Ok(n.to_string())
}

/// Build the actionable out-of-range message for a value outside the int64 domain.
/// Contains the substring `out_of_range`, names the offending value, states the range,
/// and gives the remediation (SPEC 2.3.2).
pub fn out_of_range_message(value: &str) -> String {
    format!(
        "out_of_range: integer {} is outside the canonical int64 domain [-9223372036854775808, 9223372036854775807]; model larger values as strings (SPEC 2.3.2)",
        value
    )
}

pub fn is_bare_key(s: &str) -> bool {
    BARE_KEY_RE.is_match(s)
}

pub fn format_key(s: &str) -> String {
    if is_bare_key(s) {
        s.to_string()
    } else {
        quote_string(s)
    }
}

pub fn parse_scalar(s: &str, tabular_context: bool) -> Result<ScalarValue, String> {
    if s.is_empty() {
        return Ok(ScalarValue::Str(String::new()));
    }
    if s.starts_with('"') {
        return parse_quoted_string(s).map(ScalarValue::Str);
    }
    if s == "-" {
        return Ok(ScalarValue::Null);
    }
    if s == "~" {
        if !tabular_context {
            return Err("invalid_missing: ~ outside tabular row cell".into());
        }
        return Ok(ScalarValue::Missing);
    }
    if s == "^" {
        if !tabular_context {
            return Err("invalid_attachment_marker: ^ outside tabular row cell".into());
        }
        return Ok(ScalarValue::Attachment);
    }
    if s == "true" {
        return Ok(ScalarValue::Bool(true));
    }
    if s == "false" {
        return Ok(ScalarValue::Bool(false));
    }
    if JSON_NUMBER_RE.is_match(s) {
        // Token shape follows the numeric domain (SPEC 2.3.2): a bare-integer literal
        // (no '.', no 'e'/'E') is an int64-domain integer and MUST parse to an exact
        // i64, not through f64 (which silently approximates magnitudes beyond 2^53).
        // `-0` is a bare token and parses to i64 0 (canonical zero, SPEC 2.3.1). A
        // decimal or exponent literal is a double.
        if !s.contains('.') && !s.contains('e') && !s.contains('E') {
            match s.parse::<i64>() {
                Ok(n) => return Ok(ScalarValue::Int(n)),
                // parse::<i64> returns Err only on overflow here (the JSON number regex
                // already guaranteed a valid integer lexeme), i.e. the literal is outside
                // the int64 domain: raise an out-of-range error rather than approximating.
                Err(_) => return Err(out_of_range_message(s)),
            }
        }
        if let Ok(f) = s.parse::<f64>() {
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
            if i + 1 >= bytes.len() {
                return Err("unterminated_quote".into());
            }
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
                    if i + 4 >= bytes.len() {
                        return Err("invalid_escape: incomplete unicode".into());
                    }
                    let hex = &s[i + 1..i + 5];
                    let code = u16::from_str_radix(hex, 16)
                        .map_err(|_| format!("invalid_escape: invalid unicode \\u{}", hex))?;
                    if (0xD800..=0xDBFF).contains(&code) {
                        if i + 10 >= bytes.len() || bytes[i + 5] != b'\\' || bytes[i + 6] != b'u' {
                            return Err("invalid_surrogate: isolated high surrogate".into());
                        }
                        let hex2 = &s[i + 7..i + 11];
                        let low = u16::from_str_radix(hex2, 16).map_err(|_| {
                            format!("invalid_surrogate: invalid low surrogate \\u{}", hex2)
                        })?;
                        if !(0xDC00..=0xDFFF).contains(&low) {
                            return Err("invalid_surrogate: expected low surrogate".into());
                        }
                        let combined =
                            0x10000 + (code as u32 - 0xD800) * 0x400 + (low as u32 - 0xDC00);
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
            return Err(format!(
                "invalid_escape: unescaped control U+{:04x}",
                bytes[i]
            ));
        }
        // Literal character: may be a multi-byte UTF-8 sequence. `bytes[i] as char`
        // would reinterpret each byte as Latin-1 and corrupt it, so copy the whole
        // char and advance by its UTF-8 length.
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    Err("unterminated_quote".into())
}

pub fn split_respecting_quotes(s: &str, delim: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut escaped = false;
    for c in s.chars() {
        if escaped {
            current.push(c);
            escaped = false;
            continue;
        }
        if c == '\\' && in_quote {
            current.push(c);
            escaped = true;
            continue;
        }
        if c == '"' {
            in_quote = !in_quote;
            current.push(c);
            continue;
        }
        if c == delim && !in_quote {
            parts.push(current.clone());
            current.clear();
            continue;
        }
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
    if inner.is_empty() {
        return Ok(Vec::new());
    }
    let raw = split_respecting_quotes(inner, ',');
    let mut fields = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for f in raw {
        let f = f.trim();
        let name = if f.len() >= 2 && f.starts_with('"') && f.ends_with('"') {
            parse_quoted_string(f)?
        } else {
            if !is_bare_key(f) {
                return Err(format!("invalid field name: {}", f));
            }
            f.to_string()
        };
        if !seen.insert(name.clone()) {
            return Err(format!("duplicate_field_name: {}", name));
        }
        fields.push(name);
    }
    Ok(fields)
}

/// Returns the BYTE offset of the closing brace that matches the opening one.
/// Callers slice `s` by byte offset (`&s[..idx + 1]`), so this must return a
/// byte index: `.chars().enumerate()` yields char positions, which truncate the
/// slice when the field name contains a multibyte char.
pub fn find_closing_brace(s: &str) -> Option<usize> {
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
        if c == '}' && !in_quote {
            return Some(i);
        }
    }
    None
}
