//! GCF generic encoder: serializes serde_json::Value into GCF generic profile.

use crate::scalar::{format_key, format_number, format_scalar};
use serde_json::Value;

/// Options for controlling generic encoding behavior.
#[derive(Debug, Clone, Default)]
pub struct GenericOptions {
    /// When true, disables promotion of fixed-shape nested objects to path
    /// columns (e.g. "customer>name"). Nested objects use attachment syntax
    /// instead. Set when targeting open-weight models that show lower
    /// comprehension on flattened encoding.
    pub no_flatten: bool,
}

/// Encode any JSON value into GCF generic profile.
pub fn encode_generic(data: &Value) -> String {
    encode_generic_with_options(data, &GenericOptions::default())
}

/// Encode any JSON value into GCF generic profile with the given options.
pub fn encode_generic_with_options(data: &Value, opts: &GenericOptions) -> String {
    let mut out = String::from("GCF profile=generic\n");
    encode_root_value(data, &mut out, opts);
    out
}

fn encode_root_value(v: &Value, out: &mut String, opts: &GenericOptions) {
    match v {
        Value::Null => out.push_str("=-\n"),
        Value::Object(map) => encode_object(map, out, 0, opts),
        Value::Array(arr) => encode_root_array(arr, out, opts),
        Value::Bool(b) => {
            out.push('=');
            out.push_str(if *b { "true" } else { "false" });
            out.push('\n');
        }
        Value::Number(n) => {
            out.push('=');
            out.push_str(&format_number(n));
            out.push('\n');
        }
        Value::String(_) => {
            out.push('=');
            out.push_str(&format_scalar(v, '\0'));
            out.push('\n');
        }
    }
}

fn encode_object(map: &serde_json::Map<String, Value>, out: &mut String, depth: usize, opts: &GenericOptions) {
    let prefix = indent(depth);
    for (key, value) in map {
        let fk = format_key(key);
        match value {
            Value::Object(sub) => {
                out.push_str(&prefix);
                out.push_str("## ");
                out.push_str(&fk);
                out.push('\n');
                encode_object(sub, out, depth + 1, opts);
            }
            Value::Array(arr) => encode_named_array(&fk, arr, out, depth, opts),
            _ => {
                out.push_str(&prefix);
                out.push_str(&fk);
                out.push('=');
                out.push_str(&format_scalar(value, '\0'));
                out.push('\n');
            }
        }
    }
}

fn encode_root_array(arr: &[Value], out: &mut String, opts: &GenericOptions) {
    if arr.is_empty() {
        out.push_str("## [0]\n");
        return;
    }
    if all_primitives(arr) {
        let vals: Vec<String> = arr.iter().map(|v| format_scalar(v, ',')).collect();
        out.push_str(&format!("## [{}]: {}\n", arr.len(), vals.join(",")));
        return;
    }
    if let Some(fields) = tabular_fields(arr) {
        encode_tabular("## ", arr, &fields, out, 0, opts);
        return;
    }
    encode_expanded("## ", arr, out, 0, opts);
}

fn encode_named_array(name: &str, arr: &[Value], out: &mut String, depth: usize, opts: &GenericOptions) {
    let prefix = indent(depth);
    if arr.is_empty() {
        out.push_str(&format!("{}## {} [0]\n", prefix, name));
        return;
    }
    if all_primitives(arr) {
        let vals: Vec<String> = arr.iter().map(|v| format_scalar(v, ',')).collect();
        out.push_str(&format!(
            "{}{}[{}]: {}\n",
            prefix,
            name,
            arr.len(),
            vals.join(",")
        ));
        return;
    }
    if let Some(fields) = tabular_fields(arr) {
        encode_tabular(&format!("{}## {} ", prefix, name), arr, &fields, out, depth, opts);
        return;
    }
    encode_expanded(&format!("{}## {} ", prefix, name), arr, out, depth, opts);
}

fn tabular_fields(arr: &[Value]) -> Option<Vec<String>> {
    if arr.is_empty() {
        return None;
    }
    let mut field_order = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for item in arr {
        let map = item.as_object()?;
        for k in map.keys() {
            if seen.insert(k.clone()) {
                field_order.push(k.clone());
            }
        }
    }
    if field_order.is_empty() {
        return None;
    }
    Some(field_order)
}

