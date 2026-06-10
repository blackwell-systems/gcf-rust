//! GCF v2.0 generic encoder: serializes serde_json::Value into GCF generic profile.

use serde_json::Value;
use crate::scalar::{format_scalar, format_key, format_number};

/// Encode any JSON value into GCF v2.0 generic profile.
pub fn encode_generic(data: &Value) -> String {
    let mut out = String::from("GCF profile=generic\n");
    encode_root_value(data, &mut out);
    out
}

fn encode_root_value(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("=-\n"),
        Value::Object(map) => encode_object(map, out, 0),
        Value::Array(arr) => encode_root_array(arr, out),
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

fn encode_object(map: &serde_json::Map<String, Value>, out: &mut String, depth: usize) {
    let prefix = indent(depth);
    for (key, value) in map {
        let fk = format_key(key);
        match value {
            Value::Object(sub) => {
                out.push_str(&prefix);
                out.push_str("## ");
                out.push_str(&fk);
                out.push('\n');
                encode_object(sub, out, depth + 1);
            }
            Value::Array(arr) => encode_named_array(&fk, arr, out, depth),
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

fn encode_root_array(arr: &[Value], out: &mut String) {
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
        encode_tabular("## ", arr, &fields, out, 0);
        return;
    }
    encode_expanded("## ", arr, out, 0);
}

fn encode_named_array(name: &str, arr: &[Value], out: &mut String, depth: usize) {
    let prefix = indent(depth);
    if arr.is_empty() {
        out.push_str(&format!("{}## {} [0]\n", prefix, name));
        return;
    }
    if all_primitives(arr) {
        let vals: Vec<String> = arr.iter().map(|v| format_scalar(v, ',')).collect();
        out.push_str(&format!("{}{}[{}]: {}\n", prefix, name, arr.len(), vals.join(",")));
        return;
    }
    if let Some(fields) = tabular_fields(arr) {
        encode_tabular(&format!("{}## {} ", prefix, name), arr, &fields, out, depth);
        return;
    }
    encode_expanded(&format!("{}## {} ", prefix, name), arr, out, depth);
}

fn tabular_fields(arr: &[Value]) -> Option<Vec<String>> {
    if arr.is_empty() { return None; }
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
    if field_order.is_empty() { return None; }
    Some(field_order)
}

fn encode_tabular(header_prefix: &str, arr: &[Value], fields: &[String], out: &mut String, depth: usize) {
    let prefix = indent(depth);
    let fmt_fields: Vec<String> = fields.iter().map(|f| format_key(f)).collect();
    out.push_str(&format!("{}[{}]{{{}}}\n", header_prefix, arr.len(), fmt_fields.join(",")));

    for (i, item) in arr.iter().enumerate() {
        let map = match item.as_object() {
            Some(m) => m,
            None => continue,
        };

        let mut cells = Vec::new();
        let mut attachments: Vec<(&str, &Value)> = Vec::new();
        let mut row_has_attachment = false;

        for f in fields {
            match map.get(f) {
                None => cells.push("~".to_string()),
                Some(Value::Null) => cells.push("-".to_string()),
                Some(v) if v.is_object() || v.is_array() => {
                    cells.push("^".to_string());
                    attachments.push((f, v));
                    row_has_attachment = true;
                }
                Some(v) => cells.push(format_scalar(v, '|')),
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

        for (att_name, att_val) in &attachments {
            let att_prefix = format!("{}  ", prefix);
            let fk = format_key(att_name);
            match att_val {
                Value::Object(sub) => {
                    out.push_str(&format!("{}.{} {{}}\n", att_prefix, fk));
                    encode_object(sub, out, depth + 2);
                }
                Value::Array(sub) => encode_attachment_array(&att_prefix, &fk, sub, out, depth + 2),
                _ => {}
            }
        }
    }
}

fn encode_attachment_array(att_prefix: &str, fk: &str, arr: &[Value], out: &mut String, depth: usize) {
    if arr.is_empty() {
        out.push_str(&format!("{}.{} [0]\n", att_prefix, fk));
    } else if all_primitives(arr) {
        let vals: Vec<String> = arr.iter().map(|v| format_scalar(v, ',')).collect();
        out.push_str(&format!("{}.{} [{}]: {}\n", att_prefix, fk, arr.len(), vals.join(",")));
    } else if let Some(fields) = tabular_fields(arr) {
        encode_tabular(&format!("{}.{} ", att_prefix, fk), arr, &fields, out, depth);
    } else {
        encode_expanded(&format!("{}.{} ", att_prefix, fk), arr, out, depth);
    }
}

fn encode_expanded(header_prefix: &str, arr: &[Value], out: &mut String, depth: usize) {
    let prefix = indent(depth);
    out.push_str(&format!("{}[{}]\n", header_prefix, arr.len()));
    for (i, item) in arr.iter().enumerate() {
        match item {
            Value::Object(map) => {
                out.push_str(&format!("{}@{} {{}}\n", prefix, i));
                encode_object(map, out, depth + 1);
            }
            Value::Array(sub) => encode_expanded_array_item(&prefix, i, sub, out, depth),
            _ => {
                out.push_str(&format!("{}@{} ={}\n", prefix, i, format_scalar(item, '\0')));
            }
        }
    }
}

fn encode_expanded_array_item(prefix: &str, idx: usize, arr: &[Value], out: &mut String, depth: usize) {
    if arr.is_empty() {
        out.push_str(&format!("{}@{} [0]\n", prefix, idx));
    } else if all_primitives(arr) {
        let vals: Vec<String> = arr.iter().map(|v| format_scalar(v, ',')).collect();
        out.push_str(&format!("{}@{} [{}]: {}\n", prefix, idx, arr.len(), vals.join(",")));
    } else if let Some(fields) = tabular_fields(arr) {
        encode_tabular(&format!("{}@{} ", prefix, idx), arr, &fields, out, depth + 1);
    } else {
        encode_expanded(&format!("{}@{} ", prefix, idx), arr, out, depth + 1);
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
}
