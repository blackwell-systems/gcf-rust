use gcf::{decode_generic, encode_generic, encode_generic_with_options, GenericOptions};
use serde_json::Value;
use std::io::Write;
use std::time::Instant;

fn iterations() -> usize {
    std::env::var("FUZZ_ITERATIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000)
}
const PROGRESS_INTERVAL: usize = 10_000_000;

fn gen_key(seed: u64) -> String {
    let chars: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-.>";
    let mut rng = seed;
    let len = (rng_next(&mut rng) % 14) as usize + 1;
    let mut s = String::with_capacity(len);
    s.push(chars[(rng_next(&mut rng) % 52) as usize] as char);
    for _ in 1..len {
        s.push(chars[(rng_next(&mut rng) % chars.len() as u64) as usize] as char);
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

// Simple xorshift64 PRNG for speed
fn rng_next(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

fn gen_scalar(rng: &mut u64) -> Value {
    match rng_next(rng) % 6 {
        0 => Value::Null,
        1 => Value::Bool(rng_next(rng) % 2 == 0),
        2 => Value::Number(serde_json::Number::from(
            (rng_next(rng) % 200001) as i64 - 100000,
        )),
        3 => {
            let f = ((rng_next(rng) % 200000) as f64 - 100000.0) / 100.0;
            serde_json::Number::from_f64(f)
                .map(Value::Number)
                .unwrap_or(Value::Null)
        }
        _ => Value::String(gen_string(rng)),
    }
}

fn gen_value(rng: &mut u64, depth: usize, max_depth: usize) -> Value {
    if depth >= max_depth {
        return gen_scalar(rng);
    }
    match rng_next(rng) % 6 {
        0 => Value::Null,
        1 => Value::Bool(rng_next(rng) % 2 == 0),
        2 => gen_scalar(rng),
        3 => Value::String(gen_string(rng)),
        4 => gen_object(rng, depth, max_depth),
        5 => gen_array(rng, depth, max_depth),
        _ => gen_scalar(rng),
    }
}

fn gen_object(rng: &mut u64, depth: usize, max_depth: usize) -> Value {
    let n = (rng_next(rng) % 6) as usize;
    let mut map = serde_json::Map::new();
    for _ in 0..n {
        let key = gen_key(*rng);
        *rng = rng.wrapping_add(key.len() as u64);
        map.insert(key, gen_value(rng, depth + 1, max_depth));
    }
    Value::Object(map)
}

fn gen_array(rng: &mut u64, depth: usize, max_depth: usize) -> Value {
    let n = (rng_next(rng) % 8) as usize;
    let mut arr = Vec::with_capacity(n);
    for _ in 0..n {
        arr.push(gen_value(rng, depth + 1, max_depth));
    }
    Value::Array(arr)
}

fn gen_tabular(rng: &mut u64) -> Value {
    let num_rows = (rng_next(rng) % 15) as usize + 1;
    let num_cols = (rng_next(rng) % 6) as usize + 1;
    let mut keys = Vec::with_capacity(num_cols);
    for _ in 0..num_cols {
        let k = gen_key(*rng);
        *rng = rng.wrapping_add(k.len() as u64);
        if !keys.contains(&k) {
            keys.push(k);
        }
    }
    let mut rows = Vec::with_capacity(num_rows);
    for _ in 0..num_rows {
        let mut map = serde_json::Map::new();
        for k in &keys {
            map.insert(k.clone(), gen_scalar(rng));
        }
        rows.push(Value::Object(map));
    }
    Value::Array(rows)
}

fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Number(a), Value::Number(b)) => {
            // Compare as f64 for numeric equality
            let fa = a.as_f64().unwrap_or(f64::NAN);
            let fb = b.as_f64().unwrap_or(f64::NAN);
            fa == fb
        }
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| values_equal(x, y))
        }
        (Value::Object(a), Value::Object(b)) => {
            if a.len() != b.len() {
                return false;
            }
            a.iter()
                .all(|(k, v)| b.get(k).map_or(false, |bv| values_equal(v, bv)))
        }
        _ => false,
    }
}

