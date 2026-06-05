use serde_json::Value;

/// Encode any JSON value into GCF tabular format.
/// Works on objects, arrays, and primitives via serde_json::Value.
pub fn encode_generic(data: &Value) -> String {
    match data {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => format_number(n),
        Value::String(s) => s.clone(),
        Value::Array(arr) => {
            if arr.is_empty() {
                return String::new();
            }
            let mut lines: Vec<String> = Vec::new();
            encode_array(arr, "root", &mut lines, 0);
            let mut out = lines.join("\n");
            out.push('\n');
            out
        }
        Value::Object(map) => {
            let mut lines: Vec<String> = Vec::new();
            encode_object_entries(map, &mut lines, 0);
            let mut out = lines.join("\n");
            out.push('\n');
            out
        }
    }
}

fn format_number(n: &serde_json::Number) -> String {
    if let Some(i) = n.as_i64() {
        return i.to_string();
    }
    if let Some(u) = n.as_u64() {
        return u.to_string();
    }
    if let Some(f) = n.as_f64() {
        // Use %g-style formatting: no trailing zeros.
        let s = format!("{}", f);
        return s;
    }
    n.to_string()
}

fn format_value(v: &Value) -> String {
    match v {
        Value::Null => "-".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => format_number(n),
        Value::String(s) => {
            if s.is_empty() {
                return "\"\"".to_string();
            }
            if s.contains('|') || s.contains('\n') {
                let escaped = s.replace('"', "\\\"");
                return format!("\"{}\"", escaped);
            }
            s.clone()
        }
        _ => "-".to_string(),
    }
}

fn indent(depth: usize) -> String {
    "  ".repeat(depth)
}

fn is_object(v: &Value) -> bool {
    matches!(v, Value::Object(_))
}

fn is_array(v: &Value) -> bool {
    matches!(v, Value::Array(_))
}

fn is_uniform_object_array(arr: &[Value]) -> bool {
    if arr.is_empty() {
        return false;
    }
    let first = match &arr[0] {
        Value::Object(m) => m,
        _ => return false,
    };
    if first.is_empty() {
        return false;
    }
    let first_keys: std::collections::HashSet<&String> = first.keys().collect();

    let check_count = arr.len().min(5);
    for item in arr.iter().take(check_count).skip(1) {
        let obj = match item {
            Value::Object(m) => m,
            _ => return false,
        };
        let item_keys: std::collections::HashSet<&String> = obj.keys().collect();
        let overlap = first_keys.intersection(&item_keys).count();
        if (overlap as f64) < (first_keys.len() as f64) * 0.7 {
            return false;
        }
    }
    true
}

fn encode_array(arr: &[Value], name: &str, lines: &mut Vec<String>, depth: usize) {
    let prefix = indent(depth);

    if arr.is_empty() {
        lines.push(format!("{}## {} [0]", prefix, name));
        return;
    }

    if is_uniform_object_array(arr) {
        encode_tabular(arr, name, lines, depth);
        return;
    }

    // Primitive array: inline as comma-separated values.
    let all_primitive = arr.iter().all(|item| !is_object(item) && !is_array(item));
    if all_primitive {
        let vals: Vec<String> = arr.iter().map(|item| format_value(item)).collect();
        lines.push(format!("{}{}[{}]: {}", prefix, name, arr.len(), vals.join(",")));
        return;
    }

    // Non-uniform with objects: per-item encoding.
    lines.push(format!("{}## {} [{}]", prefix, name, arr.len()));
    for (i, item) in arr.iter().enumerate() {
        if is_object(item) {
            lines.push(format!("{}@{}", prefix, i));
            if let Value::Object(map) = item {
                encode_object_entries(map, lines, depth + 1);
            }
        } else if is_array(item) {
            if let Value::Array(sub) = item {
                encode_array(sub, &i.to_string(), lines, depth + 1);
            }
        } else {
            lines.push(format!("{}@{} {}", prefix, i, format_value(item)));
        }
    }
}

fn encode_tabular(arr: &[Value], name: &str, lines: &mut Vec<String>, depth: usize) {
    let prefix = indent(depth);
    let first = match &arr[0] {
        Value::Object(m) => m,
        _ => return,
    };

    // Collect all keys from all items (preserving insertion order from first, then extras).
    let mut all_keys: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for item in arr {
        if let Value::Object(m) = item {
            for k in m.keys() {
                if seen.insert(k.clone()) {
                    all_keys.push(k.clone());
                }
            }
        }
    }

    // Separate primitive from nested fields (sample from first element).
    let mut primitive_fields: Vec<String> = Vec::new();
    let mut nested_fields: Vec<String> = Vec::new();

    for key in &all_keys {
        let sample = first.get(key);
        match sample {
            Some(v) if is_object(v) || is_array(v) => nested_fields.push(key.clone()),
            _ => primitive_fields.push(key.clone()),
        }
    }

    // Header.
    let field_list = primitive_fields.join(",");
    lines.push(format!(
        "{}## {} [{}]{{{}}}",
        prefix,
        name,
        arr.len(),
        field_list
    ));

    let has_nested = !nested_fields.is_empty();

    for (i, item) in arr.iter().enumerate() {
        let obj = match item {
            Value::Object(m) => m,
            _ => continue,
        };

        let vals: Vec<String> = primitive_fields
            .iter()
            .map(|f| match obj.get(f) {
                Some(v) if v.is_null() => "-".to_string(),
                Some(v) => format_value(v),
                None => "-".to_string(),
            })
            .collect();

        let row_str = vals.join("|");

        if has_nested {
            lines.push(format!("{}@{} {}", prefix, i, row_str));
            // Inline nested fields after the row.
            for nf in &nested_fields {
                if let Some(nv) = obj.get(nf) {
                    if nv.is_null() {
                        continue;
                    }
                    if let Value::Array(sub) = nv {
                        encode_array(sub, nf, lines, depth + 1);
                    } else if let Value::Object(sub) = nv {
                        lines.push(format!("{}.{}", indent(depth + 1), nf));
                        encode_object_entries(sub, lines, depth + 2);
                    }
                }
            }
        } else {
            lines.push(format!("{}{}", prefix, row_str));
        }
    }
}

