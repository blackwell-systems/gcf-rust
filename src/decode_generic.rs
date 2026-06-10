//! GCF v2.0 generic decoder: parses GCF generic or graph profile text into serde_json::Value.

use crate::decode::decode;
use crate::scalar::{
    find_closing_brace, parse_quoted_string, parse_scalar, split_field_decl,
    split_respecting_quotes, ScalarValue,
};
use serde_json::{Map, Number, Value};

/// Decode GCF v2.0 text into a generic `serde_json::Value`.
pub fn decode_generic(input: &str) -> Result<Value, String> {
    let input = input.trim_end_matches(['\n', '\r']);
    if input.is_empty() {
        return Err("missing_header: empty input".into());
    }

    let lines: Vec<&str> = input.split('\n').collect();
    let header = lines[0].trim_end_matches('\r');
    if !header.starts_with("GCF ") {
        return Err("missing_header: first line does not begin with GCF".into());
    }

    let profile = parse_header_profile(header)?;

    if profile == "graph" {
        let p = decode(input).map_err(|e| e.to_string())?;
        return Ok(payload_to_value(&p));
    }

    if profile != "generic" {
        return Err(format!("unknown_profile: {}", profile));
    }

    // Filter body.
    let mut content_lines: Vec<String> = Vec::new();
    let mut deferred_count = 0;
    let mut summary_line = String::new();

    for line in &lines[1..] {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        // Tab check.
        for c in line.chars() {
            if c == '\t' {
                return Err("tab_indentation: tabs in leading whitespace".into());
            }
            if c != ' ' {
                break;
            }
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with("# ") {
            continue;
        }
        if trimmed.starts_with("##! ") {
            summary_line = trimmed.to_string();
            continue;
        }
        if trimmed.starts_with("## ") && trimmed.contains("[?]") {
            deferred_count += 1;
        }
        content_lines.push(line.to_string());
    }

    if !summary_line.is_empty() && deferred_count > 0 {
        validate_summary_counts(&summary_line, deferred_count, &content_lines)?;
    }

    if content_lines.is_empty() {
        return Ok(Value::Object(Map::new()));
    }

    let first = content_lines[0].trim_start();

    // Root scalar.
    if first.starts_with('=') {
        if content_lines.len() > 1 {
            return Err("trailing_characters: extra lines after root scalar".into());
        }
        return scalar_to_value(&parse_scalar(first.strip_prefix("=").unwrap(), false)?);
    }

    // Root array.
    if first.starts_with("## [") {
        let (arr, _) = parse_array_from_header(&content_lines, 0, 0, &first[3..])?;
        return Ok(arr);
    }

    // Root object.
    let mut result = Map::new();
    parse_object_body(&content_lines, 0, 0, &mut result)?;
    Ok(Value::Object(result))
}

fn parse_header_profile(header: &str) -> Result<String, String> {
    let parts: Vec<&str> = header.split_whitespace().collect();
    if parts.len() < 2 {
        return Err("missing_profile".into());
    }
    let mut seen = std::collections::HashSet::new();
    let mut profile = String::new();
    for p in &parts[1..] {
        let eq = p
            .find('=')
            .ok_or_else(|| format!("malformed_header_field: {}", p))?;
        let key = &p[..eq];
        if !seen.insert(key.to_string()) {
            return Err(format!("duplicate_header_field: {}", key));
        }
        if key == "profile" {
            profile = p[eq + 1..].to_string();
        }
    }
    if profile.is_empty() {
        return Err("missing_profile".into());
    }
    Ok(profile)
}

fn scalar_to_value(sv: &ScalarValue) -> Result<Value, String> {
    match sv {
        ScalarValue::Null => Ok(Value::Null),
        ScalarValue::Bool(b) => Ok(Value::Bool(*b)),
        ScalarValue::Int(i) => Ok(Value::Number(Number::from(*i))),
        ScalarValue::Float(f) => Ok(Value::Number(
            Number::from_f64(*f).unwrap_or_else(|| Number::from(0)),
        )),
        ScalarValue::Str(s) => Ok(Value::String(s.clone())),
        ScalarValue::Missing => Err("invalid_missing: ~ in non-tabular context".into()),
        ScalarValue::Attachment => {
            Err("invalid_attachment_marker: ^ in non-tabular context".into())
        }
    }
}

fn parse_object_body(
    lines: &[String],
    start: usize,
    depth: usize,
    out: &mut Map<String, Value>,
) -> Result<usize, String> {
    let ind = "  ".repeat(depth);
    let mut i = start;
    while i < lines.len() {
        let line = &lines[i];
        if depth > 0 && !line.starts_with(&ind) {
            break;
        }
        let content = if depth > 0 {
            &line[ind.len()..]
        } else {
            line.as_str()
        };
        if !content.is_empty() && content.starts_with(' ') {
            return Err("invalid_indent: indentation increases by more than one level".into());
        }

        // Array section.
        if let Some(hdr) = content.strip_prefix("## ") {
            if let Some(bi) = hdr.find(" [") {
                let name = parse_key_from_header(&hdr[..bi])?;
                check_dup(out, &name)?;
                let (arr, consumed) = parse_array_from_header(lines, i, depth, &hdr[bi..])?;
                out.insert(name, arr);
                i += consumed;
                continue;
            }
            let name = parse_key_from_header(hdr)?;
            check_dup(out, &name)?;
            i += 1;
            let mut nested = Map::new();
            let consumed = parse_object_body(lines, i, depth + 1, &mut nested)?;
            out.insert(name, Value::Object(nested));
            i += consumed;
            continue;
        }

        // Inline array.
        if !content.starts_with('@') && !content.starts_with("##") {
            if let Some(bracket_idx) = content.find('[') {
                if bracket_idx > 0 {
                    let rest = &content[bracket_idx..];
                    if let Some(close_idx) = rest.find(']') {
                        let after = &rest[close_idx + 1..];
                        if after.starts_with(": ") || after == ":" {
                            let name = parse_key_from_header(&content[..bracket_idx])?;
                            check_dup(out, &name)?;
                            let (arr, _) = parse_array_from_header(lines, i, depth, rest)?;
                            out.insert(name, arr);
                            i += 1;
                            continue;
                        }
                    }
                }
            }
        }

        // Key=value.
        if let Some(eq_idx) = find_kv_split(content) {
            if eq_idx > 0 {
                let name = parse_key_from_header(&content[..eq_idx])?;
                check_dup(out, &name)?;
                let val = scalar_to_value(&parse_scalar(&content[eq_idx + 1..], false)?)?;
                out.insert(name, val);
                i += 1;
                continue;
            }
        }

        i += 1;
    }
    Ok(i - start)
}

fn find_kv_split(s: &str) -> Option<usize> {
    if s.is_empty() {
        return None;
    }
    let bytes = s.as_bytes();
    if bytes[0] == b'"' {
        let mut i = 1;
        while i < bytes.len() {
            if bytes[i] == b'\\' {
                i += 2;
                continue;
            }
            if bytes[i] == b'"' {
                return if i + 1 < bytes.len() && bytes[i + 1] == b'=' {
                    Some(i + 1)
                } else {
                    None
                };
            }
            i += 1;
        }
        return None;
    }
    s.find('=')
}

fn parse_key_from_header(s: &str) -> Result<String, String> {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') {
        parse_quoted_string(s)
    } else {
        Ok(s.to_string())
    }
}

