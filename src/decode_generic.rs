#![allow(
    clippy::collapsible_if,
    clippy::collapsible_else_if,
    clippy::manual_strip,
    clippy::type_complexity
)]
//! GCF generic decoder: parses GCF generic or graph profile text into serde_json::Value.

use crate::decode::decode;
use crate::scalar::{
    find_closing_brace, parse_quoted_string, parse_scalar, split_field_decl,
    split_respecting_quotes, ScalarValue,
};
use serde_json::{Map, Number, Value};

/// Decode GCF text into a generic `serde_json::Value`.
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
    parse_tabular_body_with_shared(lines, start, depth, fields, expected_count, None)
}

/// v3 tabular body parser with inline schemas, no-indent attachments, and shared array schemas.
fn parse_tabular_body_with_shared(
    lines: &[String],
    start: usize,
    depth: usize,
    fields: &[String],
    expected_count: i64,
    parent_shared_schemas: Option<&std::collections::HashMap<String, Vec<String>>>,
) -> Result<(Vec<Value>, usize), String> {
    let ind = "  ".repeat(depth);
    let mut rows: Vec<Value> = Vec::new();
    let mut i = start;

    // Track inline schemas declared by ^{fields}.
    let mut inline_schemas: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    // Track shared array schemas: field -> fields list (from first row's attachment).
    let mut shared_array_schemas: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    if let Some(parent) = parent_shared_schemas {
        for (k, v) in parent {
            shared_array_schemas.insert(k.clone(), v.clone());
        }
    }

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
        if !content.is_empty() && content.as_bytes()[0] == b' ' {
            let trimmed = content.trim_start();
            if trimmed.starts_with('.') {
                break; // attachment lines handled below
            }
            break;
        }

        // Strip @N prefix (must be @digits).
        let mut row_data = content;
        let mut row_has_id = false;
        if row_data.starts_with('@') {
            if let Some(sp) = row_data.find(' ') {
                let id_str = &row_data[1..sp];
                let valid_id = !id_str.is_empty() && id_str.bytes().all(|b| b.is_ascii_digit());
                if valid_id {
                    row_data = &row_data[sp + 1..];
                    row_has_id = true;
                }
            }
        }

        let vals = split_respecting_quotes(row_data, '|');
        if vals.len() != fields.len() {
            return Err(format!(
                "row_width_mismatch: expected {} fields, got {}",
                fields.len(),
                vals.len()
            ));
        }

        let mut row: Map<String, Value> = Map::new();
        let mut traditional_att_fields: Vec<String> = Vec::new();
        let mut inline_att_fields: Vec<String> = Vec::new();
        let mut inline_att_order: Vec<String> = Vec::new();

        for (j, f) in fields.iter().enumerate() {
            let cell_val = &vals[j];

            // Check for ^{fields} inline schema declaration.
            if cell_val.starts_with("^{") && cell_val.ends_with('}') {
                let schema_str = &cell_val[1..]; // "{field1,field2,...}"
                let ifs = split_field_decl(schema_str)?;
                inline_schemas.insert(f.clone(), ifs);
                inline_att_fields.push(f.clone());
                inline_att_order.push(f.clone());
                continue;
            }

            let parsed = parse_scalar(cell_val, true)?;
            match parsed {
                ScalarValue::Missing => {
                    // absent: skip
                }
                ScalarValue::Attachment => {
                    // Check if this field has a stored inline schema.
                    if inline_schemas.contains_key(f) {
                        inline_att_fields.push(f.clone());
                        inline_att_order.push(f.clone());
                    } else {
                        traditional_att_fields.push(f.clone());
                    }
                }
                _ => {
                    row.insert(f.clone(), scalar_to_value(&parsed)?);
                }
            }
        }

        i += 1;

        // Build ordered list of expected attachment fields from cell order (preserving field order).
        let mut all_att_fields: Vec<String> = Vec::new();
        for f in fields {
            let is_trad = traditional_att_fields.iter().any(|tf| tf == f);
            let is_inline = inline_att_fields.iter().any(|inf| inf == f);
            if is_trad || is_inline {
                all_att_fields.push(f.clone());
            }
        }

        // Check for orphan attachments when row has ID but no ^ cells.
        if row_has_id && all_att_fields.is_empty() {
            if i < lines.len() {
                let peek_line = &lines[i];
                let peek_content = if peek_line.starts_with(&format!("{}  ", ind)) {
                    &peek_line[ind.len() + 2..]
                } else if peek_line.starts_with(&ind) {
                    &peek_line[ind.len()..]
                } else {
                    ""
                };
                if peek_content.starts_with('.') {
                    let (orphan_name, _) = parse_attachment_name(&peek_content[1..]);
                    return Err(format!(
                        "orphan_attachment: .{} without matching ^ cell",
                        orphan_name
                    ));
                }
            }
        }

        if row_has_id && !all_att_fields.is_empty() {
            let mut resolved_attachments: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            let mut inline_idx: usize = 0;

            while i < lines.len() && resolved_attachments.len() < all_att_fields.len() {
                let a_line = &lines[i];
                let a_content = if a_line.starts_with(&format!("{}  ", ind)) {
                    &a_line[ind.len() + 2..]
                } else if a_line.starts_with(&ind) {
                    &a_line[ind.len()..]
                } else {
                    break;
                };

                // Line starts with ".": traditional or prefixed inline attachment.
                if a_content.starts_with('.') {
                    let rest = &a_content[1..];
                    let (att_name, after_name_raw) = parse_attachment_name(rest);
                    let after_name = after_name_raw.trim_start();

                    // Check orphan: attachment for field not in all_att_fields.
                    let is_expected = all_att_fields.iter().any(|af| af == &att_name);
                    if !is_expected {
                        return Err(format!(
                            "orphan_attachment: {} without matching ^ cell",
                            att_name
                        ));
                    }
                    // Check duplicate.
                    if resolved_attachments.contains(&att_name) {
                        return Err(format!("duplicate_attachment: {}", att_name));
                    }

                    // Check if this field has inline schema and data is pipe-delimited (not {} or [).
                    if let Some(ifs) = inline_schemas.get(&att_name) {
                        if !after_name.starts_with("{}") && !after_name.starts_with('[') {
                            // Prefixed inline data: .fieldname val1|val2|...
                            let inline_vals = split_respecting_quotes(after_name, '|');
                            if inline_vals.len() != ifs.len() {
                                return Err(format!(
                                    "inline_width_mismatch: {} expected {}, got {}",
                                    att_name,
                                    ifs.len(),
                                    inline_vals.len()
                                ));
                            }
                            let mut obj = Map::new();
                            for (k, inf) in ifs.iter().enumerate() {
                                let p = parse_scalar(&inline_vals[k], true)?;
                                match p {
                                    ScalarValue::Missing => {}
                                    _ => {
                                        obj.insert(inf.clone(), scalar_to_value(&p)?);
                                    }
                                }
                            }
                            row.insert(att_name.clone(), Value::Object(obj));
                            resolved_attachments.insert(att_name);
                            i += 1;
                            continue;
                        }
                    }

                    // Traditional attachment: .fieldname {} or .fieldname [N]...
                    let (att_name_t, att_val, consumed, parsed_fields) =
                        parse_attachment_v3(lines, i, rest, depth + 2, &shared_array_schemas)?;
                    // Store authoritative field order from the header for shared schema.
                    if rows.is_empty() {
                        if let Some(pf) = parsed_fields {
                            shared_array_schemas.insert(att_name_t.clone(), pf);
                        }
                    }
                    resolved_attachments.insert(att_name_t.clone());
                    row.insert(att_name_t, att_val);
                    i += consumed;
                    continue;
                }

                // No-prefix line: must be positional inline data.
                let mut found_inline = false;
                let mut next_inline_field = String::new();
                while inline_idx < inline_att_order.len() {
                    let candidate = &inline_att_order[inline_idx];
                    if !resolved_attachments.contains(candidate) {
                        next_inline_field = candidate.clone();
                        found_inline = true;
                        break;
                    }
                    inline_idx += 1;
                }
                if !found_inline {
                    break; // no more inline fields expected
                }

                let ifs = inline_schemas
                    .get(&next_inline_field)
                    .ok_or_else(|| {
                        format!("missing inline schema for field: {}", next_inline_field)
                    })?
                    .clone();
                let inline_vals = split_respecting_quotes(a_content, '|');
                if inline_vals.len() != ifs.len() {
                    return Err(format!(
                        "inline_width_mismatch: {} expected {}, got {}",
                        next_inline_field,
                        ifs.len(),
                        inline_vals.len()
                    ));
                }
                let mut obj = Map::new();
                for (k, inf) in ifs.iter().enumerate() {
                    let p = parse_scalar(&inline_vals[k], true)?;
                    match p {
                        ScalarValue::Missing => {}
                        _ => {
                            obj.insert(inf.clone(), scalar_to_value(&p)?);
                        }
                    }
                }
                resolved_attachments.insert(next_inline_field.clone());
                row.insert(next_inline_field, Value::Object(obj));
                inline_idx += 1;
                i += 1;
            }

            // Verify all attachment fields resolved.
            for f in &all_att_fields {
                if !resolved_attachments.contains(f) {
                    return Err(format!("missing_attachment: {}", f));
                }
            }

            // Check for extra attachment lines after all fields resolved (duplicate).
            if i < lines.len() {
                let extra_line = &lines[i];
                let extra_content = if extra_line.starts_with(&format!("{}  ", ind)) {
                    &extra_line[ind.len() + 2..]
                } else if extra_line.starts_with(&ind) {
                    &extra_line[ind.len()..]
                } else {
                    ""
                };
                if extra_content.starts_with('.') {
                    let (extra_name, _) = parse_attachment_name(&extra_content[1..]);
                    if resolved_attachments.contains(&extra_name) {
                        return Err(format!("duplicate_attachment: {}", extra_name));
                    }
                }
            }
        }

        rows.push(Value::Object(row));

        if expected_count >= 0 && rows.len() as i64 >= expected_count {
            break;
        }
    }
    Ok((rows, i - start))
}