fn encode_object_entries(
    map: &serde_json::Map<String, Value>,
    lines: &mut Vec<String>,
    depth: usize,
) {
    let prefix = indent(depth);

    for (key, value) in map {
        if value.is_null() {
            continue;
        }
        if let Value::Array(arr) = value {
            encode_array(arr, key, lines, depth);
        } else if let Value::Object(sub) = value {
            lines.push(format!("{}## {}", indent(depth + 1), key));
            encode_object_entries(sub, lines, depth + 2);
        } else {
            lines.push(format!("{}{}={}", prefix, key, format_value(value)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_generic_tabular() {
        let data = json!({
            "employees": [
                {"id": 1, "name": "Alice", "department": "Engineering", "salary": 95000},
                {"id": 2, "name": "Bob", "department": "Sales", "salary": 72000},
            ]
        });
        let output = encode_generic(&data);
        // serde_json Map iterates keys alphabetically: department, id, name, salary
        assert!(output.contains("## employees [2]{department,id,name,salary}"));
        assert!(output.contains("Engineering|1|Alice|95000"));
        assert!(output.contains("Sales|2|Bob|72000"));
    }

    #[test]
    fn test_generic_primitive() {
        let data = json!(42);
        let output = encode_generic(&data);
        assert_eq!(output, "42");
    }

    #[test]
    fn test_generic_null() {
        let data = json!(null);
        let output = encode_generic(&data);
        assert_eq!(output, "");
    }

    #[test]
    fn test_generic_nested_object() {
        let data = json!({
            "name": "test",
            "config": {
                "debug": true,
                "level": 5
            }
        });
        let output = encode_generic(&data);
        assert!(output.contains("name=test"));
        assert!(output.contains("## config"));
        assert!(output.contains("debug=true"));
        assert!(output.contains("level=5"));
    }

    #[test]
    fn test_generic_null_in_table() {
        let data = json!({
            "items": [
                {"a": 1, "b": null},
                {"a": 2, "b": 3},
            ]
        });
        let output = encode_generic(&data);
        // null should render as "-"
        assert!(output.contains("1|-"));
        assert!(output.contains("2|3"));
    }

    #[test]
    fn test_generic_boolean() {
        let data = json!({"flag": true, "other": false});
        let output = encode_generic(&data);
        assert!(output.contains("flag=true"));
        assert!(output.contains("other=false"));
    }

    #[test]
    fn test_generic_string_with_pipe() {
        let data = json!({"val": "a|b"});
        let output = encode_generic(&data);
        assert!(output.contains("val=\"a|b\""));
    }

    #[test]
    fn test_generic_empty_string() {
        let data = json!({"val": ""});
        let output = encode_generic(&data);
        assert!(output.contains("val=\"\""));
    }

    #[test]
    fn test_generic_nested_array_in_row() {
        let data = json!({
            "users": [
                {"name": "Alice", "tags": ["admin", "user"]},
                {"name": "Bob", "tags": ["user"]},
            ]
        });
        let output = encode_generic(&data);
        // Should have @N prefix because of nested fields
        assert!(output.contains("@0 Alice"));
        assert!(output.contains("@1 Bob"));
        assert!(output.contains("tags["));
    }

    #[test]
    fn test_generic_non_uniform_array() {
        let data = json!({
            "items": [1, "two", true]
        });
        let output = encode_generic(&data);
        assert!(output.contains("items[3]: 1,two,true"));
    }

    #[test]
    fn test_generic_string_with_quotes_and_pipe() {
        let data = json!({"val": "say \"hello|world\""});
        let output = encode_generic(&data);
        // Should quote because of pipe, and escape inner quotes
        assert!(output.contains("val=\"say \\\"hello|world\\\"\""));
    }

    #[test]
    fn test_generic_top_level_array() {
        let data = json!([
            {"id": 1, "name": "x"},
            {"id": 2, "name": "y"},
        ]);
        let output = encode_generic(&data);
        assert!(output.contains("## root [2]{"));
        assert!(output.contains("1|x"));
        assert!(output.contains("2|y"));
    }
}