fn check_dup(map: &Map<String, Value>, key: &str) -> Result<(), String> {
    if map.contains_key(key) {
        Err(format!("duplicate_key: {}", key))
    } else {
        Ok(())
    }
}

fn parse_array_from_header(
    lines: &[String],
    header_line: usize,
    depth: usize,
    bracket_part: &str,
) -> Result<(Value, usize), String> {
    let bp = bracket_part.trim_start();
    if !bp.starts_with('[') {
        return Err("invalid_count".into());
    }
    let close = bp.find(']').ok_or("invalid_count")?;
    let count_str = &bp[1..close];
    let after = &bp[close + 1..];
    let count: i64 = if count_str == "?" {
        -1
    } else {
        parse_count(count_str)? as i64
    };

    if count == 0 && !after.starts_with('{') && !after.starts_with(':') {
        return Ok((Value::Array(vec![]), 1));
    }

    // Inline.
    if after.starts_with(": ") || after == ":" {
        let vals_str = if after.starts_with(": ") {
            after.strip_prefix(": ").unwrap()
        } else {
            ""
        };
        if vals_str.is_empty() {
            if count > 0 {
                return Err(format!("count_mismatch: declared {}, got 0", count));
            }
            return Ok((Value::Array(vec![]), 1));
        }
        let vals = split_respecting_quotes(vals_str, ',');
        if count >= 0 && vals.len() as i64 != count {
            return Err(format!(
                "count_mismatch: declared {}, got {}",
                count,
                vals.len()
            ));
        }
        let parsed: Result<Vec<Value>, String> = vals
            .iter()
            .map(|v| scalar_to_value(&parse_scalar(v.trim(), false)?))
            .collect();
        return Ok((Value::Array(parsed?), 1));
    }

    // Tabular.
    if after.starts_with('{') {
        let brace_end = find_closing_brace(after).ok_or("invalid field declaration")?;
        let fields = split_field_decl(&after[..brace_end + 1])?;
        let (rows, consumed) = parse_tabular_body(lines, header_line + 1, depth, &fields, count)?;
        if count >= 0 && rows.len() as i64 != count {
            return Err(format!(
                "count_mismatch: declared {}, got {}",
                count,
                rows.len()
            ));
        }
        return Ok((Value::Array(rows), consumed + 1));
    }

    // Expanded.
    let (items, consumed) = parse_expanded_body(lines, header_line + 1, depth)?;
    if count >= 0 && items.len() as i64 != count {
        return Err(format!(
            "count_mismatch: declared {}, got {}",
            count,
            items.len()
        ));
    }
    Ok((Value::Array(items), consumed + 1))
}