/// Parse attachment name from the rest of an attachment line (after the leading dot).
/// Returns (name, remainder_after_name).
fn parse_attachment_name(rest: &str) -> (String, &str) {
    if rest.starts_with('"') {
        let bytes = rest.as_bytes();
        let mut j = 1;
        while j < bytes.len() {
            if bytes[j] == b'\\' {
                j += 2;
                continue;
            }
            if bytes[j] == b'"' {
                if let Ok(parsed) = parse_quoted_string(&rest[..j + 1]) {
                    return (parsed, &rest[j + 1..]);
                }
                return (String::new(), rest);
            }
            j += 1;
        }
        (String::new(), rest)
    } else {
        if let Some(sp) = rest.find(' ') {
            (rest[..sp].to_string(), &rest[sp..])
        } else {
            (rest.to_string(), "")
        }
    }
}

/// v3 parse_attachment that returns parsed field names for shared schema support.
/// Returns (name, value, lines_consumed, parsed_fields).
fn parse_attachment_v3(
    lines: &[String],
    line_idx: usize,
    rest: &str,
    depth: usize,
    shared_schemas: &std::collections::HashMap<String, Vec<String>>,
) -> Result<(String, Value, usize, Option<Vec<String>>), String> {
    let (name, after_name_raw) = parse_attachment_name(rest);
    if name.is_empty() && !rest.starts_with("\"\"") {
        return Err("invalid attachment".into());
    }
    let after_name = after_name_raw.trim_start();

    // Object: {}
    if after_name.starts_with("{}") {
        let mut nested = Map::new();
        let consumed = parse_object_body(lines, line_idx + 1, depth, &mut nested)?;
        return Ok((name, Value::Object(nested), consumed + 1, None));
    }

    // Array: [N]{fields} or [N]: or [N]
    if after_name.starts_with('[') {
        let close_bracket = after_name
            .find(']')
            .ok_or_else(|| "invalid_count: missing ]".to_string())?;
        let after_close = &after_name[close_bracket + 1..];

        // [N]{fields} - has its own schema.
        if after_close.starts_with('{') {
            let end_brace = find_closing_brace(after_close);
            let mut parsed_fields: Option<Vec<String>> = None;
            if let Some(eb) = end_brace {
                if let Ok(pf) = split_field_decl(&after_close[..eb + 1]) {
                    parsed_fields = Some(pf);
                }
            }
            let (arr, consumed) = parse_array_from_header(lines, line_idx, depth, after_name)?;
            return Ok((name, arr, consumed, parsed_fields));
        }

        // [N]: values (inline primitive array): don't use shared schema.
        if after_close.starts_with(": ") || after_close == ":" {
            let (arr, consumed) = parse_array_from_header(lines, line_idx, depth, after_name)?;
            return Ok((name, arr, consumed, None));
        }

        // [N] without {fields}: check for shared schema.
        // Only use shared schema if the next line looks tabular (not @N expanded).
        if let Some(sf) = shared_schemas.get(&name) {
            let count_str = &after_name[1..close_bracket];
            let count: i64 = if count_str == "?" {
                -1
            } else {
                parse_count(count_str)? as i64
            };
            if count == 0 {
                return Ok((name, Value::Array(vec![]), 1, None));
            }
            // Peek at next line: if it starts with @ it's expanded, not tabular.
            let mut use_shared = true;
            let next_idx = line_idx + 1;
            let indent_str = "  ".repeat(depth);
            if next_idx < lines.len() {
                let next_line = &lines[next_idx];
                let next_content = if depth > 0 && next_line.starts_with(&indent_str) {
                    &next_line[indent_str.len()..]
                } else {
                    next_line.as_str()
                };
                if next_content.trim_start().starts_with('@') {
                    use_shared = false;
                }
            }
            if use_shared {
                let (tab_rows, consumed) = parse_tabular_body_with_shared(
                    lines,
                    line_idx + 1,
                    depth,
                    sf,
                    count,
                    Some(shared_schemas),
                )?;
                if count >= 0 && tab_rows.len() as i64 != count {
                    return Err(format!(
                        "count_mismatch: declared {}, got {}",
                        count,
                        tab_rows.len()
                    ));
                }
                return Ok((name, Value::Array(tab_rows), consumed + 1, None));
            }
        }

        // No shared schema: standard expanded array.
        let (arr, consumed) = parse_array_from_header(lines, line_idx, depth, after_name)?;
        return Ok((name, arr, consumed, None));
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
