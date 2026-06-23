use gcf::{decode_generic, encode_generic};
use serde_json::Value;
use std::io::Write;
use std::time::Instant;

const ITERATIONS: usize = 100_000_000;
const PROGRESS_INTERVAL: usize = 10_000_000;

fn rng_next(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

fn gen_key(rng: &mut u64) -> String {
    let chars: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-.";
    let len = (rng_next(rng) % 14) as usize + 1;
    let mut s = String::with_capacity(len);
    s.push(chars[(rng_next(rng) % 52) as usize] as char);
    for _ in 1..len {
        s.push(chars[(rng_next(rng) % chars.len() as u64) as usize] as char);
    }
    s
}

fn gen_string(rng: &mut u64) -> String {
    let chars: &[u8] =
        b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 _-.,;:/?&=~";
    let len = (rng_next(rng) % 20) as usize;
    let mut s = String::with_capacity(len);
    for _ in 0..len {
        s.push(chars[(rng_next(rng) % chars.len() as u64) as usize] as char);
    }
    s
}

// TOML-safe scalar: no null (TOML doesn't support null)
fn gen_toml_scalar(rng: &mut u64) -> Value {
    match rng_next(rng) % 4 {
        0 => Value::Bool(rng_next(rng) % 2 == 0),
        1 => Value::Number(serde_json::Number::from(
            (rng_next(rng) % 200001) as i64 - 100000,
        )),
        2 => {
            let f = ((rng_next(rng) % 200000) as f64 - 100000.0) / 100.0;
            serde_json::Number::from_f64(f)
                .map(Value::Number)
                .unwrap_or(Value::Bool(false))
        }
        _ => Value::String(gen_string(rng)),
    }
}

// TOML-safe value: no null, homogeneous arrays only
fn gen_toml_value(rng: &mut u64, depth: usize, max_depth: usize) -> Value {
    if depth >= max_depth {
        return gen_toml_scalar(rng);
    }
    match rng_next(rng) % 5 {
        0 => gen_toml_scalar(rng),
        1 => gen_toml_scalar(rng),
        2 => {
            // Nested table
            let n = (rng_next(rng) % 4) as usize + 1;
            let mut map = serde_json::Map::new();
            for _ in 0..n {
                map.insert(gen_key(rng), gen_toml_value(rng, depth + 1, max_depth));
            }
            Value::Object(map)
        }
        3 => {
            // Homogeneous array of scalars (same type)
            let n = (rng_next(rng) % 5) as usize;
            let arr: Vec<Value> = (0..n).map(|_| Value::String(gen_string(rng))).collect();
            Value::Array(arr)
        }
        _ => {
            // Array of tables (homogeneous objects)
            let n = (rng_next(rng) % 4) as usize + 1;
            let num_cols = (rng_next(rng) % 3) as usize + 1;
            let keys: Vec<String> = (0..num_cols).map(|_| gen_key(rng)).collect();
            let arr: Vec<Value> = (0..n)
                .map(|_| {
                    let mut map = serde_json::Map::new();
                    for k in &keys {
                        map.insert(k.clone(), gen_toml_scalar(rng));
                    }
                    Value::Object(map)
                })
                .collect();
            Value::Array(arr)
        }
    }
}

fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Number(a), Value::Number(b)) => {
            a.as_f64().unwrap_or(f64::NAN) == b.as_f64().unwrap_or(f64::NAN)
        }
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| values_equal(x, y))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, v)| b.get(k).map_or(false, |bv| values_equal(v, bv)))
        }
        _ => false,
    }
}

#[test]
fn toml_100m() {
    let start = Instant::now();
    let mut passed = 0usize;
    let mut skipped = 0usize;

    for seed in 0..ITERATIONS {
        let mut rng = seed as u64 + 1;

        // TOML root must be a table
        let n = (rng_next(&mut rng) % 5) as usize + 1;
        let mut map = serde_json::Map::new();
        for _ in 0..n {
            map.insert(gen_key(&mut rng), gen_toml_value(&mut rng, 0, 2));
        }
        let original = Value::Object(map);

        // Serialize to TOML
        let toml_str = match toml::to_string(&original) {
            Ok(s) => s,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };

        // Parse back
        let parsed: Value = match toml::from_str(&toml_str) {
            Ok(v) => v,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };

        // GCF round-trip
        let encoded = encode_generic(&parsed);
        let decoded = match decode_generic(&encoded) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("FAIL TOML seed={seed}: decode error: {e}");
                std::process::exit(1);
            }
        };

        if !values_equal(&parsed, &decoded) {
            let a = serde_json::to_string(&parsed).unwrap();
            let b = serde_json::to_string(&decoded).unwrap();
            eprintln!("FAIL TOML seed={seed}: mismatch\n  original: {a}\n  decoded:  {b}");
            std::process::exit(1);
        }
        passed += 1;

        if passed % PROGRESS_INTERVAL == 0 {
            let elapsed = start.elapsed().as_secs_f64();
            let rate = (passed + skipped) as f64 / elapsed;
            let remaining = (ITERATIONS - passed - skipped) as f64 / rate;
            eprint!(
                "\r  TOML: {passed} passed, {skipped} skipped ({:.1}%) {rate:.0}/s ETA {:.0}m   ",
                (passed + skipped) as f64 / ITERATIONS as f64 * 100.0,
                remaining / 60.0,
            );
            std::io::stderr().flush().ok();
        }
    }

    let elapsed = start.elapsed().as_secs_f64();
    eprintln!(
        "\r  TOML: {passed} passed, {skipped} skipped (100%) in {elapsed:.1}s ({:.0}/s)          ",
        passed as f64 / elapsed,
    );
}