fn parse_tabular_body(
    lines: &[String],
    start: usize,
    depth: usize,
    fields: &[String],
    expected_count: i64,
) -> Result<(Vec<Value>, usize), String> {
    let ind = "  ".repeat(depth);
    let mut rows: Vec<Value> = Vec::new();
    let mut i = start;

    while i < lines.len() {
        let line = &lines[i];
        let content = if depth > 0 {
            if !line.starts_with(&ind) {
                break;
            }
            &line[ind.len()..]
        } else {
            line.as_str()
        };
        if content.starts_with("## ") || content.starts_with("##!") {
            break;
        }
        if !content.is_empty() && content.starts_with(' ') {
            let trimmed = content.trim_start();
            if trimmed.starts_with('.') {
                return Err(format!("orphan_attachment: {}", trimmed));
            }
            break;
        }

        let mut row_data = content;
        let mut row_has_id = false;
        if row_data.starts_with('@') {
            if let Some(sp) = row_data.find(' ') {
                row_data = &row_data[sp + 1..];
                row_has_id = true;
            }
        }

        let vals = split_respecting_quotes(row_data, '|');
        if vals.len() != fields.len() {
            return Err(format!(
                "row_width_mismatch: expected {}, got {}",
                fields.len(),
                vals.len()
            ));
        }

        let mut cell_values: Map<String, Value> = Map::new();
        let mut attachment_fields: Vec<String> = Vec::new();
        let mut missing_fields: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for (j, f) in fields.iter().enumerate() {
            let parsed = parse_scalar(&vals[j], true)?;
            match parsed {
                ScalarValue::Missing => {
                    missing_fields.insert(f.clone());
                }
                ScalarValue::Attachment => {
                    attachment_fields.push(f.clone());
                }
                _ => {
                    cell_values.insert(f.clone(), scalar_to_value(&parsed)?);
                }
            }
        }
        i += 1;

        // Parse attachments.
        let mut attachment_values: Map<String, Value> = Map::new();
        if row_has_id && !attachment_fields.is_empty() {
            let att_indent = format!("{}  ", ind);
            while i < lines.len() {
                let al = &lines[i];
                if !al.starts_with(&att_indent) {
                    break;
                }
                let ac = &al[att_indent.len()..];
                if !ac.starts_with('.') {
                    break;
                }
                let (name, val, consumed) = parse_attachment(lines, i, &ac[1..], depth + 2)?;
                if attachment_values.contains_key(&name) {
                    return Err(format!("duplicate_attachment: {}", name));
                }
                attachment_values.insert(name, val);
                i += consumed;
            }
            for f in &attachment_fields {
                if !attachment_values.contains_key(f) {
                    return Err(format!("missing_attachment: {}", f));
                }
            }
        }

        // Check orphan.
        if !row_has_id || attachment_fields.is_empty() {
            let att_indent = format!("{}  ", ind);
            if i < lines.len() && lines[i].starts_with(&att_indent) {
                let peek = &lines[i][att_indent.len()..];
                if peek.starts_with('.') {
                    return Err(format!("orphan_attachment: {}", peek));
                }
            }
        }

        // Build row in field order.
        let mut row = Map::new();
        for f in fields {
            if missing_fields.contains(f) {
                continue;
            }
            if let Some(v) = cell_values.remove(f) {
                row.insert(f.clone(), v);
                continue;
            }
            if let Some(v) = attachment_values.remove(f) {
                row.insert(f.clone(), v);
                continue;
            }
        }
        rows.push(Value::Object(row));

        if expected_count >= 0 && rows.len() as i64 >= expected_count {
            break;
        }
    }
    Ok((rows, i - start))
}

