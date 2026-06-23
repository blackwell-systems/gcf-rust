use gcf::{decode_generic, encode_generic};
use serde_json::Value;
use std::io::Write;
use std::time::Instant;

// 665M to bring YAML total to 1B (335M already done)
const ITERATIONS: usize = 665_000_000;
const PROGRESS_INTERVAL: usize = 10_000_000;
// Start seeds at 334M to avoid overlap with previous run
const SEED_OFFSET: usize = 334_000_000;

fn rng_next(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

fn gen_key(seed: u64) -> String {
    let chars: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-.";
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
        4 => {
            let n = (rng_next(rng) % 6) as usize;
            let mut map = serde_json::Map::new();
            for _ in 0..n {
                let key = gen_key(*rng);
                *rng = rng.wrapping_add(key.len() as u64);
                map.insert(key, gen_value(rng, depth + 1, max_depth));
            }
            Value::Object(map)
        }
        5 => {
            let n = (rng_next(rng) % 8) as usize;
            Value::Array(
                (0..n)
                    .map(|_| gen_value(rng, depth + 1, max_depth))
                    .collect(),
            )
        }
        _ => gen_scalar(rng),
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
fn yaml_to_1b() {
    let start = Instant::now();
    let mut passed = 0usize;

    for i in 0..ITERATIONS {
        let seed = (i + SEED_OFFSET) as u64 + 1;
        let mut rng = seed;
        let v = gen_value(&mut rng, 0, 3);
        let yaml_str = match serde_yaml::to_string(&v) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let parsed: Value = match serde_yaml::from_str(&yaml_str) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let encoded = encode_generic(&parsed);
        let decoded = match decode_generic(&encoded) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("FAIL YAML seed={}: decode error: {e}", i + SEED_OFFSET);
                std::process::exit(1);
            }
        };

        if !values_equal(&parsed, &decoded) {
            eprintln!("FAIL YAML seed={}: mismatch", i + SEED_OFFSET);
            std::process::exit(1);
        }
        passed += 1;

        if passed % PROGRESS_INTERVAL == 0 {
            let elapsed = start.elapsed().as_secs_f64();
            let rate = passed as f64 / elapsed;
            let remaining = (ITERATIONS - passed) as f64 / rate;
            let total_yaml = passed + 335_000_000;
            eprint!(
                "\r  YAML: {passed}/{ITERATIONS} ({:.1}%) {rate:.0}/s ETA {:.0}m | total: {total_yaml}   ",
                passed as f64 / ITERATIONS as f64 * 100.0,
                remaining / 60.0,
            );
            std::io::stderr().flush().ok();
        }
    }

    let elapsed = start.elapsed().as_secs_f64();
    let total_yaml = passed + 335_000_000;
    eprintln!(
        "\r  YAML: {passed}/{ITERATIONS} (100%) in {elapsed:.1}s ({:.0}/s) | TOTAL: {total_yaml}          ",
        passed as f64 / elapsed,
    );
}