fn inline_schema_fields(arr: &[Value], field_name: &str) -> Option<Vec<String>> {
    if arr.is_empty() {
        return None;
    }
    let first = arr[0].as_object()?;
    let first_val = first.get(field_name)?;
    let first_obj = first_val.as_object()?;
    if first_obj.is_empty() {
        return None;
    }

    let mut canonical_keys: Option<Vec<String>> = None;
    for item in arr {
        let map = item.as_object()?;
        let v = match map.get(field_name) {
            Some(Value::Null) | None => continue,
            Some(v) => v,
        };
        let obj = v.as_object()?;
        for val in obj.values() {
            if val.is_object() || val.is_array() {
                return None;
            }
        }
        let keys: Vec<String> = obj.keys().cloned().collect();
        match &canonical_keys {
            None => canonical_keys = Some(keys),
            Some(ck) => {
                if keys != *ck {
                    return None;
                }
            }
        }
    }
    let ck = canonical_keys?;
    if ck.len() < 3 {
        return None;
    }
    Some(ck)
}

fn shared_array_schema(arr: &[Value], field_name: &str) -> Option<Vec<String>> {
    if arr.is_empty() {
        return None;
    }
    let first = arr[0].as_object()?;
    let first_val = first.get(field_name)?;
    first_val.as_array()?;

    let mut canonical_fields: Option<Vec<String>> = None;
    for item in arr {
        let map = item.as_object()?;
        let v = match map.get(field_name) {
            Some(Value::Null) | None => continue,
            Some(v) => v,
        };
        let sub_arr = v.as_array()?;
        let fields = tabular_fields(sub_arr)?;
        // All values must be scalars.
        for sub_item in sub_arr {
            let sub_map = sub_item.as_object()?;
            for val in sub_map.values() {
                if val.is_object() || val.is_array() {
                    return None;
                }
            }
        }
        match &canonical_fields {
            None => canonical_fields = Some(fields),
            Some(cf) => {
                if fields != *cf {
                    return None;
                }
            }
        }
    }
    canonical_fields
}

/// A flattened leaf column produced by analyzing a nested object.
struct FlatLeaf {
    path: String,      // ">" separated path (e.g. "customer>name")
    keys: Vec<String>, // key chain to traverse from row object
}

/// Analyze whether a field across all rows contains a fixed-shape nested object
/// that can be flattened. Returns leaf descriptors if flattenable, None otherwise.
fn analyze_flattenable(
    arr: &[Value],
    field_name: &str,
    parent_path: &str,
) -> Option<Vec<FlatLeaf>> {
    // Field names containing ">" cannot be flattened (would create ambiguous paths).
    if field_name.contains('>') {
        return None;
    }
    let mut canonical_keys: Option<Vec<String>> = None;
    let mut canonical_shape: std::collections::HashMap<String, &'static str> =
        std::collections::HashMap::new();

    for item in arr {
        let map = item.as_object()?;
        let v = match map.get(field_name) {
            None | Some(Value::Null) => continue,
            Some(v) => v,
        };
        let obj = v.as_object()?; // Not an object? Bail.
        if v.is_array() {
            return None;
        }

        let keys: Vec<String> = obj.keys().cloned().collect();
        if let Some(ref ck) = canonical_keys {
            if keys != *ck {
                return None;
            }
            // Verify shape consistency.
            for k in &keys {
                let val = &obj[k];
                let expected = canonical_shape.get(k.as_str())?;
                match *expected {
                    "scalar" => {
                        if val.is_object() || val.is_array() {
                            return None;
                        }
                    }
                    "nested" => {
                        if val.is_array() {
                            return None;
                        }
                        if !val.is_null() && !val.is_object() {
                            return None;
                        }
                    }
                    _ => {}
                }
            }
        } else {
            for k in &keys {
                if k.contains('>') {
                    return None;
                }
                let val = &obj[k];
                if val.is_array() {
                    return None;
                }
                let kind = if val.is_object() { "nested" } else { "scalar" };
                canonical_shape.insert(k.clone(), kind);
            }
            canonical_keys = Some(keys);
        }
    }

    let ck = canonical_keys?;
    let current_path = if parent_path.is_empty() {
        field_name.to_string()
    } else {
        format!("{}>{}", parent_path, field_name)
    };

    let parent_keys: Vec<String> = if parent_path.is_empty() {
        vec![field_name.to_string()]
    } else {
        let mut pk: Vec<String> = parent_path.split('>').map(|s| s.to_string()).collect();
        pk.push(field_name.to_string());
        pk
    };

    let mut leaves = Vec::new();
    for k in &ck {
        let kind = canonical_shape.get(k.as_str()).copied().unwrap_or("scalar");
        if kind == "scalar" {
            let mut keys = parent_keys.clone();
            keys.push(k.clone());
            leaves.push(FlatLeaf {
                path: format!("{}>{}", current_path, k),
                keys,
            });
        } else {
            // Nested: extract sub-objects and recurse.
            let sub_arr: Vec<Value> = arr
                .iter()
                .map(|item| {
                    item.as_object()
                        .and_then(|m| m.get(field_name))
                        .and_then(|v| if v.is_null() { None } else { Some(v.clone()) })
                        .unwrap_or(Value::Object(serde_json::Map::new()))
                })
                .collect();
            let sub_leaves = analyze_flattenable(&sub_arr, k, &current_path)?;
            if sub_leaves.is_empty() {
                return None; // Empty nested object cannot be represented by flattening.
            }
            leaves.extend(sub_leaves);
        }
    }

    // Guard: reject if any row has non-null object with all-null leaves (ambiguous with null parent).
    if !leaves.is_empty() {
        for item in arr {
            let map = match item.as_object() {
                Some(m) => m,
                None => continue,
            };
            let v = match map.get(field_name) {
                None | Some(Value::Null) => continue,
                Some(v) => v,
            };
            if !v.is_object() {
                continue;
            }
            let all_null = leaves.iter().all(|leaf| {
                resolve_key_chain(item, &leaf.keys)
                    .map(|val| val.is_null())
                    .unwrap_or(false)
            });
            if all_null {
                return None;
            }
        }
    }

    Some(leaves)
}

