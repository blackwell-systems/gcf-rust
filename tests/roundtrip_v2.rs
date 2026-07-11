//! Property-based round-trip tests for GCF v2.0.

use gcf::{decode_generic, encode_generic};
use serde_json::{json, Value};
use std::collections::HashMap;

const DEFAULT_ITERATIONS: usize = 100_000;

fn get_iterations() -> usize {
    std::env::var("GCF_ITERATIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_ITERATIONS)
}

// Simple xorshift32 PRNG.
struct Rng(u32);
impl Rng {
    fn new(seed: u32) -> Self {
        Self(seed)
    }
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }
    fn int(&mut self, max: u32) -> u32 {
        self.next() % max
    }
    fn float(&mut self) -> f64 {
        (self.next() as f64) / (u32::MAX as f64)
    }
}

fn gen_value(rng: &mut Rng, depth: usize, max_depth: usize) -> Value {
    if depth >= max_depth {
        return gen_scalar(rng);
    }
    match rng.int(10) {
        0 => Value::Null,
        1 => Value::Bool(rng.float() < 0.5),
        2 => json!(gen_number(rng)),
        3 | 4 => Value::String(gen_string(rng)),
        5 | 6 => gen_object(rng, depth, max_depth),
        7 | 8 => gen_array(rng, depth, max_depth),
        _ => gen_scalar(rng),
    }
}

fn gen_scalar(rng: &mut Rng) -> Value {
    match rng.int(5) {
        0 => Value::Null,
        1 => Value::Bool(rng.float() < 0.5),
        2 => json!(gen_number(rng)),
        _ => Value::String(gen_string(rng)),
    }
}

fn gen_number(rng: &mut Rng) -> f64 {
    match rng.int(7) {
        0 => 0.0,
        1 => (rng.int(1000)) as f64,
        2 => -(rng.int(1000) as f64),
        3 => (rng.int(1000000) as f64) + rng.float(),
        4 => -0.0_f64,
        5 => ((rng.int(999) + 1) as f64) * 1e18,
        6 => ((rng.int(999) + 1) as f64) * 1e-10,
        _ => rng.float() * 2000.0 - 1000.0,
    }
}

fn gen_string(rng: &mut Rng) -> String {
    let n = rng.int(20) as usize;
    let chars = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let special = b" |,=\"\\#@\n\t~^+-.";
    let mut s = String::with_capacity(n);
    for _ in 0..n {
        if rng.float() < 0.2 {
            s.push(special[rng.int(special.len() as u32) as usize] as char);
        } else {
            s.push(chars[rng.int(chars.len() as u32) as usize] as char);
        }
    }
    s
}

fn gen_bare_key(rng: &mut Rng) -> String {
    let chars = b"abcdefghijklmnopqrstuvwxyz_";
    let n = 1 + rng.int(8) as usize;
    (0..n)
        .map(|_| chars[rng.int(chars.len() as u32) as usize] as char)
        .collect()
}

fn gen_object(rng: &mut Rng, depth: usize, max_depth: usize) -> Value {
    let n = rng.int(6) as usize;
    let mut map = serde_json::Map::new();
    for _ in 0..n {
        let key = gen_bare_key(rng);
        if !map.contains_key(&key) {
            map.insert(key, gen_value(rng, depth + 1, max_depth));
        }
    }
    Value::Object(map)
}

fn gen_array(rng: &mut Rng, depth: usize, max_depth: usize) -> Value {
    let n = rng.int(6) as usize;
    let mut arr = Vec::with_capacity(n);
    match rng.int(4) {
        0 => {
            for _ in 0..n {
                arr.push(gen_scalar(rng));
            }
        }
        1 => {
            let fields: Vec<String> = (0..1 + rng.int(4) as usize)
                .map(|_| gen_bare_key(rng))
                .collect();
            for _ in 0..n {
                let mut obj = serde_json::Map::new();
                for f in &fields {
                    if rng.float() > 0.2 {
                        obj.insert(f.clone(), gen_scalar(rng));
                    }
                }
                arr.push(Value::Object(obj));
            }
        }
        2 => {
            for _ in 0..n {
                let mut obj = serde_json::Map::new();
                obj.insert(gen_bare_key(rng), gen_scalar(rng));
                if rng.float() < 0.3 && depth + 1 < max_depth {
                    obj.insert(gen_bare_key(rng), gen_value(rng, depth + 2, max_depth));
                }
                arr.push(Value::Object(obj));
            }
        }
        _ => {
            for _ in 0..n {
                arr.push(gen_value(rng, depth + 1, max_depth));
            }
        }
    }
    Value::Array(arr)
}

fn numeric_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(an), Value::Number(bn)) => an.as_f64() == bn.as_f64(),
        _ => false,
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
        (Value::Number(_), Value::Number(_)) => numeric_equal(a, b),
        _ => a == b,
    }
}

// A fixed nested schema: a scalar leaf or an ordered set of named sub-shapes.
enum FlatShape {
    Scalar,
    Nested(Vec<(String, FlatShape)>),
}

fn gen_flat_shape(rng: &mut Rng, depth: usize, max_depth: usize) -> FlatShape {
    if depth >= max_depth || rng.float() < 0.45 {
        return FlatShape::Scalar;
    }
    let mut sub = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for _ in 0..1 + rng.int(3) as usize {
        let k = gen_bare_key(rng);
        if seen.insert(k.clone()) {
            sub.push((k, gen_flat_shape(rng, depth + 1, max_depth)));
        }
    }
    if sub.is_empty() {
        FlatShape::Scalar
    } else {
        FlatShape::Nested(sub)
    }
}