fn parse_attachment(
    lines: &[String],
    line_idx: usize,
    rest: &str,
    depth: usize,
) -> Result<(String, Value, usize), String> {
    let (name, after_name) = if rest.starts_with('"') {
        let mut close_idx = None;
        let bytes = rest.as_bytes();
        let mut j = 1;
        while j < bytes.len() {
            if bytes[j] == b'\\' {
                j += 2;
                continue;
            }
            if bytes[j] == b'"' {
                close_idx = Some(j);
                break;
            }
            j += 1;
        }
        let ci = close_idx.ok_or("unterminated_quote")?;
        let name = parse_quoted_string(&rest[..ci + 1])?;
        (name, rest[ci + 1..].trim_start())
    } else {
        let sp = rest
            .find(' ')
            .ok_or_else(|| format!("invalid attachment: {}", rest))?;
        (rest[..sp].to_string(), rest[sp..].trim_start())
    };

    if after_name.starts_with("{}") {
        let mut nested = Map::new();
        let consumed = parse_object_body(lines, line_idx + 1, depth, &mut nested)?;
        return Ok((name, Value::Object(nested), consumed + 1));
    }
    if after_name.starts_with('[') {
        let (arr, consumed) = parse_array_from_header(lines, line_idx, depth, after_name)?;
        return Ok((name, arr, consumed));
    }
    Err(format!("invalid attachment form: {}", after_name))
}