/// Traverse an object following a key chain, returning the leaf value.
fn resolve_key_chain(item: &Value, keys: &[String]) -> Option<Value> {
    if keys.is_empty() {
        return None;
    }
    let mut current = item.as_object()?.get(&keys[0])?.clone();
    for k in &keys[1..] {
        current = current.as_object()?.get(k)?.clone();
    }
    Some(current)
}

/// Check if the top-level key exists in the item.
fn key_exists(item: &Value, key: &str) -> bool {
    item.as_object().is_some_and(|m| m.contains_key(key))
}

/// A column in the expanded (flattened) field list.
struct FlatColumn {
    header_name: String,
    col_type: FlatColType,
    field: String,
    keys: Vec<String>,
}

enum FlatColType {
    Flat,
    Original,
}

fn encode_tabular(
    header_prefix: &str,
    arr: &[Value],
    fields: &[String],
    out: &mut String,
    depth: usize,
    opts: &GenericOptions,
) {
    let prefix = indent(depth);

    // Phase 0: Analyze fields for flattening.
    let mut flatten_map: std::collections::HashMap<String, Vec<FlatLeaf>> =
        std::collections::HashMap::new();
    if !opts.no_flatten {
        for f in fields {
            if let Some(leaves) = analyze_flattenable(arr, f, "") {
                if !leaves.is_empty() {
                    flatten_map.insert(f.clone(), leaves);
                }
            }
        }
    }

    // Fields whose names contain ">" must not appear as tabular columns
    // because the decoder would interpret them as flattened path columns.
    // Track them for per-row attachment emission (spec rule 7.4.6.1.4).
    let gt_fields: std::collections::HashSet<&String> = fields
        .iter()
        .filter(|f| !flatten_map.contains_key(*f) && f.contains('>'))
        .collect();

    // Build expanded column list.
    let mut columns: Vec<FlatColumn> = Vec::new();
    for f in fields {
        if gt_fields.contains(f) {
            continue;
        }
        if let Some(leaves) = flatten_map.get(f) {
            for leaf in leaves {
                columns.push(FlatColumn {
                    header_name: format_key(&leaf.path),
                    col_type: FlatColType::Flat,
                    field: f.clone(),
                    keys: leaf.keys.clone(),
                });
            }
        } else {
            columns.push(FlatColumn {
                header_name: format_key(f),
                col_type: FlatColType::Original,
                field: f.clone(),
                keys: Vec::new(),
            });
        }
    }

    // If all fields were excluded (all contain ">"), fall back to expanded.
    if columns.is_empty() {
        encode_expanded(header_prefix, arr, out, depth, opts);
        return;
    }

    // Pre-compute inline schemas and shared array schemas (skip flattened fields).
    let mut inline_schemas: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    let mut shared_arr_schemas: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for f in fields {
        if flatten_map.contains_key(f) {
            continue;
        }
        if let Some(ifs) = inline_schema_fields(arr, f) {
            inline_schemas.insert(f.clone(), ifs);
        }
        if let Some(sas) = shared_array_schema(arr, f) {
            shared_arr_schemas.insert(f.clone(), sas);
        }
    }

    let header_fields: Vec<&str> = columns.iter().map(|c| c.header_name.as_str()).collect();
    out.push_str(&format!(
        "{}[{}]{{{}}}\n",
        header_prefix,
        arr.len(),
        header_fields.join(",")
    ));

    for (i, item) in arr.iter().enumerate() {
        let map = match item.as_object() {
            Some(m) => m,
            None => continue,
        };

        let mut cells = Vec::new();
        struct Att {
            name: String,
            value: Value,
            inline: bool,
            inline_fields: Option<Vec<String>>,
        }
        let mut attachments: Vec<Att> = Vec::new();
        let mut row_has_attachment = false;

        for col in &columns {
            match col.col_type {
                FlatColType::Flat => {
                    // Resolve value via key chain.
                    if !key_exists(item, &col.keys[0]) {
                        cells.push("~".to_string());
                    } else {
                        // Check if the top-level field itself is null.
                        let top_val = item.as_object().and_then(|m| m.get(&col.keys[0]));
                        if top_val == Some(&Value::Null) {
                            cells.push("-".to_string());
                        } else {
                            match resolve_key_chain(item, &col.keys) {
                                None => cells.push("~".to_string()),
                                Some(Value::Null) => cells.push("-".to_string()),
                                Some(v) => cells.push(format_scalar(&v, '|')),
                            }
                        }
                    }
                    continue;
                }
                FlatColType::Original => {}
            }

            let f = &col.field;
            match map.get(f.as_str()) {
                None => cells.push("~".to_string()),
                Some(Value::Null) => cells.push("-".to_string()),
                Some(v) if v.is_object() || v.is_array() => {
                    if let Some(ifs) = inline_schemas.get(f) {
                        if v.is_object() {
                            if i == 0 {
                                let fmt_if: Vec<String> =
                                    ifs.iter().map(|k| format_key(k)).collect();
                                cells.push(format!("^{{{}}}", fmt_if.join(",")));
                            } else {
                                cells.push("^".to_string());
                            }
                            attachments.push(Att {
                                name: f.clone(),
                                value: v.clone(),
                                inline: true,
                                inline_fields: Some(ifs.clone()),
                            });
                        } else {
                            cells.push("^".to_string());
                            attachments.push(Att {
                                name: f.clone(),
                                value: v.clone(),
                                inline: false,
                                inline_fields: None,
                            });
                        }
                    } else {
                        cells.push("^".to_string());
                        attachments.push(Att {
                            name: f.clone(),
                            value: v.clone(),
                            inline: false,
                            inline_fields: None,
                        });
                    }
                    row_has_attachment = true;
                }
                Some(v) => cells.push(format_scalar(v, '|')),
            }
        }

        // Emit fields with ">" in their names as per-row attachments.
        if let Some(obj) = item.as_object() {
            for f in fields {
                if !gt_fields.contains(f) {
                    continue;
                }
                if let Some(v) = obj.get(f) {
                    row_has_attachment = true;
                    attachments.push(Att {
                        name: f.clone(),
                        value: v.clone(),
                        inline: false,
                        inline_fields: None,
                    });
                }
            }
        }

        let row = cells.join("|");
        if row_has_attachment {
            out.push_str(&format!("{}@{} {}\n", prefix, i, row));
        } else {
            out.push_str(&prefix);
            out.push_str(&row);
            out.push('\n');
        }

        for att in &attachments {
            let fk = format_key(&att.name);
            if att.inline {
                if let (Some(ifs), Some(obj)) = (&att.inline_fields, att.value.as_object()) {
                    let vals: Vec<String> = ifs
                        .iter()
                        .map(|inf| match obj.get(inf) {
                            None => "~".to_string(),
                            Some(v) => format_scalar(v, '|'),
                        })
                        .collect();
                    out.push_str(&format!("{}{}\n", prefix, vals.join("|")));
                }
            } else {
                match &att.value {
                    Value::Object(sub) => {
                        out.push_str(&format!("{}.{} {{}}\n", prefix, fk));
                        encode_object(sub, out, depth + 2, opts);
                    }
                    Value::Array(sub) => {
                        if let Some(sas) = shared_arr_schemas.get(&att.name) {
                            if i > 0 {
                                encode_attachment_array_shared(
                                    &prefix,
                                    &fk,
                                    sub,
                                    out,
                                    depth + 2,
                                    sas,
                                    opts,
                                );
                            } else {
                                encode_attachment_array(&prefix, &fk, sub, out, depth + 2, opts);
                            }
                        } else {
                            encode_attachment_array(&prefix, &fk, sub, out, depth + 2, opts);
                        }
                    }
                    _ => {
                        // Scalar attachment (e.g. field names containing ">").
                        out.push_str(&format!(
                            "{}.{} ={}\n",
                            prefix,
                            fk,
                            format_scalar(&att.value, '\0')
                        ));
                    }
                }
            }
        }
    }
}