fn gcf_roundtrip(data: &Value) -> Result<(), String> {
    // Test both flatten-on and flatten-off.
    for no_flatten in [false, true] {
        let encoded = encode_generic_with_options(data, &GenericOptions { no_flatten });
        let decoded = decode_generic(&encoded)
            .map_err(|e| format!("decode failed (no_flatten={no_flatten}): {e}"))?;
        if !values_equal(data, &decoded) {
            let a = serde_json::to_string(data).unwrap();
            let b = serde_json::to_string(&decoded).unwrap();
            return Err(format!(
                "mismatch (no_flatten={no_flatten})\n  original: {a}\n  decoded:  {b}"
            ));
        }
    }
    Ok(())
}

fn run_format_test(name: &str, iterations: usize, gen: impl Fn(&mut u64) -> Value) {
    let start = Instant::now();
    let mut passed = 0usize;

    for seed in 0..iterations {
        let mut rng = seed as u64 + 1; // avoid 0 seed
        let data = gen(&mut rng);

        if let Err(e) = gcf_roundtrip(&data) {
            eprintln!("FAIL {name} seed={seed}: {e}");
            std::process::exit(1);
        }
        passed += 1;

        if passed % PROGRESS_INTERVAL == 0 {
            let elapsed = start.elapsed().as_secs_f64();
            let rate = passed as f64 / elapsed;
            let remaining = (iterations - passed) as f64 / rate;
            eprint!(
                "\r  {name}: {passed}/{iterations} ({:.1}%) {rate:.0}/s ETA {:.0}m   ",
                passed as f64 / iterations as f64 * 100.0,
                remaining / 60.0,
            );
            std::io::stderr().flush().ok();
        }
    }

    let elapsed = start.elapsed().as_secs_f64();
    eprintln!(
        "\r  {name}: {passed}/{iterations} (100%) in {elapsed:.1}s ({:.0}/s)          ",
        passed as f64 / elapsed,
    );
}

// JSON omitted: already proven at 1B+ round-trips in existing fuzz suite.
// Tabular omitted: subset of JSON value space, same code path.

#[test]
fn multiformat_yaml() {
    run_format_test("YAML", iterations(), |rng| {
        let v = gen_value(rng, 0, 3);
        let yaml_str = serde_yaml::to_string(&v).unwrap();
        let parsed: Value = serde_yaml::from_str(&yaml_str).unwrap_or(Value::Null);
        parsed
    });
}

#[test]
fn multiformat_csv() {
    run_format_test("CSV", iterations(), |rng| {
        let num_rows = (rng_next(rng) % 10) as usize + 1;
        let num_cols = (rng_next(rng) % 5) as usize + 1;
        let mut keys = Vec::new();
        for _ in 0..num_cols {
            let k = gen_key(*rng);
            *rng = rng.wrapping_add(k.len() as u64);
            if !keys.contains(&k) {
                keys.push(k);
            }
        }
        if keys.is_empty() {
            return Value::Array(vec![]);
        }

        let mut buf = Vec::new();
        {
            let mut wtr = csv::Writer::from_writer(&mut buf);
            wtr.write_record(&keys).unwrap();
            for _ in 0..num_rows {
                let row: Vec<String> = keys.iter().map(|_| gen_string(rng)).collect();
                wtr.write_record(&row).unwrap();
            }
            wtr.flush().unwrap();
        }

        let mut rdr = csv::Reader::from_reader(&buf[..]);
        let headers: Vec<String> = rdr
            .headers()
            .unwrap()
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut rows = Vec::new();
        for result in rdr.records() {
            let record = result.unwrap();
            let mut map = serde_json::Map::new();
            for (i, h) in headers.iter().enumerate() {
                if let Some(val) = record.get(i) {
                    map.insert(h.clone(), Value::String(val.to_string()));
                }
            }
            rows.push(Value::Object(map));
        }
        Value::Array(rows)
    });
}

#[test]
fn multiformat_msgpack() {
    run_format_test("MessagePack", iterations(), |rng| {
        let v = gen_value(rng, 0, 4);
        let packed = rmp_serde::to_vec(&v).unwrap();
        let parsed: Value = rmp_serde::from_slice(&packed).unwrap_or(Value::Null);
        parsed
    });
}

// Tabular test removed: subset of JSON value space.
