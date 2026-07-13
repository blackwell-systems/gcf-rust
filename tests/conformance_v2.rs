//! Conformance tests for GCF v2.0 (133 fixtures).

use gcf::{
    decode_generic, decode_generic_delta, encode_delta, encode_generic, encode_generic_delta,
    encode_with_session, generic_pack_root, pack_root, pack_root_canonical_bytes,
    verify_generic_delta, DeltaPayload, Edge, GenericDeltaPayload, GenericDeltaSession, GenericSet,
    Payload, ReanchorPolicy, Session, StreamEncoder, StreamOptions, Symbol,
};
use serde_json::{Map, Value};
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
    options: Option<Value>,
    /// Captures fixture keys not modeled above (e.g. `calls`, `canonicalBytes`,
    /// `base_snapshot`) so no operation is silently starved of its inputs.
    #[serde(flatten)]
    extra: serde_json::Map<String, Value>,
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

fn set_from_value(v: &Value) -> GenericSet {
    GenericSet {
        name: v["name"].as_str().unwrap_or("").to_string(),
        key: v["key"].as_str().unwrap_or("").to_string(),
        fields: v["fields"]
            .as_array()
            .map(|a| a.iter().map(|f| f.as_str().unwrap().to_string()).collect())
            .unwrap_or_default(),
        rows: v["rows"]
            .as_array()
            .map(|a| a.iter().map(|r| r.as_object().unwrap().clone()).collect())
            .unwrap_or_default(),
    }
}

fn rows_from(v: &Value, key: &str) -> Vec<Map<String, Value>> {
    v.get(key)
        .and_then(|x| x.as_array())
        .map(|a| a.iter().map(|r| r.as_object().unwrap().clone()).collect())
        .unwrap_or_default()
}

fn delta_from_value(v: &Value) -> GenericDeltaPayload {
    GenericDeltaPayload {
        tool: v["tool"].as_str().unwrap_or("").to_string(),
        key: v["key"].as_str().unwrap_or("").to_string(),
        fields: v["fields"]
            .as_array()
            .map(|a| a.iter().map(|f| f.as_str().unwrap().to_string()).collect())
            .unwrap_or_default(),
        base_root: v["baseRoot"].as_str().unwrap_or("").to_string(),
        new_root: v["newRoot"].as_str().unwrap_or("").to_string(),
        added: rows_from(v, "added"),
        changed: rows_from(v, "changed"),
        removed: v
            .get("removed")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default(),
        delta_tokens: v.get("deltaTokens").and_then(|x| x.as_u64()).unwrap_or(0),
        full_tokens: v.get("fullTokens").and_then(|x| x.as_u64()).unwrap_or(0),
    }
}

/// Build a graph Symbol vector from a fixture input's `symbols` array.
fn symbols_from(v: &Value) -> Vec<Symbol> {
    v["symbols"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|sym| Symbol {
            qualified_name: sym["qualifiedName"].as_str().unwrap_or("").to_string(),
            kind: sym["kind"].as_str().unwrap_or("").to_string(),
            score: sym["score"].as_f64().unwrap_or(0.0),
            provenance: sym["provenance"].as_str().unwrap_or("").to_string(),
            distance: sym["distance"].as_i64().unwrap_or(0) as i32,
            signature: String::new(),
            components: Default::default(),
        })
        .collect()
}

/// Build a graph Edge vector from a fixture input's `edges` array.
fn edges_from(v: &Value) -> Vec<Edge> {
    v["edges"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|edge| Edge {
            source: edge["source"].as_str().unwrap_or("").to_string(),
            target: edge["target"].as_str().unwrap_or("").to_string(),
            edge_type: edge["edgeType"].as_str().unwrap_or("").to_string(),
            status: edge["status"].as_str().unwrap_or("").to_string(),
        })
        .collect()
}

/// Build a list of graph Symbols from a plain array of {qualifiedName, kind, score, provenance}
/// (delta added/removed sections carry no distance).
fn delta_symbols_from(v: &Value, key: &str) -> Vec<Symbol> {
    v.get(key)
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|sym| Symbol {
            qualified_name: sym["qualifiedName"].as_str().unwrap_or("").to_string(),
            kind: sym["kind"].as_str().unwrap_or("").to_string(),
            score: sym["score"].as_f64().unwrap_or(0.0),
            provenance: sym["provenance"].as_str().unwrap_or("").to_string(),
            distance: 0,
            signature: String::new(),
            components: Default::default(),
        })
        .collect()
}