fn encode_attachment_array_shared(
    att_prefix: &str,
    fk: &str,
    arr: &[Value],
    out: &mut String,
    depth: usize,
    shared_fields: &[String],
    opts: &GenericOptions,
) {
    if arr.is_empty() {
        out.push_str(&format!("{}.{} [0]\n", att_prefix, fk));
        return;
    }
    if all_primitives(arr) {
        let vals: Vec<String> = arr.iter().map(|v| format_scalar(v, ',')).collect();
        out.push_str(&format!(
            "{}.{} [{}]: {}\n",
            att_prefix,
            fk,
            arr.len(),
            vals.join(",")
        ));
        return;
    }
    if let Some(fields) = tabular_fields(arr) {
        if fields == shared_fields {
            let p = indent(depth);
            out.push_str(&format!("{}.{} [{}]\n", att_prefix, fk, arr.len()));
            for item in arr {
                if let Some(obj) = item.as_object() {
                    let cells: Vec<String> = shared_fields
                        .iter()
                        .map(|f| match obj.get(f) {
                            None => "~".to_string(),
                            Some(Value::Null) => "-".to_string(),
                            Some(v) => format_scalar(v, '|'),
                        })
                        .collect();
                    out.push_str(&format!("{}{}\n", p, cells.join("|")));
                }
            }
            return;
        }
    }
    encode_attachment_array(att_prefix, fk, arr, out, depth, opts);
}

