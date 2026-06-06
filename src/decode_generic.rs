use crate::decode::decode;
use serde_json::{Map, Number, Value};

/// Decode GCF tabular text into a generic `serde_json::Value`.
///
/// Handles tabular arrays, key-value pairs, nested sections, inline primitive
/// arrays, nested fields in tabular rows, empty arrays, and value parsing
/// (`-` = null, `true`/`false` = bool, numbers, quoted strings).
///
/// If the input starts with `GCF ` (graph profile), falls back to [`decode()`]
/// and returns the Payload as a JSON object.
pub fn decode_generic(input: &str) -> Result<Value, Box<dyn std::error::Error>> {
    let input = input.trim_end_matches(['\n', '\r']);
    if input.is_empty() {
        return Ok(Value::Null);
    }

    let lines: Vec<&str> = input.split('\n').collect();

    // Graph profile fallback.
    if !lines.is_empty() && lines[0].starts_with("GCF ") {
        let p = decode(input)?;
        return Ok(payload_to_value(&p));
    }

    let mut result = Map::new();
    parse_object(&lines, 0, 0, &mut result);
    Ok(Value::Object(result))
}

/// Parse key=value, ## section, tabular array, and inline array lines at the
/// given indentation depth. Returns the number of lines consumed.
fn parse_object(lines: &[&str], start: usize, depth: usize, out: &mut Map<String, Value>) -> usize {
    let indent = "  ".repeat(depth);
    let mut i = start;

    while i < lines.len() {
        let trimmed = lines[i].trim_end_matches('\r');

        if trimmed.is_empty() || trimmed.starts_with("# ") {
            i += 1;
            continue;
        }

        // Check indentation: if less than expected depth, we're done.
        if depth > 0 && !trimmed.starts_with(&indent) {
            break;
        }

        let content = if depth > 0 {
            &trimmed[indent.len()..]
        } else {
            trimmed
        };

        // Skip _summary lines.
        if content.starts_with("## _summary") {
            i += 1;
            continue;
        }

        // Tabular array: ## name [count]{fields}
        if let Some(header) = content.strip_prefix("## ") {
            if let Some(bracket_idx) = header.find(" [") {
                let name = &header[..bracket_idx];
                let rest = &header[bracket_idx + 2..];
                if let Some(close_bracket) = rest.find(']') {
                    let after_bracket = &rest[close_bracket + 1..];
                    if after_bracket.starts_with('{') {
                        // Tabular with field declaration.
                        if let Some(field_end) = after_bracket.find('}') {
                            let fields: Vec<&str> =
                                after_bracket[1..field_end].split(',').collect();
                            i += 1;
                            let (rows, consumed) = parse_tabular_rows(lines, i, depth, &fields);
                            out.insert(name.to_string(), Value::Array(rows));
                            i += consumed;
                            continue;
                        }
                    } else {
                        // Count-only header.
                        let count_str = &rest[..close_bracket];
                        if count_str == "0" {
                            out.insert(name.to_string(), Value::Array(vec![]));
                            i += 1;
                            continue;
                        }
                        // Non-uniform array with @N items.
                        i += 1;
                        let (items, consumed) = parse_non_uniform_array(lines, i, depth);
                        out.insert(name.to_string(), Value::Array(items));
                        i += consumed;
                        continue;
                    }
                }
            }

            // Plain section header: ## key (nested object).
            let mut name = header;
            if let Some(idx) = name.find(" [") {
                name = &name[..idx];
            }
            i += 1;
            let mut nested = Map::new();
            let consumed = parse_object(lines, i, depth + 1, &mut nested);
            out.insert(name.to_string(), Value::Object(nested));
            i += consumed;
            continue;
        }

        // Inline primitive array: name[N]: val1,val2,...
        if let Some(bracket_idx) = content.find('[') {
            if bracket_idx > 0 {
                if let Some(colon_idx) = content.find("]: ") {
                    if colon_idx > bracket_idx {
                        let name = &content[..bracket_idx];
                        let vals_str = &content[colon_idx + 3..];
                        let vals = parse_primitive_values(vals_str);
                        out.insert(name.to_string(), Value::Array(vals));
                        i += 1;
                        continue;
                    }
                }
            }
        }

        // Key=value pair.
        if let Some(eq_idx) = content.find('=') {
            if eq_idx > 0 {
                let key = &content[..eq_idx];
                let val = &content[eq_idx + 1..];
                out.insert(key.to_string(), parse_value(val));
                i += 1;
                continue;
            }
        }

        // Unrecognized line, skip.
        i += 1;
    }

    i - start
}