fn delta_edges_from(v: &Value, key: &str) -> Vec<Edge> {
    v.get(key)
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|edge| Edge {
            source: edge["source"].as_str().unwrap_or("").to_string(),
            target: edge["target"].as_str().unwrap_or("").to_string(),
            edge_type: edge["edgeType"].as_str().unwrap_or("").to_string(),
            status: String::new(),
        })
        .collect()
}

#[test]
fn test_conformance_v2() {
    let fixtures = load_fixtures();
    if fixtures.is_empty() {
        eprintln!("SKIP: conformance fixtures not found");
        return;
    }
    // Floor assertion: a green run MUST have exercised the full shared suite. A
    // present-but-short fixture set (mispathed or partial checkout) fails loudly
    // rather than passing having verified almost nothing. A wholly-absent directory
    // yields an empty vec and soft-skips above; in CI the separate gcf checkout step
    // fails loudly if the repo cannot be cloned.
    const MIN_FIXTURES: usize = 150;
    assert!(
        fixtures.len() >= MIN_FIXTURES,
        "discovered only {} conformance fixtures, expected at least {}; the shared gcf fixture set is incomplete or mispathed",
        fixtures.len(),
        MIN_FIXTURES
    );

    let mut passed = 0;
    let mut skipped = 0;
    let mut failed = 0;

    for (rel_path, fix) in &fixtures {
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
                    // Buffered graph encode (distinct from generic encode and the
                    // streaming encoder). Build a graph Payload and compare bytes.
                    let inp = match fix.input.as_ref() {
                        Some(v) => v,
                        None => {
                            skipped += 1;
                            continue;
                        }
                    };
                    let mut payload = Payload {
                        tool: inp["tool"].as_str().unwrap_or("").to_string(),
                        token_budget: inp["tokenBudget"].as_i64().unwrap_or(0),
                        tokens_used: inp["tokensUsed"].as_i64().unwrap_or(0),
                        pack_root: inp["packRoot"].as_str().unwrap_or("").to_string(),
                        symbols: Vec::new(),
                        edges: Vec::new(),
                    };
                    for sym in inp["symbols"].as_array().cloned().unwrap_or_default() {
                        payload.symbols.push(Symbol {
                            qualified_name: sym["qualifiedName"].as_str().unwrap_or("").to_string(),
                            kind: sym["kind"].as_str().unwrap_or("").to_string(),
                            score: sym["score"].as_f64().unwrap_or(0.0),
                            provenance: sym["provenance"].as_str().unwrap_or("").to_string(),
                            distance: sym["distance"].as_i64().unwrap_or(0) as i32,
                            signature: String::new(),
                            components: Default::default(),
                        });
                    }
                    for edge in inp["edges"].as_array().cloned().unwrap_or_default() {
                        payload.edges.push(Edge {
                            source: edge["source"].as_str().unwrap_or("").to_string(),
                            target: edge["target"].as_str().unwrap_or("").to_string(),
                            edge_type: edge["edgeType"].as_str().unwrap_or("").to_string(),
                            status: edge["status"].as_str().unwrap_or("").to_string(),
                        });
                    }
                    let got = gcf::encode(&payload);
                    if got != expected_str {
                        eprintln!(
                            "FAIL {}: graph-encode mismatch\n  got: {:?}\n  exp: {:?}",
                            rel_path, got, expected_str
                        );
                        failed += 1;
                    } else {
                        passed += 1;
                    }
                    continue;
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
            "generic-pack-root" => {
                let set = set_from_value(fix.input.as_ref().unwrap());
                let got = generic_pack_root(&set);
                let exp = fix.expected.as_ref().and_then(|v| v.as_str()).unwrap();
                if got != exp {
                    eprintln!(
                        "FAIL {}: pack-root mismatch\n  got: {}\n  exp: {}",
                        rel_path, got, exp
                    );
                    failed += 1;
                } else {
                    passed += 1;
                }
            }
            "generic-delta" => {
                let d = delta_from_value(fix.input.as_ref().unwrap());
                let got = encode_generic_delta(&d);
                let exp = fix.expected.as_ref().and_then(|v| v.as_str()).unwrap();
                if got != exp {
                    eprintln!(
                        "FAIL {}: delta encode mismatch\n  got: {:?}\n  exp: {:?}",
                        rel_path, got, exp
                    );
                    failed += 1;
                } else {
                    passed += 1;
                }
            }
            "generic-delta-verify" | "generic-delta-decode" => {
                let inp = fix.input.as_ref().unwrap();
                let base = set_from_value(&inp["base"]);
                let expected_new_root = inp["expectedNewRoot"].as_str().unwrap();
                let outcome = if fix.operation == "generic-delta-verify" {
                    verify_generic_delta(&base, &delta_from_value(&inp["delta"]), expected_new_root)
                } else {
                    match decode_generic_delta(inp["wire"].as_str().unwrap()) {
                        Ok(d) => verify_generic_delta(&base, &d, expected_new_root),
                        Err(e) => Err(e),
                    }
                };
                match (outcome, fix.expected_error.as_ref()) {
                    (Ok(res), None) => {
                        let exp = fix.expected.as_ref().and_then(|v| v.as_str()).unwrap();
                        if generic_pack_root(&res) != exp {
                            eprintln!("FAIL {}: applied root mismatch", rel_path);
                            failed += 1;
                        } else {
                            passed += 1;
                        }
                    }
                    (Ok(_), Some(exp_err)) => {
                        eprintln!(
                            "FAIL {}: expected error '{}', got success",
                            rel_path, exp_err
                        );
                        failed += 1;
                    }
                    (Err(e), Some(exp_err)) => {
                        if e.contains(exp_err) {
                            passed += 1;
                        } else {
                            eprintln!(
                                "FAIL {}: wrong error\n  got: {}\n  expected: {}",
                                rel_path, e, exp_err
                            );
                            failed += 1;
                        }
                    }
                    (Err(e), None) => {
                        eprintln!("FAIL {}: unexpected error: {}", rel_path, e);
                        failed += 1;
                    }
                }
            }
            "generic-delta-session" => {
                let inp = fix.input.as_ref().unwrap();
                let expected = fix.expected.as_ref().unwrap();
                let base = set_from_value(&inp["base"]);
                let tool = inp["tool"].as_str().unwrap_or("").to_string();
                let policy = match inp["policy"]["mode"].as_str().unwrap_or("fixedN") {
                    "sizeGuard" => ReanchorPolicy::size_guard(),
                    _ => ReanchorPolicy::fixed_n(inp["policy"]["n"].as_u64().unwrap_or(0) as usize),
                };
                let mut s = GenericDeltaSession::new(base, tool, policy);
                let initial_full = expected["initialFull"].as_str().unwrap();
                let mut ok = true;
                if s.current_full() != initial_full {
                    eprintln!(
                        "FAIL {}: initial full mismatch\n  got: {:?}\n  exp: {:?}",
                        rel_path,
                        s.current_full(),
                        initial_full
                    );
                    ok = false;
                }
                let updates = inp["updates"].as_array().cloned().unwrap_or_default();
                let emissions = expected["emissions"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                for (i, up) in updates.iter().enumerate() {
                    let (wire, is_full) = match s.next(set_from_value(up)) {
                        Ok(r) => r,
                        Err(e) => {
                            eprintln!("FAIL {}: turn {} error: {}", rel_path, i + 1, e);
                            ok = false;
                            break;
                        }
                    };
                    let exp_full = emissions[i]["isFull"].as_bool().unwrap();
                    let exp_wire = emissions[i]["wire"].as_str().unwrap();
                    if is_full != exp_full {
                        eprintln!(
                            "FAIL {}: turn {} isFull={}, want {}",
                            rel_path,
                            i + 1,
                            is_full,
                            exp_full
                        );
                        ok = false;
                    }
                    if wire != exp_wire {
                        eprintln!(
                            "FAIL {}: turn {} wire mismatch\n  got: {:?}\n  exp: {:?}",
                            rel_path,
                            i + 1,
                            wire,
                            exp_wire
                        );
                        ok = false;
                    }
                }
                if ok {
                    passed += 1;
                } else {
                    failed += 1;
                }
            }
            "graph-stream-encode" => {
                // Skip a fixture requesting stream options this runner does not support.
                // labeledTrailerCounts (SPEC 8.4.1) IS supported; skip only if the options
                // object carries any OTHER key.
                if fix
                    .options
                    .as_ref()
                    .and_then(|o| o.as_object())
                    .map_or(false, |m| m.keys().any(|k| k != "labeledTrailerCounts"))
                {
                    skipped += 1;
                    continue;
                }
                let labeled_trailer_counts = fix
                    .options
                    .as_ref()
                    .and_then(|o| o.get("labeledTrailerCounts"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let inp = fix.input.as_ref().unwrap();
                let expected = match fix.expected.as_ref().and_then(|v| v.as_str()) {
                    Some(s) => s,
                    None => {
                        skipped += 1;
                        continue;
                    }
                };
                let tool = inp["tool"].as_str().unwrap_or("");
                let opts = StreamOptions {
                    token_budget: inp["tokenBudget"].as_i64().unwrap_or(0),
                    tokens_used: inp["tokensUsed"].as_i64().unwrap_or(0),
                    pack_root: inp["packRoot"].as_str().unwrap_or("").to_string(),
                    session: inp["session"].as_bool().unwrap_or(false),
                    labeled_trailer_counts,
                };
                let mut buf: Vec<u8> = Vec::new();
                {
                    let enc = StreamEncoder::new(&mut buf, tool, opts);
                    for sym in inp["symbols"].as_array().cloned().unwrap_or_default() {
                        enc.write_symbol(&Symbol {
                            qualified_name: sym["qualifiedName"].as_str().unwrap_or("").to_string(),
                            kind: sym["kind"].as_str().unwrap_or("").to_string(),
                            score: sym["score"].as_f64().unwrap_or(0.0),
                            provenance: sym["provenance"].as_str().unwrap_or("").to_string(),
                            distance: sym["distance"].as_i64().unwrap_or(0) as i32,
                            signature: String::new(),
                            components: Default::default(),
                        });
                    }
                    for edge in inp["edges"].as_array().cloned().unwrap_or_default() {
                        enc.write_edge(&Edge {
                            source: edge["source"].as_str().unwrap_or("").to_string(),
                            target: edge["target"].as_str().unwrap_or("").to_string(),
                            edge_type: edge["edgeType"].as_str().unwrap_or("").to_string(),
                            status: String::new(),
                        });
                    }
                    enc.close();
                }
                let got = String::from_utf8(buf).unwrap();
                if got != expected {
                    eprintln!(
                        "FAIL {}: graph-stream-encode mismatch\n  got: {:?}\n  exp: {:?}",
                        rel_path, got, expected
                    );
                    failed += 1;
                } else {
                    passed += 1;
                }
            }
            "pack-root" => {
                // Graph pack root: content-addressed sha256 of the canonical snapshot.
                let inp = fix.input.as_ref().unwrap();
                let symbols = symbols_from(inp);
                let edges = edges_from(inp);
                let exp = fix.expected.as_ref().and_then(|v| v.as_str()).unwrap();
                // If the fixture carries the exact pre-hash bytes, verify them too so
                // any divergence is caught before the hash rather than only in the digest.
                if let Some(exp_bytes) = fix
                    .extra
                    .get("canonicalBytes")
                    .and_then(|v| v.as_str())
                {
                    let got_bytes = pack_root_canonical_bytes(&symbols, &edges);
                    if got_bytes != exp_bytes {
                        eprintln!(
                            "FAIL {}: pack-root canonicalBytes mismatch\n  got: {:?}\n  exp: {:?}",
                            rel_path, got_bytes, exp_bytes
                        );
                        failed += 1;
                        continue;
                    }
                }
                let got = pack_root(&symbols, &edges);
                if got != exp {
                    eprintln!(
                        "FAIL {}: pack-root mismatch\n  got: {}\n  exp: {}",
                        rel_path, got, exp
                    );
                    failed += 1;
                } else {
                    passed += 1;
                }
            }
            "roundtrip" => {
                let input = match fix.input.as_ref() {
                    Some(v) => v,
                    None => {
                        skipped += 1;
                        continue;
                    }
                };
                let encoded = encode_generic(input);
                // If expected is a string, verify the encoded output matches it.
                if let Some(Value::String(exp)) = fix.expected.as_ref() {
                    if &encoded != exp {
                        eprintln!(
                            "FAIL {}: roundtrip encode mismatch\n  got: {:?}\n  exp: {:?}",
                            rel_path, encoded, exp
                        );
                        failed += 1;
                        continue;
                    }
                }
                // Verify round-trip: decode(encode(input)) == input.
                match decode_generic(&encoded) {
                    Ok(decoded) => {
                        if !structural_equal(input, &decoded) {
                            eprintln!(
                                "FAIL {}: roundtrip mismatch\n  input: {}\n  decoded: {}",
                                rel_path, input, decoded
                            );
                            failed += 1;
                        } else {
                            passed += 1;
                        }
                    }
                    Err(e) => {
                        eprintln!("FAIL {}: roundtrip decode error: {}", rel_path, e);
                        failed += 1;
                    }
                }
            }
            "session" => {
                // A single Session carries state across all calls.
                let calls = fix
                    .extra
                    .get("calls")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let session = Session::new();
                let mut ok = true;
                for (i, call) in calls.iter().enumerate() {
                    let inp = &call["input"];
                    let payload = Payload {
                        tool: inp["tool"].as_str().unwrap_or("").to_string(),
                        token_budget: 0,
                        tokens_used: 0,
                        pack_root: String::new(),
                        symbols: symbols_from(inp),
                        edges: edges_from(inp),
                    };
                    let got = encode_with_session(&payload, &session);
                    let exp = call["expected"].as_str().unwrap_or("");
                    if got != exp {
                        eprintln!(
                            "FAIL {}: session call {} mismatch\n  got: {:?}\n  exp: {:?}",
                            rel_path,
                            i + 1,
                            got,
                            exp
                        );
                        ok = false;
                    }
                }
                if ok {
                    passed += 1;
                } else {
                    failed += 1;
                }
            }
            "delta" => {
                // graph-delta fixtures share the "delta" operation but come in two
                // shapes: an ENCODE scenario (input is a DeltaPayload struct) and a
                // VERIFY scenario (input is a pre-encoded wire string, with
                // base_snapshot present). The verify shape needs the graph delta wire
                // decoder, which is not yet implemented, so route it to the same
                // allow-listed skip as delta-verify.
                let is_verify_shape = matches!(fix.input.as_ref(), Some(Value::String(_)))
                    || fix.extra.contains_key("base_snapshot");
                if is_verify_shape {
                    eprintln!(
                        "SKIP {}: delta-verify: graph delta wire decoder not yet implemented",
                        rel_path
                    );
                    skipped += 1;
                    continue;
                }
                let inp = fix.input.as_ref().unwrap();
                let full_tokens = inp.get("fullTokens").and_then(|x| x.as_i64()).unwrap_or(0);
                let delta_tokens = inp.get("deltaTokens").and_then(|x| x.as_i64()).unwrap_or(0);
                let d = DeltaPayload {
                    tool: inp["tool"].as_str().unwrap_or("").to_string(),
                    base_root: inp["baseRoot"].as_str().unwrap_or("").to_string(),
                    new_root: inp["newRoot"].as_str().unwrap_or("").to_string(),
                    removed: delta_symbols_from(inp, "removed"),
                    added: delta_symbols_from(inp, "added"),
                    removed_edges: delta_edges_from(inp, "removedEdges"),
                    added_edges: delta_edges_from(inp, "addedEdges"),
                    delta_tokens,
                    full_tokens,
                };
                let exp = fix.expected.as_ref().and_then(|v| v.as_str()).unwrap();
                let got = encode_delta(&d);
                if got != exp {
                    eprintln!(
                        "FAIL {}: delta encode mismatch\n  got: {:?}\n  exp: {:?}",
                        rel_path, got, exp
                    );
                    failed += 1;
                } else {
                    passed += 1;
                }
            }
            "delta-verify" => {
                // Graph delta wire decoder not yet implemented in rust.
                eprintln!(
                    "SKIP {}: delta-verify: graph delta wire decoder not yet implemented",
                    rel_path
                );
                skipped += 1;
            }
            op => {
                panic!(
                    "unhandled operation {:?}; must be handled or explicitly allow-listed",
                    op
                );
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