fn encode_attachment_array(
    att_prefix: &str,
    fk: &str,
    arr: &[Value],
    out: &mut String,
    depth: usize,
    opts: &GenericOptions,
) {
    if arr.is_empty() {
        out.push_str(&format!("{}.{} [0]\n", att_prefix, fk));
    } else if all_primitives(arr) {
        let vals: Vec<String> = arr.iter().map(|v| format_scalar(v, ',')).collect();
        out.push_str(&format!(
            "{}.{} [{}]: {}\n",
            att_prefix,
            fk,
            arr.len(),
            vals.join(",")
        ));
    } else if let Some(fields) = tabular_fields(arr) {
        encode_tabular(&format!("{}.{} ", att_prefix, fk), arr, &fields, out, depth, opts);
    } else {
        encode_expanded(&format!("{}.{} ", att_prefix, fk), arr, out, depth, opts);
    }
}

fn encode_expanded(header_prefix: &str, arr: &[Value], out: &mut String, depth: usize, opts: &GenericOptions) {
    let prefix = indent(depth);
    out.push_str(&format!("{}[{}]\n", header_prefix, arr.len()));
    for (i, item) in arr.iter().enumerate() {
        match item {
            Value::Object(map) => {
                out.push_str(&format!("{}@{} {{}}\n", prefix, i));
                encode_object(map, out, depth + 1, opts);
            }
            Value::Array(sub) => encode_expanded_array_item(&prefix, i, sub, out, depth, opts),
            _ => {
                out.push_str(&format!(
                    "{}@{} ={}\n",
                    prefix,
                    i,
                    format_scalar(item, '\0')
                ));
            }
        }
    }
}

fn encode_expanded_array_item(
    prefix: &str,
    idx: usize,
    arr: &[Value],
    out: &mut String,
    depth: usize,
    opts: &GenericOptions,
) {
    if arr.is_empty() {
        out.push_str(&format!("{}@{} [0]\n", prefix, idx));
    } else if all_primitives(arr) {
        let vals: Vec<String> = arr.iter().map(|v| format_scalar(v, ',')).collect();
        out.push_str(&format!(
            "{}@{} [{}]: {}\n",
            prefix,
            idx,
            arr.len(),
            vals.join(",")
        ));
    } else if let Some(fields) = tabular_fields(arr) {
        encode_tabular(
            &format!("{}@{} ", prefix, idx),
            arr,
            &fields,
            out,
            depth + 1,
            opts,
        );
    } else {
        encode_expanded(&format!("{}@{} ", prefix, idx), arr, out, depth + 1, opts);
    }
}