/// Parse pipe-separated rows following a tabular header.
fn parse_tabular_rows(
    lines: &[&str],
    start: usize,
    depth: usize,
    fields: &[&str],
) -> (Vec<Value>, usize) {
    let indent = "  ".repeat(depth);
    let mut rows: Vec<Value> = Vec::new();
    let mut i = start;

    while i < lines.len() {
        let line = lines[i].trim_end_matches('\r');
        if line.is_empty() {
            i += 1;
            continue;
        }

        let content = if depth > 0 {
            if !line.starts_with(&indent) {
                break;
            }
            &line[indent.len()..]
        } else {
            line
        };

        // Stop at next section header or _summary.
        if content.starts_with("## ") {
            break;
        }

        // Skip comments.
        if content.starts_with("# ") {
            i += 1;
            continue;
        }

        // Strip @N prefix if present.
        let mut row_data = content;
        let mut has_nested = false;
        if row_data.starts_with('@') {
            if let Some(space_idx) = row_data.find(' ') {
                row_data = &row_data[space_idx + 1..];
                has_nested = true;
            }
        }

        // Parse pipe-separated values.
        let vals: Vec<&str> = row_data.split('|').collect();
        let mut row = Map::new();
        for (j, f) in fields.iter().enumerate() {
            if j < vals.len() {
                row.insert(f.to_string(), parse_value(vals[j]));
            } else {
                row.insert(f.to_string(), Value::Null);
            }
        }

        i += 1;

        // Parse nested fields (.fieldname).
        if has_nested {
            let nested_indent = format!("{}  ", indent);
            while i < lines.len() {
                let nested_line = lines[i].trim_end_matches('\r');
                if !nested_line.starts_with(&nested_indent) {
                    break;
                }
                let nested_content = &nested_line[nested_indent.len()..];

                if let Some(field_name) = nested_content.strip_prefix('.') {
                    i += 1;
                    let mut nested = Map::new();
                    let consumed = parse_object(lines, i, depth + 2, &mut nested);
                    row.insert(field_name.to_string(), Value::Object(nested));
                    i += consumed;
                } else {
                    break;
                }
            }
        }

        rows.push(Value::Object(row));
    }

    (rows, i - start)
}

/// Parse @N items in a non-uniform array section.
fn parse_non_uniform_array(lines: &[&str], start: usize, depth: usize) -> (Vec<Value>, usize) {
    let indent = "  ".repeat(depth);
    let mut items: Vec<Value> = Vec::new();
    let mut i = start;

    while i < lines.len() {
        let line = lines[i].trim_end_matches('\r');
        if line.is_empty() {
            i += 1;
            continue;
        }

        let content = if depth > 0 {
            if !line.starts_with(&indent) {
                break;
            }
            &line[indent.len()..]
        } else {
            line
        };

        if content.starts_with("## ") {
            break;
        }

        if content.starts_with('@') {
            if let Some(space_idx) = content.find(' ') {
                let val = &content[space_idx + 1..];
                items.push(parse_value(val));
            }
            i += 1;
        } else {
            break;
        }
    }

    (items, i - start)
}

/// Convert a slice of comma-separated tokens to typed values.
fn parse_primitive_values(vals_str: &str) -> Vec<Value> {
    vals_str.split(',').map(|t| parse_value(t.trim())).collect()
}

/// Convert a single GCF value string to a `serde_json::Value`.
fn parse_value(s: &str) -> Value {
    if s == "-" {
        return Value::Null;
    }
    if s == "true" {
        return Value::Bool(true);
    }
    if s == "false" {
        return Value::Bool(false);
    }
    if s == "\"\"" {
        return Value::String(String::new());
    }
    // Quoted string.
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        let inner = &s[1..s.len() - 1];
        let inner = inner.replace("\\\"", "\"").replace("\\\\", "\\");
        return Value::String(inner);
    }
    // Try integer.
    if let Ok(n) = s.parse::<i64>() {
        return Value::Number(Number::from(n));
    }
    // Try float.
    if let Ok(f) = s.parse::<f64>() {
        if let Some(n) = Number::from_f64(f) {
            return Value::Number(n);
        }
    }
    Value::String(s.to_string())
}