fn materialize_flat_shape(rng: &mut Rng, shape: &FlatShape) -> Value {
    match shape {
        FlatShape::Scalar => gen_scalar(rng),
        FlatShape::Nested(sub) => {
            let mut map = serde_json::Map::new();
            for (k, s) in sub {
                // A nested sub-object is sometimes null (intermediate null — the case the
                // pre-fix encoder dropped) instead of a full object.
                let v = if !matches!(s, FlatShape::Scalar) && rng.float() < 0.3 {
                    Value::Null
                } else {
                    materialize_flat_shape(rng, s)
                };
                map.insert(k.clone(), v);
            }
            Value::Object(map)
        }
    }
}

fn gen_flattenable_array(rng: &mut Rng) -> Value {
    let mut schema: Vec<(String, FlatShape)> = vec![("id".to_string(), FlatShape::Scalar)];
    let mut seen = std::collections::HashSet::new();
    seen.insert("id".to_string());
    let mut has_nested = false;
    for _ in 0..1 + rng.int(3) as usize {
        let k = gen_bare_key(rng);
        if !seen.insert(k.clone()) {
            continue;
        }
        let s = gen_flat_shape(rng, 1, 3);
        if !matches!(s, FlatShape::Scalar) {
            has_nested = true;
        }
        schema.push((k, s));
    }
    if !has_nested {
        let k = gen_bare_key(rng);
        let inner = FlatShape::Nested(vec![(
            gen_bare_key(rng),
            FlatShape::Nested(vec![(gen_bare_key(rng), FlatShape::Scalar)]),
        )]);
        schema.push((k, inner));
    }
    let rows = 2 + rng.int(6) as usize;
    let mut arr = Vec::with_capacity(rows);
    for _ in 0..rows {
        let mut row = serde_json::Map::new();
        for (f, s) in &schema {
            let x = rng.float();
            if x < 0.12 {
                continue; // field absent this row
            } else if x < 0.24 {
                row.insert(f.clone(), Value::Null); // field present-null (top-level null)
            } else {
                row.insert(f.clone(), materialize_flat_shape(rng, s));
            }
        }
        arr.push(Value::Object(row));
    }
    Value::Array(arr)
}

// Aligned arrays whose shared fields are fixed-shape nested objects, with a field
// or an intermediate nested level sometimes null/absent — the v3.2 flatten path the
// scalar-only generator never produces, so flatten/unflatten and its null-at-depth
// losslessness edge would otherwise be unexercised.
#[test]
fn test_flatten_roundtrip() {
    let iterations = get_iterations();
    let mut rng = Rng::new(7);
    for i in 0..iterations {
        let val = gen_flattenable_array(&mut rng);
        let gcf = encode_generic(&val);
        let decoded = decode_generic(&gcf).unwrap_or_else(|e| {
            panic!(
                "iteration {}: decode failed: {}\n  input: {}\n  gcf: {:?}",
                i, e, val, gcf
            );
        });
        assert!(
            structural_equal(&val, &decoded),
            "iteration {}: round-trip mismatch\n  input: {}\n  decoded: {}\n  gcf: {:?}",
            i,
            val,
            decoded,
            gcf
        );
    }
}

#[test]
fn test_random_roundtrip() {
    let iterations = get_iterations();
    let mut rng = Rng::new(42);
    for i in 0..iterations {
        let val = gen_value(&mut rng, 0, 4);
        let gcf = encode_generic(&val);
        let decoded = decode_generic(&gcf).unwrap_or_else(|e| {
            panic!(
                "iteration {}: decode failed: {}\n  input: {}\n  gcf: {:?}",
                i, e, val, gcf
            );
        });
        assert!(
            structural_equal(&val, &decoded),
            "iteration {}: round-trip mismatch\n  input:   {}\n  decoded: {}\n  gcf: {:?}",
            i,
            val,
            decoded,
            gcf
        );
    }
}

#[test]
fn test_adversarial_roundtrip() {
    let collision_strings = vec![
        "true",
        "false",
        "-",
        "~",
        "^",
        "0",
        "1",
        "42",
        "-1",
        "3.14",
        "1e10",
        "-0",
        "",
        " ",
        "  ",
        " x",
        "x ",
        "#",
        "# comment",
        "@0",
        "@handle",
        "+1",
        ".5",
        "+.3",
        "01",
        "00",
        "null",
        "NULL",
        "|",
        ",",
        "=",
        "\"",
        "\\",
        "\n",
        "\r",
        "\t",
        "a|b",
        "a,b",
        "a=b",
        "hello world",
    ];

    let iterations = get_iterations();
    let mut rng = Rng::new(99);
    for i in 0..iterations {
        let val = if rng.float() < 0.3 {
            Value::String(
                collision_strings[rng.int(collision_strings.len() as u32) as usize].to_string(),
            )
        } else {
            gen_value(&mut rng, 0, 3)
        };
        let gcf = encode_generic(&val);
        let decoded = decode_generic(&gcf).unwrap_or_else(|e| {
            panic!(
                "iteration {}: decode failed: {}\n  input: {}\n  gcf: {:?}",
                i, e, val, gcf
            );
        });
        assert!(
            structural_equal(&val, &decoded),
            "iteration {}: round-trip mismatch\n  input:   {}\n  decoded: {}\n  gcf: {:?}",
            i,
            val,
            decoded,
            gcf
        );
    }
}