fn all_primitives(arr: &[Value]) -> bool {
    arr.iter().all(|v| !v.is_object() && !v.is_array())
}

fn indent(depth: usize) -> String {
    "  ".repeat(depth)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_generic_header() {
        let data = json!({"name": "Alice"});
        let output = encode_generic(&data);
        assert!(output.starts_with("GCF profile=generic\n"));
    }

    #[test]
    fn test_generic_root_scalar() {
        assert_eq!(encode_generic(&json!(42)), "GCF profile=generic\n=42\n");
        assert_eq!(encode_generic(&json!(true)), "GCF profile=generic\n=true\n");
        assert_eq!(encode_generic(&json!(null)), "GCF profile=generic\n=-\n");
    }

    #[test]
    fn test_generic_quoting() {
        let data = json!({"val": "true"});
        let output = encode_generic(&data);
        assert!(output.contains("val=\"true\""));
    }

    #[test]
    fn test_no_flatten_option() {
        let data = json!({
            "orders": [
                {"id": "ORD-1", "customer": {"name": "Alice", "email": "alice@co.com"}, "total": 99.99},
                {"id": "ORD-2", "customer": {"name": "Bob", "email": "bob@co.com"}, "total": 49.99}
            ]
        });

        // Default (flatten on): should have path columns.
        let with_flatten = encode_generic(&data);
        assert!(with_flatten.contains("customer>"), "expected path columns with default, got:\n{}", with_flatten);

        // Flatten off: should have attachment syntax, no path columns.
        let no_flatten = encode_generic_with_options(&data, &GenericOptions { no_flatten: true });
        assert!(!no_flatten.contains("customer>"), "expected no path columns with no_flatten, got:\n{}", no_flatten);
        assert!(no_flatten.contains(".customer"), "expected attachment syntax with no_flatten, got:\n{}", no_flatten);

        // Both must round-trip (compare as Values to ignore key order).
        let decoded_on = crate::decode_generic(&with_flatten).expect("decode flatten-on failed");
        let decoded_off = crate::decode_generic(&no_flatten).expect("decode flatten-off failed");
        assert_eq!(data, decoded_on, "flatten-on round-trip mismatch");
        assert_eq!(data, decoded_off, "flatten-off round-trip mismatch");
    }

    #[test]
    fn test_gt_field_edge_cases() {
        let cases: Vec<(&str, Value)> = vec![
            ("literal > key", json!([{">": 1}, {">": 2}])),
            ("> at start", json!([{">foo": "a", "id": 1}, {">foo": "b", "id": 2}])),
            ("> at end", json!([{"foo>": "a", "id": 1}, {"foo>": "b", "id": 2}])),
            ("double >>", json!([{"a>>b": "x"}, {"a>>b": "y"}])),
            ("multiple > in key", json!([{"a>b>c": "x"}, {"a>b>c": "y"}])),
            ("> field with null", json!([{"a>b": null, "id": 1}, {"a>b": "hello", "id": 2}])),
            ("> field with object", json!([
                {"a>b": {"x": 1}, "id": 1},
                {"a>b": {"x": 2}, "id": 2},
            ])),
            ("> field with array", json!([
                {"a>b": [1, 2], "id": 1},
                {"a>b": [3], "id": 2},
            ])),
            ("all fields have >", json!([{">": 1, "a>b": 2}, {">": 3, "a>b": 4}])),
            ("mix of > literal and flattened", json!([
                {"id": 1, "x>y": "lit", "nested": {"a": "v1", "b": "v2"}},
                {"id": 2, "x>y": "lit2", "nested": {"a": "v3", "b": "v4"}},
            ])),
            ("> field absent in some rows", json!([
                {"id": 1, "a>b": "present"},
                {"id": 2},
            ])),
            ("key looks like flattened path", json!([
                {"id": 1, "customer>name": "Alice"},
                {"id": 2, "customer>name": "Bob"},
            ])),
        ];

        for (name, data) in &cases {
            for no_flatten in [false, true] {
                let opts = GenericOptions { no_flatten };
                let encoded = encode_generic_with_options(data, &opts);
                let decoded = crate::decode_generic(&encoded).unwrap_or_else(|e| {
                    panic!("{} (no_flatten={}): decode failed: {}\n  gcf: {:?}", name, no_flatten, e, encoded);
                });
                assert_eq!(
                    data, &decoded,
                    "{} (no_flatten={}): round-trip mismatch\n  gcf: {:?}",
                    name, no_flatten, encoded
                );
            }
        }
    }
}