fn parse_expanded_body(
    lines: &[String],
    start: usize,
    depth: usize,
) -> Result<(Vec<Value>, usize), String> {
    let ind = "  ".repeat(depth);
    let mut items: Vec<Value> = Vec::new();
    let mut i = start;

    while i < lines.len() {
        let line = &lines[i];
        let content = if depth > 0 {
            if !line.starts_with(&ind) {
                break;
            }
            &line[ind.len()..]
        } else {
            line.as_str()
        };
        if content.starts_with("## ") || content.starts_with("##!") {
            break;
        }
        if !content.starts_with('@') {
            break;
        }

        let sp = match content.find(' ') {
            Some(s) => s,
            None => break,
        };

        // Validate item ID.
        let id_str = &content[1..sp];
        if let Ok(id) = id_str.parse::<usize>() {
            if id != items.len() {
                return Err(format!(
                    "invalid_item_id: expected @{}, got @{}",
                    items.len(),
                    id_str
                ));
            }
        }

        let marker = &content[sp + 1..];

        if marker.starts_with('=') {
            let val = scalar_to_value(&parse_scalar(marker.strip_prefix("=").unwrap(), false)?)?;
            items.push(val);
            i += 1;
            continue;
        }
        if marker.starts_with("{}") {
            let mut nested = Map::new();
            i += 1;
            let consumed = parse_object_body(lines, i, depth + 1, &mut nested)?;
            items.push(Value::Object(nested));
            i += consumed;
            continue;
        }
        if marker.starts_with('[') {
            let (arr, consumed) = parse_array_from_header(lines, i, depth + 1, marker)?;
            items.push(arr);
            i += consumed;
            continue;
        }
        break;
    }
    Ok((items, i - start))
}

fn parse_count(s: &str) -> Result<usize, String> {
    if s == "0" {
        return Ok(0);
    }
    if s.is_empty() || s.starts_with('0') {
        return Err(format!("invalid_count: {}", s));
    }
    s.parse::<usize>()
        .map_err(|_| format!("invalid_count: {}", s))
}

fn payload_to_value(p: &crate::types::Payload) -> Value {
    let syms: Vec<Value> = p
        .symbols
        .iter()
        .map(|s| {
            serde_json::json!({
                "qualifiedName": s.qualified_name,
                "kind": s.kind,
                "score": s.score,
                "provenance": s.provenance,
                "distance": s.distance,
            })
        })
        .collect();
    let edges: Vec<Value> = p
        .edges
        .iter()
        .map(|e| {
            serde_json::json!({
                "source": e.source,
                "target": e.target,
                "edgeType": e.edge_type,
                "status": e.status,
            })
        })
        .collect();
    serde_json::json!({
        "tool": p.tool,
        "tokenBudget": p.token_budget,
        "tokensUsed": p.tokens_used,
        "packRoot": p.pack_root,
        "symbols": syms,
        "edges": edges,
    })
}

fn validate_summary_counts(
    summary_line: &str,
    deferred_count: usize,
    content_lines: &[String],
) -> Result<(), String> {
    let counts_str = summary_line
        .split_whitespace()
        .find(|p| p.starts_with("counts="))
        .map(|p| &p[7..])
        .unwrap_or("");
    if counts_str.is_empty() {
        return Ok(());
    }
    let count_vals: Vec<&str> = counts_str.split(',').collect();
    if count_vals.len() != deferred_count {
        return Err(format!(
            "count_mismatch: summary has {} count entries but {} deferred sections",
            count_vals.len(),
            deferred_count
        ));
    }
    let mut actual_counts: Vec<usize> = Vec::new();
    let mut in_deferred = false;
    let mut current_count = 0;
    for line in content_lines {
        let trimmed = line.trim_start();
        if trimmed.starts_with("## ") && trimmed.contains("[?]") {
            if in_deferred {
                actual_counts.push(current_count);
            }
            in_deferred = true;
            current_count = 0;
            continue;
        }
        if trimmed.starts_with("## ") {
            if in_deferred {
                actual_counts.push(current_count);
                in_deferred = false;
            }
            continue;
        }
        if in_deferred && !trimmed.starts_with(' ') && !trimmed.starts_with('.') {
            current_count += 1;
        }
    }
    if in_deferred {
        actual_counts.push(current_count);
    }
    for (idx, cv) in count_vals.iter().enumerate() {
        let declared: usize = cv
            .parse()
            .map_err(|_| format!("count_mismatch: invalid count value '{}'", cv))?;
        if idx < actual_counts.len() && declared != actual_counts[idx] {
            return Err(format!(
                "count_mismatch: section {} declared {} in summary, actual {}",
                idx, declared, actual_counts[idx]
            ));
        }
    }
    Ok(())
}