/// Convert a Payload to a generic JSON value for uniform return type.
fn payload_to_value(p: &crate::types::Payload) -> Value {
    let syms: Vec<Value> = p
        .symbols
        .iter()
        .map(|s| {
            let mut m = Map::new();
            m.insert(
                "qualifiedName".to_string(),
                Value::String(s.qualified_name.clone()),
            );
            m.insert("kind".to_string(), Value::String(s.kind.clone()));
            m.insert(
                "score".to_string(),
                Number::from_f64(s.score)
                    .map(Value::Number)
                    .unwrap_or(Value::Null),
            );
            m.insert(
                "provenance".to_string(),
                Value::String(s.provenance.clone()),
            );
            m.insert(
                "distance".to_string(),
                Value::Number(Number::from(s.distance)),
            );
            Value::Object(m)
        })
        .collect();

    let edges: Vec<Value> = p
        .edges
        .iter()
        .map(|e| {
            let mut m = Map::new();
            m.insert("source".to_string(), Value::String(e.source.clone()));
            m.insert("target".to_string(), Value::String(e.target.clone()));
            m.insert("edgeType".to_string(), Value::String(e.edge_type.clone()));
            if !e.status.is_empty() {
                m.insert("status".to_string(), Value::String(e.status.clone()));
            }
            Value::Object(m)
        })
        .collect();

    let mut m = Map::new();
    m.insert("tool".to_string(), Value::String(p.tool.clone()));
    m.insert(
        "tokenBudget".to_string(),
        Value::Number(Number::from(p.token_budget)),
    );
    m.insert(
        "tokensUsed".to_string(),
        Value::Number(Number::from(p.tokens_used)),
    );
    m.insert("packRoot".to_string(), Value::String(p.pack_root.clone()));
    m.insert("symbols".to_string(), Value::Array(syms));
    m.insert("edges".to_string(), Value::Array(edges));
    Value::Object(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_generic_tabular() {
        let input = "## employees [3]{id,name,department,salary}\n1|Alice|Engineering|95000\n2|Bob|Sales|72000\n3|Carol|Marketing|85000\n";
        let result = decode_generic(input).unwrap();
        let employees = result["employees"].as_array().unwrap();
        assert_eq!(employees.len(), 3);
        assert_eq!(employees[0]["name"], "Alice");
        assert_eq!(employees[0]["salary"], 95000);
    }

    #[test]
    fn test_decode_generic_key_value() {
        let input = "name=my-service\nversion=2.1.0\nport=5432\nactive=true\n";
        let result = decode_generic(input).unwrap();
        assert_eq!(result["name"], "my-service");
        assert_eq!(result["port"], 5432);
        assert_eq!(result["active"], true);
    }

    #[test]
    fn test_decode_generic_nested_sections() {
        let input =
            "name=app\n## database\n  host=db.example.com\n  port=5432\n## cache\n  ttl=3600\n";
        let result = decode_generic(input).unwrap();
        assert_eq!(result["database"]["host"], "db.example.com");
        assert_eq!(result["cache"]["ttl"], 3600);
    }

    #[test]
    fn test_decode_generic_inline_primitive_array() {
        let input = "name=svc\ntags[3]: production,us-east-1,critical\nports[2]: 8080,8443\n";
        let result = decode_generic(input).unwrap();
        let tags = result["tags"].as_array().unwrap();
        assert_eq!(tags.len(), 3);
        assert_eq!(tags[0], "production");
        let ports = result["ports"].as_array().unwrap();
        assert_eq!(ports[0], 8080);
    }

    #[test]
    fn test_decode_generic_tabular_with_nested() {
        let input = "## orders [2]{id,total,status}\n@0 1001|249.99|shipped\n  .customer\n    name=Alice\n    tier=premium\n@1 1002|89.50|pending\n  .customer\n    name=Bob\n    tier=standard\n";
        let result = decode_generic(input).unwrap();
        let orders = result["orders"].as_array().unwrap();
        assert_eq!(orders.len(), 2);
        assert_eq!(orders[0]["id"], 1001);
        assert_eq!(orders[0]["customer"]["name"], "Alice");
    }

    #[test]
    fn test_decode_generic_graph_fallback() {
        let input = "GCF tool=test budget=100 tokens=50 symbols=1 edges=0\n## targets\n@0 fn a.A 0.90 lsp\n";
        let result = decode_generic(input).unwrap();
        assert_eq!(result["tool"], "test");
        let syms = result["symbols"].as_array().unwrap();
        assert_eq!(syms.len(), 1);
    }

    #[test]
    fn test_decode_generic_empty_array() {
        let input = "## items [0]\n";
        let result = decode_generic(input).unwrap();
        let items = result["items"].as_array().unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn test_decode_generic_null_and_booleans() {
        let input = "active=true\ndisabled=false\nmissing=-\n";
        let result = decode_generic(input).unwrap();
        assert_eq!(result["active"], true);
        assert_eq!(result["disabled"], false);
        assert!(result["missing"].is_null());
    }

    #[test]
    fn test_decode_generic_empty_input() {
        let result = decode_generic("").unwrap();
        assert!(result.is_null());
    }

    #[test]
    fn test_decode_generic_roundtrip() {
        let data = serde_json::json!({
            "employees": [
                {"id": 1, "name": "Alice", "department": "Engineering", "salary": 95000},
                {"id": 2, "name": "Bob", "department": "Sales", "salary": 72000},
            ]
        });
        let encoded = crate::generic::encode_generic(&data);
        let decoded = decode_generic(&encoded).unwrap();
        let employees = decoded["employees"].as_array().unwrap();
        assert_eq!(employees.len(), 2);
        assert_eq!(employees[0]["name"], "Alice");
    }
}
