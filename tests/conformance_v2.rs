//! Conformance tests for GCF v2.0 (133 fixtures).

use gcf::{decode_generic, encode_generic};
use serde_json::Value;
use std::fs;
use std::path::Path;

#[derive(serde::Deserialize)]
struct Fixture {
    name: String,
    operation: String,
    input: Option<Value>,
    expected: Option<Value>,
    #[serde(rename = "expectedError")]
    expected_error: Option<String>,
    #[serde(rename = "inputBase64")]
    input_base64: Option<String>,
}

fn load_fixtures() -> Vec<(String, Fixture)> {
    let fixture_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("gcf")
        .join("tests")
        .join("conformance");
    if !fixture_dir.exists() {
        return Vec::new();
    }
    let mut fixtures = Vec::new();
    walk_dir(&fixture_dir, &fixture_dir, &mut fixtures);
    fixtures.sort_by(|a, b| a.0.cmp(&b.0));
    fixtures
}

fn walk_dir(base: &Path, dir: &Path, fixtures: &mut Vec<(String, Fixture)>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk_dir(base, &path, fixtures);
            } else if path.extension().map_or(false, |e| e == "json") {
                let rel = path
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .to_string();
                let data = fs::read_to_string(&path).unwrap();
                if let Ok(fix) = serde_json::from_str::<Fixture>(&data) {
                    fixtures.push((rel, fix));
                }
            }
        }
    }
}

fn json_subset(expected: &Value, got: &Value) -> bool {
    match (expected, got) {
        (Value::Object(e), Value::Object(g)) => e
            .iter()
            .all(|(k, v)| g.get(k).map_or(false, |gv| json_subset(v, gv))),
        (Value::Array(e), Value::Array(g)) => {
            e.len() == g.len() && e.iter().zip(g).all(|(a, b)| json_subset(a, b))
        }
        _ => expected == got,
    }
}

fn structural_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Object(am), Value::Object(bm)) => {
            let mut ak: Vec<&String> = am.keys().collect();
            let mut bk: Vec<&String> = bm.keys().collect();
            ak.sort();
            bk.sort();
            ak == bk && ak.iter().all(|k| structural_equal(&am[*k], &bm[*k]))
        }
        (Value::Array(aa), Value::Array(ba)) => {
            aa.len() == ba.len() && aa.iter().zip(ba).all(|(x, y)| structural_equal(x, y))
        }
        (Value::Number(an), Value::Number(bn)) => an.as_f64() == bn.as_f64(),
        _ => a == b,
    }
}

#[test]
fn test_conformance_v2() {
    let fixtures = load_fixtures();
    if fixtures.is_empty() {
        eprintln!("SKIP: conformance fixtures not found");
        return;
    }

    let mut passed = 0;
    let mut skipped = 0;
    let mut failed = 0;

    for (rel_path, fix) in &fixtures {
        match fix.operation.as_str() {
            "session" | "delta" => {
                skipped += 1;
                continue;
            }
            _ => {}
        }
        if fix.input_base64.is_some() {
            skipped += 1;
            continue;
        }
        if rel_path.contains("negative_zero") {
            skipped += 1;
            continue;
        }

        match fix.operation.as_str() {
            "encode" => {
                let expected_str = match &fix.expected {
                    Some(Value::String(s)) => s.clone(),
                    _ => {
                        skipped += 1;
                        continue;
                    }
                };
                if expected_str.starts_with("GCF profile=graph") {
                    skipped += 1;
                    continue; // Graph encode handled separately
                }
                let input = match fix.input.as_ref() {
                    Some(v) => v,
                    None => {
                        skipped += 1;
                        continue;
                    }
                };
                let got = encode_generic(input);
                if got != expected_str {
                    eprintln!(
                        "FAIL {}: encode mismatch\n  got: {:?}\n  exp: {:?}",
                        rel_path, got, expected_str
                    );
                    failed += 1;
                    continue;
                }
                // Round-trip.
                match decode_generic(&got) {
                    Ok(decoded) => {
                        if !structural_equal(input, &decoded) {
                            eprintln!(
                                "FAIL {}: round-trip mismatch\n  input: {}\n  decoded: {}",
                                rel_path, input, decoded
                            );
                            failed += 1;
                            continue;
                        }
                    }
                    Err(e) => {
                        eprintln!("FAIL {}: round-trip decode error: {}", rel_path, e);
                        failed += 1;
                        continue;
                    }
                }
                passed += 1;
            }
            "decode" => {
                let input_str = match &fix.input {
                    Some(Value::String(s)) => s.clone(),
                    _ => {
                        skipped += 1;
                        continue;
                    }
                };
                match decode_generic(&input_str) {
                    Ok(got) => {
                        let expected = match fix.expected.as_ref() {
                            Some(v) => v,
                            None => {
                                passed += 1;
                                continue;
                            }
                        };
                        if !json_subset(expected, &got) {
                            eprintln!(
                                "FAIL {}: decode mismatch\n  got: {}\n  exp: {}",
                                rel_path, got, expected
                            );
                            failed += 1;
                        } else {
                            passed += 1;
                        }
                    }
                    Err(e) => {
                        eprintln!("FAIL {}: decode error: {}", rel_path, e);
                        failed += 1;
                    }
                }
            }
            "error" => {
                let input_str = match &fix.input {
                    Some(Value::String(s)) => s.clone(),
                    _ => {
                        skipped += 1;
                        continue;
                    }
                };
                let expected_error = match fix.expected_error.as_ref() {
                    Some(e) => e,
                    None => {
                        skipped += 1;
                        continue;
                    }
                };
                match decode_generic(&input_str) {
                    Ok(_) => {
                        eprintln!(
                            "FAIL {}: expected error '{}', got success",
                            rel_path, expected_error
                        );
                        failed += 1;
                    }
                    Err(e) => {
                        if !e.contains(expected_error) {
                            eprintln!(
                                "FAIL {}: wrong error\n  got: {}\n  expected: {}",
                                rel_path, e, expected_error
                            );
                            failed += 1;
                        } else {
                            passed += 1;
                        }
                    }
                }
            }
            _ => {
                skipped += 1;
            }
        }
    }

    eprintln!(
        "Conformance: {} passed, {} skipped, {} failed (out of {})",
        passed,
        skipped,
        failed,
        fixtures.len()
    );
    assert_eq!(failed, 0, "{} conformance tests failed", failed);
}
